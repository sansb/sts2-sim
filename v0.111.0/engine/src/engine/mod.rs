//! The engine core (PORT_PLAN.md §8 R0.5, #1289) — first vertical slice.
//!
//! # What this slice is
//!
//! Exactly enough of `combat_sim` for an **unmodified Ironclad starter deck**
//! (`STRIKE_IRONCLAD` / `DEFEND_IRONCLAD` / `BASH`) against the two
//! `TOADPOLE`s of `fixtures/canonical_state_v2_ironclad_toadpoles.json` to
//! play end-to-end: [`legal_actions`], [`apply_action`], the damage pipeline,
//! draw/shuffle with exact `dotnet_sort` reshuffle parity, turn structure, and
//! the enemy phase.
//!
//! # What this slice is not
//!
//! Everything else. That is not a gap to be filled in later by widening these
//! functions — it is the design (D6). [`admission`] walks an entry's whole
//! reachable content closure before the engine touches it and returns a typed
//! [`AdmissionRefusal`] naming **every** capability it is missing, and each
//! dispatch site below refuses by kind rather than falling through. A state
//! the gate admits is one every function here is total on; a state it refuses
//! never reaches them. Refusal completeness is as much the deliverable as the
//! happy path.
//!
//! Concretely, the implemented registry is:
//!
//! | axis | implemented | everything else |
//! |---|---|---|
//! | card step kinds | `Attack`, `Block`, `Vulnerable` | [`EngineRefusal::StepKindNotModeled`] |
//! | monster move kinds | generated family manifests | [`EngineRefusal::MoveKindNotModeled`] |
//! | powers | admission's side-specific registries | [`MissingCapability::PowerState`] |
//! | actions | play a card, end the turn | [`EngineRefusal::PotionsNotModeled`] |
//! | continuations | none — the stack must be empty | [`MissingCapability::ContinuationFrame`] |
//!
//! ## Why three monster move kinds and not one
//!
//! #1289's slice statement says "monster `attack` moves". `TOADPOLE`'s loop is
//! `SPIKE_SPIT` / `WHIRL` / `SPIKEN` (`content_tables::LOOPS`), and the
//! checked-in fixture starts its two monsters at loop positions 2 and 1 — so
//! an attack-only enemy phase cannot complete even one enemy turn on the very
//! fixture the slice is defined against, and the required end-to-end
//! differential could reach no terminal. `buff_thorns` and `spit_attack` are
//! both four-line `monster_act` branches (frozen Python, deleted #2827) whose entire
//! behaviour is Thorns arithmetic plus the shared attack command, so porting
//! them faithfully was strictly smaller than the alternative of a slice with
//! no reachable terminal. Thorns retaliation is in the pipeline for the same
//! reason: `SPIKEN` makes it reachable.
//!
//! # Structure
//!
//! The submodules mirror Python's own decomposition rather than collapsing it,
//! so later slices grow into the shape instead of rewriting it:
//!
//! * [`damage`] — `player_attack` / `damage_monster` / `monster_attack_player`
//!   and the death cascade;
//! * [`play`] — `_apply_action_impl`'s play branch and the `_run_steps_inner`
//!   step dispatch;
//! * [`draw`] — `draw_cards` and the stable reshuffle;
//! * [`turn`] — `begin_player_turn` / `end_player_turn` / `_run_enemy_phase`;
//! * [`admission`] — the D6 entry gate.
//!
//! # Events
//!
//! [`Event`] is `Copy`, carries interned ids and integers only, and never
//! allocates (D7). The hot form is [`apply_action_into`], which buffers into a
//! caller-owned `Vec` that a search loop reuses; [`apply_action`] is the
//! convenience wrapper that owns one.

pub mod admission;
pub(crate) mod allies;
pub mod cards;
pub mod damage;
pub mod draw;
pub(crate) mod hook_action;
pub(crate) mod monsters;
pub mod native_checkpoint;
pub mod orbs;
pub mod play;
pub(crate) mod potions;
pub mod presentation;
pub(crate) mod puzzle;
pub(crate) mod relics;
pub(crate) mod selection;
pub use selection::{ordered_selection_answer, recorded_selection_answer, selected_card_uids};
pub mod turn;

use std::fmt;

thread_local! {
    /// Execution-only switch for the admission-dark ActionReplay driver.
    /// Public transitions never set it, so the legacy R26 quotient remains
    /// byte-for-byte originless until the atomic Rust/Python publication.
    static REPLAY_LIFECYCLE_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static EXACT_ROOT_REHEARSAL_BYPASS: std::cell::Cell<Option<(usize, Action)>> =
        const { std::cell::Cell::new(None) };
    /// The opening's deferred turn-one Draw fixup (#3404): armed only by
    /// [`deal_opening_hand_deferring_turn_one_fixup`] and consumed by the
    /// turn-one hand draw ([`turn::apply_deferred_turn_one_pile_fixup`]).
    static DEFERRED_TURN_ONE_FIXUP: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Consume the opening's deferred turn-one fixup, if one is armed.
pub(crate) fn take_deferred_turn_one_fixup() -> bool {
    DEFERRED_TURN_ONE_FIXUP.with(|armed| armed.replace(false))
}

struct ExactRootRehearsalGuard(Option<(usize, Action)>);

impl ExactRootRehearsalGuard {
    fn enter(state: &HotState, action: Action) -> Result<Self, EngineRefusal> {
        let saved = EXACT_ROOT_REHEARSAL_BYPASS
            .with(|slot| slot.replace(Some((state as *const HotState as usize, action))));
        if saved.is_some() {
            EXACT_ROOT_REHEARSAL_BYPASS.with(|slot| slot.set(saved));
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        Ok(Self(None))
    }
}

impl Drop for ExactRootRehearsalGuard {
    fn drop(&mut self) {
        EXACT_ROOT_REHEARSAL_BYPASS.with(|slot| slot.set(self.0.take()));
    }
}

fn consume_exact_root_rehearsal_bypass(state: &HotState, action: &Action) -> bool {
    EXACT_ROOT_REHEARSAL_BYPASS.with(|slot| {
        slot.get()
            .filter(|(identity, expected)| {
                *identity == state as *const HotState as usize && expected == action
            })
            .is_some_and(|_| {
                slot.set(None);
                true
            })
    })
}

struct ReplayLifecycleGuard(bool);

impl ReplayLifecycleGuard {
    fn enter(enabled: bool) -> Result<Self, EngineRefusal> {
        if enabled {
            REPLAY_LIFECYCLE_DEPTH.with(|depth| {
                depth.set(
                    depth
                        .get()
                        .checked_add(1)
                        .ok_or(EngineRefusal::CounterOverflow("replay lifecycle depth"))?,
                );
                Ok::<_, EngineRefusal>(())
            })?;
        }
        Ok(Self(enabled))
    }
}

impl Drop for ReplayLifecycleGuard {
    fn drop(&mut self) {
        if self.0 {
            REPLAY_LIFECYCLE_DEPTH.with(|depth| {
                depth.set(depth.get().checked_sub(1).expect("balanced replay scope"));
            });
        }
    }
}

pub(crate) fn replay_lifecycle_is_active() -> bool {
    REPLAY_LIFECYCLE_DEPTH.with(|depth| depth.get() != 0)
}

use crate::catalog::{CardAtom, CardSpec, CardTargetType, Catalog, CompiledArg};
use crate::hooks::{HookEvent, HookSubject};
use crate::hot::{ActionReplayRecord, HotCard, HotState, PileId};
use crate::ids::{CardId, MonsterKind, MoveKind, PotionId, PowerId, RelicId, StepKind};

pub use admission::{
    AdmissionRefusal, CapabilityManifest, MissingCapability, admit, capability_manifest,
};
pub use monsters::{
    BattlewornObjective, QUEEN_HP, THIEVING_HOPPER_ESCAPE_ARTIST, THIEVING_HOPPER_FLUTTER,
    THIEVING_HOPPER_HP, TORCH_HEAD_AMALGAM_HP, construct_aeonglass_boss,
};

/// A selected physical-card uid, niche-packed so `Action` stays small: the
/// uid is stored +1 in a `NonZeroU32`, making the None case free. Constructed
/// and read only through `new`/`get`, so no site can forget the offset.
/// (#1377's `Option<u32>` field cost 8 aligned bytes and tripped the
/// allocated-bytes/transition ceiling on the perf lane — the uid+1 encoding
/// plus the size pin below make the regression class unrepresentable.)
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct SelectionRef(Option<core::num::NonZeroU32>);

impl SelectionRef {
    /// No second card was chosen.
    pub const NONE: Self = Self(None);

    /// Pack an optional physical-card uid.
    #[must_use]
    pub fn new(uid: Option<u32>) -> Self {
        Self(uid.map(|u| {
            core::num::NonZeroU32::new(u.checked_add(1).expect("uid + 1 overflow"))
                .expect("nonzero by construction")
        }))
    }

    /// Unpack the physical-card uid this reference names.
    #[must_use]
    pub fn get(self) -> Option<u32> {
        self.0.map(|n| n.get() - 1)
    }
}

/// One player decision.
///
/// The slice's whole action vocabulary. `Play` names the physical card by uid
/// — the identity Python's allocator issued — rather than by hand position, so
/// an action stays meaningful across a state it was not enumerated in.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// Play the hand card with this uid, at its optional typed target byte.
    Play {
        /// The physical card's uid.
        uid: u32,
        /// `AnyAlly` programs interpret this as the stable Player key 0/1;
        /// enemy-targeted programs interpret it as the monster **roster
        /// index** (`combat_sim`'s `choice`), never its slot or uid.
        target: Option<u8>,
        /// A second physical card chosen by the action body (for example,
        /// Burning Pact's card to exhaust).
        selection: SelectionRef,
    },
    /// Resolve the current external card-selection continuation.
    Select {
        /// Either the exact replay-card uid or the deterministic ordinal of
        /// one generic selection answer. The pending payload determines
        /// which closed interpretation applies.
        answer: SelectionAnswer,
    },
    /// Drink the potion in this belt slot. Bodies remain refused in Part A,
    /// but slot and optional enemy target already have their final shape.
    UsePotion {
        /// Belt slot index.
        slot: u8,
        /// Enemy roster index for enemy-targeted potions; `None` for the
        /// represented solo player and non-targeted/selection potions.
        target: Option<u8>,
    },
    /// End the player turn, running the enemy phase and the next turn start.
    EndTurn,
}

/// `Action` is stored once per legal move in a caller-owned `Vec` retained by
/// each search worker. Its width therefore controls retained buffer capacity
/// and the traffic needed to partition/copy actions, so it remains a
/// load-bearing perf fact rather than an implementation detail — the same
/// discipline `hot.rs` applies to the hot-state types.
const _: () = assert!(std::mem::size_of::<Action>() == 12);

/// One external selection answer without putting a variable-width card list
/// in every hot action.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum SelectionAnswer {
    /// The historical `exhaust_draw` replay seam names one physical card.
    CardUid(u32),
    /// Generic selectors enumerate their exact answer list deterministically.
    OptionIndex(u32),
}

/// Which side of the board an event's subject is on.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Subject {
    /// The player.
    Player,
    /// One monster, by its creation-order `uid`.
    Monster(u32),
}

/// One observable engine event.
///
/// `Copy`, interned, and allocation-free by construction (D7): every payload
/// is a small integer or a generated id, and names are resolved only at the
/// canonical boundary. The differential compares event tokens *outside* the
/// hot loop.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    /// A card left the hand and began its play.
    CardPlayed {
        /// Physical uid.
        uid: u32,
        /// Interned identity.
        atom: CardAtom,
        /// Energy spent.
        energy: i16,
    },
    /// A card came to rest in a pile after its play finished.
    CardResolved {
        /// Physical uid.
        uid: u32,
        /// Where it landed.
        pile: PileId,
    },
    /// One damage result against a monster.
    MonsterDamaged {
        /// The target.
        uid: u32,
        /// Block consumed.
        blocked: i32,
        /// HP actually lost.
        unblocked: i32,
        /// HP after the hit.
        hp: i32,
    },
    /// A monster reached zero HP and ran its death cleanup.
    MonsterDied {
        /// The dead monster.
        uid: u32,
    },
    /// One damage result against the player.
    PlayerDamaged {
        /// Block consumed.
        blocked: i32,
        /// HP actually lost.
        hp_lost: i32,
        /// HP after the hit.
        hp: i32,
    },
    /// The player gained block from a card.
    PlayerBlockGained {
        /// The full positive modified gain passed to history/listeners. Native
        /// Block storage may clamp below this amount at its fixed ceiling.
        amount: i32,
        /// Block after the gain.
        block: i32,
    },
    /// A power's amount changed on some creature.
    PowerChanged {
        /// Whose power.
        subject: Subject,
        /// Which power.
        power: PowerId,
        /// The new amount.
        amount: i32,
    },
    /// One card moved from the draw pile to the hand.
    CardDrawn {
        /// Physical uid.
        uid: u32,
    },
    /// The discard pile was sorted and shuffled back into the draw pile.
    Reshuffled {
        /// How many cards moved.
        cards: u16,
    },
    /// A monster executed one move.
    MonsterMoved {
        /// The actor.
        uid: u32,
        /// The move's dispatch kind.
        kind: MoveKind,
    },
    /// The player turn ended.
    TurnEnded {
        /// The turn that just ended.
        turn: i16,
    },
    /// A player turn began.
    TurnBegan {
        /// The new turn number.
        turn: i16,
    },
    /// Combat reached a terminal state.
    CombatOver {
        /// Whether the player survived (every monster dead).
        player_won: bool,
    },
}

/// The successor state plus the events its transition emitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transition {
    /// The state after the action.
    pub state: HotState,
    /// Events, in the order the engine produced them.
    pub events: Vec<Event>,
}

/// Why the engine could not run an action.
///
/// Every variant names a concrete missing mechanic or a concrete contract
/// violation. There is no catch-all and no variant meaning "close enough"
/// (`SOLVER_INVARIANTS.md` I5, ported).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineRefusal {
    /// The entry is outside the implemented slice.
    NotAdmitted(AdmissionRefusal),
    /// The action names a uid that is not in hand.
    CardNotInHand(u32),
    /// A body froze a card's physical identity and it was gone from the pile
    /// by the time the body reached it. Python raises at the same point
    /// rather than re-resolving by payload; reaching it here means an engine
    /// path moved a card the freezing body did not expect.
    FrozenCardVanished {
        /// The frozen physical uid.
        uid: u32,
        /// Where the body expected to find it.
        pile: PileId,
    },
    /// A running card body could not resolve its explicit physical source uid
    /// to exactly one of the five live piles. Direct AutoPlay sources may
    /// legitimately remain outside Play, so a pile-specific absence would be
    /// misleading here; zero and duplicate matches both fail closed.
    ActiveCardNotUnique {
        /// The active physical uid supplied to the body.
        uid: u32,
        /// How many live pile instances carried that uid.
        matches: usize,
    },
    /// The named card carries an identity this fight never interned.
    UnknownAtom(CardAtom),
    /// A mid-combat creator requested an identity outside the catalog's
    /// deterministically pre-interned creation closure.
    UnknownMintIdentity(crate::catalog::CardIdentity),
    /// The card costs more energy than the player has.
    NotEnoughEnergy {
        /// The card's resolved cost.
        cost: i64,
        /// Energy available.
        energy: i16,
    },
    /// The card is on `_card_can_play`'s multiplayer identity census (#1613)
    /// and its live player-count or teammate-relation condition currently
    /// blocks play.
    ///
    /// Enumeration omits these, so a well-formed search never produces one.
    /// Reaching this means a *forged* or replayed action named the card
    /// directly, and it must not be reported as a resource shortage — the
    /// player could hold unlimited energy and still not be able to play it.
    CardNotPlayableSolo(CardId),
    /// The card costs more Stars than the player has.
    NotEnoughStars {
        /// The card's resolved fixed Star cost.
        cost: i64,
        /// Stars available.
        stars: i16,
    },
    /// A targeted card was played without a target, or vice versa.
    TargetMismatch {
        /// Whether the card requires one.
        required: bool,
    },
    /// The card body requires a second card selection and none was supplied,
    /// or the action supplied one to a body that does not consume it.
    SelectionMismatch {
        /// Whether this card requires a selection.
        required: bool,
    },
    /// The chosen roster index is outside the roster, or names a corpse.
    BadTarget(u8),
    /// The named potion slot is outside the fixed belt or currently null.
    BadPotionSlot(u8),
    /// A potion action omitted its required enemy target, or supplied one to
    /// a non-enemy-targeted model.
    PotionTargetMismatch {
        /// Whether this potion requires an enemy target.
        required: bool,
    },
    /// The action arrived at a terminal state.
    CombatOver,
    /// A card step kind this slice does not implement.
    StepKindNotModeled(StepKind),
    /// A monster move kind this slice does not implement.
    MoveKindNotModeled(MoveKind),
    /// An orb kind whose passive/evoke body this slice does not implement.
    OrbKindNotModeled(crate::hot::OrbKind),
    /// A step or move whose argument tuple does not match its implemented
    /// shape. The admission gate normally catches this at entry; reaching it
    /// here means a table changed under a live catalog.
    MalformedArgs(&'static str),
    /// Potions are not modeled in this slice.
    PotionsNotModeled,
    /// A counter would leave the range its hot slot can represent. Python's
    /// ints are unbounded; refusing beats wrapping.
    CounterOverflow(&'static str),
    /// Two represented player-power callbacks share one event, but native
    /// acquisition order is not serialized and the live values make that
    /// order observable for this action.
    PowerOrderNotModeled(&'static str),
    /// A unique/non-stackable power was applied while its one live instance
    /// was already present, and native restack semantics are not represented.
    PowerRestackNotModeled(PowerId),
    /// A continuation frame was pushed, which this slice cannot resume from.
    /// Every modeled body runs to completion inside one action.
    ContinuationNotModeled,
    /// The action is not legal in this state.
    ///
    /// Its own variant since #2473: the replay-aware apply path used to report
    /// a failed legality test as [`Self::ContinuationNotModeled`], so a driver
    /// reading `"a continuation frame is not resumable in this slice"` was
    /// sent to the continuation subsystem for a state that had no continuation
    /// at all — before or after. The `&'static str` names the test that
    /// refused, not the action.
    ActionNotLegal(&'static str),
    /// A live (not `history.over`) state enumerated no legal action and no
    /// more specific refusal explains why (#2985). A search prunes such a
    /// state as refused rather than aborting; see [`legal_actions_checked`].
    NoLegalActions,
    /// A pending selection carried reserved bits or an ordinal outside the
    /// closed pile/kind routing vocabularies.
    PendingSelectionRouting(u8),
    /// A monster kind whose move loop this fight never compiled.
    MonsterLoopNotModeled(MonsterKind),
    /// A relic subscribes to a hook the engine fires and has no modeled body.
    /// Unreachable through [`admit`], which refuses every relic; reaching it
    /// means a table was built past the gate.
    HookNotModeled {
        /// The subscribing relic.
        relic: RelicId,
        /// The event being fired.
        event: HookEvent,
    },
    /// `OstyCmd.Summon` reached a dead or absent Osty while the combat is
    /// ending; see `SoloPetState::summon_ending` (#3246).
    EndingSummonNotModeled(&'static str),
    /// A random-target listener picked a live enemy while the combat is
    /// ending, and native would run `CreatureCmd.Damage` against it (#3261).
    /// `<Damage>d__12` (RVA `0x3e96c8`) has no ending gate on entry; the
    /// full pipeline under IsEnding is not represented.
    EndingDamageNotModeled(&'static str),
    /// A card-body step kind was reached while the combat is ending, and its
    /// native command's ending gate has not been audited (#3495). See
    /// `play::step_kind_runs_after_combat_end`.
    EndingStepNotModeled(StepKind),
    /// A body read a sentinel-backed history counter that this fight never
    /// tracked (still at its `-1` "absent" sentinel), so the native count is
    /// unknown (#3003: Voltaic played in a fight whose root held none).
    UntrackedCounterNotModeled(&'static str),
    /// A power subscribes to a fan-out event with no modeled body.
    PowerHookNotModeled {
        /// The subscribing power.
        power: PowerId,
        /// The event being fired.
        event: HookEvent,
    },
}

impl fmt::Display for EngineRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAdmitted(refusal) => write!(f, "entry not admitted: {refusal}"),
            Self::CardNotInHand(uid) => write!(f, "card#{uid} is not in hand"),
            Self::FrozenCardVanished { uid, pile } => write!(
                f,
                "frozen card#{uid} left the {:?} pile before its body reached it",
                pile.as_str()
            ),
            Self::ActiveCardNotUnique { uid, matches } => write!(
                f,
                "active card#{uid} has {matches} live pile matches instead of one"
            ),
            Self::UnknownAtom(atom) => write!(f, "card atom {atom} was never interned"),
            Self::UnknownMintIdentity(identity) => write!(
                f,
                "mid-combat card identity {:?}+{} was not pre-interned",
                identity.id.as_str(),
                identity.upgrade
            ),
            Self::NotEnoughEnergy { cost, energy } => {
                write!(f, "card costs {cost} with {energy} energy available")
            }
            Self::NotEnoughStars { cost, stars } => {
                write!(f, "card costs {cost} with {stars} Stars available")
            }
            Self::CardNotPlayableSolo(id) => write!(
                f,
                "{id:?} is unplayable with one living player (multiplayer-only \
                 CanPlay census)"
            ),
            Self::TargetMismatch { required } => {
                if *required {
                    f.write_str("a targeted card was played without a target")
                } else {
                    f.write_str("an untargeted card was played with a target")
                }
            }
            Self::SelectionMismatch { required } => {
                if *required {
                    f.write_str("this card requires a second card selection")
                } else {
                    f.write_str("this card does not accept a second card selection")
                }
            }
            Self::BadTarget(index) => write!(f, "monster index {index} is absent or dead"),
            Self::CombatOver => f.write_str("combat has already ended"),
            Self::StepKindNotModeled(kind) => {
                write!(f, "card step kind {:?} is not modeled", kind.as_str())
            }
            Self::MoveKindNotModeled(kind) => {
                write!(f, "monster move kind {:?} is not modeled", kind.as_str())
            }
            Self::OrbKindNotModeled(kind) => {
                write!(f, "orb kind {:?} is not modeled", kind.as_str())
            }
            Self::MalformedArgs(site) => write!(f, "malformed argument tuple at {site}"),
            Self::PotionsNotModeled => f.write_str("potions are not modeled in this slice"),
            Self::CounterOverflow(name) => write!(f, "counter {name} left its hot range"),
            Self::PowerOrderNotModeled(event) => {
                write!(f, "player-power acquisition order is observable at {event}")
            }
            Self::PowerRestackNotModeled(power) => {
                write!(f, "power {:?} cannot be restacked exactly", power.as_str())
            }
            Self::ContinuationNotModeled => {
                f.write_str("a continuation frame is not resumable in this slice")
            }
            Self::ActionNotLegal(test) => {
                write!(f, "the action is not legal in this state: {test}")
            }
            Self::NoLegalActions => f.write_str("a live state enumerated no legal action"),
            Self::PendingSelectionRouting(raw) => {
                write!(f, "pending selection routing byte {raw:#04x} is invalid")
            }
            Self::BadPotionSlot(slot) => {
                write!(f, "potion belt slot {slot} is absent or empty")
            }
            Self::PotionTargetMismatch { required } => write!(
                f,
                "potion target presence does not match required={required}"
            ),
            Self::MonsterLoopNotModeled(kind) => {
                write!(f, "no compiled move loop for {:?}", kind.as_str())
            }
            Self::HookNotModeled { relic, event } => write!(
                f,
                "relic {:?} subscribes to {:?}, which has no modeled body",
                relic.as_str(),
                event.as_str()
            ),
            Self::EndingSummonNotModeled(site) => write!(
                f,
                "{site} summons a dead or absent Osty while the combat is ending"
            ),
            Self::EndingDamageNotModeled(site) => {
                write!(f, "{site} damages a live enemy while the combat is ending")
            }
            Self::EndingStepNotModeled(kind) => write!(
                f,
                "card step {:?} runs while the combat is ending, and its ending gate is not audited",
                kind.as_str()
            ),
            Self::UntrackedCounterNotModeled(counter) => {
                write!(f, "{counter} is read but was never tracked in this fight")
            }
            Self::PowerHookNotModeled { power, event } => write!(
                f,
                "power {:?} subscribes to {:?}, which has no modeled body",
                power.as_str(),
                event.as_str()
            ),
        }
    }
}

impl std::error::Error for EngineRefusal {}

impl From<crate::hot::PendingSelectionRoutingError> for EngineRefusal {
    fn from(error: crate::hot::PendingSelectionRoutingError) -> Self {
        Self::PendingSelectionRouting(error.raw())
    }
}

impl From<AdmissionRefusal> for EngineRefusal {
    fn from(refusal: AdmissionRefusal) -> Self {
        Self::NotAdmitted(refusal)
    }
}

/// Everything one card-step body may touch.
///
/// The generated dispatch in [`crate::steps`] hands this to a family body; the
/// body reads `args`, which are **compiled** (typed ids and integers — no
/// content text ever reaches here, by construction of the admission-time
/// compiler).
pub struct StepCtx<'a> {
    /// The state being advanced in place.
    pub state: &'a mut HotState,
    /// This fight's constants: specs, compiled programs, hook tables.
    pub catalog: &'a Catalog,
    /// The card whose program is running.
    pub spec: &'a CardSpec,
    /// The exact physical card whose body is running.
    pub source_uid: u32,
    /// The typed target operand: a monster-roster index for enemy-targeted
    /// rows, or a stable Player key for `AnyAlly` rows.
    pub target: Option<usize>,
    /// The selected physical-card uid for a body with a manual card choice.
    pub selection: Option<u32>,
    /// The resolved Energy-X or Star-X of this play. Python
    /// `resolve_energy_x_value` (frozen, deleted #2827) and `resolve_star_x_value` define the two resource-specific paths; this is zero for
    /// every card without either X cost.
    ///
    /// Captured once at `SpendResources` for a manual play, or at the direct
    /// AutoPlay entry without payment. Generated replays retain that first
    /// capture rather than re-reading the resource pool; a gain inside the
    /// body therefore cannot inflate the same play series' X.
    pub x_value: i64,
    /// This step's compiled arguments.
    pub args: &'a [CompiledArg],
    /// The transition's event buffer.
    pub events: &'a mut Vec<Event>,
}

/// Everything one monster-move body may touch.
pub struct MoveCtx<'a> {
    /// The state being advanced in place.
    pub state: &'a mut HotState,
    /// This fight's constants.
    pub catalog: &'a Catalog,
    /// The acting monster's roster index.
    pub actor: usize,
    /// This move's compiled arguments.
    pub args: &'a [CompiledArg],
    /// The transition's event buffer.
    pub events: &'a mut Vec<Event>,
}

impl MoveCtx<'_> {
    /// One ascension-tiered move constant at the tier this fight's move rows
    /// were compiled at ([`Catalog::ascension`], #2828): a body's pin of its
    /// own row compares against this, never against one hard-coded tier.
    pub fn tier(&self, tier: crate::content_tables::AscensionTier) -> i64 {
        self.catalog.tier(tier)
    }
}

/// The hook events this engine fires, in the order a turn reaches them.
///
/// The manifest reports this list, and `tests/hot_path_contract.rs` derives the
/// claim rather than trusting it: every event named here must appear at a fire
/// site in an engine module, and every `HookEvent` absent from it must not.
///
/// `BeforeCombatStart` and `AfterRoomEntered` joined the list at #2528 E4a,
/// when entry synthesis moved into Rust: [`crate::entry::opening`] now builds
/// the post-`start_combat` state itself instead of being handed one Python
/// built, so the two combat-start hooks have a Rust fire site for the first
/// time. They are fired only from [`fire_after_room_entered`] and
/// [`fire_before_combat_start`], which run **once per fight** at the opening —
/// never on a per-node path, and never from a document that already reflects
/// them (`from_canonical` still loads a post-opening state without replaying
/// either).
pub const FIRE_POINTS: [HookEvent; 17] = [
    HookEvent::AfterRoomEntered,
    HookEvent::BeforeCombatStart,
    HookEvent::BeforeSideTurnStart,
    HookEvent::AfterBlockCleared,
    HookEvent::AfterEnergyReset,
    HookEvent::ModifyMaxEnergy,
    HookEvent::ModifyHandDraw,
    HookEvent::AfterCardDrawn,
    HookEvent::AfterPlayerTurnStart,
    HookEvent::AfterPlayerTurnStartLate,
    HookEvent::AfterSideTurnStart,
    HookEvent::BeforeCardPlayed,
    HookEvent::AfterEnergySpent,
    HookEvent::AfterBlockGained,
    HookEvent::AfterCardPlayed,
    HookEvent::BeforeSideTurnEnd,
    HookEvent::AfterSideTurnEnd,
];

/// Fire one hook event over its subscriber list (D5).
///
/// The cost of a fire point with nothing subscribed is **one bit test**, which
/// is the whole reason the table exists: Python's fire points walk every relic
/// and every power, and the port fires ~30 of them per turn.
///
/// Relic-template subscribers are interpreted by [`relics`] in authenticated
/// inventory order. Power subscribers retain their typed refusal until their
/// separate fan-out bodies are wired here.
pub(crate) fn fire_hook(
    catalog: &Catalog,
    event: HookEvent,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    // Reaching a fire point is the coverage fact, subscribers or not: the
    // question a wave PR's smoke config asks is whether the turn structure ran
    // through this event at all (PORT_PLAN §4).
    crate::coverage::record_hook(event);
    // `_fire_relic_templates` stops before its first inventory listener once
    // combat has ended. Some surrounding turn-start groups still reach their
    // structural fire point after an earlier damage listener ended combat.
    if state.history.over {
        return Ok(());
    }
    let hooks = catalog.hooks();
    if !hooks.has(event) {
        return Ok(());
    }
    for subscriber in hooks.subscribers(event) {
        match subscriber.subject {
            HookSubject::Relic(relic) => {
                relics::fire_template_subscriber(catalog, subscriber, relic, event, state, events)?;
            }
            HookSubject::Power(power) => {
                return Err(EngineRefusal::PowerHookNotModeled { power, event });
            }
        }
        if state.history.over {
            break;
        }
    }
    Ok(())
}

/// [`fire_hook`] for a relic hook whose template subscribers share one native
/// listener walk with hand-written relic bodies, run in the vouched inventory
/// order (#3400).
///
/// Native walks relic listeners in `Player.Relics` order
/// (`CombatState/<IterateHookListeners>d__69::MoveNext` RVA `0x3f9720`), and
/// [`crate::hooks::HookTable::subscribers`] keeps that order within an event.
/// So each inventory entry, in turn, either fires its template subscription
/// (the next subscriber, if it is this relic's) or is handed to `hand`, which
/// runs the entry's hand-written body if it has one. Template subscribers
/// keep `fire_hook`'s ending rule: none fires once combat has ended. `hand`
/// is called for every other entry whatever the ending state, because each
/// hand-written body carries its own native gate. Only a vouched inventory
/// ([`crate::hooks::HookTable::dispatch_ordered`]) states that order; the
/// caller keeps [`fire_hook`] and its fixed order for any other.
pub(crate) fn fire_hook_in_inventory_order(
    catalog: &Catalog,
    event: HookEvent,
    state: &mut HotState,
    events: &mut Vec<Event>,
    mut hand: impl FnMut(RelicId, &mut HotState, &mut Vec<Event>) -> Result<(), EngineRefusal>,
) -> Result<(), EngineRefusal> {
    // A relic-template hook has relic subscriptions only; power fan-outs are
    // a different category ([`crate::hooks::HookCategory`]).
    debug_assert_eq!(event.category(), crate::hooks::HookCategory::RelicTemplate);
    crate::coverage::record_hook(event);
    let hooks = catalog.hooks();
    let subscribers = if hooks.has(event) {
        hooks.subscribers(event)
    } else {
        &[]
    };
    let mut next = 0;
    for &relic in hooks.relics() {
        match subscribers.get(next) {
            Some(subscriber) if subscriber.subject == HookSubject::Relic(relic) => {
                next += 1;
                if !state.history.over {
                    relics::fire_template_subscriber(
                        catalog, subscriber, relic, event, state, events,
                    )?;
                }
            }
            _ => hand(relic, state, events)?,
        }
    }
    // `HookTable::build` derives every relic subscription from one inventory
    // entry, in inventory order, so the walk consumes them all.
    debug_assert_eq!(next, subscribers.len());
    Ok(())
}

/// `Hook::AfterRoomEntered` — **one** pass over **run-level** listeners only.
///
/// `Hook/<AfterRoomEntered>d__72::MoveNext` (v0.111.0 RVA `0x3d0eec`) pushes
/// `ldnull` as the `childCombatState` argument to
/// `IRunState::IterateHookListeners` at IL_001d, where
/// `Hook/<BeforeCombatStart>d__18::MoveNext` (`0x3d2574`) pushes the live
/// combat state at IL_0025. `RunState::IterateHookListeners` (`0x4ec08`, body
/// `0x30e588`) only descends into the `CombatState` walk when a combat state
/// was supplied, so **a combat power cannot observe `AfterRoomEntered` at
/// all** — which is exactly why Python fires only relic templates there
/// (frozen Python `start_combat`, deleted #2827).
///
/// Wiring this to the same subscriber list as `BeforeCombatStart` would be a
/// silent widening, so a power subscriber here is a typed refusal rather than
/// a body that runs one hook early. The one-pass shape is the hook's own: the
/// enumerator at IL_0028 is walked once and never restarted.
///
/// Fires after the deck is instantiated and shuffled and before the deal,
/// which is what makes Stone Cracker's pre-draw upgrade well-defined
/// (`StoneCracker::<AfterRoomEntered>d__4` `0x3782d4`).
pub(crate) fn fire_after_room_entered(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    let event = HookEvent::AfterRoomEntered;
    crate::coverage::record_hook(event);
    if state.history.over {
        return Ok(());
    }
    let hooks = catalog.hooks();
    // The compiled walk is skipped when nothing subscribes, but the function
    // does not return early: Ghost Seed's body below is not a compiled
    // subscriber and must run either way.
    let subscribers = if hooks.has(event) {
        hooks.subscribers(event)
    } else {
        &[]
    };
    for subscriber in subscribers {
        match subscriber.subject {
            HookSubject::Relic(relic) => {
                relics::fire_template_subscriber(catalog, subscriber, relic, event, state, events)?;
            }
            // Unreachable from the game: `IterateHookListeners(null)` never
            // reaches a creature's Powers. Refused rather than fired, because
            // firing it would make Rust's subscriber domain wider than the
            // native one.
            HookSubject::Power(power) => {
                return Err(EngineRefusal::PowerHookNotModeled { power, event });
            }
        }
        if state.history.over {
            break;
        }
    }
    // Ghost Seed is not a template relic, so it has no compiled subscriber in
    // the walk above. The oracle runs it right after that walk, in the same
    // acquisition-ordered group (frozen Python `start_combat`, deleted #2827); the body and
    // its ordering argument are `cards::ghost_seed_after_room_entered`.
    cards::ghost_seed_after_room_entered(state, catalog);
    Ok(())
}

/// `Hook::BeforeCombatStart` — **two** complete passes, not one.
///
/// `Hook/<BeforeCombatStart>d__18::MoveNext` (v0.111.0 RVA `0x3d2574`) walks
/// `IterateHookListeners` to completion calling
/// `AbstractModel::BeforeCombatStart` (IL_002a–IL_00be), then walks the list
/// **again from the start** calling `AbstractModel::BeforeCombatStartLate`
/// (IL_0103–IL_0133). A single-pass dispatch would reorder every `…Late`
/// subscriber against every ordinary one.
///
/// Python models exactly this split: Petrified Toad is applied *after*
/// `_fire_relic_templates(s, "BeforeCombatStart")` (frozen Python `start_combat`, deleted #2827),
/// with the comment that the hook "first completes every ordinary listener,
/// then snapshots the distinct `BeforeCombatStartLate` pass".
///
/// # The second pass, and why it is empty here rather than absent
///
/// `BeforeCombatStartLate` has exactly **two** implementors at v0.111.0:
/// `AbstractModel::BeforeCombatStartLate` (`0x7a00f`), whose whole body is
/// `call Task::get_CompletedTask; ret`, and `PetrifiedToad` (`0x9939c`). So
/// the second pass is a no-op for every owner who does not hold Petrified
/// Toad, and that is a measured fact rather than an assumption.
///
/// Petrified Toad is not a template relic, so it has no compiled subscriber to
/// walk. Its body is [`petrified_toad_before_combat_start_late`], called by
/// name as the whole of the second pass, so the pass is always derived from
/// catalog ownership — never skipped because the compiled subscriber list
/// happened to be empty. (Until #2827 the second pass refused by name here.)
/// The Osty amount `BoundPhylactery` summons.
///
/// `BoundPhylactery::get_CanonicalVars` (v0.111.0 RVA `0x91471`) is
/// `ldsfld Decimal::One; newobj SummonVar::.ctor` at `IL_0001`-`IL_0006`.
const BOUND_PHYLACTERY_SUMMON: i32 = 1;

/// The Osty amount `PhylacteryUnbound` summons at combat start.
///
/// `PhylacteryUnbound::get_CanonicalVars` (v0.111.0 RVA `0x9959d`) builds
/// `SummonVar("StartOfCombat", 5)` at `IL_0009`-`IL_0014` and
/// `SummonVar("StartOfTurn", 2)` at `IL_001c`-`IL_0027`. The turn-start 2 is
/// `relics::after_side_turn_start_late`'s row.
const PHYLACTERY_UNBOUND_COMBAT_START_SUMMON: i32 = 5;

/// The two phylacteries' combat-start Osty summons (#2827).
///
/// # Bound Phylactery
///
/// `BoundPhylactery/<BeforeCombatStart>d__8::MoveNext` (v0.111.0 RVA
/// `0x3206b8`) calls `SummonPet` at `IL_001e` unconditionally. `SummonPet`
/// (`<SummonPet>d__10::MoveNext`, `0x32077c`) is
/// `OstyCmd::Summon(Owner, DynamicVars.Summon.BaseValue)` at `IL_0039`: a
/// fresh Osty at `amount`/`amount`, or `+amount` to both on a living one,
/// which is exactly [`crate::pet::SoloPetState::summon`]. The turn>1 re-summon
/// is the other half of the relic, in `relics::after_energy_reset_late`.
///
/// # Phylactery Unbound
///
/// `PhylacteryUnbound/<BeforeCombatStart>d__10::MoveNext` (v0.111.0 RVA
/// `0x32e234`) has no guard: it builds a `ThrowingPlayerChoiceContext`
/// (`IL_001d`) and awaits `OstyCmd::Summon(Owner,
/// DynamicVars["StartOfCombat"].BaseValue)` at `IL_003e`, which is
/// [`PHYLACTERY_UNBOUND_COMBAT_START_SUMMON`]. Its other opening hook is the
/// turn-start `AfterSideTurnStart` (`<AfterSideTurnStart>d__11::MoveNext`,
/// `0x32e134`): an owner-participates test (`Contains<Creature>` at
/// `IL_002e`, `leave` at `IL_0035` otherwise) and then `OstyCmd::Summon(Owner,
/// DynamicVars["StartOfTurn"].BaseValue)` at `IL_005b`, with **no** turn test,
/// so it fires on turn one too. That half was already
/// `relics::after_side_turn_start_late`'s `summon(2)` row, which the opening
/// reaches through `deal_opening_hand`, so an owner's first document carries a
/// 7/7 Osty.
///
/// # Where it runs
///
/// The oracle runs both after `_fire_relic_templates(s, "BeforeCombatStart")`
/// and the relic pets, in `relics_entering` order
/// (frozen Python, deleted #2827), and so does this, after the compiled
/// listeners and [`passive_relic_pets_before_combat_start`]. Holding both is
/// refused before either summon: the oracle refuses the pair in
/// `start_combat`, the opening refuses it by
/// name (`entry::opening`'s `OpeningRefusal::RelicCombinationRefused`), and
/// `engine::admission` refuses it for every other root. So the order between
/// the two summons is never observed. No RNG stream is touched
/// (`summon_ally`'s harness-verified note).
fn phylactery_before_combat_start(
    catalog: &Catalog,
    state: &mut HotState,
) -> Result<(), EngineRefusal> {
    if state.history.over {
        return Ok(());
    }
    for (relic, amount, what) in [
        (
            RelicId::RelicBoundPhylactery,
            BOUND_PHYLACTERY_SUMMON,
            "Bound Phylactery Osty",
        ),
        (
            RelicId::RelicPhylacteryUnbound,
            PHYLACTERY_UNBOUND_COMBAT_START_SUMMON,
            "Phylactery Unbound Osty",
        ),
    ] {
        if !catalog.hooks().owns(relic) {
            continue;
        }
        summon_osty(state, amount, what)?;
        crate::coverage::record_relic(relic);
    }
    Ok(())
}

/// `OstyCmd.Summon(owner, amount)` under the live IsEnding projection.
///
/// Native `<Summon>d__0::MoveNext` (RVA `0x3ee040`) has no IsEnding gate of
/// its own; the gate lives in the `CreatureCmd.Heal` it reaches through
/// `GainMaxHp` (`0x3eb2f0` `IL_010b`) or directly (`IL_03f7`), and
/// `<Heal>d__20` (`0x3eb4b0` `IL_0041`-`IL_005f`) skips a non-player
/// creature while `CombatManager.IsEnding`. So every summon site reads
/// [`damage::damage_combat_is_ending`] (the IsEnding projection, not
/// `history.over`) at the moment it runs; `SoloPetState::summon_ending`
/// carries the arithmetic and the refused dead/absent branch (#3246).
pub(crate) fn summon_osty(
    state: &mut HotState,
    amount: i32,
    site: &'static str,
) -> Result<(), EngineRefusal> {
    let ending = damage::damage_combat_is_ending(state);
    state
        .fanouts
        .mutate_pet(|pet| pet.summon_ending(amount, ending))
        .map_err(|error| match error {
            crate::pet::PetStateError::EndingSummonNotLive => {
                EngineRefusal::EndingSummonNotModeled(site)
            }
            _ => EngineRefusal::CounterOverflow(site),
        })
}

/// The `CombatTargets` pick of an AfterCardPlayed random-hit listener that has
/// no ending gate of its own (#3261): Kusarigama, `HauntPower` and
/// `SerpentFormPower`.
///
/// # What the DLL does (v0.111.0, sha256 `9cb4f1ad…`)
///
/// Each body calls `Rng.NextItem(CombatTargets, HittableEnemies)` without
/// reading `IsEnding`: `Kusarigama/<AfterCardPlayed>d__19` (`0x327fe0`)
/// gates only on `CombatManager.IsInProgress` (IL_003d-IL_0047) before its
/// `NextItem` at IL_00be, `HauntPower/<AfterCardPlayed>d__4` (`0x33be58`)
/// has no combat gate before IL_008d, and `SerpentFormPower/
/// <AfterCardPlayed>d__13` (`0x343ef4`) none before IL_0128. `IsInProgress`
/// holds through the whole AfterCardPlayed suffix (#3253: it is cleared only
/// from `CheckWinCondition`, after `GameAction.Execute`). `Rng.NextItem`
/// (`0x5ee74`) returns the default without drawing on an empty list
/// (IL_0024-IL_0030), and otherwise draws once (`NextInt` IL_0034), which is
/// [`damage::roll_target`].
///
/// So the draw is ungated, and this helper does not read `history.over`.
/// Rust sets `history.over` during a play only when no monster is alive
/// (`damage::finish_monster_death_body`'s terminal) or on owner death, which
/// deactivates the player's hooks first, so the no-target arm is the exact
/// no-draw path. Native keeps no secondary enemy alive past the last
/// primary's death either: `<KillWithoutCheckingWinCondition>d__15`
/// (`0x3ebe90`) kills every living teammate when all of them are secondary
/// (IL_05be-IL_060e), which `finish_secondary_death_cascade` ports.
///
/// # What refuses
///
/// A pick while [`damage::damage_combat_is_ending`] holds. The body would
/// then await `CreatureCmd.Damage` (`<Damage>d__12` `0x3e96c8`), which has no
/// ending check on entry and skips only `CombatHistory.DamageReceived`
/// under IsEnding (IL_072c-IL_0742) while running every hook. That pipeline
/// under IsEnding is not represented, so it refuses by name
/// ([`EngineRefusal::EndingDamageNotModeled`]) after the draw; the outer
/// action clone keeps the refusal atomic.
pub(crate) fn roll_ungated_after_card_played_target(
    state: &mut HotState,
    site: &'static str,
) -> Result<Option<usize>, EngineRefusal> {
    let target = damage::roll_target(state)?;
    if target.is_some() && damage::damage_combat_is_ending(state) {
        return Err(EngineRefusal::EndingDamageNotModeled(site));
    }
    Ok(target)
}

/// Seed the native creature-id counter once the combat-start pets exist
/// (#3039).
///
/// # What the DLL does (v0.111.0, sha256 `9cb4f1ad…`)
///
/// `CombatState::AttachCreature` (RVA `0x13718f`) gives every attached
/// creature `CombatId = _nextCreatureId++` (`IL_0009`-`IL_0022`). The order
/// at combat start is:
///
/// 1. the player, `CombatRoom/<EnterInternal>d__40::MoveNext` (`0x31078c`)
///    `IL_007d` `CombatState::AddPlayer` (`0x137059`, `AttachCreature` at
///    `IL_0008`): id 0 in a solo fight;
/// 2. the encounter's enemies, `CombatRoom/<StartCombat>d__46::MoveNext`
///    (`0x310bf0`) `IL_00fc` `CombatState::CreateCreature` (`0x137074`,
///    `AttachCreature` at `IL_007e`), in `MonstersWithSlots` order: ids
///    1..=n, which are this crate's monster uids 0..n;
/// 3. the pets, from `Hook.BeforeCombatStart`
///    (`CombatManager/<StartCombatInternal>d__98::MoveNext` `0x3f71b0`
///    `IL_023b`): Byrdpip (`<SummonPet>d__18` `0x3211e4` `IL_0023`) and
///    Pael's Legion (`<SummonPet>d__38` `0x32cc88` `IL_0023`) each
///    `PlayerCmd.AddPet`, which creates the pet
///    (`<AddPet>d__14`1::MoveNext` `0x3ee68c` `IL_0054`
///    `ICombatState::CreateCreature`), and a first `OstyCmd.Summon` (Bound
///    Phylactery, Phylactery Unbound; `<Summon>d__0` `0x3ee040` `IL_01e9`)
///    does too.
///
/// So the first spawned enemy takes uid `n + pets`, and native .mcr target
/// ids confirm it: in a Byrdpip Phrog Parasite fight the Phrog is target 1,
/// the pet 2 and the four Wrigglers 3..=6.
///
/// # What this writes
///
/// The counter only when a pet exists: with none, `max(uid) + 1` already is
/// the counter and the zero default keeps every pet-less document
/// byte-identical. The order among the pets is unobservable here, since
/// only their count moves the counter.
fn seed_creature_uid_counter(catalog: &Catalog, state: &mut HotState) -> Result<(), EngineRefusal> {
    if state.fanouts.next_creature_uid() != 0 {
        return Ok(());
    }
    let pets = u32::from(catalog.hooks().owns(RelicId::RelicByrdpip))
        + u32::from(catalog.hooks().owns(RelicId::RelicPaelsLegion))
        + u32::from(state.fanouts.pet().has_die_for_you());
    if pets == 0 {
        return Ok(());
    }
    let enemies = state
        .monsters
        .iter()
        .map(|monster| monster.uid)
        .max()
        .map_or(Some(0), |uid| uid.checked_add(1))
        .ok_or(EngineRefusal::CounterOverflow("creature id counter"))?;
    let next = enemies
        .checked_add(pets)
        .ok_or(EngineRefusal::CounterOverflow("creature id counter"))?;
    state.fanouts.set_next_creature_uid(next);
    Ok(())
}

/// Vambrace's combat-start reset (#2827).
///
/// `Vambrace::BeforeCombatStart` (v0.111.0 RVA `0x9d7c2`) is synchronous and
/// unconditional: `ldnull; set_TriggeringCard` at `IL_0001`-`IL_0003`,
/// `ldc.i4.0; set_BlockGainedThisCombat` at `IL_0008`-`IL_000a`, and
/// `RelicModel::set_Status(1)` (display) at `IL_000f`-`IL_0011`. The two
/// fields are what `Vambrace::ModifyBlockMultiplicative` (`0x9d7e0`) reads to
/// double the owner's first card block of the combat, and
/// `AfterModifyingBlockAmount` (`0x9d846`) / `AfterCardPlayed` (`0x9d880`) are
/// their only in-combat writers — both already ported in `engine::damage`.
///
/// So the hook's effect on combat state is "the doubling is available and no
/// card is latched": `vambrace_available = true`, `vambrace_trigger_uid =
/// None`. The oracle writes the same thing into the constructed `State`,
/// `vambrace_available="RELIC.VAMBRACE" in relics` with the trigger at its
/// `-1` default (frozen Python `start_combat`, deleted #2827). The writes touch only the relic's own
/// fields, so their position among the other `BeforeCombatStart` listeners is
/// unobservable; this runs beside the other hand-authored pass-1 bodies. Before
/// #2827 the opening gated the relic, because nothing wrote the flag and the
/// opened document would have carried `vambrace_available = false`: an owned
/// Vambrace that never doubled anything.
fn vambrace_before_combat_start(catalog: &Catalog, state: &mut HotState) {
    if !catalog.hooks().owns(RelicId::RelicVambrace) {
        return;
    }
    state.fanouts.set_vambrace_available(true);
    state.fanouts.set_vambrace_trigger_uid(None);
    crate::coverage::record_relic(RelicId::RelicVambrace);
}

/// Byrdpip's and Pael's Legion's combat-start pets (#2827).
///
/// # What the DLL does (v0.111.0, sha256 `9cb4f1ad…`)
///
/// * `Byrdpip/<BeforeCombatStart>d__17::MoveNext` (RVA `0x321120`) calls
///   `Byrdpip::SummonPet` at `IL_001e` unconditionally, and
///   `<SummonPet>d__18::MoveNext` (`0x3211e4`) is
///   `PlayerCmd::AddPet<Byrdpip>(Owner)` at `IL_0023`.
/// * `PaelsLegion/<BeforeCombatStart>d__32::MoveNext` (`0x32cbc4`) is the same
///   shape: `SummonPet` at `IL_001e`, whose `<SummonPet>d__38::MoveNext`
///   (`0x32cc88`) is `PlayerCmd::AddPet<PaelsLegion>(Owner)` at `IL_0023`.
/// * Both pet monsters pin `get_MinInitialHp`/`get_MaxInitialHp` to
///   `ldc.i4 9999` (`Byrdpip` `0xb08b8`/`0xb08bf`, `PaelsLegion`
///   `0xbb33f`/`0xbb346`), so creation rolls no HP. Byrdpip's move machine is
///   a single self-looping `NOTHING_MOVE` (`0xb091c`). Neither relic reads an
///   RNG stream at this hook. The only `Rng` in either body is Byrdpip's skin
///   pick in `AfterObtained` (`0x320ecc` `IL_0036`), and that runs on pickup,
///   outside combat.
/// * `PlayerCmd/<AddPet>d__15::MoveNext` (`0x3ee7b0`) ends in `CreatureCmd::Add`,
///   and `CreatureCmd/<Add>d__2::MoveNext` (`0x3e9234`) fires
///   `Hook::AfterCreatureAddedToCombat` at `IL_01f0`. Its only relic listeners
///   at v0.111.0 are Philosopher's Stone and Fur Coat. Both reject the owner's
///   side (`combat_sim.py` `_philosophers_stone_after_opponent_added` and
///   `_fur_coat_after_opponent_added`: "player/Osty additions never enter"),
///   and both are still in `entry::opening`'s room-entry gate anyway.
///
/// # Why nothing is written
///
/// The pet is a projection of ownership in this crate. `boundary.rs`'s
/// `PlayerSlot::RelicPets` writes one `[kind, 0, 9999, 9999]` row per owned
/// relic in the oracle's sorted order, and `from_canonical` refuses a
/// document whose roster disagrees with ownership. So the opening's pre-hook
/// document already carries the roster (`entry::opening::pre_hook_document`
/// says why that is exact), and Pael's Legion's per-fight
/// `paels_legion_cooldown = 0` is seeded there too (frozen Python `start_combat`, deleted #2827).
/// There is no hot state to write. This function records that the hook ran,
/// at the point where the oracle runs it:
/// `start_combat`, after `_fire_relic_templates(s,
/// "BeforeCombatStart")` and before the Osty summons (`start_combat`). The
/// oracle sets the pets unconditionally, so this does not gate on
/// `history.over`.
///
/// Pael's Legion's other opening hook is its turn-1 `AfterSideTurnStart`
/// (`<AfterSideTurnStart>d__36::MoveNext`, `0x32c9d4`). It already runs in
/// `relics::after_side_turn_start_late`. On turn 1 the cooldown is 0, so
/// native's unconditional `Cooldown - 1` (`IL_0056`-`IL_005b`) goes to -1.
/// The oracle's canonical 0 stands for every native value <= 0, so nothing
/// changes. `GetPet<PaelsLegion>` (`IL_0081`) has a target, because the pet
/// was added here.
fn passive_relic_pets_before_combat_start(catalog: &Catalog) {
    for relic in [RelicId::RelicByrdpip, RelicId::RelicPaelsLegion] {
        if catalog.hooks().owns(relic) {
            crate::coverage::record_relic(relic);
        }
    }
}

/// Petrified Toad's `BeforeCombatStartLate` Shaped Rock (#2827).
///
/// # What the DLL does (v0.111.0, sha256 `9cb4f1ad…`)
///
/// `PetrifiedToad::BeforeCombatStartLate` (RVA `0x9939c`) is the relic's only
/// combat override. Its body, `<BeforeCombatStartLate>d__4::MoveNext`
/// (`0x32dd04`), is `RelicModel::Flash` (`IL_001e`) and then an
/// unconditional `PotionCmd::TryToProcure<PotionShapedRock>(Owner)`
/// (`IL_0024`-`IL_0029`), awaited, with its `PotionProcureResult` discarded
/// (`IL_007b`-`IL_0080`). There is no ending test in the body, and none in
/// `Hook/<BeforeCombatStart>d__18::MoveNext`'s second walk (`0x3d2574`,
/// `IL_0103`-`IL_0133`), so this does not gate on `history.over`; nor does
/// the oracle.
///
/// `TryToProcure` is [`potions::procure_potion`].
/// `PotionCmd/<TryToProcure>d__1::MoveNext` (`0x3ef588`) returns a failed
/// result when `Hook::ShouldProcurePotion` (`IL_004b`, Sozu's veto) says no,
/// else calls `Player::AddPotionInternal(potion, -1)` (`IL_008a`-`IL_008b`,
/// the first empty slot; a full belt fails), and only on success fires
/// `Hook::AfterPotionProcured` (`IL_0151`, Belt Buckle's unlatch). Between
/// those it appends to the map point's `PotionChoices` history and plays UI,
/// neither of which is combat state. That is the oracle's
/// `_try_procure_generated_potion` (frozen Python, deleted #2827), reached
/// through `_apply_petrified_toad_entry`.
///
/// # Where it runs
///
/// The oracle runs it after the ordinary `BeforeCombatStart` listeners, the
/// relic pets and the Osty summons (frozen Python `start_combat`, deleted #2827), which is
/// this function's position in [`fire_before_combat_start`]. The belt's
/// capacity must be exact for "first empty slot" to mean the game's slot;
/// `entry::opening` refuses a Toad fight whose belt is not
/// (`OpeningRefusal::PotionBeltNotExact`), as the oracle does. No RNG stream is
/// read: the potion is fixed, not generated.
///
/// `DELICATE_FROND`, the other procurer on the same combat start, runs in the
/// ordinary pass ([`delicate_frond_before_combat_start`]) and so always
/// before this: a belt the Frond filled leaves the Toad's procurement to fail.
/// `BELT_BUCKLE` (whose latch a procurement clears) is still refused in
/// `entry::opening`'s room-entry gate.
fn petrified_toad_before_combat_start_late(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !catalog.hooks().owns(RelicId::RelicPetrifiedToad) {
        return Ok(());
    }
    potions::procure_potion(state, PotionId::PotionShapedRock, events)?;
    crate::coverage::record_relic(RelicId::RelicPetrifiedToad);
    Ok(())
}

/// Delicate Frond's `BeforeCombatStart` belt fill (#3533).
///
/// The body and its IL are [`potions::delicate_frond_before_combat_start`].
/// Delicate Frond is not a template relic, so like the Toad it has no
/// compiled subscriber and is called by name from catalog ownership.
///
/// # Where it runs
///
/// It is an ordinary `BeforeCombatStart` listener, so it belongs to the first
/// walk of `Hook/<BeforeCombatStart>d__18::MoveNext` (`0x3d2574`,
/// `IL_002a`-`IL_00be`) and completes before the `…Late` walk starts. Native
/// orders it among the other first-walk listeners by relic acquisition,
/// which this function does not reproduce: it runs after all of them. That
/// is exact because the body reads and writes only the belt and the
/// `CombatPotionGeneration` stream, and the one first-walk peer that touches
/// either, `BELT_BUCKLE` (`AfterPotionProcured` unlatches its Dexterity), is
/// refused by `entry::opening`'s room-entry gate.
fn delicate_frond_before_combat_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !catalog.hooks().owns(RelicId::RelicDelicateFrond) {
        return Ok(());
    }
    potions::delicate_frond_before_combat_start(state, catalog, events)?;
    crate::coverage::record_relic(RelicId::RelicDelicateFrond);
    Ok(())
}

pub(crate) fn fire_before_combat_start(
    catalog: &Catalog,
    state: &mut HotState,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    // Pass 1: every ordinary listener, to completion.
    fire_hook(catalog, HookEvent::BeforeCombatStart, state, events)?;
    passive_relic_pets_before_combat_start(catalog);
    phylactery_before_combat_start(catalog, state)?;
    seed_creature_uid_counter(catalog, state)?;
    vambrace_before_combat_start(catalog, state);
    delicate_frond_before_combat_start(catalog, state, events)?;
    // Pass 2: the distinct `…Late` walk. `PETRIFIED_TOAD` is its only
    // v0.111.0 subscriber (`0x9939c`); the base body (`0x7a00f`) is
    // `Task::get_CompletedTask; ret`, so every other listener contributes
    // nothing and the pass is exactly the Toad's body.
    petrified_toad_before_combat_start_late(catalog, state, events)
}

/// The opening's first `begin_player_turn` — the deal.
///
/// `combat_sim.start_combat` ends with `begin_player_turn(s)` (frozen Python, deleted #2827), and the state it returns *is* the canonical entry
/// document every consumer of this engine has been handed until now. The
/// opening ([`crate::entry::opening`]) now produces that state itself, so this
/// is the one production entry point into the turn-start walk that is not
/// reached from an `end_turn`.
///
/// It is a named wrapper rather than a re-export because the two are different
/// claims: `turn::begin_player_turn` starts *a* player turn, and this starts
/// *the first* one, on a state no action has touched.
pub fn deal_opening_hand(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    turn::begin_player_turn(state, catalog, events)
}

/// [`deal_opening_hand`] for an opening whose pre-hook Draw is still in
/// shuffled order (#3404; `entry::opening::defers_turn_one_fixup` carries
/// the IL): the turn-one Imbued/Innate fixup runs after the BeforeHandDraw
/// relics, where `SetupPlayerTurn` runs it.
///
/// The deal must reach the hand draw. A turn start that parks before it
/// (a Toolbox choice) would leave the fixup unapplied in a published state,
/// so it refuses by name instead.
pub fn deal_opening_hand_deferring_turn_one_fixup(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    struct Disarm;
    impl Drop for Disarm {
        fn drop(&mut self) {
            DEFERRED_TURN_ONE_FIXUP.with(|armed| armed.set(false));
        }
    }
    if state.turn != 1 {
        return Err(EngineRefusal::MalformedArgs(
            "deferred turn-one fixup outside turn one",
        ));
    }
    DEFERRED_TURN_ONE_FIXUP.with(|armed| armed.set(true));
    let _disarm = Disarm;
    turn::begin_player_turn(state, catalog, events)?;
    if DEFERRED_TURN_ONE_FIXUP.with(std::cell::Cell::get) {
        return Err(EngineRefusal::MalformedArgs(
            "deferred turn-one fixup before the hand draw",
        ));
    }
    Ok(())
}

/// Total the `max_energy` / `draw_bonus` contributions of one modifier hook
/// (`_relic_modifier_total`).
///
/// A modifier hook is *read*, not applied: `_player_max_energy` and the hand
/// draw sum their subscribers instead of letting them mutate. With no
/// subscriber the total is zero — the same value Python computes over an empty
/// relic list, which is why the slice's arithmetic is unchanged by this wiring.
pub(crate) fn modifier_total(
    catalog: &Catalog,
    event: HookEvent,
    state: &HotState,
) -> Result<i16, EngineRefusal> {
    crate::coverage::record_hook(event);
    relics::modifier_total(catalog, event, state)
}

fn replay_selection_actions_into(
    state: &HotState,
    catalog: &Catalog,
    buffer: &mut LegalActionBuffer,
) -> Result<(), EngineRefusal> {
    buffer.candidates.clear();
    buffer
        .candidates
        .extend_from_slice(state.piles.get(PileId::Hand).as_slice());
    selection::sort_physical_card_pick_payloads_in_place(state, catalog, &mut buffer.candidates)?;
    // Python's exact singleton `_pick_subsets` sorts its uid-sensitive
    // candidates by complete payload, then its incremental subset expansion
    // publishes the singletons in reverse. The sort is stable for payload-
    // equal siblings, so that final reversal also reverses live-Hand order.
    buffer
        .actions
        .extend(buffer.candidates.iter().rev().map(|card| Action::Select {
            answer: SelectionAnswer::CardUid(card.uid),
        }));
    Ok(())
}

/// Caller-owned storage for legal-action enumeration.
///
/// Every vector is retained across calls and cleared before the next state is
/// examined. One buffer belongs to one search worker; it is neither global nor
/// thread-local, and the returned action slice prevents a refill while a
/// caller still borrows it.
#[derive(Debug, Default)]
pub struct LegalActionBuffer {
    actions: Vec<Action>,
    alive: Vec<u8>,
    targets: Vec<u8>,
    seen: Vec<usize>,
    rest: Vec<(usize, HotCard)>,
    candidates: Vec<HotCard>,
    selected_positions: Vec<usize>,
    /// The engine refusal that cut the last enumeration short, if one was
    /// named (#2985). [`legal_actions_checked`] reports it.
    refusal: Option<EngineRefusal>,
}

impl LegalActionBuffer {
    /// Create an empty reusable action buffer.
    pub fn new() -> Self {
        Self::default()
    }

    fn clear(&mut self) {
        self.actions.clear();
        self.alive.clear();
        self.targets.clear();
        self.seen.clear();
        self.rest.clear();
        self.candidates.clear();
        self.selected_positions.clear();
        self.refusal = None;
    }
}

fn append_selection_ordinals(actions: &mut Vec<Action>, count: u32) {
    actions.extend((0..count).map(|option| Action::Select {
        answer: SelectionAnswer::OptionIndex(option),
    }));
}

fn push_legal_action(
    actions: &mut Vec<Action>,
    attack_boundary: &mut usize,
    is_attack: bool,
    action: Action,
) {
    if is_attack {
        actions.insert(*attack_boundary, action);
        *attack_boundary += 1;
    } else {
        actions.push(action);
    }
}

/// Current-build potion target partition used by Python's legal enumerator.
/// `PotionModel.IsValidTarget` is v0.111.0 RVA `0x8328c`; these eight models
/// have enemy-only targets, while every other manually usable known potion is
/// represented by `None` at the solo action boundary.
fn potion_targets_enemy(potion: PotionId) -> bool {
    matches!(
        potion,
        PotionId::WeakPotion
            | PotionId::FirePotion
            | PotionId::VulnerablePotion
            | PotionId::BeetleJuice
            | PotionId::PoisonPotion
            | PotionId::PotionOfDoom
            | PotionId::PowderedDemise
            | PotionId::PotionShapedRock
    )
}

fn validate_potion_action(
    state: &HotState,
    catalog: &Catalog,
    slot: u8,
    target: Option<u8>,
) -> Result<PotionId, EngineRefusal> {
    if !potions::potion_belt_state_is_exact(state, catalog) {
        return Err(EngineRefusal::MalformedArgs("potion belt state"));
    }
    let potion = state
        .fanouts
        .potion_slots()
        .get(usize::from(slot))
        .copied()
        .flatten()
        .ok_or(EngineRefusal::BadPotionSlot(slot))?;
    if !potions::is_supported(potion) {
        // Validate the public capability before any target/provenance work.
        // In particular, R52D1 keeps Colorless Potion's private generation
        // machinery dormant while its recursive Entropy closure is open.
        return Err(EngineRefusal::PotionsNotModeled);
    }
    if potion == PotionId::FairyInABottle {
        // Usage.Passive: only the authenticated player-lethal path may
        // consume Fairy in a Bottle. A forged public UsePotion must refuse
        // before the slot or wrapper stack changes.
        return Err(EngineRefusal::MalformedArgs("passive potion use"));
    }
    if !potions::potion_target_roster_is_exact(state, potion) {
        return Err(EngineRefusal::MalformedArgs("potion AnyAlly target roster"));
    }
    let requires_target = potion_targets_enemy(potion);
    if target.is_some() != requires_target {
        return Err(EngineRefusal::PotionTargetMismatch {
            required: requires_target,
        });
    }
    if let Some(target) = target
        && state
            .monsters
            .get(usize::from(target))
            .is_none_or(|monster| monster.hp <= 0)
    {
        return Err(EngineRefusal::BadTarget(target));
    }
    if let Some(target) = target {
        potions::target_application_is_exact(state, potion, usize::from(target), false)?;
    }
    if potion == PotionId::BottledPotential
        && !crate::steps::defect_rare::bottled_potential_entry_is_valid(state, catalog)
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if potion == PotionId::BottledPotential {
        draw::bottled_stratagem_selection_cardinality_is_exact(state)?;
    }
    if potion == PotionId::Clarity && state.fanouts.clarity().checked_add(3).is_none() {
        return Err(EngineRefusal::CounterOverflow("clarity"));
    }
    if state.fanouts.potion_belt_buckle()
        && !state.fanouts.potion_belt_buckle_applied()
        && state
            .fanouts
            .potion_slots()
            .iter()
            .filter(|slot| slot.is_some())
            .count()
            == 1
        && state
            .powers
            .value(PowerId::Dexterity)
            .checked_add(2)
            .is_none()
    {
        return Err(EngineRefusal::CounterOverflow(
            "Belt Buckle last-potion Dexterity",
        ));
    }
    if potion == PotionId::RegenPotion {
        damage::null_applier_power_amount_changed_is_exact(state)?;
        state
            .fanouts
            .regen()
            .checked_add(5)
            .ok_or(EngineRefusal::CounterOverflow("regen"))?;
    }
    potions::manual_part_d_prerequisites_are_exact(state, catalog, potion)?;
    potions::preflight_manual_potion_rng(state, catalog, potion)?;
    Ok(potion)
}

/// The legal actions in `state`, in Python's exact enumeration order.
///
/// Order is part of the differential contract, not a heuristic detail the port
/// may re-derive: `combat_sim.legal_actions` (frozen Python, deleted #2827) enumerates **attacks
/// first, lowest-HP target first**, then the other plays, then potions, then
/// `end`, because the solver's DFS reaches its first win sooner that way. The
/// list is compared exactly and ordered.
///
/// A pending continuation is handled first. Replay continuations expose exact
/// Hand uids in their established order; generated `select` continuations
/// expose compact ordinals over the full Python-ordered subset list. No
/// variable-width answer is stored in [`Action`].
///
/// Without a pending continuation, enumeration reduces to:
///
/// 1. distinct alive targets, ascending by HP, stable in roster order, then
///    deduplicated by whole-monster identity (Python's `Monster.key()`, which
///    is auto-derived from every field — inside the slice, two monsters agree
///    on every field iff their [`crate::hot::HotMonster`]s are equal);
/// 2. hand cards in hand order, deduplicated by the key `legal_actions`
///    builds (frozen Python, deleted #2827) — which depends on
///    [`crate::hot::HotState::exact_piles`]:
///
///    * **off:** the key is the card's payload alone, so every later copy of
///      an already-offered identity is skipped;
///    * **on:** the key is the payload *plus the ordered payloads of the rest
///      of the hand*. Two equal cards still collapse when removing either
///      leaves the same remaining sequence (`[S, S, D]`), and stop collapsing
///      when it does not (`[S, D, S]` offers both).
///
///    Interned atoms stand in for payloads in both cases, which is the same
///    equivalence once the admission gate has refused every per-instance card
///    slot. Python's third form — keying on physical uid as well — needs a
///    self-return or Hopper listener, all refused content;
/// 3. `end`.
///
/// The exact-piles action tuple that `legal_actions` appends (frozen Python, deleted #2827) also
/// carries a fourth element, the hand position. It is not part of the port's
/// wire encoding: an action
/// names its card by uid, which is strictly finer, so the differential
/// discards the position rather than the port carrying two encodings.
///
/// A terminal state yields no actions. Python would still enumerate — its
/// callers check `s.over` first — so this is a deliberate, documented
/// narrowing rather than a claim about `legal_actions`; the differential never
/// queries a terminal state.
pub fn legal_actions_into<'a>(
    state: &HotState,
    catalog: &Catalog,
    buffer: &'a mut LegalActionBuffer,
) -> &'a [Action] {
    // This must precede every pending/terminal/malformed early return. A
    // caller may reuse the same buffer across unrelated states and catalogs.
    buffer.clear();
    if !cards::call_local_ethereal_provenance_is_exact(state, catalog) {
        return &buffer.actions;
    }
    if let Some(pending) = state.pending.as_deref() {
        if pending.is_relic_selection() {
            if let Ok(count) = relics::relic_selection_action_count(state, catalog) {
                append_selection_ordinals(&mut buffer.actions, count);
            }
            return &buffer.actions;
        }
        if pending.stratagem_potion_record(&state.frames).is_some()
            || pending.stratagem_draw_record(&state.frames).is_some()
        {
            if play::persisted_card_play_stack_is_exact(state, catalog).is_ok()
                && let Ok(count) = draw::stratagem_selection_action_count(state, catalog)
            {
                append_selection_ordinals(&mut buffer.actions, count);
            }
            return &buffer.actions;
        }
        if pending.potion_finish_record(&state.frames).is_some() {
            if play::persisted_card_play_stack_is_exact(state, catalog).is_ok() {
                // #2985's rule for this pick too: a refusing representative
                // count (Python's payload order over Gambler's Brew twins,
                // say) reaches `legal_actions_checked` by name.
                match potions::selection_action_count(state, catalog) {
                    Ok(count) => append_selection_ordinals(&mut buffer.actions, count),
                    Err(refusal) => buffer.refusal = Some(refusal),
                }
            }
            return &buffer.actions;
        }
        if pending.generation_potion_record(&state.frames).is_some() {
            if play::persisted_card_play_stack_is_exact(state, catalog).is_ok()
                && let Ok(count) = potions::generation_selection_action_count(state, catalog)
            {
                append_selection_ordinals(&mut buffer.actions, count);
            }
            return &buffer.actions;
        }
        if pending
            .turn_start_hand_choice_record(&state.frames)
            .is_some()
        {
            if let Ok(Some(count)) = turn::turn_start_hand_choice_action_count(state, catalog) {
                append_selection_ordinals(&mut buffer.actions, count);
            }
            return &buffer.actions;
        }
        if pending
            .foregone_before_hand_draw_record(&state.frames)
            .is_some()
        {
            if let Ok(count) = turn::foregone_selection_action_count(state) {
                append_selection_ordinals(&mut buffer.actions, count);
            }
            return &buffer.actions;
        }
        if pending.enemy_phase_record(&state.frames).is_some() {
            if !turn::kd_pending_is_exact(state)
                || catalog.requires_action_replay()
                    && !matches!(
                        state.frames.as_slice().first(),
                        Some(crate::frame::Frame::ActionReplay { .. })
                    )
            {
                return &buffer.actions;
            }
            append_selection_ordinals(&mut buffer.actions, 2);
            return &buffer.actions;
        }
        if catalog.requires_action_replay()
            && !matches!(
                state.frames.as_slice().first(),
                Some(crate::frame::Frame::ActionReplay { .. })
            )
            && state
                .frames
                .as_slice()
                .iter()
                .any(|frame| matches!(frame, crate::frame::Frame::CardPlay { .. }))
        {
            return &buffer.actions;
        }
        if play::persisted_card_play_stack_is_exact(state, catalog).is_err() {
            return &buffer.actions;
        }
        let Some((record, active)) = state.pending_card_play(pending) else {
            return &buffer.actions;
        };
        if play::persisted_card_play_context(state, catalog, record).is_err()
            && play::stale_apc_pending_selector_target(state, catalog, record, active).is_none()
        {
            return &buffer.actions;
        }
        if record.stage != crate::hot::CardPlayStage::Body
            || !matches!(
                record.source,
                crate::hot::CardPlaySource::Manual
                    | crate::hot::CardPlaySource::Auto
                    | crate::hot::CardPlaySource::Hellraiser
                    | crate::hot::CardPlaySource::Stampede
                    | crate::hot::CardPlaySource::DrawPileFlip
                    | crate::hot::CardPlaySource::SlyDiscard
                    | crate::hot::CardPlaySource::BeatDown
                    | crate::hot::CardPlaySource::Catastrophe
                    | crate::hot::CardPlaySource::Eidolon
                    | crate::hot::CardPlaySource::KnifeTrap
                    | crate::hot::CardPlaySource::Uproar
                    | crate::hot::CardPlaySource::Decisions
            )
            || record.is_power_auto
        {
            return &buffer.actions;
        }
        let Ok((_, kind)) = record.route() else {
            return &buffer.actions;
        };
        match kind {
            crate::hot::PendingSelectionKind::Replay => {
                if replay_selection_actions_into(state, catalog, buffer).is_err() {
                    buffer.actions.clear();
                };
            }
            crate::hot::PendingSelectionKind::Program => {
                let Some(spec) = catalog.spec(active.atom) else {
                    return &buffer.actions;
                };
                let Some(step_index) = record
                    .next_step
                    .checked_sub(1)
                    .and_then(|index| usize::try_from(index).ok())
                else {
                    return &buffer.actions;
                };
                // #2985: a generic pick whose option count refuses (Python's
                // payload order, say) used to leave only an empty list, which
                // a search read as a dead end and aborted on.
                match selection::option_count(state, catalog, spec, step_index) {
                    Ok(count) => append_selection_ordinals(&mut buffer.actions, count),
                    Err(refusal) => buffer.refusal = Some(refusal),
                }
            }
            crate::hot::PendingSelectionKind::Tutor => {
                buffer
                    .candidates
                    .extend_from_slice(state.piles.get(crate::hot::PileId::Draw).as_slice());
                if selection::sort_physical_card_pick_payloads_in_place(
                    state,
                    catalog,
                    &mut buffer.candidates,
                )
                .is_err()
                {
                    buffer.actions.clear();
                    return &buffer.actions;
                }
                buffer
                    .actions
                    .extend(buffer.candidates.iter().rev().map(|card| Action::Select {
                        answer: SelectionAnswer::CardUid(card.uid),
                    }));
            }
            crate::hot::PendingSelectionKind::Purity
            | crate::hot::PendingSelectionKind::SeekerStrike
            | crate::hot::PendingSelectionKind::Abundance
            | crate::hot::PendingSelectionKind::Discovery
            | crate::hot::PendingSelectionKind::Splash
            | crate::hot::PendingSelectionKind::HandCap
            | crate::hot::PendingSelectionKind::Quasar
            | crate::hot::PendingSelectionKind::Glimmer => {
                let Ok(count) = selection::special_option_count(state, catalog, pending) else {
                    return &buffer.actions;
                };
                append_selection_ordinals(&mut buffer.actions, count);
            }
        }
        return &buffer.actions;
    }
    if !state.frames.is_empty()
        && !state
            .pending
            .as_deref()
            .is_some_and(|pending| pending.enemy_phase_record(&state.frames).is_some())
    {
        return &buffer.actions;
    }
    if state.history.over {
        return &buffer.actions;
    }

    buffer.alive.extend(
        (0..state.monsters.len())
            .filter(|index| state.monsters[*index].hp > 0)
            .map(|index| index as u8),
    );
    // Python: `sorted(alive_idx, key=lambda i: s.monsters[i].hp)` — stable, so
    // equal HP keeps roster order.
    buffer
        .alive
        .sort_by_key(|index| state.monsters[*index as usize].hp);
    for &index in &buffer.alive {
        let candidate = &state.monsters[index as usize];
        if buffer
            .targets
            .iter()
            .any(|seen| state.monsters[*seen as usize] == *candidate)
        {
            continue;
        }
        buffer.targets.push(index);
    }

    // Build the final stable partition directly. The old three-vector form
    // allocated independent `attacks` and `others` buffers, then copied both
    // into `actions` on every enumeration. Inserting each attack at the end
    // of the attack prefix preserves Python's attack-first/hand-order contract
    // while leaving skills in their original relative order, with one action
    // allocation instead of three. Hand is capped at ten at the boundary, so
    // shifting the tiny suffix is bounded work rather than a hidden hot-path
    // heap trade.
    let mut attack_boundary = 0;
    let sovereign_reachable = catalog.has_sovereign_blade();
    // The positions already offered, which is Python's `seen` set of dedup
    // keys — held as positions so the key can be *recomputed* per comparison
    // instead of materialized, which keeps the enumeration allocation-free
    // over the hand.
    let hand = state.piles.get(PileId::Hand).as_slice();
    for (position, card) in hand.iter().enumerate() {
        let Some(spec) = catalog.spec(card.atom) else {
            continue;
        };
        if !play::can_play(catalog, state, *card, spec)
            || play::preflight_instanced_power_acquisition_transition(state, catalog, spec).is_err()
        {
            continue;
        }
        let duplicate = buffer.seen.iter().any(|offered| {
            hand[*offered].atom == card.atom
                && (!state.exact_piles || rest_of_hand_agrees(state, hand, *offered, position))
        });
        if duplicate {
            continue;
        }
        buffer.seen.push(position);
        let selects_exhaust = catalog
            .steps(spec)
            .first()
            .is_some_and(|step| step.kind == StepKind::ExhaustDraw);
        if selects_exhaust {
            buffer.rest.clear();
            buffer.rest.extend(
                hand.iter()
                    .copied()
                    .enumerate()
                    .filter(|(index, _)| *index != position),
            );
            if buffer.rest.is_empty() {
                // Zero candidates: `CardSelectCmd::FromHand` returns empty
                // and the body skips straight to its Draw (#2977; IL cited at
                // `steps::ironclad_uncommon::exhaust_draw`). The play is
                // still legal — it simply carries no selection.
                push_legal_action(
                    &mut buffer.actions,
                    &mut attack_boundary,
                    spec.is_attack,
                    Action::Play {
                        uid: card.uid,
                        target: None,
                        selection: SelectionRef::NONE,
                    },
                );
            } else if state.exact_piles {
                buffer.selected_positions.clear();
                for (rest_position, (_, selected)) in buffer.rest.iter().enumerate() {
                    let duplicate = buffer.selected_positions.iter().any(|offered| {
                        buffer.rest[*offered].1.atom == selected.atom
                            && rest_of_indexed_hand_agrees(
                                state,
                                &buffer.rest,
                                *offered,
                                rest_position,
                            )
                    });
                    if !duplicate {
                        buffer.selected_positions.push(rest_position);
                        push_legal_action(
                            &mut buffer.actions,
                            &mut attack_boundary,
                            spec.is_attack,
                            Action::Play {
                                uid: card.uid,
                                target: None,
                                selection: SelectionRef::new(Some(selected.uid)),
                            },
                        );
                    }
                }
            } else {
                buffer.candidates.clear();
                buffer
                    .candidates
                    .extend(buffer.rest.iter().map(|(_, card)| *card));
                buffer.candidates.sort_by_key(|candidate| {
                    catalog
                        .spec(candidate.atom)
                        .map(|candidate_spec| candidate_spec.identity)
                });
                buffer.candidates.dedup_by_key(|candidate| candidate.atom);
                for selected in &buffer.candidates {
                    push_legal_action(
                        &mut buffer.actions,
                        &mut attack_boundary,
                        spec.is_attack,
                        Action::Play {
                            uid: card.uid,
                            target: None,
                            selection: SelectionRef::new(Some(selected.uid)),
                        },
                    );
                }
            }
        } else if matches!(spec.identity.id, CardId::Shiv)
            || sovereign_reachable && matches!(spec.identity.id, CardId::SovereignBlade)
        {
            // Shiv and Sovereign Blade have live target shapes. Route by
            // identity before trusting either static metadata field so a
            // malformed row cannot escape the I5 resolver through
            // `targeted == false`. Every other card retains the original
            // catalog-only branch below, including unrelated Dynamic cards.
            let target_type = {
                let Ok(target_type) = play::effective_target_type(state, catalog, spec, card.uid)
                else {
                    continue;
                };
                target_type
            };
            if !play::target_type_requires_choice(spec, target_type) {
                push_legal_action(
                    &mut buffer.actions,
                    &mut attack_boundary,
                    spec.is_attack,
                    Action::Play {
                        uid: card.uid,
                        target: None,
                        selection: SelectionRef::NONE,
                    },
                );
            } else if target_type == CardTargetType::AnyAlly {
                if state.hp > 0 {
                    push_legal_action(
                        &mut buffer.actions,
                        &mut attack_boundary,
                        spec.is_attack,
                        Action::Play {
                            uid: card.uid,
                            target: Some(0),
                            selection: SelectionRef::NONE,
                        },
                    );
                }
                let ally = state.fanouts.multiplayer_ally();
                if state.multiplayer_ally_key == 1 && ally.alive {
                    push_legal_action(
                        &mut buffer.actions,
                        &mut attack_boundary,
                        spec.is_attack,
                        Action::Play {
                            uid: card.uid,
                            target: Some(1),
                            selection: SelectionRef::NONE,
                        },
                    );
                }
            } else {
                for index in &buffer.targets {
                    push_legal_action(
                        &mut buffer.actions,
                        &mut attack_boundary,
                        spec.is_attack,
                        Action::Play {
                            uid: card.uid,
                            target: Some(*index),
                            selection: SelectionRef::NONE,
                        },
                    );
                }
            }
        } else if spec.targeted {
            if spec.target_type == CardTargetType::AnyAlly {
                if state.hp > 0 {
                    push_legal_action(
                        &mut buffer.actions,
                        &mut attack_boundary,
                        spec.is_attack,
                        Action::Play {
                            uid: card.uid,
                            target: Some(0),
                            selection: SelectionRef::NONE,
                        },
                    );
                }
                let ally = state.fanouts.multiplayer_ally();
                if state.multiplayer_ally_key == 1 && ally.alive {
                    push_legal_action(
                        &mut buffer.actions,
                        &mut attack_boundary,
                        spec.is_attack,
                        Action::Play {
                            uid: card.uid,
                            target: Some(1),
                            selection: SelectionRef::NONE,
                        },
                    );
                }
            } else {
                for index in &buffer.targets {
                    push_legal_action(
                        &mut buffer.actions,
                        &mut attack_boundary,
                        spec.is_attack,
                        Action::Play {
                            uid: card.uid,
                            target: Some(*index),
                            selection: SelectionRef::NONE,
                        },
                    );
                }
            }
        } else {
            push_legal_action(
                &mut buffer.actions,
                &mut attack_boundary,
                spec.is_attack,
                Action::Play {
                    uid: card.uid,
                    target: None,
                    selection: SelectionRef::NONE,
                },
            );
        }
    }
    // Python iterates the authoritative nullable belt in ascending slot
    // order. Duplicate identities are deliberately not deduplicated: using
    // either physical slot changes the subsequent sparse topology.
    for (slot, potion) in state.fanouts.potion_slots().iter().copied().enumerate() {
        let Some(potion) = potion else {
            continue;
        };
        if potion == PotionId::FairyInABottle {
            // Usage.Passive: only ShouldDie may consume this potion.
            continue;
        }
        let Ok(slot) = u8::try_from(slot) else {
            buffer.actions.clear();
            return &buffer.actions;
        };
        if !potions::potion_target_roster_is_exact(state, potion) {
            // One skip ahead of *both* the enemy-targeted loop and the
            // target-less append, so an unrepresentable pick narrows every
            // target kind alike. Since #2497 no live local Osty reaches here
            // for a supported body (no supported body is AnyAlly); a live
            // teammate still does for the kind-5 bodies. The target-less arm
            // reaches the same skip through `validate_potion_action`.
            continue;
        }
        if potion_targets_enemy(potion) {
            buffer.actions.extend(
                buffer
                    .targets
                    .iter()
                    .copied()
                    .filter(|target| {
                        potions::target_application_is_exact(
                            state,
                            potion,
                            usize::from(*target),
                            false,
                        )
                        .is_ok()
                    })
                    .map(|target| Action::UsePotion {
                        slot,
                        target: Some(target),
                    }),
            );
        } else {
            let action = Action::UsePotion { slot, target: None };
            if validate_potion_action(state, catalog, slot, None).is_ok() {
                buffer.actions.push(action);
            }
        }
    }
    buffer.actions.push(Action::EndTurn);
    &buffer.actions
}

/// [`legal_actions_into`] for a driver that must never mistake a refusal for
/// a dead end (#2985).
///
/// `Err` carries the engine refusal that cut enumeration short where one is
/// named (a generic card-selection pick whose option count refuses), or
/// [`EngineRefusal::NoLegalActions`] when a live (not `history.over`) state
/// enumerates nothing for any other reason — every other enumeration failure
/// still reads as an empty list inside [`legal_actions_into`]. So a
/// nonterminal state never yields `Ok(&[])`: a caller prunes it as refused,
/// exactly as it prunes a refused transition. A terminal state yields
/// `Ok(&[])`.
pub fn legal_actions_checked<'a>(
    state: &HotState,
    catalog: &Catalog,
    buffer: &'a mut LegalActionBuffer,
) -> Result<&'a [Action], EngineRefusal> {
    let _ = legal_actions_into(state, catalog, buffer);
    if let Some(refusal) = buffer.refusal.clone() {
        return Err(refusal);
    }
    if buffer.actions.is_empty() && !state.history.over {
        return Err(EngineRefusal::NoLegalActions);
    }
    Ok(&buffer.actions)
}

/// Allocating convenience wrapper for direct callers and the differential.
/// Search loops should keep one [`LegalActionBuffer`] per worker and call
/// [`legal_actions_into`] instead.
pub fn legal_actions(state: &HotState, catalog: &Catalog) -> Vec<Action> {
    let mut buffer = LegalActionBuffer::new();
    let _ = legal_actions_into(state, catalog, &mut buffer);
    buffer.actions
}

/// Whether two live Hand uids are the same card as far as [`legal_actions`]
/// is concerned.
///
/// [`legal_actions_into`] offers ONE representative per group of physically
/// identical Hand cards, under exactly this key: equal interned atom, plus —
/// when `exact_piles` is on — agreement of the rest of the hand after removing
/// either. This mirrors that key rather than re-deriving one, so the two can
/// only be wrong together.
///
/// It exists for #2473. `apply` accepts the caller's *actual* uid (the legacy
/// path resolves it in `play_card` and never consults the list at all), which
/// is what lets a recorded human line replay: a human plays the copy they
/// clicked, not the copy enumeration happened to offer. The replay-aware path
/// tested literal membership of the deduplicated list instead, so the same
/// engine accepted or refused the same human action depending on whether the
/// deck happened to contain an action-replay card — 9 of 318 captured fights.
fn hand_uids_are_interchangeable(state: &HotState, offered: u32, requested: u32) -> bool {
    if offered == requested {
        return true;
    }
    let hand = state.piles.get(PileId::Hand).as_slice();
    let (Some(left), Some(right)) = (
        hand.iter().position(|card| card.uid == offered),
        hand.iter().position(|card| card.uid == requested),
    ) else {
        return false;
    };
    hand[left].atom == hand[right].atom
        && (!state.exact_piles || rest_of_hand_agrees(state, hand, left, right))
}

/// Whether `action` is the public action `offered` names, allowing any
/// physical uid inside the group [`legal_actions`] collapsed (see
/// [`hand_uids_are_interchangeable`]).
///
/// Only `Play` carries a deduplicated uid. `Select`, `UsePotion` and `EndTurn`
/// are compared exactly: selection ordinals and potion slots are already
/// unique, and a `Select` by card uid enumerates every candidate without
/// collapsing any.
fn action_matches_modulo_hand_dedupe(state: &HotState, offered: &Action, action: &Action) -> bool {
    match (offered, action) {
        (
            Action::Play {
                uid: offered_uid,
                target: offered_target,
                selection: offered_selection,
            },
            Action::Play {
                uid,
                target,
                selection,
            },
        ) => {
            offered_target == target
                && hand_uids_are_interchangeable(state, *offered_uid, *uid)
                && match (offered_selection.get(), selection.get()) {
                    (None, None) => true,
                    (Some(offered_selected), Some(selected)) => {
                        hand_uids_are_interchangeable(state, offered_selected, selected)
                    }
                    _ => false,
                }
        }
        _ => offered == action,
    }
}

/// Whether `action` is legal in `state` modulo the hand dedupe: some action
/// [`legal_actions`] offers names it, allowing any physical uid inside a group
/// that list collapsed onto its representative
/// ([`action_matches_modulo_hand_dedupe`]).
///
/// This is the ONE legality test for a caller-named action. The public
/// replay-aware path, the parked-root receipt minter
/// ([`install_parked_action_replay_root`]) and the receipt's loader
/// (`boundary::authenticate_action_replay`) all read it, so a receipt minted
/// on the uid the human actually clicked is admitted by the same key that
/// accepted the action hot (#3249).
///
/// Sound only because the dedup key is sound: with `exact_piles` on it
/// compares the payload chain between the two positions (atom, flags and
/// `card_states`, see [`rest_of_hand_agrees`]); with it off, every writer that
/// makes two equal-`(id, upgrade)` copies distinguishable promotes the piles
/// first (`cards::live_cards_need_exact_piles`), so equal atoms there are
/// equal payloads. This introduces no new equivalence.
pub(crate) fn action_is_legal_modulo_hand_dedupe(
    state: &HotState,
    catalog: &Catalog,
    action: &Action,
) -> bool {
    legal_actions(state, catalog)
        .iter()
        .any(|offered| action_matches_modulo_hand_dedupe(state, offered, action))
}

/// Whether removing `left` and removing `right` leave the same ordered hand.
///
/// The exact-piles half of `legal_actions`' dedup key (frozen Python, deleted #2827): Python builds
/// `tuple(tuple(c) for c in rest)` for each position and compares the tuples.
/// Comparing the two sequences element-wise is the same test without building
/// either — `rest(i)[k]` is `hand[k]` before the removal point and
/// `hand[k + 1]` after it.
fn rest_of_hand_agrees(state: &HotState, hand: &[HotCard], left: usize, right: usize) -> bool {
    let skip = |removed: usize, index: usize| hand[if index < removed { index } else { index + 1 }];
    (0..hand.len().saturating_sub(1))
        .all(|index| card_payload_agrees(state, skip(left, index), skip(right, index)))
}

/// The exact-piles dedup test for the second selected card. `hand` is the
/// original hand with the played card removed.
fn rest_of_indexed_hand_agrees(
    state: &HotState,
    hand: &[(usize, HotCard)],
    left: usize,
    right: usize,
) -> bool {
    let skip =
        |removed: usize, index: usize| hand[if index < removed { index } else { index + 1 }].1;
    (0..hand.len().saturating_sub(1))
        .all(|index| card_payload_agrees(state, skip(left, index), skip(right, index)))
}

/// Compare the physical payload Python's exact-pile key observes, excluding
/// the solver-only uid that identifies which otherwise-equal copy moved.
fn card_payload_agrees(state: &HotState, left: HotCard, right: HotCard) -> bool {
    left.atom == right.atom
        && left.flags == right.flags
        && state.card_states.get_ref(left.uid) == state.card_states.get_ref(right.uid)
}

#[cold]
#[inline(never)]
fn replay_root_action(action: &Action) -> Option<crate::hot::ActionReplayRootAction> {
    match *action {
        Action::Play {
            uid,
            target,
            selection,
        } => Some(crate::hot::ActionReplayRootAction::Play {
            uid,
            target,
            selection_uid: selection.get(),
        }),
        Action::EndTurn => Some(crate::hot::ActionReplayRootAction::EndTurn),
        Action::UsePotion { slot, target } => {
            Some(crate::hot::ActionReplayRootAction::UsePotion { slot, target })
        }
        Action::Select { .. } => None,
    }
}

#[cold]
#[inline(never)]
fn replay_answer(action: &Action) -> Option<crate::hot::ActionReplayAnswer> {
    match *action {
        Action::Select {
            answer: SelectionAnswer::CardUid(uid),
        } => Some(crate::hot::ActionReplayAnswer::CardUid(uid)),
        Action::Select {
            answer: SelectionAnswer::OptionIndex(index),
        } => Some(crate::hot::ActionReplayAnswer::OptionIndex(index)),
        _ => None,
    }
}

#[cold]
#[inline(never)]
fn install_parked_action_replay_root(
    predecessor: &HotState,
    parked: &mut HotState,
    catalog: &Catalog,
    action: &Action,
) -> Result<(), EngineRefusal> {
    let root_action = replay_root_action(action).ok_or(EngineRefusal::ContinuationNotModeled)?;
    // The same dedupe-aware test the public path applies (#3249): a recorded
    // human play names the copy that was clicked, which need not be the
    // representative `legal_actions` offers (0QQ4T432GXT6 n45 Prepared 63 vs
    // 59; PSZXJD2L0HTW n27/n47 Hidden Daggers). The receipt keeps that uid —
    // nothing canonicalizes it onto the representative — and
    // `boundary::authenticate_action_replay` re-checks it with this same
    // helper on load, so the minted document stays admissible. It carries the
    // honest refusal name (#2473) so a driver that hits it is not sent to the
    // continuation subsystem for what is a legality question.
    if !action_is_legal_modulo_hand_dedupe(predecessor, catalog, action) {
        return Err(EngineRefusal::ActionNotLegal("replay root legality"));
    }
    let predecessor = crate::boundary::HotBoundary::try_to_canonical(predecessor, catalog)
        .map_err(|_| EngineRefusal::ContinuationNotModeled)?;
    if predecessor.refusal.is_some()
        || !predecessor.continuations.is_empty()
        || predecessor.player.contains_key("pending")
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    let replay = crate::hot::ActionReplayRecord {
        predecessor_json: predecessor.canonical_json().into_bytes(),
        action: root_action,
        answers: Vec::new(),
    };
    parked
        .install_action_replay_root(&replay)
        .ok_or(EngineRefusal::ContinuationNotModeled)
}

/// Apply one action, buffering events into a caller-owned vector.
///
/// The hot form: the buffer is cleared and refilled, so a search loop that
/// keeps one `Vec` around allocates for events exactly never.
/// Keep the capability dispatch outlined so a caller's ordinary action loop
/// does not retain the replay bit or its cold destination across iterations.
#[inline(never)]
pub fn apply_action_into(
    state: &HotState,
    catalog: &Catalog,
    action: &Action,
    events: &mut Vec<Event>,
) -> Result<HotState, EngineRefusal> {
    if crate::steps::regent_uncommon::void_form_listener_is_reachable(state)
        && !crate::steps::regent_uncommon::void_form_private_state_is_exact(state)
    {
        events.clear();
        return Err(EngineRefusal::MalformedArgs("void form private state"));
    }
    if let Action::UsePotion { slot, target } = action
        && let Err(refusal) = validate_potion_action(state, catalog, *slot, *target)
    {
        events.clear();
        return Err(refusal);
    }
    let resumable_potion_action = matches!(action, Action::UsePotion { slot, .. }
        if state.fanouts.potion_slots().get(usize::from(*slot)).copied().flatten()
            .is_some_and(potions::is_resumable));
    let resumable_potion_pending = state.pending.as_deref().is_some_and(|pending| {
        pending.potion_finish_record(&state.frames).is_some()
            || pending.generation_potion_record(&state.frames).is_some()
            || pending.stratagem_potion_record(&state.frames).is_some()
            || pending.stratagem_draw_record(&state.frames).is_some()
    });
    // v111 StratagemPower.AfterShuffle MoveNext0x34688c awaits a real
    // selection even when the shuffle belongs to ordinary turn-start Draw.
    // The same Draw hooks can suspend on a selecting Hellraiser Strike.
    // Such EndTurn actions need a replay receipt before their parked Draw is
    // validated, independently of card-body Draw reachability (#2659).
    let resumable_hand_draw = matches!(action, Action::EndTurn)
        && (catalog.cardplay_draw_hook_can_suspend()
            || state.powers.value(PowerId::Stratagem) > 0
            || state.powers.value(PowerId::Foregone) > 0);
    if catalog.requires_action_replay()
        || resumable_hand_draw
        || resumable_potion_action
        || resumable_potion_pending
        || matches!(
            state.frames.as_slice().first(),
            Some(crate::frame::Frame::ActionReplay { .. })
        )
    {
        apply_public_action_with_replay(state, catalog, action, events)
    } else {
        apply_action_into_legacy(state, catalog, action, events)
    }
}

/// Certified ordinary transition path. Replay-only root discovery, TLS scope,
/// and parked-root maintenance stay out of this hot body.
#[inline(never)]
fn apply_action_into_legacy(
    state: &HotState,
    catalog: &Catalog,
    action: &Action,
    events: &mut Vec<Event>,
) -> Result<HotState, EngineRefusal> {
    events.clear();
    if !cards::call_local_ethereal_provenance_is_exact(state, catalog) {
        return Err(EngineRefusal::MalformedArgs(
            "Call of the Void local Ethereal provenance",
        ));
    }
    if matches!(
        state.frames.as_slice().first(),
        Some(crate::frame::Frame::ActionReplay { .. })
    ) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if state.history.over && !(state.pending.is_some() && matches!(action, Action::Select { .. })) {
        return Err(EngineRefusal::CombatOver);
    }
    if state.fanouts.teammate_power_pending_raw().is_some() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if !state.frames.is_empty()
        && !state
            .pending
            .as_deref()
            .is_some_and(|pending| pending.enemy_phase_record(&state.frames).is_some())
    {
        play::persisted_card_play_stack_is_exact(state, catalog)?;
    }
    if state.pending.is_some() && !matches!(action, Action::Select { .. }) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if state.pending.is_none() && matches!(action, Action::Select { .. }) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if state.pending.is_none() && !state.frames.is_empty() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if state.fanouts.void_form_end_turn_requested() && state.pending.is_none() {
        if !crate::steps::regent_uncommon::void_form_private_state_is_exact(state) {
            return Err(EngineRefusal::MalformedArgs("void form private state"));
        }
        return Err(EngineRefusal::MalformedArgs(
            "unresolved void form end-turn request",
        ));
    }
    let mut next = state.clone();
    let action_result = (|| {
        match action {
            Action::Play {
                uid,
                target,
                selection,
            } => play::play_card(&mut next, catalog, *uid, *target, selection.get(), events)?,
            Action::Select { answer } => match answer {
                SelectionAnswer::OptionIndex(ordinal)
                    if next
                        .pending
                        .as_deref()
                        .is_some_and(crate::hot::PendingSelection::is_relic_selection) =>
                {
                    relics::resume_relic_selection(&mut next, catalog, *ordinal, events)?
                }
                SelectionAnswer::OptionIndex(ordinal)
                    if next.pending.as_deref().is_some_and(|pending| {
                        pending.stratagem_potion_record(&next.frames).is_some()
                            || pending.stratagem_draw_record(&next.frames).is_some()
                    }) =>
                {
                    draw::resume_stratagem_selection(&mut next, catalog, *ordinal, events)?
                }
                SelectionAnswer::OptionIndex(ordinal)
                    if next.pending.as_deref().is_some_and(|pending| {
                        pending.enemy_phase_record(&next.frames).is_some()
                    }) =>
                {
                    turn::resume_kd_curse(&mut next, catalog, *ordinal, events)?
                }
                SelectionAnswer::OptionIndex(ordinal)
                    if next.pending.as_deref().is_some_and(|pending| {
                        pending.stratagem_potion_record(&next.frames).is_some()
                            || pending.stratagem_draw_record(&next.frames).is_some()
                    }) =>
                {
                    draw::resume_stratagem_selection(&mut next, catalog, *ordinal, events)?
                }
                SelectionAnswer::OptionIndex(ordinal)
                    if next.pending.as_deref().is_some_and(|pending| {
                        pending
                            .foregone_before_hand_draw_record(&next.frames)
                            .is_some()
                    }) =>
                {
                    turn::resume_foregone_before_hand_draw(&mut next, catalog, *ordinal, events)?
                }
                SelectionAnswer::OptionIndex(ordinal)
                    if next.pending.as_deref().is_some_and(|pending| {
                        pending
                            .turn_start_hand_choice_record(&next.frames)
                            .is_some()
                    }) =>
                {
                    turn::resume_turn_start_hand_choice(&mut next, catalog, *ordinal, events)?
                }
                SelectionAnswer::OptionIndex(ordinal)
                    if next.pending.as_deref().is_some_and(|pending| {
                        pending.generation_potion_record(&next.frames).is_some()
                    }) =>
                {
                    potions::resume_generation_selection(&mut next, catalog, *ordinal, events)?
                }
                SelectionAnswer::OptionIndex(ordinal)
                    if next.pending.as_deref().is_some_and(|pending| {
                        pending.potion_finish_record(&next.frames).is_some()
                    }) =>
                {
                    potions::resume_selection(&mut next, catalog, *ordinal, events)?
                }
                _ => play::resume_selection(&mut next, catalog, *answer, events)?,
            },
            Action::EndTurn => turn::end_player_turn(&mut next, catalog, events)?,
            Action::UsePotion { slot, target } => {
                if crate::steps::regent_uncommon::void_form_listener_is_reachable(state)
                    && !crate::steps::regent_uncommon::void_form_private_state_is_exact(state)
                {
                    return Err(EngineRefusal::MalformedArgs("void form private state"));
                }
                let potion = validate_potion_action(state, catalog, *slot, *target)?;
                potions::use_potion(&mut next, catalog, *slot, *target, potion, events)?;
            }
        }
        let _ = play::drive_until_depth(&mut next, catalog, events, 0)?;
        let rootless_tyranny =
            play::rootless_tyranny_after_card_exhausted_stack_is_exact(&next, catalog).is_ok();
        if !rootless_tyranny
            && (next.pending.is_some()
                && (next
                    .frames
                    .as_slice()
                    .iter()
                    .any(|frame| matches!(frame, crate::frame::Frame::CardPlay { .. }))
                    || next
                        .pending
                        .as_deref()
                        .is_some_and(|pending| pending.enemy_phase_record(&next.frames).is_some()))
                || next.pending.as_deref().is_some_and(|pending| {
                    pending.potion_finish_record(&next.frames).is_some()
                        || pending.generation_potion_record(&next.frames).is_some()
                        || pending.stratagem_potion_record(&next.frames).is_some()
                        || pending.stratagem_draw_record(&next.frames).is_some()
                        || (matches!(action, Action::EndTurn)
                            && pending
                                .foregone_before_hand_draw_record(&next.frames)
                                .is_some())
                }))
        {
            // A few unit-level body tests intentionally use a smaller
            // hand-built catalog; keep those execution-only continuations
            // rootless rather than minting a receipt that could never pass
            // the public boundary authenticator.
            //
            // The test is `fight_is_covered_by`, not
            // `same_content_ignoring_replay_root`. The latter is derived
            // `PartialEq` over the catalog arenas, so it also compares which
            // `CardAtom` each identity was interned as — and interning follows
            // the order a canonical document's piles are walked. This engine
            // builds ONE catalog per session, from the *entry* document, so by
            // the time a mid-fight action parks, the same fight re-interned
            // from the state in hand has drifted two ways: its identity
            // closure has narrowed as cards left the live piles, and on
            // `73WAG17274JJ@6` two identities had simply swapped atoms 8 and
            // 9, with identical identity, reachable and potential sets. The
            // old comparison read either as a different fight and skipped the
            // install, which is #2474 — 11 of 318 captured fights carrying
            // `continuations: length python=2 rust=1` against a Python that
            // installs the root unconditionally on a parked public action —
            // the `replay_root_candidate` arm of `apply_action`
            // (frozen Python, deleted #2827), which carries no catalog
            // condition at all.
            //
            // Numbering cannot matter to the receipt, because the receipt is a
            // *document*: `boundary::catalog_from_canonical` recurses into a
            // stored predecessor and builds the loaded catalog from it
            // (`boundary.rs:11298-11324`), so the pair
            // `authenticate_action_replay` compares is produced from one
            // document by construction.
            let predecessor = crate::boundary::HotBoundary::try_to_canonical(state, catalog)
                .map_err(|_| EngineRefusal::ContinuationNotModeled)?;
            let rebuilt = crate::boundary::HotBoundary::catalog_from_canonical(&predecessor)
                .map_err(|_| EngineRefusal::ContinuationNotModeled)?;
            if rebuilt.fight_is_covered_by(catalog) {
                install_parked_action_replay_root(state, &mut next, catalog, action)?;
            }
        }
        if !next
            .frames
            .continuation_store_is_valid(next.pending.as_deref())
            || next.pending.is_none() && !next.frames.is_empty()
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        Ok(())
    })();
    if let Err(refusal) = action_result {
        events.clear();
        return Err(refusal);
    }
    if next.fanouts.void_form_end_turn_requested()
        && next.frames.is_empty()
        && next.pending.is_none()
    {
        service_void_form_end_turn(&mut next, catalog, events, false)?;
    }
    draw::publish_removed_draw_objects(&mut next)?;
    Ok(next)
}

/// Ashwater/Gambler's Brew ordered answers are accepted everywhere a legal
/// answer is, but collapsed out of `legal_actions` onto their representative
/// (#2524, `potions::potion_selection_at`). Every legality check that gates a
/// Select must admit them too, or a recorded order could apply hot yet fail
/// its own replay receipt.
pub(crate) fn is_ordered_extension_select(
    state: &HotState,
    catalog: &Catalog,
    action: &Action,
) -> bool {
    matches!(
        action,
        Action::Select {
            answer: SelectionAnswer::OptionIndex(ordinal),
        } if potions::is_ordered_extension_ordinal(state, catalog, *ordinal)
    )
}

/// Cold public whole-action transaction for a catalog whose admitted source
/// closure contains Stoke. The catalog bit is setup-time/source-specific;
/// mutable provenance and the complete recursive leaf closure were proved by
/// admission before any transition. Exact legal-action membership is checked
/// before the replay-aware COW execution publishes its successor.
#[cold]
#[inline(never)]
fn apply_public_action_with_replay(
    state: &HotState,
    catalog: &Catalog,
    action: &Action,
    events: &mut Vec<Event>,
) -> Result<HotState, EngineRefusal> {
    if !is_ordered_extension_select(state, catalog, action)
        && !action_is_legal_modulo_hand_dedupe(state, catalog, action)
    {
        events.clear();
        return Err(EngineRefusal::ActionNotLegal("public action legality"));
    }
    if consume_exact_root_rehearsal_bypass(state, action) {
        return apply_action_into_replay(state, catalog, action, events);
    }
    let terminal_aware_root = state.pending.is_none()
        && state.frames.is_empty()
        && (catalog.eidolon_recursive_root_requires_rehearsal()
            || catalog.terminal_aware_root_step_reachable());
    if terminal_aware_root
        && let Err(refusal) = play::rehearse_preserving_active_plays(|| {
            let _scope = ExactRootRehearsalGuard::enter(state, *action)?;
            let mut rehearsal_events = Vec::new();
            let first =
                apply_public_action_with_replay(state, catalog, action, &mut rehearsal_events)?;
            if first.pending.is_none() {
                return if first.frames.is_empty() {
                    Ok(())
                } else {
                    Err(EngineRefusal::ContinuationNotModeled)
                };
            }
            let expected_root = match first.frames.as_slice().first().copied() {
                Some(crate::frame::Frame::ActionReplay { record }) => first
                    .frames
                    .action_replay(record)
                    .ok_or(EngineRefusal::ContinuationNotModeled)?,
                _ => return Err(EngineRefusal::ContinuationNotModeled),
            };
            if replay_root_action(action) != Some(expected_root.action) {
                return Err(EngineRefusal::ContinuationNotModeled);
            }
            rehearse_pending_replay_tree(first, catalog, &expected_root, state)
        })
    {
        events.clear();
        return Err(refusal);
    }
    apply_action_into_replay(state, catalog, action, events)
}

/// Cold replay-only transition entry used by the independently rooted import
/// authenticator. Public execution remains origin-free until the general
/// nested/plural batch driver publishes the same lifecycle.
#[cold]
#[inline(never)]
pub(crate) fn apply_action_with_replay_witness(
    state: &HotState,
    catalog: &Catalog,
    action: &Action,
    events: &mut Vec<Event>,
) -> Result<HotState, EngineRefusal> {
    apply_action_into_replay(state, catalog, action, events)
}

/// Exhaustively rehearse every legal answer below one already-parked public
/// action while retaining its exact replay root.
///
/// This is the shared cold proof used by Distilled and by card bodies whose
/// post-Draw suffix is conditional on the selected descendant surviving.  A
/// completed leaf must consume the complete continuation stack.  Repeated
/// states on one path are refused as live recurrence; repeated states reached
/// by independent branches are memoized by the complete semantic `HotState`.
#[cold]
#[inline(never)]
pub(crate) fn rehearse_pending_replay_tree(
    first: HotState,
    catalog: &Catalog,
    expected_root: &ActionReplayRecord,
    predecessor: &HotState,
) -> Result<(), EngineRefusal> {
    fn action_replay_root_count(state: &HotState) -> usize {
        state
            .frames
            .as_slice()
            .iter()
            .filter(|frame| matches!(frame, crate::frame::Frame::ActionReplay { .. }))
            .count()
    }

    if first.pending.is_none() || action_replay_root_count(&first) != 1 {
        return Err(EngineRefusal::ContinuationNotModeled);
    }

    #[derive(Clone)]
    struct RehearsalNode {
        state: HotState,
        ancestors: Vec<HotState>,
        replay: ActionReplayRecord,
    }

    let mut work = vec![RehearsalNode {
        state: first,
        ancestors: vec![predecessor.clone()],
        replay: expected_root.clone(),
    }];
    let mut memoized = Vec::<HotState>::new();
    while let Some(node) = work.pop() {
        if memoized.iter().any(|seen| seen == &node.state) {
            continue;
        }
        memoized.push(node.state.clone());
        if action_replay_root_count(&node.state) != 1 {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        let replay = match node.state.frames.as_slice().first().copied() {
            Some(crate::frame::Frame::ActionReplay { record }) => node
                .state
                .frames
                .action_replay(record)
                .ok_or(EngineRefusal::ContinuationNotModeled)?,
            _ => return Err(EngineRefusal::ContinuationNotModeled),
        };
        if replay != node.replay
            || replay.predecessor_json != expected_root.predecessor_json
            || replay.action != expected_root.action
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        let actions = legal_actions(&node.state, catalog);
        if actions.is_empty() {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        for action in actions.into_iter().rev() {
            // Events are transition output only. Every future-relevant
            // counter, RNG cursor, UID and listener cursor is in HotState.
            let mut branch_events = Vec::new();
            let next = apply_action_with_replay_witness(
                &node.state,
                catalog,
                &action,
                &mut branch_events,
            )?;
            if next.pending.is_none() {
                if !next.frames.is_empty() {
                    return Err(EngineRefusal::ContinuationNotModeled);
                }
                continue;
            }
            let next_replay = match next.frames.as_slice().first().copied() {
                Some(crate::frame::Frame::ActionReplay { record }) => next
                    .frames
                    .action_replay(record)
                    .ok_or(EngineRefusal::ContinuationNotModeled)?,
                _ => return Err(EngineRefusal::ContinuationNotModeled),
            };
            let answer = replay_answer(&action).ok_or(EngineRefusal::ContinuationNotModeled)?;
            let mut expected_answers = replay.answers.clone();
            expected_answers.push(answer);
            if next_replay.predecessor_json != expected_root.predecessor_json
                || next_replay.action != expected_root.action
                || next_replay.answers != expected_answers
            {
                return Err(EngineRefusal::ContinuationNotModeled);
            }
            if next == node.state || node.ancestors.iter().any(|ancestor| ancestor == &next) {
                return Err(EngineRefusal::ContinuationNotModeled);
            }
            let mut ancestors = node.ancestors.clone();
            ancestors.push(node.state.clone());
            work.push(RehearsalNode {
                state: next,
                ancestors,
                replay: next_replay,
            });
        }
    }
    Ok(())
}

#[cold]
#[inline(never)]
fn apply_action_into_replay(
    state: &HotState,
    catalog: &Catalog,
    action: &Action,
    events: &mut Vec<Event>,
) -> Result<HotState, EngineRefusal> {
    events.clear();
    if !cards::call_local_ethereal_provenance_is_exact(state, catalog) {
        return Err(EngineRefusal::MalformedArgs(
            "Call of the Void local Ethereal provenance",
        ));
    }
    let replay_root = match state.frames.as_slice().first().copied() {
        Some(crate::frame::Frame::ActionReplay { record }) => Some(
            state
                .frames
                .action_replay(record)
                .ok_or(EngineRefusal::ContinuationNotModeled)
                .map(|replay| (record, replay))?,
        ),
        _ => None,
    };
    if catalog.requires_action_replay()
        && state
            .pending
            .as_deref()
            .is_some_and(|pending| pending.enemy_phase_record(&state.frames).is_some())
        && replay_root.is_none()
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if state.history.over && !(state.pending.is_some() && matches!(action, Action::Select { .. })) {
        return Err(EngineRefusal::CombatOver);
    }
    if state.fanouts.teammate_power_pending_raw().is_some() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if !state.frames.is_empty()
        && !state
            .pending
            .as_deref()
            .is_some_and(|pending| pending.enemy_phase_record(&state.frames).is_some())
    {
        play::persisted_card_play_stack_is_exact(state, catalog)?;
    }
    if state.pending.is_some() && !matches!(action, Action::Select { .. }) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if state.pending.is_none() && matches!(action, Action::Select { .. }) {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if state.pending.is_none() && !state.frames.is_empty() {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if replay_root.is_some()
        && (!matches!(action, Action::Select { .. })
            || !(is_ordered_extension_select(state, catalog, action)
                || legal_actions(state, catalog).contains(action)))
    {
        return Err(EngineRefusal::ContinuationNotModeled);
    }
    if state.fanouts.void_form_end_turn_requested() && state.pending.is_none() {
        if !crate::steps::regent_uncommon::void_form_private_state_is_exact(state) {
            return Err(EngineRefusal::MalformedArgs("void form private state"));
        }
        return Err(EngineRefusal::MalformedArgs(
            "unresolved void form end-turn request",
        ));
    }
    // A parked receipt-owned Draw (Centennial Puzzle, Swift) has no
    // resumable frame below it: the receipt re-executes the root action with
    // the answers as a tape (#3114, #3115).
    if let Some((_, replay)) = replay_root.as_ref()
        && puzzle::parked_draw_depth(state).is_some()
    {
        let answer = replay_answer(action).ok_or(EngineRefusal::ContinuationNotModeled)?;
        let resumed = puzzle::resume_receipt_owned_park(catalog, replay, answer, events);
        if resumed.is_err() {
            events.clear();
        }
        return resumed;
    }
    let mut next = state.clone();
    // The native action this transaction runs or resumes, for a choice
    // deferred into a queued hook action (#3387).
    let hook_scope = hook_action::scope_for(
        action,
        replay_root.as_ref().map(|(_, replay)| replay.action),
    );
    let action_result = (|| {
        let _replay_lifecycle = ReplayLifecycleGuard::enter(true)?;
        let _hook_scope = hook_action::ScopeGuard::enter(hook_scope);
        let carrier = puzzle::CarrierGuard::enter();
        let dispatched = (|| -> Result<(), EngineRefusal> {
            match action {
                Action::Play {
                    uid,
                    target,
                    selection,
                } => play::play_card(&mut next, catalog, *uid, *target, selection.get(), events)?,
                Action::Select { answer } => match answer {
                    SelectionAnswer::OptionIndex(ordinal)
                        if next
                            .pending
                            .as_deref()
                            .is_some_and(crate::hot::PendingSelection::is_relic_selection) =>
                    {
                        relics::resume_relic_selection(&mut next, catalog, *ordinal, events)?
                    }
                    SelectionAnswer::OptionIndex(ordinal)
                        if next.pending.as_deref().is_some_and(|pending| {
                            pending.generation_potion_record(&next.frames).is_some()
                        }) =>
                    {
                        potions::resume_generation_selection(&mut next, catalog, *ordinal, events)?
                    }
                    SelectionAnswer::OptionIndex(ordinal)
                        if next.pending.as_deref().is_some_and(|pending| {
                            pending.stratagem_potion_record(&next.frames).is_some()
                                || pending.stratagem_draw_record(&next.frames).is_some()
                        }) =>
                    {
                        draw::resume_stratagem_selection(&mut next, catalog, *ordinal, events)?
                    }
                    SelectionAnswer::OptionIndex(ordinal)
                        if next.pending.as_deref().is_some_and(|pending| {
                            pending.enemy_phase_record(&next.frames).is_some()
                        }) =>
                    {
                        turn::resume_kd_curse(&mut next, catalog, *ordinal, events)?
                    }
                    SelectionAnswer::OptionIndex(ordinal)
                        if next.pending.as_deref().is_some_and(|pending| {
                            pending
                                .foregone_before_hand_draw_record(&next.frames)
                                .is_some()
                        }) =>
                    {
                        turn::resume_foregone_before_hand_draw(
                            &mut next, catalog, *ordinal, events,
                        )?
                    }
                    SelectionAnswer::OptionIndex(ordinal)
                        if next.pending.as_deref().is_some_and(|pending| {
                            pending
                                .turn_start_hand_choice_record(&next.frames)
                                .is_some()
                        }) =>
                    {
                        turn::resume_turn_start_hand_choice(&mut next, catalog, *ordinal, events)?
                    }
                    SelectionAnswer::OptionIndex(ordinal)
                        if next.pending.as_deref().is_some_and(|pending| {
                            pending.potion_finish_record(&next.frames).is_some()
                        }) =>
                    {
                        potions::resume_selection(&mut next, catalog, *ordinal, events)?
                    }
                    _ => play::resume_selection(&mut next, catalog, *answer, events)?,
                },
                Action::EndTurn => turn::end_player_turn(&mut next, catalog, events)?,
                Action::UsePotion { slot, target } => {
                    if crate::steps::regent_uncommon::void_form_listener_is_reachable(state)
                        && !crate::steps::regent_uncommon::void_form_private_state_is_exact(state)
                    {
                        return Err(EngineRefusal::MalformedArgs("void form private state"));
                    }
                    let potion = validate_potion_action(state, catalog, *slot, *target)?;
                    potions::use_potion(&mut next, catalog, *slot, *target, potion, events)?;
                }
            }
            Ok(())
        })();
        carrier.settle(&mut next, dispatched)?;
        let rootless_tyranny =
            play::rootless_tyranny_after_card_exhausted_stack_is_exact(&next, catalog).is_ok();
        // A turn-start relic choice is a self-contained hook snapshot, with
        // no CardPlay frame or suspended enclosing action. Its sentinel has
        // no word-record offset to rebase beneath an ActionReplay frame.
        let rootless_relic = next
            .pending
            .as_deref()
            .is_some_and(crate::hot::PendingSelection::is_relic_selection)
            && relics::relic_pending_is_exact(&next, catalog);
        // An ordinary hand Draw can finish at an independent Tyranny /
        // Tools of the Trade turn-start choice. Preserve its existing rootless
        // ownership just as the legacy dispatcher does; only the strict
        // already-supported stack grammar can grant this exemption.
        let rootless_hand_choice = next.pending.as_deref().is_some_and(|pending| {
            pending
                .turn_start_hand_choice_record(&next.frames)
                .is_some()
        }) && play::persisted_card_play_stack_is_exact(&next, catalog)
            .is_ok();
        let mut installed_root = if replay_root.is_none()
            && next.pending.is_some()
            && !rootless_tyranny
            && !rootless_relic
            && !rootless_hand_choice
        {
            install_parked_action_replay_root(state, &mut next, catalog, action)?;
            true
        } else {
            false
        };
        let stop_depth = usize::from(replay_root.is_some() || installed_root);
        let driven = play::drive_until_depth(&mut next, catalog, events, stop_depth).map(|_| ());
        carrier.settle(&mut next, driven)?;
        // The action has finished: its queued hook action is the next ready
        // action, and it parks as an ordinary Draw rooted at this action
        // (#3387, `engine::hook_action`).
        if hook_action::publish_queued(&mut next)? && replay_root.is_none() {
            install_parked_action_replay_root(state, &mut next, catalog, action)?;
            installed_root = true;
        }
        match (replay_root.as_ref(), installed_root, next.pending.is_some()) {
            // The resumed end-turn action can finish and reach the next
            // turn's independent Mittens choice. Its sentinel is not a word
            // offset: retire the completed receipt before authenticating it.
            (Some(_), false, true)
                if next
                    .pending
                    .as_deref()
                    .is_some_and(crate::hot::PendingSelection::is_relic_selection) =>
            {
                next.frames
                    .pop_top_action_replay()
                    .ok_or(EngineRefusal::ContinuationNotModeled)?;
                if !relics::relic_pending_is_exact(&next, catalog) {
                    return Err(EngineRefusal::ContinuationNotModeled);
                }
            }
            // A receipt-owned re-execution publishes its complete tape itself
            // once every answer has been consumed (#3114).
            (Some(_), false, true) if puzzle::tape_is_installed() => {}
            (Some((record, _)), false, true) => {
                let answer = replay_answer(action).ok_or(EngineRefusal::ContinuationNotModeled)?;
                next.append_action_replay_answer(*record, answer)
                    .ok_or(EngineRefusal::ContinuationNotModeled)?;
            }
            (Some(_), false, false) => {
                next.frames
                    .pop_top_action_replay()
                    .ok_or(EngineRefusal::ContinuationNotModeled)?;
            }
            (None, true, true) | (None, false, _) => {}
            _ => return Err(EngineRefusal::ContinuationNotModeled),
        }
        if !next
            .frames
            .continuation_store_is_valid(next.pending.as_deref())
            || next.pending.is_none() && !next.frames.is_empty()
        {
            return Err(EngineRefusal::ContinuationNotModeled);
        }
        Ok(())
    })();
    if let Err(refusal) = action_result {
        events.clear();
        return Err(refusal);
    }
    if next.fanouts.void_form_end_turn_requested()
        && next.frames.is_empty()
        && next.pending.is_none()
    {
        service_void_form_end_turn(&mut next, catalog, events, true)?;
    }
    draw::publish_removed_draw_objects(&mut next)?;
    Ok(next)
}

/// Service Void Form's deferred native EndTurn only after the root action has
/// fully unwound. Inactive ordinary actions branch around this cold body;
/// play, resume, and direct turn entry authenticate the same private quotient
/// before any of their own mutation.
///
/// The state on entry is native's completed-play checkpoint:
/// `VoidForm/<OnPlay>d__5::MoveNext` RVA `0x3c69e8` IL_0131 calls
/// `PlayerCmd::EndTurn` RVA `0x13362f`, whose only effect (IL_0029) is
/// `SetReadyToEndTurn`, so the PlayCardAction finishes before the turn ends.
/// A recording caller observes it ([`native_checkpoint`], #3242).
///
/// The EndTurn that follows is the player's ordinary one: the End Turn
/// button's `EndPlayerTurnAction::ExecuteAction` RVA `0x10bd64` IL_002e
/// calls the same `PlayerCmd::EndTurn`. So in a replay-capable fight
/// (`replay` is the enclosing transaction's path and the catalog requires
/// receipts) it runs as its own public EndTurn transaction on the
/// replay path, with this depth-zero state as its receipt's predecessor: a
/// receipt-owned park inside the new turn (#3309, a History Course dupe that
/// selects) then carries the same EndTurn receipt a player-issued EndTurn
/// would. Every other fight keeps the direct call.
#[cold]
#[inline(never)]
fn service_void_form_end_turn(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
    replay: bool,
) -> Result<(), EngineRefusal> {
    if !crate::steps::regent_uncommon::void_form_private_state_is_exact(state) {
        return Err(EngineRefusal::MalformedArgs("void form private state"));
    }
    // Native's PlayCardAction checkpoint precedes the requested EndTurn
    // (`PlayerCmd::EndTurn` RVA `0x13362f` IL_0029 only marks the player
    // ready): expose this boundary to a recording caller (#3242).
    native_checkpoint::observe(
        native_checkpoint::NativeCheckpointKind::VoidFormEndTurnRequest,
        state,
    );
    state.fanouts.set_void_form_end_turn_requested(false);
    if replay && catalog.requires_action_replay() && !state.history.over {
        let mut turn_events = Vec::new();
        return match apply_action_into_replay(state, catalog, &Action::EndTurn, &mut turn_events) {
            Ok(next) => {
                *state = next;
                events.extend(turn_events);
                Ok(())
            }
            Err(error) => {
                events.clear();
                Err(error)
            }
        };
    }
    if !state.history.over
        && let Err(error) = turn::end_player_turn(state, catalog, events)
    {
        events.clear();
        return Err(error);
    }
    Ok(())
}

/// Apply one action, returning the successor state and its events.
pub fn apply_action(
    state: &HotState,
    catalog: &Catalog,
    action: &Action,
) -> Result<Transition, EngineRefusal> {
    let mut events = Vec::new();
    let state = apply_action_into(state, catalog, action, &mut events)?;
    Ok(Transition { state, events })
}

#[cfg(test)]
/// Bring a hand-built v0.111.0 combat-entry state up to the provenance a
/// real projected root carries, in the two places `HotState::at_defaults()`
/// deliberately does not.
///
/// **`inky_attack_damage`.** `at_defaults()` seeds `1`, which is a TRUE
/// value — the v0.110.1 `EnchantDamageAdditive` — and not a sentinel; the
/// current build removed the additive member and its value is `0`. Frozen
/// Python carries both, as the constants `INKY_ATTACK_DAMAGE_V1101 = 1`
/// and `INKY_ATTACK_DAMAGE = 0`, selected by build in
/// `_inky_attack_damage_for_build` (deleted #2827). The
/// default is NOT the thing to change: it mirrors the frozen Python
/// dataclass default that wire elision depends on, pinned by
/// `defaults_agree_with_the_hot_constructors`. A real v0.111.0 root
/// therefore carries an explicit `0` — the entry adapter in
/// `crate::entry::opening` emits it precisely because it differs from the
/// field default — and the Inky Shiv body itself refuses anything else.
/// Every Blade-of-Ink fixture in `steps::silent_rare` already assigns `0`
/// by hand; these fixtures did not, and it never mattered until Entropy's
/// five-class closure made `BladeOfInk` (SILENT) reachable and
/// `current-build Inky attack damage` started — correctly — refusing a
/// v0.110.1-shaped state.
///
/// **The AfterEnergyReset ledger.** `at_defaults()` starts it
/// legacy-unknown, `try_to_canonical` emits the field only when it is
/// explicit, and the rebuilt state is then refused as an ambiguous legacy
/// order as soon as a reset peer becomes reachable. This calls the same
/// adapter `from_canonical` calls for a wire `[]`, rather than writing the
/// literal: `try_to_canonical` echoes the stored ledger instead of
/// re-deriving it, so a fixture that ever holds a live reset listener is
/// rejected by the boundary instead of quietly claiming an order.
pub(crate) fn give_a_real_root_s_current_build_provenance(state: &mut crate::hot::HotState) {
    state.inky_attack_damage = 0;
    state.fanouts.initialize_after_energy_reset_order_at_entry();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boundary::HotBoundary;
    use crate::canonical::CanonicalStateV2;
    use crate::catalog::{CardIdentity, CatalogBuilder};
    use crate::hot::{
        CARD_FLAG_DEFAULT_PHYSICAL_STATE, CARD_FLAG_HEXED, CARD_FLAG_LEGACY,
        CARD_FLAG_SOVEREIGN_BLADE_STATE, HotCard, HotMonster, LEGACY_CARD_UID, MiseryToken,
        MultiplayerAllyState, PanacheInstance, PileId,
    };
    use crate::ids::{CardId, MonsterKind, PowerId};
    use crate::powers::SlotWire;

    pub(crate) const FIXTURE: &str =
        include_str!("../../fixtures/canonical_state_v2_ironclad_toadpoles.json");

    pub(crate) fn fixture() -> (HotState, Catalog) {
        let document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        admit(&document, &state, &catalog).unwrap();
        (state, catalog)
    }

    #[test]
    fn live_dupe_non_strike_attack_selection_retains_remove_route() {
        let (mut state, catalog) = fixture();
        state.energy = 3;
        state.exact_piles = true;
        state.piles.get_mut(PileId::Hand).make_mut()[0].flags |=
            CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_DUPE;
        let mut wire = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        wire.piles.get_mut("hand").unwrap()[0].id = "DAGGER_THROW".to_owned();
        let catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let state = HotBoundary::from_canonical(&wire, &catalog).unwrap();
        admit(&wire, &state, &catalog).unwrap();
        let action = Action::Play {
            uid: 0,
            target: Some(0),
            selection: SelectionRef::new(None),
        };
        assert!(legal_actions(&state, &catalog).contains(&action));
        let parked = apply_action(&state, &catalog, &action).unwrap().state;
        assert!(parked.pending.is_some());
        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt = HotBoundary::from_canonical(&wire, &catalog).unwrap();
        assert_eq!(rebuilt, parked);
        admit(&wire, &rebuilt, &catalog).unwrap();
        let mut wrong_route = wire.clone();
        wrong_route.continuations.last_mut().unwrap().fields.insert(
            "result_location".to_owned(),
            serde_json::json!(["discard", "bottom"]),
        );
        assert!(HotBoundary::from_canonical(&wrong_route, &catalog).is_err());
        for action in legal_actions(&rebuilt, &catalog) {
            let next = apply_action(&rebuilt, &catalog, &action).unwrap().state;
            assert!(next.pending.is_none());
            assert!(PileId::ALL.into_iter().all(|pile| {
                next.piles
                    .get(pile)
                    .as_slice()
                    .iter()
                    .all(|card| card.uid != 0)
            }));
        }
    }

    #[test]
    fn live_dupe_bash_juggling_clone_survives_cold_publication() {
        let (mut state, catalog) = fixture();
        state.energy = 3;
        state.exact_piles = true;
        let mut bash = state.piles.get_mut(PileId::Draw).make_mut().remove(3);
        bash.flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_DUPE;
        state.piles.get_mut(PileId::Hand).make_mut().push(bash);
        state.powers.set(PowerId::Juggling, SlotWire::Int, 1);
        state.powers.set(PowerId::JugglingAttacks, SlotWire::Int, 2);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        let wire = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let state = HotBoundary::from_canonical(&wire, &catalog).unwrap();
        admit(&wire, &state, &catalog).unwrap();
        let next = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 8,
                target: Some(0),
                selection: SelectionRef::new(None),
            },
        )
        .unwrap()
        .state;
        assert_eq!(next.next_card_uid, 11);
        assert!(PileId::ALL.into_iter().all(|pile| {
            next.piles
                .get(pile)
                .as_slice()
                .iter()
                .all(|card| card.uid != 8)
        }));
        let clone = next
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .find(|card| card.uid == 10)
            .unwrap();
        assert_ne!(clone.flags & crate::hot::CARD_FLAG_DUPE, 0);
        let wire = HotBoundary::try_to_canonical(&next, &catalog).unwrap();
        let rebuilt = HotBoundary::from_canonical(&wire, &catalog).unwrap();
        assert_eq!(rebuilt, next);
        admit(&wire, &rebuilt, &catalog).unwrap();
    }

    /// Native CardModel.GetResultLocationForCardPlay (v111 RVA 0x7e018)
    /// checks IsDupe before normal/exhaust routing. Cold publication must not
    /// turn the same object into an ordinary reusable Strike.
    #[test]
    fn live_dupe_strike_codec_preserves_identity_costs_and_removal() {
        let (mut state, catalog) = fixture();
        state.energy = 3;
        state.exact_piles = true;
        state.piles.get_mut(PileId::Hand).make_mut()[0].flags |=
            CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_DUPE;
        state.card_states.append_local_cost_modifier(
            0,
            crate::hot::LocalCostModifier {
                kind: crate::hot::LocalCostModifierKind::Set,
                amount: 0,
                expiration: crate::hot::LocalCostExpiration::UntilPlayed,
                reduce_only: false,
            },
        );
        for affliction in [
            0,
            crate::hot::CARD_FLAG_BOUND,
            crate::hot::CARD_FLAG_RINGING,
            crate::hot::CARD_FLAG_HEXED,
        ] {
            let mut variant = state.clone();
            variant.piles.get_mut(PileId::Hand).make_mut()[0].flags |= affliction;
            let wire = HotBoundary::try_to_canonical(&variant, &catalog).unwrap();
            let rebuilt = HotBoundary::from_canonical(&wire, &catalog).unwrap();
            assert_eq!(rebuilt, variant);
            assert_eq!(
                HotBoundary::try_to_canonical(&rebuilt, &catalog).unwrap(),
                wire
            );
        }
        state
            .card_states
            .set(0, crate::hot::CardInstanceState::default());
        let wire = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let rebuilt = HotBoundary::from_canonical(&wire, &rebuilt_catalog).unwrap();
        admit(&wire, &rebuilt, &rebuilt_catalog).unwrap();
        let mut with_hellraiser = rebuilt.clone();
        with_hellraiser
            .powers
            .set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut with_hellraiser);
        let guarded_wire =
            HotBoundary::try_to_canonical(&with_hellraiser, &rebuilt_catalog).unwrap();
        admit(&guarded_wire, &with_hellraiser, &rebuilt_catalog).unwrap();

        let mut ordinary = wire.clone();
        ordinary.piles.get_mut("hand").unwrap()[0].extra.clear();
        assert_ne!(wire.canonical_json(), ordinary.canonical_json());
        let action = Action::Play {
            uid: 0,
            target: Some(0),
            selection: SelectionRef::new(None),
        };
        assert!(legal_actions(&rebuilt, &rebuilt_catalog).contains(&action));
        let next = apply_action(&rebuilt, &rebuilt_catalog, &action)
            .unwrap()
            .state;
        assert_eq!(next.monsters[0].hp, 20);
        assert_eq!(next.energy, 2);
        assert!(PileId::ALL.into_iter().all(|pile| {
            next.piles
                .get(pile)
                .as_slice()
                .iter()
                .all(|card| card.uid != 0)
        }));
        assert!(next.card_states.get_ref(0).is_none());
        let next_wire = HotBoundary::try_to_canonical(&next, &rebuilt_catalog).unwrap();
        assert_eq!(
            HotBoundary::from_canonical(&next_wire, &rebuilt_catalog).unwrap(),
            next
        );

        for tail in [
            serde_json::json!([["CARD_DUPE", false]]),
            serde_json::json!([["CARD_DUPE", 1]]),
            serde_json::json!([["CARD_DUPE", true], ["CARD_DUPE", true]]),
            serde_json::json!([["CARD_DUPE", true, 0]]),
        ] {
            let mut forged = wire.clone();
            forged.piles.get_mut("hand").unwrap()[0].extra = tail.as_array().unwrap().clone();
            assert!(HotBoundary::from_canonical(&forged, &rebuilt_catalog).is_err());
        }
        let mut nonstrike = wire.clone();
        nonstrike.piles.get_mut("hand").unwrap()[0].id = "DEFEND_IRONCLAD".to_owned();
        assert!(HotBoundary::from_canonical(&nonstrike, &rebuilt_catalog).is_err());
        let mut missing_physical = wire;
        missing_physical.piles.get_mut("hand").unwrap()[0].physical_state = None;
        assert!(HotBoundary::from_canonical(&missing_physical, &rebuilt_catalog).is_err());
    }

    #[test]
    fn exact_root_rehearsal_token_is_pointer_action_once_and_unwind_scoped() {
        let state = HotState::at_defaults();
        let other = HotState::at_defaults();
        let action = Action::UsePotion {
            slot: 2,
            target: Some(1),
        };
        let _scope = ExactRootRehearsalGuard::enter(&state, action).unwrap();
        assert!(ExactRootRehearsalGuard::enter(&other, action).is_err());
        assert!(!consume_exact_root_rehearsal_bypass(&other, &action));
        assert!(!consume_exact_root_rehearsal_bypass(
            &state,
            &Action::UsePotion {
                slot: 1,
                target: Some(1),
            }
        ));
        assert!(!consume_exact_root_rehearsal_bypass(
            &state,
            &Action::UsePotion {
                slot: 2,
                target: None,
            }
        ));
        assert!(!consume_exact_root_rehearsal_bypass(
            &state,
            &Action::EndTurn
        ));
        assert!(consume_exact_root_rehearsal_bypass(&state, &action));
        assert!(!consume_exact_root_rehearsal_bypass(&state, &action));
        drop(_scope);

        {
            let _unwind_scope = ExactRootRehearsalGuard::enter(&state, action).unwrap();
        }
        assert!(!consume_exact_root_rehearsal_bypass(&state, &action));
    }

    fn entropic_fixture(
        slots: Vec<Option<PotionId>>,
        sozu: bool,
        words: [u64; 4],
        counter: u64,
    ) -> (HotState, Catalog) {
        entropic_fixture_with_relics(slots, sozu, words, counter, &[])
    }

    fn entropic_fixture_with_relics(
        slots: Vec<Option<PotionId>>,
        sozu: bool,
        words: [u64; 4],
        counter: u64,
        relics: &[RelicId],
    ) -> (HotState, Catalog) {
        let mut builder = CatalogBuilder::new();
        for id in crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109
            .into_iter()
            .chain(crate::content_tables::GENERATION_POTION_SKILL_POOL_V1091)
            .chain(crate::content_tables::GENERATION_POTION_POWER_POOL_V1091)
            .chain(crate::content_tables::ENTROPY_COLORLESS_TRANSFORM_POOL_V109)
        {
            builder.intern(plain_identity(id, 0)).unwrap();
        }
        if relics.contains(&RelicId::RelicUnceasingTop) {
            builder.mark_persistent_action_replay_required();
        }
        builder.set_relics(relics).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.reward_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.entropy_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.fully_unlocked_card_pool_epochs = true;
        give_a_real_root_s_current_build_provenance(&mut state);
        state.rng.set(
            crate::hot::RngStream::Generation,
            crate::hot::RngStreamState {
                words: [31, 32, 33, 34],
                counter: 0,
            },
        );
        state.rng.set(
            crate::hot::RngStream::PotionGeneration,
            crate::hot::RngStreamState { words, counter },
        );
        assert!(
            state
                .fanouts
                .set_potion_belt(slots, sozu, false, false, false, true)
        );
        (state, catalog)
    }

    #[test]
    fn the_fixture_enumerates_attacks_first_then_skills_then_end() {
        let (mut state, catalog) = fixture();
        assert_eq!(
            legal_actions(&state, &catalog),
            vec![
                // Strike, lowest-HP monster first (roster index 1 at 25 HP).
                Action::Play {
                    uid: 0,
                    target: Some(1),
                    selection: SelectionRef::NONE,
                },
                Action::Play {
                    uid: 0,
                    target: Some(0),
                    selection: SelectionRef::NONE,
                },
                // Defend, untargeted.
                Action::Play {
                    uid: 2,
                    target: None,
                    selection: SelectionRef::NONE,
                },
                Action::EndTurn,
            ]
        );

        // A skill physically preceding the first distinct Attack still lands
        // after the complete attack partition. The first Strike in the new
        // hand order (uid 1) is the dedup representative.
        state.piles.get_mut(PileId::Hand).make_mut().swap(0, 2);
        assert_eq!(
            legal_actions(&state, &catalog),
            vec![
                Action::Play {
                    uid: 1,
                    target: Some(1),
                    selection: SelectionRef::NONE,
                },
                Action::Play {
                    uid: 1,
                    target: Some(0),
                    selection: SelectionRef::NONE,
                },
                Action::Play {
                    uid: 2,
                    target: None,
                    selection: SelectionRef::NONE,
                },
                Action::EndTurn,
            ]
        );
    }

    #[test]
    fn legal_action_buffer_reuses_storage_without_leaking_between_states_or_catalogs() {
        let (large, large_catalog) = fixture();
        let mut buffer = LegalActionBuffer::new();
        let expected_large = legal_actions(&large, &large_catalog);
        assert_eq!(
            legal_actions_into(&large, &large_catalog, &mut buffer),
            expected_large
        );

        let mut terminal = large.clone();
        terminal.history.over = true;
        assert!(legal_actions_into(&terminal, &large_catalog, &mut buffer).is_empty());

        let mut small_builder = CatalogBuilder::new();
        let defend = plain_identity(CardId::DefendIronclad, 0);
        small_builder.intern(defend).unwrap();
        let small_catalog = small_builder.build();
        let mut small = HotState::at_defaults();
        small.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 77,
            atom: small_catalog.atom(&defend).unwrap(),
            flags: 0,
        });
        let expected_small = legal_actions(&small, &small_catalog);
        assert_eq!(
            legal_actions_into(&small, &small_catalog, &mut buffer),
            expected_small
        );
        assert_eq!(
            expected_small,
            [
                Action::Play {
                    uid: 77,
                    target: None,
                    selection: SelectionRef::NONE,
                },
                Action::EndTurn,
            ]
        );

        assert_eq!(
            legal_actions_into(&large, &large_catalog, &mut buffer),
            expected_large
        );
    }

    #[test]
    fn sovereign_blade_legal_actions_share_the_live_seeking_target_shape() {
        let identity = CardIdentity {
            id: CardId::SovereignBlade,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 31,
            atom,
            flags: crate::hot::CARD_FLAG_SOVEREIGN_BLADE_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.energy = 3;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        state.card_states.set(
            source.uid,
            crate::hot::CardInstanceState {
                damage_growth: 10,
                ..crate::hot::CardInstanceState::default()
            },
        );
        state.rng.set(
            crate::hot::RngStream::Sel,
            crate::hot::RngStreamState {
                words: [5, 6, 7, 8],
                counter: 0,
            },
        );
        for uid in [41, 42] {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
            monster.uid = uid;
            monster.slot = uid as i32;
            state.monsters_mut().push(monster);
        }

        // Reuse one caller-owned buffer across an ordinary catalog, a terminal
        // early return, and every live/malformed Sovereign shape. This proves
        // both clear-before-return and catalog capability recomputation.
        let (ordinary, ordinary_catalog) = fixture();
        let mut buffer = LegalActionBuffer::new();
        assert_eq!(
            legal_actions_into(&ordinary, &ordinary_catalog, &mut buffer),
            legal_actions(&ordinary, &ordinary_catalog)
        );
        let mut terminal = ordinary.clone();
        terminal.history.over = true;
        assert!(legal_actions_into(&terminal, &ordinary_catalog, &mut buffer).is_empty());

        let targeted = [
            Action::Play {
                uid: source.uid,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
            Action::Play {
                uid: source.uid,
                target: Some(1),
                selection: SelectionRef::NONE,
            },
            Action::EndTurn,
        ];
        assert_eq!(legal_actions(&state, &catalog), targeted);
        assert_eq!(legal_actions_into(&state, &catalog, &mut buffer), targeted);

        state.powers.set(PowerId::SeekingEdge, SlotWire::Int, 1);
        let untargeted = [
            Action::Play {
                uid: source.uid,
                target: None,
                selection: SelectionRef::NONE,
            },
            Action::EndTurn,
        ];
        assert_eq!(legal_actions(&state, &catalog), untargeted);
        assert_eq!(
            legal_actions_into(&state, &catalog, &mut buffer),
            untargeted
        );

        state.powers.set(PowerId::SeekingEdge, SlotWire::Int, 2);
        assert_eq!(legal_actions(&state, &catalog), [Action::EndTurn]);
        assert_eq!(
            legal_actions_into(&state, &catalog, &mut buffer),
            [Action::EndTurn]
        );

        // Identity routing comes before either static target metadata field.
        // A malformed Sovereign row cannot escape the live resolver by
        // claiming that it is untargeted: enumeration omits it and direct
        // execution returns the same I5 refusal without mutation.
        state.powers.set(PowerId::SeekingEdge, SlotWire::Int, 0);
        let mut malformed_catalog = catalog.clone();
        malformed_catalog
            .spec_mut_for_test(source.atom)
            .unwrap()
            .targeted = false;
        assert_eq!(legal_actions(&state, &malformed_catalog), [Action::EndTurn]);
        assert_eq!(
            legal_actions_into(&state, &malformed_catalog, &mut buffer),
            [Action::EndTurn]
        );
        let before = state.clone();
        assert!(matches!(
            play::play_card(
                &mut state,
                &malformed_catalog,
                source.uid,
                Some(0),
                None,
                &mut Vec::new(),
            ),
            Err(EngineRefusal::MalformedArgs(
                "Sovereign Blade dynamic target"
            ))
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn shiv_legal_actions_share_the_live_fan_target_shape() {
        let identity = CardIdentity {
            id: CardId::Shiv,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let source = HotCard {
            uid: 31,
            atom,
            flags: crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.energy = 3;
        state.next_card_uid = 32;
        state.piles.get_mut(PileId::Hand).make_mut().push(source);
        for uid in [41, 42] {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
            monster.uid = uid;
            monster.slot = uid as i32;
            state.monsters_mut().push(monster);
        }

        assert_eq!(
            legal_actions(&state, &catalog),
            [
                Action::Play {
                    uid: source.uid,
                    target: Some(0),
                    selection: SelectionRef::NONE
                },
                Action::Play {
                    uid: source.uid,
                    target: Some(1),
                    selection: SelectionRef::NONE
                },
                Action::EndTurn,
            ]
        );

        state.powers.set(PowerId::FanOfKnives, SlotWire::Int, 1);
        assert_eq!(
            legal_actions(&state, &catalog),
            [
                Action::Play {
                    uid: source.uid,
                    target: None,
                    selection: SelectionRef::NONE
                },
                Action::EndTurn,
            ]
        );

        state.powers.set(PowerId::FanOfKnives, SlotWire::Int, 2);
        assert_eq!(legal_actions(&state, &catalog), [Action::EndTurn]);

        state.powers.set(PowerId::FanOfKnives, SlotWire::Int, 1);
        let mut malformed_catalog = catalog.clone();
        malformed_catalog
            .spec_mut_for_test(source.atom)
            .unwrap()
            .targeted = false;
        assert_eq!(legal_actions(&state, &malformed_catalog), [Action::EndTurn]);
        let before = state.clone();
        assert!(matches!(
            play::play_card(
                &mut state,
                &malformed_catalog,
                source.uid,
                None,
                None,
                &mut Vec::new(),
            ),
            Err(EngineRefusal::MalformedArgs("Shiv dynamic target"))
        ));
        assert_eq!(state, before);
    }

    #[test]
    fn ordinary_dynamic_targeted_and_untargeted_legal_actions_are_unchanged() {
        let mut builder = CatalogBuilder::new();
        let shiv = builder
            .intern(CardIdentity {
                id: CardId::Shiv,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend = builder
            .intern(CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.energy = 3;
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: defend,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom: shiv,
                flags: 0,
            },
        ]);
        for uid in [10, 11] {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
            monster.uid = uid;
            state.monsters_mut().push(monster);
        }
        let expected = [
            Action::Play {
                uid: 2,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
            Action::Play {
                uid: 2,
                target: Some(1),
                selection: SelectionRef::NONE,
            },
            Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
            Action::EndTurn,
        ];
        assert_eq!(legal_actions(&state, &catalog), expected);
        let mut buffer = LegalActionBuffer::new();
        assert_eq!(legal_actions_into(&state, &catalog, &mut buffer), expected);
    }

    #[test]
    fn burning_pact_enumerates_one_action_per_distinct_selected_card() {
        let identities = [
            CardIdentity {
                id: CardId::BurningPact,
                upgrade: 0,
                enchantment: None,
            },
            CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            },
            CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            },
        ];
        let mut builder = CatalogBuilder::new();
        for identity in identities {
            builder.intern(identity).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        for (uid, identity) in identities.into_iter().enumerate() {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: uid as u32,
                atom: catalog.atom(&identity).unwrap(),
                flags: 0,
            });
        }

        let expected = vec![
            Action::Play {
                uid: 0,
                target: None,
                selection: SelectionRef::new(Some(2)),
            },
            Action::Play {
                uid: 0,
                target: None,
                selection: SelectionRef::new(Some(1)),
            },
            Action::Play {
                uid: 2,
                target: None,
                selection: SelectionRef::NONE,
            },
            Action::EndTurn,
        ];
        assert_eq!(legal_actions(&state, &catalog), expected);
        let mut buffer = LegalActionBuffer::new();
        assert_eq!(legal_actions_into(&state, &catalog, &mut buffer), expected);

        state.exact_piles = true;
        assert_eq!(
            legal_actions_into(&state, &catalog, &mut buffer),
            legal_actions(&state, &catalog),
            "exact-pile ExhaustDraw dedup reuses rest and selected-position scratch"
        );
    }

    /// A small Ironclad fixture for Burning Pact's zero-candidate branch
    /// (#2977): one Toadpole, the listed Hand and Draw piles, 3 energy.
    fn empty_hand_pact_fixture(
        identities: &[CardIdentity],
        hand: &[(u32, usize)],
        draw: &[(u32, usize)],
        prepare: impl FnOnce(&mut HotState),
    ) -> Option<(HotState, Catalog)> {
        let mut builder = CatalogBuilder::new();
        for identity in identities {
            builder.intern(*identity).unwrap();
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        let card = |(uid, index): (u32, usize)| HotCard {
            uid,
            atom: catalog.atom(&identities[index]).unwrap(),
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        };
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend(hand.iter().copied().map(card));
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend(draw.iter().copied().map(card));
        state.next_card_uid = hand.iter().chain(draw).map(|(uid, _)| *uid).max().unwrap() + 1;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.uid = 90;
        state.monsters_mut().push(monster);
        prepare(&mut state);
        // Round-trip through the boundary, as a search entry does, so the
        // catalog derives its own action-replay requirement (Havoc plus a
        // selecting child needs the replay root).
        // `None` only for a card whose isolated placement is not itself a
        // representable state (the table-wide regression skips those).
        let document = HotBoundary::try_to_canonical(&state, &catalog).ok()?;
        let catalog = HotBoundary::catalog_from_canonical(&document).ok()?;
        let state = HotBoundary::from_canonical(&document, &catalog).ok()?;
        Some((state, catalog))
    }

    /// #2977 witness 1 — the reported shape. Havoc is the whole Hand and
    /// flips Burning Pact off the Draw pile. Native `CardSelectCmd::FromHand`
    /// returns empty, so Pact exhausts nothing and draws its two cards; the
    /// old engine parked a Replay selection with no candidates, leaving a
    /// live state whose `legal_actions` was empty.
    #[test]
    fn havoc_autoplayed_burning_pact_over_an_empty_hand_draws_without_parking() {
        for upgrade in [0, 1] {
            let identities = [
                plain_identity(CardId::Havoc, 0),
                plain_identity(CardId::BurningPact, upgrade),
                plain_identity(CardId::StrikeIronclad, 0),
            ];
            let (state, catalog) = empty_hand_pact_fixture(
                &identities,
                &[(1, 0)],
                &[(2, 1), (3, 2), (4, 2), (5, 2), (6, 2)],
                |_| {},
            )
            .unwrap();
            let next = apply_action(
                &state,
                &catalog,
                &Action::Play {
                    uid: 1,
                    target: None,
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap()
            .state;

            assert!(next.pending.is_none(), "zero candidates park nothing");
            assert!(next.frames.is_empty());
            let hand: Vec<_> = next
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect();
            let drawn = 2 + usize::from(upgrade);
            assert_eq!(hand, [3, 4, 5][..drawn], "the Draw still runs");
            assert_eq!(
                next.piles
                    .get(PileId::Exhaust)
                    .as_slice()
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
                [2],
                "only Havoc's forced exhaust of Pact itself"
            );
            assert_eq!(next.history.card_plays_finished_combat, 2);
            assert!(!legal_actions(&next, &catalog).is_empty());
        }
    }

    /// #2977 witness 2 — the replay route through the same park site
    /// (`play_index > 0`). Burst replays a manual Burning Pact whose first
    /// body exhausted the only other Hand card and drew from empty piles, so
    /// the replay's `FromHand` again sees zero candidates.
    #[test]
    fn burst_replayed_burning_pact_over_an_emptied_hand_completes_without_parking() {
        let identities = [
            plain_identity(CardId::BurningPact, 0),
            plain_identity(CardId::StrikeIronclad, 0),
        ];
        let (state, catalog) =
            empty_hand_pact_fixture(&identities, &[(1, 0), (2, 1)], &[], |state| {
                state.powers.set(PowerId::Burst, SlotWire::Int, 1);
                crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(state);
            })
            .unwrap();
        let next = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::new(Some(2)),
            },
        )
        .unwrap()
        .state;

        assert!(next.pending.is_none());
        assert!(next.frames.is_empty());
        assert!(next.piles.get(PileId::Hand).is_empty());
        assert_eq!(
            next.piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [2]
        );
        assert_eq!(next.history.card_plays_finished_combat, 2);
        assert_eq!(legal_actions(&next, &catalog), [Action::EndTurn]);
    }

    /// #2977 witness 2b, reshaped by #3250 — the persisted twin of the
    /// replay park site (`advance_top_card_play_one`'s BeforeCardPlayed
    /// stage). Burst plus Echo Form give three plays. The second sees two
    /// candidates and parks; answering it leaves one card, so the third play
    /// is resumed from the persisted frame over a one-card Hand. Native
    /// `FromHand` (`<FromHand>d__28` RVA `0x3e7568` IL_0167-IL_018d) returns
    /// that card without a choice, so the third play exhausts it unprompted.
    #[test]
    fn third_replay_of_burning_pact_resumes_over_one_card_without_parking() {
        let identities = [
            plain_identity(CardId::BurningPact, 0),
            plain_identity(CardId::StrikeIronclad, 0),
        ];
        let (state, catalog) = empty_hand_pact_fixture(
            &identities,
            &[(1, 0), (2, 1), (3, 1), (4, 1)],
            &[],
            |state| {
                state.powers.set(PowerId::Burst, SlotWire::Int, 1);
                state.powers.set(PowerId::EchoForm, SlotWire::Int, 1);
                crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(state);
            },
        )
        .unwrap();
        let parked = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::new(Some(2)),
            },
        )
        .unwrap()
        .state;
        assert!(
            parked.pending.is_some(),
            "the second play sees two candidates"
        );
        let offered = legal_actions(&parked, &catalog);
        assert_eq!(offered.len(), 2);
        for uid in [3, 4] {
            assert!(offered.contains(&Action::Select {
                answer: SelectionAnswer::CardUid(uid),
            }));
        }

        let next = apply_action(
            &parked,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::CardUid(3),
            },
        )
        .unwrap()
        .state;
        assert!(next.pending.is_none(), "one candidate parks nothing");
        assert!(next.frames.is_empty());
        assert!(next.piles.get(PileId::Hand).is_empty());
        assert_eq!(
            next.piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [2, 3, 4]
        );
        assert_eq!(next.history.card_plays_finished_combat, 3);
        assert!(!legal_actions(&next, &catalog).is_empty());
    }

    /// #3250 witness — the inline replay site over one candidate. Burst
    /// replays a manual Burning Pact whose first body left exactly one other
    /// card in Hand. Burning Pact's prefs are non-manual with `MinSelect` 1
    /// (`BurningPact/<OnPlay>d__5` RVA `0x390434` IL_0032-IL_0038, ctor RVA
    /// `0x1397f8` IL_006a-IL_0088), so the replay's `FromHand` returns that
    /// card at IL_0167-IL_018d with no `PlayerChoice` and it is exhausted.
    #[test]
    fn burst_replayed_burning_pact_over_one_card_exhausts_it_without_parking() {
        let identities = [
            plain_identity(CardId::BurningPact, 0),
            plain_identity(CardId::StrikeIronclad, 0),
        ];
        let (state, catalog) =
            empty_hand_pact_fixture(&identities, &[(1, 0), (2, 1), (3, 1)], &[], |state| {
                state.powers.set(PowerId::Burst, SlotWire::Int, 1);
                crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(state);
            })
            .unwrap();
        let next = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::new(Some(2)),
            },
        )
        .unwrap()
        .state;

        assert!(next.pending.is_none(), "one candidate parks nothing");
        assert!(next.frames.is_empty());
        assert!(next.piles.get(PileId::Hand).is_empty());
        assert_eq!(
            next.piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [2, 3]
        );
        assert_eq!(next.history.card_plays_finished_combat, 2);
        assert_eq!(legal_actions(&next, &catalog), [Action::EndTurn]);
    }

    /// #3250 witness — the auto-play route (the Y4RZYNMTUZ9C n29 shape, with
    /// Havoc standing in for Cascade: both are the FrozenAutoBatch parent).
    /// Havoc flips Burning Pact off the Draw pile with exactly one other card
    /// in Hand. Native `FromHand` auto-resolves the non-manual `MinSelect` 1
    /// selection (IL_0167-IL_018d), so Pact exhausts that card and draws with
    /// no choice; the old engine parked a Replay selection over it.
    #[test]
    fn havoc_autoplayed_burning_pact_over_one_card_exhausts_it_without_parking() {
        for upgrade in [0, 1] {
            let identities = [
                plain_identity(CardId::Havoc, 0),
                plain_identity(CardId::BurningPact, upgrade),
                plain_identity(CardId::StrikeIronclad, 0),
            ];
            let (state, catalog) = empty_hand_pact_fixture(
                &identities,
                &[(1, 0), (7, 2)],
                &[(2, 1), (3, 2), (4, 2), (5, 2), (6, 2)],
                |_| {},
            )
            .unwrap();
            let next = apply_action(
                &state,
                &catalog,
                &Action::Play {
                    uid: 1,
                    target: None,
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap()
            .state;

            assert!(next.pending.is_none(), "one candidate parks nothing");
            assert!(next.frames.is_empty());
            let hand: Vec<_> = next
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect();
            let drawn = 2 + usize::from(upgrade);
            assert_eq!(hand, [3, 4, 5][..drawn], "the Draw still runs");
            assert_eq!(
                next.piles
                    .get(PileId::Exhaust)
                    .as_slice()
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
                [7, 2],
                "the sole Hand card, then Havoc's forced exhaust of Pact"
            );
            assert_eq!(next.history.card_plays_finished_combat, 2);
            assert!(!legal_actions(&next, &catalog).is_empty());
        }
    }

    /// #3250 control — two candidates still prompt on the auto-play route:
    /// `Count` 2 exceeds `MinSelect` 1, so `FromHand` falls through
    /// IL_0184 `bgt.s` to the selector.
    #[test]
    fn havoc_autoplayed_burning_pact_over_two_cards_still_parks() {
        let identities = [
            plain_identity(CardId::Havoc, 0),
            plain_identity(CardId::BurningPact, 0),
            plain_identity(CardId::StrikeIronclad, 0),
            plain_identity(CardId::DefendIronclad, 0),
        ];
        let (state, catalog) = empty_hand_pact_fixture(
            &identities,
            &[(1, 0), (7, 2), (8, 3)],
            &[(2, 1), (3, 2), (4, 2)],
            |_| {},
        )
        .unwrap();
        let next = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(next.pending.is_some(), "two candidates park the choice");
        assert_eq!(
            legal_actions(&next, &catalog),
            [
                Action::Select {
                    answer: SelectionAnswer::CardUid(7),
                },
                Action::Select {
                    answer: SelectionAnswer::CardUid(8),
                },
            ]
        );
    }

    /// #2977 witness 3 — the manual route. A Burning Pact alone in Hand is
    /// natively playable (`BurningPact` overrides no playability gate); its
    /// `FromHand` returns empty and the body draws. The old enumerator offered
    /// no action for it, and `play_card` refused the selection-free form.
    #[test]
    fn manual_burning_pact_alone_in_hand_is_offered_and_draws() {
        let identities = [
            plain_identity(CardId::BurningPact, 0),
            plain_identity(CardId::StrikeIronclad, 0),
        ];
        for exact_piles in [false, true] {
            let (state, catalog) = empty_hand_pact_fixture(
                &identities,
                &[(1, 0)],
                &[(2, 1), (3, 1), (4, 1)],
                |state| state.exact_piles = exact_piles,
            )
            .unwrap();
            let play = Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            };
            assert_eq!(legal_actions(&state, &catalog), [play, Action::EndTurn]);
            let mut buffer = LegalActionBuffer::new();
            assert_eq!(
                legal_actions_into(&state, &catalog, &mut buffer),
                legal_actions(&state, &catalog)
            );

            let next = apply_action(&state, &catalog, &play).unwrap().state;
            assert!(next.pending.is_none() && next.frames.is_empty());
            assert_eq!(
                next.piles
                    .get(PileId::Hand)
                    .as_slice()
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
                [2, 3]
            );
            assert!(next.piles.get(PileId::Exhaust).is_empty());
            assert_eq!(next.energy, 2);

            // A selection-free play over a non-empty rest of Hand is still the
            // construction error it was: the candidate list is not empty there.
            let (crowded, crowded_catalog) =
                empty_hand_pact_fixture(&identities, &[(1, 0), (2, 1)], &[], |state| {
                    state.exact_piles = exact_piles
                })
                .unwrap();
            assert!(!legal_actions(&crowded, &crowded_catalog).contains(&play));
            assert!(apply_action(&crowded, &crowded_catalog, &play).is_err());
        }
    }

    /// #2977 regression over the whole card table: Havoc auto-playing any
    /// printed card into an empty Hand either refuses, ends the combat, or
    /// leaves a state with at least one legal action. A parked selection with
    /// no candidates is the one outcome that is never acceptable — a search
    /// reaching it aborts with "no legal rollout actions".
    #[test]
    fn havoc_autoplay_into_an_empty_hand_never_strands_the_search() {
        let mut stranded = Vec::new();
        let mut exercised = Vec::new();
        for row in crate::catalog::variant_free_card_rows() {
            let identities = [
                plain_identity(CardId::Havoc, 0),
                plain_identity(row.id, row.upgrade),
                plain_identity(CardId::StrikeIronclad, 0),
            ];
            let Some((state, catalog)) = empty_hand_pact_fixture(
                &identities,
                &[(1, 0)],
                &[(2, 1), (3, 2), (4, 2), (5, 2)],
                |_| {},
            ) else {
                continue;
            };
            let Ok(transition) = apply_action(
                &state,
                &catalog,
                &Action::Play {
                    uid: 1,
                    target: None,
                    selection: SelectionRef::NONE,
                },
            ) else {
                continue;
            };
            exercised.push(row.name);
            let next = transition.state;
            if !next.history.over && legal_actions(&next, &catalog).is_empty() {
                stranded.push(row.name);
            }
        }
        // Non-vacuity: both Burning Pact rows reach a transition, as do the
        // other hand-selecting rows this route can flip.
        for name in ["BURNING_PACT", "BURNING_PACT+", "PURITY", "ARMAMENTS"] {
            assert!(exercised.contains(&name), "{name} never transitioned");
        }
        assert!(stranded.is_empty(), "stranded after Havoc: {stranded:?}");
    }

    fn replay_selection_state(catalog: &Catalog, hand: &[(u32, CardIdentity)]) -> HotState {
        let active_identity = plain_identity(CardId::BurningPact, 0);
        let active = HotCard {
            uid: 99,
            atom: catalog.atom(&active_identity).unwrap(),
            flags: 0,
        };
        let mut state = HotState::at_defaults();
        state.piles.get_mut(PileId::Play).make_mut().push(active);
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend(hand.iter().map(|(uid, identity)| HotCard {
                uid: *uid,
                atom: catalog.atom(identity).unwrap(),
                flags: 0,
            }));
        state.next_card_uid = hand
            .iter()
            .map(|(uid, _)| *uid)
            .chain(std::iter::once(active.uid))
            .max()
            .unwrap_or(0)
            + 1;
        let mut record = crate::hot::CardPlayRecord::pending_for_test(active.uid);
        record.selection_kind = Some(crate::hot::PendingSelectionKind::Replay);
        record.next_step = 0;
        record.spent = 1;
        record.selection_amount = 3;
        let pending = state.frames.push_pending_for_test(&record);
        state.pending = Some(std::sync::Arc::new(pending));
        state
    }

    fn plain_identity(id: CardId, upgrade: u8) -> CardIdentity {
        CardIdentity {
            id,
            upgrade,
            enchantment: None,
        }
    }

    /// `apply` accepts a non-representative duplicate on BOTH paths (#2473).
    ///
    /// [`legal_actions`] offers one representative uid per group of physically
    /// identical Hand cards. `apply_action_into_legacy` never consulted that
    /// list, so a recorded human line — which names the copy the human
    /// actually clicked — replayed fine; `apply_public_action_with_replay`
    /// tested literal membership and refused, so the same engine accepted or
    /// refused the same action depending on whether the deck happened to
    /// contain an action-replay card. Worse, it reported the refusal as
    /// `ContinuationNotModeled` on a state with no continuation at all.
    ///
    /// The assertion is that both paths reach the same successor, so the test
    /// fails on the old code twice over: the replay path refused, and the
    /// refusal it returned named the wrong subsystem.
    #[test]
    fn both_apply_paths_accept_a_non_representative_duplicate_uid() {
        let defend = plain_identity(CardId::DefendIronclad, 0);

        let mut ordinary = CatalogBuilder::new();
        ordinary.intern_reachable(defend).unwrap();
        ordinary.intern_monster(MonsterKind::Toadpole).unwrap();
        let ordinary = ordinary.build();
        assert!(!ordinary.requires_action_replay());

        let mut replaying = CatalogBuilder::new();
        replaying.intern_reachable(defend).unwrap();
        replaying
            .intern_reachable(plain_identity(CardId::Hologram, 0))
            .unwrap();
        replaying.intern_monster(MonsterKind::Toadpole).unwrap();
        replaying.mark_persistent_action_replay_required();
        let replaying = replaying.build();
        assert!(
            replaying.requires_action_replay(),
            "the second catalog must select the replay-aware apply path"
        );

        let mut successors = Vec::new();
        for catalog in [&ordinary, &replaying] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 3;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = 4;
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 50));
            let atom = catalog.atom(&defend).unwrap();
            state
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .extend([1, 2, 3].map(|uid| HotCard {
                    uid,
                    atom,
                    flags: 0,
                }));

            let offered: Vec<u32> = legal_actions(&state, catalog)
                .into_iter()
                .filter_map(|action| match action {
                    Action::Play { uid, .. } => Some(uid),
                    _ => None,
                })
                .collect();
            assert_eq!(
                offered,
                [1],
                "three identical Defends collapse to one representative"
            );

            // uid 3 is a real physical card in hand and NOT the representative.
            let action = Action::Play {
                uid: 3,
                target: None,
                selection: SelectionRef::new(None),
            };
            let next = apply_action_into(&state, catalog, &action, &mut Vec::new())
                .expect("apply accepts any member of the group");
            assert!(
                next.piles
                    .get(PileId::Hand)
                    .as_slice()
                    .iter()
                    .all(|card| card.uid != 3),
                "the uid the caller named is the one that moved"
            );
            successors.push(next);
        }

        assert_eq!(
            successors[0].piles.get(PileId::Discard).as_slice(),
            successors[1].piles.get(PileId::Discard).as_slice(),
            "the two paths must not disagree about what an action IS"
        );

        // A uid that is genuinely absent still refuses — under its own name.
        let mut absent = HotState::at_defaults();
        absent.hp = 50;
        absent.energy = 3;
        absent.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        absent.next_card_uid = 4;
        absent
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        absent.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: replaying.atom(&defend).unwrap(),
            flags: 0,
        });
        assert_eq!(
            apply_action_into(
                &absent,
                &replaying,
                &Action::Play {
                    uid: 99,
                    target: None,
                    selection: SelectionRef::new(None),
                },
                &mut Vec::new(),
            ),
            Err(EngineRefusal::ActionNotLegal("public action legality")),
            "an illegal action is illegal, not an unresumable continuation"
        );
    }

    #[test]
    fn result_exhaust_dark_draw_has_route_owner_and_round_trips() {
        let mut builder = CatalogBuilder::new();
        let entrance = builder
            .intern_reachable(plain_identity(CardId::DramaticEntrance, 0))
            .unwrap();
        let sculpting = builder
            .intern_reachable(plain_identity(CardId::SculptingStrike, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_live_dark_embrace_reachable();
        builder.mark_live_stratagem_reachable();
        let catalog = builder.build();
        assert!(catalog.requires_action_replay());

        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 5;
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        state
            .fanouts
            .register_after_side_turn_end_power(crate::hot::AfterSideTurnEndPowerToken::DarkEmbrace)
            .unwrap();
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        state.rng.set(
            crate::hot::RngStream::Rng,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
        );
        assert!(
            state
                .fanouts
                .set_potion_belt(vec![None], false, false, false, false, true)
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: entrance,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            HotCard {
                uid: 2,
                atom: sculpting,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 50);
        monster.max_hp = 50;
        state.monsters_mut().push(monster);

        let mut direct = state.clone();
        let mut direct_events = Vec::new();
        let direct_result =
            play::play_card(&mut direct, &catalog, 1, None, None, &mut direct_events);
        assert_eq!(direct_result, Ok(()), "{:#?}", direct.frames.as_slice());
        assert!(direct.pending.is_some(), "{:#?}", direct.frames.as_slice());
        let action = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };
        assert_eq!(
            install_parked_action_replay_root(&state, &mut direct, &catalog, &action),
            Ok(()),
            "{:#?}",
            direct.frames.as_slice()
        );
        assert_eq!(
            play::persisted_card_play_stack_is_exact(&direct, &catalog),
            Ok(()),
            "{:#?}",
            direct.frames.as_slice()
        );

        let parked = apply_action(&state, &catalog, &action).unwrap().state;
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::CardFinish { .. },
                    crate::frame::Frame::AfterCardExhaustedPower { .. },
                    crate::frame::Frame::Draw { .. },
                ]
            ),
            "{:#?}",
            parked.frames.as_slice()
        );
        assert!(parked.pending.is_some());
        let canonical = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&canonical).unwrap();
        let rebuilt = HotBoundary::from_canonical(&canonical, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&canonical, &rebuilt, &rebuilt_catalog), Ok(()));
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
            canonical
        );
    }

    /// A parking hand whose Dramatic Entrances are `copies`, with a third
    /// same-atom copy (uid 9) in the Discard pile. The Dark Embrace +
    /// Stratagem scaffold is
    /// `result_exhaust_dark_draw_has_route_owner_and_round_trips`'s: the Draw
    /// pile is empty, so the exhaust draw shuffles and parks on the Stratagem
    /// pick, and the play mints a replay-root receipt.
    fn parking_entrance_hand(copies: &[(u32, u16)]) -> (HotState, Catalog) {
        let mut builder = CatalogBuilder::new();
        let entrance = builder
            .intern_reachable(plain_identity(CardId::DramaticEntrance, 0))
            .unwrap();
        let sculpting = builder
            .intern_reachable(plain_identity(CardId::SculptingStrike, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_live_dark_embrace_reachable();
        builder.mark_live_stratagem_reachable();
        let catalog = builder.build();
        assert!(catalog.requires_action_replay());

        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 10;
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        state
            .fanouts
            .register_after_side_turn_end_power(crate::hot::AfterSideTurnEndPowerToken::DarkEmbrace)
            .unwrap();
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        state.rng.set(
            crate::hot::RngStream::Rng,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
        );
        assert!(
            state
                .fanouts
                .set_potion_belt(vec![None], false, false, false, false, true)
        );
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend(copies.iter().map(|&(uid, flags)| HotCard {
                uid,
                atom: entrance,
                flags,
            }));
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            HotCard {
                uid: 9,
                atom: entrance,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: sculpting,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 50);
        monster.max_hp = 50;
        state.monsters_mut().push(monster);
        (state, catalog)
    }

    fn play_uid(uid: u32) -> Action {
        Action::Play {
            uid,
            target: None,
            selection: SelectionRef::NONE,
        }
    }

    fn offered_play_uids(state: &HotState, catalog: &Catalog) -> Vec<u32> {
        legal_actions(state, catalog)
            .into_iter()
            .filter_map(|action| match action {
                Action::Play { uid, .. } => Some(uid),
                _ => None,
            })
            .collect()
    }

    /// #3249: a parked play of a NON-representative copy mints a receipt on
    /// the uid the play named, and that receipt loads.
    ///
    /// `legal_actions` offers uid 1 for the two identical Entrances; the
    /// recorded play is uid 5 (0QQ4T432GXT6 n45's Prepared 63-vs-59 and
    /// PSZXJD2L0HTW n27/n47's Hidden Daggers have this shape). Before the fix
    /// the minter's strict `contains` refused it as
    /// `ActionNotLegal("replay root legality")`. The receipt must carry 5, not
    /// the representative, so the replayed successor moves the card native
    /// moved; and `authenticate_action_replay` must admit it on load.
    #[test]
    fn parked_root_accepts_a_non_representative_duplicate_uid() {
        let (state, catalog) = parking_entrance_hand(&[
            (1, CARD_FLAG_DEFAULT_PHYSICAL_STATE),
            (5, CARD_FLAG_DEFAULT_PHYSICAL_STATE),
        ]);
        assert_eq!(
            offered_play_uids(&state, &catalog),
            [1],
            "two identical adjacent Entrances collapse to one representative"
        );
        let action = play_uid(5);
        assert!(action_is_legal_modulo_hand_dedupe(
            &state, &catalog, &action
        ));

        // The minter, directly.
        let mut direct = state.clone();
        let result = play::play_card(&mut direct, &catalog, 5, None, None, &mut Vec::new());
        assert_eq!(result, Ok(()));
        assert!(direct.pending.is_some());
        assert_eq!(
            install_parked_action_replay_root(&state, &mut direct, &catalog, &action),
            Ok(())
        );

        // Through the public path, then through the loader.
        let parked = apply_action(&state, &catalog, &action).unwrap().state;
        let Some(crate::frame::Frame::ActionReplay { record }) =
            parked.frames.as_slice().first().copied()
        else {
            panic!("{:#?}", parked.frames.as_slice());
        };
        assert_eq!(
            parked.frames.action_replay(record).unwrap().action,
            crate::hot::ActionReplayRootAction::Play {
                uid: 5,
                target: None,
                selection_uid: None,
            },
            "the receipt keeps the recorded uid, not the representative"
        );
        let hand: Vec<u32> = parked
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .map(|card| card.uid)
            .collect();
        assert_eq!(hand, [1], "the named copy is the one that moved");
        let canonical = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&canonical).unwrap();
        let rebuilt = HotBoundary::from_canonical(&canonical, &rebuilt_catalog)
            .expect("authenticate_action_replay admits the non-representative receipt");
        assert_eq!(admit(&canonical, &rebuilt, &rebuilt_catalog), Ok(()));
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
            canonical
        );
    }

    /// #3249's refusing branches: the dedupe is not a wildcard.
    ///
    /// - A same-atom copy that is NOT physically identical (different flags,
    ///   so a different payload under `exact_piles`) is not interchangeable
    ///   with the representative. `legal_actions` offers it separately, and
    ///   the equivalence must not fold it onto uid 1.
    /// - A same-atom copy outside the Hand (uid 9, in Discard) is not a member
    ///   of any offered group; the minter refuses it by its legality name.
    /// - An absent uid refuses the same way.
    /// - The loader refuses a forged receipt naming an illegal uid.
    #[test]
    fn parked_root_refuses_what_the_hand_dedupe_does_not_cover() {
        let (distinct, catalog) =
            parking_entrance_hand(&[(1, CARD_FLAG_DEFAULT_PHYSICAL_STATE), (5, 0)]);
        assert_eq!(
            offered_play_uids(&distinct, &catalog),
            [1, 5],
            "copies with different payloads are not collapsed"
        );
        assert!(!hand_uids_are_interchangeable(&distinct, 1, 5));
        assert!(!action_matches_modulo_hand_dedupe(
            &distinct,
            &play_uid(1),
            &play_uid(5)
        ));

        let (state, catalog) = parking_entrance_hand(&[
            (1, CARD_FLAG_DEFAULT_PHYSICAL_STATE),
            (5, CARD_FLAG_DEFAULT_PHYSICAL_STATE),
        ]);
        let mut parked = state.clone();
        assert_eq!(
            play::play_card(&mut parked, &catalog, 1, None, None, &mut Vec::new()),
            Ok(())
        );
        assert!(parked.pending.is_some());
        for uid in [9, 99] {
            let action = play_uid(uid);
            assert!(!action_is_legal_modulo_hand_dedupe(
                &state, &catalog, &action
            ));
            assert_eq!(
                install_parked_action_replay_root(&state, &mut parked.clone(), &catalog, &action),
                Err(EngineRefusal::ActionNotLegal("replay root legality")),
                "uid {uid}"
            );
        }

        // Forge a receipt naming the Discard-pile copy: the loader refuses it.
        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        parked
            .install_action_replay_root(&ActionReplayRecord {
                predecessor_json: predecessor.canonical_json().into_bytes(),
                action: crate::hot::ActionReplayRootAction::Play {
                    uid: 9,
                    target: None,
                    selection_uid: None,
                },
                answers: Vec::new(),
            })
            .unwrap();
        let forged = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let forged_catalog = HotBoundary::catalog_from_canonical(&forged).unwrap();
        let refusal = HotBoundary::from_canonical(&forged, &forged_catalog)
            .expect_err("a receipt naming an illegal uid must not load");
        assert!(
            format!("{refusal:?}").contains("ActionReplay root action is not exactly legal"),
            "{refusal:?}"
        );
    }

    #[test]
    fn plural_purity_exhaust_owner_exposes_and_resumes_nested_stratagem() {
        let mut builder = CatalogBuilder::new();
        let purity = builder
            .intern_reachable(plain_identity(CardId::Purity, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let strike = builder
            .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        let sculpting = builder
            .intern_reachable(plain_identity(CardId::SculptingStrike, 0))
            .unwrap();
        let bash = builder
            .intern_reachable(plain_identity(CardId::Bash, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_live_dark_embrace_reachable();
        builder.mark_live_stratagem_reachable();
        let catalog = builder.build();

        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 8;
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
        );
        assert!(
            state
                .fanouts
                .set_potion_belt(vec![None], false, false, false, false, true)
        );
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: purity,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 6,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 3,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            HotCard {
                uid: 4,
                atom: sculpting,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 7,
                atom: bash,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));

        let selecting = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        let parked = apply_action(
            &selecting,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(3),
            },
        )
        .unwrap()
        .state;
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::CardPlay { .. },
                crate::frame::Frame::AfterCardExhaustedPower { .. },
                crate::frame::Frame::Draw { .. },
            ]
        ));
        assert_eq!(
            play::persisted_card_play_stack_is_exact(&parked, &catalog),
            Ok(())
        );
        HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let answers = legal_actions(&parked, &catalog);
        assert!(!answers.is_empty());

        let completed = apply_action(&parked, &catalog, &answers[0])
            .expect("the nested Stratagem answer must resume Purity")
            .state;
        assert!(completed.pending.is_none());
        assert!(completed.frames.is_empty());
        assert_eq!(completed.history.card_plays_finished_combat, 1);
        assert_eq!(completed.history.owner_cards_exhausted_combat, 3);
        assert!(
            completed
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .any(|card| card.uid == 1)
        );
    }

    #[test]
    fn dedicated_exhaust_returns_roundtrip_and_resume_frozen_suffixes() {
        use crate::hot::{AfterCardExhaustedReturnKind, RngStream, RngStreamState};

        #[derive(Clone, Copy, Debug)]
        enum Case {
            SecondWind,
            FiendFire,
            Flak,
            Stoke,
        }

        for case in [Case::SecondWind, Case::FiendFire, Case::Flak, Case::Stoke] {
            let source_id = match case {
                Case::SecondWind => CardId::SecondWind,
                Case::FiendFire => CardId::FiendFire,
                Case::Flak => CardId::FlakCannon,
                Case::Stoke => CardId::Stoke,
            };
            let mut builder = CatalogBuilder::new();
            builder
                .intern_reachable(plain_identity(source_id, 0))
                .unwrap();
            for id in [
                CardId::DefendIronclad,
                CardId::StrikeIronclad,
                CardId::Bash,
                CardId::Burn,
            ] {
                builder.intern_reachable(plain_identity(id, 0)).unwrap();
            }
            if !matches!(case, Case::Stoke) {
                builder
                    .intern_reachable(plain_identity(CardId::SculptingStrike, 0))
                    .unwrap();
            }
            if matches!(case, Case::Stoke) {
                for id in crate::content_tables::STOKE_CARD_POOL_V109 {
                    for upgrade in [0, 1] {
                        builder.intern(plain_identity(id, upgrade)).unwrap();
                    }
                }
            }
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.mark_live_dark_embrace_reachable();
            builder.mark_live_stratagem_reachable();
            let catalog = builder.build();

            let atom = |id| catalog.atom(&plain_identity(id, 0)).unwrap();
            let card = |uid, id| HotCard {
                uid,
                atom: atom(id),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            };
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 3;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.exact_piles = true;
            state.next_card_uid = 7;
            state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
            state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            assert!(
                state
                    .fanouts
                    .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
            );
            assert!(
                state
                    .fanouts
                    .set_potion_belt(vec![None], false, false, false, false, true)
            );
            state.rng.set(
                RngStream::Rng,
                RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
            state.rng.set(
                RngStream::Generation,
                RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
            if matches!(case, Case::Stoke) {
                state.reward_card_pool = Some(crate::catalog::RewardPool::Ironclad);
                state.entropy_card_pool = Some(crate::catalog::RewardPool::Ironclad);
                state.fully_unlocked_card_pool_epochs = true;
            }
            state
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(card(1, source_id));
            match case {
                Case::SecondWind => state.piles.get_mut(PileId::Hand).make_mut().extend([
                    card(2, CardId::DefendIronclad),
                    card(6, CardId::DefendIronclad),
                ]),
                Case::FiendFire | Case::Stoke => {
                    state.piles.get_mut(PileId::Hand).make_mut().extend([
                        card(2, CardId::DefendIronclad),
                        card(6, CardId::StrikeIronclad),
                    ])
                }
                Case::Flak => state
                    .piles
                    .get_mut(PileId::Hand)
                    .make_mut()
                    .push(card(2, CardId::Burn)),
            }
            if matches!(case, Case::Flak) {
                state
                    .piles
                    .get_mut(PileId::Discard)
                    .make_mut()
                    .push(card(3, CardId::Burn));
            }
            state.piles.get_mut(PileId::Discard).make_mut().extend([
                card(
                    if matches!(case, Case::Flak) { 4 } else { 3 },
                    if matches!(case, Case::Stoke) {
                        CardId::DefendIronclad
                    } else {
                        CardId::SculptingStrike
                    },
                ),
                card(
                    if matches!(case, Case::Flak) { 5 } else { 4 },
                    CardId::DefendIronclad,
                ),
                card(if matches!(case, Case::Flak) { 6 } else { 5 }, CardId::Bash),
            ]);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 100));

            let action = Action::Play {
                uid: 1,
                target: matches!(case, Case::FiendFire).then_some(0),
                selection: SelectionRef::NONE,
            };
            if !matches!(case, Case::Stoke) {
                let predecessor_wire = HotBoundary::try_to_canonical(&state, &catalog)
                    .unwrap_or_else(|error| {
                        panic!("{case:?} predecessor failed to encode: {error:?}")
                    });
                let predecessor_catalog =
                    HotBoundary::catalog_from_canonical(&predecessor_wire).unwrap();
                let predecessor =
                    HotBoundary::from_canonical(&predecessor_wire, &predecessor_catalog)
                        .unwrap_or_else(|error| {
                            panic!("{case:?} predecessor failed to reload: {error:?}")
                        });
                assert_eq!(
                    admit(&predecessor_wire, &predecessor, &predecessor_catalog),
                    Ok(()),
                    "{case:?} predecessor"
                );
            }
            let parked = apply_action(&state, &catalog, &action)
                .unwrap_or_else(|error| panic!("{case:?} failed to park: {error:?}"))
                .state;
            let owner = parked
                .frames
                .as_slice()
                .iter()
                .find_map(|frame| match *frame {
                    crate::frame::Frame::AfterCardExhaustedPower { record } => {
                        parked.frames.after_card_exhausted_power(record)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{case:?} lacks its Exhaust owner"));
            let expected_return = match case {
                Case::SecondWind => AfterCardExhaustedReturnKind::SecondWind,
                Case::FiendFire => AfterCardExhaustedReturnKind::FiendFire,
                Case::Flak => AfterCardExhaustedReturnKind::Flak,
                Case::Stoke => AfterCardExhaustedReturnKind::Stoke,
            };
            assert_eq!(owner.return_kind, expected_return, "{case:?}");
            assert_eq!(owner.source_uid, Some(1), "{case:?}");
            assert_eq!(
                owner.remaining().collect::<Vec<_>>(),
                vec![if matches!(case, Case::Flak) { 3 } else { 6 }]
            );
            assert!(parked.pending.is_some(), "{case:?}");

            let wire = HotBoundary::try_to_canonical(&parked, &catalog)
                .unwrap_or_else(|error| panic!("{case:?} failed to encode: {error:?}"));
            let rebuilt_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
            let rebuilt = HotBoundary::from_canonical(&wire, &rebuilt_catalog)
                .unwrap_or_else(|error| panic!("{case:?} failed to reload: {error:?}"));
            assert_eq!(admit(&wire, &rebuilt, &rebuilt_catalog), Ok(()), "{case:?}");
            let answers = legal_actions(&rebuilt, &rebuilt_catalog);
            assert!(!answers.is_empty(), "{case:?}");
            let completed = apply_action(&rebuilt, &rebuilt_catalog, &answers[0])
                .unwrap_or_else(|error| panic!("{case:?} failed to resume: {error:?}"))
                .state;
            assert!(completed.pending.is_none(), "{case:?}");
            assert!(completed.frames.is_empty(), "{case:?}");
            assert_eq!(completed.history.card_plays_finished_combat, 1, "{case:?}");
            assert_eq!(
                completed.history.owner_cards_exhausted_combat,
                if matches!(case, Case::FiendFire) {
                    3
                } else {
                    2
                },
                "{case:?}"
            );
            match case {
                Case::SecondWind => assert_eq!(completed.block, 10),
                Case::FiendFire => {
                    assert_eq!(completed.monsters[0].hp, 86);
                    assert!(
                        completed
                            .piles
                            .get(PileId::Exhaust)
                            .as_slice()
                            .iter()
                            .any(|card| card.uid == 1)
                    );
                }
                Case::Flak => assert_eq!(completed.monsters[0].hp, 84),
                Case::Stoke => {
                    assert_eq!(completed.history.owner_generated_cards_combat, 2);
                    assert_eq!(completed.rng.get(RngStream::Generation).counter, 2);
                }
            }
        }
    }

    #[test]
    fn result_bearing_card_draws_roundtrip_and_resume_once() {
        use crate::hot::DrawCaller;

        for source_id in [CardId::Pillage, CardId::Restlessness] {
            let mut builder = CatalogBuilder::new();
            for id in [
                source_id,
                CardId::DefendIronclad,
                CardId::Burn,
                CardId::Dazed,
            ] {
                builder.intern_reachable(plain_identity(id, 0)).unwrap();
            }
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.mark_live_stratagem_reachable();
            let catalog = builder.build();
            let atom = |id| catalog.atom(&plain_identity(id, 0)).unwrap();
            let card = |uid, id| HotCard {
                uid,
                atom: atom(id),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            };
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 3;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.exact_piles = true;
            state.next_card_uid = 5;
            state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            assert!(
                state
                    .fanouts
                    .set_potion_belt(vec![None], false, false, false, false, true)
            );
            state
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .push(card(1, source_id));
            state.piles.get_mut(PileId::Discard).make_mut().extend([
                card(2, CardId::DefendIronclad),
                card(3, CardId::Burn),
                card(4, CardId::Dazed),
            ]);
            state
                .monsters_mut()
                .push(HotMonster::new(MonsterKind::Toadpole, 50));

            assert!(catalog.requires_action_replay(), "{source_id:?}");
            assert!(
                super::play::cardplay_result_draw_step_is_exact(
                    catalog.spec(atom(source_id)).unwrap(),
                    &catalog,
                    0,
                ),
                "{source_id:?}"
            );
            let initial_actions = legal_actions(&state, &catalog);
            assert!(
                initial_actions
                    .iter()
                    .any(|action| matches!(action, Action::Play { uid: 1, .. })),
                "{source_id:?} initial actions: {initial_actions:?}"
            );

            let parked = apply_action(
                &state,
                &catalog,
                &Action::Play {
                    uid: 1,
                    target: (source_id == CardId::Pillage).then_some(0),
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap_or_else(|error| panic!("{source_id:?} failed to park: {error:?}"))
            .state;
            let draw = parked
                .frames
                .as_slice()
                .iter()
                .find_map(|frame| match *frame {
                    crate::frame::Frame::Draw { record } => parked.frames.draw(record),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("{source_id:?} lacks its result Draw"));
            assert_eq!(
                draw.caller,
                if source_id == CardId::Pillage {
                    DrawCaller::Pillage
                } else {
                    DrawCaller::RestlessnessOneRemaining
                }
            );
            assert_eq!(draw.requested, 1);
            assert_eq!(draw.completed, 0);
            assert_eq!(draw.drawn().len(), 0);

            let wire = HotBoundary::try_to_canonical(&parked, &catalog)
                .unwrap_or_else(|error| panic!("{source_id:?} failed to encode: {error:?}"));
            let rebuilt_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
            let draw_position = wire
                .continuations
                .iter()
                .position(|frame| matches!(frame.frame_type.as_str(), "DrawFrame"))
                .unwrap();
            let mut forged_caller = wire.clone();
            forged_caller.continuations[draw_position].fields.insert(
                "caller".to_owned(),
                serde_json::json!(if source_id == CardId::Pillage {
                    "restlessness_one_remaining"
                } else {
                    "pillage"
                }),
            );
            let forged_error =
                HotBoundary::from_canonical(&forged_caller, &rebuilt_catalog).unwrap_err();
            assert!(
                matches!(
                    forged_error,
                    crate::boundary::BoundaryRefusal::MalformedFrame {
                        ref frame_type,
                        ref detail,
                } if matches!(frame_type.as_str(), "DrawFrame")
                    && matches!(detail.as_str(), "Draw caller does not match its parent")
                ),
                "{forged_error:?}"
            );

            let mut missing_root = wire.clone();
            missing_root.continuations.remove(0);
            assert!(HotBoundary::from_canonical(&missing_root, &rebuilt_catalog).is_err());
            let rebuilt = HotBoundary::from_canonical(&wire, &rebuilt_catalog)
                .unwrap_or_else(|error| panic!("{source_id:?} failed to reload: {error:?}"));
            assert_eq!(admit(&wire, &rebuilt, &rebuilt_catalog), Ok(()));
            let answers = legal_actions(&rebuilt, &rebuilt_catalog);
            assert!(!answers.is_empty(), "{source_id:?}");
            let completed = apply_action(&rebuilt, &rebuilt_catalog, &answers[0])
                .unwrap_or_else(|error| panic!("{source_id:?} failed to resume: {error:?}"))
                .state;
            assert!(completed.pending.is_none(), "{source_id:?}");
            assert!(completed.frames.is_empty(), "{source_id:?}");
            assert_eq!(completed.history.card_plays_finished_combat, 1);
            if source_id == CardId::Pillage {
                assert_eq!(completed.monsters[0].hp, 44);
                assert_eq!(completed.cards_drawn_combat, 1);
            } else {
                assert_eq!(completed.energy, 5);
                assert_eq!(completed.cards_drawn_combat, 2);
            }
        }
    }

    #[test]
    fn python_projected_restlessness_draws_load_and_resume_in_rust() {
        // Five Restlessness parks, projected by the frozen v0.111.0 Python
        // oracle and committed as data (#2827 item D): the inline
        // `combat_sim` script this test used to run is in git history, and
        // `tools/frozen_oracle_data.py --check` pins the fixture's bytes.
        let documents: Vec<CanonicalStateV2> = serde_json::from_str(include_str!(
            "../../fixtures/frozen_python_restlessness_draws_v1.json"
        ))
        .expect("the frozen Python projection holds five canonical documents");
        let expected = [
            ("restlessness_one_remaining", 7),
            ("restlessness_final", 7),
            ("restlessness_two_remaining", 8),
            ("restlessness_one_remaining", 8),
            ("restlessness_final", 8),
        ];
        assert_eq!(documents.len(), expected.len());

        for (document, (caller, expected_energy)) in documents.into_iter().zip(expected) {
            let draw_position = document
                .continuations
                .iter()
                .position(|frame| matches!(frame.frame_type.as_str(), "DrawFrame"))
                .expect("the Python state is parked in a Draw");
            assert_eq!(
                document.continuations[draw_position].fields["caller"],
                serde_json::json!(caller)
            );
            assert_eq!(
                document.continuations[draw_position].fields["caller_locals"],
                serde_json::json!([])
            );
            let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();

            for (field, value) in [
                (
                    "caller",
                    serde_json::json!(if matches!(caller, "restlessness_two_remaining") {
                        "restlessness_final"
                    } else {
                        "restlessness_two_remaining"
                    }),
                ),
                ("caller_locals", serde_json::json!([1, 2])),
            ] {
                let mut forged = document.clone();
                forged.continuations[draw_position]
                    .fields
                    .insert(field.to_owned(), value);
                let before = forged.clone();
                let error = HotBoundary::from_canonical(&forged, &catalog).unwrap_err();
                let rejected_exactly = match &error {
                    crate::boundary::BoundaryRefusal::MalformedFrame { frame_type, detail } => {
                        matches!(frame_type.as_str(), "DrawFrame")
                            && matches!(detail.as_str(), "Draw caller does not match its parent")
                    }
                    crate::boundary::BoundaryRefusal::UnrepresentableValue {
                        entity: crate::boundary::Entity::Player,
                        field,
                        detail,
                    } => {
                        matches!(field.as_str(), "pending")
                            && detail.contains("ActionReplay transcript does not reproduce")
                    }
                    _ => false,
                };
                assert!(rejected_exactly, "{field}: {error:?}");
                assert_eq!(forged, before, "{field}");
            }

            let rebuilt = HotBoundary::from_canonical(&document, &catalog).unwrap();
            assert_eq!(admit(&document, &rebuilt, &catalog), Ok(()));
            let actions = legal_actions(&rebuilt, &catalog);
            assert!(!actions.is_empty(), "{caller}");
            let completed = apply_action(&rebuilt, &catalog, &actions[0])
                .unwrap_or_else(|error| panic!("{caller} failed to resume: {error:?}"))
                .state;
            assert!(completed.pending.is_none(), "{caller}");
            assert!(completed.frames.is_empty(), "{caller}");
            assert_eq!(completed.energy, expected_energy, "{caller}");
            assert_eq!(completed.history.card_plays_finished_combat, 1, "{caller}");
        }
    }

    #[test]
    fn python_projected_replayed_tutor_card_uid_loads_and_resumes_in_rust() {
        // The replayed Tutor park, projected by the frozen v0.111.0 Python
        // oracle and committed as data (#2827 item D; the inline script is in
        // git history, the bytes are pinned by `tools/frozen_oracle_data.py`).
        let document: CanonicalStateV2 = serde_json::from_str(include_str!(
            "../../fixtures/frozen_python_tutor_replay_v1.json"
        ))
        .expect("the frozen Python projection is a canonical Tutor state");
        assert_eq!(
            document.continuations[0].fields["answers"],
            serde_json::json!([{"kind": "card_uid", "value": 22}])
        );
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert!(crate::boundary::action_replay_is_authenticated(
            &document, &state, &catalog
        ));
        assert_eq!(admit(&document, &state, &catalog), Ok(()));
        assert_eq!(state.piles.get(PileId::Draw).len(), 2);
        let action = legal_actions(&state, &catalog)
            .into_iter()
            .find(|action| {
                matches!(
                    action,
                    Action::Select {
                        answer: SelectionAnswer::CardUid(23)
                    }
                )
            })
            .expect("the projected second Tutor body selects the remaining Defend");
        let completed = apply_action(&state, &catalog, &action).unwrap().state;
        assert!(completed.pending.is_none());
        assert!(completed.frames.is_empty());
        assert!(
            completed
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .any(|card| card.uid == 23)
        );
    }

    #[test]
    fn havoc_rejected_forced_exhaust_retains_batch_owner_under_dark_draw() {
        let mut builder = CatalogBuilder::new();
        let havoc = builder
            .intern_reachable(plain_identity(CardId::Havoc, 0))
            .unwrap();
        let dazed = builder
            .intern_reachable(plain_identity(CardId::Dazed, 0))
            .unwrap();
        let sculpting = builder
            .intern_reachable(plain_identity(CardId::SculptingStrike, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_live_dark_embrace_reachable();
        builder.mark_live_stratagem_reachable();
        let catalog = builder.build();

        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 6;
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
        );
        assert!(
            state
                .fanouts
                .set_potion_belt(vec![None], false, false, false, false, true)
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: havoc,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 2,
            atom: dazed,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            HotCard {
                uid: 3,
                atom: sculpting,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        let action = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };
        let mut events = Vec::new();
        let parked = apply_action_with_replay_witness(&state, &catalog, &action, &mut events)
            .expect("replay-owned Havoc must park its rejected forced Exhaust child");
        assert!(parked.pending.is_some(), "{:#?}", parked.frames.as_slice());
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::FrozenAutoBatch { .. },
                    crate::frame::Frame::AfterCardExhaustedPower { .. },
                    crate::frame::Frame::Draw { .. },
                ]
            ),
            "{:#?}",
            parked.frames.as_slice()
        );
        assert_eq!(
            play::persisted_card_play_stack_is_exact(&parked, &catalog),
            Ok(())
        );
        let canonical = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        assert!(!canonical.continuations.is_empty());
    }

    #[test]
    fn cascade_result_exhaust_retains_card_finish_and_batch_owners() {
        let mut builder = CatalogBuilder::new();
        let cascade = builder
            .intern_reachable(plain_identity(CardId::Cascade, 0))
            .unwrap();
        let entrance = builder
            .intern_reachable(plain_identity(CardId::DramaticEntrance, 0))
            .unwrap();
        let sculpting = builder
            .intern_reachable(plain_identity(CardId::SculptingStrike, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_live_dark_embrace_reachable();
        builder.mark_live_stratagem_reachable();
        let catalog = builder.build();

        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 1;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 6;
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
        );
        assert!(
            state
                .fanouts
                .set_potion_belt(vec![None], false, false, false, false, true)
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: cascade,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 2,
            atom: entrance,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            HotCard {
                uid: 3,
                atom: sculpting,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 50));
        let action = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };
        let mut events = Vec::new();
        let parked = apply_action_with_replay_witness(&state, &catalog, &action, &mut events)
            .expect("replay-owned Cascade must park its result-Exhaust child");
        assert!(parked.pending.is_some(), "{:#?}", parked.frames.as_slice());
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::FrozenAutoBatch { .. },
                    crate::frame::Frame::CardFinish { .. },
                    crate::frame::Frame::AfterCardExhaustedPower { .. },
                    crate::frame::Frame::Draw { .. },
                ]
            ),
            "{:#?}",
            parked.frames.as_slice()
        );
        assert_eq!(
            play::persisted_card_play_stack_is_exact(&parked, &catalog),
            Ok(())
        );
        let canonical = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        assert!(!canonical.continuations.is_empty());
    }

    #[test]
    fn outer_draw_autoplay_owners_roundtrip_cardplay_draw_and_resume_suffix_once() {
        use crate::hot::{RngStream, RngStreamState};

        for (parent_id, child_id) in
            [CardId::Havoc, CardId::Cascade]
                .into_iter()
                .flat_map(|parent| {
                    [CardId::PommelStrike, CardId::Pillage, CardId::Restlessness]
                        .into_iter()
                        .map(move |child| (parent, child))
                })
        {
            let mut builder = CatalogBuilder::new();
            let parent = builder
                .intern_reachable(plain_identity(parent_id, 0))
                .unwrap();
            let child = builder
                .intern_reachable(plain_identity(child_id, 0))
                .unwrap();
            let strike = builder
                .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
                .unwrap();
            let defend = builder
                .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
                .unwrap();
            let bash = builder
                .intern_reachable(plain_identity(CardId::Bash, 0))
                .unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.mark_live_stratagem_reachable();
            let catalog = builder.build();

            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 2;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.exact_piles = true;
            state.next_card_uid = 6;
            state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            assert!(
                state
                    .fanouts
                    .set_potion_belt(vec![None], false, false, false, false, true,)
            );
            for stream in [RngStream::Rng, RngStream::Targets] {
                state.rng.set(
                    stream,
                    RngStreamState {
                        words: [1, 2, 3, 4],
                        counter: 0,
                    },
                );
            }
            let physical = CARD_FLAG_DEFAULT_PHYSICAL_STATE;
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: parent,
                flags: physical,
            });
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 2,
                atom: child,
                flags: physical,
            });
            if parent_id == CardId::Cascade {
                state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                    uid: 3,
                    atom: strike,
                    flags: physical,
                });
            }
            state.piles.get_mut(PileId::Discard).make_mut().extend([
                HotCard {
                    uid: 4,
                    atom: defend,
                    flags: physical,
                },
                HotCard {
                    uid: 5,
                    atom: bash,
                    flags: physical,
                },
            ]);
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
            monster.max_hp = 1_000;
            monster.loop_pos = 2;
            state.monsters = std::sync::Arc::new(vec![monster]);

            let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            let rebuilt_catalog = HotBoundary::catalog_from_canonical(&predecessor).unwrap();
            let rebuilt_state = HotBoundary::from_canonical(&predecessor, &rebuilt_catalog)
                .unwrap_or_else(|error| panic!("{parent_id:?} predecessor reload: {error:?}"));
            assert_eq!(
                admit(&predecessor, &rebuilt_state, &rebuilt_catalog),
                Ok(()),
                "{parent_id:?}/{child_id:?}"
            );
            assert!(
                legal_actions(&rebuilt_state, &rebuilt_catalog)
                    .iter()
                    .any(|action| matches!(action, Action::Play { uid: 1, .. })),
                "{parent_id:?}/{child_id:?}: {:?}",
                legal_actions(&rebuilt_state, &rebuilt_catalog)
            );
            let parked = apply_action(
                &rebuilt_state,
                &rebuilt_catalog,
                &Action::Play {
                    uid: 1,
                    target: None,
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap_or_else(|error| panic!("{parent_id:?}/{child_id:?} failed to park: {error:?}"))
            .state;
            assert!(
                matches!(
                    parked.frames.as_slice(),
                    [
                        crate::frame::Frame::ActionReplay { .. },
                        crate::frame::Frame::CardPlay { .. },
                        crate::frame::Frame::FrozenAutoBatch { .. },
                        crate::frame::Frame::CardPlay { .. },
                        crate::frame::Frame::Draw { .. },
                    ]
                ),
                "{parent_id:?}/{child_id:?}: {:#?}",
                parked.frames.as_slice()
            );
            let draw = match parked.frames.top().unwrap() {
                crate::frame::Frame::Draw { record } => parked.frames.draw(record).unwrap(),
                _ => unreachable!(),
            };
            assert_eq!(
                draw.caller,
                match child_id {
                    CardId::PommelStrike => crate::hot::DrawCaller::CardPlay,
                    CardId::Pillage => crate::hot::DrawCaller::Pillage,
                    CardId::Restlessness => crate::hot::DrawCaller::RestlessnessOneRemaining,
                    _ => unreachable!(),
                }
            );
            assert_eq!(
                play::persisted_card_play_stack_is_exact(&parked, &rebuilt_catalog),
                Ok(())
            );
            let wire = HotBoundary::try_to_canonical(&parked, &rebuilt_catalog).unwrap();
            if parent_id == CardId::Havoc && child_id == CardId::PommelStrike {
                for (field, value) in [
                    ("requested", serde_json::json!(0)),
                    ("completed", serde_json::json!(1)),
                    ("from_hand_draw", serde_json::json!(true)),
                    ("stage", serde_json::json!("early_hook")),
                    ("caller", serde_json::json!("Pillage")),
                    ("caller_locals", serde_json::json!([1])),
                ] {
                    let mut tampered = wire.clone();
                    tampered.continuations[4]
                        .fields
                        .insert(field.to_owned(), value);
                    assert!(
                        HotBoundary::from_canonical(&tampered, &rebuilt_catalog).is_err(),
                        "root replay rejects Draw {field} drift"
                    );
                }
                let mut candidate_tampered = wire.clone();
                candidate_tampered.player.get_mut("pending").unwrap()[2]
                    .as_array_mut()
                    .unwrap()
                    .pop();
                assert!(
                    HotBoundary::from_canonical(&candidate_tampered, &rebuilt_catalog).is_err(),
                    "rootless Draw rejects frozen/live Stratagem candidate drift"
                );
                let pending = parked.pending.as_deref().unwrap();
                let wrong_record = match parked.frames.as_slice()[3] {
                    crate::frame::Frame::CardPlay { record } => record,
                    _ => unreachable!(),
                };
                for (frame_uid, frame_record) in [
                    (pending.frame_uid.wrapping_add(1), pending.frame_record),
                    (pending.frame_uid, wrong_record),
                ] {
                    let mut tampered = parked.clone();
                    tampered.pending = Some(std::sync::Arc::new(crate::hot::PendingSelection {
                        frame_uid,
                        frame_record,
                    }));
                    assert_eq!(
                        play::persisted_card_play_stack_is_exact(&tampered, &rebuilt_catalog),
                        Err(EngineRefusal::ContinuationNotModeled),
                        "pending Draw identity drift refuses"
                    );
                }
                let mut detached = parked.clone();
                detached.piles.get_mut(PileId::Draw).make_mut().clear();
                assert_ne!(detached, parked, "the clone mutation is load-bearing");
                assert!(parked.pending.is_some(), "the parked source stays intact");

                let mut rollback = parked.clone();
                rollback.history.card_plays_finished_combat = i32::MAX;
                let before = rollback.clone();
                let mut events = vec![Event::TurnBegan { turn: 91 }];
                let answer = legal_actions(&rollback, &rebuilt_catalog)[0];
                assert!(
                    apply_action_into(&rollback, &rebuilt_catalog, &answer, &mut events,).is_err()
                );
                assert_eq!(
                    rollback, before,
                    "failed outer Draw action is mutation-free"
                );
                assert!(
                    events.is_empty(),
                    "failed outer Draw action leaks no events"
                );
            }
            let rebuilt_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
            let rebuilt = HotBoundary::from_canonical(&wire, &rebuilt_catalog)
                .unwrap_or_else(|error| panic!("{parent_id:?}/{child_id:?} reload: {error:?}"));
            assert_eq!(admit(&wire, &rebuilt, &rebuilt_catalog), Ok(()));

            let answers = legal_actions(&rebuilt, &rebuilt_catalog);
            assert!(!answers.is_empty(), "{parent_id:?}/{child_id:?}");
            let completed = apply_action(&rebuilt, &rebuilt_catalog, &answers[0])
                .unwrap_or_else(|error| panic!("{parent_id:?}/{child_id:?} resume: {error:?}"))
                .state;
            assert!(completed.pending.is_none(), "{parent_id:?}/{child_id:?}");
            assert!(completed.frames.is_empty(), "{parent_id:?}/{child_id:?}");
            assert_eq!(
                completed.history.card_plays_finished_combat,
                if parent_id == CardId::Cascade { 3 } else { 2 },
                "{parent_id:?}/{child_id:?}"
            );
        }
    }

    #[test]
    fn mayhem_autopre_roundtrips_a_cardplay_owned_draw_and_resumes_once() {
        let mut builder = CatalogBuilder::new();
        let pommel = builder
            .intern_reachable(plain_identity(CardId::PommelStrike, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let strike = builder
            .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_live_stratagem_reachable();
        let catalog = builder.build();

        let physical = CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 18;
        state.powers.set(PowerId::Mayhem, SlotWire::Int, 1);
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        assert!(
            state
                .fanouts
                .set_potion_belt(vec![None], false, false, false, false, true)
        );
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((2..12).map(|uid| HotCard {
                uid,
                atom: defend,
                flags: physical,
            }));
        state.piles.get_mut(PileId::Draw).make_mut().extend(
            (13..17)
                .map(|uid| HotCard {
                    uid,
                    atom: defend,
                    flags: physical,
                })
                .chain([
                    HotCard {
                        uid: 17,
                        atom: strike,
                        flags: physical,
                    },
                    HotCard {
                        uid: 1,
                        atom: pommel,
                        flags: physical,
                    },
                ]),
        );
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);

        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&predecessor).unwrap();
        let state = HotBoundary::from_canonical(&predecessor, &catalog).unwrap();
        admit(&predecessor, &state, &catalog).unwrap();
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .expect("AutoPre Mayhem parks Pommel Strike's owned Draw")
            .state;
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::Phase { .. },
                    crate::frame::Frame::FrozenAutoBatch { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                ]
            ),
            "{:#?}",
            parked.frames.as_slice()
        );
        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let parked = HotBoundary::from_canonical(&wire, &catalog).unwrap();
        admit(&wire, &parked, &catalog).unwrap();
        let completed = apply_action(&parked, &catalog, &legal_actions(&parked, &catalog)[0])
            .unwrap()
            .state;
        assert!(completed.pending.is_none());
        assert!(completed.frames.is_empty());
        assert_eq!(completed.history.card_plays_finished_combat, 1);
    }

    #[test]
    fn stampede_live_roundtrips_a_pillage_owned_draw_and_resumes_once() {
        let mut builder = CatalogBuilder::new();
        let pillage = builder
            .intern_reachable(plain_identity(CardId::Pillage, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_live_stratagem_reachable();
        let catalog = builder.build();

        let physical = CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.turn = 2;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.player_side_active = true;
        state.exact_piles = true;
        state.next_card_uid = 5;
        state.powers.set(PowerId::Stampede, SlotWire::Int, 1);
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        assert!(
            state
                .fanouts
                .set_potion_belt(vec![None], false, false, false, false, true)
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: pillage,
            flags: physical,
        });
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            HotCard {
                uid: 2,
                atom: defend,
                flags: physical,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: physical,
            },
            HotCard {
                uid: 4,
                atom: defend,
                flags: physical,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);

        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&predecessor).unwrap();
        let state = HotBoundary::from_canonical(&predecessor, &catalog).unwrap();
        admit(&predecessor, &state, &catalog).unwrap();
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .expect("Stampede parks Pillage's owned Draw")
            .state;
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::Phase { .. },
                    crate::frame::Frame::LiveListener { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                ]
            ),
            "{:#?}",
            parked.frames.as_slice()
        );
        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let parked = HotBoundary::from_canonical(&wire, &catalog).unwrap();
        admit(&wire, &parked, &catalog).unwrap();
        let after_stampede = apply_action(&parked, &catalog, &legal_actions(&parked, &catalog)[0])
            .unwrap()
            .state;
        assert!(
            matches!(
                after_stampede.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::Draw { .. }
                ]
            ),
            "the distinct turn-start Draw follows the completed Stampede owner: {:#?}",
            after_stampede.frames.as_slice()
        );
        assert_eq!(after_stampede.history.card_plays_finished_combat, 1);
        let completed = apply_action(
            &after_stampede,
            &catalog,
            &legal_actions(&after_stampede, &catalog)[0],
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none());
        assert!(completed.frames.is_empty());
        assert_eq!(completed.history.card_plays_finished_combat, 1);
    }

    #[test]
    fn sly_discard_roundtrips_a_restlessness_owned_draw_and_resumes_once() {
        let mut builder = CatalogBuilder::new();
        let survivor = builder
            .intern_reachable(plain_identity(CardId::Survivor, 0))
            .unwrap();
        let restlessness = builder
            .intern_reachable(plain_identity(CardId::Restlessness, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.mark_live_stratagem_reachable();
        let catalog = builder.build();

        let physical = CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 5;
        state.powers.set(PowerId::MasterPlanner, SlotWire::Int, 1);
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        play::hydrate_after_card_played_power_order_for_test(&mut state);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        assert!(
            state
                .fanouts
                .set_potion_belt(vec![None], false, false, false, false, true)
        );
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: survivor,
                flags: physical,
            },
            HotCard {
                uid: 2,
                atom: restlessness,
                flags: physical,
            },
        ]);
        state.card_states.set_local_sly(2);
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            HotCard {
                uid: 3,
                atom: defend,
                flags: physical,
            },
            HotCard {
                uid: 4,
                atom: defend,
                flags: physical,
            },
        ]);

        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&predecessor).unwrap();
        let state = HotBoundary::from_canonical(&predecessor, &catalog).unwrap();
        admit(&predecessor, &state, &catalog).unwrap();
        let selecting = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        let parked = selecting;
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::FrozenAutoBatch { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                ]
            ),
            "{:#?}",
            parked.frames.as_slice()
        );
        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let parked = HotBoundary::from_canonical(&wire, &catalog).unwrap();
        admit(&wire, &parked, &catalog).unwrap();
        let completed = apply_action(&parked, &catalog, &legal_actions(&parked, &catalog)[0])
            .unwrap()
            .state;
        assert!(completed.pending.is_none());
        assert!(completed.frames.is_empty());
        assert_eq!(completed.history.card_plays_finished_combat, 2);
    }

    #[test]
    fn cardplay_owned_draw_round_trips_and_resumes_the_parent_suffix_once() {
        let mut builder = CatalogBuilder::new();
        let pommel = builder
            .intern_reachable(plain_identity(CardId::PommelStrike, 0))
            .unwrap();
        let seeker = builder
            .intern_reachable(plain_identity(CardId::SeekerStrike, 0))
            .unwrap();
        let strike = builder
            .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let bash = builder
            .intern_reachable(plain_identity(CardId::Bash, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 7;
        state.exact_piles = true;
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.rng.set(
            crate::hot::RngStream::Sel,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.rng.set(
            crate::hot::RngStream::Targets,
            crate::hot::RngStreamState {
                words: [5, 6, 7, 8],
                counter: 0,
            },
        );
        let physical = CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: pommel,
            flags: physical,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 2,
                atom: pommel,
                flags: physical,
            },
            HotCard {
                uid: 3,
                atom: seeker,
                flags: physical,
            },
            HotCard {
                uid: 4,
                atom: strike,
                flags: physical,
            },
            HotCard {
                uid: 5,
                atom: defend,
                flags: physical,
            },
            HotCard {
                uid: 6,
                atom: bash,
                flags: physical,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);

        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));
        let parked = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. }
                ]
            ),
            "{:#?}",
            parked.frames
        );
        let draw_record = match parked.frames.as_slice()[2] {
            crate::frame::Frame::Draw { record } => record,
            _ => unreachable!(),
        };
        assert_eq!(
            parked.frames.draw(draw_record).unwrap().caller,
            crate::hot::DrawCaller::CardPlay
        );
        let nested_draw_record = match parked.frames.as_slice()[4] {
            crate::frame::Frame::Draw { record } => record,
            _ => unreachable!(),
        };
        assert_eq!(
            parked.frames.draw(nested_draw_record).unwrap().caller,
            crate::hot::DrawCaller::CardPlay
        );
        assert_eq!(parked.cards_drawn_combat, 2);

        let canonical = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        assert_eq!(canonical.continuations[2].fields["caller"], "none");
        assert_eq!(
            canonical.continuations[2].fields["caller_locals"],
            serde_json::json!([])
        );
        assert_eq!(canonical.continuations[4].fields["caller"], "none");
        assert_eq!(
            canonical.continuations[4].fields["caller_locals"],
            serde_json::json!([])
        );
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&canonical).unwrap();
        assert!(rebuilt_catalog.requires_action_replay());
        let rebuilt = HotBoundary::from_canonical(&canonical, &rebuilt_catalog).unwrap();
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
            canonical
        );
        assert_eq!(admit(&canonical, &rebuilt, &rebuilt_catalog), Ok(()));

        let mut forged_locals = canonical.clone();
        forged_locals.continuations[2]
            .fields
            .insert("caller_locals".to_owned(), serde_json::json!([1]));
        assert!(HotBoundary::from_canonical(&forged_locals, &rebuilt_catalog).is_err());
        let mut forged_caller = canonical.clone();
        forged_caller.continuations[2]
            .fields
            .insert("caller".to_owned(), serde_json::json!("potion_epilogue"));
        assert!(HotBoundary::from_canonical(&forged_caller, &rebuilt_catalog).is_err());

        let mut overflow = rebuilt.clone();
        overflow.history.card_plays_finished_combat = i32::MAX;
        let before = overflow.clone();
        let mut events = vec![Event::TurnEnded { turn: 99 }];
        assert_eq!(
            apply_action_into(
                &overflow,
                &rebuilt_catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(0),
                },
                &mut events,
            )
            .unwrap_err(),
            EngineRefusal::CounterOverflow("card_plays_finished_combat")
        );
        assert_eq!(overflow, before);
        assert!(events.is_empty());

        let completed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none());
        assert!(completed.frames.is_empty());
        assert_eq!(completed.cards_drawn_combat, 2);
        assert_eq!(completed.history.card_plays_finished_combat, 3);
    }

    #[test]
    fn turn_start_stratagem_without_draw_cards_roundtrips_and_resumes_once() {
        for tyranny in [false, true] {
            let mut builder = CatalogBuilder::new();
            let defend = builder
                .intern_reachable(plain_identity(CardId::DefendSilent, 0))
                .unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 3;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = 7;
            state.exact_piles = true;
            state.powers.set(PowerId::Stratagem, SlotWire::Int, 2);
            if tyranny {
                state.powers.set(PowerId::Tyranny, SlotWire::Int, 1);
                assert!(
                    state
                        .fanouts
                        .set_turn_start_hand_choice_order(&[PowerId::Tyranny])
                );
            }
            state.rng.set(
                crate::hot::RngStream::Rng,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 1,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            for uid in 2..7 {
                state
                    .piles
                    .get_mut(PileId::Discard)
                    .make_mut()
                    .push(HotCard {
                        uid,
                        atom: defend,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    });
            }
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
            monster.max_hp = 1_000;
            monster.loop_pos = 2;
            state.monsters = std::sync::Arc::new(vec![monster]);
            let root = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            let catalog = HotBoundary::catalog_from_canonical(&root).unwrap();
            let state = HotBoundary::from_canonical(&root, &catalog).unwrap();
            assert!(!catalog.requires_action_replay());
            assert!(catalog.cardplay_draw_hook_can_suspend());
            assert!(!catalog.cardplay_no_result_draw_can_suspend());
            assert_eq!(admit(&root, &state, &catalog), Ok(()));
            let parked = apply_action(&state, &catalog, &Action::EndTurn)
                .unwrap()
                .state;
            assert!(matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::Draw { .. }
                ]
            ));
            let draw = parked
                .pending
                .as_deref()
                .unwrap()
                .stratagem_draw_record(&parked.frames)
                .unwrap();
            assert_eq!(draw.completed, 1);
            assert_eq!(draw.shuffle_candidates().len(), 5);
            assert_eq!(parked.cards_drawn_combat, 1);
            assert_eq!(parked.rng.get(crate::hot::RngStream::Rng).counter, 4);
            let cold = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
            let loaded_catalog = HotBoundary::catalog_from_canonical(&cold).unwrap();
            let loaded = HotBoundary::from_canonical(&cold, &loaded_catalog).unwrap();
            assert_eq!(admit(&cold, &loaded, &loaded_catalog), Ok(()));
            let actions = legal_actions(&loaded, &loaded_catalog);
            assert!(!actions.is_empty());
            for action in actions {
                let mut resumed = apply_action(&loaded, &loaded_catalog, &action)
                    .unwrap()
                    .state;
                if tyranny {
                    // The same EndTurn now pauses for a later, independent choice.
                    let wire = HotBoundary::try_to_canonical(&resumed, &loaded_catalog).unwrap();
                    let second_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
                    let second = HotBoundary::from_canonical(&wire, &second_catalog).unwrap();
                    assert_eq!(admit(&wire, &second, &second_catalog), Ok(()));
                    let choices = legal_actions(&second, &second_catalog);
                    assert_eq!(choices.len(), 6);
                    resumed = apply_action(&second, &second_catalog, &choices[0])
                        .unwrap()
                        .state;
                }
                assert!(resumed.pending.is_none());
                assert!(resumed.frames.is_empty());
                assert_eq!(resumed.turn, parked.turn);
                assert_eq!(resumed.hp, parked.hp);
                assert_eq!(resumed.cards_drawn_combat, 4);
                assert_eq!(resumed.history.non_hand_draws_this_turn, 0);
                assert_eq!(
                    resumed.piles.get(PileId::Hand).len(),
                    if tyranny { 5 } else { 6 }
                );
                assert_eq!(resumed.rng.get(crate::hot::RngStream::Rng).counter, 4);
            }
            let mut forged = cold.clone();
            forged.continuations.remove(0);
            assert!(HotBoundary::from_canonical(&forged, &loaded_catalog).is_err());
        }
    }

    #[test]
    fn cardplay_owned_draw_derives_replay_for_blocking_stratagem_without_a_selector_body() {
        let mut builder = CatalogBuilder::new();
        let pommel = builder
            .intern_reachable(plain_identity(CardId::PommelStrike, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let bash = builder
            .intern_reachable(plain_identity(CardId::Bash, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        assert!(!catalog.requires_action_replay());

        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 4;
        state.exact_piles = true;
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        state.rng.set(
            crate::hot::RngStream::Rng,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: pommel,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            HotCard {
                uid: 2,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: bash,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);

        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&predecessor).unwrap();
        assert!(rebuilt_catalog.requires_action_replay());
        let rebuilt = HotBoundary::from_canonical(&predecessor, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&predecessor, &rebuilt, &rebuilt_catalog), Ok(()));

        let parked = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Play {
                uid: 1,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::CardPlay { .. },
                crate::frame::Frame::Draw { .. }
            ]
        ));
        assert!(
            parked
                .pending
                .as_deref()
                .is_some_and(|pending| { pending.stratagem_draw_record(&parked.frames).is_some() })
        );
        assert_eq!(parked.cards_drawn_combat, 0);
        let canonical = HotBoundary::try_to_canonical(&parked, &rebuilt_catalog).unwrap();
        let roundtrip_catalog = HotBoundary::catalog_from_canonical(&canonical).unwrap();
        let roundtrip = HotBoundary::from_canonical(&canonical, &roundtrip_catalog).unwrap();
        assert_eq!(
            HotBoundary::try_to_canonical(&roundtrip, &roundtrip_catalog).unwrap(),
            canonical
        );
        assert_eq!(admit(&canonical, &roundtrip, &roundtrip_catalog), Ok(()));

        let mut overflow = roundtrip.clone();
        overflow.history.card_plays_finished_combat = i32::MAX;
        let before = overflow.clone();
        let mut events = vec![Event::TurnEnded { turn: 99 }];
        assert_eq!(
            apply_action_into(
                &overflow,
                &roundtrip_catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(0),
                },
                &mut events,
            )
            .unwrap_err(),
            EngineRefusal::CounterOverflow("card_plays_finished_combat")
        );
        assert_eq!(overflow, before);
        assert!(events.is_empty());

        let completed = apply_action(
            &roundtrip,
            &roundtrip_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.cards_drawn_combat, 1);
        assert_eq!(completed.history.card_plays_finished_combat, 1);
        assert_eq!(
            completed.rng.get(crate::hot::RngStream::Rng).counter,
            parked.rng.get(crate::hot::RngStream::Rng).counter
        );
    }

    #[test]
    fn distilled_retains_a_child_cardplay_parent_across_its_owned_draw() {
        let mut builder = CatalogBuilder::new();
        let pommel = builder
            .intern_reachable(plain_identity(CardId::PommelStrike, 0))
            .unwrap();
        let seeker = builder
            .intern_reachable(plain_identity(CardId::SeekerStrike, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let strike = builder
            .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 9;
        state.exact_piles = true;
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        for stream in [crate::hot::RngStream::Sel, crate::hot::RngStream::Targets] {
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        let physical = CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: pommel,
                flags: physical,
            },
            HotCard {
                uid: 2,
                atom: defend,
                flags: physical,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: physical,
            },
            HotCard {
                uid: 4,
                atom: pommel,
                flags: physical,
            },
            HotCard {
                uid: 5,
                atom: seeker,
                flags: physical,
            },
            HotCard {
                uid: 6,
                atom: defend,
                flags: physical,
            },
            HotCard {
                uid: 7,
                atom: strike,
                flags: physical,
            },
            HotCard {
                uid: 8,
                atom: defend,
                flags: physical,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);

        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::PotionFinish { .. },
                    crate::frame::Frame::FrozenAutoBatch { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. }
                ]
            ),
            "{:#?}",
            parked.frames
        );
        let canonical = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&canonical).unwrap();
        let rebuilt = HotBoundary::from_canonical(&canonical, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&canonical, &rebuilt, &rebuilt_catalog), Ok(()));

        let completed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.cards_drawn_combat, 2);
        assert_eq!(completed.history.card_plays_finished_combat, 5);
        assert_eq!(completed.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn cardplay_owned_draw_validates_ordinary_listener_draw_nesting() {
        let mut builder = CatalogBuilder::new();
        let pommel = builder
            .intern_reachable(plain_identity(CardId::PommelStrike, 0))
            .unwrap();
        let dazed = builder
            .intern_reachable(plain_identity(CardId::Dazed, 0))
            .unwrap();
        let seeker = builder
            .intern_reachable(plain_identity(CardId::SeekerStrike, 0))
            .unwrap();
        let strike = builder
            .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 8;
        state.exact_piles = true;
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        state.powers.set(PowerId::Pagestorm, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Pagestorm])
        );
        for stream in [crate::hot::RngStream::Sel, crate::hot::RngStream::Targets] {
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        let physical = CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: pommel,
            flags: physical,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 2,
                atom: dazed,
                flags: physical,
            },
            HotCard {
                uid: 3,
                atom: seeker,
                flags: physical,
            },
            HotCard {
                uid: 4,
                atom: strike,
                flags: physical,
            },
            HotCard {
                uid: 5,
                atom: defend,
                flags: physical,
            },
            HotCard {
                uid: 6,
                atom: strike,
                flags: physical,
            },
            HotCard {
                uid: 7,
                atom: defend,
                flags: physical,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);

        let parked = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::CardPlay { .. },
                crate::frame::Frame::Draw { .. },
                crate::frame::Frame::AfterCardDrawnPower { .. },
                crate::frame::Frame::Draw { .. },
                crate::frame::Frame::CardPlay { .. }
            ]
        ));
        assert_eq!(
            play::persisted_card_play_stack_is_exact(&parked, &catalog),
            Ok(())
        );
        let completed = apply_action(
            &parked,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.cards_drawn_combat, 2);
        assert_eq!(completed.history.card_plays_finished_combat, 2);
    }

    #[test]
    fn malformed_pending_routing_refuses_before_actions_or_projection() {
        let identities = [
            plain_identity(CardId::BurningPact, 0),
            plain_identity(CardId::StrikeIronclad, 0),
        ];
        let mut builder = CatalogBuilder::new();
        for identity in identities {
            builder.intern(identity).unwrap();
        }
        let catalog = builder.build();
        let valid = replay_selection_state(&catalog, &[(7, identities[1])]);
        let document = HotBoundary::try_to_canonical(&valid, &catalog).unwrap();

        // Only ordinals 0..COUNT and the canonical absent sentinel 255 exist;
        // a live pending bit makes the sentinel invalid too.
        for raw in [11, 254, 255] {
            let mut state = valid.clone();
            let pending = state.pending.as_deref().unwrap().clone();
            state
                .frames
                .corrupt_selection_ordinal_for_test(&pending, raw);
            let before = state.clone();

            assert!(legal_actions(&state, &catalog).is_empty());
            assert!(HotBoundary::try_to_canonical(&state, &catalog).is_err());
            assert!(
                admit(&document, &state, &catalog)
                    .unwrap_err()
                    .missing()
                    .any(|item| {
                        item == MissingCapability::ArgumentShape("pending selection routing")
                    })
            );
            let mut events = vec![Event::CardResolved {
                uid: 999,
                pile: PileId::Discard,
            }];
            assert_eq!(
                apply_action_into(
                    &state,
                    &catalog,
                    &Action::Select {
                        answer: SelectionAnswer::CardUid(7),
                    },
                    &mut events,
                ),
                Err(EngineRefusal::ContinuationNotModeled)
            );
            assert_eq!(state, before);
            assert!(events.is_empty());
        }
    }

    #[test]
    fn replay_selection_preserves_python_descending_payload_order() {
        let identities = [
            plain_identity(CardId::BurningPact, 0),
            plain_identity(CardId::Claw, 0),
            plain_identity(CardId::Whirlwind, 0),
            plain_identity(CardId::Impatience, 1),
            plain_identity(CardId::Infection, 0),
        ];
        let mut builder = CatalogBuilder::new();
        for identity in identities {
            builder.intern(identity).unwrap();
        }
        let catalog = builder.build();
        let state = replay_selection_state(
            &catalog,
            &[
                (15, identities[1]),
                (5, identities[2]),
                (23, identities[3]),
                (26, identities[4]),
            ],
        );

        assert_eq!(
            legal_actions(&state, &catalog),
            [5, 26, 23, 15]
                .into_iter()
                .map(|uid| Action::Select {
                    answer: SelectionAnswer::CardUid(uid),
                })
                .collect::<Vec<_>>()
        );
        let mut buffer = LegalActionBuffer::new();
        assert_eq!(
            legal_actions_into(&state, &catalog, &mut buffer),
            legal_actions(&state, &catalog)
        );
    }

    #[test]
    fn replay_selection_reverses_equal_payload_live_hand_order() {
        let identities = [
            plain_identity(CardId::BurningPact, 0),
            plain_identity(CardId::Infection, 0),
        ];
        let mut builder = CatalogBuilder::new();
        for identity in identities {
            builder.intern(identity).unwrap();
        }
        let catalog = builder.build();
        let state = replay_selection_state(
            &catalog,
            &[
                (26, identities[1]),
                (28, identities[1]),
                (25, identities[1]),
            ],
        );

        assert_eq!(
            legal_actions(&state, &catalog),
            [25, 28, 26]
                .into_iter()
                .map(|uid| Action::Select {
                    answer: SelectionAnswer::CardUid(uid),
                })
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn replay_selection_orders_genetic_algorithm_and_ringing_by_complete_payload() {
        let identities = [
            plain_identity(CardId::BurningPact, 0),
            plain_identity(CardId::GeneticAlgorithm, 0),
        ];
        let mut builder = CatalogBuilder::new();
        for identity in identities {
            builder.intern(identity).unwrap();
        }
        let catalog = builder.build();
        let mut state =
            replay_selection_state(&catalog, &[(31, identities[1]), (32, identities[1])]);
        let physical = crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        let genetic = crate::hot::CARD_FLAG_GENETIC_ALGORITHM_STATE;
        let ringing = crate::hot::CARD_FLAG_RINGING;
        let hand = state.piles.get_mut(PileId::Hand).make_mut();
        hand[0].flags = physical | genetic;
        hand[1].flags = physical | ringing;
        state.card_states.set(
            31,
            crate::hot::CardInstanceState {
                genetic_algorithm: crate::hot::GeneticAlgorithmState::from_parts(2, None).unwrap(),
                ..crate::hot::CardInstanceState::default()
            },
        );

        assert_eq!(
            legal_actions(&state, &catalog),
            [31, 32]
                .into_iter()
                .map(|uid| Action::Select {
                    answer: SelectionAnswer::CardUid(uid),
                })
                .collect::<Vec<_>>(),
            "Python sorts Ringing < GA, then singleton expansion reverses"
        );

        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 33,
            atom: catalog.atom(&identities[1]).unwrap(),
            flags: physical | genetic | ringing,
        });
        state.card_states.set(
            33,
            crate::hot::CardInstanceState {
                genetic_algorithm: crate::hot::GeneticAlgorithmState::from_parts(2, None).unwrap(),
                ..crate::hot::CardInstanceState::default()
            },
        );
        assert_eq!(
            legal_actions(&state, &catalog),
            [33, 31, 32]
                .into_iter()
                .map(|uid| Action::Select {
                    answer: SelectionAnswer::CardUid(uid),
                })
                .collect::<Vec<_>>(),
            "Python sorts Ringing < GA < GA+Ringing, then singleton expansion reverses"
        );
    }

    #[test]
    fn replay_selection_refuses_incomparable_genetic_algorithm_deck_rows() {
        let identities = [
            plain_identity(CardId::BurningPact, 0),
            plain_identity(CardId::GeneticAlgorithm, 0),
        ];
        let mut builder = CatalogBuilder::new();
        for identity in identities {
            builder.intern(identity).unwrap();
        }
        let catalog = builder.build();
        let mut state =
            replay_selection_state(&catalog, &[(41, identities[1]), (42, identities[1])]);
        let flags = crate::hot::CARD_FLAG_DEFAULT_PHYSICAL_STATE
            | crate::hot::CARD_FLAG_GENETIC_ALGORITHM_STATE;
        for card in state.piles.get_mut(PileId::Hand).make_mut() {
            card.flags = flags;
        }
        for (uid, row) in [(41, None), (42, Some(7))] {
            state.card_states.set(
                uid,
                crate::hot::CardInstanceState {
                    genetic_algorithm: crate::hot::GeneticAlgorithmState::from_parts(2, row)
                        .unwrap(),
                    ..crate::hot::CardInstanceState::default()
                },
            );
        }

        let mut buffer = LegalActionBuffer::new();
        assert_eq!(
            replay_selection_actions_into(&state, &catalog, &mut buffer),
            Err(EngineRefusal::MalformedArgs(
                "selection Genetic Algorithm deck-row order"
            ))
        );
        assert!(
            legal_actions(&state, &catalog).is_empty(),
            "the public infallible action API exposes comparator refusal as no legal action"
        );
    }

    #[test]
    fn duplicator_removes_the_exact_slot_and_adds_one_intensity() {
        let (mut state, catalog) = fixture();
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::Duplicator), Some(PotionId::Duplicator)],
            false,
            false,
            false,
            false,
            true,
        ));
        let successor = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 1,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert_eq!(
            successor.fanouts.potion_slots(),
            [Some(PotionId::Duplicator), None]
        );
        assert_eq!(successor.fanouts.duplication(), 1);
        assert!(successor.frames.is_empty());
        assert!(successor.pending.is_none());
        assert_eq!(state.fanouts.duplication(), 0);
        assert!(
            state
                .fanouts
                .potion_belt_shares_store_with(&state.clone().fanouts)
        );
        assert!(
            !state
                .fanouts
                .potion_belt_shares_store_with(&successor.fanouts)
        );
    }

    #[test]
    fn unsupported_potion_body_refuses_before_physical_removal() {
        let (mut state, catalog) = fixture();
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::Ambergris)],
            false,
            false,
            false,
            false,
            true,
        ));
        let before = state.clone();
        let mut events = vec![Event::TurnEnded { turn: 99 }];
        assert_eq!(
            apply_action_into(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
                &mut events,
            ),
            Err(EngineRefusal::PotionsNotModeled)
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn supported_potion_native_target_types_are_pinned() {
        // #2497: every supported body's `get_TargetType` is read and none is
        // AnyAlly (6), the one `PotionModel::IsValidTarget` (`0x8328c`) kind a
        // pet can pass. A body added to `SUPPORTED` without its read fails
        // here rather than silently inheriting the pet-safe answer.
        let mut by_kind = vec![Vec::<PotionId>::new(); 10];
        for potion in PotionId::ALL {
            let kind = potions::native_target_type(potion);
            assert_eq!(kind.is_some(), potions::is_supported(potion), "{potion:?}");
            if let Some(kind) = kind {
                assert!(potions::native_target_excludes_pets(potion), "{potion:?}");
                by_kind[usize::from(kind)].push(potion);
            }
        }
        assert_eq!(by_kind[1], [PotionId::FairyInABottle]);
        assert_eq!(
            by_kind[2],
            [
                PotionId::BeetleJuice,
                PotionId::FirePotion,
                PotionId::PoisonPotion,
                PotionId::PotionOfDoom,
                PotionId::PotionShapedRock,
                PotionId::PowderedDemise,
                PotionId::VulnerablePotion,
                PotionId::WeakPotion,
            ]
        );
        assert_eq!(
            by_kind[3],
            [
                PotionId::ExplosiveAmpoule,
                PotionId::FoulPotion,
                PotionId::PotionOfBinding,
                PotionId::ShacklingPotion,
            ]
        );
        assert_eq!(by_kind[5].len(), 49);
        assert_eq!(
            (0..by_kind.len())
                .filter(|kind| !by_kind[*kind].is_empty())
                .collect::<Vec<_>>(),
            [1, 2, 3, 5]
        );
        // The three bodies the 2026-09-26 census stopped on with a live Osty.
        for potion in [
            PotionId::BlessingOfTheForge,
            PotionId::BlockPotion,
            PotionId::LiquidBronze,
        ] {
            assert_eq!(potions::native_target_type(potion), Some(5));
        }
    }

    #[test]
    fn supported_potion_body_census_is_the_complete_r52e_set() {
        let expected = [
            PotionId::Ashwater,
            PotionId::AttackPotion,
            PotionId::BeetleJuice,
            PotionId::BlessingOfTheForge,
            PotionId::BlockPotion,
            PotionId::BloodPotion,
            PotionId::BoneBrew,
            PotionId::BottledPotential,
            PotionId::Clarity,
            PotionId::ColorlessPotion,
            PotionId::CosmicConcoction,
            PotionId::CunningPotion,
            PotionId::CureAll,
            PotionId::DexterityPotion,
            PotionId::DistilledChaos,
            PotionId::DropletOfPrecognition,
            PotionId::Duplicator,
            PotionId::EnergyPotion,
            PotionId::EntropicBrew,
            PotionId::EssenceOfDarkness,
            PotionId::ExplosiveAmpoule,
            PotionId::FairyInABottle,
            PotionId::FirePotion,
            PotionId::FlexPotion,
            PotionId::FocusPotion,
            PotionId::Fortifier,
            PotionId::FoulPotion,
            PotionId::FruitJuice,
            PotionId::FyshOil,
            PotionId::GamblersBrew,
            PotionId::GhostInAJar,
            PotionId::GigantificationPotion,
            PotionId::GlowwaterPotion,
            PotionId::HeartOfIron,
            PotionId::LiquidBronze,
            PotionId::LiquidMemories,
            PotionId::LuckyTonic,
            PotionId::MazalethsGift,
            PotionId::OrobicAcid,
            PotionId::PoisonPotion,
            PotionId::PotionOfBinding,
            PotionId::PotionOfCapacity,
            PotionId::PotionOfDoom,
            PotionId::PotionShapedRock,
            PotionId::PotOfGhouls,
            PotionId::PowderedDemise,
            PotionId::PowerPotion,
            PotionId::RadiantTincture,
            PotionId::RegenPotion,
            PotionId::ShacklingPotion,
            PotionId::ShipInABottle,
            PotionId::SkillPotion,
            PotionId::SneckoOil,
            PotionId::SoldiersStew,
            PotionId::SpeedPotion,
            PotionId::StableSerum,
            PotionId::StarPotion,
            PotionId::StrengthPotion,
            PotionId::SwiftPotion,
            PotionId::TouchOfInsanity,
            PotionId::VulnerablePotion,
            PotionId::WeakPotion,
        ];
        assert_eq!(potions::SUPPORTED, expected);
        assert_eq!(expected.len(), 62);
        assert_eq!(
            PotionId::ALL
                .into_iter()
                .filter(|potion| potions::is_supported(*potion))
                .collect::<Vec<_>>(),
            expected
        );
        // #3229: the two bodies no census belt carries stay unported.
        assert!(!potions::is_supported(PotionId::Ambergris));
        assert!(!potions::is_supported(PotionId::KingsCourage));
        assert!(potions::is_supported(PotionId::ColorlessPotion));
        assert!(potions::is_supported(PotionId::FairyInABottle));
        let solo_player = expected
            .into_iter()
            .filter(|potion| potions::requires_solo_player_target(*potion))
            .collect::<Vec<_>>();
        assert_eq!(
            solo_player,
            [
                PotionId::BlessingOfTheForge,
                PotionId::BlockPotion,
                PotionId::BloodPotion,
                // #3229: kind 5 (`0xabebd`, `0xabf29`) and both bodies read
                // `target.Player` (`0x34c934` IL_002e, `0x34cae0` IL_002e).
                PotionId::CosmicConcoction,
                PotionId::CunningPotion,
                PotionId::DexterityPotion,
                // Since #2497 this list is the *teammate* gate only; a live
                // local Osty no longer consults it (see
                // `supported_potion_native_target_types_are_pinned`).
                PotionId::DistilledChaos,
                PotionId::Duplicator,
                PotionId::EnergyPotion,
                // #3229: kind 5 (`0xac2d9`), channels into `target.Player`'s
                // queue (`0x34d478` IL_002b-IL_0065).
                PotionId::EssenceOfDarkness,
                PotionId::FlexPotion,
                // #2873: target kind 5 (`0xac591`) and the picked target is
                // the recipient (`0x34dbd4` IL_0049), so a teammate pick is
                // real.
                PotionId::FocusPotion,
                PotionId::Fortifier,
                PotionId::FyshOil,
                PotionId::GlowwaterPotion,
                PotionId::HeartOfIron,
                PotionId::LiquidBronze,
                PotionId::PotionOfBinding,
                // #3229: kind 5 (`0xad0c5`), `OrbCmd::AddSlots(target.Player,
                // ..)` (`0x34f630` IL_0048-IL_0063).
                PotionId::PotionOfCapacity,
                PotionId::PotionShapedRock,
                PotionId::RadiantTincture,
                PotionId::RegenPotion,
                PotionId::SpeedPotion,
                PotionId::StableSerum,
                PotionId::StarPotion,
                PotionId::StrengthPotion,
            ]
        );
        for pet_safe in [
            PotionId::BeetleJuice,
            PotionId::FirePotion,
            PotionId::PowderedDemise,
            PotionId::VulnerablePotion,
            PotionId::WeakPotion,
            PotionId::ExplosiveAmpoule,
            PotionId::ShacklingPotion,
            PotionId::GigantificationPotion,
            PotionId::LuckyTonic,
            PotionId::MazalethsGift,
            PotionId::ShipInABottle,
            PotionId::SoldiersStew,
            PotionId::AttackPotion,
            PotionId::SkillPotion,
            PotionId::PowerPotion,
            PotionId::ColorlessPotion,
            PotionId::OrobicAcid,
            PotionId::EntropicBrew,
            PotionId::FairyInABottle,
            PotionId::FruitJuice,
            // #2480: Potion of Doom is TargetType 2 (`0xad131`), Foul
            // Potion's own `!IsPet` body filter (`0x34ddc2`) keeps a pet out
            // of its damage list, and Ghost in a Jar applies Intangible to
            // `Owner.Creature` rather than to the picked ally (`0x34e544`).
            PotionId::FoulPotion,
            PotionId::GhostInAJar,
            PotionId::PotionOfDoom,
            // #3229: TargetType 2 (`0xacf6d`), an enemy-only pick.
            PotionId::PoisonPotion,
        ] {
            assert!(!potions::requires_solo_player_target(pet_safe));
        }
    }

    /// Admit a one-potion belt through the canonical round-trip, so the
    /// catalog carries the belt's boundary closure (#3229).
    fn roundtrip_with_belt(
        mut state: HotState,
        catalog: &Catalog,
        belt: Vec<Option<PotionId>>,
    ) -> (CanonicalStateV2, HotState, Catalog) {
        assert!(
            state
                .fanouts
                .set_potion_belt(belt, false, false, false, false, true)
        );
        let document = HotBoundary::try_to_canonical(&state, catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        (document, state, catalog)
    }

    /// #3229 Poison Potion: `PoisonPotion/<OnUse>d__10::MoveNext` `0x34f2c4`
    /// IL_0095 applies `PoisonPower(6)` to the enemy pick (TargetType 2,
    /// `0xacf6d`) with the player as applier and a null card.
    #[test]
    fn issue_3229_poison_potion_applies_six_poison_to_its_enemy_pick() {
        let (mut state, catalog) = fixture();
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 1);
        let (document, state, catalog) =
            roundtrip_with_belt(state, &catalog, vec![Some(PotionId::PoisonPotion)]);
        assert_eq!(admit(&document, &state, &catalog), Ok(()));

        // Target legality: kind 2 picks one living enemy, never None.
        let legal = legal_actions(&state, &catalog);
        for target in [Some(0), Some(1)] {
            assert!(legal.contains(&Action::UsePotion { slot: 0, target }));
        }
        assert!(!legal.contains(&Action::UsePotion {
            slot: 0,
            target: None
        }));
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None
                },
                &mut events
            ),
            Err(EngineRefusal::PotionTargetMismatch { required: true })
        );

        // The pick receives 6 and a fresh Poison instance uid; the other
        // monster's Artifact is untouched.
        let poisoned = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: Some(1),
            },
        )
        .unwrap()
        .state;
        assert_eq!(poisoned.fanouts.potion_slots(), [None]);
        assert!(poisoned.frames.is_empty() && poisoned.pending.is_none());
        assert_eq!(poisoned.monsters[1].powers.value(PowerId::Poison), 6);
        assert_eq!(poisoned.monsters[1].poison_uid, state.next_poison_uid);
        assert_eq!(poisoned.next_poison_uid, state.next_poison_uid + 1);
        assert_eq!(poisoned.monsters[0].powers.value(PowerId::Poison), 0);
        assert_eq!(poisoned.monsters[0].powers.value(PowerId::Artifact), 1);

        // Artifact on the pick consumes the whole Type-2 application.
        let blocked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: Some(0),
            },
        )
        .unwrap()
        .state;
        assert_eq!(blocked.monsters[0].powers.value(PowerId::Poison), 0);
        assert_eq!(blocked.monsters[0].powers.value(PowerId::Artifact), 0);
        assert_eq!(blocked.next_poison_uid, state.next_poison_uid);

        // A live stack grows additively and keeps its instance uid.
        let mut stacked = state.clone();
        stacked.monsters_mut()[1]
            .powers
            .set(PowerId::Poison, SlotWire::Int, 2);
        stacked.monsters_mut()[1].poison_uid = 7;
        stacked.next_poison_uid = 8;
        stacked.monsters_mut()[1]
            .misery_debuff_order
            .push(MiseryToken::Poison);
        let stacked = apply_action(
            &stacked,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: Some(1),
            },
        )
        .unwrap()
        .state;
        assert_eq!(stacked.monsters[1].powers.value(PowerId::Poison), 8);
        assert_eq!(stacked.monsters[1].poison_uid, 7);
        assert_eq!(stacked.next_poison_uid, 8);
    }

    /// #3229 Potion of Capacity: `0x34f630` IL_0063 `OrbCmd::AddSlots(.., 2)`,
    /// which clamps to the native ten (`0x132f14` IL_001e-IL_0037), grows only
    /// the live capacity (`OrbQueue::AddCapacity` `0x1184f2`) and returns at
    /// IsOverOrEnding (IL_0011).
    #[test]
    fn issue_3229_potion_of_capacity_adds_two_live_slots_capped_at_ten() {
        for (before, after) in [(0_u8, 2_u8), (3, 5), (9, 10), (10, 10)] {
            let (mut state, catalog) = fixture();
            state.orbs.set_slots(before);
            let base = state.orbs.base_slots();
            let successor =
                drink_fixture_potion(state, &catalog, PotionId::PotionOfCapacity, None, false)
                    .state;
            assert_eq!(successor.orbs.slots(), after, "{before}");
            assert_eq!(successor.orbs.base_slots(), base, "{before}");
            assert!(successor.orbs.as_slice().is_empty());
            assert_eq!(successor.fanouts.potion_slots(), [None]);
            assert!(successor.frames.is_empty());
        }

        // Ending (every primary dead, no veto) before the over latch: the
        // command is a no-op.
        let (mut state, catalog) = fixture();
        state.orbs.set_slots(3);
        for monster in state.monsters_mut().iter_mut() {
            monster.hp = 0;
        }
        assert!(!state.history.over);
        assert!(damage::damage_combat_is_ending(&state));
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::PotionOfCapacity)],
            false,
            false,
            false,
            false,
            true,
        ));
        let mut ending = state.clone();
        potions::use_potion(
            &mut ending,
            &catalog,
            0,
            None,
            PotionId::PotionOfCapacity,
            &mut Vec::new(),
        )
        .unwrap();
        assert_eq!(ending.orbs.slots(), 3);
    }

    /// #3229 Essence of Darkness: `0x34d478` freezes
    /// `OrbQueue.Capacity` once (IL_0037-IL_004b) and awaits
    /// `Channel<DarkOrb>` that many times (IL_0065, loop IL_00bd-IL_00d8).
    #[test]
    fn issue_3229_essence_of_darkness_channels_one_dark_per_frozen_capacity() {
        let dark = crate::hot::HotOrb::from_parts(crate::hot::OrbKind::Dark, Some(6)).unwrap();
        let frost = crate::hot::HotOrb::from_parts(crate::hot::OrbKind::Frost, None).unwrap();

        // An empty three-slot queue fills with three Dark orbs.
        let (mut state, catalog) = fixture();
        state.orbs.set_slots(3);
        let successor =
            drink_fixture_potion(state, &catalog, PotionId::EssenceOfDarkness, None, false).state;
        assert_eq!(successor.orbs.as_slice(), [dark, dark, dark]);
        assert_eq!(successor.orbs.slots(), 3);
        assert_eq!(successor.fanouts.potion_slots(), [None]);
        assert!(successor.frames.is_empty());

        // Zero capacity reads count 0: no Channel runs, so Channel's one-slot
        // bootstrap never fires.
        let (mut state, catalog) = fixture();
        state.orbs.set_slots(0);
        let successor =
            drink_fixture_potion(state, &catalog, PotionId::EssenceOfDarkness, None, false).state;
        assert!(successor.orbs.as_slice().is_empty());
        assert_eq!(successor.orbs.slots(), 0);

        // A full two-slot Frost queue: each Channel evokes its front Frost
        // (5 Block apiece) before enqueueing, leaving two Dark orbs.
        let (mut state, catalog) = fixture();
        state.block = 0;
        state.orbs.set_slots(2);
        state.orbs.set_orbs(vec![frost, frost]);
        let successor =
            drink_fixture_potion(state, &catalog, PotionId::EssenceOfDarkness, None, false).state;
        assert_eq!(successor.orbs.as_slice(), [dark, dark]);
        assert_eq!(successor.block, 10);
    }

    /// #3229 Cunning Potion: `0x34cae0` awaits the count overload of
    /// `Shiv::CreateInHand` (3, IL_0056; `0x3bb20c`) and upgrades every
    /// returned Shiv (IL_00c4-IL_00da). Admission needs both Shiv atoms.
    #[test]
    fn issue_3229_cunning_potion_mints_three_upgraded_shivs() {
        let (mut state, catalog) = fixture();
        assert!(
            catalog.atom(&plain_identity(CardId::Shiv, 1)).is_none(),
            "the bare fixture has no Shiv closure"
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::CunningPotion)],
            false,
            false,
            false,
            false,
            true,
        ));
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert!(admit(&document, &state, &catalog).unwrap_err().contains(
            admission::MissingCapability::ArgumentShape("Cunning Potion Shiv closure")
        ));

        // The boundary interns both levels from the held belt.
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert_eq!(admit(&document, &state, &catalog), Ok(()));
        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        assert!(legal_actions(&state, &catalog).contains(&action));
        let hand_before = state.piles.get(PileId::Hand).len();
        let generated_before = state.history.owner_generated_cards_combat;
        let successor = apply_action(&state, &catalog, &action).unwrap().state;
        let hand = successor.piles.get(PileId::Hand).as_slice();
        assert_eq!(hand.len(), hand_before + 3);
        for card in &hand[hand_before..] {
            assert_eq!(
                catalog.spec(card.atom).unwrap().identity,
                plain_identity(CardId::Shiv, 1)
            );
        }
        assert_eq!(
            successor.history.owner_generated_cards_combat,
            generated_before + 3
        );
        assert_eq!(successor.fanouts.potion_slots(), [None]);
        assert!(successor.frames.is_empty() && successor.pending.is_none());
    }

    /// #3229 Cosmic Concoction: `0x34c934` runs Colorless Potion's
    /// `GetDistinctForCombat(.., 3, CombatCardGeneration)` (IL_0070, one full
    /// Generation shuffle) and, per returned card in order, `CardCmd::Upgrade`
    /// (IL_0098) then one `AddGeneratedCardToCombat(.., Hand, ..)` (IL_00a7).
    #[test]
    fn issue_3229_cosmic_concoction_adds_the_first_three_shuffled_colorless_cards_upgraded() {
        let (mut state, catalog) = entropic_fixture(
            vec![Some(PotionId::CosmicConcoction)],
            false,
            [1, 2, 3, 4],
            0,
        );
        // A live primary, so combat is not ending between the members.
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        for stream in crate::hot::RngStream::ALL {
            if state.rng.is_vacant(stream) {
                state.rng.set(
                    stream,
                    crate::hot::RngStreamState {
                        words: [11, 12, 13, 14],
                        counter: 0,
                    },
                );
            }
        }
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert_eq!(admit(&document, &state, &catalog), Ok(()));
        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        assert!(legal_actions(&state, &catalog).contains(&action));

        // The native draw order: one complete shuffle of the Colorless pool.
        let pool =
            potions::fight_generation_choice_pool(&state, &catalog, PotionId::ColorlessPotion)
                .unwrap();
        let mut rehearsal = state.clone();
        let shuffled = cards::shuffle_generation_slice(&mut rehearsal, &pool).unwrap();
        let expected = shuffled
            .into_iter()
            .take(3)
            .map(|id| {
                let base = plain_identity(id, 0);
                if cards::native_card_is_upgradable(base) {
                    plain_identity(id, 1)
                } else {
                    base
                }
            })
            .collect::<Vec<_>>();
        assert!(expected.iter().any(|identity| identity.upgrade == 1));

        let hand_before = state.piles.get(PileId::Hand).len();
        let successor = apply_action(&state, &catalog, &action).unwrap().state;
        let added = successor.piles.get(PileId::Hand).as_slice()[hand_before..]
            .iter()
            .map(|card| catalog.spec(card.atom).unwrap().identity)
            .collect::<Vec<_>>();
        assert_eq!(added, expected);
        assert_eq!(
            successor.rng.get(crate::hot::RngStream::Generation).counter,
            state.rng.get(crate::hot::RngStream::Generation).counter + pool.len() as u64 - 1
        );
        assert_eq!(successor.fanouts.potion_slots(), [None]);
        assert!(successor.frames.is_empty() && successor.pending.is_none());

        // Combat ending (the only primary dead, no veto) before a member:
        // refused by name, atomically, rather than choosing the native
        // IsEnding Upgrade skip's history reading.
        let mut ending = state.clone();
        ending.monsters_mut()[0].hp = 0;
        assert!(damage::damage_combat_is_ending(&ending));
        let before = ending.clone();
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(&ending, &catalog, &action, &mut events),
            Err(EngineRefusal::MalformedArgs(
                "Cosmic Concoction member after a combat-ending or parked add"
            ))
        );
        assert_eq!(ending, before);
        assert!(events.is_empty());

        // No recorded provenance: the body refuses by name, atomically.
        let mut unrecorded = state.clone();
        unrecorded.reward_card_pool = None;
        assert_eq!(
            potions::manual_part_d_prerequisites_are_exact(
                &unrecorded,
                &catalog,
                PotionId::CosmicConcoction
            ),
            Err(EngineRefusal::MalformedArgs(
                "Cosmic Concoction generation provenance"
            ))
        );
        let before = unrecorded.clone();
        assert!(apply_action(&unrecorded, &catalog, &action).is_err());
        assert_eq!(unrecorded, before);
    }

    #[test]
    fn r52e_colorless_potion_selection_roundtrips_and_resumes() {
        let (mut state, catalog) = entropic_fixture(
            vec![Some(PotionId::ColorlessPotion)],
            false,
            [1, 2, 3, 4],
            0,
        );
        for stream in crate::hot::RngStream::ALL {
            if state.rng.is_vacant(stream) {
                state.rng.set(
                    stream,
                    crate::hot::RngStreamState {
                        words: [11, 12, 13, 14],
                        counter: 0,
                    },
                );
            }
        }
        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert_eq!(admit(&document, &state, &catalog), Ok(()));
        assert!(legal_actions(&state, &catalog).contains(&action));
        let parked = apply_action(&state, &catalog, &action).unwrap().state;
        assert!(potions::generation_pending_is_exact(&parked, &catalog));
        assert_eq!(parked.fanouts.potion_slots(), [None]);
        let parked_document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&parked_document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&parked_document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&parked_document, &rebuilt, &rebuilt_catalog), Ok(()));
        let completed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(3),
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn gambler_selection_ordinals_include_relative_sly_permutations() {
        let identity = plain_identity(CardId::FlickFlack, 0);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(identity).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 6;
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom,
                flags: 0,
            },
        ]);
        assert!(!catalog.requires_action_replay());
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::GamblersBrew)],
            false,
            false,
            false,
            false,
            true,
        ));
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::PotionFinish { .. }
                ]
            ),
            "{:#?}",
            parked.frames.as_slice()
        );
        assert_eq!(
            legal_actions(&parked, &catalog),
            (0..5)
                .map(|ordinal| Action::Select {
                    answer: SelectionAnswer::OptionIndex(ordinal),
                })
                .collect::<Vec<_>>()
        );

        let canonical = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&canonical).unwrap();
        let rebuilt = HotBoundary::from_canonical(&canonical, &rebuilt_catalog).unwrap();
        assert_eq!(rebuilt, parked);
        admit(&canonical, &rebuilt, &rebuilt_catalog).unwrap();

        let finish = match parked.frames.top().unwrap() {
            crate::frame::Frame::PotionFinish { record } => {
                parked.frames.potion_finish(record).unwrap().to_owned()
            }
            _ => unreachable!(),
        };
        let mut rootless = parked.clone();
        rootless.frames = crate::hot::Frames::new();
        let rootless_record = rootless.frames.push_potion_finish(&finish).unwrap();
        rootless.pending = Some(std::sync::Arc::new(crate::hot::PendingSelection {
            frame_uid: crate::hot::POTION_SELECTION_PENDING_UID,
            frame_record: rootless_record,
        }));
        assert!(legal_actions(&rootless, &catalog).is_empty());
        let rootless_before = rootless.clone();
        assert_eq!(
            apply_action(
                &rootless,
                &catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(0),
                },
            )
            .unwrap_err(),
            EngineRefusal::ContinuationNotModeled
        );
        assert_eq!(rootless, rootless_before);
    }

    #[test]
    fn gambler_draw_then_sly_selection_roundtrips_and_finishes_once() {
        let purity = plain_identity(CardId::Purity, 0);
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let purity_atom = builder.intern(purity).unwrap();
        let defend_atom = builder.intern(defend).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 4;
        state.powers.set(PowerId::MasterPlanner, SlotWire::Int, 1);
        crate::engine::play::hydrate_after_card_played_power_order_for_test(&mut state);
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: purity_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: defend_atom,
                flags: 0,
            },
        ]);
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 3,
            atom: defend_atom,
            flags: 0,
        });
        state.card_states.set_local_sly(1);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::GamblersBrew)],
            false,
            false,
            false,
            false,
            true,
        ));

        let selecting = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let parked = legal_actions(&selecting, &catalog)
            .iter()
            .filter_map(|action| apply_action(&selecting, &catalog, action).ok())
            .map(|transition| transition.state)
            .find(|candidate| {
                matches!(
                    candidate.frames.as_slice(),
                    [
                        crate::frame::Frame::ActionReplay { .. },
                        crate::frame::Frame::PotionFinish { .. },
                        crate::frame::Frame::FrozenAutoBatch { .. },
                        crate::frame::Frame::CardPlay { .. },
                    ]
                ) && candidate.pending.is_some()
            })
            .expect("one Gambler ordinal selects the local-Sly Purity");
        let batch = match parked.frames.as_slice()[2] {
            crate::frame::Frame::FrozenAutoBatch { record } => {
                parked.frames.frozen_auto_batch(record).unwrap()
            }
            _ => unreachable!(),
        };
        assert_eq!(batch.cursor, 1);
        assert_eq!(batch.uids().collect::<Vec<_>>(), [1]);
        let finish = match parked.frames.as_slice()[1] {
            crate::frame::Frame::PotionFinish { record } => {
                parked.frames.potion_finish(record).unwrap()
            }
            _ => unreachable!(),
        };
        assert!(finish.candidates().next().is_none());
        assert_eq!(
            play::persisted_card_play_stack_is_exact(&parked, &catalog),
            Ok(())
        );

        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let mut tampered_capture = document.clone();
        tampered_capture.continuations[2]
            .fields
            .get_mut("entries")
            .unwrap()[0]["uid"] = serde_json::json!(4);
        assert!(HotBoundary::from_canonical(&tampered_capture, &rebuilt_catalog).is_err());
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
            document
        );
        admit(&document, &rebuilt, &rebuilt_catalog).unwrap();

        let answer = legal_actions(&rebuilt, &rebuilt_catalog)
            .iter()
            .find(|action| matches!(action, Action::Select { .. }))
            .copied()
            .unwrap();
        let completed = apply_action(&rebuilt, &rebuilt_catalog, &answer).unwrap();
        assert!(completed.state.frames.is_empty());
        assert!(completed.state.pending.is_none());
        assert_eq!(completed.state.fanouts.potion_slots(), [None]);
        assert_eq!(completed.state.history.discarded_cards_this_turn, 1);
    }

    #[test]
    fn empty_touch_is_a_physical_noop_use_and_forged_target_is_atomic() {
        let catalog = CatalogBuilder::new().build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.player_phase = crate::engine::admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::TouchOfInsanity)],
            false,
            false,
            false,
            false,
            true,
        ));
        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        assert_eq!(legal_actions(&state, &catalog), [action, Action::EndTurn]);

        let before = state.clone();
        let mut forged_events = vec![Event::TurnEnded { turn: 99 }];
        assert_eq!(
            apply_action_into(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: Some(0),
                },
                &mut forged_events,
            ),
            Err(EngineRefusal::PotionTargetMismatch { required: false })
        );
        assert_eq!(state, before);
        assert!(forged_events.is_empty());

        let successor = apply_action(&state, &catalog, &action).unwrap().state;
        let mut expected = state.clone();
        assert_eq!(
            expected.fanouts.remove_potion_at(0),
            Some(PotionId::TouchOfInsanity)
        );
        assert_eq!(successor, expected);
        assert!(successor.pending.is_none());
        assert!(successor.frames.is_empty());

        let document = HotBoundary::try_to_canonical(&successor, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
            document
        );
        admit(&document, &rebuilt, &rebuilt_catalog).unwrap();
    }

    #[test]
    fn droplet_freezes_native_rarity_model_order_and_moves_the_exact_uid() {
        let mut builder = CatalogBuilder::new();
        let rage = builder.intern(plain_identity(CardId::Rage, 0)).unwrap();
        let bash = builder.intern(plain_identity(CardId::Bash, 0)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 3;
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: rage,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: bash,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DropletOfPrecognition)],
            false,
            false,
            false,
            false,
            true,
        ));

        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let finish = parked
            .pending
            .as_deref()
            .unwrap()
            .potion_finish_record(&parked.frames)
            .unwrap();
        assert_eq!(
            finish.candidates().map(|card| card.uid).collect::<Vec<_>>(),
            [2, 1]
        );
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &rebuilt, &rebuilt_catalog), Ok(()));
        let mut reordered = document.clone();
        reordered.player.get_mut("pending").unwrap()[3]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        assert!(HotBoundary::from_canonical(&reordered, &rebuilt_catalog).is_err());

        let completed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.fanouts.potion_slots(), [None]);
        assert_eq!(
            completed
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [2]
        );
        assert_eq!(
            completed
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [1]
        );
    }

    #[test]
    fn liquid_memories_sets_both_free_rows_before_full_hand_redirect() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let bash = builder.intern(plain_identity(CardId::Bash, 0)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 13;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((1..=10).map(|uid| HotCard {
                uid,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            HotCard {
                uid: 11,
                atom: bash,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 12,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::LiquidMemories)],
            false,
            false,
            false,
            false,
            true,
        ));
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let parked = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &parked, &rebuilt_catalog), Ok(()));

        let completed = apply_action(
            &parked,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert_eq!(
            completed
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [12, 11]
        );
        let selected = completed.card_states.get(11);
        assert_eq!(
            selected.local_cost_modifiers.as_slice(),
            [crate::hot::LocalCostModifier {
                kind: crate::hot::LocalCostModifierKind::Set,
                amount: 0,
                expiration: crate::hot::LocalCostExpiration::ThisTurnOrPlayed,
                reduce_only: false,
            }]
        );
        assert_eq!(selected.free_star_cost_this_turn_or_played_rows, 1);
        assert_eq!(completed.card_states.get(12), Default::default());
        assert_eq!(completed.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn repeated_touch_after_temporary_free_cost_reloads_and_expires_exactly() {
        use crate::hot::LocalCostExpiration::{ThisCombat, ThisTurn, ThisTurnOrPlayed};
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let mut catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 3;
        for uid in [1, 2] {
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        }
        state.card_states.set_to_free_this_turn(1, 1).unwrap();
        state.powers.set(PowerId::BorrowedTime, SlotWire::Int, 1);
        state
            .fanouts
            .register_after_side_turn_end_power(
                crate::hot::AfterSideTurnEndPowerToken::BorrowedTime,
            )
            .unwrap();
        assert!(state.fanouts.set_potion_belt(
            vec![
                Some(PotionId::TouchOfInsanity),
                Some(PotionId::TouchOfInsanity)
            ],
            false,
            false,
            false,
            false,
            true
        ));
        for slot in 0..2 {
            state = apply_action(&state, &catalog, &Action::UsePotion { slot, target: None })
                .unwrap()
                .state;
            let parked = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            catalog = HotBoundary::catalog_from_canonical(&parked).unwrap();
            state = HotBoundary::from_canonical(&parked, &catalog).unwrap();
            assert_eq!(admit(&parked, &state, &catalog), Ok(()));
            state = apply_action(
                &state,
                &catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(0),
                },
            )
            .unwrap()
            .state;
            let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
            state = HotBoundary::from_canonical(&document, &catalog).unwrap();
            assert_eq!(admit(&document, &state, &catalog), Ok(()));
        }
        let selected = state.card_states.get(1);
        assert_eq!(
            selected
                .local_cost_modifiers
                .star_cost_expirations(1)
                .collect::<Vec<_>>(),
            [ThisTurnOrPlayed, ThisCombat, ThisCombat]
        );
        assert_eq!(
            selected
                .local_cost_modifiers
                .as_slice()
                .iter()
                .map(|row| row.expiration)
                .collect::<Vec<_>>(),
            [ThisTurnOrPlayed, ThisCombat, ThisCombat]
        );
        assert_eq!(state.card_states.get(2), Default::default());
        assert_eq!(state.fanouts.potion_slots(), [None, None]);
        state.card_states.cleanup_local_cost_modifiers(ThisTurn);
        let selected = state.card_states.get(1);
        assert_eq!(selected.free_star_cost_this_turn_or_played_rows, 0);
        assert_eq!(
            selected
                .local_cost_modifiers
                .star_cost_expirations(0)
                .collect::<Vec<_>>(),
            [ThisCombat, ThisCombat]
        );
        assert_eq!(selected.local_cost_modifiers.as_slice().len(), 2);
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let reloaded = HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert_eq!(
            HotBoundary::try_to_canonical(&reloaded, &catalog).unwrap(),
            document
        );
    }

    #[test]
    fn touch_filters_x_and_zero_then_sets_energy_and_star_combat_rows_by_uid() {
        let mut builder = CatalogBuilder::new();
        let atoms = [
            CardId::Anger,
            CardId::DefendIronclad,
            CardId::Alignment,
            CardId::SevenStars,
            CardId::Skewer,
        ]
        .map(|id| builder.intern(plain_identity(id, 0)).unwrap());
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 6;
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend(atoms.into_iter().enumerate().map(|(index, atom)| HotCard {
                uid: u32::try_from(index + 1).unwrap(),
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::TouchOfInsanity)],
            false,
            false,
            false,
            false,
            true,
        ));
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let finish = parked
            .pending
            .as_deref()
            .unwrap()
            .potion_finish_record(&parked.frames)
            .unwrap();
        assert_eq!(
            finish.candidates().map(|card| card.uid).collect::<Vec<_>>(),
            [2, 3, 4]
        );
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let parked = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &parked, &rebuilt_catalog), Ok(()));

        for (ordinal, uid) in [(0, 2), (1, 3), (2, 4)] {
            let completed = apply_action(
                &parked,
                &rebuilt_catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(ordinal),
                },
            )
            .unwrap()
            .state;
            let selected = completed.card_states.get(uid);
            assert_eq!(
                selected.local_cost_modifiers.as_slice(),
                [crate::hot::LocalCostModifier {
                    kind: crate::hot::LocalCostModifierKind::Set,
                    amount: 0,
                    expiration: crate::hot::LocalCostExpiration::ThisCombat,
                    reduce_only: false,
                }]
            );
            assert!(selected.local_cost_modifiers.free_star_cost_this_combat());
            for sibling in [1, 2, 3, 4, 5] {
                if sibling != uid {
                    assert_eq!(completed.card_states.get(sibling), Default::default());
                }
            }
            assert_eq!(completed.fanouts.potion_slots(), [None]);
        }
    }

    #[test]
    fn cure_energy_precedes_resumable_draw_and_terminal_or_overflow_finishes_atomically() {
        let mut builder = CatalogBuilder::new();
        let pommel = builder
            .intern(plain_identity(CardId::PommelStrike, 0))
            .unwrap();
        let seeker = builder
            .intern(plain_identity(CardId::SeekerStrike, 0))
            .unwrap();
        let defend = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 7;
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        for stream in [crate::hot::RngStream::Sel, crate::hot::RngStream::Targets] {
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: pommel,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: seeker,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 6,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 3,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::CureAll)],
            false,
            false,
            false,
            false,
            true,
        ));
        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&predecessor).unwrap();
        let state = HotBoundary::from_canonical(&predecessor, &catalog).unwrap();
        assert_eq!(state.powers.value(PowerId::Hellraiser), 1);
        assert!(catalog.requires_action_replay());
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert_eq!(parked.energy, 4);
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::PotionFinish { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. }
                ]
            ),
            "frames={:#?} drawn={} hand={:?} draw={:?}",
            parked.frames,
            parked.cards_drawn_combat,
            parked.piles.get(PileId::Hand).as_slice(),
            parked.piles.get(PileId::Draw).as_slice()
        );
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let parked = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &parked, &rebuilt_catalog), Ok(()));
        let completed = apply_action(
            &parked,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert_eq!(completed.energy, 4);
        assert_eq!(completed.cards_drawn_combat, 3);
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.fanouts.potion_slots(), [None]);

        let mut overflow = state.clone();
        overflow.energy = i16::MAX;
        let before = overflow.clone();
        let mut events = vec![Event::Reshuffled { cards: 999 }];
        assert_eq!(
            apply_action_into(
                &overflow,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("energy"))
        );
        assert_eq!(overflow, before);
        assert!(events.is_empty());

        let mut terminal = state;
        terminal.powers.set(PowerId::Hellraiser, SlotWire::Int, 0);
        terminal.powers.set(PowerId::Cacophony, SlotWire::Int, 1);
        terminal.fanouts.set_cacophony_left(1);
        assert!(
            terminal
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Cacophony])
        );
        terminal.monsters_mut()[0].hp = 1;
        terminal.monsters_mut()[0].max_hp = 1;
        terminal.piles.get_mut(PileId::Draw).make_mut()[0].atom = defend;
        let terminal = apply_action(
            &terminal,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(terminal.history.over);
        assert_eq!(terminal.energy, 4);
        assert_eq!(terminal.cards_drawn_combat, 1);
        assert!(terminal.pending.is_none() && terminal.frames.is_empty());
        assert_eq!(terminal.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn clarity_draws_then_stacks_three_but_lethal_draw_suppresses_the_add() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let sculpting = builder
            .intern(plain_identity(CardId::SculptingStrike, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 3;
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 1,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::Clarity)],
            false,
            false,
            false,
            false,
            true,
        ));
        assert!(state.fanouts.set_clarity(2));

        let completed = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert_eq!(completed.cards_drawn_combat, 1);
        assert_eq!(completed.piles.get(PileId::Hand).len(), 1);
        assert_eq!(completed.fanouts.clarity(), 5);
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.fanouts.potion_slots(), [None]);
        let document = HotBoundary::try_to_canonical(&completed, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(rebuilt.fanouts.clarity(), 5);
        assert_eq!(admit(&document, &rebuilt, &rebuilt_catalog), Ok(()));

        let mut terminal = state.clone();
        terminal.powers.set(PowerId::Cacophony, SlotWire::Int, 1);
        terminal.fanouts.set_cacophony_left(1);
        assert!(
            terminal
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Cacophony])
        );
        terminal.monsters_mut()[0].hp = 1;
        terminal.monsters_mut()[0].max_hp = 1;
        let terminal = apply_action(
            &terminal,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(terminal.history.over);
        assert_eq!(terminal.cards_drawn_combat, 1);
        assert_eq!(terminal.fanouts.clarity(), 2);
        assert!(terminal.pending.is_none() && terminal.frames.is_empty());
        assert_eq!(terminal.fanouts.potion_slots(), [None]);

        let mut serial = state.clone();
        serial.piles.get_mut(PileId::Draw).make_mut().clear();
        assert!(serial.fanouts.set_potion_belt(
            vec![Some(PotionId::Clarity), Some(PotionId::Clarity)],
            false,
            false,
            false,
            false,
            true,
        ));
        assert!(serial.fanouts.set_clarity(2));
        let serial_document = HotBoundary::try_to_canonical(&serial, &catalog).unwrap();
        assert_eq!(admit(&serial_document, &serial, &catalog), Ok(()));
        for slot in [0, 1] {
            serial = apply_action(&serial, &catalog, &Action::UsePotion { slot, target: None })
                .unwrap()
                .state;
        }
        assert_eq!(serial.fanouts.clarity(), 8);
        assert_eq!(serial.fanouts.potion_slots(), [None, None]);

        let mut serial_overflow = state.clone();
        assert!(serial_overflow.fanouts.set_potion_belt(
            vec![Some(PotionId::Clarity), Some(PotionId::Clarity)],
            false,
            false,
            false,
            false,
            true,
        ));
        assert!(serial_overflow.fanouts.set_clarity(i32::MAX - 4));
        let serial_document = HotBoundary::try_to_canonical(&serial_overflow, &catalog).unwrap();
        assert!(
            admit(&serial_document, &serial_overflow, &catalog)
                .unwrap_err()
                .contains(admission::MissingCapability::ArgumentShape(
                    "Clarity amount"
                ))
        );

        let mut overflow = state;
        assert!(overflow.fanouts.set_clarity(i32::MAX - 1));
        overflow.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        overflow.piles.get_mut(PileId::Draw).make_mut()[0].atom = sculpting;
        overflow
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(HotCard {
                uid: 2,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        overflow.next_card_uid = 3;
        let before = overflow.clone();
        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        let predecessor = HotBoundary::try_to_canonical(&overflow, &catalog).unwrap();
        assert!(
            admit(&predecessor, &overflow, &catalog)
                .unwrap_err()
                .contains(admission::MissingCapability::ArgumentShape(
                    "Clarity amount"
                ))
        );
        assert!(!legal_actions(&overflow, &catalog).contains(&action));
        let mut events = vec![Event::Reshuffled { cards: 999 }];
        assert_eq!(
            apply_action_into(&overflow, &catalog, &action, &mut events),
            Err(EngineRefusal::CounterOverflow("clarity"))
        );
        assert_eq!(overflow, before);
        assert!(events.is_empty());
    }

    #[test]
    fn clarity_recursive_cardplay_draw_roundtrips_resumes_and_rolls_back() {
        let mut builder = CatalogBuilder::new();
        let pommel = builder
            .intern(plain_identity(CardId::PommelStrike, 0))
            .unwrap();
        let seeker = builder
            .intern(plain_identity(CardId::SeekerStrike, 0))
            .unwrap();
        let defend = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 6;
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        for stream in [crate::hot::RngStream::Sel, crate::hot::RngStream::Targets] {
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: pommel,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: seeker,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::Clarity)],
            false,
            false,
            false,
            false,
            true,
        ));
        assert!(state.fanouts.set_clarity(2));

        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&predecessor).unwrap();
        let state = HotBoundary::from_canonical(&predecessor, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::PotionFinish { .. },
                crate::frame::Frame::Draw { .. },
                crate::frame::Frame::CardPlay { .. },
                crate::frame::Frame::Draw { .. },
                crate::frame::Frame::CardPlay { .. },
            ]
        ));
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let parked = HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert_eq!(admit(&document, &parked, &catalog), Ok(()));

        let checkpoint = parked.clone();
        let mut events = vec![Event::Reshuffled { cards: 999 }];
        assert!(
            apply_action_into(
                &parked,
                &catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(99),
                },
                &mut events,
            )
            .is_err()
        );
        assert_eq!(parked, checkpoint);
        assert!(events.is_empty());

        let completed = apply_action(
            &parked,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.cards_drawn_combat, 2);
        assert_eq!(completed.fanouts.clarity(), 5);
        assert_eq!(completed.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn public_demon_clarity_prep_applications_record_first_order_and_restacks_do_not_move() {
        let mut builder = CatalogBuilder::new();
        let demon = builder
            .intern_reachable(plain_identity(CardId::DemonForm, 0))
            .unwrap();
        let prep = builder
            .intern_reachable(plain_identity(CardId::PrepTime, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 10;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 3;
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: demon,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: prep,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        state.monsters_mut().push(monster);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::Clarity), Some(PotionId::Clarity)],
            false,
            false,
            false,
            false,
            true,
        ));

        state = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        state = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        state = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 2,
                target: None,
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(
            &*state.after_side_turn_start_power_order().unwrap(),
            &[
                crate::hot::AfterSideTurnStartToken::DemonForm,
                crate::hot::AfterSideTurnStartToken::Clarity,
                crate::hot::AfterSideTurnStartToken::PrepTime,
            ]
        );

        // A second Clarity application stacks the existing object in place.
        state = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 1,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert_eq!(state.fanouts.clarity(), 6);
        assert_eq!(
            &*state.after_side_turn_start_power_order().unwrap(),
            &[
                crate::hot::AfterSideTurnStartToken::DemonForm,
                crate::hot::AfterSideTurnStartToken::Clarity,
                crate::hot::AfterSideTurnStartToken::PrepTime,
            ]
        );
    }

    #[test]
    fn snecko_full_legacy_hand_normalizes_distinct_uids_before_exact_cost_rolls() {
        let strike = plain_identity(CardId::StrikeIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(strike).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = false;
        state.next_card_uid = 0;
        state.piles.set(
            PileId::Hand,
            crate::hot::HotPile::from_cards(
                (0..10)
                    .map(|_| HotCard {
                        uid: LEGACY_CARD_UID,
                        atom,
                        flags: CARD_FLAG_LEGACY,
                    })
                    .collect(),
            ),
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::SneckoOil)],
            false,
            false,
            false,
            false,
            true,
        ));

        let successor = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(successor.frames.is_empty());
        assert!(successor.pending.is_none());
        assert!(successor.exact_piles);
        assert_eq!(successor.next_card_uid, 10);
        assert_eq!(
            successor
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            (0..10).collect::<Vec<_>>()
        );
        assert_eq!(
            successor
                .rng
                .get(crate::hot::RngStream::EnergyCosts)
                .counter,
            10
        );
        assert!(
            successor
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .all(|card| successor
                    .card_states
                    .get(card.uid)
                    .local_cost_modifiers
                    .as_slice()
                    .len()
                    == 1)
        );

        let document = HotBoundary::try_to_canonical(&successor, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
            document
        );
        admit(&document, &rebuilt, &rebuilt_catalog).unwrap();
    }

    #[test]
    fn snecko_partial_and_reshuffled_legacy_draws_normalize_only_before_suffix() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        let defend = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let catalog = builder.build();
        let legacy = |atom| HotCard {
            uid: LEGACY_CARD_UID,
            atom,
            flags: CARD_FLAG_LEGACY,
        };
        let potion_state = || {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = 0;
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::SneckoOil)],
                false,
                false,
                false,
                false,
                true,
            ));
            state
        };

        let mut partial = potion_state();
        partial.piles.set(
            PileId::Hand,
            crate::hot::HotPile::from_cards(vec![legacy(strike)]),
        );
        partial.piles.set(
            PileId::Draw,
            crate::hot::HotPile::from_cards(vec![legacy(defend), legacy(defend)]),
        );
        let partial = apply_action(
            &partial,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(partial.frames.is_empty() && partial.pending.is_none());
        assert_eq!(partial.next_card_uid, 3);
        assert_eq!(
            partial
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(
            partial.rng.get(crate::hot::RngStream::EnergyCosts).counter,
            3
        );

        let mut reshuffled = potion_state();
        reshuffled.piles.set(
            PileId::Hand,
            crate::hot::HotPile::from_cards(vec![legacy(strike)]),
        );
        reshuffled.piles.set(
            PileId::Discard,
            crate::hot::HotPile::from_cards(vec![legacy(defend), legacy(defend)]),
        );
        let reshuffled = apply_action(
            &reshuffled,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(reshuffled.frames.is_empty() && reshuffled.pending.is_none());
        assert_eq!(reshuffled.next_card_uid, 3);
        assert_eq!(reshuffled.piles.get(PileId::Hand).len(), 3);
        assert_eq!(reshuffled.rng.get(crate::hot::RngStream::Rng).counter, 1);
        assert_eq!(
            reshuffled
                .rng
                .get(crate::hot::RngStream::EnergyCosts)
                .counter,
            3
        );
    }

    #[test]
    fn snecko_selecting_hellraiser_draw_roundtrips_then_rolls_live_hand_once() {
        let mut builder = CatalogBuilder::new();
        let pommel = builder
            .intern(plain_identity(CardId::PommelStrike, 0))
            .unwrap();
        let seeker = builder
            .intern(plain_identity(CardId::SeekerStrike, 0))
            .unwrap();
        let defend = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 10;
        state.exact_piles = true;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        for stream in [crate::hot::RngStream::Sel, crate::hot::RngStream::Targets] {
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 8,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend(
            std::iter::once(HotCard {
                uid: 1,
                atom: pommel,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            })
            .chain(std::iter::once(HotCard {
                uid: 2,
                atom: seeker,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }))
            .chain([3, 4, 5, 6, 7, 9].into_iter().map(|uid| HotCard {
                uid,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            })),
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::SneckoOil)],
            false,
            false,
            false,
            false,
            true,
        ));
        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&predecessor).unwrap();
        let state = HotBoundary::from_canonical(&predecessor, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));

        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::PotionFinish { .. },
                crate::frame::Frame::Draw { .. },
                crate::frame::Frame::CardPlay { .. },
                crate::frame::Frame::Draw { .. },
                crate::frame::Frame::CardPlay { .. }
            ]
        ));
        assert_eq!(
            parked.rng.get(crate::hot::RngStream::EnergyCosts).counter,
            0
        );
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let parked = HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert_eq!(admit(&document, &parked, &catalog), Ok(()));

        let completed = apply_action(
            &parked,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.cards_drawn_combat, 8);
        assert_eq!(completed.piles.get(PileId::Hand).len(), 8);
        assert_eq!(
            completed
                .rng
                .get(crate::hot::RngStream::EnergyCosts)
                .counter,
            8
        );
        assert_eq!(completed.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn snecko_terminal_draw_stops_prefix_then_rolls_the_live_hand() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 4;
        state.exact_piles = true;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1);
        monster.max_hp = 1;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.powers.set(PowerId::Cacophony, SlotWire::Int, 1);
        state.fanouts.set_cacophony_left(1);
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Cacophony])
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 2,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::SneckoOil)],
            false,
            false,
            false,
            false,
            true,
        ));

        let completed = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(completed.history.over);
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.piles.get(PileId::Hand).len(), 2);
        assert_eq!(completed.piles.get(PileId::Draw).len(), 1);
        assert_eq!(completed.cards_drawn_combat, 1);
        assert_eq!(
            completed
                .rng
                .get(crate::hot::RngStream::EnergyCosts)
                .counter,
            2
        );
        assert_eq!(completed.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn gambler_paired_draw_parks_before_sly_then_resumes_both_children_once() {
        let purity = plain_identity(CardId::Purity, 0);
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let sculpting = plain_identity(CardId::SculptingStrike, 0);
        let mut builder = CatalogBuilder::new();
        let purity_atom = builder.intern(purity).unwrap();
        let defend_atom = builder.intern(defend).unwrap();
        let sculpting_atom = builder.intern(sculpting).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 5;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.powers.set(PowerId::MasterPlanner, SlotWire::Int, 1);
        crate::engine::play::hydrate_after_card_played_power_order_for_test(&mut state);
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: purity_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: defend_atom,
                flags: 0,
            },
        ]);
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 4,
            atom: defend_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 3,
            atom: sculpting_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.card_states.set_local_sly(1);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::GamblersBrew)],
            false,
            false,
            false,
            false,
            true,
        ));
        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));

        let selecting = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let draw_parked = legal_actions(&selecting, &catalog)
            .iter()
            .filter_map(|action| apply_action(&selecting, &catalog, action).ok())
            .map(|transition| transition.state)
            .find(|candidate| {
                matches!(
                    candidate.frames.as_slice(),
                    [
                        crate::frame::Frame::ActionReplay { .. },
                        crate::frame::Frame::PotionFinish { .. },
                        crate::frame::Frame::Draw { .. },
                        crate::frame::Frame::CardPlay { .. },
                    ]
                ) && match candidate.frames.as_slice()[1] {
                    crate::frame::Frame::PotionFinish { record } => candidate
                        .frames
                        .potion_finish(record)
                        .is_some_and(|finish| finish.candidates().map(|card| card.uid).eq([1])),
                    _ => false,
                }
            })
            .expect("Gambler selection parks in the paired Draw's Hellraiser child");
        let finish = match draw_parked.frames.as_slice()[1] {
            crate::frame::Frame::PotionFinish { record } => {
                draw_parked.frames.potion_finish(record).unwrap()
            }
            _ => unreachable!(),
        };
        assert_eq!(
            finish.candidates().map(|card| card.uid).collect::<Vec<_>>(),
            [1]
        );
        assert_eq!(
            play::persisted_card_play_stack_is_exact(&draw_parked, &catalog),
            Ok(())
        );
        let document = HotBoundary::try_to_canonical(&draw_parked, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let draw_parked = HotBoundary::from_canonical(&document, &catalog).unwrap();
        admit(&document, &draw_parked, &catalog).unwrap();

        let after_draw_child = apply_action(
            &draw_parked,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(matches!(
            after_draw_child.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::PotionFinish { .. },
                crate::frame::Frame::FrozenAutoBatch { .. },
                crate::frame::Frame::CardPlay { .. },
            ]
        ));
        let document = HotBoundary::try_to_canonical(&after_draw_child, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let after_draw_child = HotBoundary::from_canonical(&document, &catalog).unwrap();
        admit(&document, &after_draw_child, &catalog).unwrap();

        let completed = apply_action(
            &after_draw_child,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none());
        assert!(completed.frames.is_empty());
        assert_eq!(completed.fanouts.potion_slots(), [None]);
        assert_eq!(completed.history.discarded_cards_this_turn, 1);
        assert_eq!(completed.cards_drawn_combat, 1);
    }

    #[test]
    fn gambler_payload_equal_sly_permutation_roundtrips_at_second_cursor() {
        let purity = plain_identity(CardId::Purity, 0);
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let purity_atom = builder.intern(purity).unwrap();
        let defend_atom = builder.intern(defend).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 5;
        state.powers.set(PowerId::MasterPlanner, SlotWire::Int, 2);
        crate::engine::play::hydrate_after_card_played_power_order_for_test(&mut state);
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: purity_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: purity_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 3,
                atom: defend_atom,
                flags: 0,
            },
            HotCard {
                uid: 4,
                atom: defend_atom,
                flags: 0,
            },
        ]);
        state.card_states.set_local_sly(1);
        state.card_states.set_local_sly(2);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::GamblersBrew)],
            false,
            false,
            false,
            false,
            true,
        ));

        let selecting = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let first = legal_actions(&selecting, &catalog)
            .iter()
            .filter_map(|action| apply_action(&selecting, &catalog, action).ok())
            .map(|transition| transition.state)
            .find(|candidate| match candidate.frames.as_slice().get(2) {
                Some(crate::frame::Frame::FrozenAutoBatch { record }) => candidate
                    .frames
                    .frozen_auto_batch(*record)
                    .is_some_and(|batch| batch.uids().eq([2, 1])),
                _ => false,
            })
            .expect("the exact relative Sly permutation [uid 2, uid 1] is legal");
        let batch = match first.frames.as_slice()[2] {
            crate::frame::Frame::FrozenAutoBatch { record } => {
                first.frames.frozen_auto_batch(record).unwrap()
            }
            _ => unreachable!(),
        };
        assert_eq!(batch.cursor, 1);
        let document = HotBoundary::try_to_canonical(&first, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let first = HotBoundary::from_canonical(&document, &catalog).unwrap();
        admit(&document, &first, &catalog).unwrap();

        let second = apply_action(
            &first,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        let batch = match second.frames.as_slice()[2] {
            crate::frame::Frame::FrozenAutoBatch { record } => {
                second.frames.frozen_auto_batch(record).unwrap()
            }
            _ => unreachable!(),
        };
        assert_eq!(batch.uids().collect::<Vec<_>>(), [2, 1]);
        assert_eq!(batch.cursor, 2);
        let document = HotBoundary::try_to_canonical(&second, &catalog).unwrap();
        let canonical_refuses = |tampered: &CanonicalStateV2| {
            let Ok(catalog) = HotBoundary::catalog_from_canonical(tampered) else {
                return true;
            };
            match HotBoundary::from_canonical(tampered, &catalog) {
                Err(_) => true,
                Ok(state) => admit(tampered, &state, &catalog).is_err(),
            }
        };
        let mut swapped = document.clone();
        swapped.continuations[2]
            .fields
            .get_mut("entries")
            .and_then(serde_json::Value::as_array_mut)
            .unwrap()
            .reverse();
        assert!(canonical_refuses(&swapped));
        let mut wrong_parent = document.clone();
        wrong_parent.continuations[1]
            .fields
            .insert("name".to_owned(), serde_json::Value::from("Clarity"));
        assert!(canonical_refuses(&wrong_parent));
        let mut stale_cursor = document.clone();
        stale_cursor.continuations[2]
            .fields
            .insert("cursor".to_owned(), serde_json::Value::from(0));
        assert!(canonical_refuses(&stale_cursor));
        let mut wrong_answer = document.clone();
        wrong_answer.continuations[0]
            .fields
            .get_mut("answers")
            .and_then(serde_json::Value::as_array_mut)
            .and_then(|answers| answers.first_mut())
            .and_then(serde_json::Value::as_object_mut)
            .unwrap()
            .insert("value".to_owned(), serde_json::Value::from(0));
        assert!(canonical_refuses(&wrong_answer));

        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let second = HotBoundary::from_canonical(&document, &catalog).unwrap();
        admit(&document, &second, &catalog).unwrap();

        let completed = apply_action(
            &second,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.frames.is_empty());
        assert!(completed.pending.is_none());
        assert_eq!(completed.history.discarded_cards_this_turn, 2);
        assert_eq!(completed.cards_drawn_combat, 2);
    }

    #[test]
    fn gambler_lethal_paired_draw_skips_sly_tail_and_finishes() {
        let flick = plain_identity(CardId::FlickFlack, 0);
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let flick_atom = builder.intern(flick).unwrap();
        let defend_atom = builder.intern(defend).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 3;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1);
        monster.max_hp = 1;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.powers.set(PowerId::Cacophony, SlotWire::Int, 1);
        state.fanouts.set_cacophony_left(1);
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Cacophony])
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: flick_atom,
            flags: 0,
        });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 2,
            atom: defend_atom,
            flags: 0,
        });
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::GamblersBrew)],
            false,
            false,
            false,
            false,
            true,
        ));
        let selecting = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let completed = legal_actions(&selecting, &catalog)
            .iter()
            .filter_map(|action| apply_action(&selecting, &catalog, action).ok())
            .map(|transition| transition.state)
            .find(|candidate| candidate.history.discarded_cards_this_turn == 1)
            .unwrap();
        assert!(completed.history.over);
        assert!(completed.frames.is_empty());
        assert!(completed.pending.is_none());
        assert_eq!(completed.history.owner_card_plays_finished_this_turn, 0);
        assert_eq!(completed.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn gambler_first_sly_lethal_stops_later_sibling_and_finishes() {
        let flick = plain_identity(CardId::FlickFlack, 0);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(flick).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 3;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1);
        monster.max_hp = 1;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom,
                flags: 0,
            },
        ]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::GamblersBrew)],
            false,
            false,
            false,
            false,
            true,
        ));
        let selecting = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let completed = legal_actions(&selecting, &catalog)
            .iter()
            .filter_map(|action| apply_action(&selecting, &catalog, action).ok())
            .map(|transition| transition.state)
            .find(|candidate| {
                candidate.history.discarded_cards_this_turn == 2 && candidate.history.over
            })
            .unwrap();
        assert!(completed.frames.is_empty());
        assert!(completed.pending.is_none());
        assert_eq!(completed.history.owner_card_plays_finished_this_turn, 1);
        assert_eq!(completed.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn ashwater_exhaust_adds_one_physical_card_and_one_event() {
        let (mut state, catalog) = fixture();
        state.piles.get_mut(PileId::Hand).make_mut().truncate(1);
        let uid = state.piles.get(PileId::Hand).as_slice()[0].uid;
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::Ashwater)],
            false,
            false,
            false,
            false,
            true,
        ));
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let completed = apply_action(
            &parked,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(1),
            },
        )
        .unwrap();
        let physical_count = PileId::ALL
            .into_iter()
            .flat_map(|pile| completed.state.piles.get(pile).as_slice())
            .filter(|card| card.uid == uid)
            .count();
        assert_eq!(physical_count, 1);
        assert_eq!(
            completed
                .events
                .iter()
                .filter(|event| matches!(event, Event::CardResolved { uid: event_uid, pile: PileId::Exhaust } if *event_uid == uid))
                .count(),
            1
        );
    }

    #[test]
    fn ashwater_admits_both_fnp_dark_orders_and_applies_each_once() {
        let (mut state, catalog) = fixture();
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        state.powers.set(PowerId::FeelNoPain, SlotWire::Int, 2);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::Ashwater)],
            false,
            false,
            false,
            false,
            true,
        ));
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::FeelNoPain, PowerId::DarkEmbrace])
        );
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&document, &state, &catalog), Ok(()));
        let selecting = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let completed = apply_action(
            &selecting,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(1),
            },
        )
        .unwrap()
        .state;
        assert_eq!(completed.block, state.block + 2);

        let mut fnp_only = state.clone();
        fnp_only.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 0);
        assert!(
            fnp_only
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::FeelNoPain])
        );
        let fnp_only = apply_action(
            &fnp_only,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let fnp_only = apply_action(
            &fnp_only,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(1),
            },
        )
        .unwrap()
        .state;
        assert_eq!(fnp_only.block, state.block + 2);
        assert!(fnp_only.frames.is_empty());

        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::DarkEmbrace, PowerId::FeelNoPain])
        );
        assert_eq!(
            admit(
                &HotBoundary::try_to_canonical(&state, &catalog).unwrap(),
                &state,
                &catalog
            ),
            Ok(())
        );
    }

    #[test]
    fn ashwater_drum_tail_folds_replay_burst_echo_and_rolls_back_overflow() {
        let drum = plain_identity(CardId::DrumOfBattle, 0);
        let mut builder = CatalogBuilder::new();
        let drum_atom = builder.intern(drum).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 1;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 2;
        state.history.plays_this_turn = 1;
        state.powers.set(PowerId::Burst, SlotWire::Int, 2);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.powers.set(PowerId::EchoForm, SlotWire::Int, 2);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: drum_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        let mut physical = state.card_states.get(1);
        physical.set_base_replay_count(Some(2)).unwrap();
        state.card_states.set(1, physical);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::Ashwater)],
            false,
            false,
            false,
            false,
            true,
        ));

        let selecting = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let completed = apply_action(
            &selecting,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(1),
            },
        )
        .unwrap()
        .state;
        assert_eq!(completed.energy, 11);
        assert_eq!(completed.powers.value(PowerId::Burst), 1);
        assert_eq!(completed.powers.value(PowerId::EchoForm), 2);
        assert_eq!(completed.history.owner_cards_exhausted_combat, 1);
        assert!(completed.frames.is_empty());

        let mut overflow = state;
        overflow.energy = i16::MAX;
        let selecting = apply_action(
            &overflow,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let before = selecting.clone();
        assert_eq!(
            apply_action(
                &selecting,
                &catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(1),
                },
            ),
            Err(EngineRefusal::CounterOverflow("energy"))
        );
        assert_eq!(selecting, before);
    }

    #[test]
    fn ashwater_lethal_dark_draw_suppresses_drum_tail() {
        let drum = plain_identity(CardId::DrumOfBattle, 0);
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let drum_atom = builder.intern(drum).unwrap();
        let defend_atom = builder.intern(defend).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = i16::MAX;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 3;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1);
        monster.max_hp = 1;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        state.powers.set(PowerId::Cacophony, SlotWire::Int, 1);
        state.fanouts.set_cacophony_left(1);
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
        );
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Cacophony])
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: drum_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 2,
            atom: defend_atom,
            flags: 0,
        });
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::Ashwater)],
            false,
            false,
            false,
            false,
            true,
        ));
        let selecting = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let completed = apply_action(
            &selecting,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(1),
            },
        )
        .unwrap()
        .state;
        assert!(completed.history.over);
        assert_eq!(completed.energy, i16::MAX);
        assert!(completed.frames.is_empty());
        assert_eq!(completed.fanouts.potion_slots(), [None]);
    }

    /// Two payload-equal Defends in Hand, Feel No Pain before Dark Embrace, and
    /// a Draw pile whose Dark Embrace draw reaches a selecting Seeker Strike
    /// through Hellraiser: an Ashwater two-card answer parks mid-sequence.
    fn ashwater_fnp_dark_seeker_root() -> (HotState, Catalog) {
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let pommel = plain_identity(CardId::PommelStrike, 0);
        let seeker = plain_identity(CardId::SeekerStrike, 0);
        let mut builder = CatalogBuilder::new();
        let defend_atom = builder.intern(defend).unwrap();
        let pommel_atom = builder.intern(pommel).unwrap();
        let seeker_atom = builder.intern(seeker).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 8;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        state.powers.set(PowerId::FeelNoPain, SlotWire::Int, 3);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        for stream in [crate::hot::RngStream::Sel, crate::hot::RngStream::Targets] {
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::FeelNoPain, PowerId::DarkEmbrace])
        );
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: defend_atom,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom: defend_atom,
                flags: 0,
            },
        ]);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 3,
                atom: pommel_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: seeker_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: defend_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 6,
                atom: defend_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 7,
                atom: defend_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::Ashwater)],
            false,
            false,
            false,
            false,
            true,
        ));
        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&predecessor).unwrap();
        let state = HotBoundary::from_canonical(&predecessor, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));
        (state, catalog)
    }

    #[test]
    fn ashwater_fnp_first_dark_draw_roundtrips_without_double_block() {
        let (state, catalog) = ashwater_fnp_dark_seeker_root();
        let selecting = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let parked = legal_actions(&selecting, &catalog)
            .iter()
            .filter_map(|action| apply_action(&selecting, &catalog, action).ok())
            .map(|transition| transition.state)
            .find(|candidate| {
                matches!(
                    candidate.frames.as_slice(),
                    [
                        crate::frame::Frame::ActionReplay { .. },
                        crate::frame::Frame::PotionFinish { .. },
                        crate::frame::Frame::Draw { .. },
                        crate::frame::Frame::CardPlay { .. },
                        crate::frame::Frame::Draw { .. },
                        crate::frame::Frame::CardPlay { .. },
                    ]
                ) && match candidate.frames.as_slice()[1] {
                    crate::frame::Frame::PotionFinish { record } => candidate
                        .frames
                        .potion_finish(record)
                        .is_some_and(|finish| {
                            finish.current_uid == Some(1)
                                && finish.body_stage
                                    == crate::hot::PotionBodyStage::AshwaterAfterFnp
                                && finish.candidates().map(|card| card.uid).eq([2])
                        }),
                    _ => false,
                }
            })
            .expect("two-card Ashwater selection parks in Dark Embrace Draw");
        assert_eq!(parked.history.owner_cards_exhausted_combat, 1);
        assert_eq!(parked.block, 3);
        assert_eq!(
            play::persisted_card_play_stack_is_exact(&parked, &catalog),
            Ok(())
        );
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        assert_eq!(document.continuations[2].fields["caller_locals"][3], true);
        let mut replay_marker = document.clone();
        replay_marker.continuations[2]
            .fields
            .get_mut("caller_locals")
            .unwrap()[3] = serde_json::json!(false);
        let replay_catalog = HotBoundary::catalog_from_canonical(&replay_marker).unwrap();
        assert!(HotBoundary::from_canonical(&replay_marker, &replay_catalog).is_err());
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let parked = HotBoundary::from_canonical(&document, &catalog).unwrap();
        admit(&document, &parked, &catalog).unwrap();

        let completed = apply_action(
            &parked,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.frames.is_empty());
        assert!(completed.pending.is_none());
        assert_eq!(completed.history.owner_cards_exhausted_combat, 2);
        assert_eq!(completed.block, 6);
        assert_eq!(
            completed
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .filter(|card| matches!(card.uid, 1 | 2))
                .count(),
            2
        );
        assert_eq!(completed.fanouts.potion_slots(), [None]);
    }

    /// The reversed pick of two payload-equal Defends is an ordered-extension
    /// answer (#2524): not offered, but applied exactly, recorded in the
    /// parked ActionReplay receipt, re-authenticated on cold import, and
    /// completed with the Exhaust pile in pick order.
    #[test]
    fn ashwater_reversed_extension_answer_parks_reloads_and_exhausts_in_pick_order() {
        let (state, catalog) = ashwater_fnp_dark_seeker_root();
        let selecting = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let forward = selection::ordered_selection_answer(&selecting, &catalog, &[1, 2])
            .unwrap()
            .unwrap();
        let reversed = selection::ordered_selection_answer(&selecting, &catalog, &[2, 1])
            .unwrap()
            .unwrap();
        let offered = legal_actions(&selecting, &catalog);
        assert!(offered.contains(&Action::Select { answer: forward }));
        assert!(!offered.contains(&Action::Select { answer: reversed }));
        let parked = apply_action(&selecting, &catalog, &Action::Select { answer: reversed })
            .unwrap()
            .state;
        let Some(crate::frame::Frame::PotionFinish { record }) = parked.frames.as_slice().get(1)
        else {
            panic!("the reversed answer parks under Ashwater: {parked:#?}")
        };
        let finish = parked.frames.potion_finish(*record).unwrap();
        assert_eq!(finish.current_uid, Some(2), "uid 2 exhausts first");
        assert!(finish.candidates().map(|card| card.uid).eq([1]));
        assert_eq!(
            parked
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [2]
        );
        // The receipt carries the extension ordinal; cold import accepts it.
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt = HotBoundary::catalog_from_canonical(&document).unwrap();
        let cold = HotBoundary::from_canonical(&document, &rebuilt).unwrap();
        admit(&document, &cold, &rebuilt).unwrap();
        assert_eq!(cold, parked);
        let completed = apply_action(
            &cold,
            &rebuilt,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.frames.is_empty());
        assert_eq!(completed.block, 6);
        assert_eq!(
            completed
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .filter(|uid| matches!(uid, 1 | 2))
                .collect::<Vec<_>>(),
            [2, 1],
            "Exhaust records the pick order"
        );
    }

    #[test]
    fn ashwater_midnight_snapshot_may_overlap_remaining_and_tracks_live_mutation() {
        let midnight = plain_identity(CardId::Midnight, 0);
        let sculpting = plain_identity(CardId::SculptingStrike, 0);
        let mut builder = CatalogBuilder::new();
        let midnight_atom = builder.intern(midnight).unwrap();
        let sculpting_atom = builder.intern(sculpting).unwrap();
        let defend_atom = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 11;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
        );
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: midnight_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: midnight_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 10,
            atom: defend_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 3,
            atom: sculpting_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::Ashwater)],
            false,
            false,
            false,
            false,
            true,
        ));
        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));

        let selecting = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let parked = legal_actions(&selecting, &catalog)
            .iter()
            .filter_map(|action| apply_action(&selecting, &catalog, action).ok())
            .map(|transition| transition.state)
            .find(|candidate| {
                matches!(
                    candidate.frames.as_slice(),
                    [
                        crate::frame::Frame::ActionReplay { .. },
                        crate::frame::Frame::PotionFinish { .. },
                        crate::frame::Frame::Draw { .. },
                        crate::frame::Frame::CardPlay { .. },
                    ]
                ) && match candidate.frames.as_slice()[1] {
                    crate::frame::Frame::PotionFinish { record } => candidate
                        .frames
                        .potion_finish(record)
                        .is_some_and(|finish| {
                            finish.current_uid == Some(1)
                                && finish.aux == 2
                                && finish.candidates().map(|card| card.uid).eq([2, 1, 2])
                        }),
                    _ => false,
                }
            })
            .expect("Midnight snapshot overlaps the remaining selected suffix");

        // A Midnight entering during the awaited child is not in the frozen
        // prefix: its AfterEnteredCombat backfill observes the already-
        // incremented Exhaust history. The synthetic extra card intentionally
        // breaks the replay transcript, but must remain individually valid at
        // the full subtotal rather than inheriting the prefix's history-1.
        let mut entered_during_child = parked.clone();
        entered_during_child
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .push(HotCard {
                uid: 4,
                atom: midnight_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        entered_during_child.next_card_uid = 11;
        entered_during_child.card_states.append_local_cost_modifier(
            4,
            crate::hot::LocalCostModifier {
                kind: crate::hot::LocalCostModifierKind::Add,
                amount: -1,
                expiration: crate::hot::LocalCostExpiration::ThisCombat,
                reduce_only: false,
            },
        );
        let entered_document =
            HotBoundary::try_to_canonical(&entered_during_child, &catalog).unwrap();
        let entered_refusal =
            admit(&entered_document, &entered_during_child, &catalog).unwrap_err();
        assert!(!entered_refusal.contains(admission::MissingCapability::CardInstanceState(4)));
        let mut missing_backfill = entered_during_child;
        missing_backfill
            .card_states
            .set(4, crate::hot::CardInstanceState::default());
        let missing_document = HotBoundary::try_to_canonical(&missing_backfill, &catalog).unwrap();
        assert!(
            admit(&missing_document, &missing_backfill, &catalog)
                .unwrap_err()
                .contains(admission::MissingCapability::CardInstanceState(4))
        );

        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        assert_eq!(document.continuations[2].fields["caller_locals"][3], false);
        let mut skipped_fnp = document.clone();
        skipped_fnp.continuations[2]
            .fields
            .get_mut("caller_locals")
            .unwrap()[3] = serde_json::json!(true);
        let skipped_catalog = HotBoundary::catalog_from_canonical(&skipped_fnp).unwrap();
        assert!(HotBoundary::from_canonical(&skipped_fnp, &skipped_catalog).is_err());
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let parked = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        admit(&document, &parked, &rebuilt_catalog).unwrap();

        let after_sculpting = legal_actions(&parked, &rebuilt_catalog)
            .iter()
            .filter_map(|action| apply_action(&parked, &rebuilt_catalog, action).ok())
            .map(|transition| transition.state)
            .find(|candidate| candidate.card_states.get(2).local_ethereal())
            // #3022: Sculpting Strike applies Ethereal, not Retain.
            .expect("Sculpting Strike can make the remaining Midnight Ethereal");
        assert!(after_sculpting.frames.is_empty());
        assert!(after_sculpting.pending.is_none());
        assert_eq!(after_sculpting.history.owner_cards_exhausted_combat, 2);
        assert_eq!(after_sculpting.fanouts.potion_slots(), [None]);
        assert!(after_sculpting.card_states.get(2).local_ethereal());
        assert!(!after_sculpting.card_states.get(2).local_retain);
        assert_eq!(
            after_sculpting
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .filter(|uid| matches!(uid, 1 | 2))
                .count(),
            2
        );
        assert_eq!(
            after_sculpting
                .card_states
                .get(2)
                .local_cost_modifiers
                .as_slice()
                .iter()
                .filter(|row| {
                    row.kind == crate::hot::LocalCostModifierKind::Add
                        && row.amount == -1
                        && row.expiration == crate::hot::LocalCostExpiration::ThisCombat
                })
                .count(),
            2,
            "the remaining Midnight observes both serial exhaust snapshots"
        );
    }

    #[test]
    fn ashwater_fnp_juggernaut_lethal_skips_dark_draw_and_leaves_remaining_in_hand() {
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(defend).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.next_card_uid = 4;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1);
        monster.max_hp = 1;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        state.powers.set(PowerId::FeelNoPain, SlotWire::Int, 1);
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::FeelNoPain, PowerId::DarkEmbrace])
        );
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom,
                flags: 0,
            },
            HotCard {
                uid: 2,
                atom,
                flags: 0,
            },
        ]);
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 3,
            atom,
            flags: 0,
        });
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::Ashwater)],
            false,
            false,
            false,
            false,
            true,
        ));
        let selecting = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let completed = legal_actions(&selecting, &catalog)
            .iter()
            .filter_map(|action| apply_action(&selecting, &catalog, action).ok())
            .map(|transition| transition.state)
            .find(|candidate| {
                candidate.history.over && candidate.history.owner_cards_exhausted_combat == 1
            })
            .unwrap();
        assert!(completed.frames.is_empty());
        assert!(completed.pending.is_none());
        assert!(
            completed
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .any(|card| matches!(card.uid, 1 | 2))
        );
        assert_eq!(
            completed
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .filter(|card| matches!(card.uid, 1 | 2))
                .count(),
            1
        );
        assert_eq!(completed.fanouts.potion_slots(), [None]);
        assert_eq!(
            completed.piles.get(PileId::Draw).as_slice()[0].uid,
            3,
            "terminal Dark Embrace Draw consumes no card"
        );
    }

    fn drink_fixture_potion(
        mut state: HotState,
        catalog: &Catalog,
        potion: PotionId,
        target: Option<u8>,
        belt_buckle: bool,
    ) -> Transition {
        assert!(state.fanouts.set_potion_belt(
            vec![Some(potion)],
            false,
            belt_buckle,
            false,
            false,
            true,
        ));
        apply_action(&state, catalog, &Action::UsePotion { slot: 0, target }).unwrap()
    }

    #[test]
    fn part_b_scalar_and_player_power_bodies_apply_exact_amounts() {
        for potion in [
            PotionId::BloodPotion,
            PotionId::BlockPotion,
            PotionId::DexterityPotion,
            PotionId::EnergyPotion,
            PotionId::FlexPotion,
            PotionId::FocusPotion,
            PotionId::Fortifier,
            PotionId::FyshOil,
            PotionId::GigantificationPotion,
            PotionId::HeartOfIron,
            PotionId::LiquidBronze,
            PotionId::LuckyTonic,
            PotionId::MazalethsGift,
            PotionId::RadiantTincture,
            PotionId::ShipInABottle,
            PotionId::SpeedPotion,
            PotionId::StableSerum,
            PotionId::StrengthPotion,
        ] {
            let (mut state, catalog) = fixture();
            state.hp = 40;
            state.block = if potion == PotionId::Fortifier { 12 } else { 0 };
            let transition = drink_fixture_potion(state, &catalog, potion, None, false);
            let successor = transition.state;
            assert_eq!(successor.fanouts.potion_slots(), [None], "{potion:?}");
            assert!(successor.frames.is_empty(), "{potion:?}");
            match potion {
                PotionId::BloodPotion => assert_eq!(successor.hp, 56),
                PotionId::BlockPotion => assert_eq!(successor.block, 12),
                PotionId::DexterityPotion => {
                    assert_eq!(successor.powers.value(PowerId::Dexterity), 2)
                }
                PotionId::EnergyPotion => assert_eq!(successor.energy, 5),
                PotionId::FlexPotion => assert_eq!(successor.temp_strength, 5),
                PotionId::FocusPotion => {
                    assert_eq!(successor.powers.value(PowerId::Focus), 2);
                    assert_eq!(
                        transition
                            .events
                            .iter()
                            .filter(|event| matches!(
                                event,
                                Event::PowerChanged {
                                    subject: Subject::Player,
                                    power: PowerId::Focus,
                                    amount: 2,
                                }
                            ))
                            .count(),
                        1
                    );
                }
                PotionId::Fortifier => {
                    assert_eq!(successor.block, 36);
                    assert_eq!(
                        transition
                            .events
                            .iter()
                            .filter(|event| matches!(
                                event,
                                Event::PlayerBlockGained { amount: 24, .. }
                            ))
                            .count(),
                        1
                    );
                }
                PotionId::FyshOil => {
                    assert_eq!(successor.powers.value(PowerId::Strength), 1);
                    assert_eq!(successor.powers.value(PowerId::Dexterity), 1);
                }
                PotionId::GigantificationPotion => {
                    assert_eq!(successor.fanouts.gigantification(), 1)
                }
                PotionId::HeartOfIron => assert_eq!(successor.powers.value(PowerId::Plating), 7),
                PotionId::LiquidBronze => assert_eq!(successor.powers.value(PowerId::Thorns), 3),
                PotionId::LuckyTonic => assert_eq!(successor.powers.value(PowerId::Buffer), 1),
                PotionId::MazalethsGift => assert_eq!(successor.fanouts.player_ritual(), 1),
                PotionId::RadiantTincture => {
                    assert_eq!(successor.energy, 4);
                    assert_eq!(successor.fanouts.radiance(), 3);
                }
                PotionId::ShipInABottle => {
                    assert_eq!(successor.block, 10);
                    assert_eq!(successor.powers.value(PowerId::BlockNextTurn), 10);
                }
                PotionId::SpeedPotion => {
                    assert_eq!(successor.powers.value(PowerId::TempDexterity), 5)
                }
                PotionId::StableSerum => {
                    assert_eq!(successor.powers.value(PowerId::RetainHand), 2)
                }
                PotionId::StrengthPotion => {
                    assert_eq!(successor.powers.value(PowerId::Strength), 2)
                }
                _ => unreachable!(),
            }
        }
    }

    /// #2873: Focus Potion (`FocusPotion/<OnUse>d__10::MoveNext` RVA
    /// `0x34dbd4`) admits in a belt, drinks through the public `UsePotion`
    /// path, and its permanent Focus reaches both orb reads.
    ///
    /// The belt is the witness run's (`G9BSN165F970` fights 6 and 7): Liquid
    /// Memories beside Focus Potion, refused before this slice only for
    /// `unsupported potion belt body`.
    #[test]
    fn focus_potion_admits_drinks_publicly_and_feeds_orb_passive_and_evoke() {
        let (mut state, catalog) = fixture();
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::LiquidMemories), Some(PotionId::FocusPotion)],
            false,
            false,
            false,
            false,
            true,
        ));
        let document = HotBoundary::to_canonical(&state, &catalog);
        assert_eq!(admit(&document, &state, &catalog), Ok(()));
        let action = Action::UsePotion {
            slot: 1,
            target: None,
        };
        assert!(legal_actions(&state, &catalog).contains(&action));

        // Target kind 5 never takes an enemy pick; a forged one refuses
        // atomically.
        let before = state.clone();
        let mut events = vec![Event::TurnEnded { turn: 99 }];
        assert_eq!(
            apply_action_into(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 1,
                    target: Some(0),
                },
                &mut events,
            ),
            Err(EngineRefusal::PotionTargetMismatch { required: false })
        );
        assert_eq!(state, before);
        assert!(events.is_empty());

        let successor = apply_action(&state, &catalog, &action).unwrap().state;
        assert_eq!(
            successor.fanouts.potion_slots(),
            [Some(PotionId::LiquidMemories), None]
        );
        assert!(successor.frames.is_empty() && successor.pending.is_none());
        assert_eq!(successor.powers.value(PowerId::Focus), 2);

        // FocusPower::ModifyOrbValue (`0xa27f7`) adds the amount on every
        // read: Frost's passive is 2 + 2 and its evoke is 5 + 2.
        let frost = crate::hot::HotOrb::from_parts(crate::hot::OrbKind::Frost, None).unwrap();
        let mut passive = successor.clone();
        passive.orbs.set_slots(1);
        passive.orbs.set_orbs(vec![frost]);
        let block = passive.block;
        orbs::turn_end_passives(&mut passive, &catalog, &mut Vec::new()).unwrap();
        assert_eq!(passive.block, block + 4);

        let mut evoke = successor.clone();
        evoke.orbs.set_slots(1);
        evoke.orbs.set_orbs(vec![frost]);
        let block = evoke.block;
        orbs::evoke_front(&mut evoke, &catalog, true, &mut Vec::new()).unwrap();
        assert_eq!(evoke.block, block + 7);

        // The write is additive over a signed amount (AllowNegative): -3
        // Focus becomes -1, and the Frost passive clamps at 2 - 1.
        let mut negative = state.clone();
        negative.powers.set(PowerId::Focus, SlotWire::Int, -3);
        let mut negative = apply_action(&negative, &catalog, &action).unwrap().state;
        assert_eq!(negative.powers.value(PowerId::Focus), -1);
        negative.orbs.set_slots(1);
        negative.orbs.set_orbs(vec![frost]);
        let block = negative.block;
        orbs::turn_end_passives(&mut negative, &catalog, &mut Vec::new()).unwrap();
        assert_eq!(negative.block, block + 1);

        // An overflowing stack refuses before any belt or power write.
        let mut overflow = state.clone();
        overflow
            .powers
            .set(PowerId::Focus, SlotWire::Int, i32::MAX - 1);
        let before = overflow.clone();
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(&overflow, &catalog, &action, &mut events),
            Err(EngineRefusal::CounterOverflow("player potion power"))
        );
        assert_eq!(overflow, before);
    }

    #[test]
    fn part_b_flat_block_bodies_apply_shadowmeld_and_native_storage_cap() {
        for (potion, expected_block, expected_next_turn) in [
            (PotionId::BlockPotion, 27, 0),
            (PotionId::Fortifier, 15, 0),
            (PotionId::ShipInABottle, 23, 10),
        ] {
            let (mut state, catalog) = fixture();
            state.block = 3;
            state.powers.set(PowerId::Shadowmeld, SlotWire::Int, 1);

            let successor = drink_fixture_potion(state, &catalog, potion, None, false).state;

            assert_eq!(successor.block, expected_block, "{potion:?}");
            assert_eq!(
                successor.powers.value(PowerId::BlockNextTurn),
                expected_next_turn,
                "{potion:?}"
            );
        }

        for (potion, full_modified_gain) in [
            (PotionId::BlockPotion, 12),
            (PotionId::Fortifier, 1_999_999_990),
            (PotionId::ShipInABottle, 10),
        ] {
            let (mut state, catalog) = fixture();
            state.block = 999_999_995;
            let transition = drink_fixture_potion(state, &catalog, potion, None, false);
            assert_eq!(transition.state.block, 999_999_999, "{potion:?}");
            assert!(
                transition.events.contains(&Event::PlayerBlockGained {
                    amount: full_modified_gain,
                    block: 999_999_999,
                }),
                "{potion:?}"
            );
        }
    }

    #[test]
    fn part_b_enemy_bodies_use_exact_target_domains_and_artifact() {
        for (potion, target) in [
            (PotionId::BeetleJuice, Some(1)),
            (PotionId::FirePotion, Some(1)),
            (PotionId::PowderedDemise, Some(1)),
            (PotionId::VulnerablePotion, Some(1)),
            (PotionId::WeakPotion, Some(1)),
            (PotionId::ExplosiveAmpoule, None),
            (PotionId::PotionOfBinding, None),
            (PotionId::ShacklingPotion, None),
        ] {
            let (mut state, catalog) = fixture();
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Artifact, SlotWire::Int, 1);
            let successor = drink_fixture_potion(state, &catalog, potion, target, false).state;
            match potion {
                PotionId::BeetleJuice => {
                    assert_eq!(successor.monsters[1].powers.value(PowerId::Shrink), 4);
                    assert_eq!(successor.monsters[0].powers.value(PowerId::Artifact), 1);
                }
                PotionId::FirePotion => {
                    assert_eq!(successor.monsters[0].hp, 26);
                    assert_eq!(successor.monsters[1].hp, 5);
                }
                PotionId::PowderedDemise => {
                    assert_eq!(successor.monsters[1].powers.value(PowerId::Demise), 9)
                }
                PotionId::VulnerablePotion => {
                    assert_eq!(successor.monsters[1].powers.value(PowerId::Vuln), 3)
                }
                PotionId::WeakPotion => {
                    assert_eq!(successor.monsters[1].powers.value(PowerId::Weak), 3)
                }
                PotionId::ExplosiveAmpoule => {
                    assert_eq!(
                        (successor.monsters[0].hp, successor.monsters[1].hp),
                        (16, 15)
                    )
                }
                PotionId::PotionOfBinding => {
                    assert_eq!(successor.monsters[0].powers.value(PowerId::Artifact), 0);
                    assert_eq!(successor.monsters[0].powers.value(PowerId::Weak), 0);
                    assert_eq!(successor.monsters[0].powers.value(PowerId::Vuln), 1);
                    assert_eq!(successor.monsters[1].powers.value(PowerId::Weak), 1);
                    assert_eq!(successor.monsters[1].powers.value(PowerId::Vuln), 1);
                }
                PotionId::ShacklingPotion => {
                    assert_eq!(successor.monsters[0].powers.value(PowerId::Artifact), 0);
                    assert_eq!(successor.monsters[0].powers.value(PowerId::Strength), 0);
                    assert_eq!(successor.monsters[1].powers.value(PowerId::Strength), -7);
                    assert_eq!(
                        successor.monsters[1].powers.value(PowerId::TempStrength),
                        -7
                    );
                }
                _ => unreachable!(),
            }
        }
    }

    /// #2480 bodies, against the same `fixture()` roster the Part B enemy
    /// test uses: monsters[0] enters at 26 HP and monsters[1] at 25.
    #[test]
    fn issue_2480_potion_bodies_apply_exact_amounts() {
        for (potion, target) in [
            (PotionId::GhostInAJar, None),
            (PotionId::StarPotion, None),
            (PotionId::PotionShapedRock, Some(1)),
            (PotionId::PotionOfDoom, Some(1)),
            (PotionId::FoulPotion, None),
        ] {
            let (mut state, catalog) = fixture();
            state.hp = 40;
            state.block = 0;
            // Artifact on the untargeted monster proves the Type-2 bodies
            // route through the received-side gate without touching it.
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Artifact, SlotWire::Int, 1);
            let successor = drink_fixture_potion(state, &catalog, potion, target, false).state;
            assert_eq!(successor.fanouts.potion_slots(), [None], "{potion:?}");
            assert!(successor.frames.is_empty(), "{potion:?}");
            match potion {
                // `0x34e544` IL_0056: Apply<IntangiblePower>(1) to
                // Owner.Creature, never to the picked ally.
                PotionId::GhostInAJar => {
                    assert_eq!(successor.powers.value(PowerId::Intangible), 1);
                    assert_eq!(successor.hp, 40);
                }
                // `0x3508b0` IL_0043: GainStars(StarsVar 3) on target.Player.
                PotionId::StarPotion => {
                    assert_eq!(successor.stars, 3);
                    assert_eq!(successor.history.stars_gained_this_turn, 3);
                }
                // `0x34f84c` IL_004c: DamageVar(15, props 4) to the pick.
                PotionId::PotionShapedRock => {
                    assert_eq!(successor.monsters[1].hp, 10);
                    assert_eq!(successor.monsters[0].hp, 26);
                    assert_eq!(successor.monsters[0].powers.value(PowerId::Artifact), 1);
                }
                // `0x34f738` IL_006f: Apply<DoomPower>(33) by the player.
                PotionId::PotionOfDoom => {
                    assert_eq!(successor.monsters[1].powers.value(PowerId::Doom), 33);
                    assert_eq!(successor.monsters[0].powers.value(PowerId::Doom), 0);
                    assert_eq!(successor.monsters[0].powers.value(PowerId::Artifact), 1);
                    assert!(successor.history.doom_applied_by_player_this_turn);
                }
                // `0x34ddd0`: one DamageVar(12, props 4) over
                // `Creatures.Where(!IsPet)` — every enemy AND the player.
                PotionId::FoulPotion => {
                    assert_eq!(
                        (successor.monsters[0].hp, successor.monsters[1].hp),
                        (14, 13)
                    );
                    assert_eq!(successor.hp, 28);
                }
                _ => unreachable!(),
            }
        }
    }

    /// #3244: `CombatState::get_Creatures` (`0x136e51`) puts the player
    /// first, and `<Damage>d__12` (`0x3e96c8`) commits every target (through
    /// IL_0aa4) before its one Kill (IL_0eb4). A Foul Potion that kills the
    /// last enemy therefore still takes 12 off the player, and the fight is
    /// won. This replaces the former refusal of an "unrecoverable" order.
    #[test]
    fn foul_potion_player_share_lands_when_the_enemy_share_ends_the_combat() {
        let (mut state, catalog) = fixture();
        state.hp = 40;
        state.block = 0;
        state.monsters_mut().truncate(1);
        state.monsters_mut()[0].hp = 5;
        let transition = drink_fixture_potion(state, &catalog, PotionId::FoulPotion, None, false);
        let successor = transition.state;
        assert_eq!(successor.hp, 28);
        assert!(successor.monsters[0].hp <= 0);
        assert!(successor.history.over);
        assert!(
            transition
                .events
                .contains(&Event::CombatOver { player_won: true })
        );
    }

    /// Use a one-slot belt holding `potion`, untargeted, keeping the events.
    fn use_untargeted_belt_potion(
        mut state: HotState,
        catalog: &Catalog,
        potion: PotionId,
    ) -> Result<(HotState, Vec<Event>), EngineRefusal> {
        assert!(state.fanouts.set_potion_belt(
            vec![Some(potion)],
            false,
            false,
            false,
            false,
            true,
        ));
        let mut events = Vec::new();
        apply_action_into(
            &state,
            catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
            &mut events,
        )
        .map(|successor| (successor, events))
    }

    /// #3244 refusals: Rust runs Foul Potion's whole player share before the
    /// enemy commits, so each case where that swap is observable refuses by
    /// name. A zero loss (full block) admits even with both listeners, and a
    /// non-lethal loss at the boundary admits.
    #[test]
    fn foul_potion_refuses_an_observable_player_dispatch_swap() {
        let lethal = "Foul Potion player share may be lethal inside the batch";
        let listener = "Foul Potion player dispatch listener before the enemy commits";
        type Setup = fn(&mut HotState);
        // (setup, block before the potion, expected refusal)
        let cases: [(Setup, i32, Option<&str>); 6] = [
            (|state| state.hp = 12, 0, Some(lethal)),
            (
                |state| {
                    state.hp = 11;
                    state.block = 1;
                },
                1,
                Some(lethal),
            ),
            (
                |state| state.powers.set(PowerId::Inferno, SlotWire::Int, 1),
                0,
                Some(listener),
            ),
            (
                |state| state.fanouts.set_puzzle_armed(true),
                0,
                Some(listener),
            ),
            (
                |state| {
                    state.powers.set(PowerId::Inferno, SlotWire::Int, 1);
                    state.fanouts.set_puzzle_armed(true);
                    state.block = 12;
                },
                12,
                None,
            ),
            (
                |state| {
                    state.hp = 12;
                    state.block = 1;
                },
                1,
                None,
            ),
        ];
        for (index, (setup, block, refusal)) in cases.into_iter().enumerate() {
            let (mut state, catalog) = fixture();
            state.hp = 40;
            state.block = 0;
            setup(&mut state);
            let hp = state.hp;
            let result = use_untargeted_belt_potion(state, &catalog, PotionId::FoulPotion);
            match refusal {
                Some(name) => assert_eq!(
                    result.map(|_| ()),
                    Err(EngineRefusal::MalformedArgs(name)),
                    "case {index}"
                ),
                None => {
                    let (successor, _) = result.unwrap();
                    assert_eq!(
                        (successor.monsters[0].hp, successor.monsters[1].hp),
                        (14, 13),
                        "case {index}"
                    );
                    assert_eq!(successor.hp, hp - (12 - block), "case {index}");
                }
            }
        }
    }

    /// #3244 witness (WQQ1NTPA1BW3 n11, census fight f20445ca8108dfad): both
    /// enemies die to one potion Damage command. `<Damage>d__12`
    /// (`0x3e96c8`) commits both HP losses before its one Kill (IL_0eb4), so
    /// at the first death Gremlin Horn's `<AfterDeath>d__6` (`0x326170`)
    /// already sees the combat ending: `<GainEnergy>d__3` (`0x3ee8a0`
    /// IL_0030-IL_003c) grants nothing and the Horn draws nothing. The
    /// per-target walk this replaces paid +1 energy and a draw at the first
    /// death. The control keeps one enemy alive, so its one death pays out.
    #[test]
    fn aoe_potions_kill_as_one_batch_so_gremlin_horn_sees_the_ending() {
        for potion in [PotionId::ExplosiveAmpoule, PotionId::FoulPotion] {
            for (second_hp, pays_out) in [(5, false), (30, true)] {
                let (mut state, catalog) = fixture();
                state.hp = 40;
                state.block = 0;
                state.monsters_mut()[0].hp = 5;
                state.monsters_mut()[1].hp = second_hp;
                state.fanouts.set_gremlin_horn_owned(true);
                let energy = state.energy;
                let hand = state.piles.get(PileId::Hand).len();
                let draw = state.piles.get(PileId::Draw).len();
                let (successor, events) =
                    use_untargeted_belt_potion(state, &catalog, potion).unwrap();
                let label = format!("{potion:?} second_hp={second_hp}");
                assert!(successor.monsters[0].hp <= 0, "{label}");
                assert_eq!(successor.history.over, !pays_out, "{label}");
                if pays_out {
                    assert_eq!(successor.energy, energy + 1, "{label}");
                    assert_eq!(
                        successor.piles.get(PileId::Hand).len(),
                        hand + usize::from(draw > 0),
                        "{label}"
                    );
                } else {
                    assert_eq!(successor.energy, energy, "{label}");
                    assert_eq!(successor.piles.get(PileId::Hand).len(), hand, "{label}");
                    assert_eq!(successor.piles.get(PileId::Draw).len(), draw, "{label}");
                    // Both HP losses are committed before either death.
                    let first_death = events
                        .iter()
                        .position(|event| matches!(event, Event::MonsterDied { .. }))
                        .unwrap();
                    let damaged = events[..first_death]
                        .iter()
                        .filter(|event| matches!(event, Event::MonsterDamaged { .. }))
                        .count();
                    assert_eq!(damaged, 2, "{label}");
                }
                if potion == PotionId::FoulPotion {
                    assert_eq!(successor.hp, 28, "{label}");
                }
            }
        }
    }

    #[test]
    fn powdered_demise_and_beetle_admit_unrelated_temp_strength_and_shriek() {
        for extra in [PowerId::TempStrength, PowerId::Shriek] {
            for (potion, applied_power, expected) in [
                (PotionId::PowderedDemise, PowerId::Demise, 9),
                (PotionId::BeetleJuice, PowerId::Shrink, 4),
            ] {
                let (mut state, fixture_catalog) = fixture();
                state.monsters_mut().truncate(1);
                if extra == PowerId::TempStrength {
                    state.monsters_mut()[0]
                        .powers
                        .set(PowerId::Strength, SlotWire::Int, -7);
                    state.monsters_mut()[0]
                        .powers
                        .set(PowerId::TempStrength, SlotWire::Int, -7);
                } else {
                    state.monsters_mut()[0].kind = MonsterKind::TerrorEel;
                    state.monsters_mut()[0].loop_pos = 0;
                    state.monsters_mut()[0]
                        .powers
                        .set(PowerId::Shriek, SlotWire::Bool, 1);
                }
                assert!(state.fanouts.set_potion_belt(
                    vec![Some(potion)],
                    false,
                    false,
                    false,
                    false,
                    true,
                ));

                let document = HotBoundary::to_canonical(&state, &fixture_catalog);
                let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
                let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
                assert_eq!(
                    admit(&document, &state, &catalog),
                    Ok(()),
                    "{potion:?}/{extra:?}"
                );
                assert!(
                    damage::misery_scalar_state_is_exact(&state.monsters[0]).is_err(),
                    "the broader Misery clone gate must remain closed"
                );

                let successor = apply_action(
                    &state,
                    &catalog,
                    &Action::UsePotion {
                        slot: 0,
                        target: Some(0),
                    },
                )
                .unwrap()
                .state;
                assert_eq!(successor.monsters[0].powers.value(applied_power), expected);
                if extra == PowerId::TempStrength {
                    assert_eq!(successor.monsters[0].powers.value(PowerId::Strength), -7);
                    assert_eq!(
                        successor.monsters[0].powers.value(PowerId::TempStrength),
                        -7
                    );
                } else {
                    assert_eq!(successor.monsters[0].powers.value(PowerId::Shriek), 1);
                }

                let successor_document =
                    HotBoundary::try_to_canonical(&successor, &catalog).unwrap();
                let rebuilt_catalog =
                    HotBoundary::catalog_from_canonical(&successor_document).unwrap();
                let rebuilt =
                    HotBoundary::from_canonical(&successor_document, &rebuilt_catalog).unwrap();
                assert_eq!(
                    admit(&successor_document, &rebuilt, &rebuilt_catalog),
                    Ok(())
                );
                assert_eq!(rebuilt, successor);
            }
        }
    }

    #[test]
    fn represented_osty_admits_player_only_part_c_and_rejects_forged_targets_atomically() {
        let safe = [
            PotionId::Ashwater,
            PotionId::BottledPotential,
            PotionId::Clarity,
            PotionId::CureAll,
            PotionId::DropletOfPrecognition,
            PotionId::GamblersBrew,
            PotionId::GigantificationPotion,
            PotionId::LiquidMemories,
            PotionId::LuckyTonic,
            PotionId::MazalethsGift,
            PotionId::ShipInABottle,
            PotionId::SneckoOil,
            PotionId::SoldiersStew,
            PotionId::SwiftPotion,
            PotionId::TouchOfInsanity,
        ];
        for potion in safe {
            let (mut state, catalog) = fixture();
            state.fanouts.set_osty(Some((3, 5))).unwrap();
            assert!(state.fanouts.set_potion_belt(
                vec![Some(potion)],
                false,
                false,
                false,
                false,
                true,
            ));
            let document = HotBoundary::to_canonical(&state, &catalog);
            assert_eq!(admit(&document, &state, &catalog), Ok(()), "{potion:?}");
            let action = Action::UsePotion {
                slot: 0,
                target: None,
            };
            assert!(
                legal_actions(&state, &catalog).contains(&action),
                "{potion:?}"
            );
            let before = state.clone();
            let mut events = vec![Event::TurnEnded { turn: 99 }];
            assert_eq!(
                apply_action_into(
                    &state,
                    &catalog,
                    &Action::UsePotion {
                        slot: 0,
                        target: Some(0),
                    },
                    &mut events,
                ),
                Err(EngineRefusal::PotionTargetMismatch { required: false }),
                "{potion:?}"
            );
            assert_eq!(state, before, "{potion:?}");
            assert!(events.is_empty(), "{potion:?}");
        }

        // #2497: a live local Osty is no native pick for any supported body
        // (none is TargetType 6, the only `PotionModel::IsValidTarget`
        // `0x8328c` branch a pet can pass), so it neither refuses the root nor
        // narrows the action space. Each body the teammate gate still holds is
        // offered with its native target and the roster guard lets it through.
        for potion in potions::SUPPORTED
            .into_iter()
            .filter(|potion| potions::requires_solo_player_target(*potion))
        {
            let (mut state, catalog) = fixture();
            state.fanouts.set_osty(Some((3, 5))).unwrap();
            assert!(state.fanouts.set_potion_belt(
                vec![Some(potion)],
                false,
                false,
                false,
                false,
                true,
            ));
            assert!(potions::native_target_excludes_pets(potion), "{potion:?}");
            assert!(
                potions::potion_target_roster_is_exact(&state, potion),
                "{potion:?}"
            );
            let document = HotBoundary::to_canonical(&state, &catalog);
            // Assert the absence of *this* capability rather than a clean
            // admission: Distilled Chaos owes an unrelated Part D closure this
            // bare fixture cannot satisfy, and that blocker is not what the
            // Osty is being tested for.
            assert!(
                !admit(&document, &state, &catalog).is_err_and(|refusal| refusal.contains(
                    admission::MissingCapability::ArgumentShape("potion AnyAlly target roster")
                )),
                "{potion:?}"
            );
            // The native target: an AnyEnemy body (Potion-Shaped Rock, kind 2)
            // is offered against a living enemy, every other body target-less.
            let action = Action::UsePotion {
                slot: 0,
                target: if potion == PotionId::PotionShapedRock {
                    Some(0)
                } else {
                    None
                },
            };
            let offered = legal_actions(&state, &catalog)
                .iter()
                .filter(|action| matches!(action, Action::UsePotion { slot: 0, .. }))
                .cloned()
                .collect::<Vec<_>>();
            if potion == PotionId::PotionShapedRock {
                // Every offered pick is a living enemy slot; the Osty is not a
                // `monsters` entry and so can never be one.
                assert!(offered.contains(&action), "{potion:?}");
                assert!(offered.iter().all(|action| matches!(
                    action,
                    Action::UsePotion { target: Some(target), .. }
                        if state.monsters[usize::from(*target)].hp > 0
                )));
            } else if !matches!(
                potion,
                // Both owe a Part D closure this bare fixture cannot
                // satisfy (Cosmic Concoction: Colorless generation
                // provenance, #3229), so neither is offered here.
                PotionId::DistilledChaos | PotionId::CosmicConcoction
            ) {
                assert_eq!(offered, vec![action], "{potion:?}");
            }
            let mut events = Vec::new();
            assert_ne!(
                apply_action_into(&state, &catalog, &action, &mut events),
                Err(EngineRefusal::MalformedArgs("potion AnyAlly target roster")),
                "{potion:?}"
            );

            // The same belt behind a live teammate key stays a whole-root
            // refusal: a kind-5 body can pick the teammate *player*, which
            // `MultiplayerAllyState` cannot carry, and a forged use refuses
            // atomically.
            let mut multiplayer = state.clone();
            multiplayer.fanouts.set_osty(None).unwrap();
            multiplayer.multiplayer_ally_key = 1;
            assert!(
                !potions::potion_target_roster_is_exact(&multiplayer, potion),
                "{potion:?}"
            );
            let multiplayer_document = HotBoundary::to_canonical(&multiplayer, &catalog);
            assert!(
                admit(&multiplayer_document, &multiplayer, &catalog)
                    .unwrap_err()
                    .contains(admission::MissingCapability::ArgumentShape(
                        "potion AnyAlly target roster"
                    )),
                "{potion:?}"
            );
            let before = multiplayer.clone();
            let mut events = vec![Event::TurnEnded { turn: 99 }];
            assert_eq!(
                apply_action_into(&multiplayer, &catalog, &action, &mut events),
                Err(EngineRefusal::MalformedArgs("potion AnyAlly target roster")),
                "{potion:?}"
            );
            assert_eq!(multiplayer, before, "{potion:?}");
        }

        // The pet arm of the roster guard: a body whose native target kind is
        // unread (here an unsupported potion) is refused while an Osty lives
        // and admitted once it is gone.
        let (mut state, _catalog) = fixture();
        assert!(potions::native_target_type(PotionId::Ambergris).is_none());
        assert!(!potions::native_target_excludes_pets(PotionId::Ambergris));
        assert!(potions::potion_target_roster_is_exact(
            &state,
            PotionId::Ambergris
        ));
        state.fanouts.set_osty(Some((3, 5))).unwrap();
        assert!(!potions::potion_target_roster_is_exact(
            &state,
            PotionId::Ambergris
        ));
    }

    #[test]
    fn live_osty_leaves_every_teammate_gated_potion_effect_as_in_the_solo_roster() {
        // #2497: every body the teammate gate holds routes its effect to the
        // picked `target` or to `Owner.Creature` (e.g. Block Potion
        // `<OnUse>d__10` `0x34c0a8` IL_0029-IL_003b gains block on `target`;
        // Liquid Bronze `<OnUse>d__10` `0x34eb78` applies Thorns to `target`
        // at IL_0056; Blessing of the Forge `OnUse` `0xabb34` upgrades
        // `target.Player`'s Hand at IL_002d-IL_0057). The kind-5 pick is an
        // `IsPlayer` creature, so with an Osty out the recipient is still the
        // player: the successor equals the Osty-free successor with the Osty
        // added back, and the events are identical.
        let mut exercised = Vec::new();
        for potion in potions::SUPPORTED
            .into_iter()
            .filter(|potion| potions::requires_solo_player_target(*potion))
        {
            let (mut solo, catalog) = fixture();
            assert!(solo.fanouts.set_potion_belt(
                vec![Some(potion)],
                false,
                false,
                false,
                false,
                true,
            ));
            // Re-derive the catalog from the belt-bearing document so a held
            // Blessing of the Forge interns its Hand's upgrade closure.
            let wire = HotBoundary::try_to_canonical(&solo, &catalog).unwrap();
            let catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
            let solo = HotBoundary::from_canonical(&wire, &catalog).unwrap();
            let action = Action::UsePotion {
                slot: 0,
                target: (potion == PotionId::PotionShapedRock).then_some(0),
            };
            let mut with_osty = solo.clone();
            with_osty.fanouts.set_osty(Some((3, 5))).unwrap();
            let mut solo_events = Vec::new();
            let solo_result = apply_action_into(&solo, &catalog, &action, &mut solo_events);
            let mut osty_events = Vec::new();
            let osty_result = apply_action_into(&with_osty, &catalog, &action, &mut osty_events);
            let Ok(solo_successor) = solo_result else {
                // A body this bare fixture cannot drive (e.g. Distilled
                // Chaos's Part D closure) refuses identically either way.
                assert_eq!(
                    osty_result.map(|_| ()),
                    solo_result.map(|_| ()),
                    "{potion:?}"
                );
                continue;
            };
            let mut expected = solo_successor.clone();
            expected.fanouts.set_osty(Some((3, 5))).unwrap();
            assert_eq!(osty_result.unwrap(), expected, "{potion:?}");
            assert_eq!(osty_events, solo_events, "{potion:?}");
            // The effect is real and lands on the player, not a no-op.
            match potion {
                PotionId::BlockPotion => {
                    assert_eq!(expected.block, solo.block + 12);
                }
                PotionId::LiquidBronze => {
                    assert_eq!(
                        expected.powers.value(PowerId::Thorns),
                        solo.powers.value(PowerId::Thorns) + 3
                    );
                }
                PotionId::BlessingOfTheForge => {
                    assert_ne!(
                        expected.piles.get(PileId::Hand).as_slice(),
                        solo.piles.get(PileId::Hand).as_slice()
                    );
                }
                _ => {}
            }
            exercised.push(potion);
        }
        for potion in [
            PotionId::BlessingOfTheForge,
            PotionId::BlockPotion,
            PotionId::LiquidBronze,
            PotionId::PotionOfBinding,
            PotionId::PotionShapedRock,
            PotionId::RegenPotion,
        ] {
            assert!(exercised.contains(&potion), "{potion:?}");
        }
    }

    #[test]
    fn removed_dupe_draw_chains_pagestorm_pause_keeps_current_object() {
        let pommel = plain_identity(CardId::StrikeIronclad, 0);
        let seeker = plain_identity(CardId::SculptingStrike, 0);
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let pommel_atom = builder.intern(pommel).unwrap();
        let seeker_atom = builder.intern(seeker).unwrap();
        let defend_atom = builder.intern(defend).unwrap();
        builder
            .intern_monster(MonsterKind::TorchHeadAmalgam)
            .unwrap();
        builder.intern_monster(MonsterKind::Queen).unwrap();
        builder
            .set_relics(&[crate::ids::RelicId::RelicGhostSeed])
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 8;
        state.exact_piles = true;
        let mut amalgam = HotMonster::new(
            MonsterKind::TorchHeadAmalgam,
            monsters::TORCH_HEAD_AMALGAM_HP,
        );
        amalgam.max_hp = monsters::TORCH_HEAD_AMALGAM_HP;
        amalgam.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
        let mut queen = HotMonster::new(MonsterKind::Queen, monsters::QUEEN_HP);
        queen.max_hp = monsters::QUEEN_HP;
        queen.uid = 1;
        queen.slot = 1;
        queen.loop_pos = 1;
        state.monsters = std::sync::Arc::new(vec![amalgam, queen]);
        state.powers.set(PowerId::ChainsOfBinding, SlotWire::Int, 3);
        state.powers.set(PowerId::Pagestorm, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&[PowerId::ChainsOfBinding, PowerId::Pagestorm])
        );
        assert!(state.fanouts.set_before_side_turn_end_order(&[
            crate::hot::BeforeSideTurnEndToken::ChainsOfBinding
        ]));
        let mut original = crate::hot::CardInstanceState::default();
        original.set_local_ethereal(true);
        state.card_states.set(1, original);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        for stream in [
            crate::hot::RngStream::Sel,
            crate::hot::RngStream::Targets,
            crate::hot::RngStream::EnergyCosts,
        ] {
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        state.piles.set(
            PileId::Hand,
            crate::hot::HotPile::from_cards(vec![HotCard {
                uid: 6,
                atom: defend_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }]),
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 7,
            atom: defend_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.set(
            PileId::Draw,
            crate::hot::HotPile::from_cards(vec![
                HotCard {
                    uid: 1,
                    atom: pommel_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_DUPE,
                },
                HotCard {
                    uid: 2,
                    atom: seeker_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 3,
                    atom: defend_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 4,
                    atom: defend_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 5,
                    atom: defend_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
            ]),
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::SwiftPotion)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.rng.set(
            crate::hot::RngStream::EnergyCosts,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        let predecessor_document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&predecessor_document).unwrap();
        let state = HotBoundary::from_canonical(&predecessor_document, &catalog).unwrap();
        assert_eq!(admit(&predecessor_document, &state, &catalog), Ok(()));
        assert!(
            legal_actions(&state, &catalog).contains(&Action::UsePotion {
                slot: 0,
                target: None,
            })
        );
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(parked.pending.is_some(), "{parked:#?}");
        assert_eq!(
            parked.card_states.removed_draw_objects().len(),
            1,
            "piles={:?} frames={:?}",
            parked.piles,
            parked.frames
        );
        assert!(parked.card_states.get_ref(1).is_none());
        let retained = parked.card_states.removed_draw_object(1).unwrap();
        assert_ne!(retained.card.flags & crate::hot::CARD_FLAG_BOUND, 0);
        assert!(retained.state.local_cost_modifiers.as_slice().is_empty());
        assert_eq!(retained.state.free_star_cost_this_turn_or_played_rows, 0);

        assert!(
            parked
                .frames
                .as_slice()
                .iter()
                .any(|frame| matches!(frame, crate::frame::Frame::AfterCardDrawnPower { .. }))
        );
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(rebuilt, parked);

        let resumed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &legal_actions(&rebuilt, &rebuilt_catalog)[0],
        )
        .unwrap()
        .state;
        assert!(resumed.pending.is_none(), "{resumed:#?}");
        assert!(resumed.frames.is_empty(), "{resumed:#?}");
        assert_eq!(resumed.cards_drawn_combat, 4);
        assert!(resumed.card_states.removed_draw_objects().is_empty());
        assert!(resumed.fanouts.potion_slots()[0].is_none());
    }

    #[test]
    fn removed_dupe_draw_survives_later_hellraiser_choice_and_cold_reload() {
        let pommel = plain_identity(CardId::StrikeIronclad, 0);
        let seeker = plain_identity(CardId::SculptingStrike, 0);
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let pommel_atom = builder.intern(pommel).unwrap();
        let seeker_atom = builder.intern(seeker).unwrap();
        let defend_atom = builder.intern(defend).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 8;
        state.exact_piles = true;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
        monster.max_hp = 30;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        for stream in [crate::hot::RngStream::Sel, crate::hot::RngStream::Targets] {
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        state.piles.set(
            PileId::Hand,
            crate::hot::HotPile::from_cards(vec![HotCard {
                uid: 6,
                atom: defend_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }]),
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 7,
            atom: defend_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.set(
            PileId::Draw,
            crate::hot::HotPile::from_cards(vec![
                HotCard {
                    uid: 1,
                    atom: pommel_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_DUPE,
                },
                HotCard {
                    uid: 2,
                    atom: seeker_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 3,
                    atom: defend_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 4,
                    atom: defend_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 5,
                    atom: defend_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
            ]),
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::SwiftPotion)],
            false,
            false,
            false,
            false,
            true,
        ));
        let predecessor_document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&predecessor_document).unwrap();
        let state = HotBoundary::from_canonical(&predecessor_document, &catalog).unwrap();
        assert_eq!(admit(&predecessor_document, &state, &catalog), Ok(()));
        // #3136: Scrape and the former "unowned" Draw sources read a removed
        // DUPE result exactly, so both DUPE and ordinary roots admit.
        for dupe in [false, true] {
            for source in [
                "SCRAPE",
                "BIG_BANG",
                "ESCAPE_PLAN",
                "EXPERTISE",
                "IMPATIENCE",
            ] {
                let mut guarded = predecessor_document.clone();
                guarded.piles.get_mut("hand").unwrap()[0].id = source.to_owned();
                if !dupe {
                    guarded.piles.get_mut("draw").unwrap()[0].extra.clear();
                }
                let guard_catalog = HotBoundary::catalog_from_canonical(&guarded).unwrap();
                let guard_state = HotBoundary::from_canonical(&guarded, &guard_catalog).unwrap();
                assert_eq!(
                    admit(&guarded, &guard_state, &guard_catalog),
                    Ok(()),
                    "{source} dupe={dupe}"
                );
            }
        }

        assert!(
            legal_actions(&state, &catalog).contains(&Action::UsePotion {
                slot: 0,
                target: None,
            })
        );
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(parked.pending.is_some(), "{parked:#?}");
        assert_eq!(
            parked.card_states.removed_draw_objects().len(),
            1,
            "piles={:?} frames={:?}",
            parked.piles,
            parked.frames
        );
        assert!(parked.card_states.get_ref(1).is_none());
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(rebuilt, parked);
        admit(&document, &rebuilt, &rebuilt_catalog).unwrap();
        for forged_uid in [6_u32, parked.next_card_uid] {
            let mut forged = document.clone();
            let removed = forged
                .continuations
                .iter_mut()
                .filter_map(|frame| frame.fields.get_mut("drawn"))
                .filter_map(serde_json::Value::as_array_mut)
                .flat_map(|entries| entries.iter_mut())
                .find(|entry| matches!(entry[0].as_str(), Some("removed")))
                .unwrap();
            removed[1][0] = serde_json::json!(forged_uid);
            assert!(HotBoundary::from_canonical(&forged, &rebuilt_catalog).is_err());
        }
        let mut rootless = document.clone();
        rootless.continuations.remove(0);
        assert!(HotBoundary::from_canonical(&rootless, &rebuilt_catalog).is_err());
        let mut forged = document.clone();
        let removed = forged
            .continuations
            .iter_mut()
            .filter_map(|frame| frame.fields.get_mut("drawn"))
            .filter_map(serde_json::Value::as_array_mut)
            .flat_map(|entries| entries.iter_mut())
            .find(|entry| matches!(entry[0].as_str(), Some("removed")))
            .unwrap();
        removed[1][1].as_array_mut().unwrap().pop();
        assert!(HotBoundary::from_canonical(&forged, &rebuilt_catalog).is_err());

        let resumed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &legal_actions(&rebuilt, &rebuilt_catalog)[0],
        )
        .unwrap()
        .state;
        assert!(resumed.pending.is_none(), "{resumed:#?}");
        assert!(resumed.frames.is_empty(), "{resumed:#?}");
        assert_eq!(resumed.cards_drawn_combat, 3);
        assert!(resumed.card_states.removed_draw_objects().is_empty());
        assert!(resumed.fanouts.potion_slots()[0].is_none());
    }

    /// #3136 public witness fixture: Hellraiser live, `hand` in Hand, and a
    /// DUPE Strike on top of Draw followed by `draw_tail`.
    fn removed_dupe_result_reader_fixture(
        hand: CardIdentity,
        draw_tail: &[CardIdentity],
        relics: &[crate::ids::RelicId],
    ) -> (Catalog, HotState) {
        let strike = plain_identity(CardId::StrikeIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let strike_atom = builder.intern_reachable(strike).unwrap();
        let hand_atom = builder.intern_reachable(hand).unwrap();
        let tail_atoms: Vec<_> = draw_tail
            .iter()
            .map(|identity| builder.intern_reachable(*identity).unwrap())
            .collect();
        builder.mark_live_hellraiser_reachable();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.set_relics(relics).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 60);
        monster.max_hp = 60;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        for stream in [crate::hot::RngStream::Sel, crate::hot::RngStream::Targets] {
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        let mut draw = vec![HotCard {
            uid: 1,
            atom: strike_atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | crate::hot::CARD_FLAG_DUPE,
        }];
        for (offset, atom) in tail_atoms.into_iter().enumerate() {
            draw.push(HotCard {
                uid: 2 + offset as u32,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        }
        let hand_uid = 2 + draw_tail.len() as u32;
        state.next_card_uid = hand_uid + 1;
        state
            .piles
            .set(PileId::Draw, crate::hot::HotPile::from_cards(draw));
        state.piles.set(
            PileId::Hand,
            crate::hot::HotPile::from_cards(vec![HotCard {
                uid: hand_uid,
                atom: hand_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }]),
        );
        (catalog, state)
    }

    fn play_removed_dupe_result_reader(catalog: &Catalog, state: &HotState) -> HotState {
        let card = state.piles.get(PileId::Hand).as_slice()[0];
        let uid = card.uid;
        let targeted = catalog.spec(card.atom).unwrap().target_type == CardTargetType::AnyEnemy;
        let document = HotBoundary::try_to_canonical(state, catalog).unwrap();
        assert_eq!(admit(&document, state, catalog), Ok(()));
        let next = apply_action(
            state,
            catalog,
            &Action::Play {
                uid,
                target: targeted.then_some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(next.pending.is_none(), "{next:#?}");
        assert!(next.frames.is_empty(), "{next:#?}");
        assert!(next.card_states.removed_draw_objects().is_empty());
        assert!(
            crate::engine::play::unique_live_card_location(&next, 1)
                .unwrap()
                .is_none(),
            "the DUPE Strike left combat"
        );
        next
    }

    #[test]
    fn scrape_discards_a_removed_dupe_strike_result_without_moving_it() {
        // Scrape 7 dmg draws 4: the DUPE Strike (auto-played for 6, then
        // removed) and three Defends. All four cost 1, so all four are
        // selected; the removed one counts as discarded and fires Tough
        // Bandages, but CardPileCmd.Add moves nothing (#3136).
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let (catalog, state) = removed_dupe_result_reader_fixture(
            plain_identity(CardId::Scrape, 0),
            &[defend, defend, defend],
            &[crate::ids::RelicId::RelicToughBandages],
        );
        let next = play_removed_dupe_result_reader(&catalog, &state);
        assert_eq!(next.monsters[0].hp, 60 - 7 - 6);
        assert_eq!(next.history.discarded_cards_this_turn, 4);
        assert_eq!(next.block, 12, "Tough Bandages fired once per occurrence");
        let discard: Vec<_> = next
            .piles
            .get(PileId::Discard)
            .as_slice()
            .iter()
            .map(|card| card.uid)
            .collect();
        assert_eq!(discard, [2, 3, 4, 5]);
        assert!(next.piles.get(PileId::Hand).is_empty());
    }

    #[test]
    fn scrape_prices_a_removed_dupe_strike_from_its_local_rows_only() {
        // SetToFreeThisCombat's local Set-0 row survives the auto-play
        // cleanup and makes the removed Strike cost 0: Scrape keeps it out of the
        // discard, so only the three Defends count.
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let (catalog, mut state) = removed_dupe_result_reader_fixture(
            plain_identity(CardId::Scrape, 0),
            &[defend, defend, defend],
            &[crate::ids::RelicId::RelicToughBandages],
        );
        state.card_states.set_to_free_this_combat(1, 1).unwrap();
        let next = play_removed_dupe_result_reader(&catalog, &state);
        assert_eq!(next.history.discarded_cards_this_turn, 3);
        assert_eq!(next.block, 9);
    }

    #[test]
    fn parked_scrape_draw_resumes_over_a_removed_dupe_entry_after_cold_reload() {
        // The DUPE Strike is removed before Seeker Strike's selection parks
        // Scrape's Draw. The parked Draw carries the removed
        // entry through the canonical boundary, and the resumed Scrape tail
        // reads it as a returned object (`scrape_draw_entries_are_exact`).
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let (catalog, state) = removed_dupe_result_reader_fixture(
            plain_identity(CardId::Scrape, 0),
            &[
                defend,
                plain_identity(CardId::SeekerStrike, 0),
                defend,
                defend,
                defend,
            ],
            &[crate::ids::RelicId::RelicToughBandages],
        );
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&document, &state, &catalog), Ok(()));
        let parked = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 7,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(parked.pending.is_some(), "{parked:#?}");
        assert!(parked.card_states.removed_draw_object(1).is_some());
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        // Atoms renumber with the rebuilt catalog; the document is the identity.
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
            document
        );
        admit(&document, &rebuilt, &rebuilt_catalog).unwrap();
        let resumed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &legal_actions(&rebuilt, &rebuilt_catalog)[0],
        )
        .unwrap()
        .state;
        assert!(resumed.pending.is_none(), "{resumed:#?}");
        assert!(resumed.frames.is_empty(), "{resumed:#?}");
        assert!(resumed.card_states.removed_draw_objects().is_empty());
        assert_eq!(resumed.cards_drawn_combat, 4);
        assert_eq!(resumed.history.discarded_cards_this_turn, 4);
        assert_eq!(resumed.block, 12);
        // Seeker Strike's chosen card is not a Draw result and stays in Hand.
        assert_eq!(resumed.piles.get(PileId::Hand).len(), 1);
        assert!(
            crate::engine::play::unique_live_card_location(&resumed, 1)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn escape_plan_reads_a_removed_dupe_strike_as_its_attack_first_result() {
        let (catalog, state) =
            removed_dupe_result_reader_fixture(plain_identity(CardId::EscapePlan, 0), &[], &[]);
        let next = play_removed_dupe_result_reader(&catalog, &state);
        assert_eq!(next.block, 0, "an Attack first result grants no block");
        assert_eq!(next.monsters[0].hp, 60 - 6);
    }

    #[test]
    fn expertise_and_big_bang_and_impatience_complete_over_a_removed_dupe_result() {
        let defend = plain_identity(CardId::DefendIronclad, 0);
        for source in [CardId::Expertise, CardId::BigBang, CardId::Impatience] {
            let (catalog, state) = removed_dupe_result_reader_fixture(
                plain_identity(source, 0),
                &[defend, defend],
                &[],
            );
            let next = play_removed_dupe_result_reader(&catalog, &state);
            assert_eq!(next.monsters[0].hp, 60 - 6, "{source:?}");
            if source == CardId::Expertise {
                // Expertise draws 2: the removed Strike and Defend uid 2.
                // Retain reaches the live Defend; the removed object's write
                // left with it at publication.
                assert!(next.card_states.get(2).transient_retain);
                assert!(next.card_states.get_ref(1).is_none());
            }
        }
    }

    #[test]
    fn swift_potion_draw_parks_and_resumes_one_selecting_hellraiser_strike() {
        let pommel = plain_identity(CardId::PommelStrike, 0);
        let seeker = plain_identity(CardId::SeekerStrike, 0);
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let pommel_atom = builder.intern(pommel).unwrap();
        let seeker_atom = builder.intern(seeker).unwrap();
        let defend_atom = builder.intern(defend).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 7;
        state.exact_piles = true;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
        monster.max_hp = 30;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        for stream in [crate::hot::RngStream::Sel, crate::hot::RngStream::Targets] {
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        state.piles.set(
            PileId::Hand,
            crate::hot::HotPile::from_cards(vec![HotCard {
                uid: 6,
                atom: defend_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }]),
        );
        state.piles.set(
            PileId::Draw,
            crate::hot::HotPile::from_cards(vec![
                HotCard {
                    uid: 1,
                    atom: pommel_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 2,
                    atom: seeker_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 3,
                    atom: defend_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 4,
                    atom: defend_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 5,
                    atom: defend_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
            ]),
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::SwiftPotion)],
            false,
            false,
            false,
            false,
            true,
        ));
        let predecessor_document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&predecessor_document).unwrap();
        let state = HotBoundary::from_canonical(&predecessor_document, &catalog).unwrap();
        assert_eq!(admit(&predecessor_document, &state, &catalog), Ok(()));
        assert!(
            legal_actions(&state, &catalog).contains(&Action::UsePotion {
                slot: 0,
                target: None,
            })
        );
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(parked.pending.is_some(), "{parked:#?}");
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::PotionFinish { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. },
                ]
            ),
            "{:#?}",
            parked.frames
        );
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(rebuilt, parked);

        let resumed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(resumed.pending.is_none(), "{resumed:#?}");
        assert!(resumed.frames.is_empty(), "{resumed:#?}");
        assert_eq!(resumed.cards_drawn_combat, 4);
        assert!(resumed.fanouts.potion_slots()[0].is_none());
    }

    /// Glowwater's two halves, with and without a parking Dark Embrace.
    ///
    /// `GlowwaterPotion/<OnUse>d__10::MoveNext` `0x34e764` freezes
    /// `Hand.Cards.ToList()`, awaits one `CardCmd::Exhaust` per element, then
    /// awaits `CardPileCmd::Draw(CardsVar 10)`.
    fn glowwater_fixture(dark_embrace: i32) -> (HotState, Catalog) {
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let strike = plain_identity(CardId::StrikeIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let defend_atom = builder.intern(defend).unwrap();
        let strike_atom = builder.intern(strike).unwrap();
        // A live selecting strike is what lets a Dark Embrace Draw park at all
        // (`Catalog::after_card_exhausted_dark_can_suspend`).
        let pommel_atom = builder
            .intern(plain_identity(CardId::PommelStrike, 0))
            .unwrap();
        let seeker_atom = builder
            .intern(plain_identity(CardId::SeekerStrike, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 17;
        state.exact_piles = true;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
        monster.max_hp = 30;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        if dark_embrace > 0 {
            state
                .powers
                .set(PowerId::DarkEmbrace, SlotWire::Int, dark_embrace);
            assert!(
                state
                    .fanouts
                    .register_after_card_exhausted(PowerId::DarkEmbrace)
            );
            state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
            for stream in [crate::hot::RngStream::Sel, crate::hot::RngStream::Targets] {
                state.rng.set(
                    stream,
                    crate::hot::RngStreamState {
                        words: [1, 2, 3, 4],
                        counter: 0,
                    },
                );
            }
        }
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.piles.set(
            PileId::Hand,
            crate::hot::HotPile::from_cards(
                (1..=3)
                    .map(|uid| HotCard {
                        uid,
                        atom: strike_atom,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    })
                    .collect(),
            ),
        );
        state.piles.set(
            PileId::Draw,
            crate::hot::HotPile::from_cards(
                (4..=16)
                    .map(|uid| HotCard {
                        uid,
                        atom: match (dark_embrace > 0, uid) {
                            (true, 4) => pommel_atom,
                            (true, 5) => seeker_atom,
                            _ => defend_atom,
                        },
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    })
                    .collect(),
            ),
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::GlowwaterPotion)],
            false,
            false,
            false,
            false,
            true,
        ));
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert_eq!(admit(&document, &state, &catalog), Ok(()));
        (state, catalog)
    }

    #[test]
    fn glowwater_exhausts_the_whole_hand_then_draws_ten() {
        let (state, catalog) = glowwater_fixture(0);
        assert!(
            legal_actions(&state, &catalog).contains(&Action::UsePotion {
                slot: 0,
                target: None,
            })
        );
        let successor = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(successor.pending.is_none(), "{successor:#?}");
        assert!(successor.frames.is_empty(), "{successor:#?}");
        assert_eq!(successor.fanouts.potion_slots(), [None]);
        // All three snapshot cards left Hand for Exhaust, and none of them
        // came back: the Draw(10) can only see the 13 Draw-pile cards.
        assert_eq!(
            successor
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(successor.piles.get(PileId::Hand).as_slice().len(), 10);
        assert!(
            successor
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .all(|card| card.uid >= 4)
        );
        assert_eq!(successor.piles.get(PileId::Draw).as_slice().len(), 3);
        assert_eq!(successor.cards_drawn_combat, 10);
        assert_eq!(successor.history.owner_cards_exhausted_combat, 3);
        assert!(successor.history.owner_card_exhausted_this_turn);
    }

    #[test]
    fn glowwater_exhaust_walk_parks_on_dark_embrace_and_roundtrips() {
        let (state, catalog) = glowwater_fixture(1);
        let mut parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        // The first Exhaust hands off to a Dark Embrace Draw owned by the
        // Glowwater producer record, under the potion's own PotionFinish.
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::PotionFinish { .. },
                    crate::frame::Frame::AfterCardExhaustedPower { .. },
                    ..
                ]
            ),
            "{:#?}",
            parked.frames
        );
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(rebuilt, parked);

        // Drive the parked walk to completion through public actions only.
        let mut guard = 0;
        while !parked.frames.is_empty() {
            guard += 1;
            assert!(guard < 32, "{:#?}", parked.frames);
            let actions = legal_actions(&parked, &rebuilt_catalog);
            let action = actions.first().copied().expect("a resuming action");
            parked = apply_action(&parked, &rebuilt_catalog, &action)
                .unwrap()
                .state;
        }
        assert!(parked.pending.is_none(), "{parked:#?}");
        assert_eq!(parked.fanouts.potion_slots(), [None]);
        assert_eq!(parked.history.owner_cards_exhausted_combat, 3);
        // Every snapshot card left Hand for Exhaust and none returned.
        assert!((1..=3).all(|uid| {
            parked
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .any(|card| card.uid == uid)
        }));
        assert!(
            parked
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .all(|card| card.uid >= 4)
        );
        // Three Dark Embrace draws plus the Draw(10) tail. The exact total is
        // below 13 only because Hellraiser auto-plays some drawn cards and
        // Pommel Strike draws again; what this pins is that the tail ran at
        // all, which a dropped `glowwater_draw_tail` would break.
        assert!(
            parked.cards_drawn_combat >= 10,
            "{}",
            parked.cards_drawn_combat
        );
    }

    #[test]
    fn bottled_potential_exact_hand_resumes_draw_suffix() {
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let silent_defend = plain_identity(CardId::DefendSilent, 0);
        let defect_defend = plain_identity(CardId::DefendDefect, 0);
        let regent_defend = plain_identity(CardId::DefendRegent, 0);
        let sculpting = plain_identity(CardId::SculptingStrike, 0);
        let mut builder = CatalogBuilder::new();
        let defend_atom = builder.intern(defend).unwrap();
        let silent_defend_atom = builder.intern(silent_defend).unwrap();
        let defect_defend_atom = builder.intern(defect_defend).unwrap();
        let regent_defend_atom = builder.intern(regent_defend).unwrap();
        let sculpting_atom = builder.intern(sculpting).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 15;
        state.cards_drawn_combat = 0;
        state.history.non_hand_draws_this_turn = 0;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 11,
                atom: silent_defend_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 12,
                atom: defend_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 13,
                atom: defect_defend_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 14,
                atom: regent_defend_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 10,
                atom: sculpting_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::BottledPotential)],
            false,
            false,
            false,
            false,
            true,
        ));
        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));

        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::PotionFinish { .. },
                crate::frame::Frame::Draw { .. },
                crate::frame::Frame::CardPlay { .. },
            ]
        ));
        assert_eq!(parked.next_card_uid, 15);
        let mut normalized_defends = PileId::ALL
            .into_iter()
            .flat_map(|pile| parked.piles.get(pile).as_slice())
            .filter(|card| matches!(card.atom, atom if atom == defend_atom || atom == silent_defend_atom))
            .map(|card| card.uid)
            .collect::<Vec<_>>();
        normalized_defends.sort_unstable();
        assert_eq!(normalized_defends, [11, 12]);
        assert_eq!(parked.rng.get(crate::hot::RngStream::Rng).counter, 4);

        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let parked = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        admit(&document, &parked, &rebuilt_catalog).unwrap();
        let completed = legal_actions(&parked, &rebuilt_catalog)
            .iter()
            .filter_map(|action| apply_action(&parked, &rebuilt_catalog, action).ok())
            .map(|transition| transition.state)
            .find(|candidate| candidate.frames.is_empty() && candidate.pending.is_none())
            .expect("Sculpting choice resumes Bottled Draw and finishes once");
        assert_eq!(completed.fanouts.potion_slots(), [None]);
        assert_eq!(completed.cards_drawn_combat, 5);
        assert_eq!(completed.history.non_hand_draws_this_turn, 5);
    }

    #[test]
    fn bottled_potential_direct_legacy_body_normalizes_before_shuffle() {
        let mut builder = CatalogBuilder::new();
        let silent = builder
            .intern(plain_identity(CardId::DefendSilent, 0))
            .unwrap();
        let ironclad = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 6;
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: LEGACY_CARD_UID,
                atom: silent,
                flags: CARD_FLAG_LEGACY,
            },
            HotCard {
                uid: LEGACY_CARD_UID,
                atom: ironclad,
                flags: CARD_FLAG_LEGACY,
            },
        ]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::BottledPotential)],
            false,
            false,
            false,
            false,
            true,
        ));
        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert!(admit(&predecessor, &state, &catalog).is_err());

        let successor = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(successor.frames.is_empty());
        assert!(successor.pending.is_none());
        assert_eq!(successor.next_card_uid, 8);
        let mut uids = successor
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .map(|card| card.uid)
            .collect::<Vec<_>>();
        uids.sort_unstable();
        assert_eq!(uids, [6, 7]);
        assert!(
            successor
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .all(|card| card.flags & CARD_FLAG_LEGACY == 0)
        );
        assert_eq!(successor.rng.get(crate::hot::RngStream::Rng).counter, 1);
        let document = HotBoundary::try_to_canonical(&successor, &catalog).unwrap();
        assert_eq!(admit(&document, &successor, &catalog), Ok(()));
    }

    #[test]
    fn bottled_potential_preserves_both_distinguishable_tie_input_orders() {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 3;
        state.exact_piles = true;
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 2,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::BottledPotential)],
            false,
            false,
            false,
            false,
            true,
        ));
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&document, &state, &catalog), Ok(()));
        let successor = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert_eq!(successor.fanouts.potion_slots(), [None]);
        assert!(successor.pending.is_none());

        for reversed in [false, true] {
            let mut divergent = state.clone();
            divergent.piles.get_mut(PileId::Draw).make_mut()[0].flags = 0;
            if reversed {
                let hand = divergent.piles.get_mut(PileId::Hand).make_mut()[0];
                let draw = divergent.piles.get_mut(PileId::Draw).make_mut()[0];
                divergent.piles.get_mut(PileId::Hand).make_mut()[0] = draw;
                divergent.piles.get_mut(PileId::Draw).make_mut()[0] = hand;
            }
            let input = [
                divergent.piles.get(PileId::Draw).as_slice()[0].uid,
                divergent.piles.get(PileId::Hand).as_slice()[0].uid,
            ];
            let public = apply_action(
                &divergent,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            )
            .unwrap()
            .state;
            assert_eq!(public.fanouts.potion_slots(), [None]);
            assert!(public.frames.is_empty() && public.pending.is_none());
            assert_eq!(
                public
                    .piles
                    .get(PileId::Hand)
                    .as_slice()
                    .iter()
                    .map(|card| (card.uid, card.flags))
                    .collect::<Vec<_>>(),
                [
                    (
                        input[1],
                        divergent.piles.get(PileId::Hand).as_slice()[0].flags
                    ),
                    (
                        input[0],
                        divergent.piles.get(PileId::Draw).as_slice()[0].flags
                    )
                ]
            );
            let frozen = divergent.piles.get(PileId::Hand).as_slice().to_vec();
            let mut events = Vec::new();
            assert_eq!(
                draw::bottled_potential_shuffle(&mut divergent, &catalog, &frozen, &mut events,),
                Ok(false)
            );
            let actual = divergent
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>();
            assert_eq!(actual, [input[1], input[0]]);
            assert_eq!(
                events,
                [
                    Event::CardResolved {
                        uid: input[1],
                        pile: PileId::Draw,
                    },
                    Event::Reshuffled { cards: 2 },
                ]
            );
        }
    }

    #[test]
    fn bottled_potential_hashset_input_is_discard_then_draw_then_frozen_hand() {
        let mut builder = CatalogBuilder::new();
        let atom = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 4;
        state.exact_piles = true;
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(HotCard {
                uid: 1,
                atom,
                flags: 0,
            });
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 2,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 3,
            atom,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | 4,
        });
        let frozen = state.piles.get(PileId::Hand).as_slice().to_vec();
        let mut events = Vec::new();
        assert_eq!(
            draw::bottled_potential_shuffle(&mut state, &catalog, &frozen, &mut events),
            Ok(false)
        );
        assert_eq!(state.rng.get(crate::hot::RngStream::Rng).counter, 2);
        assert_eq!(
            state
                .piles
                .get(PileId::Draw)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [2, 3, 1]
        );
        assert_eq!(events.last(), Some(&Event::Reshuffled { cards: 3 }));
    }

    #[test]
    fn bottled_potential_stratagem_parks_after_shuffle_roundtrips_and_finishes_once() {
        let mut builder = CatalogBuilder::new();
        let silent = builder
            .intern(plain_identity(CardId::DefendSilent, 0))
            .unwrap();
        let ironclad = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let defect = builder
            .intern(plain_identity(CardId::DefendDefect, 0))
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 4;
        state.exact_piles = true;
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: silent,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: ironclad,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 3,
            atom: defect,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::BottledPotential)],
            false,
            false,
            false,
            false,
            true,
        ));
        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));

        let mut direct = state.clone();
        let mut direct_events = Vec::new();
        assert_eq!(
            potions::use_potion(
                &mut direct,
                &catalog,
                0,
                None,
                PotionId::BottledPotential,
                &mut direct_events,
            ),
            Ok(())
        );
        assert!(direct.pending.is_some(), "{direct:#?}");
        assert_eq!(
            install_parked_action_replay_root(
                &state,
                &mut direct,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            ),
            Ok(())
        );
        assert_eq!(
            play::persisted_card_play_stack_is_exact(&direct, &catalog),
            Ok(())
        );

        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::PotionFinish { .. }
            ]
        ));
        assert!(
            parked
                .pending
                .as_deref()
                .is_some_and(|pending| pending.stratagem_potion_record(&parked.frames).is_some())
        );
        assert_eq!(parked.fanouts.potion_slots(), [None]);
        assert_eq!(parked.rng.get(crate::hot::RngStream::Rng).counter, 2);
        assert_eq!(parked.cards_drawn_combat, 0);
        assert_eq!(legal_actions(&parked, &catalog).len(), 3);
        assert_eq!(
            play::persisted_card_play_stack_is_exact(&parked, &catalog),
            Ok(())
        );

        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let pending_wire = document.player["pending"].as_array().unwrap();
        assert_eq!(pending_wire[0], serde_json::json!("stratagem_select"));
        assert_eq!(
            pending_wire[1],
            serde_json::json!(["select", "draw", 1, 1, null, ["move", "hand", "bottom"]])
        );
        assert_eq!(pending_wire[2].as_array().unwrap().len(), 3);
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &rebuilt, &rebuilt_catalog), Ok(()));
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
            document
        );

        let completed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none());
        assert!(completed.frames.is_empty());
        assert_eq!(completed.fanouts.potion_slots(), [None]);
        assert_eq!(completed.rng.get(crate::hot::RngStream::Rng).counter, 2);
        assert_eq!(completed.cards_drawn_combat, 2);
        assert_eq!(completed.history.non_hand_draws_this_turn, 2);
        assert_eq!(completed.piles.get(PileId::Hand).len(), 3);

        let mut tampered = document.clone();
        tampered.player.get_mut("pending").unwrap()[1][2] = serde_json::json!(2);
        assert!(HotBoundary::from_canonical(&tampered, &rebuilt_catalog).is_err());
        let mut reordered = document.clone();
        reordered.player.get_mut("pending").unwrap()[2]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        assert!(HotBoundary::from_canonical(&reordered, &rebuilt_catalog).is_err());
        let mut removed = document.clone();
        removed.player.get_mut("pending").unwrap()[2]
            .as_array_mut()
            .unwrap()
            .pop();
        assert!(HotBoundary::from_canonical(&removed, &rebuilt_catalog).is_err());
        let mut duplicated = document.clone();
        let first = duplicated.player["pending"][2][0].clone();
        duplicated.player.get_mut("pending").unwrap()[2]
            .as_array_mut()
            .unwrap()
            .push(first);
        assert!(HotBoundary::from_canonical(&duplicated, &rebuilt_catalog).is_err());
        let mut changed_id = document.clone();
        changed_id.player.get_mut("pending").unwrap()[2][0][1][0] =
            serde_json::json!(CardId::DefendSilent.as_str());
        assert!(HotBoundary::from_canonical(&changed_id, &rebuilt_catalog).is_err());
        let mut wrong_root = document.clone();
        wrong_root.continuations[0]
            .fields
            .get_mut("action")
            .unwrap()["slot"] = serde_json::json!(1);
        assert!(HotBoundary::from_canonical(&wrong_root, &rebuilt_catalog).is_err());
    }

    #[test]
    fn dark_embrace_ethereal_draw_parks_roundtrips_resets_once_and_rolls_back() {
        let mut builder = CatalogBuilder::new();
        let dazed = builder
            .intern_reachable(plain_identity(CardId::Dazed, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_live_dark_embrace_reachable();
        builder.mark_live_stratagem_reachable();
        let catalog = builder.build();
        assert!(catalog.requires_action_replay());

        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 6;
        state.exact_piles = true;
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 2);
        state
            .fanouts
            .register_after_side_turn_end_power(crate::hot::AfterSideTurnEndPowerToken::DarkEmbrace)
            .unwrap();
        state.powers.set(PowerId::DoubleDamage, SlotWire::Int, 2);
        state
            .fanouts
            .register_after_side_turn_end_power(
                crate::hot::AfterSideTurnEndPowerToken::DoubleDamage,
            )
            .unwrap();
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
        );
        state.rng.set(
            crate::hot::RngStream::Rng,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: dazed,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: dazed,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((3..=5).map(|uid| HotCard {
                uid,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));
        let mut conqueror_owner = HotMonster::new(MonsterKind::Toadpole, 50);
        conqueror_owner.max_hp = 50;
        conqueror_owner
            .powers
            .set(PowerId::Conqueror, SlotWire::Int, 2);
        conqueror_owner
            .misery_debuff_order
            .push(crate::hot::MiseryToken::Conqueror);
        state.monsters_mut().push(conqueror_owner);

        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert_eq!(parked.fanouts.dark_embrace_ethereal(), 2);
        assert_eq!(
            parked.powers.value(PowerId::DoubleDamage),
            1,
            "the captured suffix advances before the Dark child is exposed"
        );
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::AfterSideTurnEndPower { .. },
                crate::frame::Frame::DarkEmbraceSideEnd { .. },
                crate::frame::Frame::Draw { .. }
            ]
        ));
        let draw_record = match parked.frames.as_slice()[3] {
            crate::frame::Frame::Draw { record } => record,
            _ => unreachable!(),
        };
        let draw = parked.frames.draw(draw_record).unwrap();
        assert_eq!(draw.caller, crate::hot::DrawCaller::DarkEmbraceSideEnd);
        assert_eq!(
            draw.requested, 4,
            "live Amount 2 times true-Ethereal tally 2"
        );
        assert!(parked.pending.is_some());

        let canonical = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        assert_eq!(
            canonical.continuations[1].frame_type,
            "AfterSideTurnEndPowerFrame"
        );
        assert_eq!(
            canonical.continuations[2].frame_type,
            "DarkEmbraceSideEndFrame"
        );
        assert_eq!(
            canonical.continuations[3].fields["caller"],
            serde_json::json!("dark_embrace_side_end")
        );
        assert_eq!(
            canonical.continuations[3].fields["caller_locals"],
            serde_json::json!([])
        );
        assert_eq!(
            canonical.continuations[1].fields["conqueror_reachable_at_entry"],
            serde_json::json!(true),
            "the parked owner retains the entry-time enemy suffix carrier"
        );
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&canonical).unwrap();
        let rebuilt = HotBoundary::from_canonical(&canonical, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&canonical, &rebuilt, &rebuilt_catalog), Ok(()));
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
            canonical
        );

        let mut future_uid = canonical.clone();
        let next_uid = future_uid.player["next_after_side_turn_end_power_uid"].clone();
        future_uid.continuations[1]
            .fields
            .get_mut("listeners")
            .unwrap()[0][1] = next_uid.clone();
        future_uid.continuations[1]
            .fields
            .insert("dark_pending_uid".to_owned(), next_uid);
        assert!(HotBoundary::from_canonical(&future_uid, &rebuilt_catalog).is_err());

        let mut wrong_caller = canonical.clone();
        wrong_caller.continuations[3]
            .fields
            .insert("caller".to_owned(), serde_json::json!("none"));
        assert!(HotBoundary::from_canonical(&wrong_caller, &rebuilt_catalog).is_err());
        let mut wrong_requested = canonical.clone();
        wrong_requested.continuations[3]
            .fields
            .insert("requested".to_owned(), serde_json::json!(3));
        assert!(HotBoundary::from_canonical(&wrong_requested, &rebuilt_catalog).is_err());

        let before = rebuilt.clone();
        let mut events = vec![Event::TurnBegan { turn: 99 }];
        assert!(
            apply_action_into(
                &rebuilt,
                &rebuilt_catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(99),
                },
                &mut events,
            )
            .is_err()
        );
        assert_eq!(rebuilt, before);
        assert!(events.is_empty());

        let mut forged_private = rebuilt.clone();
        forged_private
            .powers
            .set(PowerId::JugglingAttacks, SlotWire::Int, 2);
        let before = forged_private.clone();
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(
                &forged_private,
                &rebuilt_catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(0),
                },
                &mut events,
            ),
            // The forged private power makes the answer illegal, which is a
            // legality refusal and says so since #2473.
            Err(EngineRefusal::ActionNotLegal("public action legality"))
        );
        assert_eq!(forged_private, before);
        assert!(events.is_empty());

        let completed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.fanouts.dark_embrace_ethereal(), 0);
        assert_eq!(
            completed.powers.value(PowerId::DoubleDamage),
            1,
            "resume must not replay the already-completed suffix"
        );
        assert_eq!(completed.history.owner_cards_exhausted_combat, 2);
        assert_eq!(
            completed.monsters[0].powers.value(PowerId::Conqueror),
            1,
            "resume runs the retained post-Hook enemy suffix exactly once"
        );
    }

    #[test]
    fn dark_embrace_draw_authenticates_recursive_hellraiser_cardplay_draw() {
        let mut builder = CatalogBuilder::new();
        let dazed = builder
            .intern_reachable(plain_identity(CardId::Dazed, 0))
            .unwrap();
        let pommel = builder
            .intern_reachable(plain_identity(CardId::PommelStrike, 0))
            .unwrap();
        let seeker = builder
            .intern_reachable(plain_identity(CardId::SeekerStrike, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_live_dark_embrace_reachable();
        builder.mark_live_hellraiser_reachable();
        let catalog = builder.build();
        assert!(catalog.dark_embrace_side_end_can_suspend());

        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 7;
        state.exact_piles = true;
        state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
        state
            .fanouts
            .register_after_side_turn_end_power(crate::hot::AfterSideTurnEndPowerToken::DarkEmbrace)
            .unwrap();
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        state
            .fanouts
            .register_after_side_turn_end_power(crate::hot::AfterSideTurnEndPowerToken::Hellraiser)
            .unwrap();
        state.rng.set(
            crate::hot::RngStream::Sel,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        assert!(
            state
                .fanouts
                .set_potion_belt(vec![None], false, false, false, false, true,)
        );
        assert!(
            state
                .fanouts
                .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: dazed,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 2,
                atom: pommel,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: seeker,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 6,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters_mut().push(monster);

        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::AfterSideTurnEndPower { .. },
                    crate::frame::Frame::DarkEmbraceSideEnd { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. }
                ]
            ),
            "nested Dark/CardPlay frames: {:#?}",
            parked.frames.as_slice()
        );
        let canonical = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&canonical).unwrap();
        let rebuilt = HotBoundary::from_canonical(&canonical, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&canonical, &rebuilt, &rebuilt_catalog), Ok(()));
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
            canonical
        );

        let completed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.fanouts.dark_embrace_ethereal(), 0);
    }

    #[test]
    fn dark_hellraiser_reset_peers_preserve_native_order_across_a_park() {
        #[derive(Copy, Clone, Debug)]
        enum Peer {
            Juggling,
            Panache,
            PaleBlueDot,
        }

        fn fixture(peer: Peer, reset_before_dark: bool, selecting: bool) -> (HotState, Catalog) {
            let mut builder = CatalogBuilder::new();
            let dazed = builder
                .intern_reachable(plain_identity(CardId::Dazed, 0))
                .unwrap();
            let pommel = builder
                .intern_reachable(plain_identity(CardId::PommelStrike, 0))
                .unwrap();
            let seeker = builder
                .intern_reachable(plain_identity(CardId::SeekerStrike, 0))
                .unwrap();
            let strike = builder
                .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
                .unwrap();
            let defend = builder
                .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
                .unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.mark_live_dark_embrace_reachable();
            builder.mark_live_hellraiser_reachable();
            let catalog = builder.build();

            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 3;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = if selecting { 13 } else { 9 };
            state.exact_piles = true;
            state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
            state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
            let peer_uid = u32::from(!reset_before_dark);
            state.fanouts.set_next_after_side_turn_end_power_uid(3);
            match peer {
                Peer::Juggling => {
                    state.powers.set(PowerId::Juggling, SlotWire::Int, 1);
                    state.powers.set(PowerId::JugglingAttacks, SlotWire::Int, 0);
                }
                Peer::Panache => {
                    state.powers.set(PowerId::Panache, SlotWire::Int, 10);
                    assert!(state.fanouts.set_panache_instances(&[PanacheInstance {
                        uid: peer_uid,
                        amount: 10,
                        cards_left: 5,
                        already_applied: true,
                    }]));
                }
                Peer::PaleBlueDot => {
                    state.powers.set(PowerId::PaleBlueDot, SlotWire::Int, 1);
                    state.history.owner_card_plays_finished_this_turn =
                        if selecting { 3 } else { 4 };
                }
            }
            let peer_token = match peer {
                Peer::Juggling => crate::hot::AfterSideTurnEndPowerToken::Juggling,
                Peer::Panache => crate::hot::AfterSideTurnEndPowerToken::Panache,
                Peer::PaleBlueDot => crate::hot::AfterSideTurnEndPowerToken::PaleBlueDot,
            };
            let order = if reset_before_dark {
                [
                    peer_token,
                    crate::hot::AfterSideTurnEndPowerToken::DarkEmbrace,
                    crate::hot::AfterSideTurnEndPowerToken::Hellraiser,
                ]
            } else {
                [
                    crate::hot::AfterSideTurnEndPowerToken::DarkEmbrace,
                    peer_token,
                    crate::hot::AfterSideTurnEndPowerToken::Hellraiser,
                ]
            };
            assert!(
                state
                    .fanouts
                    .set_after_side_turn_end_power_order(&order.map(|token| {
                        crate::hot::AfterSideTurnEndPowerEntry {
                            token,
                            uid: if token == crate::hot::AfterSideTurnEndPowerToken::Hellraiser {
                                2
                            } else if token == peer_token {
                                peer_uid
                            } else {
                                u32::from(reset_before_dark)
                            },
                        }
                    }))
            );
            crate::engine::play::hydrate_after_card_played_power_order_for_test(&mut state);
            assert!(
                state
                    .fanouts
                    .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
            );
            state.rng.set(
                crate::hot::RngStream::Sel,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: dazed,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            if selecting {
                state.piles.get_mut(PileId::Draw).make_mut().extend([
                    HotCard {
                        uid: 2,
                        atom: pommel,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    },
                    HotCard {
                        uid: 3,
                        atom: seeker,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    },
                    HotCard {
                        uid: 4,
                        atom: defend,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    },
                    HotCard {
                        uid: 5,
                        atom: defend,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    },
                ]);
                state
                    .piles
                    .get_mut(PileId::Draw)
                    .make_mut()
                    .extend((6..=12).map(|uid| HotCard {
                        uid,
                        atom: defend,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    }));
            } else {
                state.piles.get_mut(PileId::Draw).make_mut().extend(
                    std::iter::once(HotCard {
                        uid: 2,
                        atom: strike,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    })
                    .chain((3..=8).map(|uid| HotCard {
                        uid,
                        atom: defend,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    })),
                );
            }
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
            monster.max_hp = 1_000;
            state.monsters_mut().push(monster);
            (state, catalog)
        }

        for peer in [Peer::Juggling, Peer::Panache, Peer::PaleBlueDot] {
            for reset_before_dark in [false, true] {
                let (state, catalog) = fixture(peer, reset_before_dark, true);
                let parked = apply_action(&state, &catalog, &Action::EndTurn)
                    .unwrap()
                    .state;
                assert!(parked.pending.is_some());
                match peer {
                    Peer::Juggling => assert_eq!(
                        parked.powers.value(PowerId::JugglingAttacks),
                        if reset_before_dark { 2 } else { 0 }
                    ),
                    Peer::Panache => {
                        assert_eq!(
                            parked
                                .fanouts
                                .panache_instance(u32::from(!reset_before_dark))
                                .unwrap()
                                .cards_left,
                            5
                        )
                    }
                    Peer::PaleBlueDot => {
                        assert!(!parked.fanouts.pale_blue_dot_used());
                        assert_eq!(parked.powers.value(PowerId::DrawNextTurn), 0);
                    }
                }
                let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
                let rebuilt_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
                assert_eq!(
                    wire.continuations[1].frame_type,
                    "AfterSideTurnEndPowerFrame"
                );
                for (field, value) in [
                    ("cursor", serde_json::json!(0)),
                    ("dark_pending_uid", serde_json::json!(99)),
                    ("ordinary_suffix_invoked", serde_json::json!(false)),
                ] {
                    let mut forged = wire.clone();
                    forged.continuations[1]
                        .fields
                        .insert(field.to_owned(), value);
                    assert!(
                        HotBoundary::from_canonical(&forged, &rebuilt_catalog).is_err(),
                        "forged {peer:?} {field}"
                    );
                }
                let mut wrong_listener_uid = wire.clone();
                wrong_listener_uid.continuations[1]
                    .fields
                    .get_mut("listeners")
                    .unwrap()[0][1] = serde_json::json!(99);
                assert!(
                    HotBoundary::from_canonical(&wrong_listener_uid, &rebuilt_catalog).is_err()
                );
                let mut wrong_listener_order = wire.clone();
                wrong_listener_order.continuations[1]
                    .fields
                    .get_mut("listeners")
                    .unwrap()
                    .as_array_mut()
                    .unwrap()
                    .swap(0, 1);
                assert!(
                    HotBoundary::from_canonical(&wrong_listener_order, &rebuilt_catalog).is_err()
                );
                let mut wrong_peer_payload = wire.clone();
                wrong_peer_payload.continuations[1]
                    .fields
                    .get_mut("listeners")
                    .unwrap()[usize::from(!reset_before_dark)][2] = serde_json::json!(99);
                assert!(
                    HotBoundary::from_canonical(&wrong_peer_payload, &rebuilt_catalog).is_err()
                );
                let rebuilt = HotBoundary::from_canonical(&wire, &rebuilt_catalog).unwrap();
                assert_eq!(admit(&wire, &rebuilt, &rebuilt_catalog), Ok(()));
                let completed_transition = apply_action(
                    &rebuilt,
                    &rebuilt_catalog,
                    &Action::Select {
                        answer: SelectionAnswer::OptionIndex(0),
                    },
                )
                .unwrap();
                let completed = completed_transition.state;
                assert!(completed.pending.is_none() && completed.frames.is_empty());
                match peer {
                    Peer::Juggling => assert_eq!(
                        completed.powers.value(PowerId::JugglingAttacks),
                        if reset_before_dark { 2 } else { 0 }
                    ),
                    Peer::Panache => {
                        assert_eq!(
                            completed
                                .fanouts
                                .panache_instance(u32::from(!reset_before_dark))
                                .unwrap()
                                .cards_left,
                            3
                        )
                    }
                    Peer::PaleBlueDot => {
                        assert!(completed.fanouts.pale_blue_dot_used());
                        assert_eq!(completed.powers.value(PowerId::DrawNextTurn), 0);
                        assert_eq!(
                            completed_transition
                                .events
                                .iter()
                                .filter(|event| matches!(
                                    event,
                                    Event::PowerChanged {
                                        subject: Subject::Player,
                                        power: PowerId::DrawNextTurn,
                                        amount: 1,
                                    }
                                ))
                                .count(),
                            1,
                            "Pale Blue Dot publishes its next-turn draw exactly once before turn-start consumes it"
                        );
                    }
                }

                let (sync, sync_catalog) = fixture(peer, reset_before_dark, false);
                let sync_transition = apply_action(&sync, &sync_catalog, &Action::EndTurn).unwrap();
                let sync = sync_transition.state;
                assert!(sync.pending.is_none() && sync.frames.is_empty());
                match peer {
                    Peer::Juggling => assert_eq!(
                        sync.powers.value(PowerId::JugglingAttacks),
                        if reset_before_dark { 1 } else { 0 }
                    ),
                    Peer::Panache => assert_eq!(
                        sync.fanouts
                            .panache_instance(u32::from(!reset_before_dark))
                            .unwrap()
                            .cards_left,
                        if reset_before_dark { 4 } else { 5 }
                    ),
                    Peer::PaleBlueDot => {
                        assert_eq!(sync.fanouts.pale_blue_dot_used(), reset_before_dark);
                        assert_eq!(sync.powers.value(PowerId::DrawNextTurn), 0);
                        assert_eq!(
                            sync_transition
                                .events
                                .iter()
                                .filter(|event| matches!(
                                    event,
                                    Event::PowerChanged {
                                        subject: Subject::Player,
                                        power: PowerId::DrawNextTurn,
                                        amount: 1,
                                    }
                                ))
                                .count(),
                            1
                        );
                    }
                }
            }
        }
    }

    /// #3383: Tender joins the Dark Embrace + Hellraiser side-end peers that
    /// the AfterSideTurnEnd object ledger orders exactly.
    ///
    /// `TenderPower/<AfterCardPlayed>d__12::MoveNext` RVA `0x348e10`
    /// (v0.111.0, SHA-256 `9cb4f1ad…`) counts every owner card, AutoPlay
    /// included (owner test IL_0028-0042, `CardsPlayedThisTurn += 1` at
    /// IL_004a-0054), then Applies -1 Strength (IL_0078) and -1 Dexterity
    /// (IL_00ec). `<AfterSideTurnEnd>d__13::MoveNext` RVA `0x348fb4` re-reads
    /// the live counter for its +Strength (IL_0049) and +Dexterity (IL_00c3)
    /// Applies and zeroes it after them (IL_012e-012f).
    ///
    /// The fixture has one card already played this turn (count 1, Strength
    /// and Dexterity -1), a Dazed in Hand for Dark Embrace's tally, and a
    /// Strike on top of the Draw pile.
    ///
    /// * Synchronous Strike, Dark before Tender: the Strike counts
    ///   (count 2, -2/-2), and Tender then restores both cards and zeroes.
    /// * Synchronous Strike, Tender before Dark: Tender restores the one card
    ///   and zeroes, and the Strike afterwards carries into the next turn
    ///   (count 1, -1/-1).
    /// * Parked (Pommel Strike draws a selecting Seeker Strike):
    ///   `Hook/<AfterSideTurnEnd>d__81::MoveNext` RVA `0x3d11c0` awaits each
    ///   listener through `HookPlayerChoiceContext.
    ///   AssignTaskAndWaitForPauseOrCompletion`. So Tender's callback runs
    ///   before the choice in either order, and neither in-flight play has
    ///   reached AfterCardPlayed yet. Both plays then count after the answer:
    ///   count 2, -2/-2, whichever order the ledger holds.
    #[test]
    fn dark_hellraiser_tender_side_end_follows_the_ledger_order() {
        fn fixture(tender_before_dark: bool, selecting: bool) -> (HotState, Catalog) {
            let mut builder = CatalogBuilder::new();
            let dazed = builder
                .intern_reachable(plain_identity(CardId::Dazed, 0))
                .unwrap();
            let pommel = builder
                .intern_reachable(plain_identity(CardId::PommelStrike, 0))
                .unwrap();
            let seeker = builder
                .intern_reachable(plain_identity(CardId::SeekerStrike, 0))
                .unwrap();
            let strike = builder
                .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
                .unwrap();
            let defend = builder
                .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
                .unwrap();
            builder.intern_private_tender_hunter().unwrap();
            builder.mark_live_dark_embrace_reachable();
            builder.mark_live_hellraiser_reachable();
            let catalog = builder.build();

            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 3;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = if selecting { 13 } else { 9 };
            state.exact_piles = true;
            state.powers.set(PowerId::DarkEmbrace, SlotWire::Int, 1);
            state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
            state.powers.set(PowerId::Tender, SlotWire::Int, 1);
            assert!(state.fanouts.set_tender_state(true, 1));
            state.powers.set(PowerId::Strength, SlotWire::Int, -1);
            state.powers.set(PowerId::Dexterity, SlotWire::Int, -1);
            state.fanouts.set_next_after_side_turn_end_power_uid(3);
            let tender_uid = u32::from(!tender_before_dark);
            let order = if tender_before_dark {
                [
                    crate::hot::AfterSideTurnEndPowerToken::Tender,
                    crate::hot::AfterSideTurnEndPowerToken::DarkEmbrace,
                    crate::hot::AfterSideTurnEndPowerToken::Hellraiser,
                ]
            } else {
                [
                    crate::hot::AfterSideTurnEndPowerToken::DarkEmbrace,
                    crate::hot::AfterSideTurnEndPowerToken::Tender,
                    crate::hot::AfterSideTurnEndPowerToken::Hellraiser,
                ]
            };
            assert!(
                state
                    .fanouts
                    .set_after_side_turn_end_power_order(&order.map(|token| {
                        crate::hot::AfterSideTurnEndPowerEntry {
                            token,
                            uid: match token {
                                crate::hot::AfterSideTurnEndPowerToken::Hellraiser => 2,
                                crate::hot::AfterSideTurnEndPowerToken::Tender => tender_uid,
                                _ => u32::from(tender_before_dark),
                            },
                        }
                    }))
            );
            crate::engine::play::hydrate_after_card_played_power_order_for_test(&mut state);
            assert!(
                state
                    .fanouts
                    .set_after_card_exhausted_order(&[PowerId::DarkEmbrace])
            );
            state.rng.set(
                crate::hot::RngStream::Sel,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: dazed,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            let top: Vec<HotCard> = if selecting {
                vec![
                    HotCard {
                        uid: 2,
                        atom: pommel,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    },
                    HotCard {
                        uid: 3,
                        atom: seeker,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    },
                ]
            } else {
                vec![HotCard {
                    uid: 2,
                    atom: strike,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                }]
            };
            let first_defend = if selecting { 4 } else { 3 };
            let next_uid = state.next_card_uid;
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend(
                    top.into_iter()
                        .chain((first_defend..next_uid).map(|uid| HotCard {
                            uid,
                            atom: defend,
                            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                        })),
                );
            let mut hunter = HotMonster::new(MonsterKind::HunterKiller, 126);
            hunter.max_hp = 126;
            hunter.uid = 23;
            let mut ai = crate::hot::RandomAiState::new();
            assert!(ai.set_next(Some(crate::engine::turn::HUNTER_BITE_INDEX)));
            assert!(ai.set_log(&[
                crate::engine::turn::HUNTER_GOOP_INDEX,
                crate::engine::turn::HUNTER_BITE_INDEX,
            ]));
            hunter.random_ai = ai;
            state.monsters_mut().push(hunter);
            (state, catalog)
        }

        fn stats(state: &HotState) -> (i32, i32, i32) {
            (
                state.fanouts.tender_cards_played(),
                state.powers.value(PowerId::Strength),
                state.powers.value(PowerId::Dexterity),
            )
        }

        for tender_before_dark in [false, true] {
            // The retired admission wall: the entry admits.
            let (sync, catalog) = fixture(tender_before_dark, false);
            let entry = HotBoundary::try_to_canonical(&sync, &catalog).unwrap();
            assert_eq!(admit(&entry, &sync, &catalog), Ok(()));
            let done = apply_action(&sync, &catalog, &Action::EndTurn)
                .unwrap()
                .state;
            assert!(done.pending.is_none() && done.frames.is_empty());
            if tender_before_dark {
                // Tender restored first, so the Strike hits at Strength 0
                // and its own -1/-1 carries into the next turn.
                assert_eq!(stats(&done), (1, -1, -1));
                assert_eq!(done.monsters[0].hp, 126 - 6);
            } else {
                // The Strike hits at Strength -1, then Tender restores both.
                assert_eq!(stats(&done), (0, 0, 0));
                assert_eq!(done.monsters[0].hp, 126 - 5);
            }

            let (state, catalog) = fixture(tender_before_dark, true);
            let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));
            let parked = apply_action(&state, &catalog, &Action::EndTurn)
                .unwrap()
                .state;
            assert!(parked.pending.is_some());
            // Tender's callback ran before the park in either order, and
            // neither in-flight play has reached AfterCardPlayed yet.
            assert_eq!(stats(&parked), (0, 0, 0));
            let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
            let rebuilt_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
            let rebuilt = HotBoundary::from_canonical(&wire, &rebuilt_catalog).unwrap();
            assert_eq!(admit(&wire, &rebuilt, &rebuilt_catalog), Ok(()));
            assert_eq!(
                HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
                wire
            );
            let completed = apply_action(
                &rebuilt,
                &rebuilt_catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(0),
                },
            )
            .unwrap()
            .state;
            assert!(completed.pending.is_none() && completed.frames.is_empty());
            assert_eq!(stats(&completed), (2, -2, -2));
            // Pommel and Seeker both hit before the pause: at Strength -1
            // when Dark precedes Tender, at 0 when Tender already restored.
            assert_eq!(
                completed.monsters[0].hp,
                if tender_before_dark { 108 } else { 110 }
            );
        }
    }

    #[test]
    fn stratagem_freezes_native_view_order_but_unranks_payload_order() {
        let ids = [
            CardId::Bash,
            CardId::DefendIronclad,
            CardId::StrikeIronclad,
            CardId::Rage,
            CardId::Thunderclap,
            CardId::Toxic,
        ];
        let mut builder = CatalogBuilder::new();
        let atoms = ids.map(|id| builder.intern(plain_identity(id, 0)).unwrap());
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 7;
        state.exact_piles = true;
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        // Deliberately reverse the already mixed-rarity source. Full-shuffle
        // RNG may permute it again; the frozen view must be independent of
        // that physical order except for true compare ties.
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend(atoms.iter().rev().enumerate().map(|(index, atom)| HotCard {
                uid: u32::try_from(index + 1).unwrap(),
                atom: *atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::BottledPotential)],
            false,
            false,
            false,
            false,
            true,
        ));
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let frozen = parked
            .pending
            .as_deref()
            .unwrap()
            .stratagem_potion_record(&parked.frames)
            .unwrap()
            .candidates()
            .map(|card| catalog.spec(card.atom).unwrap().identity.id)
            .collect::<Vec<_>>();
        assert_eq!(
            frozen,
            [
                CardId::Bash,
                CardId::DefendIronclad,
                CardId::StrikeIronclad,
                CardId::Thunderclap,
                CardId::Rage,
                CardId::Toxic,
            ]
        );
        let action_order = (0..6)
            .map(|ordinal| {
                let selected = draw::stratagem_selection_at(&parked, &catalog, ordinal).unwrap();
                assert_eq!(selected.len(), 1);
                catalog.spec(selected[0].atom).unwrap().identity.id
            })
            .collect::<Vec<_>>();
        assert_eq!(
            action_order,
            [
                CardId::Toxic,
                CardId::Thunderclap,
                CardId::StrikeIronclad,
                CardId::Rage,
                CardId::DefendIronclad,
                CardId::Bash,
            ]
        );
    }

    #[test]
    fn stacked_stratagem_offers_every_physical_pick_order_and_cold_resumes() {
        for amount in [2_u32, 3] {
            let mut builder = CatalogBuilder::new();
            let defend = builder
                .intern(plain_identity(CardId::DefendIronclad, 0))
                .unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = amount + 2;
            state.exact_piles = true;
            state
                .powers
                .set(PowerId::Stratagem, SlotWire::Int, amount as i32);
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend((1..=amount + 1).map(|uid| HotCard {
                    uid,
                    atom: defend,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                }));
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
            monster.max_hp = 100;
            monster.loop_pos = 2;
            state.monsters = std::sync::Arc::new(vec![monster]);
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::BottledPotential)],
                false,
                false,
                false,
                false,
                true
            ));
            let doc = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            admit(&doc, &state, &catalog).unwrap();
            let parked = apply_action(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            )
            .unwrap()
            .state;
            let frozen = parked
                .pending
                .as_deref()
                .unwrap()
                .stratagem_potion_record(&parked.frames)
                .unwrap()
                .candidates()
                .map(|card| card.uid)
                .collect::<Vec<_>>();
            let expected_indices: Vec<Vec<usize>> = if amount == 2 {
                vec![
                    vec![1, 2],
                    vec![2, 1],
                    vec![0, 2],
                    vec![2, 0],
                    vec![0, 1],
                    vec![1, 0],
                ]
            } else {
                [[1, 2, 3], [0, 2, 3], [0, 1, 3], [0, 1, 2]]
                    .into_iter()
                    .flat_map(|[a, b, c]| {
                        [
                            vec![a, b, c],
                            vec![a, c, b],
                            vec![b, a, c],
                            vec![b, c, a],
                            vec![c, a, b],
                            vec![c, b, a],
                        ]
                    })
                    .collect()
            };
            let expected: Vec<Vec<u32>> = expected_indices
                .iter()
                .map(|indices| indices.iter().map(|index| frozen[*index]).collect())
                .collect();
            assert_eq!(legal_actions(&parked, &catalog).len(), expected.len());
            let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
            let cold_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
            let cold = HotBoundary::from_canonical(&wire, &cold_catalog).unwrap();
            admit(&wire, &cold, &cold_catalog).unwrap();
            for (ordinal, picks) in expected.iter().enumerate() {
                let chosen =
                    draw::stratagem_selection_at(&parked, &catalog, ordinal as u32).unwrap();
                assert_eq!(chosen.iter().map(|c| c.uid).collect::<Vec<_>>(), *picks);
                let action = Action::Select {
                    answer: SelectionAnswer::OptionIndex(ordinal as u32),
                };
                let next = apply_action(&parked, &catalog, &action).unwrap().state;
                let reloaded = apply_action(&cold, &cold_catalog, &action).unwrap().state;
                assert_eq!(
                    HotBoundary::try_to_canonical(&next, &catalog).unwrap(),
                    HotBoundary::try_to_canonical(&reloaded, &cold_catalog).unwrap()
                );
                let hand = next.piles.get(PileId::Hand).as_slice();
                assert_eq!(
                    hand[..amount as usize]
                        .iter()
                        .map(|c| c.uid)
                        .collect::<Vec<_>>(),
                    *picks
                );
                assert!(next.pending.is_none() && next.frames.is_empty());
                assert_eq!(next.fanouts.potion_slots(), [None]);
            }
            assert!(
                draw::stratagem_selection_at(&parked, &catalog, expected.len() as u32).is_err()
            );
        }
    }

    #[test]
    fn stacked_stratagem_pick_order_decides_which_card_fills_the_last_hand_slot() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 13;
        state.exact_piles = true;
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 2);
        for (pile, uids) in [(PileId::Discard, 1..=3), (PileId::Hand, 4..=12)] {
            state
                .piles
                .get_mut(pile)
                .make_mut()
                .extend(uids.map(|uid| HotCard {
                    uid,
                    atom: defend,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                }));
        }
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::SwiftPotion)],
            false,
            false,
            false,
            false,
            true
        ));
        let doc = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        admit(&doc, &state, &catalog).unwrap();
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let first = draw::stratagem_selection_at(&parked, &catalog, 0).unwrap();
        let reverse = draw::stratagem_selection_at(&parked, &catalog, 1).unwrap();
        assert_eq!(
            [first[0].uid, first[1].uid],
            [reverse[1].uid, reverse[0].uid]
        );
        for (ordinal, selected) in [(0, first), (1, reverse)] {
            let next = apply_action(
                &parked,
                &catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(ordinal),
                },
            )
            .unwrap()
            .state;
            assert_eq!(next.piles.get(PileId::Hand).len(), 10);
            assert_eq!(
                next.piles.get(PileId::Hand).as_slice()[9].uid,
                selected[0].uid
            );
            assert_eq!(next.piles.get(PileId::Discard).as_slice(), &selected[1..]);
            assert!(next.pending.is_none() && next.frames.is_empty());
            assert_eq!(next.fanouts.potion_slots(), [None]);
        }
    }

    #[test]
    fn bottled_stratagem_singleton_surface_handles_sixty_four_equal_cards() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 65;
        state.exact_piles = true;
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((1..=64).map(|uid| HotCard {
                uid,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::BottledPotential)],
            false,
            false,
            false,
            false,
            true,
        ));
        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));

        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let frozen = parked
            .pending
            .as_deref()
            .unwrap()
            .stratagem_potion_record(&parked.frames)
            .unwrap()
            .candidates()
            .collect::<Vec<_>>();
        assert_eq!(frozen.len(), 64);
        let actions = legal_actions(&parked, &catalog);
        assert_eq!(actions.len(), 64);
        assert_eq!(
            actions.first(),
            Some(&Action::Select {
                answer: SelectionAnswer::OptionIndex(0)
            })
        );
        assert_eq!(
            actions.last(),
            Some(&Action::Select {
                answer: SelectionAnswer::OptionIndex(63)
            })
        );
        assert_eq!(
            draw::stratagem_selection_at(&parked, &catalog, 0)
                .unwrap()
                .as_slice(),
            &frozen[63..64]
        );
        assert_eq!(
            draw::stratagem_selection_at(&parked, &catalog, 63)
                .unwrap()
                .as_slice(),
            &frozen[0..1]
        );

        for ordinal in [0, 63] {
            let completed = apply_action(
                &parked,
                &catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(ordinal),
                },
            )
            .unwrap()
            .state;
            assert!(completed.pending.is_none() && completed.frames.is_empty());
            assert_eq!(completed.fanouts.potion_slots(), [None]);
            let selected = if ordinal == 0 { frozen[63] } else { frozen[0] };
            assert!(
                completed
                    .piles
                    .get(PileId::Hand)
                    .as_slice()
                    .contains(&selected)
            );
        }
    }

    #[test]
    fn bottled_stratagem_permutation_overflow_refuses_before_slot_or_shuffle() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 14;
        state.exact_piles = true;
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 12);
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((1..=13).map(|uid| HotCard {
                uid,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::BottledPotential)],
            false,
            false,
            false,
            false,
            true,
        ));
        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert!(admit(&predecessor, &state, &catalog).unwrap_err().contains(
            admission::MissingCapability::ArgumentShape("Stratagem selection cardinality")
        ));
        assert!(!legal_actions(&state, &catalog).contains(&action));
        let before = state.clone();
        let mut events = vec![Event::Reshuffled { cards: 999 }];
        assert_eq!(
            apply_action_into(&state, &catalog, &action, &mut events),
            Err(EngineRefusal::CounterOverflow("stratagem options"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    /// Bottled Potential's shuffle leaves Status Slimed above Uncommon Rage,
    /// and Stratagem's no-choice arm keeps that live order (#3621): the
    /// selector view would have put Rage first.
    #[test]
    fn bottled_stratagem_no_choice_takes_the_shuffled_pile_in_live_order() {
        let mut builder = CatalogBuilder::new();
        let rage = builder.intern(plain_identity(CardId::Rage, 0)).unwrap();
        let slimed = builder.intern(plain_identity(CardId::Slimed, 0)).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 3;
        state.exact_piles = true;
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 2);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: rage,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: slimed,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::BottledPotential)],
            false,
            false,
            false,
            false,
            true,
        ));

        let completed = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.fanouts.potion_slots(), [None]);
        assert_eq!(
            completed
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| catalog.spec(card.atom).unwrap().identity.id)
                .collect::<Vec<_>>(),
            [CardId::Slimed, CardId::Rage]
        );
        assert_eq!(completed.cards_drawn_combat, 0);
        assert_eq!(completed.rng.get(crate::hot::RngStream::Rng).counter, 1);
    }

    #[test]
    fn bottled_draw5_reshuffle_reparks_under_its_own_draw_cursor() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let bash = builder.intern(plain_identity(CardId::Bash, 0)).unwrap();
        let sculpting0 = builder
            .intern(plain_identity(CardId::SculptingStrike, 0))
            .unwrap();
        let sculpting1 = builder
            .intern(plain_identity(CardId::SculptingStrike, 1))
            .unwrap();
        let strike = builder
            .intern(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 6;
        state.exact_piles = true;
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: bash,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: sculpting0,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: sculpting1,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::BottledPotential)],
            false,
            false,
            false,
            false,
            true,
        ));
        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));

        let explicit = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let defend_ordinal = (0..legal_actions(&explicit, &catalog).len())
            .find(|ordinal| {
                draw::stratagem_selection_at(&explicit, &catalog, *ordinal as u32)
                    .is_ok_and(|cards| cards.len() == 1 && cards[0].atom == defend)
            })
            .unwrap();
        let mut current = apply_action(
            &explicit,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(defend_ordinal as u32),
            },
        )
        .unwrap()
        .state;
        for _ in 0..8 {
            if current
                .pending
                .as_deref()
                .is_some_and(|pending| pending.stratagem_draw_record(&current.frames).is_some())
            {
                break;
            }
            assert!(current.pending.is_some(), "{current:#?}");
            current = apply_action(
                &current,
                &catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(0),
                },
            )
            .unwrap()
            .state;
        }
        let nested = current
            .pending
            .as_deref()
            .and_then(|pending| pending.stratagem_draw_record(&current.frames))
            .expect("Draw5's own reshuffle opens the second Stratagem choice");
        assert_eq!(nested.caller, crate::hot::DrawCaller::PotionEpilogue);
        assert_eq!(nested.stage, crate::hot::DrawStage::AfterShuffle);
        assert!(nested.completed > 0 && nested.completed < nested.requested);
        assert_eq!(current.rng.get(crate::hot::RngStream::Rng).counter, 6);

        let document = HotBoundary::try_to_canonical(&current, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let mut current = HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert_eq!(admit(&document, &current, &catalog), Ok(()));
        for _ in 0..8 {
            if current.pending.is_none() {
                break;
            }
            current = apply_action(
                &current,
                &catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(0),
                },
            )
            .unwrap()
            .state;
        }
        assert!(current.pending.is_none() && current.frames.is_empty());
        assert_eq!(current.fanouts.potion_slots(), [None]);
        assert_eq!(current.rng.get(crate::hot::RngStream::Rng).counter, 6);
        assert_eq!(current.cards_drawn_combat, 5);
    }

    #[test]
    fn swift_reshuffle_stratagem_owns_frozen_draw_cursor_and_resumes_once() {
        let mut builder = CatalogBuilder::new();
        let silent = builder
            .intern(plain_identity(CardId::DefendSilent, 0))
            .unwrap();
        let ironclad = builder
            .intern(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let defect = builder
            .intern(plain_identity(CardId::DefendDefect, 0))
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 4;
        state.exact_piles = true;
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            HotCard {
                uid: 1,
                atom: silent,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: ironclad,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defect,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::SwiftPotion)],
            false,
            false,
            false,
            false,
            true,
        ));
        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::PotionFinish { .. },
                crate::frame::Frame::Draw { .. }
            ]
        ));
        let draw = parked
            .pending
            .as_deref()
            .unwrap()
            .stratagem_draw_record(&parked.frames)
            .unwrap();
        assert_eq!(draw.stage, crate::hot::DrawStage::AfterShuffle);
        assert_eq!(draw.completed, 0);
        assert_eq!(draw.shuffle_candidates().len(), 3);
        assert_eq!(parked.rng.get(crate::hot::RngStream::Rng).counter, 2);

        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &rebuilt, &rebuilt_catalog), Ok(()));
        let completed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.frames.is_empty());
        assert!(completed.pending.is_none());
        assert_eq!(completed.fanouts.potion_slots(), [None]);
        assert_eq!(completed.rng.get(crate::hot::RngStream::Rng).counter, 2);
        assert_eq!(completed.cards_drawn_combat, 2);
        assert_eq!(completed.history.non_hand_draws_this_turn, 2);
        assert_eq!(completed.piles.get(PileId::Hand).len(), 3);

        let mut reordered = document.clone();
        reordered.player.get_mut("pending").unwrap()[2]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        assert!(HotBoundary::from_canonical(&reordered, &rebuilt_catalog).is_err());
        let mut payload_drift = document.clone();
        payload_drift.player.get_mut("pending").unwrap()[2][0][1][7] = serde_json::json!(null);
        assert!(HotBoundary::from_canonical(&payload_drift, &rebuilt_catalog).is_err());
    }

    #[test]
    fn end_turn_pagestorm_recurses_twice_then_resumes_one_selecting_hellraiser_strike() {
        let pommel = plain_identity(CardId::PommelStrike, 0);
        let seeker = plain_identity(CardId::SeekerStrike, 0);
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let status = plain_identity(CardId::Dazed, 0);
        let mut builder = CatalogBuilder::new();
        let pommel_atom = builder.intern(pommel).unwrap();
        let seeker_atom = builder.intern(seeker).unwrap();
        let defend_atom = builder.intern(defend).unwrap();
        let status_atom = builder.intern(status).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        state.energy = 3;
        state.turn = 2;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.player_side_active = true;
        state.next_card_uid = 9;
        state.exact_piles = true;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        state.powers.set(PowerId::NoDraw, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.powers.set(PowerId::Pagestorm, SlotWire::Int, 1);
        for stream in [crate::hot::RngStream::Sel, crate::hot::RngStream::Targets] {
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Pagestorm])
        );
        state.piles.set(
            PileId::Hand,
            crate::hot::HotPile::from_cards(vec![HotCard {
                uid: 2,
                atom: defend_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }]),
        );
        state.piles.set(
            PileId::Draw,
            crate::hot::HotPile::from_cards(vec![
                HotCard {
                    uid: 3,
                    atom: status_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 4,
                    atom: status_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 1,
                    atom: pommel_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 5,
                    atom: seeker_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 6,
                    atom: defend_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 7,
                    atom: defend_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 8,
                    atom: defend_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
            ]),
        );
        let predecessor_document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&predecessor_document).unwrap();
        let state = HotBoundary::from_canonical(&predecessor_document, &catalog).unwrap();
        assert_eq!(admit(&predecessor_document, &state, &catalog), Ok(()));

        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert!(parked.pending.is_some(), "{parked:#?}");
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::AfterCardDrawnPower { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::AfterCardDrawnPower { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. },
                ]
            ),
            "{:#?}",
            parked.frames
        );
        assert_eq!(parked.cards_drawn_combat, 4);
        assert_eq!(parked.history.non_hand_draws_this_turn, 3);
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(rebuilt, parked);

        let checkpoint = rebuilt.clone();
        let mut rejected_events = vec![Event::Reshuffled { cards: 999 }];
        assert!(
            apply_action_into(
                &rebuilt,
                &rebuilt_catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(99),
                },
                &mut rejected_events,
            )
            .is_err()
        );
        assert_eq!(rebuilt, checkpoint);
        assert!(rejected_events.is_empty());

        let mut resumed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        for _ in 0..3 {
            if resumed.pending.is_none() {
                break;
            }
            let document = HotBoundary::try_to_canonical(&resumed, &rebuilt_catalog).unwrap();
            let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
            let round_tripped = HotBoundary::from_canonical(&document, &catalog).unwrap();
            resumed = apply_action(
                &round_tripped,
                &catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(0),
                },
            )
            .unwrap()
            .state;
        }
        assert!(resumed.pending.is_none(), "{resumed:#?}");
        assert!(resumed.frames.is_empty(), "{resumed:#?}");
        assert_eq!(resumed.cards_drawn_combat, 9);
        assert_eq!(resumed.history.non_hand_draws_this_turn, 4);
        assert_eq!(resumed.player_phase, admission::PHASE_ORDINARY_ACTIONS);
    }

    #[test]
    fn swift_iteration_nested_draw_round_trips_and_resumes_hellraiser_once() {
        // A non-Ethereal status: Sculpting Strike's filter excludes Ethereal
        // (#3022), and this witness needs two Hand candidates to park.
        let status = plain_identity(CardId::Wound, 0);
        let sculpting = plain_identity(CardId::SculptingStrike, 0);
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let status_atom = builder.intern(status).unwrap();
        let sculpting_atom = builder.intern(sculpting).unwrap();
        let defend_atom = builder.intern(defend).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 4;
        state.exact_piles = true;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.powers.set(PowerId::Iteration, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Iteration])
        );
        state.piles.set(
            PileId::Hand,
            crate::hot::HotPile::from_cards(vec![HotCard {
                uid: 2,
                atom: defend_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }]),
        );
        state.piles.set(
            PileId::Draw,
            crate::hot::HotPile::from_cards(vec![
                HotCard {
                    uid: 3,
                    atom: status_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 1,
                    atom: sculpting_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
            ]),
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::SwiftPotion)],
            false,
            false,
            false,
            false,
            true,
        ));
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::PotionFinish { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::AfterCardDrawnPower { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. },
                ]
            ),
            "{:#?}",
            parked.frames
        );
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(rebuilt, parked);
        let mut resumed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        for _ in 0..3 {
            if resumed.pending.is_none() {
                break;
            }
            let document = HotBoundary::try_to_canonical(&resumed, &rebuilt_catalog).unwrap();
            let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
            let round_tripped = HotBoundary::from_canonical(&document, &catalog).unwrap();
            resumed = apply_action(
                &round_tripped,
                &catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(0),
                },
            )
            .unwrap()
            .state;
        }
        assert!(resumed.pending.is_none(), "{resumed:#?}");
        assert!(resumed.frames.is_empty(), "{resumed:#?}");
        assert!(resumed.fanouts.potion_slots()[0].is_none());
    }

    #[test]
    fn cacophony_reset_generation_is_rewritten_before_nested_draw_suspends() {
        // The drawn Dazed is Ethereal (Pagestorm's trigger); Sculpting
        // Strike's filter excludes Ethereal (#3022), so the two Hand
        // candidates this witness parks on are non-Ethereal Wounds.
        let status = plain_identity(CardId::Dazed, 0);
        let sculpting = plain_identity(CardId::SculptingStrike, 0);
        let mut builder = CatalogBuilder::new();
        let status_atom = builder.intern(status).unwrap();
        let sculpting_atom = builder.intern(sculpting).unwrap();
        let wound_atom = builder.intern(plain_identity(CardId::Wound, 0)).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 5;
        state.exact_piles = true;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.powers.set(PowerId::Cacophony, SlotWire::Int, 1);
        state.powers.set(PowerId::Pagestorm, SlotWire::Int, 1);
        state.fanouts.set_cacophony_left(1);
        assert!(
            state
                .fanouts
                .set_after_card_drawn_order(&[PowerId::Cacophony, PowerId::Pagestorm,])
        );
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 3,
                atom: wound_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: wound_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.piles.set(
            PileId::Draw,
            crate::hot::HotPile::from_cards(vec![
                HotCard {
                    uid: 1,
                    atom: status_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 2,
                    atom: sculpting_atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
            ]),
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::SwiftPotion)],
            false,
            false,
            false,
            false,
            true,
        ));

        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert_eq!(parked.fanouts.cacophony_left(), 33);
        assert_eq!(parked.fanouts.cacophony_resets_completed(), 1);
        let outer_after = match parked.frames.as_slice() {
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::PotionFinish { .. },
                crate::frame::Frame::Draw { .. },
                crate::frame::Frame::AfterCardDrawnPower { record },
                crate::frame::Frame::Draw { .. },
                crate::frame::Frame::CardPlay { .. },
            ] => parked.frames.after_card_drawn_power(*record).unwrap(),
            frames => panic!("unexpected parked stack: {frames:#?}"),
        };
        assert_eq!(outer_after.cursor, 2);
        assert_eq!(outer_after.cacophony_reset_generation, 1);
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(rebuilt, parked);
        let resumed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(resumed.pending.is_none());
        assert!(resumed.frames.is_empty());
        assert_eq!(resumed.fanouts.cacophony_resets_completed(), 1);
    }

    #[test]
    fn powdered_demise_and_weak_potion_preserve_native_side_end_order() {
        for demise_first in [false, true] {
            let (mut state, catalog) = fixture();
            state.monsters_mut().truncate(1);
            state.monsters_mut()[0].hp = 9;
            assert!(state.fanouts.set_potion_belt(
                if demise_first {
                    vec![Some(PotionId::PowderedDemise), Some(PotionId::WeakPotion)]
                } else {
                    vec![Some(PotionId::WeakPotion), Some(PotionId::PowderedDemise)]
                },
                false,
                false,
                false,
                false,
                true,
            ));
            let first = apply_action(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: Some(0),
                },
            )
            .unwrap()
            .state;
            let second = apply_action(
                &first,
                &catalog,
                &Action::UsePotion {
                    slot: 1,
                    target: Some(0),
                },
            )
            .unwrap()
            .state;

            let terminal = apply_action(&second, &catalog, &Action::EndTurn)
                .unwrap()
                .state;

            assert!(terminal.history.over);
            assert_eq!(terminal.monsters[0].hp, 0);
            assert_eq!(
                terminal.monsters[0].powers.value(PowerId::Weak),
                if demise_first { 3 } else { 2 },
                "a lethal Demise suppresses only later potion listeners"
            );
        }
    }

    #[test]
    fn malformed_demise_pivot_refuses_end_turn_atomically() {
        let (mut state, catalog) = fixture();
        state.monsters_mut().truncate(1);
        let owner = &mut state.monsters_mut()[0];
        owner.powers.set(PowerId::Demise, SlotWire::Int, 9);
        owner.powers.set(PowerId::Weak, SlotWire::Int, 3);
        owner.misery_debuff_order.push(MiseryToken::Demise);
        owner.misery_debuff_order.push(MiseryToken::Demise);
        owner.misery_debuff_order.push(MiseryToken::Weak);
        let before = state.clone();
        let mut events = Vec::new();

        assert!(apply_action_into(&state, &catalog, &Action::EndTurn, &mut events).is_err());
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn beetle_juice_restacks_and_belt_buckle_runs_only_after_a_live_last_slot_use() {
        let (mut state, catalog) = fixture();
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Shrink, SlotWire::Int, 2);
        let successor =
            drink_fixture_potion(state, &catalog, PotionId::BeetleJuice, Some(0), true).state;
        assert_eq!(successor.monsters[0].powers.value(PowerId::Shrink), 6);
        assert_eq!(successor.powers.value(PowerId::Dexterity), 2);
        assert!(successor.fanouts.potion_belt_buckle_applied());

        let (mut lethal, lethal_catalog) = fixture();
        for monster in lethal.monsters_mut() {
            monster.hp = 1;
        }
        let terminal = drink_fixture_potion(
            lethal,
            &lethal_catalog,
            PotionId::ExplosiveAmpoule,
            None,
            true,
        )
        .state;
        assert!(terminal.history.over);
        assert_eq!(terminal.fanouts.potion_slots(), [None]);
        assert_eq!(terminal.powers.value(PowerId::Dexterity), 0);
        assert!(!terminal.fanouts.potion_belt_buckle_applied());

        let (mut ship, ship_catalog) = fixture();
        ship.monsters_mut()[0].hp = 5;
        ship.monsters_mut()[1].hp = 0;
        ship.powers.set(PowerId::Juggernaut, SlotWire::Int, 5);
        assert!(
            ship.fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        let ship_terminal =
            drink_fixture_potion(ship, &ship_catalog, PotionId::ShipInABottle, None, true).state;
        assert!(ship_terminal.history.over);
        assert_eq!(ship_terminal.block, 10);
        assert_eq!(ship_terminal.powers.value(PowerId::BlockNextTurn), 0);
        assert_eq!(ship_terminal.powers.value(PowerId::Dexterity), 0);
        assert!(!ship_terminal.fanouts.potion_belt_buckle_applied());
    }

    #[test]
    fn beetle_and_powdered_preflight_artifact_order_ledger_and_amounts() {
        let (mut blocked, catalog) = fixture();
        assert!(blocked.fanouts.set_potion_belt(
            vec![Some(PotionId::BeetleJuice)],
            false,
            true,
            false,
            false,
            true,
        ));
        blocked.monsters_mut()[0]
            .powers
            .set(PowerId::Weak, SlotWire::Int, 2);
        blocked.monsters_mut()[0]
            .powers
            .set(PowerId::Vuln, SlotWire::Int, 3);
        blocked.monsters_mut()[0]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 1);
        let successor = apply_action(
            &blocked,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: Some(0),
            },
        )
        .unwrap()
        .state;
        assert_eq!(successor.monsters[0].powers.value(PowerId::Artifact), 0);
        assert_eq!(successor.monsters[0].powers.value(PowerId::Shrink), 0);
        assert_eq!(successor.fanouts.potion_slots(), [None]);
        assert_eq!(successor.powers.value(PowerId::Dexterity), 2);

        let mut unblocked = blocked.clone();
        unblocked.monsters_mut()[0]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 0);
        let before = unblocked.clone();
        let mut events = vec![Event::TurnBegan { turn: 71 }];
        assert_eq!(
            apply_action_into(
                &unblocked,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: Some(0),
                },
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "Misery power acquisition order"
            ))
        );
        assert_eq!(unblocked, before);
        assert!(events.is_empty());

        for (old, expected) in [(999_999_994, Some(999_999_998)), (999_999_995, None)] {
            let (mut state, catalog) = fixture();
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::BeetleJuice)],
                false,
                false,
                false,
                false,
                true,
            ));
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Shrink, SlotWire::Int, old);
            state.monsters_mut()[0]
                .misery_debuff_order
                .push(MiseryToken::Shrink);
            let action = Action::UsePotion {
                slot: 0,
                target: Some(0),
            };
            match expected {
                Some(expected) => assert_eq!(
                    apply_action(&state, &catalog, &action)
                        .unwrap()
                        .state
                        .monsters[0]
                        .powers
                        .value(PowerId::Shrink),
                    expected
                ),
                None => assert_eq!(
                    apply_action(&state, &catalog, &action),
                    Err(EngineRefusal::CounterOverflow("Beetle Juice Shrink amount"))
                ),
            }
        }

        let (mut malformed_knockdown, catalog) = fixture();
        assert!(malformed_knockdown.fanouts.set_potion_belt(
            vec![Some(PotionId::BeetleJuice)],
            true,
            false,
            false,
            false,
            true,
        ));
        malformed_knockdown.monsters_mut()[0]
            .powers
            .set(PowerId::Knockdown, SlotWire::Int, 2);
        let before = malformed_knockdown.clone();
        let mut events = vec![Event::TurnBegan { turn: 72 }];
        assert_eq!(
            apply_action_into(
                &malformed_knockdown,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: Some(0),
                },
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "Knockdown distinct-instance state"
            ))
        );
        assert_eq!(malformed_knockdown, before);
        assert!(events.is_empty());
    }

    #[test]
    fn powdered_demise_on_a_waterfall_is_an_ordinary_application() {
        // #3428: the Waterfall revival is decided at the side-end Demise
        // anchor, so neither the held closure nor the action refuses it.
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::WaterfallGiant).unwrap();
        let catalog = builder.build();
        for loop_pos in 0..=5 {
            for pressure in [0, 20] {
                let mut state = HotState::at_defaults();
                let mut waterfall = HotMonster::new(MonsterKind::WaterfallGiant, 100);
                waterfall.max_hp = 100;
                waterfall.loop_pos = loop_pos;
                waterfall
                    .powers
                    .set(PowerId::SteamPressure, SlotWire::Int, pressure);
                state.monsters_mut().push(waterfall);
                assert!(state.fanouts.set_potion_belt(
                    vec![Some(PotionId::PowderedDemise)],
                    false,
                    false,
                    false,
                    false,
                    true,
                ));
                let case = format!("loop_pos={loop_pos} pressure={pressure}");
                assert_eq!(
                    potions::target_application_is_exact(
                        &state,
                        PotionId::PowderedDemise,
                        0,
                        false,
                    ),
                    Ok(()),
                    "{case}"
                );
                assert_eq!(
                    potions::held_target_applications_are_exact(&state, 0, 0, 1),
                    Ok(()),
                    "{case}"
                );
                let action = Action::UsePotion {
                    slot: 0,
                    target: Some(0),
                };
                assert!(legal_actions(&state, &catalog).contains(&action), "{case}");
                let after = apply_action(&state, &catalog, &action).unwrap().state;
                assert_eq!(after.monsters[0].powers.value(PowerId::Demise), 9, "{case}");
                assert_eq!(
                    after.monsters[0].misery_debuff_order.as_slice(),
                    [MiseryToken::Demise],
                    "{case}"
                );
            }
        }
    }

    #[test]
    fn held_belt_admission_closes_duplicate_targeted_amounts_serially() {
        for (potion, power, token, safe_old, unsafe_old) in [
            (
                PotionId::BeetleJuice,
                PowerId::Shrink,
                MiseryToken::Shrink,
                999_999_990,
                999_999_994,
            ),
            (
                PotionId::PowderedDemise,
                PowerId::Demise,
                MiseryToken::Demise,
                i32::MAX - 18,
                i32::MAX - 9,
            ),
        ] {
            let (mut safe, catalog) = fixture();
            assert!(safe.fanouts.set_potion_belt(
                vec![Some(potion), Some(potion)],
                false,
                false,
                false,
                false,
                true,
            ));
            safe.monsters_mut()[0]
                .powers
                .set(power, SlotWire::Int, safe_old);
            safe.monsters_mut()[0].misery_debuff_order.push(token);
            let document = HotBoundary::to_canonical(&safe, &catalog);
            assert_eq!(admit(&document, &safe, &catalog), Ok(()), "{potion:?}");

            let first = apply_action(
                &safe,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: Some(0),
                },
            )
            .unwrap()
            .state;
            assert!(
                legal_actions(&first, &catalog).contains(&Action::UsePotion {
                    slot: 1,
                    target: Some(0),
                })
            );
            let second = apply_action(
                &first,
                &catalog,
                &Action::UsePotion {
                    slot: 1,
                    target: Some(0),
                },
            )
            .unwrap()
            .state;
            let increment = if potion == PotionId::BeetleJuice {
                8
            } else {
                18
            };
            assert_eq!(second.monsters[0].powers.value(power), safe_old + increment);

            let mut unsafe_state = safe;
            unsafe_state.monsters_mut()[0]
                .powers
                .set(power, SlotWire::Int, unsafe_old);
            let document = HotBoundary::to_canonical(&unsafe_state, &catalog);
            assert!(
                admit(&document, &unsafe_state, &catalog)
                    .unwrap_err()
                    .contains(admission::MissingCapability::ArgumentShape(
                        "potion held-target closure"
                    ))
            );
        }
    }

    #[test]
    fn blessing_and_soldiers_stew_rewrite_frozen_physical_uids_once() {
        let mut document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
        document.player.insert(
            "potion_slots".to_owned(),
            serde_json::json!(["BLESSING_OF_THE_FORGE"]),
        );
        document.player.insert(
            "potions".to_owned(),
            serde_json::json!(["BLESSING_OF_THE_FORGE"]),
        );
        document.player.insert(
            "fully_unlocked_potion_pool".to_owned(),
            serde_json::json!(true),
        );
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        admit(&document, &state, &catalog).unwrap();
        let hand_before = state.piles.get(PileId::Hand).as_slice().to_vec();
        let blessing = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        for (before, after) in hand_before
            .iter()
            .zip(blessing.piles.get(PileId::Hand).as_slice())
        {
            assert_eq!(after.uid, before.uid);
            let before_identity = catalog.spec(before.atom).unwrap().identity;
            let after_identity = catalog.spec(after.atom).unwrap().identity;
            assert_eq!(after_identity.id, before_identity.id);
            assert_eq!(after_identity.upgrade, before_identity.upgrade + 1);
        }

        let mut legacy_blessing = state.clone();
        legacy_blessing.next_card_uid = 40;
        for pile in PileId::ALL {
            for card in legacy_blessing.piles.get_mut(pile).make_mut() {
                card.uid = LEGACY_CARD_UID;
                card.flags |= CARD_FLAG_LEGACY;
            }
        }
        let legacy_blessing = apply_action(
            &legacy_blessing,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert_eq!(legacy_blessing.next_card_uid, 50);
        assert!(PileId::ALL.into_iter().all(|pile| {
            legacy_blessing
                .piles
                .get(pile)
                .as_slice()
                .iter()
                .all(|card| card.flags & CARD_FLAG_LEGACY == 0)
        }));
        assert!(
            !legacy_blessing.exact_piles,
            "identity normalization alone does not force exact pile order"
        );

        let (stew_state, stew_catalog) = fixture();
        assert!(!stew_state.exact_piles);
        let strike_uids = PileId::ALL
            .into_iter()
            .flat_map(|pile| stew_state.piles.get(pile).as_slice())
            .filter(|card| stew_catalog.spec(card.atom).unwrap().strike_tag)
            .map(|card| card.uid)
            .collect::<Vec<_>>();
        let stew = drink_fixture_potion(
            stew_state,
            &stew_catalog,
            PotionId::SoldiersStew,
            None,
            false,
        )
        .state;
        assert!(stew.exact_piles);
        assert!(!strike_uids.is_empty());
        for uid in strike_uids {
            assert_eq!(stew.card_states.get(uid).base_replay_count(), Some(1));
            assert!(PileId::ALL.into_iter().any(|pile| {
                stew.piles.get(pile).as_slice().iter().any(|card| {
                    card.uid == uid && card.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE != 0
                })
            }));
        }
    }

    #[test]
    fn soldiers_stew_allocates_legacy_uids_and_promotes_only_after_a_strike_grant() {
        let (mut state, catalog) = fixture();
        for pile in PileId::ALL {
            state.piles.get_mut(pile).make_mut().clear();
        }
        let strike = catalog
            .atom(&CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let defend = catalog
            .atom(&CardIdentity {
                id: CardId::DefendIronclad,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: LEGACY_CARD_UID,
                atom: strike,
                flags: CARD_FLAG_LEGACY,
            },
            HotCard {
                uid: LEGACY_CARD_UID,
                atom: defend,
                flags: CARD_FLAG_LEGACY,
            },
        ]);
        state.next_card_uid = 40;
        state.exact_piles = false;

        let successor =
            drink_fixture_potion(state.clone(), &catalog, PotionId::SoldiersStew, None, false)
                .state;
        let hand = successor.piles.get(PileId::Hand).as_slice();
        assert_eq!(
            hand.iter().map(|card| card.uid).collect::<Vec<_>>(),
            [40, 41]
        );
        assert_eq!(successor.next_card_uid, 42);
        assert!(successor.exact_piles);
        assert!(hand.iter().all(|card| card.flags & CARD_FLAG_LEGACY == 0));
        assert_eq!(successor.card_states.get(40).base_replay_count(), Some(1));
        assert_eq!(successor.card_states.get(41).base_replay_count(), None);
        let projected = HotBoundary::try_to_canonical(&successor, &catalog).unwrap();
        assert_eq!(
            projected.player.get("exact_piles"),
            Some(&serde_json::json!(true))
        );
        assert_eq!(
            projected.piles["hand"]
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [Some(40), Some(41)]
        );

        state.piles.get_mut(PileId::Hand).make_mut().remove(0);
        let no_strike =
            drink_fixture_potion(state, &catalog, PotionId::SoldiersStew, None, false).state;
        assert!(!no_strike.exact_piles);
        assert_eq!(no_strike.next_card_uid, 40);
        assert_eq!(
            no_strike.piles.get(PileId::Hand).as_slice()[0].flags & CARD_FLAG_LEGACY,
            CARD_FLAG_LEGACY
        );
    }

    #[test]
    fn two_duplicators_duplicate_two_later_card_actions_once_each() {
        let (mut state, catalog) = fixture();
        for monster in state.monsters_mut() {
            monster.hp = 100;
            monster.max_hp = 100;
        }
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::Duplicator), Some(PotionId::Duplicator)],
            false,
            false,
            false,
            false,
            true,
        ));
        let first = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let charged = apply_action(
            &first,
            &catalog,
            &Action::UsePotion {
                slot: 1,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert_eq!(charged.fanouts.duplication(), 2);

        let once = apply_action(
            &charged,
            &catalog,
            &Action::Play {
                uid: 0,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(once.monsters[0].hp, 88);
        assert_eq!(once.fanouts.duplication(), 1);
        let twice = apply_action(
            &once,
            &catalog,
            &Action::Play {
                uid: 1,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert_eq!(twice.monsters[0].hp, 76);
        assert_eq!(twice.fanouts.duplication(), 0);
    }

    #[test]
    fn two_gigantification_stacks_bind_two_attack_commands_not_two_hits() {
        let (mut state, catalog) = fixture();
        for monster in state.monsters_mut() {
            monster.hp = 100;
            monster.max_hp = 100;
        }
        assert!(state.fanouts.set_potion_belt(
            vec![
                Some(PotionId::GigantificationPotion),
                Some(PotionId::GigantificationPotion),
            ],
            false,
            false,
            false,
            false,
            true,
        ));
        for slot in [0, 1] {
            state = apply_action(&state, &catalog, &Action::UsePotion { slot, target: None })
                .unwrap()
                .state;
        }
        assert_eq!(state.fanouts.gigantification(), 2);
        for (uid, expected_hp, remaining) in [(0, 82, 1), (1, 64, 0)] {
            state = apply_action(
                &state,
                &catalog,
                &Action::Play {
                    uid,
                    target: Some(0),
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap()
            .state;
            assert_eq!(state.monsters[0].hp, expected_hp);
            assert_eq!(state.fanouts.gigantification(), remaining);
            assert!(!state.fanouts.gigantification_bound());
        }
    }

    #[test]
    fn belt_buckle_procurement_inserts_before_unlatching_and_failures_are_inert() {
        let (mut latched, _) = fixture();
        assert!(
            latched
                .fanouts
                .set_potion_belt(vec![None], false, true, true, false, true,)
        );
        latched.powers.set(PowerId::Dexterity, SlotWire::Int, 2);
        let mut events = Vec::new();
        assert_eq!(
            potions::procure_potion(&mut latched, PotionId::FirePotion, &mut events),
            Ok(true)
        );
        assert_eq!(latched.fanouts.potion_slots(), [Some(PotionId::FirePotion)]);
        assert!(!latched.fanouts.potion_belt_buckle_applied());
        assert_eq!(latched.powers.value(PowerId::Dexterity), 0);

        let (mut sozu, _) = fixture();
        assert!(
            sozu.fanouts
                .set_potion_belt(vec![None], true, true, true, false, true,)
        );
        sozu.powers.set(PowerId::Dexterity, SlotWire::Int, 2);
        let checkpoint = sozu.clone();
        let mut failed_events = vec![Event::TurnEnded { turn: 44 }];
        assert_eq!(
            potions::procure_potion(&mut sozu, PotionId::FirePotion, &mut failed_events),
            Ok(false)
        );
        assert_eq!(sozu, checkpoint);
        assert_eq!(failed_events, [Event::TurnEnded { turn: 44 }]);

        let (mut full, _) = fixture();
        assert!(full.fanouts.set_potion_belt(
            vec![Some(PotionId::WeakPotion)],
            false,
            true,
            false,
            false,
            true,
        ));
        let full_checkpoint = full.clone();
        assert_eq!(
            potions::procure_potion(&mut full, PotionId::FirePotion, &mut Vec::new()),
            Ok(false)
        );
        assert_eq!(full, full_checkpoint);
    }

    #[test]
    fn potion_created_duration_powers_tick_at_their_native_turn_seams() {
        let (mut state, catalog) = fixture();
        assert!(state.fanouts.set_potion_belt(
            vec![
                Some(PotionId::MazalethsGift),
                Some(PotionId::RadiantTincture),
                Some(PotionId::Duplicator),
            ],
            false,
            false,
            false,
            false,
            true,
        ));
        for slot in 0..3 {
            state = apply_action(&state, &catalog, &Action::UsePotion { slot, target: None })
                .unwrap()
                .state;
        }
        assert_eq!(state.fanouts.player_ritual(), 1);
        assert_eq!(state.fanouts.radiance(), 3);
        assert_eq!(state.fanouts.duplication(), 1);

        let next_turn = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert_eq!(next_turn.powers.value(PowerId::Strength), 1);
        assert_eq!(next_turn.fanouts.player_ritual(), 1);
        assert_eq!(next_turn.energy, 4);
        assert_eq!(next_turn.fanouts.radiance(), 2);
        assert_eq!(next_turn.fanouts.duplication(), 0);
    }

    #[test]
    fn duplicate_fire_slots_remove_only_the_selected_physical_slot() {
        let (mut state, catalog) = fixture();
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::FirePotion), Some(PotionId::FirePotion)],
            false,
            false,
            false,
            false,
            true,
        ));
        let successor = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 1,
                target: Some(0),
            },
        )
        .unwrap()
        .state;
        assert_eq!(
            successor.fanouts.potion_slots(),
            [Some(PotionId::FirePotion), None]
        );
        assert_eq!(successor.monsters[0].hp, 6);
        assert_eq!(successor.monsters[1].hp, 25);
    }

    #[test]
    fn potion_body_and_finish_overflows_roll_back_the_whole_public_action() {
        let (mut duplication, catalog) = fixture();
        assert!(duplication.fanouts.set_potion_belt(
            vec![Some(PotionId::Duplicator)],
            false,
            false,
            false,
            false,
            true,
        ));
        assert!(duplication.fanouts.set_duplication(i32::MAX));

        let (mut fortifier, _) = fixture();
        fortifier.block = i32::MAX;
        assert!(fortifier.fanouts.set_potion_belt(
            vec![Some(PotionId::Fortifier)],
            false,
            false,
            false,
            false,
            true,
        ));

        let (mut buckle, _) = fixture();
        buckle
            .powers
            .set(PowerId::Dexterity, SlotWire::Int, i32::MAX);
        assert!(buckle.fanouts.set_potion_belt(
            vec![Some(PotionId::Duplicator)],
            false,
            true,
            false,
            false,
            true,
        ));

        let (mut stew, _) = fixture();
        let mut physical = stew.card_states.get(0);
        assert!(physical.set_base_replay_count(Some(i32::MAX)).is_some());
        stew.card_states.set(0, physical);
        stew.piles.get_mut(PileId::Hand).make_mut()[0].flags |= CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        assert!(stew.fanouts.set_potion_belt(
            vec![Some(PotionId::SoldiersStew)],
            false,
            false,
            false,
            false,
            true,
        ));

        for (state, action) in [
            (
                duplication,
                Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            ),
            (
                fortifier,
                Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            ),
            (
                buckle,
                Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            ),
            (
                stew,
                Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            ),
        ] {
            let checkpoint = state.clone();
            let mut events = vec![Event::TurnEnded { turn: 91 }];
            assert!(apply_action_into(&state, &catalog, &action, &mut events).is_err());
            assert_eq!(state, checkpoint);
            assert!(events.is_empty());
        }
    }

    #[test]
    fn belt_buckle_reserves_last_potion_dexterity_before_publication() {
        let (mut state, catalog) = fixture();
        state
            .powers
            .set(PowerId::Dexterity, SlotWire::Int, i32::MAX);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::EnergyPotion)],
            false,
            true,
            false,
            false,
            true,
        ));
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert!(admit(&document, &state, &catalog).unwrap_err().contains(
            MissingCapability::ArgumentShape("Belt Buckle last-potion Dexterity",)
        ));
        assert!(
            !legal_actions(&state, &catalog).contains(&Action::UsePotion {
                slot: 0,
                target: None,
            })
        );
        let before = state.clone();
        let mut events = vec![Event::TurnEnded { turn: 99 }];
        assert_eq!(
            apply_action_into(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow(
                "Belt Buckle last-potion Dexterity",
            ))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn soldiers_stew_duplicate_physical_uid_refuses_before_any_replay_write() {
        let (mut state, catalog) = fixture();
        let duplicate = state.piles.get(PileId::Hand).as_slice()[0];
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .push(duplicate);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::SoldiersStew)],
            false,
            false,
            false,
            false,
            true,
        ));
        let before = state.clone();
        let mut events = vec![Event::TurnEnded { turn: 92 }];
        assert_eq!(
            apply_action_into(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs(
                "Soldier's Stew duplicate physical uid"
            ))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn vulnerable_vicious_reshuffle_fully_drains_before_belt_buckle_finish() {
        let (mut state, catalog) = fixture();
        state.powers.set(PowerId::Vicious, SlotWire::Int, 2);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::Vicious])
        );
        let reshuffle_cards = state.piles.get(PileId::Draw).as_slice().to_vec();
        state.piles.get_mut(PileId::Draw).make_mut().clear();
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend(reshuffle_cards);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::VulnerablePotion)],
            false,
            true,
            false,
            false,
            true,
        ));
        let hand_before = state.piles.get(PileId::Hand).len();
        let rng_before = state.rng.clone();
        let transition = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: Some(0),
            },
        )
        .unwrap();
        assert_eq!(transition.state.monsters[0].powers.value(PowerId::Vuln), 3);
        assert_eq!(
            transition.state.piles.get(PileId::Hand).len(),
            hand_before + 2
        );
        assert_ne!(transition.state.rng, rng_before);
        assert_eq!(transition.state.powers.value(PowerId::Dexterity), 2);
        assert!(transition.state.fanouts.potion_belt_buckle_applied());
        assert!(transition.state.frames.is_empty());
        assert!(transition.state.pending.is_none());
        assert_eq!(
            transition
                .events
                .iter()
                .filter(|event| matches!(event, Event::Reshuffled { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn potion_legal_actions_preserve_slot_target_and_native_partition_order() {
        let (mut state, catalog) = fixture();
        assert!(state.fanouts.set_potion_belt(
            vec![
                Some(PotionId::FirePotion),
                None,
                Some(PotionId::Duplicator),
                Some(PotionId::FirePotion),
                Some(PotionId::FairyInABottle),
            ],
            false,
            false,
            false,
            false,
            true,
        ));
        let actions = legal_actions(&state, &catalog);
        assert_eq!(
            &actions[3..],
            &[
                Action::UsePotion {
                    slot: 0,
                    target: Some(1),
                },
                Action::UsePotion {
                    slot: 0,
                    target: Some(0),
                },
                Action::UsePotion {
                    slot: 2,
                    target: None,
                },
                Action::UsePotion {
                    slot: 3,
                    target: Some(1),
                },
                Action::UsePotion {
                    slot: 3,
                    target: Some(0),
                },
                Action::EndTurn,
            ],
            "cards precede ascending potion slots and EndTurn; duplicate potion slots stay distinct",
        );
        assert_eq!(std::mem::size_of::<Action>(), 12);
    }

    #[test]
    fn potion_target_partition_is_a_closed_current_catalog_census() {
        let expected = [
            PotionId::BeetleJuice,
            PotionId::FirePotion,
            PotionId::PoisonPotion,
            PotionId::PotionOfDoom,
            PotionId::PotionShapedRock,
            PotionId::PowderedDemise,
            PotionId::VulnerablePotion,
            PotionId::WeakPotion,
        ];
        let actual: Vec<_> = PotionId::ALL
            .into_iter()
            .filter(|potion| potion_targets_enemy(*potion))
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), 8);
        assert_eq!(PotionId::ALL.len(), PotionId::COUNT);
    }

    #[test]
    fn malformed_potion_slot_and_target_refuse_atomically() {
        let (mut state, catalog) = fixture();
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::FirePotion), None, Some(PotionId::Duplicator)],
            false,
            false,
            false,
            false,
            true,
        ));
        let checkpoint = state.clone();
        for (action, expected) in [
            (
                Action::UsePotion {
                    slot: 1,
                    target: Some(0),
                },
                EngineRefusal::BadPotionSlot(1),
            ),
            (
                Action::UsePotion {
                    slot: 3,
                    target: None,
                },
                EngineRefusal::BadPotionSlot(3),
            ),
            (
                Action::UsePotion {
                    slot: 0,
                    target: None,
                },
                EngineRefusal::PotionTargetMismatch { required: true },
            ),
            (
                Action::UsePotion {
                    slot: 2,
                    target: Some(0),
                },
                EngineRefusal::PotionTargetMismatch { required: false },
            ),
            (
                Action::UsePotion {
                    slot: 0,
                    target: Some(9),
                },
                EngineRefusal::BadTarget(9),
            ),
        ] {
            let mut events = vec![Event::TurnEnded { turn: 0 }];
            assert_eq!(
                apply_action_into(&state, &catalog, &action, &mut events),
                Err(expected)
            );
            assert!(events.is_empty());
            assert_eq!(state, checkpoint);
        }
    }

    #[test]
    fn forged_teammate_callback_owner_refuses_before_any_public_action_mutation() {
        let (mut state, catalog) = fixture();
        assert_eq!(state.multiplayer_ally_key, 0);
        assert!(state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            ..MultiplayerAllyState::default()
        }));
        assert!(state.fanouts.set_teammate_power_pending(Some((1, 77))));
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 999 }];

        assert_eq!(
            apply_action_into(&state, &catalog, &Action::EndTurn, &mut events),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn void_form_orphan_power_is_authenticated_at_all_four_public_action_routes() {
        let (mut state, catalog) = fixture();
        state.powers.set(PowerId::VoidForm, SlotWire::Int, 1);
        crate::engine::play::hydrate_after_card_played_power_order_for_test(&mut state);
        let before = state.clone();
        for action in [
            Action::Play {
                uid: 0,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
            Action::EndTurn,
            Action::UsePotion {
                slot: 0,
                target: None,
            },
        ] {
            let mut events = vec![Event::TurnBegan { turn: 999 }];
            assert_eq!(
                apply_action_into(&state, &catalog, &action, &mut events),
                Err(EngineRefusal::MalformedArgs("void form private state")),
                "{action:?} must not route around the private quotient"
            );
            assert_eq!(state, before);
            assert!(events.is_empty());
        }
        assert_eq!(
            apply_action(
                &state,
                &catalog,
                &Action::Select {
                    answer: SelectionAnswer::CardUid(0),
                },
            ),
            Err(EngineRefusal::MalformedArgs("void form private state")),
            "the shared private-state preflight precedes every action route"
        );

        let (mut suspended, suspended_catalog) = void_form_deferred_selection_fixture(false);
        suspended.fanouts.set_void_form_end_turn_requested(false);
        suspended.fanouts.set_void_form_hooks_registered(false);
        assert!(suspended.fanouts.set_void_form_cards_played_this_turn(0));
        let suspended_before = suspended.clone();
        let mut events = vec![Event::TurnBegan { turn: 1000 }];
        assert_eq!(
            apply_action_into(
                &suspended,
                &suspended_catalog,
                &Action::Select {
                    answer: SelectionAnswer::CardUid(42),
                },
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("void form private state"))
        );
        assert_eq!(suspended, suspended_before);
        assert!(events.is_empty());
    }

    fn void_form_deferred_selection_fixture(late_draw_refusal: bool) -> (HotState, Catalog) {
        let pact = plain_identity(CardId::BurningPact, 0);
        let strike = plain_identity(CardId::StrikeIronclad, 0);
        let defend = plain_identity(CardId::DefendIronclad, 0);
        let mut builder = CatalogBuilder::new();
        let pact_atom = builder.intern(pact).unwrap();
        let strike_atom = builder.intern(strike).unwrap();
        let defend_atom = builder.intern(defend).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();

        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.next_card_uid = 45;
        state.turn = 2;
        state.exact_piles = true;
        state.powers.set(PowerId::Burst, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.powers.set(PowerId::VoidForm, SlotWire::Int, 2);
        state.fanouts.set_void_form_hooks_registered(true);
        crate::engine::play::hydrate_after_card_played_power_order_for_test(&mut state);
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 40,
                atom: pact_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 41,
                atom: strike_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 42,
                atom: strike_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            // A second remaining candidate keeps the Burst replay's
            // selection prompted: one card would auto-resolve (#3250).
            HotCard {
                uid: 44,
                atom: strike_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        if late_draw_refusal {
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 43,
                atom: defend_atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        }
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 30);
        monster.max_hp = 30;
        state.monsters_mut().push(monster);

        // The first Burning Pact body consumes uid 41 and Burst suspends its
        // replay before the second selection.  This is the exact continuation
        // shape in which a nested Void Form request must survive the public
        // action boundary until the outermost play completes.
        play::play_card(&mut state, &catalog, 40, None, Some(41), &mut Vec::new()).unwrap();
        assert!(state.pending.is_some());
        assert!(!state.frames.is_empty());
        assert_eq!(state.fanouts.void_form_cards_played_this_turn(), 0);
        if late_draw_refusal {
            state.cards_drawn_combat = i32::MAX;
        }
        assert!(
            state
                .fanouts
                .set_void_form_cards_played_this_turn(999_999_999)
        );
        state.fanouts.set_void_form_end_turn_requested(true);
        (state, catalog)
    }

    #[test]
    fn void_form_deferred_end_turn_waits_for_replay_resume_then_services_once() {
        let (state, catalog) = void_form_deferred_selection_fixture(false);
        let start_turn = state.turn;
        let transition = apply_action(
            &state,
            &catalog,
            &Action::Select {
                answer: SelectionAnswer::CardUid(42),
            },
        )
        .unwrap();

        assert!(transition.state.pending.is_none());
        assert!(transition.state.frames.is_empty());
        assert!(!transition.state.fanouts.void_form_end_turn_requested());
        assert_eq!(transition.state.turn, start_turn + 1);
        assert!(transition.state.player_side_active);
        assert_eq!(
            transition.state.fanouts.void_form_cards_played_this_turn(),
            0,
            "the replay-loop completion increments before the deferred turn, whose owner start resets"
        );
        assert_eq!(
            transition
                .events
                .iter()
                .filter(|event| matches!(event, Event::TurnEnded { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn void_form_deferred_end_turn_exposes_the_pre_end_turn_native_checkpoint() {
        // #3242: native's completed-play checkpoint precedes the EndTurn the
        // play requested; a recording caller sees that state, and recording
        // never changes the transition.
        let (state, catalog) = void_form_deferred_selection_fixture(false);
        let start_turn = state.turn;
        let action = Action::Select {
            answer: SelectionAnswer::CardUid(42),
        };
        let plain = apply_action(&state, &catalog, &action).unwrap();
        let (recorded_transition, recorded) =
            native_checkpoint::record(|| apply_action(&state, &catalog, &action));
        let recorded_transition = recorded_transition.unwrap();
        assert_eq!(recorded_transition.state, plain.state);
        assert_eq!(recorded_transition.events, plain.events);

        assert_eq!(recorded.len(), 1);
        let (kind, checkpoint) = &recorded[0];
        assert_eq!(
            *kind,
            native_checkpoint::NativeCheckpointKind::VoidFormEndTurnRequest
        );
        assert_eq!(kind.as_str(), "void_form_end_turn_request");
        assert_eq!(checkpoint.turn, start_turn);
        assert!(checkpoint.fanouts.void_form_end_turn_requested());
        assert!(checkpoint.pending.is_none());
        assert!(checkpoint.frames.is_empty());
        assert_eq!(plain.state.turn, start_turn + 1);
    }

    #[test]
    fn void_form_deferred_end_turn_late_failure_rolls_back_the_originating_resume() {
        let (state, catalog) = void_form_deferred_selection_fixture(true);
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 777 }];
        assert_eq!(
            apply_action_into(
                &state,
                &catalog,
                &Action::Select {
                    answer: SelectionAnswer::CardUid(42),
                },
                &mut events,
            ),
            Err(EngineRefusal::CounterOverflow("cards_drawn_combat"))
        );
        assert_eq!(
            state, before,
            "the complete resume plus deferred turn is atomic"
        );
        assert!(events.is_empty(), "no play or turn prefix escapes refusal");
    }

    #[test]
    fn void_form_low_counter_request_forgery_refuses_before_resume_mutation() {
        let (mut state, catalog) = void_form_deferred_selection_fixture(false);
        assert!(state.fanouts.set_void_form_cards_played_this_turn(7));
        let before = state.clone();
        let mut events = vec![Event::TurnBegan { turn: 778 }];
        assert_eq!(
            apply_action_into(
                &state,
                &catalog,
                &Action::Select {
                    answer: SelectionAnswer::CardUid(42),
                },
                &mut events,
            ),
            Err(EngineRefusal::MalformedArgs("void form private state"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn void_form_public_end_turn_action_refuses_an_unresolved_request() {
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.powers.set(PowerId::VoidForm, SlotWire::Int, 2);
        state.fanouts.set_void_form_hooks_registered(true);
        assert!(
            state
                .fanouts
                .set_void_form_cards_played_this_turn(999_999_999)
        );
        state.fanouts.set_void_form_end_turn_requested(true);
        let before = state.clone();
        assert_eq!(
            apply_action(&state, &CatalogBuilder::new().build(), &Action::EndTurn),
            Err(EngineRefusal::MalformedArgs(
                "unresolved void form end-turn request"
            ))
        );
        assert_eq!(state, before);
    }

    #[test]
    fn an_action_naming_an_absent_card_refuses() {
        let (state, catalog) = fixture();
        assert_eq!(
            apply_action(
                &state,
                &catalog,
                &Action::Play {
                    uid: 99,
                    target: None,
                    selection: SelectionRef::NONE,
                }
            )
            .unwrap_err(),
            EngineRefusal::CardNotInHand(99)
        );
    }

    #[test]
    fn a_targeted_card_refuses_without_a_target_and_vice_versa() {
        let (state, catalog) = fixture();
        assert_eq!(
            apply_action(
                &state,
                &catalog,
                &Action::Play {
                    uid: 0,
                    target: None,
                    selection: SelectionRef::NONE,
                }
            )
            .unwrap_err(),
            EngineRefusal::TargetMismatch { required: true }
        );
        assert_eq!(
            apply_action(
                &state,
                &catalog,
                &Action::Play {
                    uid: 2,
                    target: Some(0),
                    selection: SelectionRef::NONE,
                }
            )
            .unwrap_err(),
            EngineRefusal::TargetMismatch { required: false }
        );
    }

    #[test]
    fn a_target_outside_the_roster_refuses() {
        let (state, catalog) = fixture();
        assert_eq!(
            apply_action(
                &state,
                &catalog,
                &Action::Play {
                    uid: 0,
                    target: Some(7),
                    selection: SelectionRef::NONE,
                }
            )
            .unwrap_err(),
            EngineRefusal::BadTarget(7)
        );
    }

    #[test]
    fn events_buffer_into_a_reused_vector() {
        let (state, catalog) = fixture();
        let mut events = Vec::with_capacity(64);
        let next = apply_action_into(
            &state,
            &catalog,
            &Action::Play {
                uid: 0,
                target: Some(1),
                selection: SelectionRef::NONE,
            },
            &mut events,
        )
        .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            Event::MonsterDamaged {
                uid: 1,
                unblocked: 6,
                ..
            }
        )));
        // The buffer is cleared, not appended to, on the next call.
        let previous = events.len();
        assert!(previous > 0);
        let _ = apply_action_into(&next, &catalog, &Action::EndTurn, &mut events).unwrap();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::TurnEnded { .. }))
        );
    }

    /// PORT_PLAN §6/§7's deterministic PR-time perf pin: replay a fixed
    /// seeded workload and assert an exact allocations-per-transition
    /// ceiling.
    ///
    /// Allocation counts are deterministic, so this is an equality-style pin
    /// with zero benchmark noise. It catches exactly the structural creep the
    /// port is built to avoid — an owned string in an event, a clone in a hit
    /// loop, a per-node map — on the pull request that introduces it, rather
    /// than as a throughput regression weeks later.
    ///
    /// What is measured is [`apply_action_into`] only, with a reused event
    /// buffer. Canonical projection and action enumeration are separate
    /// contracts; [`legal_actions_into`] has its own warmed zero-allocation
    /// test below.
    ///
    /// The floor is not zero and should not be: a transition clones the state
    /// and then copy-on-writes the piles it touches, which is the design
    /// (D3). The ceiling exists to keep that number from *growing*.
    #[cfg(feature = "allocation-counting")]
    #[test]
    fn allocations_per_transition_stay_under_the_ceiling() {
        use crate::allocation::thread_snapshot;

        /// Allocations per transition, averaged over the replayed line.
        /// **Measured: 10** on aarch64-apple-darwin (1,256 allocations across
        /// 120 transitions). The ceiling carries two allocations of headroom
        /// and no more; raising it is an explicit reviewed diff with a stated
        /// reason, exactly like a throughput floor (PORT_PLAN §7).
        const CEILING: u64 = 12;
        /// How many times the line is replayed. More than one, so a one-off
        /// lazy initialisation cannot hide inside the average.
        const REPLAYS: u64 = 8;

        let pins: serde_json::Value =
            serde_json::from_str(include_str!("../../fixtures/slice_line_v1.json")).unwrap();
        let script: Vec<Action> = pins["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|step| match step["action"]["kind"].as_str().unwrap() {
                "end" => Action::EndTurn,
                _ => Action::Play {
                    uid: step["action"]["uid"].as_u64().unwrap() as u32,
                    target: step["action"]
                        .get("target")
                        .and_then(serde_json::Value::as_u64)
                        .map(|index| index as u8),
                    selection: SelectionRef::NONE,
                },
            })
            .collect();
        let (entry, catalog) = fixture();
        let mut events: Vec<Event> = Vec::with_capacity(256);

        // Warm the buffer and every lazy path before the window opens.
        {
            let mut state = entry.clone();
            for action in &script {
                state = apply_action_into(&state, &catalog, action, &mut events).unwrap();
            }
        }

        let transitions = REPLAYS * script.len() as u64;
        let (before, _) = thread_snapshot();
        for _ in 0..REPLAYS {
            let mut state = entry.clone();
            for action in &script {
                state = apply_action_into(&state, &catalog, action, &mut events).unwrap();
            }
            std::hint::black_box(&state);
        }
        let (after, _) = thread_snapshot();
        let per_transition = (after - before) / transitions;
        assert!(
            per_transition <= CEILING,
            "allocations per transition rose to {per_transition} (ceiling {CEILING}); \
             a new heap allocation entered the hot path"
        );
    }

    #[cfg(feature = "allocation-counting")]
    #[test]
    fn warmed_legal_action_buffer_enumerates_the_fixture_without_allocating() {
        use crate::allocation::thread_snapshot;

        let (state, catalog) = fixture();
        let mut buffer = LegalActionBuffer::new();
        let expected = legal_actions(&state, &catalog);
        assert_eq!(legal_actions_into(&state, &catalog, &mut buffer), expected);

        let (before, _) = thread_snapshot();
        for _ in 0..128 {
            std::hint::black_box(legal_actions_into(&state, &catalog, &mut buffer));
        }
        let (after, _) = thread_snapshot();
        assert_eq!(after, before, "a warmed enumeration allocated");
    }

    #[cfg(feature = "allocation-counting")]
    #[test]
    fn warmed_targeted_potion_enumeration_validates_misery_without_allocating() {
        use crate::allocation::thread_snapshot;

        let (mut state, catalog) = fixture();
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::BeetleJuice)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Weak, SlotWire::Int, 2);
        state.monsters_mut()[0]
            .misery_debuff_order
            .push(MiseryToken::Weak);
        let expected = Action::UsePotion {
            slot: 0,
            target: Some(0),
        };
        let mut buffer = LegalActionBuffer::new();
        assert!(legal_actions_into(&state, &catalog, &mut buffer).contains(&expected));

        let (before, _) = thread_snapshot();
        for _ in 0..128 {
            std::hint::black_box(legal_actions_into(&state, &catalog, &mut buffer));
        }
        let (after, _) = thread_snapshot();
        assert_eq!(
            after, before,
            "a warmed targeted-potion Misery preflight allocated"
        );
    }

    #[test]
    fn attack_potion_parks_an_authenticated_generation_choice_and_skip_finishes_once() {
        let mut builder = CatalogBuilder::new();
        for id in crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109 {
            builder.intern(plain_identity(id, 0)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.reward_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.entropy_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.fully_unlocked_card_pool_epochs = true;
        assert!(state.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            alive: false,
            ..MultiplayerAllyState::default()
        }));
        state.rng.set(
            crate::hot::RngStream::Generation,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::AttackPotion)],
            false,
            false,
            false,
            false,
            true,
        ));

        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let finish = parked
            .pending
            .as_deref()
            .and_then(|pending| pending.generation_potion_record(&parked.frames))
            .expect("generation choice owns the pending selector");
        assert_eq!(finish.name, PotionId::AttackPotion);
        assert_eq!(finish.generation_options().len(), 3);
        let expected_choice = catalog
            .spec(finish.generation_options().next().unwrap())
            .unwrap()
            .identity;
        assert_eq!(legal_actions(&parked, &catalog).len(), 4);
        assert_eq!(parked.fanouts.potion_slots(), [None]);
        assert_eq!(
            parked.rng.get(crate::hot::RngStream::Generation).counter,
            32
        );

        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        assert_eq!(
            document.player["pending"].as_array().unwrap()[0],
            serde_json::json!("generation_potion_select")
        );
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &rebuilt, &rebuilt_catalog), Ok(()));
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
            document
        );

        let chosen = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(chosen.pending.is_none());
        assert!(chosen.frames.is_empty());
        assert_eq!(chosen.piles.get(PileId::Hand).len(), 1);
        let generated = chosen.piles.get(PileId::Hand).as_slice()[0];
        assert_eq!(generated.uid, 0);
        assert_eq!(
            rebuilt_catalog.spec(generated.atom).unwrap().identity,
            expected_choice
        );
        let generated_state = chosen.card_states.get(generated.uid);
        let energy_rows = generated_state.local_cost_modifiers.as_slice();
        if rebuilt_catalog.spec(generated.atom).unwrap().cost >= 0 {
            assert_eq!(energy_rows.len(), 1);
            assert_eq!(energy_rows[0].kind, crate::hot::LocalCostModifierKind::Set);
            assert_eq!(energy_rows[0].amount, 0);
            assert_eq!(
                energy_rows[0].expiration,
                crate::hot::LocalCostExpiration::ThisTurnOrPlayed
            );
        } else {
            assert!(energy_rows.is_empty());
        }
        assert_eq!(generated_state.free_star_cost_this_turn_or_played_rows, 1);
        assert_eq!(chosen.history.owner_generated_cards_combat, 1);

        let completed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(3),
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none());
        assert!(completed.frames.is_empty());
        assert_eq!(completed.fanouts.potion_slots(), [None]);
        assert!(completed.piles.get(PileId::Hand).is_empty());
    }

    #[test]
    fn skill_and_power_potions_pin_options_nonzero_ordinals_and_full_hand_redirect() {
        for (potion, ordinal, draws, expected) in [
            (
                PotionId::SkillPotion,
                1_u32,
                26_u64,
                [CardId::FlameBarrier, CardId::BurningPact, CardId::Stoke],
            ),
            (
                PotionId::PowerPotion,
                2_u32,
                17_u64,
                [CardId::CrimsonMantle, CardId::Stampede, CardId::FeelNoPain],
            ),
        ] {
            let (mut state, catalog) = entropic_fixture(vec![Some(potion)], false, [1, 2, 3, 4], 0);
            state.next_card_uid = 11;
            let filler = catalog
                .atom(&plain_identity(
                    crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109[0],
                    0,
                ))
                .unwrap();
            state
                .piles
                .get_mut(PileId::Hand)
                .make_mut()
                .extend((1..=10).map(|uid| HotCard {
                    uid,
                    atom: filler,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                }));
            let parked = apply_action(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            )
            .unwrap()
            .state;
            let finish = parked
                .pending
                .as_deref()
                .and_then(|pending| pending.generation_potion_record(&parked.frames))
                .unwrap();
            let ids = finish
                .generation_options()
                .map(|atom| catalog.spec(atom).unwrap().identity.id)
                .collect::<Vec<_>>();
            assert_eq!(ids, expected, "{potion:?} seeded option order");
            assert_eq!(
                parked.rng.get(crate::hot::RngStream::Generation).counter,
                draws
            );
            assert_eq!(legal_actions(&parked, &catalog).len(), 4);

            let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
            let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
            let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
            assert_eq!(
                HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
                document
            );

            let mut rootless = parked.clone();
            let rootless_finish = finish.to_owned();
            rootless.frames = crate::hot::Frames::new();
            let rootless_record = rootless
                .frames
                .push_potion_finish(&rootless_finish)
                .unwrap();
            rootless.pending = Some(std::sync::Arc::new(crate::hot::PendingSelection {
                frame_uid: crate::hot::GENERATION_POTION_PENDING_UID,
                frame_record: rootless_record,
            }));
            assert!(
                rootless
                    .pending
                    .as_deref()
                    .and_then(|pending| pending.generation_potion_record(&rootless.frames))
                    .is_none()
            );
            assert!(legal_actions(&rootless, &catalog).is_empty());
            let rootless_before = rootless.clone();
            assert_eq!(
                apply_action(
                    &rootless,
                    &catalog,
                    &Action::Select {
                        answer: SelectionAnswer::OptionIndex(ordinal),
                    },
                )
                .unwrap_err(),
                EngineRefusal::ContinuationNotModeled
            );
            assert_eq!(rootless, rootless_before);
        }
    }

    #[test]
    fn cold_generation_potion_catalogs_close_stoke_and_infernal_descendants() {
        for potion in [PotionId::SkillPotion, PotionId::OrobicAcid] {
            let (state, source_catalog) =
                entropic_fixture(vec![Some(potion), None], false, [1, 2, 3, 4], 0);
            let document = HotBoundary::try_to_canonical(&state, &source_catalog).unwrap();
            assert!(document.piles.values().all(Vec::is_empty));

            let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
            for id in crate::content_tables::STOKE_CARD_POOL_V109
                .into_iter()
                .chain(crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109)
            {
                assert!(
                    catalog
                        .atom(&crate::catalog::CardIdentity {
                            id,
                            upgrade: 0,
                            enchantment: None,
                        })
                        .is_some(),
                    "{potion:?} omitted recursive descendant {id:?}"
                );
            }
            let rebuilt = HotBoundary::from_canonical(&document, &catalog).unwrap();
            assert_eq!(
                admit(&document, &rebuilt, &catalog),
                Ok(()),
                "{potion:?} closes every generated ordinary side-start writer"
            );
        }
    }

    #[test]
    fn generation_potions_refuse_a_catalog_for_a_different_owner() {
        let (mut exact, exact_catalog) =
            entropic_fixture(vec![Some(PotionId::SkillPotion)], false, [1, 2, 3, 4], 0);
        exact.entropy_card_pool = None;
        let exact_action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        assert!(legal_actions(&exact, &exact_catalog).contains(&exact_action));
        assert!(apply_action(&exact, &exact_catalog, &exact_action).is_ok());

        for owner in [
            crate::catalog::RewardPool::Silent,
            crate::catalog::RewardPool::Defect,
            crate::catalog::RewardPool::Regent,
        ] {
            let (mut state, catalog) =
                entropic_fixture(vec![Some(PotionId::SkillPotion)], false, [1, 2, 3, 4], 0);
            state.reward_card_pool = Some(owner);
            state.entropy_card_pool = Some(owner);
            let action = Action::UsePotion {
                slot: 0,
                target: None,
            };
            let before = state.clone();
            assert!(!legal_actions(&state, &catalog).contains(&action));
            assert_eq!(
                apply_action(&state, &catalog, &action).unwrap_err(),
                EngineRefusal::MalformedArgs("generation potion pool provenance")
            );
            assert_eq!(state, before);
            let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            assert!(admit(&document, &state, &catalog).is_err());
        }
    }

    /// #3515: `DistilledChaos/<OnUse>d__8` (`0x34cec0`) awaits
    /// `CardPileCmd.AutoPlayFromDrawPile` (IL_006e), which returns at
    /// `IsOverOrEnding` before it gathers (`<AutoPlayFromDrawPile>d__23`
    /// `0x3e3638` IL_0025-0031). While the combat is ending before the over
    /// latch the potion gathers and plays nothing; the Adaptable-vetoed
    /// control plays all three Defends.
    #[test]
    fn distilled_chaos_gathers_nothing_while_combat_is_ending_before_the_over_latch() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let catalog = builder.build();
        for vetoed in [false, true] {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = 4;
            state.exact_piles = true;
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::DistilledChaos)],
                false,
                false,
                false,
                false,
                true,
            ));
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend((1..=3).map(|uid| HotCard {
                    uid,
                    atom: defend,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                }));
            crate::engine::damage::push_ending_window_roster(&mut state, vetoed);
            let result = apply_action(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            )
            .unwrap();
            assert_eq!(
                result.state.piles.get(PileId::Draw).len(),
                if vetoed { 0 } else { 3 },
                "vetoed={vetoed}"
            );
            assert_eq!(
                result.state.history.card_plays_finished_combat,
                if vetoed { 3 } else { 0 }
            );
        }
    }

    #[test]
    fn distilled_chaos_gathers_three_before_playing_and_finishes_once() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.mark_persistent_action_replay_required();
        builder
            .intern_monster(crate::ids::MonsterKind::Toadpole)
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 4;
        state.exact_piles = true;
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((1..=3).map(|uid| HotCard {
                uid,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));

        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&document, &state, &catalog), Ok(()));
        let result = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap();
        assert!(result.state.pending.is_none() && result.state.frames.is_empty());
        assert!(result.state.piles.get(PileId::Draw).is_empty());
        assert!(result.state.piles.get(PileId::Play).is_empty());
        assert_eq!(
            result
                .state
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert_eq!(result.state.history.card_plays_finished_combat, 3);
        assert_eq!(result.state.fanouts.potion_slots(), [None]);
    }

    /// Distilled Chaos auto-plays an HpLoss body (#3404): Blood Wall's
    /// `CreatureCmd::Damage` (`0x38d618` IL_0041-IL_006b) is the same command
    /// under `AutoPlayFromDrawPile`. In a Stoke-free catalog the child runs
    /// originless under the `FrozenAutoBatch` and takes the card-body path;
    /// in a persistent-replay catalog it owns a persisted CardPlay parent and
    /// takes the active-card path. Both admit and land the same successor.
    #[test]
    fn distilled_chaos_auto_plays_an_hp_loss_body_with_and_without_a_persisted_parent() {
        let mut successors = Vec::new();
        for persistent in [false, true] {
            let mut builder = CatalogBuilder::new();
            let blood_wall = builder
                .intern_reachable(plain_identity(CardId::BloodWall, 0))
                .unwrap();
            builder
                .intern_monster(crate::ids::MonsterKind::Toadpole)
                .unwrap();
            let catalog = if persistent {
                builder.build().with_action_replay_required()
            } else {
                builder.build()
            };
            assert_eq!(catalog.requires_action_replay(), persistent);
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.hp = 50;
            state.max_hp = 50;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = 2;
            state.exact_piles = true;
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::DistilledChaos)],
                false,
                false,
                false,
                false,
                true,
            ));
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 1,
                atom: blood_wall,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });

            let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            assert_eq!(admit(&document, &state, &catalog), Ok(()), "{persistent}");
            let result = apply_action(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            )
            .unwrap();
            assert!(result.state.pending.is_none() && result.state.frames.is_empty());
            assert_eq!(
                (result.state.hp, result.state.block),
                (48, 16),
                "{persistent}"
            );
            assert_eq!(result.state.history.card_plays_finished_combat, 1);
            assert_eq!(
                result
                    .state
                    .piles
                    .get(PileId::Discard)
                    .as_slice()
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
                [1]
            );
            successors.push(HotBoundary::try_to_canonical(&result.state, &catalog).unwrap());
        }
        assert_eq!(successors[0], successors[1]);
    }

    #[test]
    fn distilled_chaos_zero_one_two_gathers_finish_exactly_the_available_cards() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.mark_persistent_action_replay_required();
        builder
            .intern_monster(crate::ids::MonsterKind::Toadpole)
            .unwrap();
        let catalog = builder.build();

        for available in 0..=2_u32 {
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.hp = 50;
            state.max_hp = 50;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = available + 1;
            state.exact_piles = true;
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::DistilledChaos)],
                false,
                false,
                false,
                false,
                true,
            ));
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend((1..=available).map(|uid| HotCard {
                    uid,
                    atom: defend,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                }));

            let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            assert_eq!(admit(&document, &state, &catalog), Ok(()));
            let completed = apply_action(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            )
            .unwrap()
            .state;
            assert!(completed.pending.is_none() && completed.frames.is_empty());
            assert_eq!(
                completed.history.card_plays_finished_combat,
                i32::try_from(available).unwrap()
            );
            assert_eq!(
                completed
                    .piles
                    .get(PileId::Discard)
                    .as_slice()
                    .iter()
                    .map(|card| card.uid)
                    .collect::<Vec<_>>(),
                (1..=available).collect::<Vec<_>>()
            );
            assert!(completed.piles.get(PileId::Draw).is_empty());
            assert!(completed.piles.get(PileId::Play).is_empty());
            assert_eq!(completed.fanouts.potion_slots(), [None]);
        }
    }

    #[test]
    fn two_distilled_slots_rehearse_and_execute_as_independent_public_roots() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 4;
        state.exact_piles = true;
        assert!(state.fanouts.set_potion_belt(
            vec![
                Some(PotionId::DistilledChaos),
                Some(PotionId::DistilledChaos),
            ],
            false,
            false,
            false,
            false,
            true,
        ));
        state
            .piles
            .get_mut(PileId::Draw)
            .make_mut()
            .extend((1..=3).map(|uid| HotCard {
                uid,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));

        let first = Action::UsePotion {
            slot: 0,
            target: None,
        };
        let mut events = Vec::new();
        let after_first = apply_action_into(&state, &catalog, &first, &mut events).unwrap();
        assert_eq!(
            after_first.fanouts.potion_slots(),
            [None, Some(PotionId::DistilledChaos)]
        );
        assert_eq!(after_first.history.card_plays_finished_combat, 3);
        assert!(after_first.pending.is_none() && after_first.frames.is_empty());

        let second = Action::UsePotion {
            slot: 1,
            target: None,
        };
        let mut late_refusal = after_first.clone();
        late_refusal.history.card_plays_finished_combat = i32::MAX;
        let late_refusal_before = late_refusal.clone();
        events.push(Event::TurnEnded { turn: 99 });
        assert_eq!(
            apply_action_into(&late_refusal, &catalog, &second, &mut events),
            Err(EngineRefusal::CounterOverflow("card_plays_finished_combat"))
        );
        assert_eq!(late_refusal, late_refusal_before);
        assert!(events.is_empty());

        let second_before = after_first.clone();
        let after_second = apply_action_into(&after_first, &catalog, &second, &mut events).unwrap();
        assert_eq!(after_second.fanouts.potion_slots(), [None, None]);
        assert_eq!(after_second.history.card_plays_finished_combat, 6);
        assert!(after_second.pending.is_none() && after_second.frames.is_empty());
        assert_eq!(after_first, second_before);
    }

    #[test]
    fn held_distilled_does_not_reject_an_unrelated_parked_selection() {
        let mut builder = CatalogBuilder::new();
        let purity = builder
            .intern_reachable(plain_identity(CardId::Purity, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 3;
        state.exact_piles = true;
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 1,
                atom: purity,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let play = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };

        let mut without_potion = state.clone();
        assert_eq!(
            without_potion.fanouts.remove_potion_at(0),
            Some(PotionId::DistilledChaos)
        );
        let parked_without = apply_action(&without_potion, &catalog, &play)
            .unwrap()
            .state;
        let answer_without = legal_actions(&parked_without, &catalog)[0];
        let completed_without = apply_action(&parked_without, &catalog, &answer_without)
            .unwrap()
            .state;

        let parked = apply_action(&state, &catalog, &play).unwrap().state;
        assert!(parked.pending.is_some());
        let wire = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let rebuilt = HotBoundary::from_canonical(&wire, &rebuilt_catalog).unwrap();
        admit(&wire, &rebuilt, &rebuilt_catalog).unwrap();
        let answer = legal_actions(&rebuilt, &rebuilt_catalog)[0];
        assert_eq!(answer, answer_without);
        let mut completed = apply_action(&rebuilt, &rebuilt_catalog, &answer)
            .unwrap()
            .state;
        assert_eq!(
            completed.fanouts.remove_potion_at(0),
            Some(PotionId::DistilledChaos)
        );
        assert_eq!(completed, completed_without);
    }

    #[test]
    fn distilled_chaos_second_child_selects_roundtrips_and_naturally_exhausts() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let purity = builder
            .intern_reachable(plain_identity(CardId::Purity, 0))
            .unwrap();
        let signal_boost = builder
            .intern_reachable(plain_identity(CardId::SignalBoost, 0))
            .unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        // A live enemy keeps `damage_combat_is_ending` false (#3183).
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            99,
        ));
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 5;
        state.exact_piles = true;
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: purity,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: signal_boost,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 4,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });

        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::PotionFinish { .. },
                crate::frame::Frame::FrozenAutoBatch { .. },
                crate::frame::Frame::CardPlay { .. },
            ]
        ));
        let batch = match parked.frames.as_slice()[2] {
            crate::frame::Frame::FrozenAutoBatch { record } => {
                parked.frames.frozen_auto_batch(record).unwrap()
            }
            _ => unreachable!(),
        };
        assert_eq!(batch.cursor, 2);
        assert_eq!(batch.uids().collect::<Vec<_>>(), [1, 2, 3]);

        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &rebuilt, &rebuilt_catalog), Ok(()));
        let completed = legal_actions(&rebuilt, &rebuilt_catalog)
            .iter()
            .filter_map(|action| apply_action(&rebuilt, &rebuilt_catalog, action).ok())
            .map(|transition| transition.state)
            .find(|candidate| {
                candidate
                    .piles
                    .get(PileId::Exhaust)
                    .as_slice()
                    .iter()
                    .any(|card| card.uid == 4)
            })
            .expect("one Purity answer exhausts the sole Hand card");
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.history.card_plays_finished_combat, 3);
        assert_eq!(completed.powers.value(PowerId::SignalBoost), 1);
        let exhausted = completed
            .piles
            .get(PileId::Exhaust)
            .as_slice()
            .iter()
            .map(|card| card.uid)
            .collect::<Vec<_>>();
        assert!(
            exhausted.contains(&2),
            "Purity takes its natural Exhaust route"
        );
        assert!(
            exhausted.contains(&4),
            "Purity exhausts the selected Hand card"
        );
        assert_eq!(completed.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn distilled_chaos_first_child_ending_leaves_later_gathered_cards_in_play() {
        let mut builder = CatalogBuilder::new();
        let thunderclap = builder
            .intern_reachable(plain_identity(CardId::Thunderclap, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 4;
        state.exact_piles = true;
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: thunderclap,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);

        let completed = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(completed.history.over);
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.history.card_plays_finished_combat, 1);
        assert_eq!(
            completed
                .piles
                .get(PileId::Play)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [2, 3, 1]
        );
        assert!(completed.piles.get(PileId::Discard).is_empty());
        assert_eq!(completed.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn distilled_chaos_partial_gather_survives_blocking_reshuffle_without_drawing() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.mark_persistent_action_replay_required();
        builder
            .intern_monster(crate::ids::MonsterKind::Toadpole)
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 5;
        state.exact_piles = true;
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        state.rng.set(
            crate::hot::RngStream::Rng,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 1,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((2..=4).map(|uid| HotCard {
                uid,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));

        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));
        let mut events = Vec::new();
        let parked = apply_action_into(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
            &mut events,
        )
        .unwrap();
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::PotionFinish { .. },
                crate::frame::Frame::FrozenAutoBatch { .. },
                crate::frame::Frame::Draw { .. }
            ]
        ));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::CardDrawn { .. }))
        );
        assert_eq!(parked.history.non_hand_draws_this_turn, 0);
        assert_eq!(parked.piles.get(PileId::Play).as_slice()[0].uid, 1);

        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &rebuilt, &rebuilt_catalog), Ok(()));
        let selected = legal_actions(&rebuilt, &rebuilt_catalog)[0];
        let mut resume_events = Vec::new();
        let resumed =
            apply_action_into(&rebuilt, &rebuilt_catalog, &selected, &mut resume_events).unwrap();
        assert!(resumed.pending.is_none() && resumed.frames.is_empty());
        assert!(
            !resume_events
                .iter()
                .any(|event| matches!(event, Event::CardDrawn { .. }))
        );
        assert_eq!(resumed.history.non_hand_draws_this_turn, 0);
        assert_eq!(resumed.history.card_plays_finished_combat, 3);
        assert_eq!(resumed.piles.get(PileId::Hand).len(), 1);
        assert_eq!(resumed.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn distilled_chaos_re_resolves_later_frozen_siblings_after_live_upgrade() {
        let mut builder = CatalogBuilder::new();
        let apotheosis = builder
            .intern_reachable(plain_identity(CardId::Apotheosis, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let outbreak = builder
            .intern_reachable(plain_identity(CardId::Outbreak, 0))
            .unwrap();
        builder
            .intern_monster(crate::ids::MonsterKind::Toadpole)
            .unwrap();
        builder.intern_all_card_upgrade_closure().unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let upgraded_outbreak = catalog.atom(&plain_identity(CardId::Outbreak, 1)).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 4;
        state.exact_piles = true;
        let mut monster = HotMonster::new(crate::ids::MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.slot = 0;
        monster.uid = 10;
        state.monsters_mut().push(monster);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        // Draw index zero is the top, so Apotheosis executes first and
        // upgrades both later frozen siblings while they are already in Play.
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: apotheosis,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: outbreak,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);

        let successor = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(successor.pending.is_none() && successor.frames.is_empty());
        assert_eq!(successor.block, 8);
        assert_eq!(successor.history.card_plays_finished_combat, 3);
        assert!(
            successor
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .any(|card| card.uid == 2 && card.atom == upgraded_outbreak)
        );
        assert_eq!(successor.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn distilled_chaos_reserves_later_automatic_target_rng_before_first_choice() {
        let mut builder = CatalogBuilder::new();
        let armaments = builder
            .intern_reachable(plain_identity(CardId::Armaments, 0))
            .unwrap();
        let bash = builder
            .intern_reachable(plain_identity(CardId::Bash, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let strike = builder
            .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        builder.intern_all_card_upgrade_closure().unwrap();
        builder
            .intern_monster(crate::ids::MonsterKind::Toadpole)
            .unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 5;
        state.exact_piles = true;
        let mut monster = HotMonster::new(crate::ids::MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.slot = 0;
        monster.uid = 10;
        state.monsters_mut().push(monster);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 4,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: armaments,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: bash,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.rng.set(
            crate::hot::RngStream::Targets,
            crate::hot::RngStreamState {
                words: [5, 6, 7, 8],
                counter: u64::MAX,
            },
        );

        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        let refused_document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert!(admit(&refused_document, &state, &catalog).is_err());
        assert!(!legal_actions(&state, &catalog).contains(&action));
        let before = state.clone();
        assert!(apply_action(&state, &catalog, &action).is_err());
        assert_eq!(state, before);

        state.rng.set(
            crate::hot::RngStream::Targets,
            crate::hot::RngStreamState {
                words: [5, 6, 7, 8],
                counter: u64::MAX - 1,
            },
        );
        let parked = apply_action(&state, &catalog, &action).unwrap().state;
        assert!(parked.pending.is_some());
        assert_eq!(
            parked.rng.get(crate::hot::RngStream::Targets).counter,
            u64::MAX - 1
        );
        let selected = legal_actions(&parked, &catalog)[0];
        let successor = apply_action(&parked, &catalog, &selected).unwrap().state;
        assert!(successor.pending.is_none() && successor.frames.is_empty());
        assert_eq!(
            successor.rng.get(crate::hot::RngStream::Targets).counter,
            u64::MAX
        );
        assert_eq!(successor.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn held_distilled_chaos_projects_hand_into_its_future_gather_domain() {
        let mut builder = CatalogBuilder::new();
        let bash = builder
            .intern_reachable(plain_identity(CardId::Bash, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 7;
        state.exact_piles = true;
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: bash,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 2,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);

        // Immediate use can gather only the safe Draw cards.  Admission must
        // nevertheless reserve the Bash after an ordinary Hand->Discard turn
        // boundary, where no live enemy makes its automatic target invalid.
        assert!(
            potions::manual_part_d_prerequisites_are_exact(
                &state,
                &catalog,
                PotionId::DistilledChaos,
            )
            .is_ok()
        );
        assert!(
            potions::prospective_part_d_prerequisites_are_exact(
                &state,
                &catalog,
                PotionId::DistilledChaos,
            )
            .is_err()
        );
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert!(admit(&document, &state, &catalog).is_err());

        state.piles.get_mut(PileId::Hand).make_mut()[0].atom = defend;
        assert!(
            potions::prospective_part_d_prerequisites_are_exact(
                &state,
                &catalog,
                PotionId::DistilledChaos,
            )
            .is_ok()
        );
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        admit(&document, &state, &catalog).unwrap();
    }

    #[test]
    fn held_distilled_chaos_projects_returning_exhausted_sovereign_blade() {
        let mut builder = CatalogBuilder::new();
        let blade = builder
            .intern_reachable(plain_identity(CardId::SovereignBlade, 0))
            .unwrap();
        builder
            .intern_reachable(plain_identity(CardId::SummonForth, 0))
            .unwrap();
        builder
            .intern_monster(crate::ids::MonsterKind::Toadpole)
            .unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 2;
        state.exact_piles = true;
        state.history.owner_card_plays_finished_this_turn = i16::MAX - 255;
        let mut monster = HotMonster::new(crate::ids::MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.slot = 0;
        monster.uid = 10;
        state.monsters_mut().push(monster);
        state.rng.set(
            crate::hot::RngStream::Targets,
            crate::hot::RngStreamState {
                words: [5, 6, 7, 8],
                counter: 0,
            },
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state
            .piles
            .get_mut(PileId::Exhaust)
            .make_mut()
            .push(HotCard {
                uid: 1,
                atom: blade,
                // BaseReplayCount lives in slot 7 (#3629).
                flags: CARD_FLAG_SOVEREIGN_BLADE_STATE | CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        let mut replayed = state.card_states.get(1);
        replayed.set_base_replay_count(Some(255)).unwrap();
        state.card_states.set(1, replayed);

        assert!(
            potions::manual_part_d_prerequisites_are_exact(
                &state,
                &catalog,
                PotionId::DistilledChaos,
            )
            .is_ok()
        );
        assert!(
            potions::prospective_part_d_prerequisites_are_exact(
                &state,
                &catalog,
                PotionId::DistilledChaos,
            )
            .is_err()
        );
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert!(admit(&document, &state, &catalog).is_err());

        state.card_states.set(1, Default::default());
        let safe_result = potions::prospective_part_d_prerequisites_are_exact(
            &state,
            &catalog,
            PotionId::DistilledChaos,
        );
        assert!(safe_result.is_ok(), "{safe_result:?}");

        // The prospective Summon Forth move is a physical relocation, not a
        // payload repair. A malformed exhausted Blade therefore remains a
        // refusal instead of acquiring the native-state flag in projection.
        let mut malformed = state.clone();
        malformed.piles.get_mut(PileId::Exhaust).make_mut()[0].flags =
            CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        assert!(
            potions::prospective_part_d_prerequisites_are_exact(
                &malformed,
                &catalog,
                PotionId::DistilledChaos,
            )
            .is_err()
        );
    }

    #[test]
    fn held_distilled_chaos_closes_catalog_only_nested_play_sources() {
        fn state_with_distilled(defend: crate::catalog::CardAtom) -> HotState {
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = 2;
            state.exact_piles = true;
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::DistilledChaos)],
                false,
                false,
                false,
                false,
                true,
            ));
            state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
                uid: 1,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            state
        }

        let mut safe_builder = CatalogBuilder::new();
        let safe_defend = safe_builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        safe_builder
            .intern_reachable(plain_identity(CardId::Burst, 1))
            .unwrap();
        safe_builder.mark_persistent_action_replay_required();
        let safe_catalog = safe_builder.build();
        let safe = state_with_distilled(safe_defend);
        assert!(
            potions::prospective_part_d_prerequisites_are_exact(
                &safe,
                &safe_catalog,
                PotionId::DistilledChaos,
            )
            .is_ok()
        );
        let document = HotBoundary::try_to_canonical(&safe, &safe_catalog).unwrap();
        admit(&document, &safe, &safe_catalog).unwrap();

        let mut havoc_builder = CatalogBuilder::new();
        let havoc_defend = havoc_builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        havoc_builder
            .intern_reachable(plain_identity(CardId::Havoc, 0))
            .unwrap();
        havoc_builder.mark_persistent_action_replay_required();
        let havoc_catalog = havoc_builder.build();
        let havoc = state_with_distilled(havoc_defend);
        assert!(
            potions::manual_part_d_prerequisites_are_exact(
                &havoc,
                &havoc_catalog,
                PotionId::DistilledChaos,
            )
            .is_ok()
        );
        assert_eq!(
            potions::prospective_part_d_prerequisites_are_exact(
                &havoc,
                &havoc_catalog,
                PotionId::DistilledChaos,
            ),
            Ok(())
        );
        let havoc_document = HotBoundary::try_to_canonical(&havoc, &havoc_catalog).unwrap();
        admit(&havoc_document, &havoc, &havoc_catalog).unwrap();

        let mut imitation_builder = CatalogBuilder::new();
        let imitation_defend = imitation_builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        imitation_builder
            .intern_reachable(plain_identity(CardId::ImitationLearning, 0))
            .unwrap();
        imitation_builder
            .intern_reachable(plain_identity(CardId::Inflame, 0))
            .unwrap();
        imitation_builder.mark_persistent_action_replay_required();
        let imitation_catalog = imitation_builder.build();
        let imitation = state_with_distilled(imitation_defend);
        assert!(
            potions::manual_part_d_prerequisites_are_exact(
                &imitation,
                &imitation_catalog,
                PotionId::DistilledChaos,
            )
            .is_ok()
        );
        assert_eq!(
            potions::prospective_part_d_prerequisites_are_exact(
                &imitation,
                &imitation_catalog,
                PotionId::DistilledChaos,
            ),
            Ok(())
        );
        let imitation_document =
            HotBoundary::try_to_canonical(&imitation, &imitation_catalog).unwrap();
        admit(&imitation_document, &imitation, &imitation_catalog).unwrap();
    }

    #[test]
    fn distilled_chaos_accepts_finite_unceasing_top_hellraiser_suffix() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let strike = builder
            .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder
            .set_relics(&[crate::ids::RelicId::RelicUnceasingTop])
            .unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 7;
        state.exact_piles = true;
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.slot = 0;
        monster.uid = 10;
        state.monsters_mut().push(monster);
        state.rng.set(
            crate::hot::RngStream::Targets,
            crate::hot::RngStreamState {
                words: [5, 6, 7, 8],
                counter: 0,
            },
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.fanouts.set_unceasing_top(true);
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: defend,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 2,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);

        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        assert_eq!(
            potions::manual_part_d_prerequisites_are_exact(
                &state,
                &catalog,
                PotionId::DistilledChaos,
            ),
            Ok(())
        );
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        admit(&document, &state, &catalog).unwrap();
        assert!(legal_actions(&state, &catalog).contains(&action));
        let before = state.clone();
        let after = apply_action(&state, &catalog, &action).unwrap().state;
        assert!(after.pending.is_none() && after.frames.is_empty());
        assert_eq!(after.block, 5);
        assert_eq!(after.monsters[0].hp, 94);
        assert_eq!(state, before);
    }

    #[test]
    fn distilled_chaos_reserves_replayed_later_child_history_before_first_choice() {
        let mut builder = CatalogBuilder::new();
        let armaments = builder
            .intern_reachable(plain_identity(CardId::Armaments, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let strike = builder
            .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        builder.intern_all_card_upgrade_closure().unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 6;
        state.exact_piles = true;
        state.history.owner_card_plays_finished_this_turn = i16::MAX - 3;
        state.history.skill_plays_finished_this_turn = i16::MAX - 3;
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.piles.get_mut(PileId::Hand).make_mut().extend([
            HotCard {
                uid: 4,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: armaments,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut replayed = state.card_states.get(2);
        replayed.set_base_replay_count(Some(255)).unwrap();
        state.card_states.set(2, replayed);

        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert!(admit(&document, &state, &catalog).is_err());
        assert!(!legal_actions(&state, &catalog).contains(&action));
        let before = state.clone();
        assert_eq!(
            apply_action(&state, &catalog, &action).unwrap_err(),
            EngineRefusal::CounterOverflow("owner_card_plays_finished_this_turn")
        );
        assert_eq!(state, before);
    }

    #[test]
    fn distilled_chaos_dynamic_writer_accepts_256_and_rolls_back_late_257() {
        let mut builder = CatalogBuilder::new();
        let sword_sage = builder
            .intern_reachable(plain_identity(CardId::SwordSage, 0))
            .unwrap();
        let blade = builder
            .intern_reachable(plain_identity(CardId::SovereignBlade, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 4;
        state.exact_piles = true;
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000_000);
        monster.max_hp = 1_000_000;
        monster.slot = 0;
        monster.uid = 10;
        state.monsters_mut().push(monster);
        state.rng.set(
            crate::hot::RngStream::Rng,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 7,
            },
        );
        state.rng.set(
            crate::hot::RngStream::Targets,
            crate::hot::RngStreamState {
                words: [5, 6, 7, 8],
                counter: 11,
            },
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.piles.get_mut(PileId::Draw).make_mut().push(HotCard {
            uid: 1,
            atom: sword_sage,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            HotCard {
                uid: 2,
                atom: blade,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_SOVEREIGN_BLADE_STATE,
            },
            HotCard {
                uid: 3,
                atom: blade,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_SOVEREIGN_BLADE_STATE,
            },
        ]);
        let mut replayed = state.card_states.get(2);
        replayed.set_base_replay_count(Some(254)).unwrap();
        state.card_states.set(2, replayed);

        assert_eq!(
            potions::manual_part_d_prerequisites_are_exact(
                &state,
                &catalog,
                PotionId::DistilledChaos,
            ),
            Ok(())
        );
        let accepted = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(accepted.pending.is_none() && accepted.frames.is_empty());
        assert!((257..=259).contains(&accepted.history.card_plays_finished_combat));
        assert_eq!(accepted.card_states.get(2).base_replay_count(), Some(255));
        assert_eq!(accepted.card_states.get(3).base_replay_count(), Some(1));

        let mut overflowed = state.clone();
        let mut replayed = overflowed.card_states.get(2);
        replayed.set_base_replay_count(Some(255)).unwrap();
        overflowed.card_states.set(2, replayed);
        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        assert_eq!(
            potions::manual_part_d_prerequisites_are_exact(
                &overflowed,
                &catalog,
                PotionId::DistilledChaos,
            )
            .unwrap_err(),
            EngineRefusal::CounterOverflow("play count")
        );
        assert!(!legal_actions(&overflowed, &catalog).contains(&action));
        let before = overflowed.clone();
        let mut events = vec![Event::TurnEnded { turn: 99 }];
        assert_eq!(
            apply_action_into(&overflowed, &catalog, &action, &mut events).unwrap_err(),
            EngineRefusal::CounterOverflow("play count")
        );
        assert_eq!(overflowed, before);
        assert!(events.is_empty());
    }

    #[test]
    fn distilled_chaos_applies_signal_echo_writers_in_physical_order() {
        fn run(order: [CardIdentity; 3]) -> HotState {
            let mut builder = CatalogBuilder::new();
            let atoms = order.map(|identity| builder.intern_reachable(identity).unwrap());
            builder.mark_persistent_action_replay_required();
            builder
                .intern_monster(crate::ids::MonsterKind::Toadpole)
                .unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            // A live enemy keeps `damage_combat_is_ending` false (#3183).
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                99,
            ));
            state.hp = 50;
            state.max_hp = 50;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = 4;
            state.exact_piles = true;
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::DistilledChaos)],
                false,
                false,
                false,
                false,
                true,
            ));
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend(atoms.into_iter().enumerate().map(|(index, atom)| HotCard {
                    uid: u32::try_from(index + 1).unwrap(),
                    atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                }));
            let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            assert_eq!(admit(&document, &state, &catalog), Ok(()));
            apply_action(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            )
            .unwrap()
            .state
        }

        for signal_upgrade in 0..=1 {
            for echo_upgrade in 0..=1 {
                let signal_first = run([
                    plain_identity(CardId::SignalBoost, signal_upgrade),
                    plain_identity(CardId::EchoForm, echo_upgrade),
                    plain_identity(CardId::DefendIronclad, 0),
                ]);
                assert_eq!(signal_first.history.card_plays_finished_combat, 4);
                assert_eq!(signal_first.powers.value(PowerId::SignalBoost), 0);
                assert_eq!(signal_first.powers.value(PowerId::EchoForm), 2);
                assert_eq!(signal_first.block, 5);

                let echo_first = run([
                    plain_identity(CardId::EchoForm, echo_upgrade),
                    plain_identity(CardId::SignalBoost, signal_upgrade),
                    plain_identity(CardId::DefendIronclad, 0),
                ]);
                assert_eq!(echo_first.history.card_plays_finished_combat, 3);
                assert_eq!(echo_first.powers.value(PowerId::SignalBoost), 1);
                assert_eq!(echo_first.powers.value(PowerId::EchoForm), 1);
                assert_eq!(echo_first.block, 5);
            }
        }
    }

    #[test]
    fn distilled_chaos_burst_levels_self_replay_and_order_are_exact() {
        fn run(upgrade: u8, writer_first: bool) -> HotState {
            let mut builder = CatalogBuilder::new();
            let burst = builder
                .intern_reachable(plain_identity(CardId::Burst, upgrade))
                .unwrap();
            let defend = builder
                .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
                .unwrap();
            builder.mark_persistent_action_replay_required();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            // A live enemy keeps `damage_combat_is_ending` false (#3183).
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                99,
            ));
            state.hp = 50;
            state.max_hp = 50;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = 4;
            state.exact_piles = true;
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::DistilledChaos)],
                false,
                false,
                false,
                false,
                true,
            ));
            assert!(state.fanouts.set_duplication(1));
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            let atoms = if writer_first {
                [burst, defend, defend]
            } else {
                [defend, burst, defend]
            };
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend(atoms.into_iter().enumerate().map(|(index, atom)| HotCard {
                    uid: u32::try_from(index + 1).unwrap(),
                    atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                }));
            apply_action(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            )
            .unwrap()
            .state
        }

        for upgrade in 0..=1 {
            let writer_first = run(upgrade, true);
            assert_eq!(writer_first.history.card_plays_finished_combat, 6);
            assert_eq!(writer_first.fanouts.duplication(), 0);
            assert_eq!(
                writer_first.powers.value(PowerId::Burst),
                2 * i32::from(upgrade)
            );
            assert_eq!(writer_first.block, 20);

            let writer_middle = run(upgrade, false);
            assert_eq!(writer_middle.history.card_plays_finished_combat, 5);
            assert_eq!(writer_middle.fanouts.duplication(), 0);
            assert_eq!(
                writer_middle.powers.value(PowerId::Burst),
                i32::from(upgrade)
            );
            assert_eq!(writer_middle.block, 20);
        }
    }

    #[test]
    fn distilled_chaos_one_two_punch_levels_and_orders_are_exact() {
        fn run(upgrade: u8, writer_first: bool) -> HotState {
            let mut builder = CatalogBuilder::new();
            let one_two_punch = builder
                .intern_reachable(plain_identity(CardId::OneTwoPunch, upgrade))
                .unwrap();
            let strike = builder
                .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
                .unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.mark_persistent_action_replay_required();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = 4;
            state.exact_piles = true;
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000_000);
            monster.max_hp = 1_000_000;
            monster.slot = 0;
            monster.uid = 10;
            state.monsters_mut().push(monster);
            state.rng.set(
                crate::hot::RngStream::Targets,
                crate::hot::RngStreamState {
                    words: [5, 6, 7, 8],
                    counter: 0,
                },
            );
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::DistilledChaos)],
                false,
                false,
                false,
                false,
                true,
            ));
            let atoms = if writer_first {
                [one_two_punch, strike, strike]
            } else {
                [strike, one_two_punch, strike]
            };
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend(atoms.into_iter().enumerate().map(|(index, atom)| HotCard {
                    uid: u32::try_from(index + 1).unwrap(),
                    atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                }));
            apply_action(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            )
            .unwrap()
            .state
        }

        for upgrade in 0..=1 {
            let writer_first = run(upgrade, true);
            assert_eq!(
                writer_first.history.card_plays_finished_combat,
                4 + i32::from(upgrade)
            );
            assert_eq!(writer_first.powers.value(PowerId::OneTwoPunch), 0);

            let writer_middle = run(upgrade, false);
            assert_eq!(writer_middle.history.card_plays_finished_combat, 4);
            assert_eq!(
                writer_middle.powers.value(PowerId::OneTwoPunch),
                i32::from(upgrade)
            );
        }
    }

    #[test]
    fn distilled_chaos_sword_sage_updates_equal_atom_uids_independently() {
        for upgrade in 0..=1 {
            let mut builder = CatalogBuilder::new();
            let sword_sage = builder
                .intern_reachable(plain_identity(CardId::SwordSage, upgrade))
                .unwrap();
            let blade = builder
                .intern_reachable(plain_identity(CardId::SovereignBlade, 0))
                .unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.mark_persistent_action_replay_required();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = 4;
            state.exact_piles = true;
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000_000);
            monster.max_hp = 1_000_000;
            monster.slot = 0;
            monster.uid = 10;
            state.monsters_mut().push(monster);
            state.rng.set(
                crate::hot::RngStream::Targets,
                crate::hot::RngStreamState {
                    words: [5, 6, 7, 8],
                    counter: 0,
                },
            );
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::DistilledChaos)],
                false,
                false,
                false,
                false,
                true,
            ));
            state.piles.get_mut(PileId::Draw).make_mut().extend([
                HotCard {
                    uid: 1,
                    atom: sword_sage,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 2,
                    atom: blade,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_SOVEREIGN_BLADE_STATE,
                },
                HotCard {
                    uid: 3,
                    atom: blade,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_SOVEREIGN_BLADE_STATE,
                },
            ]);
            let mut replayed = state.card_states.get(3);
            replayed.set_base_replay_count(Some(2)).unwrap();
            state.card_states.set(3, replayed);

            let mut malformed_order = state.clone();
            malformed_order
                .powers
                .set(PowerId::Shroud, SlotWire::Int, 1);
            assert_eq!(
                potions::manual_part_d_prerequisites_are_exact(
                    &malformed_order,
                    &catalog,
                    PotionId::DistilledChaos,
                )
                .unwrap_err(),
                EngineRefusal::MalformedArgs("Sword Sage AfterPowerAmountChanged order")
            );

            let successor = apply_action(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            )
            .unwrap()
            .state;
            assert_eq!(successor.history.card_plays_finished_combat, 7);
            assert_eq!(successor.powers.value(PowerId::SwordSage), 1);
            assert_eq!(successor.card_states.get(2).base_replay_count(), Some(1));
            assert_eq!(successor.card_states.get(3).base_replay_count(), Some(3));
        }
    }

    #[test]
    fn distilled_chaos_apotheosis_re_resolves_writer_uid_and_rolls_back_late_overflow() {
        fn fixture(order: [CardId; 3]) -> (HotState, Catalog) {
            let mut builder = CatalogBuilder::new();
            let atoms = order.map(|id| builder.intern_reachable(plain_identity(id, 0)).unwrap());
            builder.intern_all_card_upgrade_closure().unwrap();
            builder.mark_persistent_action_replay_required();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            // A live enemy keeps `damage_combat_is_ending` false (#3183).
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                99,
            ));
            state.hp = 50;
            state.max_hp = 50;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = 4;
            state.exact_piles = true;
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::DistilledChaos)],
                false,
                false,
                false,
                false,
                true,
            ));
            state
                .piles
                .get_mut(PileId::Draw)
                .make_mut()
                .extend(atoms.into_iter().enumerate().map(|(index, atom)| HotCard {
                    uid: u32::try_from(index + 1).unwrap(),
                    atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                }));
            (state, catalog)
        }

        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        let (apotheosis_first, catalog) =
            fixture([CardId::Apotheosis, CardId::Burst, CardId::DefendIronclad]);
        let apotheosis_first = apply_action(&apotheosis_first, &catalog, &action)
            .unwrap()
            .state;
        assert_eq!(apotheosis_first.history.card_plays_finished_combat, 4);
        assert_eq!(apotheosis_first.powers.value(PowerId::Burst), 1);
        assert_eq!(apotheosis_first.block, 16);

        let (writer_first, catalog) =
            fixture([CardId::Burst, CardId::Apotheosis, CardId::DefendIronclad]);
        let writer_first = apply_action(&writer_first, &catalog, &action)
            .unwrap()
            .state;
        assert_eq!(writer_first.history.card_plays_finished_combat, 4);
        assert_eq!(writer_first.powers.value(PowerId::Burst), 0);
        assert_eq!(writer_first.block, 8);

        let (mut overflow, catalog) =
            fixture([CardId::Apotheosis, CardId::Burst, CardId::DefendIronclad]);
        overflow
            .powers
            .set(PowerId::Burst, SlotWire::Int, i32::MAX - 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut overflow);
        assert_eq!(
            potions::manual_part_d_prerequisites_are_exact(
                &overflow,
                &catalog,
                PotionId::DistilledChaos,
            )
            .unwrap_err(),
            EngineRefusal::CounterOverflow("burst")
        );
        let before = overflow.clone();
        let mut events = vec![Event::TurnEnded { turn: 99 }];
        assert_eq!(
            apply_action_into(&overflow, &catalog, &action, &mut events).unwrap_err(),
            EngineRefusal::CounterOverflow("burst")
        );
        assert_eq!(overflow, before);
        assert!(events.is_empty());
    }

    #[test]
    fn distilled_chaos_refuses_nonlocal_dampen_restore_before_use() {
        let mut builder = CatalogBuilder::new();
        builder.intern_magi_dampen_smoke_foundation().unwrap();
        let (_burst_l0, burst_l1) = builder
            .intern_dampen_card_pair(plain_identity(CardId::Burst, 0))
            .unwrap();
        let strike = builder
            .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 3;
        state.exact_piles = true;
        let mut flail = HotMonster::new(MonsterKind::FlailKnight, 108);
        flail.max_hp = 108;
        assert!(flail.random_ai.set_next(Some(0)));
        assert!(flail.random_ai.set_log(&[2, 0]));
        let mut spectral = HotMonster::new(MonsterKind::SpectralKnight, 97);
        spectral.max_hp = 97;
        spectral.slot = 1;
        spectral.uid = 1;
        assert!(spectral.random_ai.set_next(Some(1)));
        assert!(spectral.random_ai.set_log(&[0, 1]));
        let mut magi = HotMonster::new(MonsterKind::MagiKnight, 1);
        magi.max_hp = 89;
        magi.slot = 2;
        magi.uid = 2;
        magi.loop_pos = 1;
        state.monsters_mut().extend([flail, spectral, magi]);
        state.powers.set(PowerId::HexPower, SlotWire::Int, 2);
        state.rng.set(
            crate::hot::RngStream::Targets,
            crate::hot::RngStreamState {
                words: [5, 6, 7, 8],
                counter: 0,
            },
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_HEXED,
            },
            HotCard {
                uid: 2,
                atom: burst_l1,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | CARD_FLAG_HEXED,
            },
        ]);
        crate::engine::cards::apply_dampen_power(&mut state, &catalog, 2).unwrap();
        assert!(crate::engine::cards::dampen_state_is_exact(
            &state, &catalog
        ));

        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        assert_eq!(
            potions::manual_part_d_prerequisites_are_exact(
                &state,
                &catalog,
                PotionId::DistilledChaos,
            )
            .unwrap_err(),
            EngineRefusal::MalformedArgs(potions::DISTILLED_DAMPEN_FRONTIER)
        );
        let before = state.clone();
        let mut events = vec![Event::TurnEnded { turn: 99 }];
        assert_eq!(
            apply_action_into(&state, &catalog, &action, &mut events).unwrap_err(),
            EngineRefusal::MalformedArgs(potions::DISTILLED_DAMPEN_FRONTIER)
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn distilled_chaos_rehearses_recursive_autoplay_before_first_choice() {
        let mut builder = CatalogBuilder::new();
        let armaments = builder
            .intern_reachable(plain_identity(CardId::Armaments, 0))
            .unwrap();
        let havoc = builder
            .intern_reachable(plain_identity(CardId::Havoc, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_all_card_upgrade_closure().unwrap();
        builder.mark_persistent_action_replay_required();
        builder
            .intern_monster(crate::ids::MonsterKind::Toadpole)
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 5;
        state.exact_piles = true;
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: armaments,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: havoc,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);

        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        admit(&document, &state, &catalog).unwrap();
        assert!(legal_actions(&state, &catalog).contains(&action));
        let before = state.clone();
        let resumed = apply_action(&state, &catalog, &action).unwrap().state;
        assert!(resumed.pending.is_none());
        assert!(resumed.frames.is_empty());
        assert_eq!(resumed.block, 15);
        assert_eq!(resumed.history.card_plays_finished_combat, 4);
        assert_eq!(state, before);
    }

    #[test]
    fn distilled_chaos_rehearses_imitation_power_clone_and_uid_writer() {
        let mut builder = CatalogBuilder::new();
        let _imitation = builder
            .intern_reachable(plain_identity(CardId::ImitationLearning, 0))
            .unwrap();
        let inflame = builder
            .intern_reachable(plain_identity(CardId::Inflame, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.mark_persistent_action_replay_required();
        builder
            .intern_monster(crate::ids::MonsterKind::Toadpole)
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 4;
        state.exact_piles = true;
        assert!(state.fanouts.set_imitation_learning(0, 1));
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.piles.get_mut(PileId::Draw).make_mut().extend(
            [inflame, defend, defend]
                .into_iter()
                .enumerate()
                .map(|(index, atom)| HotCard {
                    uid: u32::try_from(index + 1).unwrap(),
                    atom,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                }),
        );

        let before = state.clone();
        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        let prerequisites = potions::prospective_part_d_prerequisites_are_exact(
            &state,
            &catalog,
            PotionId::DistilledChaos,
        );
        assert_eq!(prerequisites, Ok(()));
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        admit(&document, &state, &catalog).unwrap();
        let after = apply_action(&state, &catalog, &action).unwrap().state;
        assert!(after.pending.is_none() && after.frames.is_empty());
        assert_eq!(after.powers.value(PowerId::Strength), 4);
        assert_eq!(after.next_card_uid, 5);
        assert_eq!(after.history.card_plays_finished_combat, 4);
        assert_eq!(state, before);
    }

    #[test]
    fn r52e_distilled_alchemize_l0_l1_generate_and_procure_exactly_once() {
        for upgrade in 0..=1 {
            let mut builder = CatalogBuilder::new();
            let alchemize = builder
                .intern_reachable(plain_identity(CardId::Alchemize, upgrade))
                .unwrap();
            builder.mark_persistent_action_replay_required();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.hp = 50;
            state.max_hp = 50;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = 2;
            state.exact_piles = true;
            state.reward_card_pool = Some(crate::catalog::RewardPool::Ironclad);
            state.entropy_card_pool = Some(crate::catalog::RewardPool::Ironclad);
            state.fully_unlocked_card_pool_epochs = true;
            give_a_real_root_s_current_build_provenance(&mut state);
            state.rng.set(
                crate::hot::RngStream::PotionGeneration,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: u64::MAX - 2,
                },
            );
            for stream in crate::hot::RngStream::ALL {
                if state.rng.is_vacant(stream) {
                    state.rng.set(
                        stream,
                        crate::hot::RngStreamState {
                            words: [11, 12, 13, 14],
                            counter: 0,
                        },
                    );
                }
            }
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::DistilledChaos)],
                false,
                false,
                false,
                false,
                true,
            ));
            let card = HotCard {
                uid: 1,
                atom: alchemize,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            };
            state.piles.get_mut(PileId::Draw).make_mut().push(card);
            let action = Action::UsePotion {
                slot: 0,
                target: None,
            };
            let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
            let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
            assert_eq!(
                potions::prospective_part_d_prerequisites_are_exact(
                    &state,
                    &catalog,
                    PotionId::DistilledChaos,
                ),
                Ok(())
            );
            assert_eq!(admit(&document, &state, &catalog), Ok(()));
            assert!(legal_actions(&state, &catalog).contains(&action));
            let successor = apply_action(&state, &catalog, &action).unwrap().state;
            assert!(successor.pending.is_none() && successor.frames.is_empty());
            assert!(successor.piles.get(PileId::Draw).is_empty());
            assert_eq!(successor.piles.get(PileId::Exhaust).len(), 1);
            assert!(successor.fanouts.potion_slots()[0].is_some());
            assert_eq!(
                successor
                    .rng
                    .get(crate::hot::RngStream::PotionGeneration)
                    .counter,
                u64::MAX
            );
        }
    }

    #[test]
    fn eidolon_exhaust_alchemize_is_rehearsed_as_one_exact_outer_root() {
        fn fixture(
            counter: u64,
            eidolon_replays: Option<i32>,
            alchemize_replays: Option<i32>,
        ) -> (HotState, Catalog) {
            let mut builder = CatalogBuilder::new();
            let eidolon = builder
                .intern_reachable(plain_identity(CardId::Eidolon, 0))
                .unwrap();
            let alchemize = builder
                .intern_reachable(plain_identity(CardId::Alchemize, 0))
                .unwrap();
            builder
                .intern_reachable(plain_identity(CardId::Apotheosis, 0))
                .unwrap();
            builder
                .intern_reachable(plain_identity(CardId::Offering, 0))
                .unwrap();
            builder.intern_all_card_upgrade_closure().unwrap();
            builder.mark_persistent_action_replay_required();
            builder
                .intern_monster(crate::ids::MonsterKind::Toadpole)
                .unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.monsters_mut().push(crate::hot::HotMonster::new(
                crate::ids::MonsterKind::Toadpole,
                100,
            ));
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 10;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.exact_piles = true;
            state.reward_card_pool = Some(crate::catalog::RewardPool::Ironclad);
            state.entropy_card_pool = Some(crate::catalog::RewardPool::Ironclad);
            state.fully_unlocked_card_pool_epochs = true;
            state.next_card_uid = 3;
            state.rng.set(
                crate::hot::RngStream::PotionGeneration,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter,
                },
            );
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::BlockPotion)],
                true,
                false,
                false,
                false,
                true,
            ));
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: eidolon,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            state
                .piles
                .get_mut(PileId::Exhaust)
                .make_mut()
                .push(HotCard {
                    uid: 2,
                    atom: alchemize,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                });
            for (uid, replays) in [(1, eidolon_replays), (2, alchemize_replays)] {
                if let Some(replays) = replays {
                    let mut physical = state.card_states.get(uid);
                    physical.set_base_replay_count(Some(replays)).unwrap();
                    state.card_states.set(uid, physical);
                }
            }
            (state, catalog)
        }

        let action = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };
        let (mut state, catalog) = fixture(u64::MAX - 2, None, None);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::BlockPotion)],
            false,
            false,
            false,
            false,
            true,
        ));
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admission::admit(&document, &state, &catalog), Ok(()));
        let completed = apply_action(&state, &catalog, &action).unwrap().state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.history.card_plays_finished_combat, 2);
        assert_eq!(
            completed
                .rng
                .get(crate::hot::RngStream::PotionGeneration)
                .counter,
            u64::MAX
        );
        assert_eq!(
            completed
                .piles
                .get(PileId::Exhaust)
                .as_slice()
                .iter()
                .filter(|card| card.uid == 2)
                .count(),
            1
        );

        let (overflow, catalog) = fixture(u64::MAX - 1, None, None);
        let before = overflow.clone();
        let mut events = vec![Event::TurnEnded { turn: 99 }];
        assert_eq!(
            apply_action_into(&overflow, &catalog, &action, &mut events),
            Err(EngineRefusal::CounterOverflow(
                "Alchemize potion generation counter"
            ))
        );
        assert_eq!(overflow, before);
        assert!(events.is_empty());

        // Both active Eidolons leave Exhaust before their bodies, so the
        // nested command is finite. It can replay Alchemize again and reaches
        // the explicit potion RNG overflow; the whole public root rolls back.
        let (mut recursive, catalog) = fixture(u64::MAX - 2, None, None);
        let eidolon = catalog.atom(&plain_identity(CardId::Eidolon, 0)).unwrap();
        recursive
            .piles
            .get_mut(PileId::Exhaust)
            .make_mut()
            .push(HotCard {
                uid: 3,
                atom: eidolon,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
        recursive.next_card_uid = 4;
        let wire = HotBoundary::try_to_canonical(&recursive, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&wire).unwrap();
        let recursive = HotBoundary::from_canonical(&wire, &catalog).unwrap();
        let before = recursive.clone();
        let mut events = vec![Event::TurnEnded { turn: 99 }];
        assert_eq!(
            apply_action_into(&recursive, &catalog, &action, &mut events),
            Err(EngineRefusal::CounterOverflow(
                "Alchemize potion generation counter"
            ))
        );
        assert_eq!(recursive, before);
        assert!(events.is_empty());

        // One live generic writer belongs to the outer Eidolon and is
        // consumed/thresholded there. Its two outer bodies each execute one
        // Alchemize body; the child must not inherit the same writer again.
        for writer in [PowerId::Burst, PowerId::EchoForm] {
            let (mut state, catalog) = fixture(u64::MAX - 4, None, None);
            state.powers.set(writer, SlotWire::Int, 1);
            turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            let completed = apply_action(&state, &catalog, &action).unwrap().state;
            assert_eq!(
                completed
                    .rng
                    .get(crate::hot::RngStream::PotionGeneration)
                    .counter,
                u64::MAX,
                "{writer:?} must apply to Eidolon exactly once"
            );
            assert_eq!(completed.history.card_plays_finished_combat, 4);

            let (mut overflow, catalog) = fixture(u64::MAX - 3, None, None);
            overflow.powers.set(writer, SlotWire::Int, 1);
            turn::hydrate_after_side_turn_end_power_order_for_test(&mut overflow);
            assert!(matches!(
                apply_action(&overflow, &catalog, &action),
                Err(EngineRefusal::CounterOverflow(
                    "Alchemize potion generation counter"
                ))
            ));
        }
        let (mut duplicated, catalog) = fixture(u64::MAX - 4, None, None);
        assert!(duplicated.fanouts.set_duplication(1));
        turn::hydrate_after_side_turn_end_power_order_for_test(&mut duplicated);
        let completed = apply_action(&duplicated, &catalog, &action).unwrap().state;
        assert_eq!(
            completed
                .rng
                .get(crate::hot::RngStream::PotionGeneration)
                .counter,
            u64::MAX
        );
        assert_eq!(completed.history.card_plays_finished_combat, 4);

        // Merely holding Duplicator does not duplicate the Eidolon action.
        let (mut held, catalog) = fixture(u64::MAX - 2, None, None);
        assert!(held.fanouts.set_potion_belt(
            vec![Some(PotionId::Duplicator)],
            true,
            false,
            false,
            false,
            true,
        ));
        let completed = apply_action(&held, &catalog, &action).unwrap().state;
        assert_eq!(
            completed
                .rng
                .get(crate::hot::RngStream::PotionGeneration)
                .counter,
            u64::MAX
        );

        // A second Burst charge survives the outer Eidolon publication. If
        // Alchemize is the first frozen child it consumes that residue (three
        // attempts across the two outer bodies); an earlier exhausting Skill
        // consumes it instead, leaving two. This pins the source pile's exact
        // order rather than applying one blanket child multiplier.
        for (alchemize_first, counter) in [(true, u64::MAX - 6), (false, u64::MAX - 4)] {
            let (mut state, catalog) = fixture(counter, None, None);
            state.powers.set(PowerId::Burst, SlotWire::Int, 2);
            turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            let apotheosis = catalog
                .atom(&plain_identity(CardId::Apotheosis, 0))
                .unwrap();
            let card = HotCard {
                uid: 3,
                atom: apotheosis,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            };
            state.next_card_uid = 4;
            if alchemize_first {
                state.piles.get_mut(PileId::Exhaust).make_mut().push(card);
            } else {
                state
                    .piles
                    .get_mut(PileId::Exhaust)
                    .make_mut()
                    .insert(0, card);
            }
            let completed = apply_action(&state, &catalog, &action).unwrap().state;
            assert_eq!(
                completed
                    .rng
                    .get(crate::hot::RngStream::PotionGeneration)
                    .counter,
                u64::MAX,
                "ordered sibling must consume the residual Burst exactly"
            );
        }

        // The fixed batch is re-read in order for each outer body. A lethal
        // earlier Offering ends combat and suppresses the later Alchemize, so
        // an exhausted counter is valid and no potion RNG is consumed.
        let (mut terminal, catalog) = fixture(u64::MAX, None, None);
        terminal.hp = 1;
        let offering = catalog.atom(&plain_identity(CardId::Offering, 0)).unwrap();
        terminal.piles.get_mut(PileId::Exhaust).make_mut().insert(
            0,
            HotCard {
                uid: 3,
                atom: offering,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        );
        terminal.next_card_uid = 4;
        let completed = apply_action(&terminal, &catalog, &action).unwrap().state;
        assert!(completed.history.over);
        assert_eq!(
            completed
                .rng
                .get(crate::hot::RngStream::PotionGeneration)
                .counter,
            u64::MAX
        );

        // An unstarted Eidolon is a future root.  With a full Fairy belt,
        // exact fixed-batch order decides whether Alchemize can procure: an
        // earlier lethal Offering consumes Fairy and opens the slot, whereas
        // an Alchemize that runs first sees the belt still full.  Both bodies
        // still consume their two factory draws exactly once.
        for offering_first in [true, false] {
            let (mut state, catalog) = fixture(u64::MAX - 2, None, None);
            state.hp = 1;
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::FairyInABottle)],
                false,
                false,
                false,
                false,
                true,
            ));
            let offering = catalog.atom(&plain_identity(CardId::Offering, 0)).unwrap();
            let card = HotCard {
                uid: 3,
                atom: offering,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            };
            state.next_card_uid = 4;
            if offering_first {
                state
                    .piles
                    .get_mut(PileId::Exhaust)
                    .make_mut()
                    .insert(0, card);
            } else {
                state.piles.get_mut(PileId::Exhaust).make_mut().push(card);
            }
            let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
            let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
            let completed = apply_action(&state, &catalog, &action).unwrap().state;
            assert!(!completed.history.over);
            assert_eq!(
                completed
                    .rng
                    .get(crate::hot::RngStream::PotionGeneration)
                    .counter,
                u64::MAX
            );
            assert_eq!(
                completed.fanouts.potion_slots()[0].is_some(),
                offering_first,
                "only an earlier Fairy revival opens procurement capacity"
            );
        }

        // The same future-source classification also covers the ordinary
        // multi-action path: a supported held potion can be used before the
        // Eidolon root, after which its exhausted Alchemize child fills the
        // newly vacant slot.
        let (mut state, catalog) = fixture(u64::MAX - 2, None, None);
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::BlockPotion)],
            false,
            false,
            false,
            false,
            true,
        ));
        let after_potion = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert_eq!(after_potion.fanouts.potion_slots(), [None]);
        let completed = apply_action(&after_potion, &catalog, &action)
            .unwrap()
            .state;
        assert!(
            completed.fanouts.potion_slots()[0].is_some(),
            "slots={:?} sozu={} counter={} plays={} over={}",
            completed.fanouts.potion_slots(),
            completed.fanouts.potion_sozu(),
            completed
                .rng
                .get(crate::hot::RngStream::PotionGeneration)
                .counter,
            completed.history.card_plays_finished_combat,
            completed.history.over,
        );
        assert_eq!(
            completed
                .rng
                .get(crate::hot::RngStream::PotionGeneration)
                .counter,
            u64::MAX
        );

        // Both replay payloads are frozen independently: two outer Eidolon
        // bodies each execute two Alchemize bodies, for four exact attempts.
        let (replayed, catalog) = fixture(u64::MAX - 8, Some(1), Some(1));
        let document = HotBoundary::try_to_canonical(&replayed, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let replayed = HotBoundary::from_canonical(&document, &catalog).unwrap();
        let completed = apply_action(&replayed, &catalog, &action).unwrap().state;
        assert_eq!(
            completed
                .rng
                .get(crate::hot::RngStream::PotionGeneration)
                .counter,
            u64::MAX
        );
        assert_eq!(completed.history.card_plays_finished_combat, 6);

        let (overflow, catalog) = fixture(u64::MAX - 7, Some(1), Some(1));
        let before = overflow.clone();
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(&overflow, &catalog, &action, &mut events),
            Err(EngineRefusal::CounterOverflow(
                "Alchemize potion generation counter"
            ))
        );
        assert_eq!(overflow, before);
        assert!(events.is_empty());
    }

    #[test]
    fn alchemize_replay_reserves_only_bodies_that_survive_terminal_panache() {
        fn fixture(monster_hp: i32) -> (HotState, Catalog) {
            let mut builder = CatalogBuilder::new();
            let alchemize = builder
                .intern_reachable(plain_identity(CardId::Alchemize, 0))
                .unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.mark_persistent_action_replay_required();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 3;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.exact_piles = true;
            state.fully_unlocked_card_pool_epochs = true;
            state.reward_card_pool = Some(crate::catalog::RewardPool::Ironclad);
            state.multiplayer_ally_key = 0;
            state.next_card_uid = 2;
            state.rng.set(
                crate::hot::RngStream::PotionGeneration,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: u64::MAX - 2,
                },
            );
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::BlockPotion)],
                true,
                false,
                false,
                false,
                true,
            ));
            state.powers.set(PowerId::Burst, SlotWire::Int, 1);
            state.powers.set(PowerId::Panache, SlotWire::Int, 10);
            let panache_uid = state.fanouts.begin_panache_instance(10).unwrap();
            assert!(state.fanouts.set_panache_instances(&[PanacheInstance {
                uid: panache_uid,
                amount: 10,
                cards_left: 1,
                already_applied: true,
            }]));
            turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            play::hydrate_after_card_played_power_order_for_test(&mut state);
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: alchemize,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            let mut monster = HotMonster::new(MonsterKind::Toadpole, monster_hp);
            monster.max_hp = monster_hp;
            state.monsters_mut().push(monster);
            (state, catalog)
        }

        let action = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };
        let (lethal, catalog) = fixture(5);
        let completed = apply_action(&lethal, &catalog, &action).unwrap().state;
        assert!(completed.history.over);
        assert_eq!(completed.history.card_plays_finished_combat, 1);
        assert_eq!(
            completed
                .rng
                .get(crate::hot::RngStream::PotionGeneration)
                .counter,
            u64::MAX
        );

        let (nonlethal, catalog) = fixture(100);
        let before = nonlethal.clone();
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(&nonlethal, &catalog, &action, &mut events),
            Err(EngineRefusal::CounterOverflow(
                "Alchemize potion generation counter"
            ))
        );
        assert_eq!(nonlethal, before);
        assert!(events.is_empty());
    }

    #[test]
    fn physical_alchemize_seven_body_rng_bound_is_exact() {
        fn fixture(counter: u64) -> (HotState, Catalog) {
            let mut builder = CatalogBuilder::new();
            let alchemize = builder
                .intern_reachable(plain_identity(CardId::Alchemize, 0))
                .unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.mark_persistent_action_replay_required();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 3;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.exact_piles = true;
            state.fully_unlocked_card_pool_epochs = true;
            state.reward_card_pool = Some(crate::catalog::RewardPool::Ironclad);
            state.multiplayer_ally_key = 0;
            state.next_card_uid = 2;
            state.rng.set(
                crate::hot::RngStream::PotionGeneration,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter,
                },
            );
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::BlockPotion)],
                true,
                false,
                false,
                false,
                true,
            ));
            assert!(state.fanouts.set_duplication(1));
            state.powers.set(PowerId::Burst, SlotWire::Int, 1);
            state.powers.set(PowerId::EchoForm, SlotWire::Int, 1);
            turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            let mut physical = state.card_states.get(1);
            physical.set_base_replay_count(Some(3)).unwrap();
            state.card_states.set(1, physical);
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: alchemize,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
            monster.max_hp = 100;
            state.monsters_mut().push(monster);
            (state, catalog)
        }

        let action = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };
        let (state, catalog) = fixture(u64::MAX - 14);
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admission::admit(&document, &state, &catalog), Ok(()));
        let completed = apply_action(&state, &catalog, &action).unwrap().state;
        assert_eq!(completed.history.card_plays_finished_combat, 7);
        assert_eq!(
            completed
                .rng
                .get(crate::hot::RngStream::PotionGeneration)
                .counter,
            u64::MAX
        );

        let (exhausted, catalog) = fixture(u64::MAX - 13);
        let before = exhausted.clone();
        let document = HotBoundary::try_to_canonical(&exhausted, &catalog).unwrap();
        assert!(admission::admit(&document, &exhausted, &catalog).is_err());
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(&exhausted, &catalog, &action, &mut events),
            Err(EngineRefusal::CounterOverflow(
                "Alchemize potion generation counter"
            ))
        );
        assert_eq!(exhausted, before);
        assert!(events.is_empty());
    }

    #[test]
    fn lone_physical_alchemize_does_not_charge_source_less_catalog_writers() {
        fn fixture(counter: u64) -> (HotState, Catalog) {
            let mut builder = CatalogBuilder::new();
            let alchemize = builder
                .intern_reachable(plain_identity(CardId::Alchemize, 0))
                .unwrap();
            // Reachable catalog rows alone are not causal writers for this
            // lone root: neither card is physical or generated by an
            // independent source before Alchemize plays.
            builder
                .intern_reachable(plain_identity(CardId::Burst, 0))
                .unwrap();
            builder
                .intern_reachable(plain_identity(CardId::EchoForm, 0))
                .unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.mark_persistent_action_replay_required();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 3;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.exact_piles = true;
            state.fully_unlocked_card_pool_epochs = true;
            state.reward_card_pool = Some(crate::catalog::RewardPool::Ironclad);
            state.multiplayer_ally_key = 0;
            state.next_card_uid = 2;
            state.rng.set(
                crate::hot::RngStream::PotionGeneration,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter,
                },
            );
            assert!(state.fanouts.set_potion_belt(
                vec![Some(PotionId::BlockPotion)],
                true,
                false,
                false,
                false,
                true,
            ));
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: alchemize,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
            monster.max_hp = 100;
            state.monsters_mut().push(monster);
            (state, catalog)
        }

        let action = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::NONE,
        };
        let (state, catalog) = fixture(u64::MAX - 2);
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admission::admit(&document, &state, &catalog), Ok(()));
        let completed = apply_action(&state, &catalog, &action).unwrap().state;
        assert_eq!(completed.history.card_plays_finished_combat, 1);
        assert_eq!(
            completed
                .rng
                .get(crate::hot::RngStream::PotionGeneration)
                .counter,
            u64::MAX
        );

        let (state, catalog) = fixture(u64::MAX - 1);
        let before = state.clone();
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert!(admission::admit(&document, &state, &catalog).is_err());
        let mut events = Vec::new();
        assert_eq!(
            apply_action_into(&state, &catalog, &action, &mut events),
            Err(EngineRefusal::CounterOverflow(
                "Alchemize potion generation counter"
            ))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn active_colorless_splash_option_reserves_heirloom_before_alchemize() {
        let mut builder = CatalogBuilder::new();
        for id in crate::content_tables::ENTROPY_COLORLESS_TRANSFORM_POOL_V109
            .into_iter()
            .chain(crate::content_tables::SPLASH_ATTACK_POOL_V109)
        {
            builder.intern(plain_identity(id, 0)).unwrap();
        }
        let alchemize = builder
            .intern_reachable(plain_identity(CardId::Alchemize, 0))
            .unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let seed = (0..4096)
            .find(|seed| {
                let mut rng = crate::rng::Xoshiro256StarStar::from_seed(*seed);
                let mut pool = crate::content_tables::ENTROPY_COLORLESS_TRANSFORM_POOL_V109;
                rng.shuffle(&mut pool).unwrap();
                pool[..3].contains(&CardId::Splash)
            })
            .unwrap();
        let seeded = crate::rng::Xoshiro256StarStar::from_seed(seed);

        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.reward_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.entropy_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.fully_unlocked_card_pool_epochs = true;
        give_a_real_root_s_current_build_provenance(&mut state);
        state.next_card_uid = 2;
        state.rng.set(
            crate::hot::RngStream::Generation,
            crate::hot::RngStreamState {
                words: seeded.words,
                counter: seeded.counter,
            },
        );
        state.rng.set(
            crate::hot::RngStream::PotionGeneration,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.rng.set(
            crate::hot::RngStream::Sel,
            crate::hot::RngStreamState {
                words: [5, 6, 7, 8],
                counter: 0,
            },
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::ColorlessPotion), None],
            false,
            false,
            false,
            false,
            true,
        ));
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: alchemize,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });

        // `PotionGeneration` headroom, DERIVED rather than written down.
        // `admission.rs` reserves `required_draws = 2 * max_future_bodies`,
        // and `max_future_bodies` here is the ordinary Alchemize body, plus
        // the Duplicator this belt's vacant slot can still procure, plus one
        // for each reachable replay writer among `Burst` and `EchoForm` —
        // which Entropy's foreign-card closure now reaches (#2637).
        //
        // The count is taken off the catalog the BOUNDARY rebuilds, not off
        // the hand-built one above: the parked root's ActionReplay predecessor
        // is re-admitted against `catalog_from_canonical`'s own closure walk,
        // and that is the walk the expanded closure widened. Counting instead
        // of writing `8` is what keeps the fixture tracking the closure — a
        // literal would quietly stop matching the reachable set — and the
        // one-draw-short assertion at the end of this test pins the number as
        // the exact boundary rather than merely a large enough one.
        let reservation_for = |catalog: &Catalog| -> u64 {
            // ordinary body + the Duplicator the vacant slot can procure,
            // plus one per reachable replay writer.
            let bodies = 2 + [CardId::Burst, CardId::EchoForm]
                .into_iter()
                .filter(|id| catalog.is_reachable(plain_identity(*id, 0)))
                .count() as u64;
            2 * bodies
        };
        let reservation_probe = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let reservation_catalog = HotBoundary::catalog_from_canonical(&reservation_probe).unwrap();
        let potion_generation_headroom = reservation_for(&reservation_catalog);
        // The hand-built catalog stands in for a session's: keep what the
        // document's own closure keeps (#3660).
        let catalog =
            catalog.with_session_bookkeeping_for_test(reservation_catalog.session_bookkeeping());
        state.rng.set(
            crate::hot::RngStream::PotionGeneration,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: u64::MAX - potion_generation_headroom,
            },
        );

        let initial = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admission::admit(&initial, &state, &catalog), Ok(()));
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        let finish = parked
            .pending
            .as_deref()
            .and_then(|pending| pending.generation_potion_record(&parked.frames))
            .unwrap();
        assert!(
            finish
                .generation_options()
                .any(|atom| matches!(catalog.spec(atom).unwrap().identity.id, CardId::Splash))
        );
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(
            admission::admit(&document, &rebuilt, &rebuilt_catalog),
            Ok(())
        );

        // One draw short of the reservation must refuse, which is what makes
        // the derivation checkable rather than merely generous. This root is
        // admitted against the hand-built `catalog`, whose narrower closure
        // reaches neither replay writer, so its own boundary is the same
        // `u64::MAX - 3` this assertion always carried — derived here instead
        // of written down, from the same expression that produced the wider
        // reservation above.
        let mut insufficient = state;
        insufficient.rng.set(
            crate::hot::RngStream::PotionGeneration,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: u64::MAX - (reservation_for(&catalog) - 1),
            },
        );
        let document = HotBoundary::try_to_canonical(&insufficient, &catalog).unwrap();
        assert!(admission::admit(&document, &insufficient, &catalog).is_err());
    }

    #[test]
    fn distilled_full_hand_stratagem_consumes_exactly_three_gather_attempts() {
        let mut builder = CatalogBuilder::new();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.monsters_mut().push(crate::hot::HotMonster::new(
            crate::ids::MonsterKind::Toadpole,
            100,
        ));
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 13;
        state.exact_piles = true;
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        state.rng.set(
            crate::hot::RngStream::Rng,
            crate::hot::RngStreamState {
                words: [17, 18, 19, 20],
                counter: 0,
            },
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((1..=10).map(|uid| HotCard {
                uid,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));
        state
            .piles
            .get_mut(PileId::Discard)
            .make_mut()
            .extend((11..=12).map(|uid| HotCard {
                uid,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));

        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(parked.pending.is_some());
        let batch = match parked.frames.as_slice()[2] {
            crate::frame::Frame::FrozenAutoBatch { record } => {
                parked.frames.frozen_auto_batch(record).unwrap()
            }
            _ => unreachable!(),
        };
        assert_eq!((batch.cursor, batch.len(), batch.gather_target), (1, 0, 3));
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &rebuilt, &rebuilt_catalog), Ok(()));
        let selected = legal_actions(&rebuilt, &rebuilt_catalog)[0];
        let successor = apply_action(&rebuilt, &rebuilt_catalog, &selected)
            .unwrap()
            .state;
        assert!(successor.pending.is_none() && successor.frames.is_empty());
        assert_eq!(successor.piles.get(PileId::Hand).len(), 10);
        assert_eq!(successor.history.card_plays_finished_combat, 1);
        assert_eq!(successor.rng.get(crate::hot::RngStream::Rng).counter, 1);
        assert_eq!(successor.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn orobic_acid_dispatches_directly_and_generates_attack_skill_power_in_order() {
        let mut builder = CatalogBuilder::new();
        for id in crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109
            .into_iter()
            .chain(crate::content_tables::GENERATION_POTION_SKILL_POOL_V1091)
            .chain(crate::content_tables::GENERATION_POTION_POWER_POOL_V1091)
        {
            builder.intern(plain_identity(id, 0)).unwrap();
        }
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 10;
        state.exact_piles = true;
        state.reward_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.entropy_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            crate::hot::RngStream::Generation,
            crate::hot::RngStreamState {
                words: [9, 8, 7, 6],
                counter: 0,
            },
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::OrobicAcid)],
            false,
            false,
            false,
            false,
            true,
        ));
        let filler = catalog
            .atom(&plain_identity(
                crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109[0],
                0,
            ))
            .unwrap();
        state
            .piles
            .get_mut(PileId::Hand)
            .make_mut()
            .extend((1..=9).map(|uid| HotCard {
                uid,
                atom: filler,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            }));

        let successor = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(successor.pending.is_none());
        assert!(successor.frames.is_empty());
        assert_eq!(successor.fanouts.potion_slots(), [None]);
        assert_eq!(
            successor.rng.get(crate::hot::RngStream::Generation).counter,
            75
        );
        assert_eq!(successor.history.owner_generated_cards_combat, 3);
        assert_eq!(successor.next_card_uid, 13);
        let hand = successor.piles.get(PileId::Hand).as_slice();
        let discard = successor.piles.get(PileId::Discard).as_slice();
        assert_eq!((hand.len(), discard.len()), (10, 2));
        let generated = hand[9..].iter().chain(discard).copied().collect::<Vec<_>>();
        assert_eq!(
            generated.iter().map(|card| card.uid).collect::<Vec<_>>(),
            [10, 11, 12]
        );
        let specs = generated
            .iter()
            .map(|card| catalog.spec(card.atom).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            specs
                .iter()
                .map(|spec| spec.identity.id)
                .collect::<Vec<_>>(),
            [CardId::MoltenFist, CardId::SecondWind, CardId::StoneArmor]
        );
        assert!(specs[0].is_attack);
        assert!(specs[1].is_skill);
        assert!(specs[2].is_power);
        assert!(generated.iter().all(|card| {
            successor
                .card_states
                .get(card.uid)
                .free_star_cost_this_turn_or_played_rows
                == 1
        }));

        let mut exact_uid_boundary = state.clone();
        exact_uid_boundary.next_card_uid = u32::MAX - 3;
        let exact_uid_successor = apply_action(
            &exact_uid_boundary,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert_eq!(exact_uid_successor.next_card_uid, u32::MAX);

        let mut overflow = state.clone();
        overflow.next_card_uid = u32::MAX - 2;
        let overflow_before = overflow.clone();
        let mut overflow_events = vec![Event::TurnEnded { turn: 99 }];
        assert_eq!(
            apply_action_into(
                &overflow,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
                &mut overflow_events,
            ),
            Err(EngineRefusal::CounterOverflow("next_card_uid"))
        );
        assert_eq!(overflow, overflow_before);
        assert!(overflow_events.is_empty());

        let mut ending_entry = state.clone();
        ending_entry.history.over = true;
        let mut ending_events = Vec::new();
        potions::use_potion(
            &mut ending_entry,
            &catalog,
            0,
            None,
            PotionId::OrobicAcid,
            &mut ending_events,
        )
        .unwrap();
        assert_eq!(
            ending_entry
                .rng
                .get(crate::hot::RngStream::Generation)
                .counter,
            75
        );
        assert_eq!(ending_entry.history.owner_generated_cards_combat, 0);
        assert_eq!(ending_entry.next_generated_hook_uid, 0);
        assert_eq!(ending_entry.next_card_uid, 10);
        assert_eq!(ending_entry.piles.get(PileId::Hand).len(), 9);
        assert!(ending_entry.piles.get(PileId::Discard).is_empty());
        assert_eq!(ending_entry.fanouts.potion_slots(), [None]);
        assert!(ending_entry.pending.is_none() && ending_entry.frames.is_empty());
    }

    #[test]
    fn generation_potion_pools_cannot_reach_a_resumable_generated_hook_in_part_d() {
        let pools: [(&[CardId], Option<&str>); 4] = [
            (
                &crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109,
                Some("attack"),
            ),
            (
                &crate::content_tables::GENERATION_POTION_SKILL_POOL_V1091,
                Some("skill"),
            ),
            (
                &crate::content_tables::GENERATION_POTION_POWER_POOL_V1091,
                Some("power"),
            ),
            (
                &crate::content_tables::ENTROPY_COLORLESS_TRANSFORM_POOL_V109,
                None,
            ),
        ];
        let mut builder = CatalogBuilder::new();
        for (pool, _) in pools {
            for &id in pool {
                builder.intern(plain_identity(id, 0)).unwrap();
            }
        }
        let catalog = builder.build();

        for (pool, exact_type) in pools {
            for &id in pool {
                let spec = catalog
                    .spec(catalog.atom(&plain_identity(id, 0)).unwrap())
                    .unwrap();
                assert!(!spec.is_status && !spec.is_status_curse);
                assert_ne!(id, CardId::Wither);
                match exact_type {
                    Some("attack") => assert!(spec.is_attack),
                    Some("skill") => assert!(spec.is_skill),
                    Some("power") => assert!(spec.is_power),
                    None => assert!(spec.is_attack || spec.is_skill || spec.is_power),
                    _ => unreachable!(),
                }
            }
        }
        assert!(admission::IMPLEMENTED_RELICS.contains(&crate::ids::RelicId::RelicGremlinHorn));
        assert!(admission::IMPLEMENTED_RELICS.contains(&crate::ids::RelicId::RelicRegalite));
    }

    #[test]
    fn r52d2k_entropic_vicious_producer_census_classifies_exact_plain_and_fused_rows() {
        use std::collections::BTreeSet;

        let (state, seed_catalog) = entropic_fixture(
            vec![Some(PotionId::EntropicBrew), None],
            false,
            [1, 2, 3, 4],
            0,
        );
        let document = HotBoundary::try_to_canonical(&state, &seed_catalog).unwrap();
        let l0_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let ids = l0_catalog
            .reachable_specs()
            .map(|spec| spec.identity.id)
            .collect::<BTreeSet<_>>();
        let mut builder = CatalogBuilder::new();
        for id in ids {
            for upgrade in 0..=1 {
                if crate::content_tables::card_row(id, upgrade).is_some() {
                    builder
                        .intern_reachable(plain_identity(id, upgrade))
                        .unwrap();
                }
            }
        }
        let catalog = builder.build();
        let mut resumable = BTreeSet::new();
        let mut transaction_only = BTreeSet::new();
        for spec in catalog.reachable_specs().filter(|spec| {
            crate::engine::play::cardplay_vicious_vulnerable_producer_is_reachable(spec, &catalog)
        }) {
            let plain = spec.row.steps.iter().enumerate().any(|(index, _)| {
                crate::engine::play::cardplay_plain_vulnerable_source_step_is_exact(spec.row, index)
            });
            if plain {
                resumable.insert((spec.identity.id, spec.identity.upgrade));
            } else {
                transaction_only.insert((spec.identity.id, spec.identity.upgrade));
            }
        }
        let levels = |ids: &[CardId]| {
            ids.iter()
                .flat_map(|id| (0..=1).map(move |upgrade| (*id, upgrade)))
                .collect::<BTreeSet<_>>()
        };
        assert_eq!(
            resumable,
            levels(&[
                CardId::Assassinate,
                CardId::BeamCell,
                CardId::Comet,
                CardId::Fear,
                CardId::GammaBlast,
                // #2637 widened this census by exactly these two rows, at both
                // levels. Entropy's transform keys its candidate pool on the
                // ORIGINAL card's class rather than the owner's, so an
                // Ironclad-owned fight's persistent closure now spans all five
                // character pools: `KnowThyPlace` is REGENT and `Putrefy` is
                // NECROBINDER (`CHARACTER_CARD_POOL_ROWS_V1101`), and neither
                // could be reached from the Ironclad-only pool this census was
                // first taken under. Their classification is not asserted here
                // — the loop above derives it from
                // `cardplay_plain_vulnerable_source_step_is_exact` — so this
                // list grows only when the closure does.
                CardId::KnowThyPlace,
                CardId::Putrefy,
                CardId::Taunt,
                CardId::Tremble,
                CardId::Uppercut,
            ])
        );
        assert_eq!(
            transaction_only,
            levels(&[
                CardId::Dominate,
                CardId::HighFive,
                CardId::Misery,
                CardId::MoltenFist,
                CardId::Shockwave,
                CardId::Thunderclap,
            ])
        );
    }

    #[test]
    fn r52d2k_unconverted_vicious_producers_refuse_whole_public_play_atomically() {
        for id in [
            CardId::Dominate,
            CardId::HighFive,
            CardId::Misery,
            CardId::MoltenFist,
            CardId::Shockwave,
            CardId::Thunderclap,
        ] {
            let mut builder = CatalogBuilder::new();
            let source = builder.intern_reachable(plain_identity(id, 0)).unwrap();
            let seeker = builder
                .intern_reachable(plain_identity(CardId::SeekerStrike, 0))
                .unwrap();
            let strike = builder
                .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
                .unwrap();
            let defend = builder
                .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
                .unwrap();
            let anger = builder
                .intern_reachable(plain_identity(CardId::Anger, 0))
                .unwrap();
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder.mark_live_hellraiser_reachable();
            builder.mark_live_vicious_reachable();
            let catalog = builder.build();

            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 3;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = 6;
            state.exact_piles = true;
            state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
            state.powers.set(PowerId::Vicious, SlotWire::Int, 1);
            assert!(
                state
                    .fanouts
                    .set_after_power_amount_changed_order(&[PowerId::Vicious])
            );
            if id == CardId::HighFive {
                state.fanouts.set_osty(Some((5, 5))).unwrap();
            }
            for stream in [crate::hot::RngStream::Sel, crate::hot::RngStream::Targets] {
                state.rng.set(
                    stream,
                    crate::hot::RngStreamState {
                        words: [1, 2, 3, 4],
                        counter: 0,
                    },
                );
            }
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: source,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            state.piles.get_mut(PileId::Draw).make_mut().extend([
                HotCard {
                    uid: 2,
                    atom: seeker,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 3,
                    atom: strike,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 4,
                    atom: defend,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 5,
                    atom: anger,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
            ]);
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
            monster.max_hp = 1_000;
            monster.loop_pos = 2;
            monster.slot = 6;
            monster.uid = 9;
            if matches!(id, CardId::Misery | CardId::MoltenFist) {
                monster.powers.set(PowerId::Vuln, SlotWire::Int, 1);
            }
            if id == CardId::Misery {
                monster
                    .misery_debuff_order
                    .push(crate::hot::MiseryToken::Vuln);
            }
            let mut peer = HotMonster::new(MonsterKind::Toadpole, 1_000);
            peer.max_hp = 1_000;
            peer.loop_pos = 2;
            peer.slot = 7;
            peer.uid = 10;
            state.monsters = std::sync::Arc::new(vec![monster, peer]);
            let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            assert_eq!(admit(&document, &state, &catalog), Ok(()), "{id:?}");
            let action = legal_actions(&state, &catalog)
                .into_iter()
                .find(|action| matches!(action, Action::Play { uid: 1, .. }))
                .unwrap_or_else(|| panic!("{id:?} must be a legal public Play"));
            let before = state.clone();
            let mut events = vec![Event::TurnEnded { turn: 99 }];
            assert_eq!(
                apply_action_into(&state, &catalog, &action, &mut events),
                Err(EngineRefusal::ContinuationNotModeled),
                "{id:?} must refuse when its fused command would park Vicious"
            );
            assert_eq!(state, before, "{id:?} mutated the caller-owned state");
            assert!(events.is_empty(), "{id:?} leaked a fused-command prefix");
        }
    }

    #[test]
    fn r52d2k_distilled_fused_vicious_candidate_refuses_whole_potion_atomically() {
        let mut builder = CatalogBuilder::new();
        let shockwave = builder
            .intern_reachable(plain_identity(CardId::Shockwave, 0))
            .unwrap();
        let seeker = builder
            .intern_reachable(plain_identity(CardId::SeekerStrike, 0))
            .unwrap();
        let strike = builder
            .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let anger = builder
            .intern_reachable(plain_identity(CardId::Anger, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_live_hellraiser_reachable();
        builder.mark_live_vicious_reachable();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 8;
        state.exact_piles = true;
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        state.powers.set(PowerId::Vicious, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::Vicious])
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::DistilledChaos)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.rng.set(
            crate::hot::RngStream::Targets,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: shockwave,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: seeker,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 6,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 7,
                atom: anger,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);
        let before = state.clone();
        let mut events = vec![Event::TurnEnded { turn: 99 }];
        assert_eq!(
            apply_action_into(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
                &mut events,
            ),
            Err(EngineRefusal::ContinuationNotModeled)
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
        assert_eq!(
            state.fanouts.potion_slots(),
            [Some(PotionId::DistilledChaos)]
        );
        assert_eq!(state.piles.get(PileId::Draw).as_slice()[0].uid, 1);
        assert_eq!(state.rng.get(crate::hot::RngStream::Targets).counter, 0);
    }

    #[test]
    fn generation_and_entropic_pool_orders_and_hashes_are_exact() {
        use sha2::{Digest, Sha256};

        fn assert_card_hash(pool: &[CardId], expected: &str) {
            let bare_ids = pool
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(
                format!("{:x}", Sha256::digest(bare_ids.as_bytes())),
                expected
            );
        }

        fn assert_potion_hash(pool: &[PotionId], expected: &str) {
            let bare_ids = pool
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            assert_eq!(
                format!("{:x}", Sha256::digest(bare_ids.as_bytes())),
                expected
            );
        }

        for (pool, len, expected) in [
            (
                &crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109[..],
                33,
                "d78784af032c58f1a606681cf5fba41ea600ab7b3bb58ae5e08205ca51caa61b",
            ),
            (
                &crate::content_tables::GENERATION_POTION_SKILL_POOL_V1091[..],
                27,
                "812eadc1cb7972a99e419e9ed1cc62aef070cc63945b0ed92d276101d629547b",
            ),
            (
                &crate::content_tables::GENERATION_POTION_POWER_POOL_V1091[..],
                18,
                "ec7e18a7073b7edfb9378c43f9f141f6ef12086ba5d81e6a1d6e0c372ca080f9",
            ),
            (
                &crate::content_tables::ENTROPY_COLORLESS_TRANSFORM_POOL_V109[..],
                50,
                "54c057ab0294527b95c18cfa4ce9bc65adce9c2e65e27c5485bfa969ed01e455",
            ),
        ] {
            assert_eq!(pool.len(), len);
            assert_card_hash(pool, expected);
        }

        for (pool, expected) in [
            (
                &potions::ENTROPIC_COMMON_POOL[..],
                "7c5b4c31079c531ec9ba71bc2fcb319355fb0859ef56fe0f8f207d4ae1852ab4",
            ),
            (
                &potions::ENTROPIC_UNCOMMON_POOL[..],
                "3eaee1fb0e4bfd2db6a2aafbdb5e9cb4e9e6192816191f62d6b735e547cd1d80",
            ),
            (
                &potions::ENTROPIC_RARE_POOL[..],
                "c9f1a19c6da64773ef05a2cb62570cf83798a0b3075d4e78705b20901779b2ef",
            ),
        ] {
            assert_eq!(pool.len(), 16);
            assert_potion_hash(pool, expected);
        }

        let generate_all = [
            PotionId::BloodPotion,
            PotionId::SoldiersStew,
            PotionId::Ashwater,
            PotionId::AttackPotion,
            PotionId::BeetleJuice,
            PotionId::BlessingOfTheForge,
            PotionId::BlockPotion,
            PotionId::BottledPotential,
            PotionId::Clarity,
            PotionId::ColorlessPotion,
            PotionId::CureAll,
            PotionId::DexterityPotion,
            PotionId::DistilledChaos,
            PotionId::DropletOfPrecognition,
            PotionId::Duplicator,
            PotionId::EnergyPotion,
            PotionId::EntropicBrew,
            PotionId::ExplosiveAmpoule,
            PotionId::FairyInABottle,
            PotionId::FirePotion,
            PotionId::FlexPotion,
            PotionId::Fortifier,
            PotionId::FruitJuice,
            PotionId::FyshOil,
            PotionId::GamblersBrew,
            PotionId::GigantificationPotion,
            PotionId::HeartOfIron,
            PotionId::LiquidBronze,
            PotionId::LiquidMemories,
            PotionId::LuckyTonic,
            PotionId::MazalethsGift,
            PotionId::OrobicAcid,
            PotionId::PotionOfBinding,
            PotionId::PowderedDemise,
            PotionId::PowerPotion,
            PotionId::RadiantTincture,
            PotionId::RegenPotion,
            PotionId::ShacklingPotion,
            PotionId::ShipInABottle,
            PotionId::SkillPotion,
            PotionId::SneckoOil,
            PotionId::SpeedPotion,
            PotionId::StableSerum,
            PotionId::StrengthPotion,
            PotionId::SwiftPotion,
            PotionId::TouchOfInsanity,
            PotionId::VulnerablePotion,
            PotionId::WeakPotion,
        ];
        assert_eq!(generate_all.len(), 48);
        assert_potion_hash(
            &generate_all,
            "c45a0215824068f04cde4f21a5f3517de04debea77ec72d4d5b5e566489de1e8",
        );
        let in_combat = generate_all
            .into_iter()
            .filter(|potion| {
                !matches!(
                    potion,
                    PotionId::FairyInABottle | PotionId::FruitJuice | PotionId::RegenPotion
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(in_combat.len(), 45);
        assert_potion_hash(
            &in_combat,
            "bb52833983e8e0160953eb6c6fdcb0a68b2a0342e96bdce17a03221fa142eb65",
        );
        assert_potion_hash(
            &potions::ALCHEMIZE_UNCOMMON_POOL,
            "1f25b7fa85999f5b1008fca6913e2d959cfa1f52eeccaebc5aee8fe53f865b90",
        );
        assert_potion_hash(
            &potions::ALCHEMIZE_RARE_POOL,
            "9ff828f94f9bfc3f21a87b1410843db551b3c03db208cce6aabe608ff3b85f89",
        );
    }

    #[test]
    fn entropic_under_sozu_draws_once_without_success_only_closure() {
        let (state, _complete_catalog) =
            entropic_fixture(vec![Some(PotionId::EntropicBrew)], true, [1, 2, 3, 4], 7);
        // Sozu makes every generated identity unreachable.  In particular,
        // neither the four generated-card pools nor Distilled's downstream
        // card/RNG closure is needed for this one mandatory failed attempt.
        let empty_catalog = CatalogBuilder::new().build();
        assert_eq!(
            potions::manual_part_d_prerequisites_are_exact(
                &state,
                &empty_catalog,
                PotionId::EntropicBrew,
            ),
            Ok(())
        );
        let document = HotBoundary::try_to_canonical(&state, &empty_catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &rebuilt, &rebuilt_catalog), Ok(()));
        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        assert!(legal_actions(&rebuilt, &rebuilt_catalog).contains(&action));
        let successor = apply_action(&rebuilt, &rebuilt_catalog, &action)
            .unwrap()
            .state;
        assert_eq!(successor.fanouts.potion_slots(), [None]);
        assert_eq!(
            successor
                .rng
                .get(crate::hot::RngStream::PotionGeneration)
                .counter,
            9
        );
        assert!(successor.pending.is_none() && successor.frames.is_empty());

        let (overflow, overflow_catalog) = entropic_fixture(
            vec![Some(PotionId::EntropicBrew)],
            true,
            [1, 2, 3, 4],
            u64::MAX - 1,
        );
        let before = overflow.clone();
        let mut events = vec![Event::TurnEnded { turn: 99 }];
        assert_eq!(
            apply_action_into(&overflow, &overflow_catalog, &action, &mut events),
            Err(EngineRefusal::CounterOverflow("potion RNG counter"))
        );
        assert_eq!(overflow, before);
        assert!(events.is_empty());

        let (mut full, full_catalog) = entropic_fixture(
            vec![Some(PotionId::StrengthPotion), Some(PotionId::SpeedPotion)],
            false,
            [1, 2, 3, 4],
            12,
        );
        let before_slots = full.fanouts.potion_slots().to_vec();
        potions::entropic_brew(&mut full, &full_catalog, &mut Vec::new()).unwrap();
        assert_eq!(full.fanouts.potion_slots(), before_slots);
        assert_eq!(
            full.rng
                .get(crate::hot::RngStream::PotionGeneration)
                .counter,
            14
        );
    }

    #[test]
    fn r52e_sozu_entropic_top_parks_roundtrips_and_resumes_once() {
        let (mut source, source_catalog) = entropic_fixture_with_relics(
            vec![Some(PotionId::EntropicBrew)],
            true,
            [1, 2, 3, 4],
            0,
            &[RelicId::RelicUnceasingTop],
        );
        source.fanouts.set_osty(Some((3, 5))).unwrap();
        let atom = source_catalog
            .atom(&plain_identity(
                crate::content_tables::GENERATION_POTION_SKILL_POOL_V1091[0],
                0,
            ))
            .unwrap();
        source.next_card_uid = 3;
        source.fanouts.set_unceasing_top(true);
        source.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        source.rng.set(
            crate::hot::RngStream::Rng,
            crate::hot::RngStreamState {
                words: [9, 8, 7, 6],
                counter: 0,
            },
        );
        source.piles.get_mut(PileId::Discard).make_mut().extend([
            HotCard {
                uid: 1,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let predecessor = HotBoundary::try_to_canonical(&source, &source_catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&predecessor).unwrap();
        let state = HotBoundary::from_canonical(&predecessor, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));

        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::PotionFinish { .. },
                crate::frame::Frame::Draw { .. }
            ]
        ));
        assert!(parked.pending.is_some());
        assert_eq!(parked.fanouts.potion_slots(), [None]);
        assert_eq!(
            parked
                .rng
                .get(crate::hot::RngStream::PotionGeneration)
                .counter,
            2
        );
        let parked_document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&parked_document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&parked_document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&parked_document, &rebuilt, &rebuilt_catalog), Ok(()));
        let answer = legal_actions(&rebuilt, &rebuilt_catalog)[0];
        let completed = apply_action(&rebuilt, &rebuilt_catalog, &answer)
            .unwrap()
            .state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.fanouts.potion_slots(), [None]);
        assert_eq!(
            completed
                .rng
                .get(crate::hot::RngStream::PotionGeneration)
                .counter,
            2
        );
    }

    #[test]
    fn random_potion_rarity_thresholds_are_binary32_inclusive() {
        let rare = f32::from_bits(0x3d_cc_cc_cd);
        let uncommon = f32::from_bits(0x3e_b3_33_33);
        for (roll, expected) in [
            (f32::from_bits(rare.to_bits() - 1), "rare"),
            (rare, "rare"),
            (f32::from_bits(rare.to_bits() + 1), "uncommon"),
            (f32::from_bits(uncommon.to_bits() - 1), "uncommon"),
            (uncommon, "uncommon"),
            (f32::from_bits(uncommon.to_bits() + 1), "common"),
        ] {
            let selected = potions::random_potion_pool_for_roll(roll, false);
            match expected {
                "rare" => assert_eq!(selected, potions::ENTROPIC_RARE_POOL),
                "uncommon" => assert_eq!(selected, potions::ENTROPIC_UNCOMMON_POOL),
                "common" => assert_eq!(selected, potions::ENTROPIC_COMMON_POOL),
                _ => unreachable!(),
            }
        }
        assert_eq!(
            potions::random_potion_pool_for_roll(rare, true),
            potions::ALCHEMIZE_RARE_POOL
        );
        assert_eq!(
            potions::random_potion_pool_for_roll(uncommon, true),
            potions::ALCHEMIZE_UNCOMMON_POOL
        );
    }

    #[test]
    fn r52e_non_sozu_entropic_fills_every_null_slot_with_two_draws_each() {
        let (mut state, catalog) = entropic_fixture(
            vec![Some(PotionId::EntropicBrew), None],
            false,
            [1, 2, 3, 4],
            0,
        );
        for stream in crate::hot::RngStream::ALL {
            if state.rng.is_vacant(stream) {
                state.rng.set(
                    stream,
                    crate::hot::RngStreamState {
                        words: [11, 12, 13, 14],
                        counter: 0,
                    },
                );
            }
        }
        let action = Action::UsePotion {
            slot: 0,
            target: None,
        };
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
        assert_eq!(admit(&document, &state, &catalog), Ok(()));
        assert_eq!(
            potions::manual_part_d_prerequisites_are_exact(
                &state,
                &catalog,
                PotionId::EntropicBrew,
            ),
            Ok(())
        );
        assert!(legal_actions(&state, &catalog).contains(&action));
        let successor = apply_action(&state, &catalog, &action).unwrap().state;
        assert!(successor.fanouts.potion_slots().iter().all(Option::is_some));
        assert_eq!(
            successor
                .rng
                .get(crate::hot::RngStream::PotionGeneration)
                .counter,
            4
        );
    }

    #[test]
    fn orobic_pillar_juggernaut_terminal_hook_is_synchronous_without_refused_relics() {
        let mut builder = CatalogBuilder::new();
        for id in crate::content_tables::INFERNAL_BLADE_ATTACK_POOL_V109
            .into_iter()
            .chain(crate::content_tables::GENERATION_POTION_SKILL_POOL_V1091)
            .chain(crate::content_tables::GENERATION_POTION_POWER_POOL_V1091)
        {
            builder.intern(plain_identity(id, 0)).unwrap();
        }
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.reward_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.entropy_card_pool = Some(crate::catalog::RewardPool::Ironclad);
        state.fully_unlocked_card_pool_epochs = true;
        state.rng.set(
            crate::hot::RngStream::Generation,
            crate::hot::RngStreamState {
                words: [9, 8, 7, 6],
                counter: 0,
            },
        );
        state
            .powers
            .set(PowerId::PillarOfCreation, SlotWire::Int, 1);
        state.powers.set(PowerId::Juggernaut, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_local_generated_power_order(&[PowerId::PillarOfCreation])
        );
        assert!(
            state
                .fanouts
                .set_after_block_gained_order(&[PowerId::Juggernaut])
        );
        state
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 1));
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::OrobicAcid)],
            false,
            false,
            false,
            false,
            true,
        ));

        let successor = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;

        assert!(successor.history.over);
        assert!(successor.pending.is_none() && successor.frames.is_empty());
        assert_eq!(successor.history.owner_generated_cards_combat, 3);
        assert_eq!(successor.next_generated_hook_uid, 3);
        assert_eq!(successor.next_card_uid, 1);
        assert_eq!(successor.piles.get(PileId::Hand).len(), 1);
        assert_eq!(successor.fanouts.potion_slots(), [None]);

        let mut uid_boundary = state.clone();
        uid_boundary.next_card_uid = u32::MAX - 1;
        let boundary_successor = apply_action(
            &uid_boundary,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert!(boundary_successor.history.over);
        assert_eq!(boundary_successor.history.owner_generated_cards_combat, 3);
        assert_eq!(boundary_successor.next_generated_hook_uid, 3);
        assert_eq!(boundary_successor.next_card_uid, u32::MAX);
        assert_eq!(boundary_successor.piles.get(PileId::Hand).len(), 1);
        assert_eq!(
            boundary_successor.piles.get(PileId::Hand).as_slice()[0].uid,
            u32::MAX - 1
        );
        assert_eq!(boundary_successor.fanouts.potion_slots(), [None]);
    }

    #[test]
    fn fruit_and_regen_potions_pin_cap_clamp_wide_heal_and_early_turn_end_tick() {
        let (mut fruit, catalog) = fixture();
        fruit.hp = 999_999_998;
        fruit.max_hp = 999_999_998;
        assert!(fruit.fanouts.set_potion_belt(
            vec![Some(PotionId::FruitJuice)],
            false,
            false,
            false,
            false,
            true,
        ));
        let capped = apply_action(
            &fruit,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert_eq!((capped.hp, capped.max_hp), (999_999_999, 999_999_999));

        let mut malformed = fruit.clone();
        malformed.max_hp = 1_000_000_000;
        let checkpoint = malformed.clone();
        assert!(
            apply_action(
                &malformed,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            )
            .is_err()
        );
        assert_eq!(malformed, checkpoint);

        let (mut regen, catalog) = fixture();
        regen.hp = 1;
        regen.max_hp = 999_999_999;
        assert!(regen.fanouts.set_potion_belt(
            vec![Some(PotionId::RegenPotion)],
            false,
            false,
            false,
            false,
            true,
        ));
        assert!(regen.fanouts.set_regen(i32::MAX - 5));
        let charged = apply_action(
            &regen,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: None,
            },
        )
        .unwrap()
        .state;
        assert_eq!(charged.fanouts.regen(), i32::MAX);
        let mut charged = charged;
        charged.monsters_mut().clear();
        let ticked = apply_action(&charged, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert_eq!(ticked.hp, ticked.max_hp);
        assert_eq!(ticked.fanouts.regen(), i32::MAX - 1);
        let projected = HotBoundary::try_to_canonical(&ticked, &catalog).unwrap();
        assert_eq!(
            projected.player.get("regen"),
            Some(&serde_json::json!(i32::MAX - 1))
        );
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&projected).unwrap();
        let rebuilt = HotBoundary::from_canonical(&projected, &rebuilt_catalog).unwrap();
        assert_eq!(rebuilt.fanouts.regen(), i32::MAX - 1);
    }

    /// #3044: a Red Skull owner round-trips through the boundary with its
    /// ownership cache hydrated, and Regen's turn-end heal and Book Repair
    /// Knife's Doom-kill heal are each a `CreatureCmd.Heal` whose
    /// `AfterCurrentHpChanged` removes the Strength once the owner is past
    /// half HP.
    #[test]
    fn red_skull_hears_regen_and_book_repair_knife_heals() {
        let skull = |hp: i64| {
            let mut document: CanonicalStateV2 = serde_json::from_str(FIXTURE).unwrap();
            document
                .player
                .insert("red_skull".to_owned(), serde_json::json!(true));
            document.player.insert(
                "relics_entering".to_owned(),
                serde_json::json!(["RELIC.BURNING_BLOOD", "RELIC.RED_SKULL"]),
            );
            document
                .player
                .insert("hp".to_owned(), serde_json::json!(hp));
            document
                .player
                .insert("strength".to_owned(), serde_json::json!(3));
            let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
            let state = HotBoundary::from_canonical(&document, &catalog).unwrap();
            admit(&document, &state, &catalog).unwrap();
            assert!(state.fanouts.red_skull_owned());
            (state, catalog)
        };

        // The stale latch never escapes a transition: the boundary refuses it.
        let (mut stale, catalog) = skull(38);
        assert!(HotBoundary::try_to_canonical(&stale, &catalog).is_ok());
        stale.fanouts.set_red_skull_latch_stale(true);
        assert!(HotBoundary::try_to_canonical(&stale, &catalog).is_err());

        let (mut state, catalog) = skull(38);
        assert!(state.fanouts.set_regen(5));
        state.monsters_mut().clear();
        let ticked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert_eq!(ticked.hp, 43);
        assert_eq!(ticked.powers.value(PowerId::Strength), 0);

        let (mut state, catalog) = skull(38);
        state.set_batch_six_deep_relic_ownership(false, false, true, false);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Doom, SlotWire::Int, 99);
        turn::doom_enemy_side_end(&mut state, &catalog, &mut Vec::new()).unwrap();
        assert!(state.monsters[0].hp <= 0);
        assert!(!state.history.over, "the second Toadpole lives");
        assert_eq!(state.hp, 41);
        assert_eq!(state.powers.value(PowerId::Strength), 0);
    }

    #[test]
    fn fairy_is_passive_first_slot_heals_before_death_cleanup_and_force_kill_bypasses_it() {
        let (mut state, catalog) = fixture();
        state.hp = 1;
        assert!(state.fanouts.set_potion_belt(
            vec![
                Some(PotionId::FairyInABottle),
                Some(PotionId::FairyInABottle),
            ],
            false,
            false,
            false,
            false,
            true,
        ));
        assert!(
            !legal_actions(&state, &catalog)
                .iter()
                .any(|action| { matches!(action, Action::UsePotion { slot: 0 | 1, .. }) })
        );
        let checkpoint = state.clone();
        assert_eq!(
            apply_action(
                &state,
                &catalog,
                &Action::UsePotion {
                    slot: 0,
                    target: None,
                },
            )
            .unwrap_err(),
            EngineRefusal::MalformedArgs("passive potion use")
        );
        assert_eq!(state, checkpoint);

        let mut events = Vec::new();
        assert!(damage::damage_player_from_card(&mut state, 10_000, false, &mut events).unwrap());
        assert_eq!(state.hp, (state.max_hp * 30 / 100).max(1));
        assert!(!state.history.over);
        assert_eq!(
            state.fanouts.potion_slots(),
            [None, Some(PotionId::FairyInABottle)]
        );

        let mut force_killed = state.clone();
        force_killed.hp = 1;
        damage::force_kill_player(&mut force_killed, &mut Vec::new()).unwrap();
        assert_eq!(force_killed.hp, 0);
        assert!(force_killed.history.over);
        assert_eq!(
            force_killed.fanouts.potion_slots(),
            [None, Some(PotionId::FairyInABottle)]
        );

        let mut terminal_window = checkpoint;
        terminal_window.hp = 0;
        terminal_window.history.over = true;
        assert!(
            potions::consume_first_fairy_after_lethal(&mut terminal_window, &mut Vec::new())
                .unwrap()
        );
        assert_eq!(
            terminal_window.hp,
            (terminal_window.max_hp * 30 / 100).max(1)
        );
        assert!(terminal_window.history.over);
        assert_eq!(
            terminal_window.fanouts.potion_slots(),
            [None, Some(PotionId::FairyInABottle)]
        );
    }

    #[test]
    fn active_card_lethal_fairy_runs_the_full_wrapper_before_body_resume() {
        let mut builder = CatalogBuilder::new();
        let bloodletting = builder
            .intern_reachable(plain_identity(CardId::Bloodletting, 0))
            .unwrap();
        let seeker = builder
            .intern_reachable(plain_identity(CardId::SeekerStrike, 0))
            .unwrap();
        let strike = builder
            .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let bash = builder
            .intern_reachable(plain_identity(CardId::Bash, 0))
            .unwrap();
        let anger = builder
            .intern_reachable(plain_identity(CardId::Anger, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder
            .set_relics(&[crate::ids::RelicId::RelicUnceasingTop])
            .unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 3;
        state.max_hp = 10;
        state.energy = 0;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 7;
        state.exact_piles = true;
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::FairyInABottle)],
            false,
            true,
            false,
            false,
            true,
        ));
        state.fanouts.set_unceasing_top(true);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        state.rng.set(
            crate::hot::RngStream::Sel,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.rng.set(
            crate::hot::RngStream::Rng,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: bloodletting,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 2,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: seeker,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: bash,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 6,
                atom: anger,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);

        let action = Action::Play {
            uid: 1,
            target: None,
            selection: SelectionRef::new(None),
        };
        let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        admit(&document, &state, &catalog).unwrap();
        assert!(legal_actions(&state, &catalog).contains(&action));
        let successor = apply_action(&state, &catalog, &action).unwrap().state;
        assert_eq!(successor.hp, 3);
        assert_eq!(successor.energy, 2);
        assert_eq!(successor.powers.value(PowerId::Dexterity), 2);
        assert!(successor.fanouts.potion_belt_buckle_applied());
        assert_eq!(successor.fanouts.potion_slots(), [None]);
        assert!(!successor.history.over);
        assert!(successor.pending.is_some());
        assert!(matches!(
            successor.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::CardFinish { .. },
                crate::frame::Frame::Draw { .. },
                crate::frame::Frame::CardFinish { .. },
                crate::frame::Frame::Draw { .. },
                crate::frame::Frame::CardPlay { .. },
            ]
        ));
        assert!(
            successor
                .frames
                .as_slice()
                .iter()
                .all(|frame| !matches!(frame, crate::frame::Frame::PotionFinish { .. }))
        );
        assert_eq!(
            successor.piles.get(PileId::Discard).as_slice(),
            &[
                HotCard {
                    uid: 1,
                    atom: bloodletting,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 2,
                    atom: strike,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
            ]
        );
    }

    #[test]
    fn fairy_top_is_suppressed_by_enemy_phase_and_effect_depth_and_max_hp_suffix_continues() {
        let catalog = CatalogBuilder::new().build();

        // Batch181: CheckForEmptyHand is player-phase 2/3/4 only. A Fairy
        // wrapper reached during enemy phase 5 therefore completes without
        // publishing a Top-owned Draw, even with an empty Hand.
        let mut enemy_phase = HotState::at_defaults();
        enemy_phase.hp = 0;
        enemy_phase.max_hp = 10;
        enemy_phase.player_phase = 5;
        assert!(enemy_phase.fanouts.set_potion_belt(
            vec![Some(PotionId::FairyInABottle)],
            false,
            false,
            false,
            false,
            true,
        ));
        enemy_phase.fanouts.set_unceasing_top(true);
        assert_eq!(
            potions::begin_fairy_wrapper_after_lethal(&mut enemy_phase, &catalog, &mut Vec::new(),)
                .unwrap(),
            potions::FairyWrapperResult::Finished
        );
        assert_eq!(enemy_phase.hp, 3);
        assert!(enemy_phase.frames.is_empty() && enemy_phase.pending.is_none());

        // The same native CheckForEmptyHand call is effect-depth suppressed
        // while another potion wrapper is below Fairy. Its exact outer owner
        // remains installed and no speculative Fairy continuation is needed.
        let mut nested_effect = HotState::at_defaults();
        nested_effect.hp = 0;
        nested_effect.max_hp = 10;
        nested_effect.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        assert!(nested_effect.fanouts.set_potion_belt(
            vec![Some(PotionId::FairyInABottle)],
            false,
            false,
            false,
            false,
            true,
        ));
        nested_effect.fanouts.set_unceasing_top(true);
        nested_effect
            .frames
            .push_potion_finish(&crate::hot::PotionFinishRecord {
                name: PotionId::DistilledChaos,
                stage: crate::frame::PotionFinishStage::Effect,
                body_stage: crate::hot::PotionBodyStage::AfterChild,
                current_uid: None,
                aux: 0,
                candidates: Vec::new(),
                generation_options: Vec::new(),
            })
            .unwrap();
        assert_eq!(
            potions::begin_fairy_wrapper_after_lethal(
                &mut nested_effect,
                &catalog,
                &mut Vec::new(),
            )
            .unwrap(),
            potions::FairyWrapperResult::Finished
        );
        assert!(nested_effect.pending.is_none());
        assert!(matches!(
            nested_effect.frames.as_slice(),
            [crate::frame::Frame::PotionFinish { .. }]
        ));

        // LoseMaxHp awaits the nested lethal result, then still publishes its
        // cap after synchronous Fairy prevention.
        let mut capped = HotState::at_defaults();
        capped.hp = 1;
        capped.max_hp = 10;
        assert!(capped.fanouts.set_potion_belt(
            vec![Some(PotionId::FairyInABottle)],
            false,
            false,
            false,
            false,
            true,
        ));
        damage::lose_player_max_hp_from_card(&mut capped, None, 10, &mut Vec::new()).unwrap();
        assert_eq!((capped.hp, capped.max_hp), (1, 1));
        assert_eq!(capped.fanouts.potion_slots(), [None]);
        assert!(!capped.history.over);
        assert!(capped.frames.is_empty() && capped.pending.is_none());
    }

    #[test]
    fn unceasing_top_card_finish_recurses_through_hellraiser_and_round_trips() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        let pommel = builder
            .intern_reachable(plain_identity(CardId::PommelStrike, 0))
            .unwrap();
        let seeker = builder
            .intern_reachable(plain_identity(CardId::SeekerStrike, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let bash = builder
            .intern_reachable(plain_identity(CardId::Bash, 0))
            .unwrap();
        let anger = builder
            .intern_reachable(plain_identity(CardId::Anger, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder
            .set_relics(&[crate::ids::RelicId::RelicUnceasingTop])
            .unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 6;
        state.exact_piles = true;
        state.rng.set(
            crate::hot::RngStream::Sel,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.rng.set(
            crate::hot::RngStream::Rng,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.rng.set(
            crate::hot::RngStream::Targets,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        assert!(
            state
                .fanouts
                .set_potion_belt(vec![None], false, false, false, false, true,)
        );
        state.fanouts.set_unceasing_top(true);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 0,
            atom: strike,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: pommel,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: seeker,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: bash,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: anger,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);

        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));
        let parked = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 0,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::CardFinish { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. }
                ]
            ),
            "recursive Top frames: {:#?}",
            parked.frames.as_slice()
        );
        assert!(parked.pending.is_some());

        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        assert_eq!(document.continuations[1].frame_type, "CardFinishFrame");
        assert_eq!(document.continuations[3].frame_type, "CardPlayFrame");
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &rebuilt, &rebuilt_catalog), Ok(()));
        assert_eq!(
            HotBoundary::try_to_canonical(&rebuilt, &rebuilt_catalog).unwrap(),
            document
        );

        for (frame, wrong_uid) in [(1_usize, 99_u32), (3, 98)] {
            let mut tampered = document.clone();
            tampered.continuations[frame]
                .fields
                .insert("uid".to_owned(), serde_json::json!(wrong_uid));
            let tampered_catalog = HotBoundary::catalog_from_canonical(&tampered).unwrap();
            assert!(HotBoundary::from_canonical(&tampered, &tampered_catalog).is_err());
        }

        let before = rebuilt.clone();
        let mut events = vec![Event::Reshuffled { cards: 999 }];
        assert!(
            apply_action_into(
                &rebuilt,
                &rebuilt_catalog,
                &Action::Select {
                    answer: SelectionAnswer::OptionIndex(99),
                },
                &mut events,
            )
            .is_err()
        );
        assert_eq!(rebuilt, before);
        assert!(events.is_empty());

        let completed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &Action::Select {
                answer: SelectionAnswer::OptionIndex(0),
            },
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none());
        assert!(completed.frames.is_empty());
    }

    #[test]
    fn unceasing_top_turn_start_is_rooted_under_end_turn() {
        let mut builder = CatalogBuilder::new();
        builder
            .intern_reachable(plain_identity(CardId::SeekerStrike, 0))
            .unwrap();
        builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder
            .set_relics(&[crate::ids::RelicId::RelicUnceasingTop])
            .unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let atom = |id| catalog.atom(&plain_identity(id, 0)).unwrap();
        let mut state = HotState::at_defaults();
        state.hp = 100;
        state.max_hp = 100;
        state.energy = 3;
        state.turn = 1;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 11;
        state.exact_piles = true;
        state.rng.set(
            crate::hot::RngStream::Sel,
            crate::hot::RngStreamState {
                words: [5, 6, 7, 8],
                counter: 0,
            },
        );
        assert!(
            state
                .fanouts
                .set_potion_belt(vec![None], false, false, false, false, true,)
        );
        state.fanouts.set_unceasing_top(true);
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        state.powers.set(PowerId::ToolsOfTheTrade, SlotWire::Int, 5);
        assert!(
            state
                .fanouts
                .set_turn_start_hand_choice_order(&[PowerId::ToolsOfTheTrade])
        );
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: atom(CardId::DefendIronclad),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: atom(CardId::DefendIronclad),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: atom(CardId::DefendIronclad),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: atom(CardId::DefendIronclad),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: atom(CardId::DefendIronclad),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);

        state.rng.set(
            crate::hot::RngStream::Rng,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));
        let parked = apply_action(&state, &catalog, &Action::EndTurn)
            .unwrap()
            .state;
        assert!(
            matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::Draw { .. }
                ]
            ),
            "turn-start Top frames: {:#?}",
            parked.frames.as_slice()
        );
        let crate::frame::Frame::Draw { record } = parked.frames.as_slice()[1] else {
            unreachable!()
        };
        let draw = parked.frames.draw(record).unwrap();
        assert_eq!(draw.caller, crate::hot::DrawCaller::UnceasingTopTurnStart);
        assert!(!draw.from_hand_draw);
        assert_eq!(draw.stage, crate::hot::DrawStage::AfterShuffle);
        assert!(
            parked
                .pending
                .as_deref()
                .and_then(|pending| pending.stratagem_draw_record(&parked.frames))
                .is_some()
        );
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &rebuilt, &rebuilt_catalog), Ok(()));
        let completed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            legal_actions(&rebuilt, &rebuilt_catalog).first().unwrap(),
        )
        .unwrap()
        .state;
        assert!(completed.pending.is_none() && completed.frames.is_empty());
        assert_eq!(completed.player_phase, admission::PHASE_ORDINARY_ACTIONS);
    }

    #[test]
    fn unceasing_top_same_uid_hellraiser_reentry_is_refused_at_admission() {
        fn candidate(ids: &[CardId]) -> (crate::canonical::CanonicalStateV2, HotState, Catalog) {
            let mut builder = CatalogBuilder::new();
            for &id in ids {
                builder.intern_reachable(plain_identity(id, 0)).unwrap();
            }
            builder.intern_monster(MonsterKind::Toadpole).unwrap();
            builder
                .set_relics(&[crate::ids::RelicId::RelicUnceasingTop])
                .unwrap();
            builder.mark_persistent_action_replay_required();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 3;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.next_card_uid = 2;
            state.exact_piles = true;
            assert!(
                state
                    .fanouts
                    .set_potion_belt(vec![None], false, false, false, false, true,)
            );
            state.fanouts.set_unceasing_top(true);
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: catalog.atom(&plain_identity(ids[0], 0)).unwrap(),
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
            monster.max_hp = 1_000;
            monster.loop_pos = 2;
            state.monsters = std::sync::Arc::new(vec![monster]);
            let document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
            (document, state, catalog)
        }

        for (name, ids) in [
            (
                "intrinsic Shining Strike DrawTop",
                vec![CardId::ShiningStrike, CardId::Hellraiser],
            ),
            (
                "future Rebound DrawTop",
                vec![CardId::StrikeIronclad, CardId::Hellraiser, CardId::Rebound],
            ),
            (
                "future Nostalgia DrawTop",
                vec![
                    CardId::StrikeIronclad,
                    CardId::Hellraiser,
                    CardId::Nostalgia,
                ],
            ),
        ] {
            let (document, state, catalog) = candidate(&ids);
            assert!(
                admit(&document, &state, &catalog).unwrap_err().contains(
                    MissingCapability::ArgumentShape("Unceasing Top same-UID Hellraiser reentry",)
                ),
                "{name}"
            );
        }

        let (document, state, catalog) = candidate(&[CardId::StrikeIronclad, CardId::Hellraiser]);
        assert_eq!(admit(&document, &state, &catalog), Ok(()));
    }

    #[test]
    fn targetful_potion_top_recursion_is_replay_rooted_and_resumable() {
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        let seeker = builder
            .intern_reachable(plain_identity(CardId::SeekerStrike, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let bash = builder
            .intern_reachable(plain_identity(CardId::Bash, 0))
            .unwrap();
        let anger = builder
            .intern_reachable(plain_identity(CardId::Anger, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder
            .set_relics(&[crate::ids::RelicId::RelicUnceasingTop])
            .unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 6;
        state.exact_piles = true;
        state.rng.set(
            crate::hot::RngStream::Sel,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.rng.set(
            crate::hot::RngStream::Rng,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        assert!(state.fanouts.set_potion_belt(
            vec![Some(PotionId::FirePotion)],
            false,
            false,
            false,
            false,
            true,
        ));
        state.fanouts.set_unceasing_top(true);
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 1,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 2,
                atom: seeker,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: bash,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: anger,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);

        let predecessor = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
        assert_eq!(admit(&predecessor, &state, &catalog), Ok(()));
        let parked = apply_action(
            &state,
            &catalog,
            &Action::UsePotion {
                slot: 0,
                target: Some(0),
            },
        )
        .unwrap()
        .state;
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::PotionFinish { .. },
                crate::frame::Frame::Draw { .. },
                crate::frame::Frame::CardFinish { .. },
                crate::frame::Frame::Draw { .. },
                crate::frame::Frame::CardPlay { .. }
            ]
        ));
        let crate::frame::Frame::ActionReplay { record } = parked.frames.as_slice()[0] else {
            unreachable!()
        };
        assert!(matches!(
            parked.frames.action_replay(record).unwrap().action,
            crate::hot::ActionReplayRootAction::UsePotion {
                slot: 0,
                target: Some(0)
            }
        ));
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &rebuilt, &rebuilt_catalog), Ok(()));
        let resumed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            legal_actions(&rebuilt, &rebuilt_catalog).first().unwrap(),
        )
        .unwrap()
        .state;
        assert!(resumed.pending.is_none() && resumed.frames.is_empty());
        assert_eq!(resumed.fanouts.potion_slots(), [None]);
    }

    fn assert_vicious_parked_malformed_vectors_refuse(
        document: &crate::canonical::CanonicalStateV2,
        catalog: &Catalog,
    ) {
        let assert_malformed = |candidate: &crate::canonical::CanonicalStateV2, label: &str| {
            assert!(
                HotBoundary::from_canonical(candidate, catalog).is_err(),
                "malformed Vicious continuation admitted: {label}"
            );
        };
        for (field, value) in [
            ("cursor", serde_json::json!(0)),
            ("cursor", serde_json::json!(2)),
            ("power", serde_json::json!("weak")),
            ("amount", serde_json::json!(0)),
            ("applier", serde_json::json!("monster")),
            ("type_for_amount", serde_json::json!(1)),
            ("temporary", serde_json::json!(true)),
            ("owner_identity", serde_json::json!(["monster", 7, 99])),
        ] {
            let mut malformed = document.clone();
            malformed.continuations[2]
                .fields
                .insert(field.to_owned(), value);
            assert_malformed(&malformed, field);
        }
        let mut wrong_positive_amount = document.clone();
        wrong_positive_amount.continuations[2]
            .fields
            .insert("amount".to_owned(), serde_json::json!(1));
        assert_malformed(&wrong_positive_amount, "replayed positive amount");
        let mut wrong_parent_target = document.clone();
        wrong_parent_target.continuations[1]
            .fields
            .insert("target_identity".to_owned(), serde_json::json!([7, 99]));
        assert_malformed(&wrong_parent_target, "replayed parent target");
        let mut listener_order = document.clone();
        listener_order.continuations[2].fields.insert(
            "listeners".to_owned(),
            serde_json::json!(["sleight_of_flesh", "vicious"]),
        );
        assert_malformed(&listener_order, "listener order");
        for (field, value) in [
            ("caller", serde_json::json!("none")),
            ("card_uid", serde_json::json!(99)),
        ] {
            let mut malformed = document.clone();
            malformed.continuations[3]
                .fields
                .insert(field.to_owned(), value);
            assert_malformed(&malformed, field);
        }
        for (field, value) in [
            ("uid", serde_json::json!(99)),
            ("source", serde_json::json!("auto")),
            ("target_identity", serde_json::json!([7, 99])),
        ] {
            let mut malformed = document.clone();
            malformed.continuations[4]
                .fields
                .insert(field.to_owned(), value);
            assert_malformed(&malformed, field);
        }
        let mut wrong_child_fallback = document.clone();
        wrong_child_fallback.continuations[4]
            .fields
            .insert("target_identity".to_owned(), serde_json::json!([7, 9, 0]));
        assert_malformed(&wrong_child_fallback, "replayed child fallback");
        let mut wrong_root_uid = document.clone();
        wrong_root_uid.continuations[0]
            .fields
            .get_mut("action")
            .unwrap()["uid"] = serde_json::json!(2);
        assert_malformed(&wrong_root_uid, "replayed root uid");
        let mut duplicate_identity = document.clone();
        duplicate_identity
            .monsters
            .push(duplicate_identity.monsters[0].clone());
        duplicate_identity.continuations[1]
            .fields
            .insert("target_identity".to_owned(), serde_json::json!([7, 9, 1]));
        duplicate_identity.continuations[2].fields.insert(
            "owner_identity".to_owned(),
            serde_json::json!(["monster", 7, 9, 1]),
        );
        duplicate_identity.continuations[4]
            .fields
            .insert("target_identity".to_owned(), serde_json::json!([7, 9, 1]));
        assert_malformed(&duplicate_identity, "ambiguous replayed target fallback");
        let mut wrong_pile = document.clone();
        let child = wrong_pile.piles.get_mut("play").unwrap().pop().unwrap();
        wrong_pile.piles.get_mut("discard").unwrap().push(child);
        assert_malformed(&wrong_pile, "Hellraiser child source pile");
        let mut reordered_options = document.clone();
        reordered_options.player.get_mut("pending").unwrap()[2][4][1]
            .as_array_mut()
            .unwrap()
            .swap(0, 1);
        assert_malformed(&reordered_options, "Seeker option order");
        let mut duplicated_option = document.clone();
        let first = duplicated_option.player["pending"][2][4][1][0].clone();
        duplicated_option.player.get_mut("pending").unwrap()[2][4][1][1] = first;
        assert_malformed(&duplicated_option, "duplicate Seeker option uid");
        let mut vanished_option = document.clone();
        vanished_option.player.get_mut("pending").unwrap()[2][4][1]
            .as_array_mut()
            .unwrap()
            .pop();
        assert_malformed(&vanished_option, "vanished Seeker option uid");
    }

    #[test]
    fn vulnerable_vicious_hellraiser_seeker_roundtrips_and_finishes_peers_once() {
        let mut builder = CatalogBuilder::new();
        let bash = builder
            .intern_reachable(plain_identity(CardId::Bash, 0))
            .unwrap();
        let seeker = builder
            .intern_reachable(plain_identity(CardId::SeekerStrike, 0))
            .unwrap();
        let strike = builder
            .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let anger = builder
            .intern_reachable(plain_identity(CardId::Anger, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 7;
        state.exact_piles = true;
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.powers.set(PowerId::Vicious, SlotWire::Int, 1);
        state.powers.set(PowerId::SleightOfFlesh, SlotWire::Int, 3);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(
                    &[PowerId::Vicious, PowerId::SleightOfFlesh,]
                )
        );
        for stream in [crate::hot::RngStream::Sel, crate::hot::RngStream::Targets] {
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: bash,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 2,
                atom: seeker,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: anger,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 6,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.loop_pos = 2;
        monster.slot = 7;
        monster.uid = 9;
        state.monsters = std::sync::Arc::new(vec![monster]);

        let parked_transition = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap();
        assert_eq!(
            parked_transition.events,
            vec![
                Event::CardPlayed {
                    uid: 1,
                    atom: bash,
                    energy: 2,
                },
                Event::MonsterDamaged {
                    uid: 9,
                    blocked: 0,
                    unblocked: 8,
                    hp: 92,
                },
                Event::PowerChanged {
                    subject: Subject::Monster(9),
                    power: PowerId::Vuln,
                    amount: 2,
                },
                Event::CardDrawn { uid: 2 },
                Event::CardPlayed {
                    uid: 2,
                    atom: seeker,
                    energy: 0,
                },
                Event::MonsterDamaged {
                    uid: 9,
                    blocked: 0,
                    unblocked: 13,
                    hp: 79,
                },
            ]
        );
        let parked = parked_transition.state;
        assert!(parked.pending.is_some());
        let pending = parked.pending.as_deref().unwrap();
        let pending_play = parked.frames.card_play(pending.frame_record).unwrap();
        assert_eq!(
            pending_play
                .selection_cards()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [4, 3, 6],
        );
        assert_eq!(legal_actions(&parked, &catalog).len(), 3);
        assert_eq!(parked.rng.get(crate::hot::RngStream::Sel).counter, 3);
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::CardPlay { .. },
                crate::frame::Frame::AfterPowerAmountChanged { .. },
                crate::frame::Frame::Draw { .. },
                crate::frame::Frame::CardPlay { .. },
            ]
        ));
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        assert_vicious_parked_malformed_vectors_refuse(&document, &rebuilt_catalog);
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &rebuilt, &rebuilt_catalog), Ok(()));
        let action = legal_actions(&rebuilt, &rebuilt_catalog)[0];
        let resumed_transition = apply_action(&rebuilt, &rebuilt_catalog, &action).unwrap();
        assert_eq!(
            resumed_transition.events,
            vec![
                Event::CardResolved {
                    uid: 3,
                    pile: PileId::Hand,
                },
                Event::CardResolved {
                    uid: 2,
                    pile: PileId::Discard,
                },
                Event::MonsterDamaged {
                    uid: 9,
                    blocked: 0,
                    unblocked: 3,
                    hp: 76,
                },
                Event::CardResolved {
                    uid: 1,
                    pile: PileId::Discard,
                },
            ]
        );
        let resumed = resumed_transition.state;
        assert!(resumed.pending.is_none() && resumed.frames.is_empty());
        assert_eq!(resumed.monsters[0].powers.value(PowerId::Vuln), 2);
        assert_eq!(resumed.monsters[0].hp, 76);
        assert_eq!(resumed.history.card_plays_finished_combat, 2);
        assert_eq!(resumed.rng.get(crate::hot::RngStream::Sel).counter, 3);
    }

    #[test]
    fn vulnerable_vicious_stratagem_reshuffle_roundtrips_and_resumes_once() {
        let mut builder = CatalogBuilder::new();
        let bash = builder
            .intern_reachable(plain_identity(CardId::Bash, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_live_vicious_reachable();
        builder.mark_live_stratagem_reachable();
        let catalog = builder.build();
        assert!(catalog.requires_action_replay());

        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 4;
        state.exact_piles = true;
        state.powers.set(PowerId::Vicious, SlotWire::Int, 1);
        state.powers.set(PowerId::Stratagem, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::Vicious])
        );
        state.rng.set(
            crate::hot::RngStream::Rng,
            crate::hot::RngStreamState {
                words: [1, 2, 3, 4],
                counter: 0,
            },
        );
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: bash,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Discard).make_mut().extend([
            HotCard {
                uid: 2,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 100);
        monster.max_hp = 100;
        monster.loop_pos = 2;
        monster.slot = 4;
        monster.uid = 8;
        state.monsters = std::sync::Arc::new(vec![monster]);

        let parked = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(matches!(
            parked.frames.as_slice(),
            [
                crate::frame::Frame::ActionReplay { .. },
                crate::frame::Frame::CardPlay { .. },
                crate::frame::Frame::AfterPowerAmountChanged { .. },
                crate::frame::Frame::Draw { .. },
            ]
        ));
        let crate::frame::Frame::Draw { record } = parked.frames.as_slice()[3] else {
            unreachable!()
        };
        let draw = parked.frames.draw(record).unwrap();
        assert_eq!(draw.caller, crate::hot::DrawCaller::AfterPowerAmountChanged);
        assert_eq!(draw.stage, crate::hot::DrawStage::AfterShuffle);
        assert_eq!(legal_actions(&parked, &catalog).len(), 2);
        let rng_after_park = parked.rng.get(crate::hot::RngStream::Rng);

        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        assert_eq!(admit(&document, &rebuilt, &rebuilt_catalog), Ok(()));
        let resumed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &legal_actions(&rebuilt, &rebuilt_catalog)[0],
        )
        .unwrap()
        .state;
        assert!(resumed.pending.is_none() && resumed.frames.is_empty());
        assert_eq!(resumed.monsters[0].powers.value(PowerId::Vuln), 2);
        assert_eq!(resumed.history.card_plays_finished_combat, 1);
        assert_eq!(resumed.rng.get(crate::hot::RngStream::Rng), rng_after_park);
        assert_eq!(
            resumed
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [2, 3]
        );
        assert_eq!(
            resumed
                .piles
                .get(PileId::Discard)
                .as_slice()
                .iter()
                .map(|card| card.uid)
                .collect::<Vec<_>>(),
            [1]
        );
        assert!(resumed.piles.get(PileId::Draw).is_empty());
    }

    #[test]
    fn vicious_recursive_draw_strikes_preserve_stale_intermediate_target() {
        for draw_strike_id in [CardId::PommelStrike, CardId::MinionStrike] {
            let mut builder = CatalogBuilder::new();
            let bash = builder
                .intern_reachable(plain_identity(CardId::Bash, 0))
                .unwrap();
            let draw_strike = builder
                .intern_reachable(plain_identity(draw_strike_id, 0))
                .unwrap();
            let seeker = builder
                .intern_reachable(plain_identity(CardId::SeekerStrike, 0))
                .unwrap();
            let strike = builder
                .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
                .unwrap();
            let defend = builder
                .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
                .unwrap();
            let anger = builder
                .intern_reachable(plain_identity(CardId::Anger, 0))
                .unwrap();
            builder.intern_monster(MonsterKind::Axebot).unwrap();
            builder.mark_live_hellraiser_reachable();
            builder.mark_live_vicious_reachable();
            let catalog = builder.build();

            let mut state = HotState::at_defaults();
            state.hp = 50;
            state.max_hp = 50;
            state.energy = 3;
            state.next_card_uid = 8;
            state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
            state.exact_piles = true;
            state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
            crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
            state.powers.set(PowerId::Vicious, SlotWire::Int, 1);
            assert!(
                state
                    .fanouts
                    .set_after_power_amount_changed_order(&[PowerId::Vicious])
            );
            for stream in [
                crate::hot::RngStream::Sel,
                crate::hot::RngStream::Targets,
                crate::hot::RngStream::Niche,
            ] {
                state.rng.set(
                    stream,
                    crate::hot::RngStreamState {
                        words: [1, 2, 3, 4],
                        counter: 0,
                    },
                );
            }
            state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
                uid: 1,
                atom: bash,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            });
            state.piles.get_mut(PileId::Draw).make_mut().extend([
                HotCard {
                    uid: 2,
                    atom: draw_strike,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 3,
                    atom: seeker,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 4,
                    atom: strike,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 5,
                    atom: defend,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 6,
                    atom: anger,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                HotCard {
                    uid: 7,
                    atom: strike,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
            ]);
            let mut axebot = HotMonster::new(MonsterKind::Axebot, 16);
            axebot.max_hp = 86;
            axebot.slot = 0;
            axebot.uid = 0;
            axebot.powers.set(PowerId::Stock, SlotWire::Int, 2);
            state.monsters = std::sync::Arc::new(vec![axebot]);

            let parked = apply_action(
                &state,
                &catalog,
                &Action::Play {
                    uid: 1,
                    target: Some(0),
                    selection: SelectionRef::NONE,
                },
            )
            .unwrap_or_else(|error| panic!("{draw_strike_id:?} initial play: {error:?}"))
            .state;
            assert_eq!((parked.monsters[0].slot, parked.monsters[0].uid), (0, 1));
            assert!(matches!(
                parked.frames.as_slice(),
                [
                    crate::frame::Frame::ActionReplay { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::AfterPowerAmountChanged { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. },
                    crate::frame::Frame::Draw { .. },
                    crate::frame::Frame::CardPlay { .. },
                ]
            ));
            let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
            let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
            let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog)
                .unwrap_or_else(|error| panic!("{draw_strike_id:?} import: {error:?}"));
            assert_eq!(admit(&document, &rebuilt, &rebuilt_catalog), Ok(()));
            let resumed = apply_action(
                &rebuilt,
                &rebuilt_catalog,
                &legal_actions(&rebuilt, &rebuilt_catalog)[0],
            )
            .unwrap_or_else(|error| panic!("{draw_strike_id:?} resume: {error:?}"))
            .state;
            assert!(resumed.pending.is_none() && resumed.frames.is_empty());
            assert_eq!((resumed.monsters[0].slot, resumed.monsters[0].uid), (0, 1));
            assert_eq!(resumed.history.card_plays_finished_combat, 3);
        }
    }

    #[test]
    fn vicious_child_stock_replacement_preserves_stale_owner_across_roundtrip() {
        let mut builder = CatalogBuilder::new();
        let bash = builder
            .intern_reachable(plain_identity(CardId::Bash, 0))
            .unwrap();
        let seeker = builder
            .intern_reachable(plain_identity(CardId::SeekerStrike, 0))
            .unwrap();
        let strike = builder
            .intern_reachable(plain_identity(CardId::StrikeIronclad, 0))
            .unwrap();
        let defend = builder
            .intern_reachable(plain_identity(CardId::DefendIronclad, 0))
            .unwrap();
        let anger = builder
            .intern_reachable(plain_identity(CardId::Anger, 0))
            .unwrap();
        builder.intern_monster(MonsterKind::Axebot).unwrap();
        builder.mark_live_hellraiser_reachable();
        builder.mark_live_vicious_reachable();
        let catalog = builder.build();

        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.next_card_uid = 7;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.exact_piles = true;
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        state.powers.set(PowerId::Vicious, SlotWire::Int, 1);
        assert!(
            state
                .fanouts
                .set_after_power_amount_changed_order(&[PowerId::Vicious])
        );
        for stream in [
            crate::hot::RngStream::Sel,
            crate::hot::RngStream::Targets,
            crate::hot::RngStream::Niche,
        ] {
            state.rng.set(
                stream,
                crate::hot::RngStreamState {
                    words: [1, 2, 3, 4],
                    counter: 0,
                },
            );
        }
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: bash,
            flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
        });
        state.piles.get_mut(PileId::Draw).make_mut().extend([
            HotCard {
                uid: 2,
                atom: seeker,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 3,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 4,
                atom: defend,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 5,
                atom: anger,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            HotCard {
                uid: 6,
                atom: strike,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
        ]);
        let mut axebot = HotMonster::new(MonsterKind::Axebot, 21);
        axebot.max_hp = 86;
        axebot.slot = 0;
        axebot.uid = 0;
        axebot.powers.set(PowerId::Stock, SlotWire::Int, 2);
        state.monsters = std::sync::Arc::new(vec![axebot]);

        let parked = apply_action(
            &state,
            &catalog,
            &Action::Play {
                uid: 1,
                target: Some(0),
                selection: SelectionRef::NONE,
            },
        )
        .unwrap()
        .state;
        assert!(parked.pending.is_some());
        assert_eq!((parked.monsters[0].slot, parked.monsters[0].uid), (0, 1));
        let document = HotBoundary::try_to_canonical(&parked, &catalog).unwrap();
        let rebuilt_catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
        let rebuilt = HotBoundary::from_canonical(&document, &rebuilt_catalog).unwrap();
        let resumed = apply_action(
            &rebuilt,
            &rebuilt_catalog,
            &legal_actions(&rebuilt, &rebuilt_catalog)[0],
        )
        .unwrap()
        .state;
        assert!(resumed.pending.is_none() && resumed.frames.is_empty());
        assert_eq!((resumed.monsters[0].slot, resumed.monsters[0].uid), (0, 1));
        assert_eq!(resumed.monsters[0].powers.value(PowerId::Vuln), 0);
        assert_eq!(resumed.history.card_plays_finished_combat, 2);
    }

    #[test]
    fn a_terminal_state_offers_no_actions_and_accepts_none() {
        let (mut state, catalog) = fixture();
        state.history.over = true;
        assert!(legal_actions(&state, &catalog).is_empty());
        assert_eq!(
            apply_action(&state, &catalog, &Action::EndTurn).unwrap_err(),
            EngineRefusal::CombatOver
        );
    }
}

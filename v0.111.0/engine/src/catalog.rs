//! The per-fight catalog: interning, card specs, and per-fight constants.
//!
//! PORT_PLAN.md D2/D3. Everything a fight needs that does not change from
//! node to node lives here, once, beside the hot state — never inside it, and
//! never cloned per search node. Two jobs:
//!
//! 1. **Interning.** A physical card's immutable identity is
//!    `(CardId, upgrade, enchantment)`. The catalog maps each distinct
//!    identity to a dense `atom: u16`, so [`crate::hot::HotCard`] is 8 bytes
//!    and every per-play predicate (is it a power? does it exhaust? what does
//!    it cost?) is one array index into [`CardSpec`] — never a lookup keyed on
//!    content text, and never a linear scan like the kernel's
//!    `HotCardCatalog::intern_semantic`.
//! 2. **Per-fight constants.** The admitted game build, and (later) the
//!    reward-generation provenance. The xoshiro streams are deliberately
//!    *not* here: R0.4 parked their seed words catalog-side, but a reshuffle
//!    advances the live words in place, so the whole stream state is hot
//!    (`hot::HotRng`).
//!
//! Building is allowed to allocate and to use ordered maps freely; the
//! builder runs once per fight, at the boundary. Reading is index-only.

use std::collections::{BTreeMap, BTreeSet};

use crate::content_tables::{
    Arg, CardRow, CardType, MULTIPLAYER_ONLY_CANPLAY_CARDS, Move, Repeats, Step,
    TEAMMATE_REQUIRED_CANPLAY_CARDS, card_has_native_unplayable_keyword, card_row, monster_loop,
    random_moves,
};
use crate::hooks::{HookRefusal, HookTable};
use crate::hot::PileId;
use crate::ids::{
    CardId, EnchantmentId, FilterMode, MonsterKind, MoveKind, PowerId, RelicId, SelectOp, StepKind,
    StepWord,
};

/// How many xoshiro streams a canonical document carries.
///
/// `tools/project_state.py::RNG_FIELDS` — matched by name, so a stream added
/// under a new name is a visible schema change rather than a silent drop.
pub const RNG_STREAM_COUNT: usize = 9;

macro_rules! wire_enum {
    ($(#[$meta:meta])* $name:ident { $(($variant:ident, $wire:literal)),+ $(,)? }) => {
        $(#[$meta])*
        #[repr(u8)]
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            /// Number of variants.
            pub const COUNT: usize = [$(stringify!($variant)),+].len();
            /// Canonical names, ascending — the binary-search table.
            pub const NAMES: [&'static str; Self::COUNT] = [$($wire),+];
            /// Every variant, in discriminant order.
            pub const ALL: [$name; Self::COUNT] = [$($name::$variant),+];

            /// The canonical name of this variant.
            pub const fn as_str(&self) -> &'static str {
                Self::NAMES[*self as usize]
            }

            /// Parse a canonical name. `O(log COUNT)`, no allocation.
            #[allow(clippy::should_implement_trait)]
            pub fn from_str(name: &str) -> Option<Self> {
                match Self::NAMES.binary_search(&name) {
                    Ok(index) => Some(Self::ALL[index]),
                    Err(_) => None,
                }
            }
        }
    };
}

wire_enum! {
    /// The game build a canonical document claims provenance from.
    ///
    /// `sim/meta/SIM_VERSIONING.md`: the admitted set is `{v0.111.0}` and grows
    /// forward only. Parity is always within a build, so an unadmitted build
    /// is a refusal at the boundary rather than a best-effort load.
    GameBuild {
        (V0_111_0, "v0.111.0"),
    }
}

wire_enum! {
    /// `State.reward_card_pool`.
    ///
    /// Vocabulary: `_validate_batch172_reward_state` checks the field against
    /// `combat_sim._BATCH172_REWARD_POOLS` (frozen Python, deleted #2827).
    RewardPool {
        (Defect, "Defect"),
        (Ironclad, "Ironclad"),
        (Necrobinder, "Necrobinder"),
        (Regent, "Regent"),
        (Silent, "Silent"),
    }
}

wire_enum! {
    /// `State.reward_card_rarity_odds`.
    ///
    /// Vocabulary: `_validate_batch172_reward_state` checks
    /// `combat_sim._BATCH172_REWARD_ODDS` (frozen Python, deleted #2827).
    RewardOdds {
        (BossEncounter, "BossEncounter"),
        (EliteEncounter, "EliteEncounter"),
        (RegularEncounter, "RegularEncounter"),
    }
}

wire_enum! {
    /// Native card target type, interned once so hot action/play paths never
    /// dispatch on generated table text.
    CardTargetType {
        (AllAllies, "AllAllies"),
        (AllEnemies, "AllEnemies"),
        (AnyAlly, "AnyAlly"),
        (AnyEnemy, "AnyEnemy"),
        (Dynamic, "Dynamic"),
        (None, "None"),
        (RandomEnemy, "RandomEnemy"),
        (SelfTarget, "Self"),
    }
}

/// A half-open range into one of a fight's arenas (arguments, compiled steps,
/// compiled moves). Eight bytes, `Copy`, no pointer.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Span {
    start: u32,
    len: u16,
}

impl Span {
    /// A span over `len` items starting at `start`.
    pub fn of(start: usize, len: usize) -> Self {
        Self {
            start: start as u32,
            len: len as u16,
        }
    }

    /// `(start, end)` as `usize`s, for slicing.
    pub fn bounds(self) -> (usize, usize) {
        let start = self.start as usize;
        (start, start + self.len as usize)
    }

    /// How many items the span covers.
    pub fn len(self) -> usize {
        self.len as usize
    }

    /// Whether the span is empty.
    pub fn is_empty(self) -> bool {
        self.len == 0
    }
}

/// Which typed vocabulary a position-addressed word rule resolves against.
///
/// The rules themselves live in [`crate::engine::admission`] — this is only
/// the alphabet they are written in.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum WordClass {
    /// A pile name (`hand`, `draw`, `discard`, …) — [`PileId`].
    Pile,
    /// A `select` filter — [`FilterMode`].
    Filter,
}

/// One argument of a compiled step, move, or relic-template op.
///
/// The whole point of the compiled form (PORT_PLAN.md D2, and the #1297
/// coordinator guidance): a generated table row carries `Arg::S("hand")`, and
/// the play path must never see that text. Admission resolves every string
/// exactly once, at combat start, into one of the typed variants below —
/// against a position-addressed rule where one exists, and otherwise into the
/// dense [`StepWord`] vocabulary the generator derives from the tables
/// themselves. A word that resolves to nothing becomes [`Self::Unresolved`],
/// which the admission gate reports by name and no body may run.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum CompiledArg {
    /// An integer literal.
    I(i64),
    /// A boolean literal.
    B(bool),
    /// Python `None`.
    Nil,
    /// A card id.
    Card(CardId),
    /// A power id.
    Power(PowerId),
    /// A select verb.
    Select(SelectOp),
    /// A monster kind.
    Monster(MonsterKind),
    /// A pile, resolved from a pile-classed position.
    Pile(PileId),
    /// A `select` filter, resolved from a filter-classed position.
    Filter(FilterMode),
    /// An interned opaque word: a sub-mode selector, a counter source, a
    /// position marker — everything the tables carry as text and no rule
    /// classifies further.
    Word(StepWord),
    /// A nested tuple, as a span of this same arena.
    List(Span),
    /// A table string outside every generated vocabulary. Only reachable if
    /// the tables and the id enums were generated from different trees; the
    /// admission gate names it and refuses the fight (I5: never a guess).
    Unresolved(&'static str),
}

/// One compiled card step: the dispatch kind plus typed arguments.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct CompiledStep {
    /// The dispatch kind.
    pub kind: StepKind,
    /// Its arguments, in the catalog's argument arena.
    pub args: Span,
}

/// One compiled monster move.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct CompiledMove {
    /// The dispatch kind.
    pub kind: MoveKind,
    /// Its arguments, in the catalog's argument arena.
    pub args: Span,
    /// The loop entry's successor spec, carried through unchanged. The
    /// admission gate accepts absent successors plus the owner-pinned
    /// Tunneler fixed successor, and refuses every other fixed or conditional
    /// branch.
    pub repeats: Repeats,
}

/// One compiled argument with every nested span already followed.
///
/// [`CompiledArg::List`] is an offset into the arena it was compiled in, so
/// two arenas can hold the same program at different offsets. Resolving both
/// sides is what lets [`Catalog::fight_is_covered_by`] compare programs rather
/// than layouts.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ResolvedArg {
    Leaf(CompiledArg),
    List(Vec<ResolvedArg>),
}

fn resolve_args(arena: &[CompiledArg], span: Span) -> Vec<ResolvedArg> {
    let (start, end) = span.bounds();
    arena
        .get(start..end)
        .unwrap_or_default()
        .iter()
        .map(|arg| match arg {
            CompiledArg::List(nested) => ResolvedArg::List(resolve_args(arena, *nested)),
            leaf => ResolvedArg::Leaf(*leaf),
        })
        .collect()
}

/// Compile one argument tuple into `arena`, returning its span.
///
/// `classes` supplies a [`WordClass`] per **top-level** position; positions
/// past its end, and every nested position, resolve generically. Nested tuples
/// are compiled first, so a parent's own elements stay contiguous.
///
/// Builder-time work: it allocates a small buffer per tuple and runs once per
/// fight, like the interning map beside it.
pub fn compile_args(
    args: &'static [Arg],
    arena: &mut Vec<CompiledArg>,
    classes: &[Option<WordClass>],
) -> Span {
    compile_args_at(args, arena, classes, None)
}

/// [`compile_args`] for one monster move row at the fight's `ascension`.
///
/// Each [`Arg::Tier`] compiles to [`crate::encounters::tier`] of it, which is
/// native `AscensionHelper::GetValueIfAscension` (v0.111.0 RVA `0x106c4c`,
/// `RunManager::HasAscension(gate) ? atOrAbove : below`) through
/// `AscensionManager::HasLevel` (RVA `0x11fa83`, `_level >= gate`). So every
/// move body, and the intents query, reads the fight's tier and never the
/// table's other one (#2828).
pub fn compile_move_args(
    args: &'static [Arg],
    arena: &mut Vec<CompiledArg>,
    classes: &[Option<WordClass>],
    ascension: u8,
) -> Span {
    compile_args_at(args, arena, classes, Some(ascension))
}

/// `ascension` is `None` for a card step or relic rule: only monster move
/// rows carry tiers, so a tier anywhere else compiles to
/// [`CompiledArg::Unresolved`] and admission refuses it by name rather than
/// picking a tier.
fn compile_args_at(
    args: &'static [Arg],
    arena: &mut Vec<CompiledArg>,
    classes: &[Option<WordClass>],
    ascension: Option<u8>,
) -> Span {
    let mut buffer: Vec<CompiledArg> = Vec::with_capacity(args.len());
    for (position, arg) in args.iter().enumerate() {
        buffer.push(match arg {
            Arg::I(value) => CompiledArg::I(*value),
            Arg::B(value) => CompiledArg::B(*value),
            Arg::Nil => CompiledArg::Nil,
            Arg::Card(id) => CompiledArg::Card(*id),
            Arg::Power(id) => CompiledArg::Power(*id),
            Arg::Select(op) => CompiledArg::Select(*op),
            Arg::Monster(kind) => CompiledArg::Monster(*kind),
            Arg::List(inner) => CompiledArg::List(compile_args_at(inner, arena, &[], ascension)),
            Arg::S(word) => compile_word(classes.get(position).copied().flatten(), word),
            Arg::Tier(tier) => match ascension {
                Some(level) => CompiledArg::I(crate::encounters::tier(*tier, level)),
                None => CompiledArg::Unresolved("ascension tier outside a monster move"),
            },
        });
    }
    let start = arena.len();
    arena.extend(buffer);
    Span::of(start, args.len())
}

/// Resolve one table string against its position's rule.
///
/// A classed position that does not resolve is [`CompiledArg::Unresolved`],
/// never a silent fallback to the generic vocabulary: the rule is a claim
/// about the position, and a claim that fails is a refusal (I5).
fn compile_word(class: Option<WordClass>, word: &'static str) -> CompiledArg {
    match class {
        Some(WordClass::Pile) => PileId::from_str(word).map(CompiledArg::Pile),
        Some(WordClass::Filter) => FilterMode::from_str(word).map(CompiledArg::Filter),
        None => StepWord::from_str(word).map(CompiledArg::Word),
    }
    .unwrap_or(CompiledArg::Unresolved(word))
}

/// Collect the generated tables' nested `(CardId, upgrade)` identities.
/// Boundary-time only; false positives are harmless pre-interned constants,
/// while missing one would make a reachable mint fail closed.
fn collect_minted_identities(args: &'static [Arg], output: &mut Vec<CardIdentity>) {
    if let [Arg::Card(id), Arg::I(upgrade)] = args
        && let Ok(upgrade) = u8::try_from(*upgrade)
    {
        output.push(CardIdentity {
            id: *id,
            upgrade,
            enchantment: None,
        });
    }
    for arg in args {
        if let Arg::List(inner) = arg {
            collect_minted_identities(inner, output);
        }
    }
}

/// A physical card's immutable run enchantment payload.
///
/// `combat_sim._validated_attack_damage_enchantment` pins the exact admitted
/// shape: a 2-tuple `(id, amount)` whose amount is a non-negative `int`.
/// Anything else refuses at the boundary rather than being flattened here.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CardEnchantment {
    /// The enchantment id (payload slot 0).
    pub id: EnchantmentId,
    /// The payload amount (slot 1).
    pub amount: i32,
}

/// The interned identity of a physical card: everything about it that cannot
/// change while it exists.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CardIdentity {
    /// The content id.
    pub id: CardId,
    /// Upgrade level (the second half of the `CARDS` registry key).
    pub upgrade: u8,
    /// The immutable run enchantment, when the card carries one.
    pub enchantment: Option<CardEnchantment>,
}

/// A dense per-fight card identity index. Indexes [`Catalog::spec`].
pub type CardAtom = u16;

/// One saved Tinker Time variant of Mad Science: the card's two
/// `[SavedProperty]` integers, `TinkerTimeType` and `TinkerTimeRider` (#2942).
///
/// # Why a fight-level axis, not an identity field (v0.111.0 IL)
///
/// The pair makes Mad Science a different card — its type, target and body
/// ([`crate::content_tables::MAD_SCIENCE_VARIANT_ROWS`]) — so every spec of
/// `CardId::MadScience` is keyed by it. It is carried once per fight on the
/// [`Catalog`] rather than on [`CardIdentity`], because nothing inside one
/// combat can make two copies disagree:
///
/// * the only writer is the event: `TinkerTime/<RiderChosen>d__15::MoveNext`
///   RVA `0x384308` creates the card (`CreateCard<MadScience>`, IL_002e) and
///   stores `set_TinkerTimeType` (IL_003b) and `set_TinkerTimeRider`
///   (IL_0047) before adding it to the deck (IL_0051). A scan of every call
///   site in the assembly finds the two setters called from only two other
///   methods, `TinkerTime::GetCardTypeHoverTip` (`0xd084c`, whose card goes to
///   `new CardHoverTip` at IL_0032) and `GetRiderHoverTip` (`0xd09fc`,
///   IL_001d-IL_0037, likewise): display cards that never enter a pile;
/// * a combat copy is a memberwise clone (`Player::PopulateCombatState`
///   `0x117a90` IL_002e → `AbstractModel::MutableClone` `0x79f0c` IL_000d
///   `MemberwiseClone`), and `MadScience` overrides neither `AfterCloned`
///   nor `AfterDeserialized`, so every clone, upgrade and pile move keeps
///   both fields (`MadScience::OnUpgrade` `0xe4acb` only adds Innate);
/// * `CardModel::FromSerializable` (`0x7e31c`) `Fill`s both at IL_002a;
/// * no in-combat generator can mint a fresh Mad Science: its rarity is
///   `Event` (`MadScience::.ctor` `0xe473c` IL_0004 `ldc.i4.6`), which every
///   combat candidate filter drops (`GetFilteredTransformationOptions`
///   `0x112a30` IL_0039-IL_005e keeps Common/Uncommon/Rare, and the owner
///   pools are character pools, which hold no Event card).
///
/// So one fight's Mad Science copies carry exactly the variants of its entry
/// deck, and a document holding two different variants is refused by name at
/// the boundary rather than represented. That is a narrowing (it needs two
/// Tinker Time events in one run), and the corpus has none.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MadScienceVariant {
    /// Native `CardType` value (`TinkerTimeType`): Attack 1, Skill 2, Power 3.
    pub tinker_type: u8,
    /// Native `TinkerTime.RiderEffect` value (`TinkerTimeRider`).
    pub rider: u8,
}

impl MadScienceVariant {
    /// The pair when it is one `TinkerTime::ChooseRiderEffect` can write
    /// (the generated legal domain), else `None`.
    pub fn from_saved(tinker_type: i64, rider: i64) -> Option<Self> {
        let variant = Self {
            tinker_type: u8::try_from(tinker_type).ok()?,
            rider: u8::try_from(rider).ok()?,
        };
        crate::content_tables::mad_science_variant_row(variant.tinker_type, variant.rider, 0)
            .map(|_| variant)
    }

    /// The generated row for this variant at one level: `Err` with the
    /// variant's generated entry when its body is not ported, `Ok(None)` when
    /// the level has no row.
    pub fn row(
        self,
        upgrade: u8,
    ) -> Result<Option<&'static CardRow>, &'static crate::content_tables::MadScienceVariantRow>
    {
        match crate::content_tables::mad_science_variant_row(self.tinker_type, self.rider, upgrade)
        {
            None => Ok(None),
            Some(variant) => variant.row.as_ref().map(Some).ok_or(variant),
        }
    }

    /// Whether this is the Chaos rider (#3322), the one variant that draws
    /// from the owner's card pool. `TinkerTime.RiderEffect` is None 0,
    /// Sapping 1, Violence 2, Choking 3, Energized 4, Wisdom 5, Chaos 6,
    /// Expertise 7, Curious 8, Improvement 9 (the enum's field constants, as
    /// `dll_content.DllFacts.mad_science_variants` reads them).
    pub fn is_chaos(self) -> bool {
        self.rider == 6
    }

    /// The native `RiderEffect` name.
    pub fn rider_name(self) -> &'static str {
        crate::content_tables::mad_science_variant_row(self.tinker_type, self.rider, 0)
            .map_or("?", |row| row.rider_name)
    }
}

/// The `CARD_ROWS` entries a spec can be interned from with no fight variant:
/// every row but Mad Science's (#2942). Its `CARD_ROWS` entry is the frozen
/// registry's placeholder, never a spec; the eighteen variant rows are walked
/// by `catalog::tests::mad_science_*` and, admitted in a real opening, by
/// `entry::opening::fixture_tests::every_saved_mad_science_variant_opens_as_its_own_card_or_refuses_by_name`.
/// The whole-registry censuses iterate this.
#[cfg(test)]
pub(crate) fn variant_free_card_rows() -> impl Iterator<Item = &'static CardRow> {
    crate::content_tables::CARD_ROWS
        .iter()
        .filter(|row| row.id != CardId::MadScience)
}

/// Whether `spec` is one ported Mad Science variant program at its own level
/// with the given native rider: its row is exactly the generated variant row.
/// The per-card allowlists of the shared step bodies (`strength`, `dexterity`,
/// `strangle`) admit Mad Science through this rather than by id alone, so a
/// Mad Science row with any other body can never reach them.
pub(crate) fn is_mad_science_variant_program(spec: &CardSpec, rider: &str) -> bool {
    matches!(spec.identity.id, CardId::MadScience)
        && crate::content_tables::MAD_SCIENCE_VARIANT_ROWS
            .iter()
            .any(|variant| {
                variant.rider_name == rider
                    && variant.row.as_ref().is_some_and(|row| {
                        std::ptr::eq(row, spec.row) && row.upgrade == spec.identity.upgrade
                    })
            })
}

/// Cold whole-action rehearsal class for the Forge/Sovereign family.
///
/// Derived once from immutable identity at interning. The ordinary value is
/// zero so common card plays load one compact byte instead of re-decoding a
/// growing rare CardId set on every action.
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum ForgeRehearsal {
    #[default]
    None,
    Furnace,
    Writer,
    Sovereign,
}

/// The interned spec behind one atom: identity plus the row predicates the
/// engine would otherwise re-derive per play.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CardSpec {
    /// The interned identity.
    pub identity: CardIdentity,
    pub(crate) forge_rehearsal: ForgeRehearsal,
    /// Base energy cost (`CardRow::cost`).
    pub cost: i64,
    /// Exact current-build native card type. Python's legacy convenience
    /// projection remains `combat_sim.Card.is_attack` (frozen Python, deleted #2827); this field is
    /// instead copied from the generated six-way census.
    pub card_type: CardType,
    /// Card type: power.
    pub is_power: bool,
    /// Card type: skill.
    pub is_skill: bool,
    /// Card type: attack (native CardType 1).
    pub is_attack: bool,
    /// Exact native Status card type (not Curse).
    pub is_status: bool,
    /// Status or curse.
    pub is_status_curse: bool,
    /// Carries `CardTag.Strike`.
    pub strike_tag: bool,
    /// Exhausts on play.
    pub exhausts: bool,
    /// Requires a target.
    pub targeted: bool,
    /// Native target vocabulary, interned from `CardRow::target_type`.
    pub target_type: CardTargetType,
    /// Ethereal.
    pub ethereal: bool,
    /// Innate.
    pub innate: bool,
    /// Retain.
    pub retain: bool,
    /// X-cost.
    pub x_cost: bool,
    /// Sly.
    pub sly: bool,
    /// Playable at all.
    pub playable: bool,
    /// Carries native `CardKeyword.Unplayable` (enum value 4).
    ///
    /// This is distinct from `!playable`: several native CanPlay refusals do
    /// not carry the keyword. Catastrophe reads the keyword collection.
    pub native_unplayable: bool,
    /// Union membership in `_card_can_play`'s generated multiplayer identity
    /// gates (#1613).
    ///
    /// This flag deliberately carries no live verdict. `engine::play` uses
    /// the two generated censuses separately and evaluates their respective
    /// live count / teammate-relation conditions against `HotState`.
    pub solo_unplayable: bool,
    /// Runs a select.
    pub selects: bool,
    /// The generated row this spec was interned from, for the admission
    /// gate's keyword walk (D4/D6). Reading it is boundary-time work, not
    /// a per-play path.
    pub row: &'static CardRow,
    /// This card's compiled step program, in the catalog's step arena.
    ///
    /// The play path reads this, never `row.steps`: the compiled form is the
    /// one with no text left in it.
    pub steps: Span,
}

/// Native SoulsPower::OnEnchant (v0.111.0 RVA 0xd64a9) removes Exhaust.
/// Keep this identity-derived so upgrades and physical clones retain the delta;
/// forced exhaust and Corruption still run through their separate live readers.
/// The highest GOOPY `Amount` the catalog pre-interns (#2874).
///
/// `Goopy::AfterCardPlayed` (v0.111.0 RVA `0xd6030`, `IL_0020`-`IL_002b`)
/// adds one to the played card's own `EnchantmentModel.Amount` after every
/// CardPlay of that card, and `EnchantBlockAdditive` (`0xd6092`) reads the
/// live amount. Rust spells the amount in the immutable slot-2 identity, as
/// Python's `_enchantment_after_card_played` does, so every amount a fight
/// can reach needs its atom before hot play begins: interning `GOOPY k`
/// interns `GOOPY k+1` up to this ceiling. The closure is monotone, so a
/// mid-fight document whose Goopy card has already grown re-derives a
/// sub-catalog of the entry fight's. Growth past the ceiling refuses at the
/// rewrite (`EngineRefusal::UnknownMintIdentity`) rather than wrapping; each
/// play of a Goopy card exhausts it, so 64 is far above any observed deck.
pub(crate) const GOOPY_AMOUNT_CEILING: i32 = 64;

/// `Goopy::CanEnchant` RVA `0xd6006`: the base `CanEnchant` plus
/// `Card.Tags.Contains(CardTag.Defend /* 2 */)` (`IL_000a`-`IL_0016`). The
/// generated rows carrying the `Defend` tag are exactly these six ids, the
/// same set as Python's `_GOOPY_CARD_IDS`; the census test in
/// `engine::play` pins the correspondence.
pub(crate) fn goopy_card_is_eligible(id: CardId) -> bool {
    matches!(
        id,
        CardId::DefendDefect
            | CardId::DefendIronclad
            | CardId::DefendNecrobinder
            | CardId::DefendRegent
            | CardId::DefendSilent
            | CardId::UltimateDefend
    )
}

/// The live GOOPY `Amount` spelled by an identity, if it carries Goopy.
pub(crate) fn goopy_amount(identity: CardIdentity) -> Option<i32> {
    match identity.enchantment {
        Some(CardEnchantment {
            id: EnchantmentId::Goopy,
            amount,
        }) => Some(amount),
        _ => None,
    }
}

/// Whether the identity carries `ROYALLY_APPROVED`, whose whole in-combat
/// effect is the two keywords `RoyallyApproved::OnEnchant` adds.
///
/// Current v0.111.0 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Enchantments.RoyallyApproved::OnEnchant` RVA `0xd62c9` is exactly
/// `Card.AddKeyword(3)` (`IL_0007`-`IL_0008`) then `Card.AddKeyword(5)`
/// (`IL_0013`-`IL_0014`) — `CardKeyword.Innate` and `CardKeyword.Retain`
/// (the enum's `Constant` rows are `Innate = 3`, `Retain = 5`). The amount is
/// never read: see [`crate::engine::play::keyword_delta_enchantment_identity_is_exact`]
/// for the full method set and the payload gate.
///
/// Deriving both keywords from slot 2 rather than a mutable keyword row is
/// exact because nothing in the build removes either one again:
/// `CardModel::AddKeyword` RVA `0x7d4f6` writes the `_keywords` set that
/// `UpgradeInternal` RVA `0x7e0c8` leaves alone, and a sweep of every
/// `RemoveKeyword` call site in the DLL finds only constants `1` (Exhaust)
/// and `2` (Ethereal) — the per-card `OnUpgrade` overrides, `CreateDupe`
/// RVA `0x7e25c` IL_0038 and `SoulsPower::OnEnchant` RVA `0xd64a9` — plus
/// the `CardCmd::RemoveKeyword` wrapper RVA `0x12fde8`, which nothing in the
/// DLL calls. The one wholesale rewrite of `_keywords`,
/// `DowngradeInternal` RVA `0x7e12c` (IL_0053-0060, canonical keywords
/// again), re-runs `Enchantment.ModifyCard` at IL_0077, whose `OnEnchant`
/// call (`ModifyCard` RVA `0x7f692` IL_0015) adds both back. So an upgrade,
/// a downgrade, a clone or a pile move keeps both, as the identity does.
pub(crate) fn royally_approved_adds_innate_and_retain(identity: CardIdentity) -> bool {
    matches!(
        identity.enchantment,
        Some(CardEnchantment {
            id: EnchantmentId::RoyallyApproved,
            ..
        })
    )
}

pub(crate) fn souls_power_removes_exhaust(identity: CardIdentity) -> bool {
    matches!(
        identity.enchantment,
        Some(CardEnchantment {
            id: EnchantmentId::SoulsPower,
            amount: 1,
        })
    )
}

/// The native `CardEnergyCost._base` of a card whose identity carries
/// `TEZCATARAS_EMBER` (#3413): zero for every fixed, non-negative cost.
///
/// Current v0.111.0 DLL SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Enchantments.TezcatarasEmber::OnEnchant` RVA `0xd6643` is
/// `Card.EnergyCost.UpgradeBy(-Card.EnergyCost.GetWithModifiers(0))`
/// (IL_0001-001e) then `Card.AddKeyword(7)` (IL_0023-002a). With
/// `CostModifiers.None` (`0`), `CardEnergyCost::GetWithModifiers` RVA
/// `0x11e044` skips both the local list (IL_0037-0048, flag 2) and the
/// `ModifyEnergyCostInCombat` walk (IL_0081-0092, flag 4) and returns
/// `Math.Max(0, _base)` (IL_00c3-00ca); `CardEnergyCost::UpgradeBy` RVA
/// `0x11e31c` returns early for X-cost (IL_0018-001f) and otherwise stores
/// `Math.Max(0, _base + delta)` (IL_0024-0034). So a fixed non-negative
/// `_base` becomes exactly zero; an X-cost card keeps its base.
///
/// The zero is permanent for the card, which is why it can live in the
/// immutable slot-2 identity rather than a mutable row:
///
/// * a save re-runs it on load — `CardModel::FromSerializable` RVA `0x7e31c`
///   calls `EnchantInternal` (IL_007d) then `EnchantmentModel::ModifyCard`
///   (IL_0088, whose `OnEnchant` call is RVA `0x7f692` IL_0015) BEFORE its
///   `UpgradeInternal` loop (IL_0097-00ae);
/// * a clone copies it — `CardModel::DeepCloneFields` RVA `0x7d21c`
///   IL_005d-0071 clones the cost object, and `CardEnergyCost::Clone` RVA
///   `0x11e41c` IL_0058-005f copies `_base`; the enchantment is re-attached
///   through `EnchantInternal` (IL_00b4) without re-running `OnEnchant`;
/// * an upgrade whose `OnUpgrade` lowers the cost clamps at zero
///   (`UpgradeBy`'s `Math.Max`), and `CardModel::DowngradeInternal` RVA
///   `0x7e12c` resets `_base` to `Canonical` (IL_0042) and then re-runs
///   `ModifyCard` (IL_0077), zeroing it again.
///
/// `Canonical` itself is untouched, but every in-combat reader of it is a
/// sign test (`CardEnergyCost::SetThisTurn` RVA `0x11e1f9` IL_0005-000b and
/// its three siblings, `ConfusedPower::AfterCardDrawn` RVA `0xa07f4`
/// IL_0025-0031) or reads a pool's canonical models rather than this card
/// (Jackpot's `<OnPlay>b__3_0` RVA `0x3a80a8`); a zero `_base` and a
/// non-negative `Canonical` agree on every sign test. Readers of the base,
/// such as Mummified Hand's `GetWithModifiers(0) > 0` (RVA `0x32add6`
/// IL_0002-000e), read the zero, as the Rust spec cost does.
///
/// The Eternal keyword (`CardKeyword` 7) has no combat reader:
/// `CardModel::get_IsRemovable` RVA `0x7ce0d` is its only reader, and
/// `get_IsTransformable` RVA `0x7ce20` (IL_000c-002d) is true for any card
/// outside the Deck pile whatever `IsRemovable` says; the other
/// `IsRemovable` callers are out-of-combat deck selections (events,
/// Amalgamator, `FromDeckForRemoval`).
pub(crate) fn tezcataras_ember_base_cost(identity: CardIdentity, row: &CardRow) -> i64 {
    let tezcatara = matches!(
        identity.enchantment,
        Some(CardEnchantment {
            id: EnchantmentId::TezcatarasEmber,
            ..
        })
    );
    if tezcatara && !row.x_cost && row.cost >= 0 {
        0
    } else {
        row.cost
    }
}

/// The one provenance predicate Entropy's transform is modeled under.
///
/// Four independent consumers have to agree about whether a native Entropy
/// transform can execute in this fight, and they read the same five plain
/// values from three different places:
///
/// 1. [`crate::engine::turn::entropy_transform_pool`] — refuses the transform
///    itself, reading [`crate::hot::HotState`].
/// 2. [`crate::engine::admission::admit`] — refuses the root by name
///    (`"Entropy transform pool provenance"`), reading `HotState` plus the
///    `Sel` stream's liveness, which is RNG liveness rather than pool shape
///    and so is conjoined at that call site rather than here.
/// 3. `boundary::intern_generated_identity_closure_inner`'s `CardId::Entropy`
///    arm — decides whether to expand the persistent catalogue closure over
///    all five character pools, reading the flag this predicate sets on
///    [`CatalogBuilder`] from the canonical document.
/// 4. `boundary::entropy_catalog_closure_is_exact` — decides whether to
///    *demand* that union back, reading the same flag off the built
///    [`Catalog`].
///
/// They drifted apart once already: consumer 3 expanded on `owner.is_some()`
/// alone while consumer 1 refused on `multiplayer_ally_key != 0`, so a party
/// state claimed reachability for a mechanic that provably cannot run in it
/// and tripped an unrelated boundary guard (#2637, the Outbreak
/// `teammate_present` group). Keeping the decision in one function over plain
/// values — not over `HotState`, which consumer 3 does not have, and not over
/// a document, which consumers 1 and 2 do not have — is what stops that
/// recurring; `entropy_transform_provenance_agrees_across_all_four_consumers`
/// pins the agreement.
///
/// Every conjunct is a refusal the native derivation requires, not a
/// convenience: `GetDefaultTransformationOptions` (RVA `0x112960`
/// IL_0048–IL_005f) reads `original.Owner.UnlockState` and
/// `RunState.CardMultiplayerConstraint`, so an UNRECORDED unlock profile or a
/// party run samples a pool this crate does not derive.
///
/// # A recorded profile, not a fully-unlocked one (#3122)
///
/// Both pool arms of `0x112960` reach the one `GetUnlockedCards` call: the
/// Colorless arm loads `ModelDb.CardPool<ColorlessCardPool>()` at IL_003a and
/// branches (`br.s`) to IL_0047, the `stloc.1` the `original.Pool` arm
/// (IL_0041-IL_0042) falls into. `CardPoolModel::GetUnlockedCards` (RVA
/// `0x7e54c`) calls the pool's virtual `FilterThroughEpochs` at IL_0014. The
/// five character overrides (Ironclad `0xf1f98`, Silent `0xf2cf8`, Regent
/// `0xf28e0`, Necrobinder `0xf2468`, Defect `0xf1944`) each make three
/// `IsEpochRevealed` tests (IL_0014, IL_0042, IL_0070) and
/// `ColorlessCardPool::FilterThroughEpochs` (`0xf13f4`) makes five (IL_0014,
/// IL_0042, IL_0070, IL_009e, IL_00cc); the six other shared pools inherit
/// the identity `CardPoolModel::FilterThroughEpochs` (`0x7e5c5`). So every
/// pool an origin can route to is a function of the recorded profile's
/// gating epochs alone, and `Catalog::owner_generation_pool` /
/// `Catalog::entropy_colorless_transform_pool` derive it from ANY recorded
/// profile — the argument #2560/#2946 made for the owner-pool generators
/// ([`crate::engine::cards::unlock_profile_is_recorded`]). The third
/// parameter is therefore "the profile is recorded" (fully unlocked, or a
/// partial profile the catalog carries), not "fully unlocked".
///
/// Nothing in `0x112960`, `0x112a30` or `EntropyPower`'s
/// `<AfterPlayerTurnStart>d__6::MoveNext` (`0x339e48`) reads a reward or
/// "Entropy" owner pool: the candidate pool is keyed by the ORIGINAL card.
/// `entropy_card_pool` therefore adds no input. An explicit value that
/// CONTRADICTS `reward_card_pool` is still refused (the document disagrees
/// with itself), but an absent one — the Rust opening writes it only for a
/// solo Ironclad or Regent — is not, which is the
/// [`crate::engine::cards::owner_pool_generation_provenance_is_exact`] rule.
/// `reward_card_pool` stays required: the closure walk
/// (`boundary::entropy_catalog_closure_is_exact`) is keyed by an owner for
/// the owner-pool generators it recurses into.
///
/// # The Regent conjunct
///
/// `regent_colorless_pool_is_published` is the guard `main` spelled inline as
/// `entropy_owner != Some(Regent) || state.spectrum_shift_generation_pool()`.
/// An earlier draft of this slice dropped it while generalizing the owner
/// filter. It is kept, deliberately, and not because it was re-derived as
/// load-bearing for the *values* this transform samples — a character-pool
/// origin samples `CHARACTER_CARD_POOL_ROWS_V1101`, never the Colorless pool
/// this flag authenticates.
///
/// It is kept because the flag is the document's authority for a pool the
/// Entropy CLOSURE reaches on exactly this owner, and because six sibling
/// consumers of `owner_generation_pool` still require it. On a Regent root the
/// widened closure runs Entropy → the Regent character pool → `SpectrumShift`
/// (`content_tables::CHARACTER_CARD_POOL_ROWS_V1101`'s REGENT rows), whose own
/// closure arm interns `REGENT_COLORLESS_GENERATION_POOL_V1101` — the very
/// pool `spectrum_shift_generation_pool` authenticates. Dropping the conjunct
/// here while `steps::neutral::begin_discovery_exact`,
/// `engine::selection`, the pending-Discovery decode,
/// `steps::regent_uncommon::{bundle_of_joy_exact, manifest_authority_exact}`
/// and their two admission sites all keep it would leave two mechanics reading
/// the same table and disagreeing about whether its provenance is
/// established. #2637's hard rules say to preserve every independent guard, so
/// the asymmetry is resolved by keeping the guard, not by removing five more.
/// `entropy_refuses_a_regent_root_whose_document_omits_the_colorless_pool` is
/// the witness that would fail if this conjunct went missing again.
pub(crate) fn entropy_transform_provenance_is_exact(
    reward_card_pool: Option<RewardPool>,
    entropy_card_pool: Option<RewardPool>,
    unlock_profile_is_recorded: bool,
    multiplayer_ally_key: u8,
    regent_colorless_pool_is_published: bool,
) -> bool {
    reward_card_pool.is_some()
        && entropy_card_pool.is_none_or(|pool| Some(pool) == reward_card_pool)
        && unlock_profile_is_recorded
        && multiplayer_ally_key == 0
        && (reward_card_pool != Some(RewardPool::Regent) || regent_colorless_pool_is_published)
}

/// `CardModel::get_Pool` — which character pool a card *originates* in.
///
/// Authority: v111 DLL SHA256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
///
/// `CardFactory::GetDefaultTransformationOptions` (RVA `0x112960`) reads
/// `original.Pool` at IL_0041–IL_0047 for every card that is not Quest-typed
/// and not `Ancient`/`Event`/`Token` rarity, so a transform's candidate list is
/// keyed by the ORIGINAL card's class, independent of who owns the run. This
/// is the lookup that makes that possible.
///
/// It indexes `CHARACTER_CARD_POOL_MEMBERSHIP_V1101` — the pool's whole
/// `GenerateAllCards` MEMBERSHIP, which is what `CardPoolModel::get_AllCardIds`
/// (RVA `0x7e4dc`) hands `get_Pool`'s `FirstOrDefault` predicate (RVA
/// `0x7e433`). It deliberately does NOT index
/// `CHARACTER_CARD_POOL_ROWS_V1101`, which is pre-filtered by
/// `CanBeGeneratedInCombat` and `MultiplayerConstraint`: those two predicates
/// are `CardFactory::FilterForCombat`/`FilterForPlayerCount` and native applies
/// them to the CANDIDATES (`GetFilteredTransformationOptions` RVA `0x112a30`
/// IL_005f–IL_0087 and IL_00a0–IL_00b6), never to the original. Indexing the
/// filtered table lost 32 legal origins — among them the solo-holdable
/// Ironclad `Feed`/`Midnight`/`NotYet` and Regent `Largesse`/`Royalties` — and
/// turned each into a mid-fight refusal inside an admitted root.
///
/// So `Basic` origins survive (a `Basic` Strike is a legal transform *source*
/// even though it is never a transform *candidate* — `0x112a30`
/// IL_0039–IL_005e restricts candidates to `Common`/`Uncommon`/`Rare`), and so
/// do non-combat-generable and multiplayer-only ones. A multiplayer-only
/// ORIGIN is modeled rather than special-cased because native never consults
/// the original's own `MultiplayerConstraint` here: `GetUnlockedCards`
/// (IL_0048–IL_005f) is handed `RunState.CardMultiplayerConstraint`, a
/// run-level value that filters the candidates, and the transform's provenance
/// predicate already pins that run to solo
/// ([`entropy_transform_provenance_is_exact`]'s `multiplayer_ally_key == 0`).
///
/// `None` means the id is not a character-pool card at all — Colorless, status,
/// token and curse origins are routed to their own pools before this is
/// consulted, so a `None` here is an unmodeled origin and the caller refuses.
/// `entropy_transform_origin_dispatch_is_total_over_every_card_row` enumerates
/// every id that reaches that refusal.
///
/// The positional index below is the `steps::neutral::character_pool_index`
/// order, NOT `RewardPool::ALL`; `card_character_pool_is_a_partition_of_the_five_class_tables`
/// pins it against the table's own name column.
pub(crate) fn card_character_pool(id: CardId) -> Option<RewardPool> {
    static POOLS: std::sync::LazyLock<[Option<RewardPool>; CardId::COUNT]> =
        std::sync::LazyLock::new(|| {
            let mut pools = [None; CardId::COUNT];
            for (index, (_, members)) in crate::content_tables::CHARACTER_CARD_POOL_MEMBERSHIP_V1101
                .iter()
                .enumerate()
            {
                let pool = [
                    RewardPool::Ironclad,
                    RewardPool::Silent,
                    RewardPool::Regent,
                    RewardPool::Necrobinder,
                    RewardPool::Defect,
                ][index];
                for card in *members {
                    pools[*card as usize] = Some(pool);
                }
            }
            pools
        });
    POOLS[id as usize]
}

impl CardSpec {
    fn from_row(identity: CardIdentity, row: &'static CardRow, steps: Span) -> Self {
        let forge_rehearsal = match identity.id {
            CardId::Furnace => ForgeRehearsal::Furnace,
            CardId::SeekingEdge
            | CardId::Conqueror
            | CardId::RefineBlade
            | CardId::SummonForth
            | CardId::TheSmith
            | CardId::BigBang
            | CardId::Bulwark
            | CardId::SpoilsOfBattle => ForgeRehearsal::Writer,
            CardId::SovereignBlade => ForgeRehearsal::Sovereign,
            _ => ForgeRehearsal::None,
        };
        Self {
            identity,
            forge_rehearsal,
            steps,
            // `TezcatarasEmber::OnEnchant` RVA `0xd6643` zeroes `_base`; see
            // [`tezcataras_ember_base_cost`].
            cost: tezcataras_ember_base_cost(identity, row),
            card_type: row.card_type,
            is_power: row.card_type == CardType::Power,
            is_skill: row.card_type == CardType::Skill,
            is_attack: row.card_type == CardType::Attack,
            is_status: row.card_type == CardType::Status,
            is_status_curse: matches!(row.card_type, CardType::Status | CardType::Curse),
            strike_tag: row.strike_tag,
            // `Goopy::OnEnchant` RVA `0xd601f` is `Card.AddKeyword(1)`:
            // Exhaust, derived from slot 2 exactly as Python's
            // `card_effective_keywords` adds it for GOOPY.
            exhausts: (row.exhausts || goopy_amount(identity).is_some())
                && !souls_power_removes_exhaust(identity),
            targeted: row.targeted,
            target_type: CardTargetType::from_str(row.target_type)
                .expect("generated card target type belongs to the closed vocabulary"),
            ethereal: row.ethereal,
            // `RoyallyApproved::OnEnchant` RVA `0xd62c9` adds Innate and
            // Retain; see [`royally_approved_adds_innate_and_retain`].
            innate: row.innate || royally_approved_adds_innate_and_retain(identity),
            retain: row.retain || royally_approved_adds_innate_and_retain(identity),
            x_cost: row.x_cost,
            sly: row.sly,
            playable: row.playable,
            native_unplayable: card_has_native_unplayable_keyword(row.id, row.upgrade),
            selects: row.selects,
            solo_unplayable: MULTIPLAYER_ONLY_CANPLAY_CARDS.contains(&row.id)
                || TEAMMATE_REQUIRED_CANPLAY_CARDS.contains(&row.id),
            row,
        }
    }
}

/// A value derived from the rest of an immutable [`Catalog`], computed on
/// first use.
///
/// It is a function of fields `Catalog`'s `PartialEq` already compares, so
/// equality and `Debug` ignore whether it has been computed yet, and a clone
/// carries it along (#3420).
#[derive(Clone)]
pub(crate) struct Derived<T>(std::sync::OnceLock<T>);

impl<T> Default for Derived<T> {
    fn default() -> Self {
        Self(std::sync::OnceLock::new())
    }
}

impl<T> PartialEq for Derived<T> {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl<T> Eq for Derived<T> {}

impl<T> std::fmt::Debug for Derived<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Derived")
    }
}

/// Everything constant for the duration of one fight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    specs: Vec<CardSpec>,
    /// Identities ascending, each paired with its atom — the reverse index,
    /// so recovering an atom is a binary search rather than the kernel's
    /// linear `intern_semantic` scan.
    by_identity: Vec<(CardIdentity, CardAtom)>,
    /// Compiled step programs, addressed by [`CardSpec::steps`].
    steps: Vec<CompiledStep>,
    /// Compiled monster move loops, addressed by `by_monster`.
    moves: Vec<CompiledMove>,
    /// Monster kinds ascending, each paired with its span of `moves`.
    by_monster: Vec<(MonsterKind, Span)>,
    /// The one argument arena every compiled form points into.
    args: Vec<CompiledArg>,
    /// The per-combat hook subscriber tables (D5).
    hooks: HookTable,
    game_build: Option<GameBuild>,
    /// The fight's `AscensionLevel`, as the distance below
    /// [`crate::encounters::MODELED_ASCENSION`]: every tiered monster move
    /// constant in `args` was compiled at it (#2828). Immutable per fight.
    ascension_below_modeled: u8,
    /// Immutable normalized unlock provenance, interned against
    /// [`crate::content_tables::UNLOCK_EPOCH_UNIVERSE_V1101`]. `None` means
    /// the document either omitted the field or carried the exact
    /// fully-unlocked profile, which the hot flag already represents.
    ///
    /// A partial profile is preserved verbatim so terminal digests round-trip
    /// losslessly (#2453), and it must never grant fully-unlocked pools.
    splash_unlock_epochs: Option<Vec<&'static str>>,
    /// The document's `player.splash_unlock_epochs` exactly as it arrived,
    /// carried only so the projection round-trips. `None` means the document
    /// omitted the field.
    ///
    /// [`Self::splash_unlock_epochs`] above cannot serve this purpose: it is
    /// deliberately `None` for a profile that reveals every card-pool gating
    /// epoch, because such a profile derives the frozen pools and must stay
    /// out of `MissingCapability::PartialUnlockProfile` (#2512). A real
    /// profile is a strict SUPERSET of the 39 gameplay epochs — it also
    /// carries act, daily-run and cosmetic epochs — so re-emitting the
    /// canonical 39 for it silently rewrote the field and made every digest
    /// after entry differ from Python's (#2554: the first such profile to
    /// admit was Sean's Defect floor-33 fight, at 57 epochs).
    wire_unlock_epochs: Option<Vec<&'static str>>,
    /// Bit `i` set iff `SPLASH_CHARACTER_FIRST_UNLOCK_EPOCH_V1101[i]` is in
    /// `splash_unlock_epochs`, in `SPLASH_CHARACTER_POOL_ORDER_V1101` order.
    /// Derived once here so the Splash pool derivation never re-walks the
    /// profile's strings for the character-inclusion test (#2469).
    splash_character_mask: u8,
    /// Whether any live or prospective identity can use the sole dynamic
    /// target resolver. Frozen once at build time so ordinary action-space
    /// enumeration can guard that buffered branch once before walking the hand.
    has_sovereign_blade: bool,
    /// Whether a reachable identity in this fight is `Misery` — i.e. whether
    /// any physical power attachment this fight records can ever be READ.
    ///
    /// `Misery` is the only consumer of the ordered attachment ledger, so this
    /// is what gates ledger upkeep on the monster-Strength write path (#2693
    /// S1, [`crate::engine::damage::write_monster_strength`]). Deriving it
    /// here rather than at each writer keeps the decision a per-fight
    /// constant: a catalog is immutable, so it can never flip mid-combat and
    /// leave half a fight's provenance recorded.
    ///
    /// It is deliberately the *reachability* test alone and not admission's
    /// `misery_reachable`, which additionally requires the row to be admitted:
    /// a superset only ever records more provenance than a reader needs, and
    /// the writer gives up rather than refusing where it cannot record any.
    misery_is_reachable: bool,
    /// The catalog-only halves of admission's Stampede identity closure and
    /// Hellraiser liveness, derived at [`CatalogBuilder::build`] from the
    /// reachable specs. Each was a scan of every reachable spec, repeated on
    /// every continuation rehearsal (#3420).
    stampede_closure: crate::engine::admission::StampedeClosureFacts,
    /// Whether an execution-reachable spec is Outbreak at any level: the
    /// catalog half of `steps::silent_rare::outbreak_is_reachable`, which
    /// canonical projection asks on every parked replay root (#3420).
    outbreak_reachable: bool,
    /// Whether an execution-reachable spec carries an Alchemize, Constellation
    /// or Huddle Up exact step: the catalog half of the public path's
    /// terminal-aware root rehearsal, asked on every public action (#3420).
    terminal_aware_root_step_reachable: bool,
    /// Generation-potion pool facts, derived on first use; see
    /// [`crate::engine::potions::GenerationPoolFacts`].
    generation_pools: Derived<crate::engine::potions::GenerationPoolFacts>,
    /// Whether any interned identity is Normality.
    ///
    /// A catalog is immutable and every live card carries an interned atom,
    /// so a fight without this bit can never hold a Normality in any pile.
    /// It gates upkeep of Normality's started-play count
    /// (`engine::play::normality_blocks_card_plays`), whose only reader is a
    /// Normality in Hand.
    has_normality: bool,
    /// Whether any interned identity is Enthralled (#2561).
    ///
    /// Same immutability argument as `has_normality`: it gates the Hand scan
    /// in `engine::play::enthralled_blocks_card_play`.
    has_enthralled: bool,
    /// Whether the immutable closure contains Howl or I Am Invincible.
    ///
    /// Its physical AutoPost listener walks all five live piles. Keeping this
    /// cold fact beside the other catalog capabilities avoids a catalog scan
    /// at every turn boundary while adding nothing to [`crate::hot::HotState`].
    has_auto_post_card_listener: bool,
    /// Whether any immutable fight identity owns Batch139's self-return
    /// listener. This keeps ordinary turn entry from scanning all five piles.
    has_batch139_self_return: bool,
    /// Whether this immutable fight closure can park a replay-rooted action.
    ///
    /// Derived once from every execution-capable selector/batch producer and
    /// persistent AutoPre/AutoPost signal. It deliberately does not depend on
    /// a live Stoke card: generated descendants and persistent powers remain
    /// after their originating source moves or vanishes.
    requires_action_replay: bool,
    /// Whether an exact CardPlay-owned Draw can itself suspend in this fight.
    /// This cold capability authenticates the recursive rooted continuation
    /// grammar and source-derived admission preflights.
    cardplay_no_result_draw_can_suspend: bool,
    /// Whether Stratagem or a selecting Hellraiser Strike can suspend one
    /// exact CardPlay-owned Draw, independent of the drawing source program.
    cardplay_draw_hook_can_suspend: bool,
    /// Whether a deferred ordinary-Ethereal Dark Embrace Draw can park.
    dark_embrace_side_end_can_suspend: bool,
    /// Whether an ordinary Exhaust's immediate Dark Embrace Draw can park.
    after_card_exhausted_dark_can_suspend: bool,
    /// Whether the immutable/local closure can create that deferred tally,
    /// even when its eventual Draw is synchronous.
    dark_embrace_ethereal_reachable: bool,
    /// Cold source provenance: an exact Hammer Time card can still execute.
    hammer_time_reachable: bool,
    /// Cold source provenance: an exact Forge command can still execute.
    forge_command_reachable: bool,
    /// Whether this fight's own provenance PROVES a native Entropy transform
    /// can never execute, so its five-class closure must not be claimed.
    ///
    /// Stored inverted so that `false` — the `Default` — means "expand", which
    /// is every catalog that was not built from a canonical document. A
    /// hand-built catalog carries no provenance to prove anything with, and
    /// silently declining to expand there would weaken
    /// `entropy_catalog_closure_is_exact` for every engine fixture rather than
    /// only for the states the transform refuses. Only
    /// `boundary::catalog_from_canonical` ever sets it, from
    /// [`entropy_transform_provenance_is_exact`] over the document's own five
    /// values.
    entropy_transform_provably_inert: bool,
    /// The fight's saved Mad Science variant ([`MadScienceVariant`]), when
    /// the document holds a Mad Science. Every `CardId::MadScience` spec in
    /// this catalog is this variant's row.
    mad_science_variant: Option<MadScienceVariant>,
    /// Exact local identities reached from execution-capable physical,
    /// pending, or generated sources, in ascending order. Remote and Exhaust
    /// catalog-only identities are deliberately absent.
    reachable_identities: Vec<CardIdentity>,
    /// Source-authorized totality identities that an exact recursive runtime
    /// root may mint, but which are not all simultaneously causal futures.
    /// They provide immutable atom lookup without widening admission's
    /// semantic reachable set.
    potential_identities: Vec<CardIdentity>,
}

impl Catalog {
    /// The `AscensionLevel` this catalog's monster move rows were compiled at
    /// (#2828). The boundary refuses a document whose own ascension differs.
    pub fn ascension(&self) -> u8 {
        crate::encounters::MODELED_ASCENSION - self.ascension_below_modeled
    }

    /// One tiered move constant at this catalog's [`Self::ascension`]: the
    /// value its compiled move rows carry, for a body or validator that pins
    /// its own row.
    pub fn tier(&self, tier: crate::content_tables::AscensionTier) -> i64 {
        crate::encounters::tier(tier, self.ascension())
    }

    pub(crate) fn splash_unlock_epochs(&self) -> Option<&[&'static str]> {
        self.splash_unlock_epochs.as_deref()
    }

    /// The verbatim document profile, for lossless re-emission only.
    pub(crate) fn wire_unlock_epochs(&self) -> Option<&[&'static str]> {
        self.wire_unlock_epochs.as_deref()
    }

    /// This fight's exact ordered Splash pool for `owner`.
    ///
    /// The fully-unlocked profile keeps the frozen concatenation; a partial
    /// profile derives its pool from the census. See
    /// [`crate::steps::neutral::derive_splash_attack_pool`].
    pub(crate) fn splash_attack_pool(&self, owner: RewardPool) -> Vec<CardId> {
        match self.splash_unlock_epochs.as_deref() {
            Some(epochs) => crate::steps::neutral::derive_splash_attack_pool(
                owner,
                epochs,
                self.splash_character_mask,
            ),
            None => crate::steps::neutral::splash_pool(owner),
        }
    }

    /// This fight's exact ordered owner-pool projections (#2542).
    ///
    /// The four generators that read "the owner's unlocked CharacterCardPool"
    /// all project the same generated rows; only the rarity/type/cost
    /// comparison differs. Routing them through the catalog keeps the epoch
    /// profile in one place, exactly as [`Catalog::splash_attack_pool`] does.
    pub(crate) fn owner_generation_pool(&self, owner: RewardPool) -> Vec<CardId> {
        crate::steps::neutral::owner_generation_pool(owner, self.splash_unlock_epochs.as_deref())
    }

    /// Exact ordered owner pool for a persistent BeforeHandDraw generator.
    pub(crate) fn owner_listener_pool(&self, owner: RewardPool, power: PowerId) -> Vec<CardId> {
        crate::steps::neutral::owner_listener_pool(
            owner,
            self.splash_unlock_epochs.as_deref(),
            power,
        )
    }

    /// The Colorless candidates an Entropy transform draws under this fight's
    /// recorded profile (#3122). See
    /// [`crate::steps::neutral::entropy_colorless_transform_pool`].
    pub(crate) fn entropy_colorless_transform_pool(&self) -> Vec<CardId> {
        crate::steps::neutral::entropy_colorless_transform_pool(
            self.splash_unlock_epochs.as_deref(),
        )
    }

    /// The solo Colorless generation pool Quasar, Spectrum Shift, Bundle of
    /// Joy and Manifest Authority draw under this fight's recorded profile
    /// (the Colorless half of #2560). See
    /// [`crate::steps::neutral::colorless_generation_pool`].
    pub(crate) fn colorless_generation_pool(&self) -> Vec<CardId> {
        crate::steps::neutral::colorless_generation_pool(self.splash_unlock_epochs.as_deref())
    }

    /// Largesse's Colorless pool when it targets the LOCAL Player, under this
    /// fight's recorded profile (#3285). See
    /// [`crate::steps::neutral::largesse_colorless_pool`].
    pub(crate) fn largesse_local_colorless_pool(&self) -> Vec<CardId> {
        crate::steps::neutral::largesse_colorless_pool(self.splash_unlock_epochs.as_deref())
    }

    /// Jack of All Trades' self-excluding Colorless pool under this fight's
    /// recorded profile (#2560). See
    /// [`crate::steps::neutral::jack_of_all_trades_pool`].
    pub(crate) fn jack_of_all_trades_pool(&self) -> Vec<CardId> {
        crate::steps::neutral::jack_of_all_trades_pool(self.splash_unlock_epochs.as_deref())
    }

    /// Calamity's `Common..Rare` Attack projection of the owner pool.
    pub(crate) fn calamity_attack_pool(&self, owner: RewardPool) -> Vec<CardId> {
        crate::steps::neutral::calamity_owner_attack_pool(
            owner,
            self.splash_unlock_epochs.as_deref(),
        )
    }

    /// Jackpot's canonical-zero-cost projection of the owner pool under this
    /// fight's recorded profile (#2946).
    ///
    /// `Jackpot/<OnPlay>d__3::MoveNext` RVA `0x3a80d4` reads
    /// `Owner.Character.CardPool` (IL_00e6-IL_00eb) and filters it through
    /// `GetUnlockedCards(Owner.UnlockState, ..)` at IL_010b before its
    /// zero-cost `Where` (IL_012f) and `GetForCombat` (IL_0159), so the pool
    /// is the owner's rows filtered by the owner's own gating epochs.
    pub(crate) fn owner_zero_cost_pool(&self, owner: RewardPool) -> Vec<CardId> {
        crate::steps::neutral::owner_zero_cost_pool(owner, self.splash_unlock_epochs.as_deref())
    }

    /// The owner's one-`CardType` generation pool under this fight's recorded
    /// profile: Distraction (Skill) and White Noise (Power). See
    /// [`crate::steps::neutral::owner_type_generation_pool`].
    pub(crate) fn owner_type_generation_pool(
        &self,
        owner: RewardPool,
        card_type: crate::content_tables::CardType,
    ) -> Vec<CardId> {
        crate::steps::neutral::owner_type_generation_pool(
            owner,
            self.splash_unlock_epochs.as_deref(),
            card_type,
        )
    }

    /// Crossbow's owner Attack pool under this fight's recorded profile
    /// (#2970). See [`crate::steps::neutral::crossbow_owner_attack_pool`].
    pub(crate) fn crossbow_attack_pool(&self, owner: RewardPool) -> Vec<CardId> {
        crate::steps::neutral::crossbow_owner_attack_pool(
            owner,
            self.splash_unlock_epochs.as_deref(),
        )
    }

    /// Metamorphosis' `FilterForCombat` Attack projection of the owner pool.
    pub(crate) fn metamorphosis_attack_pool(&self, owner: RewardPool) -> Vec<CardId> {
        crate::steps::neutral::metamorphosis_owner_attack_pool(
            owner,
            self.splash_unlock_epochs.as_deref(),
        )
    }

    /// The spec behind an atom. A plain index; `None` only for an atom that
    /// this catalog never issued.
    pub fn spec(&self, atom: CardAtom) -> Option<&CardSpec> {
        self.specs.get(atom as usize)
    }

    #[cfg(test)]
    pub(crate) fn spec_mut_for_test(&mut self, atom: CardAtom) -> Option<&mut CardSpec> {
        self.specs.get_mut(atom as usize)
    }

    /// One card's compiled step program.
    pub fn steps(&self, spec: &CardSpec) -> &[CompiledStep] {
        let (start, end) = spec.steps.bounds();
        &self.steps[start..end]
    }

    /// One monster kind's compiled move table. Empty for a kind this fight
    /// never interned, or one without an executable deterministic/allowlisted
    /// random table.
    pub fn moves(&self, kind: MonsterKind) -> &[CompiledMove] {
        match self
            .by_monster
            .binary_search_by_key(&(kind as u16), |(interned, _)| *interned as u16)
        {
            Ok(index) => {
                let (start, end) = self.by_monster[index].1.bounds();
                &self.moves[start..end]
            }
            Err(_) => &[],
        }
    }

    /// The typed arguments a compiled step or move points at.
    pub fn args(&self, span: Span) -> &[CompiledArg] {
        let (start, end) = span.bounds();
        &self.args[start..end]
    }

    /// The per-combat hook tables (D5).
    pub fn hooks(&self) -> &HookTable {
        &self.hooks
    }

    /// Every table string this fight's content failed to resolve.
    ///
    /// Boundary-time work for the admission gate, which turns each one into a
    /// refusal naming the string. The healthy result is empty.
    pub fn unresolved_words(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.args.iter().filter_map(|arg| match arg {
            CompiledArg::Unresolved(word) => Some(*word),
            _ => None,
        })
    }

    /// The atom for an identity this fight interned. `O(log n)`.
    pub fn atom(&self, identity: &CardIdentity) -> Option<CardAtom> {
        match self
            .by_identity
            .binary_search_by(|entry| entry.0.cmp(identity))
        {
            Ok(index) => Some(self.by_identity[index].1),
            Err(_) => None,
        }
    }

    /// How many distinct card identities this fight interned.
    pub fn atom_count(&self) -> usize {
        self.specs.len()
    }

    /// Every identity in the fight's immutable card closure.
    ///
    /// Admission walks this rather than only the live piles: generated cards
    /// can be played later in the same fight, so their programs must pass the
    /// same all-or-nothing gate before the first action runs.
    pub fn specs(&self) -> impl Iterator<Item = &CardSpec> {
        self.specs.iter()
    }

    /// Every immutable spec reached from an execution-capable local source.
    /// Remote, imitation-only, and Exhaust-only catalog rows are excluded.
    pub(crate) fn reachable_specs(&self) -> impl Iterator<Item = &CardSpec> {
        self.reachable_identities
            .iter()
            .filter_map(|identity| self.atom(identity).and_then(|atom| self.spec(atom)))
    }

    #[cfg(test)]
    pub(crate) fn potential_specs(&self) -> impl Iterator<Item = &CardSpec> {
        self.potential_identities
            .iter()
            .filter_map(|identity| self.atom(identity).and_then(|atom| self.spec(atom)))
    }

    /// The admitted build this fight's document claimed, if any.
    pub fn game_build(&self) -> Option<GameBuild> {
        self.game_build
    }

    /// The fight's saved Mad Science variant, when the document holds a Mad
    /// Science ([`MadScienceVariant`]). The boundary re-emits it on every Mad
    /// Science card.
    pub fn mad_science_variant(&self) -> Option<MadScienceVariant> {
        self.mad_science_variant
    }

    /// Whether this fight's complete immutable closure contains Sovereign
    /// Blade. This is catalog reachability, not current-pile reachability.
    #[inline(always)]
    pub(crate) fn has_sovereign_blade(&self) -> bool {
        self.has_sovereign_blade
    }

    /// Whether `Misery` is reachable in this fight, and therefore whether the
    /// ordered physical attachment ledger has a reader at all (#2693 S1).
    #[inline(always)]
    pub(crate) fn misery_is_reachable(&self) -> bool {
        self.misery_is_reachable
    }

    /// See [`crate::engine::potions::GenerationPoolFacts`].
    pub(crate) fn generation_pools(&self) -> &crate::engine::potions::GenerationPoolFacts {
        self.generation_pools
            .0
            .get_or_init(|| crate::engine::potions::GenerationPoolFacts::of(self))
    }

    /// See the field of the same name.
    #[inline(always)]
    pub(crate) fn outbreak_reachable(&self) -> bool {
        self.outbreak_reachable
    }

    /// See the field of the same name.
    #[inline(always)]
    pub(crate) fn terminal_aware_root_step_reachable(&self) -> bool {
        self.terminal_aware_root_step_reachable
    }

    /// See [`crate::engine::admission::StampedeClosureFacts`].
    #[inline(always)]
    pub(crate) fn stampede_closure(&self) -> crate::engine::admission::StampedeClosureFacts {
        self.stampede_closure
    }

    /// Whether this fight's immutable catalog interns Normality.
    #[inline(always)]
    pub(crate) fn has_normality(&self) -> bool {
        self.has_normality
    }

    /// Whether this fight's immutable catalog interns Enthralled.
    #[inline(always)]
    pub(crate) fn has_enthralled(&self) -> bool {
        self.has_enthralled
    }

    /// Whether an exact physical AutoPost card listener can exist in this fight.
    #[inline(always)]
    pub(crate) fn has_auto_post_card_listener(&self) -> bool {
        self.has_auto_post_card_listener
    }

    /// Whether a Bolas/Thrumming Hatchet identity exists in this fight.
    #[inline(always)]
    pub(crate) fn has_batch139_self_return(&self) -> bool {
        self.has_batch139_self_return
    }

    /// Whether admitted public Play/EndTurn transitions use ActionReplay.
    #[inline(always)]
    pub(crate) fn requires_action_replay(&self) -> bool {
        self.requires_action_replay
    }

    /// Whether a no-result CardPlay Draw has a reachable blocking hook.
    #[inline(always)]
    pub(crate) fn cardplay_no_result_draw_can_suspend(&self) -> bool {
        self.cardplay_no_result_draw_can_suspend
    }

    #[inline(always)]
    pub(crate) fn cardplay_draw_hook_can_suspend(&self) -> bool {
        self.cardplay_draw_hook_can_suspend
    }

    pub(crate) fn dark_embrace_side_end_can_suspend(&self) -> bool {
        self.dark_embrace_side_end_can_suspend
    }

    pub(crate) fn after_card_exhausted_dark_can_suspend(&self) -> bool {
        self.after_card_exhausted_dark_can_suspend
    }

    /// Whether Entropy's persistent closure spans all five character pools in
    /// this fight. See [`entropy_transform_provenance_is_exact`] — this is
    /// consumers 3 and 4 reading the decision back.
    pub(crate) fn entropy_transform_closure_expands(&self) -> bool {
        !self.entropy_transform_provably_inert
    }

    pub(crate) fn with_action_replay_required(mut self) -> Self {
        self.requires_action_replay = true;
        self
    }

    pub(crate) fn same_content_ignoring_replay_root(&self, other: &Self) -> bool {
        let mut left = self.clone();
        let mut right = other.clone();
        left.requires_action_replay = false;
        right.requires_action_replay = false;
        left.cardplay_draw_hook_can_suspend = false;
        right.cardplay_draw_hook_can_suspend = false;
        left == right
    }

    /// Whether `host` is a catalog of the same fight that **covers** this one:
    /// every identity, monster and capability this catalog carries is present
    /// in `host`, compiled identically, with no atom numbering compared.
    ///
    /// This exists for the receipt-minting gate in
    /// `engine::apply_action_into_legacy`, which asks one question of the
    /// catalog rebuilt from a depth-zero predecessor document: *is the engine
    /// running the fight this document describes?* Two things make equality
    /// the wrong test for that:
    ///
    /// * **Atom numbering.** [`Self::same_content_ignoring_replay_root`] is
    ///   derived `PartialEq` over the arenas, so it compares which
    ///   [`CardAtom`] each identity was interned as — and interning follows
    ///   the order a canonical document's piles are walked. On
    ///   `73WAG17274JJ@6` two identities had simply swapped atoms 8 and 9
    ///   after a card moved Hand -> Play, with identical identity, reachable
    ///   and potential sets. Numbering cannot matter to a receipt, because a
    ///   receipt is a *document*: `boundary::catalog_from_canonical` recurses
    ///   into a stored predecessor and builds the loaded catalog from it
    ///   (`boundary.rs:11298-11324`), so the pair
    ///   `boundary::authenticate_action_replay` compares is produced from one
    ///   document by construction. (Numbering IS load-bearing there —
    ///   that authenticator splices independently replayed frames, whose
    ///   `HotCard`s carry the predecessor catalog's atoms — which is why this
    ///   is an addition beside the equality, not a replacement for it.)
    ///
    /// * **Narrowing.** The engine builds one catalog per session, from the
    ///   *entry* document. A fight's identity closure only shrinks as cards
    ///   leave the live piles, so a mid-fight rebuild is a strict subset of
    ///   the session's, and every real mid-fight park failed an equality test
    ///   for that reason alone — #2474, 11 of 318 captured fights carrying
    ///   `continuations: length python=2 rust=1` against a Python that
    ///   installs the root unconditionally on a parked public action — the
    ///   `replay_root_candidate` arm of `apply_action` (frozen Python, deleted #2827), which carries no catalog condition at all.
    ///
    /// Coverage is the direction that still catches what the gate was built to
    /// catch. A unit-level body test's hand-built catalog interns a handful of
    /// cards; the document it canonicalises to implies the full closure, so
    /// the rebuild is a strict *superset* of the hand-built one and coverage
    /// fails — those continuations stay rootless exactly as before.
    ///
    /// Cold, allocating, and O(program size): it resolves both step and move
    /// programs out of their span arenas before comparing them, because a span
    /// is an arena offset and offsets move when atoms do.
    pub(crate) fn fight_is_covered_by(&self, host: &Self) -> bool {
        // Immutable per-fight provenance: never narrows, so equality.
        if self.game_build != host.game_build
            || self.ascension_below_modeled != host.ascension_below_modeled
            || self.splash_unlock_epochs != host.splash_unlock_epochs
            || self.wire_unlock_epochs != host.wire_unlock_epochs
            || self.splash_character_mask != host.splash_character_mask
            || self.entropy_transform_provably_inert != host.entropy_transform_provably_inert
            || self.hooks != host.hooks
        {
            return false;
        }
        // Derived capabilities: a bit this document implies must be one the
        // engine is already running with. The converse is ordinary narrowing.
        // `requires_action_replay` and `cardplay_draw_hook_can_suspend` are
        // the two bits a receipt's own presence sets, and are skipped here
        // exactly as `same_content_ignoring_replay_root` skips them.
        let implied = [
            (self.has_sovereign_blade, host.has_sovereign_blade),
            (self.misery_is_reachable, host.misery_is_reachable),
            (self.has_normality, host.has_normality),
            (self.has_enthralled, host.has_enthralled),
            (
                self.has_auto_post_card_listener,
                host.has_auto_post_card_listener,
            ),
            (self.has_batch139_self_return, host.has_batch139_self_return),
            (
                self.cardplay_no_result_draw_can_suspend,
                host.cardplay_no_result_draw_can_suspend,
            ),
            (
                self.dark_embrace_side_end_can_suspend,
                host.dark_embrace_side_end_can_suspend,
            ),
            (
                self.after_card_exhausted_dark_can_suspend,
                host.after_card_exhausted_dark_can_suspend,
            ),
            (
                self.dark_embrace_ethereal_reachable,
                host.dark_embrace_ethereal_reachable,
            ),
            (self.hammer_time_reachable, host.hammer_time_reachable),
            (self.forge_command_reachable, host.forge_command_reachable),
        ];
        if implied.iter().any(|(mine, theirs)| *mine && !*theirs) {
            return false;
        }
        if !self
            .reachable_identities
            .iter()
            .all(|identity| host.is_reachable(*identity))
            || !self
                .potential_identities
                .iter()
                .all(|identity| host.is_potential(*identity))
        {
            return false;
        }
        for (identity, atom) in &self.by_identity {
            let Some(host_atom) = host.atom(identity) else {
                return false;
            };
            let (Some(mine), Some(theirs)) = (self.spec(*atom), host.spec(host_atom)) else {
                return false;
            };
            // Every field but the arena span, which is an offset, not content.
            let mut normalized = *mine;
            normalized.steps = theirs.steps;
            if normalized != *theirs {
                return false;
            }
            let (mine, theirs) = (self.steps(mine), host.steps(theirs));
            if mine.len() != theirs.len()
                || mine.iter().zip(theirs.iter()).any(|(mine, theirs)| {
                    mine.kind != theirs.kind
                        || resolve_args(&self.args, mine.args)
                            != resolve_args(&host.args, theirs.args)
                })
            {
                return false;
            }
        }
        self.by_monster.iter().all(|(kind, _)| {
            let (mine, theirs) = (self.moves(*kind), host.moves(*kind));
            mine.len() == theirs.len()
                && mine.iter().zip(theirs.iter()).all(|(mine, theirs)| {
                    mine.kind == theirs.kind
                        && mine.repeats == theirs.repeats
                        && resolve_args(&self.args, mine.args)
                            == resolve_args(&host.args, theirs.args)
                })
        })
    }

    /// Whether setup proved an execution-capable Hammer Time source.
    pub(crate) fn hammer_time_reachable(&self) -> bool {
        self.hammer_time_reachable
    }

    /// Whether setup proved an execution-capable Forge command source.
    pub(crate) fn forge_command_reachable(&self) -> bool {
        self.forge_command_reachable
    }

    /// Whether setup reached this exact identity from an executable local
    /// source. This is intentionally distinct from immutable catalog closure.
    pub(crate) fn is_reachable(&self, identity: CardIdentity) -> bool {
        self.reachable_identities.binary_search(&identity).is_ok()
    }

    /// Whether source-gated replay can mint this identity after the bounded
    /// canonical preview, without making it an unconditional semantic future.
    pub(crate) fn is_potential(&self, identity: CardIdentity) -> bool {
        self.potential_identities.binary_search(&identity).is_ok()
    }

    /// Whether a source-gated replay-capable Eidolon root can enter a generated
    /// body whose exact identity is known only through the potential set.
    pub(crate) fn eidolon_recursive_root_requires_rehearsal(&self) -> bool {
        !self.potential_identities.is_empty()
    }
}

/// Whether one exact card body can park for a player selection.
///
/// Most rows carry the generated `selects` bit. The specialized bodies below
/// own selections that are not represented by that legacy template field;
/// keep the complete cold classification here so every reachable replay
/// source installs an ActionReplay root before execution.
fn card_body_can_select(spec: &CardSpec) -> bool {
    spec.selects
        || spec.row.steps.iter().any(|step| {
            matches!(
                step.kind,
                StepKind::Select
                    | StepKind::ExhaustDraw
                    | StepKind::PurityExact
                    | StepKind::SeekerStrikeExact
                    | StepKind::AbundanceExact
                    | StepKind::DiscoveryExact
                    | StepKind::SplashExact
                    | StepKind::QuasarExact
                    | StepKind::GlimmerExact
                    | StepKind::EidolonExact
            )
        })
}

/// Why a catalog could not be built.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CatalogError {
    /// The content registry has no row for this `(id, upgrade)` pair.
    UnknownCardRow(CardId, u8),
    /// More distinct identities than a `u16` atom can address.
    AtomSpaceExhausted,
    /// A Mad Science identity was interned before the fight's saved Tinker
    /// Time variant was recorded ([`CatalogBuilder::set_mad_science_variant`]).
    MadScienceVariantUnknown,
    /// The fight's Mad Science variant is legal, but its body is not ported
    /// ([`crate::content_tables::MadScienceVariantRow::unmodeled`]).
    MadScienceVariantNotModeled(MadScienceVariant),
}

/// Interns identities while a fight's entry document is walked.
///
/// Allocation and ordered-map lookups are deliberate here: this runs once per
/// fight, off every hot path, and the resulting [`Catalog`] is index-only.
#[derive(Clone, Debug, Default)]
pub struct CatalogBuilder {
    index: BTreeMap<CardIdentity, CardAtom>,
    specs: Vec<CardSpec>,
    steps: Vec<CompiledStep>,
    moves: Vec<CompiledMove>,
    by_monster: Vec<(MonsterKind, Span)>,
    args: Vec<CompiledArg>,
    hooks: HookTable,
    game_build: Option<GameBuild>,
    splash_unlock_epochs: Option<Vec<&'static str>>,
    wire_unlock_epochs: Option<Vec<&'static str>>,
    splash_character_mask: u8,
    reachable_identities: BTreeSet<CardIdentity>,
    potential_identities: BTreeSet<CardIdentity>,
    hammer_time_reachable: bool,
    forge_command_reachable: bool,
    persistent_action_replay_required: bool,
    live_replay_modifier_mask: u8,
    stratagem_reachable: bool,
    hellraiser_reachable: bool,
    vicious_reachable: bool,
    dark_embrace_reachable: bool,
    ordinary_ethereal_reachable: bool,
    /// See [`Catalog::entropy_transform_provably_inert`]. `false` — the
    /// `Default` — means the Entropy closure expands.
    entropy_transform_provably_inert: bool,
    /// See [`Catalog::ascension`]; stored as the distance below
    /// [`crate::encounters::MODELED_ASCENSION`] so the `Default` is A10.
    ascension_below_modeled: u8,
    /// See [`Catalog::mad_science_variant`].
    mad_science_variant: Option<MadScienceVariant>,
}

impl CatalogBuilder {
    /// A builder with no identities interned and every stream unseeded.
    pub fn new() -> Self {
        Self::default()
    }

    /// The fight's `AscensionLevel` the move rows compile at (#2828).
    pub fn ascension(&self) -> u8 {
        crate::encounters::MODELED_ASCENSION - self.ascension_below_modeled
    }

    /// Record the fight's `AscensionLevel` before any monster is interned.
    ///
    /// Returns false, changing nothing, for a level outside
    /// `0..=MAX_ASCENSION`, or once a move row has been compiled at another
    /// level: rows already in the arena would keep the old tier, and a
    /// catalog that mixes two tiers is exactly what this must never build.
    pub fn set_ascension(&mut self, level: u8) -> bool {
        if level > crate::encounters::MAX_ASCENSION
            || (!self.moves.is_empty() && level != self.ascension())
        {
            return false;
        }
        self.ascension_below_modeled = crate::encounters::MODELED_ASCENSION - level;
        true
    }

    /// Record the document's claimed build provenance.
    pub fn set_game_build(&mut self, build: Option<GameBuild>) {
        self.game_build = build;
    }

    /// Record the fight's saved Mad Science variant before any Mad Science
    /// identity is interned ([`MadScienceVariant`]). Returns false, changing
    /// nothing, when a different variant is already recorded: one fight's
    /// copies cannot disagree natively, so two variants is a refusal.
    pub fn set_mad_science_variant(&mut self, variant: MadScienceVariant) -> bool {
        match self.mad_science_variant {
            Some(recorded) => recorded == variant,
            None => {
                self.mad_science_variant = Some(variant);
                true
            }
        }
    }

    /// The generated row one identity is interned from.
    ///
    /// Every id but Mad Science is its `(id, upgrade)` row of `CARD_ROWS`.
    /// Mad Science's `CARD_ROWS` entry is the frozen registry's placeholder —
    /// a union body none of the eighteen specs has — so it is never a spec:
    /// the row is the fight's variant row, and the variant must be recorded.
    pub(crate) fn row_of(&self, identity: CardIdentity) -> Result<&'static CardRow, CatalogError> {
        let unknown = CatalogError::UnknownCardRow(identity.id, identity.upgrade);
        if identity.id != CardId::MadScience {
            return card_row(identity.id, identity.upgrade).ok_or(unknown);
        }
        let variant = self
            .mad_science_variant
            .ok_or(CatalogError::MadScienceVariantUnknown)?;
        variant
            .row(identity.upgrade)
            .map_err(|_| CatalogError::MadScienceVariantNotModeled(variant))?
            .ok_or(unknown)
    }

    /// The generated row of an identity this builder already interned.
    fn interned_row(&self, identity: &CardIdentity) -> Option<&'static CardRow> {
        self.index
            .get(identity)
            .and_then(|atom| self.specs.get(*atom as usize))
            .map(|spec| spec.row)
    }

    /// Record a partial normalized unlock profile and derive its character
    /// mask. Interned epochs only: every entry must be a
    /// `UNLOCK_EPOCH_UNIVERSE_V1101` element, which the boundary enforces.
    pub(crate) fn set_splash_unlock_epochs(&mut self, epochs: Vec<&'static str>) {
        self.splash_character_mask = crate::steps::neutral::splash_character_mask(&epochs);
        self.splash_unlock_epochs = Some(epochs);
    }

    pub(crate) fn splash_unlock_epochs(&self) -> Option<&[&'static str]> {
        self.splash_unlock_epochs.as_deref()
    }

    /// Record the document's profile verbatim, for lossless re-emission.
    ///
    /// Independent of [`Self::set_splash_unlock_epochs`], which stores only a
    /// profile that actually changes a pool. See [`Catalog::wire_unlock_epochs`].
    pub(crate) fn set_wire_unlock_epochs(&mut self, epochs: Vec<&'static str>) {
        self.wire_unlock_epochs = Some(epochs);
    }

    /// The builder-time twin of [`Catalog::splash_attack_pool`]: the generated
    /// Splash closure is interned while the catalog is still being built.
    pub(crate) fn splash_attack_pool(&self, owner: RewardPool) -> Vec<CardId> {
        match self.splash_unlock_epochs.as_deref() {
            Some(epochs) => crate::steps::neutral::derive_splash_attack_pool(
                owner,
                epochs,
                self.splash_character_mask,
            ),
            None => crate::steps::neutral::splash_pool(owner),
        }
    }

    /// This fight's exact ordered owner-pool projections (#2542).
    ///
    /// The four generators that read "the owner's unlocked CharacterCardPool"
    /// all project the same generated rows; only the rarity/type/cost
    /// comparison differs. Routing them through the catalog keeps the epoch
    /// profile in one place, exactly as [`CatalogBuilder::splash_attack_pool`] does.
    pub(crate) fn owner_generation_pool(&self, owner: RewardPool) -> Vec<CardId> {
        crate::steps::neutral::owner_generation_pool(owner, self.splash_unlock_epochs.as_deref())
    }

    /// Exact ordered owner pool for a persistent BeforeHandDraw generator.
    pub(crate) fn owner_listener_pool(&self, owner: RewardPool, power: PowerId) -> Vec<CardId> {
        crate::steps::neutral::owner_listener_pool(
            owner,
            self.splash_unlock_epochs.as_deref(),
            power,
        )
    }

    /// The builder-time twin of [`Catalog::entropy_colorless_transform_pool`],
    /// read by the Entropy closure walk (#3122).
    pub(crate) fn entropy_colorless_transform_pool(&self) -> Vec<CardId> {
        crate::steps::neutral::entropy_colorless_transform_pool(
            self.splash_unlock_epochs.as_deref(),
        )
    }

    /// The builder-time twin of [`Catalog::colorless_generation_pool`], read
    /// by the Quasar / Spectrum Shift / Bundle of Joy / Manifest Authority
    /// closure walks and previews (#2560).
    pub(crate) fn colorless_generation_pool(&self) -> Vec<CardId> {
        crate::steps::neutral::colorless_generation_pool(self.splash_unlock_epochs.as_deref())
    }

    /// The builder-time twin of [`Catalog::jack_of_all_trades_pool`] (#2560).
    pub(crate) fn jack_of_all_trades_pool(&self) -> Vec<CardId> {
        crate::steps::neutral::jack_of_all_trades_pool(self.splash_unlock_epochs.as_deref())
    }

    /// Calamity's `Common..Rare` Attack projection of the owner pool.
    pub(crate) fn calamity_attack_pool(&self, owner: RewardPool) -> Vec<CardId> {
        crate::steps::neutral::calamity_owner_attack_pool(
            owner,
            self.splash_unlock_epochs.as_deref(),
        )
    }

    /// The owner's one-`CardType` generation pool under this fight's recorded
    /// profile: Distraction (Skill) and White Noise (Power). See
    /// [`crate::steps::neutral::owner_type_generation_pool`].
    pub(crate) fn owner_type_generation_pool(
        &self,
        owner: RewardPool,
        card_type: crate::content_tables::CardType,
    ) -> Vec<CardId> {
        crate::steps::neutral::owner_type_generation_pool(
            owner,
            self.splash_unlock_epochs.as_deref(),
            card_type,
        )
    }

    /// Crossbow's owner Attack pool under this fight's recorded profile
    /// (#2970). See [`crate::steps::neutral::crossbow_owner_attack_pool`].
    pub(crate) fn crossbow_attack_pool(&self, owner: RewardPool) -> Vec<CardId> {
        crate::steps::neutral::crossbow_owner_attack_pool(
            owner,
            self.splash_unlock_epochs.as_deref(),
        )
    }

    /// Metamorphosis' `FilterForCombat` Attack projection of the owner pool.
    pub(crate) fn metamorphosis_attack_pool(&self, owner: RewardPool) -> Vec<CardId> {
        crate::steps::neutral::metamorphosis_owner_attack_pool(
            owner,
            self.splash_unlock_epochs.as_deref(),
        )
    }

    /// Whether setup reached this identity from an executable local source.
    /// Catalog-only remote and Exhaust identities deliberately remain false.
    pub(crate) fn contains_reachable(&self, identity: CardIdentity) -> bool {
        self.reachable_identities.contains(&identity)
    }

    /// Intern one identity, returning its atom. Idempotent.
    ///
    /// The row's step program is **compiled here** (the #1297 rule): every
    /// string argument is resolved once, at fight setup, so the play path only
    /// ever meets typed values.
    pub fn intern(&mut self, identity: CardIdentity) -> Result<CardAtom, CatalogError> {
        self.intern_with_reachability(identity, false)
    }

    /// Intern one identity reached from an execution-capable physical or
    /// generated source. Re-visiting an already-interned identity still
    /// publishes provenance and recursively marks its immutable closure.
    pub(crate) fn intern_reachable(
        &mut self,
        identity: CardIdentity,
    ) -> Result<CardAtom, CatalogError> {
        self.intern_with_reachability(identity, true)
    }

    /// Intern one identity in the source-authorized runtime-totality set
    /// without promoting it into semantic future reachability.
    pub(crate) fn intern_potential(
        &mut self,
        identity: CardIdentity,
    ) -> Result<CardAtom, CatalogError> {
        let atom = self.intern(identity)?;
        self.potential_identities.insert(identity);
        Ok(atom)
    }

    /// Intern the exact L0/L1 executable pair used by the narrow Magi Dampen
    /// witness. The caller supplies one identity only to select card ID and
    /// enchantment; no unrelated identity becomes reachable.
    pub fn intern_dampen_card_pair(
        &mut self,
        identity: CardIdentity,
    ) -> Result<(CardAtom, CardAtom), CatalogError> {
        let l0 = self.intern_with_reachability(
            CardIdentity {
                upgrade: 0,
                ..identity
            },
            true,
        )?;
        let l1 = self.intern_with_reachability(
            CardIdentity {
                upgrade: 1,
                ..identity
            },
            true,
        )?;
        Ok((l0, l1))
    }

    /// Mark the source-absent live Furnace carrier as a future Forge command.
    pub(crate) fn mark_live_furnace_reachable(&mut self) {
        self.forge_command_reachable = true;
    }

    /// Record that this document's own provenance proves a native Entropy
    /// transform can never execute in this fight, so its five-class closure
    /// must not be interned. See [`entropy_transform_provenance_is_exact`].
    pub(crate) fn mark_entropy_transform_provably_inert(&mut self) {
        self.entropy_transform_provably_inert = true;
    }

    /// The builder-time twin of [`Catalog::entropy_transform_closure_expands`],
    /// read by the closure walk while the catalog is still being built.
    pub(crate) fn entropy_transform_closure_expands(&self) -> bool {
        !self.entropy_transform_provably_inert
    }

    /// Mark a source-less persistent AutoPre/AutoPost producer as reachable.
    pub(crate) fn mark_persistent_action_replay_required(&mut self) {
        self.persistent_action_replay_required = true;
    }

    /// Record one already-live `GeneratePlayCount` writer whose physical
    /// source may have left every pile before this cold catalog is rebuilt.
    pub(crate) fn mark_live_replay_modifier(&mut self, power: PowerId) {
        self.live_replay_modifier_mask |= match power {
            PowerId::Burst => 1,
            PowerId::EchoForm => 2,
            PowerId::OneTwoPunch => 4,
            PowerId::SignalBoost => 8,
            _ => return,
        };
    }

    /// Mark an already-live Stratagem whose source may have left every pile.
    /// Its blocking AfterShuffle selection can suspend an otherwise
    /// nonselecting CardPlay-owned Draw.
    pub(crate) fn mark_live_stratagem_reachable(&mut self) {
        self.stratagem_reachable = true;
    }

    /// Mark an already-live Hellraiser whose source may have left every pile.
    /// A selecting Strike drawn by an exact CardPlay-owned Draw can then park
    /// below an authenticated outer AutoPlay owner.
    pub(crate) fn mark_live_hellraiser_reachable(&mut self) {
        self.hellraiser_reachable = true;
    }

    /// Record an already-live Vicious whose source may have left every pile.
    /// Its awaited Draw can park only when an exact plain Vulnerable command
    /// owns the frozen AfterPowerAmountChanged continuation.
    pub(crate) fn mark_live_vicious_reachable(&mut self) {
        self.vicious_reachable = true;
    }

    /// Mark an already-live Dark Embrace whose source may have left every
    /// pile before the cold catalog is rebuilt.
    pub(crate) fn mark_live_dark_embrace_reachable(&mut self) {
        self.dark_embrace_reachable = true;
    }

    /// Mark an execution-capable physical card whose current local keyword or
    /// a live Hex owner makes it an ordinary turn-end Ethereal. Static row
    /// Ethereal is recorded by `intern_with_reachability` itself.
    pub(crate) fn mark_ordinary_ethereal_reachable(&mut self) {
        self.ordinary_ethereal_reachable = true;
    }

    fn intern_with_reachability(
        &mut self,
        identity: CardIdentity,
        reachable: bool,
    ) -> Result<CardAtom, CatalogError> {
        let row = self.row_of(identity)?;
        let newly_reachable = reachable && self.reachable_identities.insert(identity);
        if newly_reachable {
            self.stratagem_reachable |= matches!(identity.id, CardId::Stratagem);
            self.hellraiser_reachable |= matches!(identity.id, CardId::Hellraiser);
            self.vicious_reachable |= matches!(identity.id, CardId::Vicious);
            self.dark_embrace_reachable |= matches!(identity.id, CardId::DarkEmbrace);
            self.ordinary_ethereal_reachable |= row.ethereal
                && row.turn_end_dmg == 0
                && row.turn_end_hp_loss == 0
                && row.turn_end_weak == 0
                && row.turn_end_frail == 0
                && !row.turn_end_hp_loss_hand
                && !row
                    .steps
                    .iter()
                    .any(|step| step.kind == StepKind::TurnEndGoldLossExact);
            // Sculpting Strike OnPlay RVA 0x3b8cec IL_0187/IL_0189 applies
            // Ethereal to a chosen Hand card (#3022).
            self.ordinary_ethereal_reachable |=
                matches!(identity.id, CardId::CallOfTheVoid | CardId::SculptingStrike);
            self.hammer_time_reachable |=
                crate::steps::regent_forge::hammer_time_program_is_exact(row);
            self.forge_command_reachable |=
                crate::steps::regent_forge::forge_writer_program_is_supported(row)
                    || crate::steps::regent_forge::furnace_program_is_exact(row);
        }
        let atom = if let Some(atom) = self.index.get(&identity).copied() {
            if !newly_reachable {
                return Ok(atom);
            }
            atom
        } else {
            let atom = CardAtom::try_from(self.specs.len()).map_err(|_| {
                // Unreachable with 596 ids x upgrade levels, but the cast is not
                // allowed to be the thing that decides.
                CatalogError::AtomSpaceExhausted
            })?;
            let steps = self.compile_steps(row.steps);
            self.specs.push(CardSpec::from_row(identity, row, steps));
            self.index.insert(identity, atom);
            atom
        };
        // Both Shiv generators create the canonical L0 Shiv. Hidden Daggers+
        // then upgrades the returned physical instances, so its L1 row also
        // needs the canonical L1 atom before hot play begins. Intern every
        // generated leaf now so minting and upgrade stay index-only hot
        // operations and admission can validate the complete closure up front.
        if row.steps.iter().any(|step| {
            matches!(
                step.kind,
                StepKind::GenerateFixedShivs
                    | StepKind::GenerateShivsThenInkyExact
                    | StepKind::GenerateShivsThenUpgradeExact
                    | StepKind::InfiniteBlades
                    | StepKind::StormOfSteelExact
            )
        }) {
            self.intern_with_reachability(
                CardIdentity {
                    id: CardId::Shiv,
                    upgrade: 0,
                    enchantment: None,
                },
                reachable,
            )?;
            if row.steps.iter().any(|step| {
                matches!(
                    step.kind,
                    StepKind::GenerateShivsThenUpgradeExact | StepKind::StormOfSteelExact
                ) && identity.upgrade == 1
            }) {
                self.intern_with_reachability(
                    CardIdentity {
                        id: CardId::Shiv,
                        upgrade: 1,
                        enchantment: None,
                    },
                    reachable,
                )?;
            }
            if row
                .steps
                .iter()
                .any(|step| step.kind == StepKind::GenerateShivsThenInkyExact)
            {
                for upgrade in 0..=1 {
                    self.intern_with_reachability(
                        CardIdentity {
                            id: CardId::Shiv,
                            upgrade,
                            enchantment: Some(CardEnchantment {
                                id: EnchantmentId::Inky,
                                amount: 1,
                            }),
                        },
                        reachable,
                    )?;
                }
            }
        }
        // Unit C's fixed-Status programs carry their generated identity in
        // the immutable step operands. Close the catalog over every such
        // exact leaf before hot play. Gunk Up's fused body is the one compact
        // exception: its canonical operands are damage/count, while the body
        // owns the implicit L0 Slimed identity.
        for step in row.steps {
            // SoulBody owns a fixed Soul payload whose level is determined by
            // the exact source row (only Reave+ produces Soul+). Keep both
            // hot minting and generator-recursive admission index-only.
            if step.kind == StepKind::SoulBody {
                self.intern_with_reachability(
                    CardIdentity {
                        id: CardId::Soul,
                        upgrade: u8::from(matches!(
                            identity,
                            CardIdentity {
                                id: CardId::Reave,
                                upgrade: 1,
                                ..
                            }
                        )),
                        enchantment: None,
                    },
                    reachable,
                )?;
            }
            // Dirge owns the same fixed Soul model family but carries it in
            // its fused X-cost command rather than SoulBody.  The source row
            // freezes the generated level (Dirge+ is the sole L1 writer).
            if matches!(step.kind, StepKind::DirgeXExact) {
                self.intern_with_reachability(
                    CardIdentity {
                        id: CardId::Soul,
                        upgrade: identity.upgrade,
                        enchantment: None,
                    },
                    reachable,
                )?;
            }
            // Glimpse Beyond owns the same fixed generated family through a
            // different command shape: both exact rows create only canonical
            // L0 Souls for every living same-side Player. Attach that leaf to
            // the immutable writer row so physical sources and every
            // recursively generated Glimpse identity expose one identical,
            // admission-visible catalog closure before hot play.
            // Seance TransformTo<Soul> (v111 0x3b8ed8 -> 0x3e15f0)
            // creates a fresh canonical L0 Soul, even for an upgraded source
            // or selected card. Carry that leaf through all recursive pools.
            if step.kind == StepKind::GlimpseBeyondExact
                || (step.kind == StepKind::Select && matches!(identity.id, CardId::Seance))
            {
                self.intern_with_reachability(
                    CardIdentity {
                        id: CardId::Soul,
                        upgrade: 0,
                        enchantment: None,
                    },
                    reachable,
                )?;
            }
            if step.kind == StepKind::Select && matches!(identity.id, CardId::Charge) {
                // Charge upgrades AFTER the singular L0 transformation.
                for upgrade in 0..=identity.upgrade {
                    self.intern_with_reachability(
                        CardIdentity {
                            id: CardId::MinionDiveBomb,
                            upgrade,
                            enchantment: None,
                        },
                        reachable,
                    )?;
                }
            }
            if step.kind == StepKind::Select
                && matches!(identity.id, CardId::Begone | CardId::Guards)
            {
                self.intern_with_reachability(
                    CardIdentity {
                        id: if matches!(identity.id, CardId::Begone) {
                            CardId::MinionStrike
                        } else {
                            CardId::MinionSacrifice
                        },
                        upgrade: identity.upgrade,
                        enchantment: None,
                    },
                    reachable,
                )?;
            }
            // Every admitted Forge source, including recursively generated
            // identities and Furnace's persistent reader, can mint the same
            // canonical unenchanted L0 Sovereign Blade. Close that leaf here
            // so admission and every later Forge stay index-only.
            if step.kind == StepKind::Furnace
                || (step.kind == StepKind::HammerTimeExact
                    && crate::steps::regent_forge::hammer_time_program_is_exact(row))
                || (step.kind == StepKind::ForgeFamilyExact
                    && crate::steps::regent_forge::forge_writer_program_is_supported(row))
            {
                self.intern_with_reachability(
                    CardIdentity {
                        id: CardId::SovereignBlade,
                        upgrade: 0,
                        enchantment: None,
                    },
                    reachable,
                )?;
            }
            if let (
                StepKind::GenerateFixedStatus,
                [Arg::List([Arg::Card(id), Arg::I(upgrade)]), Arg::S(_)],
            ) = (step.kind, step.args)
            {
                let upgrade: u8 = (*upgrade)
                    .try_into()
                    .map_err(|_| CatalogError::UnknownCardRow(*id, u8::MAX))?;
                self.intern_with_reachability(
                    CardIdentity {
                        id: *id,
                        upgrade,
                        enchantment: None,
                    },
                    reachable,
                )?;
            }
            if step.kind == StepKind::GunkUpExact {
                self.intern_with_reachability(
                    CardIdentity {
                        id: CardId::Slimed,
                        upgrade: 0,
                        enchantment: None,
                    },
                    reachable,
                )?;
            }
            if step.kind == StepKind::BladeSymphonyExact {
                self.intern_with_reachability(
                    CardIdentity {
                        id: CardId::Shiv,
                        upgrade: 0,
                        enchantment: None,
                    },
                    reachable,
                )?;
            }
            // Sentry Mode persists after its source leaves every pile and
            // generates one canonical L0 Sweeping Gaze per live Amount at
            // each owner BeforeHandDraw. Attach the fixed leaf to the writer
            // row itself so physical and recursively generated Sentry Mode
            // sources expose the same complete admission-visible closure.
            if step.kind == StepKind::SentryMode {
                self.intern_with_reachability(
                    CardIdentity {
                        id: CardId::SweepingGaze,
                        upgrade: 0,
                        enchantment: None,
                    },
                    reachable,
                )?;
            }
            // Primal Force transforms every eligible physical Hand Attack
            // into one fresh Giant Rock at the source level. Close this
            // deterministic replacement here so physical and generated
            // Primal Force sources share one cold, admission-visible rule.
            if step.kind == StepKind::PrimalForceExact {
                self.intern_with_reachability(
                    CardIdentity {
                        id: CardId::GiantRock,
                        upgrade: identity.upgrade,
                        enchantment: None,
                    },
                    reachable,
                )?;
            }
            // Compact's one plural Transform constructs a fresh Fuel at the
            // source level for every frozen Discard Status. Keep the fixed
            // leaf attached to the row itself: generation-preview interning
            // can create Compact transitively, and that generated physical
            // copy must carry the same later-play closure as an entry-pile
            // source without a second hand-maintained boundary list.
            if step.kind == StepKind::CompactExact {
                self.intern_with_reachability(
                    CardIdentity {
                        id: CardId::Fuel,
                        upgrade: identity.upgrade,
                        enchantment: None,
                    },
                    reachable,
                )?;
            }
        }
        // Knife Trap+ upgrades every frozen live Shiv immediately before its
        // nested play. Upgrade preserves the complete enchantment identity,
        // so close over every live L0 Shiv rather than only the generated
        // unenchanted leaf. Handle both canonical pile orders: Knife Trap+
        // may be interned before or after a Shiv.
        if matches!((identity.id, identity.upgrade), (CardId::KnifeTrap, 1)) {
            let shivs = self
                .index
                .keys()
                .copied()
                .filter(|candidate| matches!((candidate.id, candidate.upgrade), (CardId::Shiv, 0)))
                .collect::<Vec<_>>();
            for shiv in shivs {
                self.intern_with_reachability(CardIdentity { upgrade: 1, ..shiv }, reachable)?;
            }
        } else if matches!((identity.id, identity.upgrade), (CardId::Shiv, 0))
            && self.index.keys().any(|candidate| {
                matches!((candidate.id, candidate.upgrade), (CardId::KnifeTrap, 1))
            })
        {
            self.intern_with_reachability(
                CardIdentity {
                    upgrade: 1,
                    ..identity
                },
                reachable,
            )?;
        }
        // `hand_cap_body("aoe_debris", damage)` owns an implicit fixed L0
        // Debris mint. The historical compact body carries only its mode and
        // damage operand, so derive the cold catalog closure from that exact
        // generated shape rather than making the hot body perform a lookup.
        // False-positive interning for a malformed owner is harmless:
        // admission still rejects the owner shape before play.
        if row.steps.iter().any(|step| {
            step.kind == StepKind::HandCapBody
                && matches!(step.args, [Arg::S("aoe_debris"), Arg::I(_)])
        }) {
            self.intern_with_reachability(
                CardIdentity {
                    id: CardId::Debris,
                    upgrade: 0,
                    enchantment: None,
                },
                reachable,
            )?;
        }
        // GOOPY's per-play Amount growth rewrites the slot-2 identity; see
        // [`GOOPY_AMOUNT_CEILING`]. Only an eligible Defend owner grows, so
        // a malformed owner (which admission refuses) mints nothing.
        if let Some(amount) = goopy_amount(identity)
            && goopy_card_is_eligible(identity.id)
            && (0..GOOPY_AMOUNT_CEILING).contains(&amount)
        {
            self.intern_with_reachability(
                CardIdentity {
                    enchantment: Some(CardEnchantment {
                        id: EnchantmentId::Goopy,
                        amount: amount + 1,
                    }),
                    ..identity
                },
                reachable,
            )?;
        }
        Ok(atom)
    }

    /// Intern every result of one reachable Jackpot level.
    ///
    /// Jackpot samples with replacement and does not Exhaust, so its complete
    /// owner pool at the source level is the smallest exact cold closure.
    pub(crate) fn intern_jackpot_full_closure(
        &mut self,
        upgrade: u8,
        jackpot_pool: &[CardId],
    ) -> Result<(), CatalogError> {
        for id in jackpot_pool {
            self.intern_reachable(CardIdentity {
                id: *id,
                upgrade,
                enchantment: None,
            })?;
        }
        Ok(())
    }

    /// Intern every repeatedly-upgraded Attack identity reachable by
    /// Aggression from the catalog's current immutable card closure.
    ///
    /// Aggression shuffles live discard indices, moves the selected physical
    /// cards, and upgrades each one once. The same card can return to Discard
    /// and be selected on later turns, so this is a transitive closure through
    /// every generated upgrade row, not merely `L0 -> L1`.
    pub fn intern_aggression_upgrade_closure(&mut self) -> Result<(), CatalogError> {
        loop {
            let identities: Vec<CardIdentity> = self.index.keys().copied().collect();
            let mut added = false;
            for identity in identities {
                let Ok(row) = self.row_of(identity) else {
                    continue;
                };
                let is_attack = row.playable
                    && !row.is_power
                    && !row.is_skill
                    && row.turn_end_dmg == 0
                    && row.turn_end_hp_loss == 0
                    && row.turn_end_weak == 0
                    && row.turn_end_frail == 0
                    && !row.turn_end_hp_loss_hand
                    && !row.is_status_curse;
                let Some(next_upgrade) = identity.upgrade.checked_add(1) else {
                    continue;
                };
                let upgraded = CardIdentity {
                    id: identity.id,
                    upgrade: next_upgrade,
                    enchantment: identity.enchantment,
                };
                let reachable = self.contains_reachable(identity);
                if is_attack
                    && card_row(identity.id, next_upgrade).is_some()
                    && (!self.index.contains_key(&upgraded)
                        || (reachable && !self.contains_reachable(upgraded)))
                {
                    self.intern_with_reachability(upgraded, reachable)?;
                    added = true;
                }
            }
            if !added {
                return Ok(());
            }
        }
    }

    /// Intern every transitive one-level upgrade reachable from this catalog.
    ///
    /// Armaments+ and Drain Power can return through Discard/Draw, while one
    /// Apotheosis pass sees every live pile and every identity already added
    /// by a prefix listener. A reachable source therefore needs the same
    /// transitive closure over every physical or generated identity in the
    /// fight catalog. This setup-only closure deliberately walks until no next
    /// generated row exists. The independent native-upgrade census pins
    /// complete row coverage for this build. Enchantments are immutable and
    /// therefore remain part of every successor identity.
    pub fn intern_all_card_upgrade_closure(&mut self) -> Result<(), CatalogError> {
        loop {
            let identities = self.index.keys().copied().collect::<Vec<_>>();
            let mut added = false;
            for identity in identities {
                let Some(next_upgrade) = identity.upgrade.checked_add(1) else {
                    continue;
                };
                if card_row(identity.id, next_upgrade).is_none() {
                    continue;
                }
                let upgraded = CardIdentity {
                    upgrade: next_upgrade,
                    ..identity
                };
                let reachable = self.contains_reachable(identity);
                if !self.index.contains_key(&upgraded)
                    || (reachable && !self.contains_reachable(upgraded))
                {
                    self.intern_with_reachability(upgraded, reachable)?;
                    added = true;
                }
            }
            if !added {
                return Ok(());
            }
        }
    }

    /// DampenPower.AfterApplied (v111 RVA 0xa1198) calls Downgrade on
    /// every upgraded physical card. Close L0 counterparts after generated
    /// pools and upgrades have been interned; preserve their reachability.
    pub(crate) fn intern_dampen_downgrade_closure(&mut self) -> Result<(), CatalogError> {
        loop {
            let identities = self.index.keys().copied().collect::<Vec<_>>();
            let mut added = false;
            for identity in identities
                .into_iter()
                .filter(|identity| identity.upgrade > 0)
            {
                let base = CardIdentity {
                    upgrade: 0,
                    ..identity
                };
                let reachable = self.contains_reachable(identity);
                if !self.index.contains_key(&base) || (reachable && !self.contains_reachable(base))
                {
                    self.intern_with_reachability(base, reachable)?;
                    added = true;
                }
                if self.potential_identities.contains(&identity) {
                    self.potential_identities.insert(base);
                }
            }
            if !added {
                return Ok(());
            }
        }
    }

    fn compile_steps(&mut self, steps: &'static [Step]) -> Span {
        let mut compiled: Vec<CompiledStep> = Vec::with_capacity(steps.len());
        for step in steps {
            let classes = crate::engine::admission::step_word_classes(step.kind);
            let args = compile_args(step.args, &mut self.args, &classes);
            compiled.push(CompiledStep {
                kind: step.kind,
                args,
            });
        }
        let start = self.steps.len();
        self.steps.extend(compiled);
        Span::of(start, steps.len())
    }

    fn compile_moves(&mut self, moves: &'static [Move]) -> Span {
        let mut compiled: Vec<CompiledMove> = Vec::with_capacity(moves.len());
        let ascension = self.ascension();
        for entry in moves {
            let classes = crate::engine::admission::move_word_classes(entry.kind);
            let args = compile_move_args(entry.args, &mut self.args, &classes, ascension);
            compiled.push(CompiledMove {
                kind: entry.kind,
                args,
                repeats: entry.repeats,
            });
        }
        let start = self.moves.len();
        self.moves.extend(compiled);
        Span::of(start, moves.len())
    }

    /// Intern one monster kind's executable generated move table. Idempotent.
    ///
    /// Deterministic loops are always executable. A random table is compiled
    /// only when its kind is in admission's named allowlist; refused machines
    /// retain an empty span, so their unreachable mints cannot leak into the
    /// fight's card-reachability closure.
    pub fn intern_monster(&mut self, kind: MonsterKind) -> Result<(), CatalogError> {
        self.intern_monster_with_private_foundation(kind, false, false, false, false)
    }

    /// Compile the one complete Strangler row for the authenticated private
    /// Constrict foundation without publishing its AI or move capability.
    ///
    /// Registration is first-wins: a prior ordinary empty Strangler span
    /// cannot be upgraded. The boundary therefore selects this method on its
    /// first Strangler registration whenever `constrict_sources` is present.
    pub(crate) fn intern_private_constrict_strangler(&mut self) -> Result<(), CatalogError> {
        self.intern_monster_with_private_foundation(
            MonsterKind::SlitheringStrangler,
            true,
            false,
            false,
            false,
        )
    }

    /// Compile Hunter Killer's exact three-state machine for canonical
    /// private Tender state without publishing its move or AI capabilities.
    pub(crate) fn intern_private_tender_hunter(&mut self) -> Result<(), CatalogError> {
        self.intern_monster_with_private_foundation(
            MonsterKind::HunterKiller,
            false,
            true,
            false,
            false,
        )
    }

    /// Compile Spectral Knight's table only for the exact public R42 Hex
    /// move-state witness. Ordinary canonical construction continues through
    /// [`Self::intern_monster`], where this unsupported random-AI encounter
    /// remains uncompiled and therefore refused.
    #[doc(hidden)]
    pub fn intern_spectral_hex_smoke_foundation(&mut self) -> Result<(), CatalogError> {
        self.intern_monster_with_private_foundation(
            MonsterKind::SpectralKnight,
            false,
            false,
            true,
            false,
        )
    }

    /// Compile Magi Knight's table only for the exact turn-two Dampen witness.
    #[doc(hidden)]
    pub fn intern_magi_dampen_smoke_foundation(&mut self) -> Result<(), CatalogError> {
        self.intern_monster_with_private_foundation(
            MonsterKind::MagiKnight,
            false,
            false,
            false,
            true,
        )
    }

    /// Compile Aeonglass and the sole exact generated Wither identity for the
    /// public R48 constructor/cycle witness.
    #[doc(hidden)]
    pub fn intern_aeonglass_smoke_foundation(&mut self) -> Result<(), CatalogError> {
        self.intern_monster(MonsterKind::Aeonglass)?;
        self.intern_with_reachability(
            CardIdentity {
                id: CardId::Wither,
                upgrade: 0,
                enchantment: None,
            },
            true,
        )?;
        Ok(())
    }

    fn intern_monster_with_private_foundation(
        &mut self,
        kind: MonsterKind,
        private_constrict: bool,
        private_tender: bool,
        private_spectral_hex: bool,
        private_magi_dampen: bool,
    ) -> Result<(), CatalogError> {
        if self
            .by_monster
            .iter()
            .any(|(interned, _)| *interned == kind)
        {
            return Ok(());
        }
        if kind == MonsterKind::PhrogParasite {
            self.intern_monster(MonsterKind::Wriggler)?;
        }
        match kind {
            MonsterKind::LivingFog => self.intern_monster(MonsterKind::GasBomb)?,
            MonsterKind::Ovicopter => self.intern_monster(MonsterKind::ToughEgg)?,
            MonsterKind::GremlinMerc => {
                self.intern_monster(MonsterKind::SneakyGremlin)?;
                self.intern_monster(MonsterKind::FatGremlin)?;
            }
            MonsterKind::TheObscura => {
                self.intern_monster(MonsterKind::Parafright)?;
            }
            MonsterKind::Fogmog => {
                self.intern_monster(MonsterKind::EyeWithTeeth)?;
            }
            MonsterKind::Fabricator => {
                for bot in [
                    MonsterKind::Guardbot,
                    MonsterKind::Noisebot,
                    MonsterKind::Stabbot,
                    MonsterKind::Zapbot,
                ] {
                    self.intern_monster(bot)?;
                }
            }
            _ => {}
        }
        let executable_random = random_moves(kind).filter(|_| {
            crate::engine::admission::random_ai_is_allowlisted(kind)
                || (private_constrict && kind == MonsterKind::SlitheringStrangler)
                || (private_tender && kind == MonsterKind::HunterKiller)
                || (private_spectral_hex && kind == MonsterKind::SpectralKnight)
                || (private_magi_dampen && kind == MonsterKind::MagiKnight)
        });
        // SlumberingBeetle::RolloutMove 0x36ac0c attacks for
        // `get_RolloutDamage` (RVA 0xbe4f4, GetValueIfAscension(9, 18, 16),
        // compiled at the fight's tier like any table row, #2828), then
        // applies Strength(2). Its SNORE/WAKE states are private overrides;
        // the ordinary awake move repeats without RNG.
        const BEETLE_ROLLOUT: &[Move] = &[Move {
            name: "ROLL_OUT_MOVE",
            kind: MoveKind::AttackStrength,
            args: &[
                Arg::Tier(crate::content_tables::move_constants::SLUMBERING_BEETLE_ROLLOUT_DAMAGE),
                Arg::I(1),
                Arg::I(2),
            ],
            repeats: crate::content_tables::Repeats::Absent,
        }];
        let native_loop = (kind == MonsterKind::SlumberingBeetle).then_some(BEETLE_ROLLOUT);
        let span = match monster_loop(kind).or(executable_random).or(native_loop) {
            Some(moves) => {
                // Build the immutable mint closure before compiling the
                // moves. A two-word nested `(CardId, upgrade)` tuple is the
                // generated tables' typed card-identity encoding.
                let mut identities = Vec::new();
                for entry in moves {
                    collect_minted_identities(entry.args, &mut identities);
                }
                // `add_status` is the one historical compact move whose card
                // identity is implicit in its kind rather than its args.
                if moves
                    .iter()
                    .any(|entry| matches!(entry.kind, MoveKind::AddStatus | MoveKind::Wriggle))
                {
                    identities.push(CardIdentity {
                        id: CardId::Infection,
                        upgrade: 0,
                        enchantment: None,
                    });
                }
                // Haunted Ship's HAUNT, Noisebot's NOISE, and Entomancer's
                // Hive reader have the same implicit-card shape: their move
                // args do not carry the identity, while the minted card is
                // fixed to L0 Dazed.
                if moves.iter().any(|entry| {
                    matches!(
                        entry.kind,
                        MoveKind::EntoSpit | MoveKind::Haunt | MoveKind::NoiseStatus
                    )
                }) {
                    identities.push(CardIdentity {
                        id: CardId::Dazed,
                        upgrade: 0,
                        enchantment: None,
                    });
                }
                // Vantom's DISMEMBER row carries only damage/hits/count; its
                // generated L0 Wound identity is fixed by the move class.
                if moves
                    .iter()
                    .any(|entry| entry.kind == MoveKind::AttackWounds)
                {
                    identities.push(CardIdentity {
                        id: CardId::Wound,
                        upgrade: 0,
                        enchantment: None,
                    });
                }
                // Test Subject's fixed generated identities are implicit in
                // its two move classes: Burning Growl mints Burn+0 and every
                // positive Multi Claw HP-loss result mints Wound+0.
                if moves
                    .iter()
                    .any(|entry| entry.kind == MoveKind::TestSubjectBurningGrowl)
                {
                    identities.push(CardIdentity {
                        id: CardId::Burn,
                        upgrade: 0,
                        enchantment: None,
                    });
                }
                if moves
                    .iter()
                    .any(|entry| entry.kind == MoveKind::TestSubjectMultiClaw)
                {
                    identities.push(CardIdentity {
                        id: CardId::Wound,
                        upgrade: 0,
                        enchantment: None,
                    });
                }
                // Soul Fysh's BECKON and GAZE carry only counts after the
                // optional attack; both mint the fixed generated Beckon+0
                // identity through separate physical-card transactions.
                if moves
                    .iter()
                    .any(|entry| matches!(entry.kind, MoveKind::AttackBeckon | MoveKind::Beckon))
                {
                    identities.push(CardIdentity {
                        id: CardId::Beckon,
                        upgrade: 0,
                        enchantment: None,
                    });
                }
                // The Insatiable's LIQUIFY args are empty; its six generated
                // Frantic Escape+0 identities are fixed by the move class.
                if moves
                    .iter()
                    .any(|entry| entry.kind == MoveKind::InsatiableLiquify)
                {
                    identities.push(CardIdentity {
                        id: CardId::FranticEscape,
                        upgrade: 0,
                        enchantment: None,
                    });
                }
                identities.sort_unstable();
                identities.dedup();
                for identity in identities {
                    self.intern_reachable(identity)?;
                }
                self.compile_moves(moves)
            }
            None => Span::default(),
        };
        self.by_monster.push((kind, span));
        Ok(())
    }

    /// Push a word no vocabulary resolves, so the admission gate's
    /// unresolved-word refusal can be exercised. Test-only: nothing in the
    /// generated tables can produce one, which is exactly why the refusal
    /// needs a synthetic way in.
    #[cfg(test)]
    pub(crate) fn push_unresolved_word_for_test(&mut self, word: &'static str) {
        self.args.push(CompiledArg::Unresolved(word));
    }

    /// Build this fight's hook subscriber tables from the relic inventory,
    /// **in inventory order** (D5; see [`crate::hooks`] for why the order is
    /// the spec and not an implementation detail).
    ///
    /// The canonical v2 schema carries the authoritative ordered relic
    /// inventory via `relics_entering` and `relics_entering_dispatch_ordered`,
    /// compiled here into the fight's hook subscriber table.
    pub fn set_relics(&mut self, relics: &[RelicId]) -> Result<(), HookRefusal> {
        self.set_relics_ordered(relics, false)
    }

    /// Build this fight's hook subscriber tables with explicit dispatch-order provenance.
    pub fn set_relics_ordered(
        &mut self,
        relics: &[RelicId],
        dispatch_ordered: bool,
    ) -> Result<(), HookRefusal> {
        self.hooks = HookTable::build_with_provenance(relics, dispatch_ordered)?;
        Ok(())
    }

    /// Freeze into the read-only per-fight catalog.
    ///
    /// The ordered builder map becomes a flat ascending vector: the lookup
    /// shape is a binary search over 16-byte entries, with no map nodes and
    /// no pointer chasing left in the read path.
    pub fn build(mut self) -> Catalog {
        let has_sovereign_blade = self
            .specs
            .iter()
            .any(|spec| matches!(spec.identity.id, CardId::SovereignBlade));
        let has_auto_post_card_listener = self.specs.iter().any(|spec| {
            matches!(
                spec.identity.id,
                CardId::HowlFromBeyond | CardId::IAmInvincible
            )
        });
        let has_batch139_self_return = self
            .specs
            .iter()
            .any(|spec| matches!(spec.identity.id, CardId::Bolas | CardId::ThrummingHatchet));
        let has_normality = self
            .specs
            .iter()
            .any(|spec| matches!(spec.identity.id, CardId::Normality));
        let has_enthralled = self
            .specs
            .iter()
            .any(|spec| matches!(spec.identity.id, CardId::Enthralled));
        let misery_is_reachable = self
            .reachable_identities
            .iter()
            .any(|identity| matches!(identity.id, CardId::Misery));
        let mut selecting_reachable = false;
        let mut selecting_skill_reachable = false;
        let mut selecting_attack_reachable = false;
        let mut selecting_power_reachable = false;
        let mut selecting_strike_reachable = false;
        for identity in &self.reachable_identities {
            let Some(spec) = self
                .index
                .get(identity)
                .and_then(|atom| self.specs.get(*atom as usize))
            else {
                continue;
            };
            if card_body_can_select(spec) {
                selecting_reachable = true;
                selecting_skill_reachable |= spec.is_skill;
                selecting_attack_reachable |= spec.is_attack;
                selecting_power_reachable |= spec.is_power;
                selecting_strike_reachable |= spec.strike_tag;
            }
        }
        let batch_producer_reachable = self.reachable_identities.iter().any(|identity| {
            self.index
                .get(identity)
                .and_then(|atom| self.specs.get(*atom as usize))
                .is_some_and(|spec| {
                    spec.row.steps.iter().any(|step| {
                        matches!(
                            step.kind,
                            StepKind::AutoplayDrawX
                                | StepKind::BeatDownExact
                                | StepKind::CalamityExact
                                | StepKind::CatastropheExact
                                | StepKind::DiscardHandDraw
                                | StepKind::EidolonExact
                                | StepKind::Havoc
                                | StepKind::HelloWorld
                                | StepKind::KnifeTrapExact
                                | StepKind::Mayhem
                                | StepKind::ScrapeExact
                                | StepKind::ShadowStepDiscardExact
                                | StepKind::StormOfSteelExact
                                | StepKind::UproarExact
                        ) || (step.kind == StepKind::Select
                            && matches!(
                                step.args.last(),
                                Some(Arg::Select(SelectOp::Discard | SelectOp::DiscardAll))
                            ))
                    })
                })
        });
        let replay_source_reachable = self.reachable_identities.iter().any(|identity| {
            // #3291: Transfigure writes BaseReplayCount on any Hand card
            // (`Transfigure/<OnPlay>d__9::MoveNext` RVA `0x3c4438`
            // IL_0161-IL_016f), so a selecting card can be replayed.
            matches!(identity.id, CardId::HiddenGem | CardId::Transfigure)
                || identity.enchantment.is_some_and(|enchantment| {
                    (matches!(enchantment.id, EnchantmentId::Glam) && enchantment.amount > 0)
                        || matches!(enchantment.id, EnchantmentId::Spiral)
                })
        });
        // Tutor's local Draw choice carries a physical uid and must always be
        // rooted before it can park. Unlike ordinary selector-only rows, its
        // canonical pending form deliberately authenticates the exact
        // depth-zero Player target through the ActionReplay predecessor.
        let tutor_reachable = self
            .reachable_identities
            .iter()
            .any(|identity| matches!(identity.id, CardId::Tutor));
        let cardplay_no_result_draw_reachable = self.reachable_identities.iter().any(|identity| {
            self.interned_row(identity).is_some_and(|row| {
                row.steps.iter().enumerate().any(|(index, _)| {
                    crate::engine::play::cardplay_no_result_draw_source_step_is_exact(row, index)
                })
            })
        });
        let cardplay_owned_tail_draw_reachable = self.reachable_identities.iter().any(|identity| {
            self.interned_row(identity).is_some_and(|row| {
                row.steps.iter().enumerate().any(|(index, _)| {
                    crate::engine::play::cardplay_owned_draw_tail_source_step_is_exact(row, index)
                })
            })
        });
        let cardplay_result_draw_reachable = self.reachable_identities.iter().any(|identity| {
            self.interned_row(identity).is_some_and(|row| {
                row.steps.iter().enumerate().any(|(index, _)| {
                    crate::engine::play::cardplay_result_draw_source_step_is_exact(row, index)
                })
            })
        });
        let cardplay_plain_vulnerable_reachable =
            self.reachable_identities.iter().any(|identity| {
                self.interned_row(identity).is_some_and(|row| {
                    let mut steps = row.steps.iter().enumerate().filter(|(index, _)| {
                        crate::engine::play::cardplay_plain_vulnerable_source_step_is_exact(
                            row, *index,
                        )
                    });
                    steps.next().is_some() && steps.next().is_none()
                })
            });
        let writer_reachable = |id| {
            self.reachable_identities
                .iter()
                .any(|identity| identity.id.eq(&id))
        };
        let generated_play_count_reachable = (selecting_skill_reachable
            && (self.live_replay_modifier_mask & 1 != 0 || writer_reachable(CardId::Burst)))
            || (selecting_reachable
                && (self.live_replay_modifier_mask & 2 != 0 || writer_reachable(CardId::EchoForm)))
            || (selecting_attack_reachable
                && (self.live_replay_modifier_mask & 4 != 0
                    || writer_reachable(CardId::OneTwoPunch)))
            || (selecting_power_reachable
                && (self.live_replay_modifier_mask & 8 != 0
                    || writer_reachable(CardId::SignalBoost)));
        let cardplay_draw_hook_can_suspend =
            self.stratagem_reachable || self.hellraiser_reachable && selecting_strike_reachable;
        let cardplay_no_result_draw_can_suspend = (cardplay_no_result_draw_reachable
            || cardplay_owned_tail_draw_reachable)
            && cardplay_draw_hook_can_suspend;
        let vicious_plain_vulnerable_can_suspend = self.vicious_reachable
            && cardplay_plain_vulnerable_reachable
            && (self.stratagem_reachable
                || self.hellraiser_reachable && selecting_strike_reachable);
        let dark_embrace_ethereal_reachable =
            self.ordinary_ethereal_reachable && self.dark_embrace_reachable;
        let dark_embrace_side_end_can_suspend = dark_embrace_ethereal_reachable
            && (self.stratagem_reachable
                || self.hellraiser_reachable && selecting_strike_reachable);
        let after_card_exhausted_dark_can_suspend = self.dark_embrace_reachable
            && (self.stratagem_reachable
                || self.hellraiser_reachable && selecting_strike_reachable);
        let ordinary_exhaust_reachable = self.reachable_identities.iter().any(|identity| {
            self.interned_row(identity).is_some_and(|row| {
                row.exhausts
                    || row.steps.iter().any(|step| {
                        matches!(
                            step.kind,
                            StepKind::Havoc
                                | StepKind::AutoplayDrawX
                                | StepKind::ExhaustDraw
                                | StepKind::ExhaustNonattacksBlock
                                | StepKind::ExhaustRandom
                                | StepKind::FiendFireExact
                                | StepKind::FlakCannonExact
                                | StepKind::PurityExact
                                | StepKind::StokeExact
                                | StepKind::ThrashExact
                        ) || matches!(
                            step.args.last(),
                            Some(crate::content_tables::Arg::Select(
                                crate::ids::SelectOp::Exhaust
                            ))
                        )
                    })
            })
        });
        // Replay-capable fights take the rooted path from their depth-zero
        // boundary. A selector-only fight can remain on the ordinary path,
        // but the public transition installs the same independent receipt if
        // it actually parks; importing that parked wire marks the capability
        // from the continuation itself.
        let enemy_choice_reachable = self
            .by_monster
            .iter()
            .any(|(kind, _)| *kind == MonsterKind::KnowledgeDemon);
        let requires_action_replay = self.specs.iter().any(|spec| {
            matches!(
                spec.identity.id,
                CardId::IAmInvincible | CardId::DecisionsDecisions
            )
        }) || tutor_reachable
            || enemy_choice_reachable
            || selecting_reachable
                && (batch_producer_reachable
                    || replay_source_reachable
                    || generated_play_count_reachable
                    || self.persistent_action_replay_required)
            || (cardplay_no_result_draw_reachable || cardplay_owned_tail_draw_reachable)
                && (selecting_reachable || self.stratagem_reachable)
            || cardplay_result_draw_reachable
                && (self.stratagem_reachable
                    || self.hellraiser_reachable && selecting_strike_reachable)
            || dark_embrace_side_end_can_suspend
            || ordinary_exhaust_reachable && after_card_exhausted_dark_can_suspend;
        // Centennial Puzzle's one-card Draws park on the same two hooks, from
        // any player-damage caller. The receipt owns the caller's suffix
        // (`engine::puzzle`, #3114), so every public action of such a fight
        // runs on the replay-capable path.
        let centennial_puzzle_draw_can_suspend = cardplay_draw_hook_can_suspend
            && self.hooks.owns(crate::ids::RelicId::RelicCentennialPuzzle);
        // The Swift enchantment's OnPlay Draw is receipt-owned the same way
        // (#3115).
        let swift_draw_can_suspend = cardplay_draw_hook_can_suspend
            && self.reachable_identities.iter().any(|identity| {
                identity.enchantment.is_some_and(|enchantment| {
                    matches!(enchantment.id, crate::ids::EnchantmentId::Swift)
                })
            });
        // Joss Paper's threshold Draw is receipt-owned the same way (#3201).
        // It has no once-per-combat latch, so owning the relic is enough.
        let joss_paper_draw_can_suspend =
            cardplay_draw_hook_can_suspend && self.hooks.owns(crate::ids::RelicId::RelicJossPaper);
        // Gremlin Horn's AfterDeath Draw begins its choice in a queued hook
        // action that parks rooted at the player action it followed (#3387,
        // `engine::hook_action`).
        let gremlin_horn_draw_can_suspend = cardplay_draw_hook_can_suspend
            && self.hooks.owns(crate::ids::RelicId::RelicGremlinHorn);
        // History Course's dupe AutoPlay is receipt-owned the same way
        // (#3309) whenever a reachable Attack can suspend under it. An Attack
        // Draw with a blocking hook already requires receipts through the
        // CardPlay Draw clauses above; a selecting Attack does not.
        let history_course_dupe_can_suspend =
            selecting_attack_reachable && self.hooks.owns(crate::ids::RelicId::RelicHistoryCourse);
        let requires_action_replay = requires_action_replay
            || history_course_dupe_can_suspend
            || vicious_plain_vulnerable_can_suspend
            || centennial_puzzle_draw_can_suspend
            || swift_draw_can_suspend
            || joss_paper_draw_can_suspend
            || gremlin_horn_draw_can_suspend;
        let by_identity: Vec<(CardIdentity, CardAtom)> = self.index.into_iter().collect();
        debug_assert!(by_identity.windows(2).all(|pair| pair[0].0 < pair[1].0));
        let reachable_identities: Vec<CardIdentity> =
            self.reachable_identities.into_iter().collect();
        debug_assert!(
            reachable_identities
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        let potential_identities: Vec<CardIdentity> =
            self.potential_identities.into_iter().collect();
        debug_assert!(
            potential_identities
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        // Interning order is document order; the read path binary-searches.
        self.by_monster
            .sort_unstable_by_key(|(kind, _)| *kind as u16);
        let mut catalog = Catalog {
            specs: self.specs,
            by_identity,
            steps: self.steps,
            moves: self.moves,
            by_monster: self.by_monster,
            args: self.args,
            hooks: self.hooks,
            game_build: self.game_build,
            ascension_below_modeled: self.ascension_below_modeled,
            splash_unlock_epochs: self.splash_unlock_epochs,
            wire_unlock_epochs: self.wire_unlock_epochs,
            splash_character_mask: self.splash_character_mask,
            has_sovereign_blade,
            misery_is_reachable,
            has_normality,
            has_enthralled,
            has_auto_post_card_listener,
            has_batch139_self_return,
            requires_action_replay,
            cardplay_no_result_draw_can_suspend,
            cardplay_draw_hook_can_suspend,
            dark_embrace_side_end_can_suspend,
            after_card_exhausted_dark_can_suspend,
            dark_embrace_ethereal_reachable,
            hammer_time_reachable: self.hammer_time_reachable,
            forge_command_reachable: self.forge_command_reachable,
            entropy_transform_provably_inert: self.entropy_transform_provably_inert,
            mad_science_variant: self.mad_science_variant,
            reachable_identities,
            potential_identities,
            stampede_closure: Default::default(),
            outbreak_reachable: false,
            terminal_aware_root_step_reachable: false,
            generation_pools: Derived::default(),
        };
        // Derived from the finished catalog's own reachable specs, steps and
        // args, none of which change after `build`.
        catalog.stampede_closure = crate::engine::admission::StampedeClosureFacts::of(&catalog);
        let outbreak_reachable = catalog
            .reachable_specs()
            .any(|spec| matches!(spec.identity.id, CardId::Outbreak));
        catalog.outbreak_reachable = outbreak_reachable;
        let terminal_aware_root_step_reachable = catalog.reachable_specs().any(|spec| {
            catalog.steps(spec).iter().any(|step| {
                matches!(
                    step.kind,
                    StepKind::AlchemizeExact
                        | StepKind::ConstellationExact
                        | StepKind::HuddleUpExact
                )
            })
        });
        catalog.terminal_aware_root_step_reachable = terminal_aware_root_step_reachable;
        catalog
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(id: CardId) -> CardIdentity {
        CardIdentity {
            id,
            upgrade: 0,
            enchantment: None,
        }
    }

    /// The build-time reachability facts read the execution-reachable specs
    /// only (#3420): a merely interned remote or Exhaust-only row sets none.
    #[test]
    fn build_time_reachability_facts_follow_reachable_specs_only() {
        let facts = |catalog: &Catalog| {
            (
                catalog.outbreak_reachable(),
                catalog.terminal_aware_root_step_reachable(),
            )
        };
        assert_eq!(facts(&CatalogBuilder::new().build()), (false, false));
        for (id, expected) in [
            (CardId::Outbreak, (true, false)),
            (CardId::Alchemize, (false, true)),
            (CardId::Constellation, (false, true)),
            (CardId::HuddleUp, (false, true)),
        ] {
            let mut reachable = CatalogBuilder::new();
            reachable.intern_reachable(plain(id)).unwrap();
            assert_eq!(facts(&reachable.build()), expected, "{id:?} reachable");

            let mut interned = CatalogBuilder::new();
            interned.intern(plain(id)).unwrap();
            let interned = interned.build();
            assert!(!interned.is_reachable(plain(id)));
            assert_eq!(facts(&interned), (false, false), "{id:?} interned only");
        }
    }

    /// #2942: the saved Tinker Time domain is exactly the nine pairs
    /// `TinkerTime::ChooseRiderEffect` can write, and nothing else.
    #[test]
    fn mad_science_variant_domain_is_the_nine_choose_rider_pairs() {
        let legal: Vec<(i64, i64)> = (-1..=11)
            .flat_map(|tinker_type| (-1..=11).map(move |rider| (tinker_type, rider)))
            .filter(|(tinker_type, rider)| {
                MadScienceVariant::from_saved(*tinker_type, *rider).is_some()
            })
            .collect();
        assert_eq!(
            legal,
            [
                (1, 1),
                (1, 2),
                (1, 3),
                (2, 4),
                (2, 5),
                (2, 6),
                (3, 7),
                (3, 8),
                (3, 9)
            ]
        );
        assert_eq!(MadScienceVariant::from_saved(i64::MAX, 1), None);
        let names: Vec<&str> = legal
            .iter()
            .map(|(t, r)| MadScienceVariant::from_saved(*t, *r).unwrap().rider_name())
            .collect();
        assert_eq!(
            names,
            [
                "Sapping",
                "Violence",
                "Choking",
                "Energized",
                "Wisdom",
                "Chaos",
                "Expertise",
                "Curious",
                "Improvement"
            ]
        );
    }

    /// #2942: a Mad Science identity is interned only under the fight's one
    /// variant; every level, the upgrade closure and every copy resolve to
    /// that variant's generated row, an unported variant refuses by name, and
    /// no other id is affected.
    #[test]
    fn mad_science_specs_are_the_fight_variant_rows() {
        let identity = plain(CardId::MadScience);
        let mut builder = CatalogBuilder::new();
        assert_eq!(
            builder.intern(identity),
            Err(CatalogError::MadScienceVariantUnknown)
        );
        let energized = MadScienceVariant::from_saved(2, 4).unwrap();
        assert!(builder.set_mad_science_variant(energized));
        assert!(builder.set_mad_science_variant(energized), "idempotent");
        assert!(
            !builder.set_mad_science_variant(MadScienceVariant::from_saved(2, 5).unwrap()),
            "one fight carries one variant"
        );
        let atom = builder.intern(identity).unwrap();
        builder.intern_all_card_upgrade_closure().unwrap();
        let strike = builder.intern(plain(CardId::StrikeIronclad)).unwrap();
        let catalog = builder.build();
        assert_eq!(catalog.mad_science_variant(), Some(energized));
        let base = catalog.spec(atom).unwrap();
        assert!(std::ptr::eq(base.row, energized.row(0).unwrap().unwrap()));
        assert!(base.is_skill && !base.targeted && !base.innate);
        let upgraded = catalog
            .spec(
                catalog
                    .atom(&CardIdentity {
                        upgrade: 1,
                        ..identity
                    })
                    .expect("the upgrade closure reaches Mad Science+"),
            )
            .unwrap();
        assert!(std::ptr::eq(
            upgraded.row,
            energized.row(1).unwrap().unwrap()
        ));
        assert!(
            upgraded.is_skill && upgraded.innate,
            "OnUpgrade adds Innate"
        );
        assert_eq!(
            catalog.spec(strike).unwrap().row,
            card_row(CardId::StrikeIronclad, 0).unwrap()
        );
        assert!(is_mad_science_variant_program(base, "Energized"));
        assert!(!is_mad_science_variant_program(base, "Wisdom"));
        assert!(!is_mad_science_variant_program(
            catalog.spec(strike).unwrap(),
            "Energized"
        ));

        // #3427 ported Curious (3, 8); Improvement is the one unported rider.
        let improvement = MadScienceVariant::from_saved(3, 9).unwrap();
        let mut builder = CatalogBuilder::new();
        builder.set_mad_science_variant(improvement);
        assert_eq!(
            builder.intern(identity),
            Err(CatalogError::MadScienceVariantNotModeled(improvement))
        );
    }

    /// #2942: the generated rows are the only Mad Science programs, the
    /// placeholder `CARD_ROWS` entry is none of them, and each ported body is
    /// built from exactly the op-language kinds the shared bodies admit.
    #[test]
    fn mad_science_variant_rows_are_the_generated_programs() {
        use crate::content_tables::{MAD_SCIENCE_VARIANT_ROWS, is_generated_card_row};
        let placeholder = card_row(CardId::MadScience, 0).unwrap();
        let mut kinds = BTreeSet::new();
        for variant in &MAD_SCIENCE_VARIANT_ROWS {
            match (&variant.row, variant.unmodeled) {
                (Some(row), None) => {
                    assert!(is_generated_card_row(row));
                    assert_ne!(row, placeholder);
                    assert_eq!(row.card_type as u8, variant.tinker_type);
                    kinds.extend(row.steps.iter().map(|step| step.kind));
                }
                (None, Some(reason)) => {
                    assert_eq!(variant.rider_name, "Improvement");
                    assert!(!reason.is_empty());
                }
                _ => panic!("a variant row is either ported or names why not"),
            }
        }
        assert_eq!(
            kinds,
            BTreeSet::from([
                StepKind::Attack,
                StepKind::Block,
                StepKind::Dexterity,
                StepKind::Draw,
                StepKind::Energy,
                StepKind::MadScienceChaosExact,
                StepKind::MadScienceCurious,
                StepKind::Strangle,
                StepKind::Strength,
                StepKind::Vulnerable,
                StepKind::Weak,
            ])
        );
        let mut forged = *placeholder;
        forged.steps = MAD_SCIENCE_VARIANT_ROWS[0].row.unwrap().steps;
        assert!(
            !is_generated_card_row(&forged),
            "a hybrid row is not generated"
        );
    }

    /// #3178: `RoyallyApproved::OnEnchant` RVA `0xd62c9` adds Innate and
    /// Retain, so every Royally Approved identity compiles both keywords at
    /// every upgrade level, and its bare twin keeps the row's own keywords.
    #[test]
    fn royally_approved_compiles_innate_and_retain_at_every_upgrade() {
        let mut builder = CatalogBuilder::new();
        let mut atoms = Vec::new();
        for id in [CardId::StrikeIronclad, CardId::DefendIronclad] {
            for upgrade in 0..=1 {
                let bare = CardIdentity {
                    id,
                    upgrade,
                    enchantment: None,
                };
                let royal = CardIdentity {
                    enchantment: Some(CardEnchantment {
                        id: EnchantmentId::RoyallyApproved,
                        amount: 1,
                    }),
                    ..bare
                };
                atoms.push((builder.intern(bare).unwrap(), false));
                atoms.push((builder.intern(royal).unwrap(), true));
            }
        }
        let catalog = builder.build();
        for (atom, royal) in atoms {
            let spec = catalog.spec(atom).unwrap();
            assert!(!spec.row.innate && !spec.row.retain, "{:?}", spec.identity);
            assert_eq!(spec.innate, royal, "{:?}", spec.identity);
            assert_eq!(spec.retain, royal, "{:?}", spec.identity);
            assert_eq!(
                royally_approved_adds_innate_and_retain(spec.identity),
                royal
            );
            // Nothing else about the compiled card moves.
            assert_eq!(spec.exhausts, spec.row.exhausts, "{:?}", spec.identity);
            assert_eq!(spec.ethereal, spec.row.ethereal, "{:?}", spec.identity);
        }
    }

    /// #2828: every `Arg::Tier` in every generated move table compiles to
    /// `GetValueIfAscension`'s tier at the level (`_level >= gate`,
    /// `AscensionManager::HasLevel` RVA `0x11fa83`), at every v0.111.0
    /// ascension, and every other argument compiles unchanged. A0 is below
    /// every gate and A10 at-or-above every gate.
    #[test]
    fn every_tiered_move_row_compiles_at_the_fights_tier() {
        let mut tiered = 0;
        for (_, rows) in crate::content_tables::LOOPS
            .iter()
            .chain(crate::content_tables::RANDOM_MOVES.iter())
        {
            for row in *rows {
                for ascension in 0..=crate::encounters::MAX_ASCENSION {
                    let mut arena = Vec::new();
                    let span = compile_move_args(row.args, &mut arena, &[], ascension);
                    let (start, end) = span.bounds();
                    for (arg, compiled) in row.args.iter().zip(&arena[start..end]) {
                        match arg {
                            Arg::Tier(tier) => {
                                let gate = tier.gate.expect("a move tier is gated");
                                let expected = if ascension >= gate {
                                    tier.at_or_above
                                } else {
                                    tier.below
                                };
                                assert_eq!(*compiled, CompiledArg::I(expected), "{}", row.name);
                                if ascension == 0 {
                                    tiered += 1;
                                }
                            }
                            Arg::I(value) => assert_eq!(*compiled, CompiledArg::I(*value)),
                            _ => {}
                        }
                    }
                }
            }
        }
        // The `LOOPS.*` and `_RANDOM_MOVES.*` rows of
        // `tools/move_constant_sites.py`, which codegen checks site by site.
        assert_eq!(tiered, 245, "tiered move arguments");
    }

    /// A tier outside a monster move row is a generator defect, so it
    /// compiles to a named `Unresolved` for admission to refuse; and a
    /// builder never mixes two tiers — its level is fixed once a row is in
    /// the arena, and a level outside `0..=10` is refused.
    #[test]
    fn a_catalog_compiles_its_move_rows_at_one_tier() {
        const STRAY: &[Arg] = &[Arg::Tier(
            crate::content_tables::move_constants::SOUL_FYSH_DE_GAS_DAMAGE,
        )];
        let mut arena = Vec::new();
        let span = compile_args(STRAY, &mut arena, &[]);
        assert_eq!(
            arena[span.bounds().0],
            CompiledArg::Unresolved("ascension tier outside a monster move")
        );

        let mut builder = CatalogBuilder::new();
        assert_eq!(builder.ascension(), crate::encounters::MODELED_ASCENSION);
        assert!(!builder.set_ascension(11));
        assert!(builder.set_ascension(8));
        builder.intern_monster(MonsterKind::SoulFysh).unwrap();
        assert!(!builder.set_ascension(9));
        assert!(builder.set_ascension(8));
        let a8 = builder.build();
        assert_eq!(a8.ascension(), 8);
        let de_gas = a8.moves(MonsterKind::SoulFysh)[1];
        assert_eq!(
            a8.args(de_gas.args),
            [CompiledArg::I(16), CompiledArg::I(1)]
        );

        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::SoulFysh).unwrap();
        let a10 = builder.build();
        assert_eq!(
            a10.args(a10.moves(MonsterKind::SoulFysh)[1].args)[0],
            CompiledArg::I(18)
        );
        // The same fight at two tiers is two programs.
        assert!(!a8.fight_is_covered_by(&a10));
        assert!(!a10.fight_is_covered_by(&a8));
        assert!(a8.fight_is_covered_by(&a8.clone()));
    }

    #[test]
    fn interning_is_idempotent_and_dense() {
        let mut builder = CatalogBuilder::new();
        let strike = builder.intern(plain(CardId::StrikeIronclad)).unwrap();
        let defend = builder.intern(plain(CardId::DefendIronclad)).unwrap();
        assert_eq!(strike, 0);
        assert_eq!(defend, 1);
        assert_eq!(
            builder.intern(plain(CardId::StrikeIronclad)).unwrap(),
            strike
        );
        let catalog = builder.build();
        assert_eq!(catalog.atom_count(), 2);
        assert_eq!(catalog.atom(&plain(CardId::StrikeIronclad)), Some(strike));
        assert_eq!(catalog.atom(&plain(CardId::DefendIronclad)), Some(defend));
        assert_eq!(catalog.atom(&plain(CardId::Bash)), None);
    }

    #[test]
    fn reachable_provenance_survives_an_earlier_catalog_only_identity_collision() {
        let mut builder = CatalogBuilder::new();
        let hammer = plain(CardId::HammerTime);
        let forge = plain(CardId::RefineBlade);
        let abundance = plain(CardId::Abundance);
        let discovery = plain(CardId::Discovery);
        builder.intern(hammer).unwrap();
        builder.intern(forge).unwrap();
        builder.intern(abundance).unwrap();
        builder.intern(discovery).unwrap();
        assert!(builder.index.contains_key(&hammer));
        assert!(!builder.contains_reachable(hammer));
        assert!(builder.index.contains_key(&forge));
        assert!(!builder.contains_reachable(forge));
        assert!(!builder.contains_reachable(abundance));
        assert!(!builder.contains_reachable(discovery));
        assert!(!builder.hammer_time_reachable);
        assert!(!builder.forge_command_reachable);

        let hammer_atom = builder.intern_reachable(hammer).unwrap();
        let forge_atom = builder.intern_reachable(forge).unwrap();
        let abundance_atom = builder.intern_reachable(abundance).unwrap();
        let discovery_atom = builder.intern_reachable(discovery).unwrap();
        assert!(builder.contains_reachable(hammer));
        assert!(builder.contains_reachable(forge));
        assert!(builder.contains_reachable(abundance));
        assert!(builder.contains_reachable(discovery));
        assert_eq!(builder.intern_reachable(hammer).unwrap(), hammer_atom);
        assert_eq!(builder.intern_reachable(forge).unwrap(), forge_atom);
        assert_eq!(builder.intern_reachable(abundance).unwrap(), abundance_atom);
        assert_eq!(builder.intern_reachable(discovery).unwrap(), discovery_atom);

        let catalog = builder.build();
        assert!(catalog.hammer_time_reachable());
        assert!(catalog.forge_command_reachable());
        assert!(catalog.is_reachable(hammer));
        assert!(catalog.is_reachable(forge));
        assert!(catalog.is_reachable(abundance));
        assert!(catalog.is_reachable(discovery));
        assert!(!catalog.is_reachable(plain(CardId::StrikeIronclad)));
        assert!(catalog.has_sovereign_blade());
    }

    #[test]
    fn recursive_card_producers_require_action_replay_for_selecting_leaves() {
        for producer in [
            CardId::HelloWorld,
            CardId::Calamity,
            CardId::BeatDown,
            CardId::Catastrophe,
            CardId::Eidolon,
            CardId::KnifeTrap,
            CardId::Uproar,
        ] {
            let mut builder = CatalogBuilder::new();
            builder.intern_reachable(plain(producer)).unwrap();
            builder.intern_reachable(plain(CardId::Hologram)).unwrap();
            let catalog = builder.build();
            assert!(catalog.requires_action_replay(), "{producer:?}");
        }

        let mut selector_only = CatalogBuilder::new();
        selector_only
            .intern_reachable(plain(CardId::Hologram))
            .unwrap();
        assert!(!selector_only.build().requires_action_replay());

        let mut persistent_without_selector = CatalogBuilder::new();
        persistent_without_selector.mark_persistent_action_replay_required();
        persistent_without_selector
            .intern_reachable(plain(CardId::DemonicShield))
            .unwrap();
        assert!(
            !persistent_without_selector.build().requires_action_replay(),
            "a wire-derived replay marker must not widen a nonselecting card catalog"
        );

        let mut knowledge_demon = CatalogBuilder::new();
        knowledge_demon
            .intern_monster(MonsterKind::KnowledgeDemon)
            .unwrap();
        assert!(
            knowledge_demon.build().requires_action_replay(),
            "the blocking enemy choice independently requires an action receipt"
        );
    }

    #[test]
    fn blocking_stratagem_requires_replay_only_with_a_cardplay_owned_draw() {
        let mut live = CatalogBuilder::new();
        live.mark_live_stratagem_reachable();
        live.intern_reachable(plain(CardId::PommelStrike)).unwrap();
        let live = live.build();
        assert!(live.requires_action_replay());
        assert!(live.cardplay_no_result_draw_can_suspend());

        let mut reachable = CatalogBuilder::new();
        reachable
            .intern_reachable(plain(CardId::Stratagem))
            .unwrap();
        reachable
            .intern_reachable(plain(CardId::PommelStrike))
            .unwrap();
        let reachable = reachable.build();
        assert!(reachable.requires_action_replay());
        assert!(reachable.cardplay_no_result_draw_can_suspend());

        let mut stratagem_without_draw = CatalogBuilder::new();
        stratagem_without_draw.mark_live_stratagem_reachable();
        stratagem_without_draw
            .intern_reachable(plain(CardId::DefendIronclad))
            .unwrap();
        let stratagem_without_draw = stratagem_without_draw.build();
        assert!(!stratagem_without_draw.requires_action_replay());
        assert!(!stratagem_without_draw.cardplay_no_result_draw_can_suspend());

        let mut draw_without_parking_hook = CatalogBuilder::new();
        draw_without_parking_hook
            .intern_reachable(plain(CardId::PommelStrike))
            .unwrap();
        let draw_without_parking_hook = draw_without_parking_hook.build();
        assert!(!draw_without_parking_hook.requires_action_replay());
        assert!(!draw_without_parking_hook.cardplay_no_result_draw_can_suspend());

        let mut hellraiser = CatalogBuilder::new();
        hellraiser.mark_live_hellraiser_reachable();
        hellraiser
            .intern_reachable(plain(CardId::PommelStrike))
            .unwrap();
        hellraiser
            .intern_reachable(plain(CardId::SeekerStrike))
            .unwrap();
        assert!(hellraiser.build().cardplay_no_result_draw_can_suspend());

        let mut synchronous_hellraiser = CatalogBuilder::new();
        synchronous_hellraiser.mark_live_hellraiser_reachable();
        synchronous_hellraiser
            .intern_reachable(plain(CardId::PommelStrike))
            .unwrap();
        synchronous_hellraiser
            .intern_reachable(plain(CardId::StrikeIronclad))
            .unwrap();
        assert!(
            !synchronous_hellraiser
                .build()
                .cardplay_no_result_draw_can_suspend()
        );
    }

    #[test]
    fn deferred_dark_embrace_sources_require_replay_for_blocking_descendants() {
        // Sculpting Strike (#3022) writes Ethereal onto a Hand card.
        for source in [
            CardId::Dazed,
            CardId::CallOfTheVoid,
            CardId::SculptingStrike,
        ] {
            let mut builder = CatalogBuilder::new();
            builder.mark_live_dark_embrace_reachable();
            builder.mark_live_stratagem_reachable();
            builder.intern_reachable(plain(source)).unwrap();
            let catalog = builder.build();
            assert!(catalog.dark_embrace_side_end_can_suspend(), "{source:?}");
            assert!(catalog.requires_action_replay(), "{source:?}");
        }

        let mut effective_hex = CatalogBuilder::new();
        effective_hex.mark_live_dark_embrace_reachable();
        effective_hex.mark_live_stratagem_reachable();
        effective_hex.mark_ordinary_ethereal_reachable();
        effective_hex
            .intern_reachable(plain(CardId::DefendIronclad))
            .unwrap();
        assert!(effective_hex.build().dark_embrace_side_end_can_suspend());

        let mut hellraiser = CatalogBuilder::new();
        hellraiser.mark_live_dark_embrace_reachable();
        hellraiser.mark_live_hellraiser_reachable();
        hellraiser
            .intern_reachable(plain(CardId::CallOfTheVoid))
            .unwrap();
        hellraiser
            .intern_reachable(plain(CardId::SeekerStrike))
            .unwrap();
        assert!(hellraiser.build().requires_action_replay());

        let mut no_dark = CatalogBuilder::new();
        no_dark.mark_live_stratagem_reachable();
        no_dark.intern_reachable(plain(CardId::Dazed)).unwrap();
        assert!(!no_dark.build().requires_action_replay());
    }

    /// #3309: History Course makes a fight replay-capable exactly when a
    /// reachable Attack can suspend under its dupe AutoPlay — a selecting
    /// Attack, or an Attack Draw with a blocking hook reachable.
    #[test]
    fn history_course_requires_replay_for_a_suspending_attack_dupe() {
        let build = |relic: bool, cards: &[CardId], hellraiser: bool| {
            let mut builder = CatalogBuilder::new();
            if relic {
                builder
                    .set_relics(&[crate::ids::RelicId::RelicHistoryCourse])
                    .unwrap();
            }
            if hellraiser {
                builder.mark_live_hellraiser_reachable();
            }
            for card in cards {
                builder.intern_reachable(plain(*card)).unwrap();
            }
            builder.build().requires_action_replay()
        };
        // Headbutt selects from Discard; nothing else here needs a receipt.
        assert!(!build(false, &[CardId::Headbutt], false));
        assert!(build(true, &[CardId::Headbutt], false));
        // A Draw Attack under a blocking hook (Hellraiser with a selecting
        // Strike) already needs receipts without the relic.
        assert!(build(
            false,
            &[CardId::PommelStrike, CardId::SeekerStrike],
            true
        ));
        // Neither a nonselecting Draw Attack without a blocking hook nor a
        // plain Attack can suspend under the dupe.
        assert!(!build(true, &[CardId::PommelStrike], false));
        assert!(!build(true, &[CardId::StrikeIronclad], false));
    }

    #[test]
    fn generated_play_count_writers_require_type_compatible_action_replay() {
        for (writer, selector) in [
            (CardId::Burst, CardId::Hologram),
            (CardId::EchoForm, CardId::Hologram),
            (CardId::OneTwoPunch, CardId::Headbutt),
        ] {
            let mut builder = CatalogBuilder::new();
            builder.intern_reachable(plain(writer)).unwrap();
            builder.intern_reachable(plain(selector)).unwrap();
            assert!(builder.build().requires_action_replay(), "{writer:?}");
        }

        let mut signal_without_a_power_selector = CatalogBuilder::new();
        signal_without_a_power_selector
            .intern_reachable(plain(CardId::SignalBoost))
            .unwrap();
        signal_without_a_power_selector
            .intern_reachable(plain(CardId::Hologram))
            .unwrap();
        assert!(
            !signal_without_a_power_selector
                .build()
                .requires_action_replay(),
            "Signal Boost must not replay a selecting Skill"
        );
        assert!(
            crate::content_tables::CARD_ROWS.iter().all(|row| {
                row.card_type != CardType::Power
                    || !matches!(row.upgrade, 0 | 1)
                    || card_row(row.id, row.upgrade).is_some_and(|row| {
                        let mut builder = CatalogBuilder::new();
                        let atom = builder
                            .intern_reachable(CardIdentity {
                                id: row.id,
                                upgrade: row.upgrade,
                                enchantment: None,
                            })
                            .unwrap();
                        !card_body_can_select(&builder.specs[atom as usize])
                    })
            }),
            "the current generated census has no supported selecting Power body"
        );

        for (power, selector, expected) in [
            (PowerId::Burst, CardId::Hologram, true),
            (PowerId::EchoForm, CardId::Hologram, true),
            (PowerId::OneTwoPunch, CardId::Headbutt, true),
            (PowerId::SignalBoost, CardId::Hologram, false),
        ] {
            let mut builder = CatalogBuilder::new();
            builder.mark_live_replay_modifier(power);
            builder.intern_reachable(plain(selector)).unwrap();
            assert_eq!(
                builder.build().requires_action_replay(),
                expected,
                "{power:?}"
            );
        }
    }

    #[test]
    fn hidden_gem_replay_closure_covers_every_implemented_eligible_card_row() {
        use crate::content_tables::CardType;
        use crate::engine::admission::capability_manifest;
        use crate::hot::{CardInstanceState, HotState};

        let implemented = capability_manifest()
            .steps
            .into_iter()
            .collect::<BTreeSet<_>>();
        let mut eligible_rows = 0;
        let mut selecting_rows = 0;
        let mut native_types_seen = [false; 6];

        for row in variant_free_card_rows().filter(|row| {
            !card_has_native_unplayable_keyword(row.id, row.upgrade)
                && !matches!(row.card_type, CardType::Curse | CardType::Quest)
                && row
                    .steps
                    .iter()
                    .all(|step| implemented.contains(&step.kind))
        }) {
            let identity = CardIdentity {
                id: row.id,
                upgrade: row.upgrade,
                enchantment: None,
            };
            let mut builder = CatalogBuilder::new();
            builder.intern_reachable(plain(CardId::HiddenGem)).unwrap();
            let atom = builder.intern_reachable(identity).unwrap();
            let catalog = builder.build();
            let spec = catalog.spec(atom).unwrap();

            eligible_rows += 1;
            native_types_seen[usize::from(row.card_type as u8 - 1)] = true;
            assert_eq!(spec.card_type, row.card_type, "{identity:?}");

            let mut state = HotState::at_defaults();
            let mut payload = CardInstanceState::default();
            payload.set_base_replay_count(Some(1)).unwrap();
            state.card_states.set(7, payload);
            assert_eq!(
                crate::engine::play::effective_replay_count(&state, spec, 7),
                Ok(1),
                "{identity:?}"
            );

            if card_body_can_select(spec) {
                selecting_rows += 1;
                assert!(
                    catalog.requires_action_replay(),
                    "Hidden Gem can make {identity:?} replay its selecting body"
                );
            }
        }

        assert!(eligible_rows > 0);
        assert!(selecting_rows > 0);
        assert_eq!(
            native_types_seen,
            [true, true, true, true, false, false],
            "Hidden Gem's exhaustive admitted closure is Attack/Skill/Power/Status"
        );
    }

    #[test]
    fn an_enchanted_card_is_a_distinct_identity() {
        let mut builder = CatalogBuilder::new();
        let plain_atom = builder.intern(plain(CardId::StrikeIronclad)).unwrap();
        let enchanted = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 0,
                enchantment: Some(CardEnchantment {
                    id: EnchantmentId::Momentum,
                    amount: 3,
                }),
            })
            .unwrap();
        assert_ne!(plain_atom, enchanted);
        let catalog = builder.build();
        assert_eq!(
            catalog.spec(plain_atom).unwrap().identity.id,
            catalog.spec(enchanted).unwrap().identity.id
        );
    }

    #[test]
    fn a_spec_carries_the_generated_row_predicates() {
        let mut builder = CatalogBuilder::new();
        let atom = builder.intern(plain(CardId::DefendIronclad)).unwrap();
        let catalog = builder.build();
        let spec = catalog.spec(atom).unwrap();
        assert!(spec.is_skill);
        assert!(!spec.is_power);
        assert!(spec.playable);
        let row = card_row(CardId::DefendIronclad, 0).unwrap();
        assert_eq!(spec.cost, row.cost);
        assert_eq!(spec.exhausts, row.exhausts);
    }

    #[test]
    fn unit_c_generators_close_over_every_exact_physical_leaf() {
        for (source, upgrade, expected) in [
            (CardId::BoostAway, 0, vec![plain(CardId::Dazed)]),
            (CardId::CollisionCourse, 0, vec![plain(CardId::Debris)]),
            (CardId::FightThrough, 0, vec![plain(CardId::Wound)]),
            (CardId::Overclock, 0, vec![plain(CardId::Burn)]),
            (CardId::Turbo, 0, vec![plain(CardId::Void)]),
            (CardId::GunkUp, 0, vec![plain(CardId::Slimed)]),
            (CardId::GlimpseBeyond, 0, vec![plain(CardId::Soul)]),
            (CardId::GlimpseBeyond, 1, vec![plain(CardId::Soul)]),
            (CardId::Furnace, 0, vec![plain(CardId::SovereignBlade)]),
            (CardId::Furnace, 1, vec![plain(CardId::SovereignBlade)]),
            (CardId::HammerTime, 0, vec![plain(CardId::SovereignBlade)]),
            (CardId::HammerTime, 1, vec![plain(CardId::SovereignBlade)]),
            (
                CardId::BeatIntoShape,
                0,
                vec![plain(CardId::SovereignBlade)],
            ),
            (
                CardId::BeatIntoShape,
                1,
                vec![plain(CardId::SovereignBlade)],
            ),
            (CardId::Conqueror, 0, vec![plain(CardId::SovereignBlade)]),
            (CardId::Conqueror, 1, vec![plain(CardId::SovereignBlade)]),
            (CardId::SeekingEdge, 0, vec![plain(CardId::SovereignBlade)]),
            (CardId::SeekingEdge, 1, vec![plain(CardId::SovereignBlade)]),
            (CardId::RefineBlade, 0, vec![plain(CardId::SovereignBlade)]),
            (CardId::RefineBlade, 1, vec![plain(CardId::SovereignBlade)]),
            (CardId::TheSmith, 0, vec![plain(CardId::SovereignBlade)]),
            (CardId::TheSmith, 1, vec![plain(CardId::SovereignBlade)]),
            (CardId::WroughtInWar, 0, vec![plain(CardId::SovereignBlade)]),
            (CardId::WroughtInWar, 1, vec![plain(CardId::SovereignBlade)]),
            (CardId::HiddenDaggers, 0, vec![plain(CardId::Shiv)]),
            (
                CardId::HiddenDaggers,
                1,
                vec![
                    plain(CardId::Shiv),
                    CardIdentity {
                        id: CardId::Shiv,
                        upgrade: 1,
                        enchantment: None,
                    },
                ],
            ),
            (
                CardId::BladeOfInk,
                0,
                vec![
                    plain(CardId::Shiv),
                    CardIdentity {
                        id: CardId::Shiv,
                        upgrade: 0,
                        enchantment: Some(CardEnchantment {
                            id: EnchantmentId::Inky,
                            amount: 1,
                        }),
                    },
                    CardIdentity {
                        id: CardId::Shiv,
                        upgrade: 1,
                        enchantment: Some(CardEnchantment {
                            id: EnchantmentId::Inky,
                            amount: 1,
                        }),
                    },
                ],
            ),
            (
                CardId::BladeOfInk,
                1,
                vec![
                    plain(CardId::Shiv),
                    CardIdentity {
                        id: CardId::Shiv,
                        upgrade: 0,
                        enchantment: Some(CardEnchantment {
                            id: EnchantmentId::Inky,
                            amount: 1,
                        }),
                    },
                    CardIdentity {
                        id: CardId::Shiv,
                        upgrade: 1,
                        enchantment: Some(CardEnchantment {
                            id: EnchantmentId::Inky,
                            amount: 1,
                        }),
                    },
                ],
            ),
        ] {
            let mut builder = CatalogBuilder::new();
            builder
                .intern(CardIdentity {
                    id: source,
                    upgrade,
                    enchantment: None,
                })
                .unwrap();
            let catalog = builder.build();
            for identity in expected {
                assert!(
                    catalog.atom(&identity).is_some(),
                    "{}+{} omitted generated {}+{}",
                    source.as_str(),
                    upgrade,
                    identity.id.as_str(),
                    identity.upgrade
                );
            }
        }
    }

    #[test]
    fn glimpse_closes_recursively_over_only_the_exact_soul_row() {
        let soul_row = card_row(CardId::Soul, 0).unwrap();
        assert_eq!(soul_row.cost, 0);
        assert_eq!(soul_row.target_type, "Self");
        assert!(soul_row.is_skill);
        assert!(soul_row.exhausts);
        assert!(matches!(
            soul_row.steps,
            [crate::content_tables::Step {
                kind: StepKind::Draw,
                args: [Arg::I(2)],
            }]
        ));

        for identity in [
            CardIdentity {
                id: CardId::GlimpseBeyond,
                upgrade: 0,
                enchantment: None,
            },
            CardIdentity {
                id: CardId::GlimpseBeyond,
                upgrade: 1,
                enchantment: None,
            },
            CardIdentity {
                id: CardId::GlimpseBeyond,
                upgrade: 1,
                enchantment: Some(CardEnchantment {
                    id: EnchantmentId::Glam,
                    amount: 2,
                }),
            },
        ] {
            let mut builder = CatalogBuilder::new();
            builder.intern(identity).unwrap();
            let catalog = builder.build();
            assert!(catalog.atom(&plain(CardId::Soul)).is_some());
            assert!(
                catalog
                    .atom(&CardIdentity {
                        id: CardId::Soul,
                        upgrade: 1,
                        enchantment: None,
                    })
                    .is_none()
            );
            assert_eq!(
                catalog
                    .specs()
                    .filter(|spec| matches!(spec.identity.id, CardId::Soul))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn furnace_closes_recursively_over_only_the_exact_l0_sovereign_blade() {
        let blade_row = card_row(CardId::SovereignBlade, 0).unwrap();
        assert_eq!(blade_row.cost, 2);
        assert_eq!(blade_row.target_type, "Dynamic");
        assert!(matches!(
            blade_row.steps,
            [crate::content_tables::Step {
                kind: StepKind::SovereignBladeExact,
                args: [],
            }]
        ));

        for identity in [
            CardIdentity {
                id: CardId::Furnace,
                upgrade: 0,
                enchantment: None,
            },
            CardIdentity {
                id: CardId::Furnace,
                upgrade: 1,
                enchantment: None,
            },
            CardIdentity {
                id: CardId::Furnace,
                upgrade: 1,
                enchantment: Some(CardEnchantment {
                    id: EnchantmentId::Glam,
                    amount: 2,
                }),
            },
        ] {
            let mut builder = CatalogBuilder::new();
            builder.intern(identity).unwrap();
            let catalog = builder.build();
            assert!(catalog.atom(&plain(CardId::SovereignBlade)).is_some());
            assert!(
                catalog
                    .atom(&CardIdentity {
                        id: CardId::SovereignBlade,
                        upgrade: 1,
                        enchantment: None,
                    })
                    .is_none()
            );
            assert_eq!(
                catalog
                    .specs()
                    .filter(|spec| matches!(spec.identity.id, CardId::SovereignBlade))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn sovereign_reachability_tracks_the_complete_prospective_catalog_closure() {
        let mut ordinary = CatalogBuilder::new();
        for id in [CardId::StrikeIronclad, CardId::DefendIronclad, CardId::Bash] {
            ordinary.intern(plain(id)).unwrap();
        }
        let ordinary = ordinary.build();
        assert!(!ordinary.has_sovereign_blade());
        assert!(ordinary.atom(&plain(CardId::SovereignBlade)).is_none());

        let mut forge = CatalogBuilder::new();
        forge.intern(plain(CardId::SeekingEdge)).unwrap();
        let forge = forge.build();
        assert!(forge.has_sovereign_blade());
        assert!(forge.atom(&plain(CardId::SovereignBlade)).is_some());
    }

    #[test]
    fn auto_post_card_capability_is_derived_from_every_immutable_identity() {
        let ordinary = CatalogBuilder::new().build();
        assert!(!ordinary.has_auto_post_card_listener());

        for identity in [
            plain(CardId::HowlFromBeyond),
            plain(CardId::IAmInvincible),
            CardIdentity {
                id: CardId::IAmInvincible,
                upgrade: 1,
                enchantment: None,
            },
            CardIdentity {
                id: CardId::HowlFromBeyond,
                upgrade: 1,
                enchantment: Some(CardEnchantment {
                    id: EnchantmentId::Momentum,
                    amount: 2,
                }),
            },
        ] {
            let mut builder = CatalogBuilder::new();
            builder.intern(identity).unwrap();
            assert!(builder.build().has_auto_post_card_listener());
        }
    }

    #[test]
    fn an_absent_row_refuses_rather_than_inventing_a_spec() {
        let mut builder = CatalogBuilder::new();
        let error = builder
            .intern(CardIdentity {
                id: CardId::StrikeIronclad,
                upgrade: 200,
                enchantment: None,
            })
            .unwrap_err();
        assert_eq!(
            error,
            CatalogError::UnknownCardRow(CardId::StrikeIronclad, 200)
        );
    }

    #[test]
    fn forge_rehearsal_class_is_derived_from_the_complete_immutable_identity_set() {
        assert_eq!(std::mem::size_of::<CardSpec>(), 56);
        assert_eq!(std::mem::align_of::<CardSpec>(), 8);
        for row in &crate::content_tables::CARD_ROWS {
            let expected = if matches!(row.id, CardId::Furnace) {
                ForgeRehearsal::Furnace
            } else if matches!(row.id, CardId::SovereignBlade) {
                ForgeRehearsal::Sovereign
            } else if crate::steps::regent_forge::forge_writer_program_is_supported(row)
                && !matches!(row.id, CardId::BeatIntoShape | CardId::WroughtInWar)
            {
                // The two exact attack-before-Forge modes own their existing
                // attack transaction. Every other exact Forge row is a rare
                // writer whose complete CardPlay prefix must be rehearsed.
                ForgeRehearsal::Writer
            } else {
                ForgeRehearsal::None
            };
            let identity = CardIdentity {
                id: row.id,
                upgrade: row.upgrade,
                enchantment: None,
            };
            let spec = CardSpec::from_row(identity, row, Span::default());
            assert_eq!(spec.forge_rehearsal, expected, "{identity:?}");
        }
    }

    #[test]
    fn aggression_interns_the_upgrade_closure_for_every_attack_identity() {
        let mut builder = CatalogBuilder::new();
        let strike = plain(CardId::StrikeIronclad);
        let defend = plain(CardId::DefendIronclad);
        builder.intern(strike).unwrap();
        builder.intern(defend).unwrap();

        builder.intern_aggression_upgrade_closure().unwrap();
        let catalog = builder.build();

        assert!(
            catalog
                .atom(&CardIdentity {
                    upgrade: 1,
                    ..strike
                })
                .is_some()
        );
        assert!(
            catalog
                .atom(&CardIdentity {
                    upgrade: 1,
                    ..defend
                })
                .is_none()
        );
    }

    #[test]
    fn armaments_closure_interns_every_cataloged_next_rung_and_preserves_enchantments() {
        let strike = plain(CardId::StrikeIronclad);
        let defend = plain(CardId::DefendIronclad);
        let glam = CardIdentity {
            id: CardId::Apotheosis,
            upgrade: 0,
            enchantment: Some(CardEnchantment {
                id: EnchantmentId::Glam,
                amount: 2,
            }),
        };
        let mut builder = CatalogBuilder::new();
        builder.intern(defend).unwrap();
        builder
            .intern(CardIdentity {
                upgrade: 1,
                ..strike
            })
            .unwrap();
        builder.intern_reachable(strike).unwrap();
        builder.intern_reachable(glam).unwrap();

        builder.intern_all_card_upgrade_closure().unwrap();
        let catalog = builder.build();

        for identity in [strike, defend, glam] {
            let upgraded = CardIdentity {
                upgrade: 1,
                ..identity
            };
            assert!(catalog.atom(&upgraded).is_some(), "{upgraded:?}");
            assert_eq!(
                catalog.is_reachable(upgraded),
                identity != defend,
                "only reachable source rungs recursively publish upgraded provenance"
            );
        }
        assert_eq!(
            catalog
                .spec(catalog.atom(&CardIdentity { upgrade: 1, ..glam }).unwrap())
                .unwrap()
                .identity
                .enchantment,
            glam.enchantment
        );
    }

    #[test]
    fn knife_trap_upgraded_interns_every_live_shiv_upgrade_in_either_order() {
        let knife = CardIdentity {
            id: CardId::KnifeTrap,
            upgrade: 1,
            enchantment: None,
        };
        let glam_shiv = CardIdentity {
            id: CardId::Shiv,
            upgrade: 0,
            enchantment: Some(CardEnchantment {
                id: EnchantmentId::Glam,
                amount: 1,
            }),
        };
        let glam_shiv_upgraded = CardIdentity {
            upgrade: 1,
            ..glam_shiv
        };
        for identities in [[glam_shiv, knife], [knife, glam_shiv]] {
            let mut builder = CatalogBuilder::new();
            for identity in identities {
                builder.intern(identity).unwrap();
            }
            assert!(builder.build().atom(&glam_shiv_upgraded).is_some());
        }

        let mut without_knife = CatalogBuilder::new();
        without_knife.intern(glam_shiv).unwrap();
        assert!(
            without_knife.build().atom(&glam_shiv_upgraded).is_none(),
            "the upgrade closure is reachable only from Knife Trap+"
        );
    }

    #[test]
    fn hand_cap_generated_leaf_closure_is_derived_from_the_debris_mode() {
        let debris = plain(CardId::Debris);

        let mut crash = CatalogBuilder::new();
        crash.intern(plain(CardId::CrashLanding)).unwrap();
        assert!(crash.build().atom(&debris).is_some());

        let mut anointed = CatalogBuilder::new();
        anointed.intern(plain(CardId::Anointed)).unwrap();
        assert!(anointed.build().atom(&debris).is_none());
    }

    #[test]
    fn fabricator_interns_every_spawned_bot_and_noise_status_identity() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Fabricator).unwrap();
        let catalog = builder.build();

        assert_eq!(catalog.moves(MonsterKind::Fabricator).len(), 3);
        for bot in [
            MonsterKind::Guardbot,
            MonsterKind::Noisebot,
            MonsterKind::Stabbot,
            MonsterKind::Zapbot,
        ] {
            assert_eq!(catalog.moves(bot).len(), 1, "{} closure", bot.as_str());
        }
        assert!(
            catalog
                .atom(&CardIdentity {
                    id: CardId::Dazed,
                    upgrade: 0,
                    enchantment: None,
                })
                .is_some(),
            "Noisebot's implicit Dazed+0 mint must be in the cold catalog"
        );
    }

    #[test]
    fn entomancer_interns_the_implicit_personal_hive_dazed_identity() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Entomancer).unwrap();
        let catalog = builder.build();
        assert_eq!(catalog.moves(MonsterKind::Entomancer).len(), 3);
        assert!(
            catalog
                .atom(&CardIdentity {
                    id: CardId::Dazed,
                    upgrade: 0,
                    enchantment: None,
                })
                .is_some()
        );
    }

    #[test]
    fn soul_fysh_interns_its_implicit_beckon_identity() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::SoulFysh).unwrap();
        let catalog = builder.build();
        assert_eq!(catalog.moves(MonsterKind::SoulFysh).len(), 5);
        let identity = CardIdentity {
            id: CardId::Beckon,
            upgrade: 0,
            enchantment: None,
        };
        let atom = catalog.atom(&identity).expect("implicit Beckon+0 atom");
        assert_eq!(catalog.spec(atom).unwrap().identity, identity);
    }

    #[test]
    fn the_insatiable_interns_its_implicit_frantic_escape_identity() {
        let mut builder = CatalogBuilder::new();
        builder.intern_monster(MonsterKind::TheInsatiable).unwrap();
        let catalog = builder.build();
        assert_eq!(catalog.moves(MonsterKind::TheInsatiable).len(), 5);
        let identity = CardIdentity {
            id: CardId::FranticEscape,
            upgrade: 0,
            enchantment: None,
        };
        let atom = catalog
            .atom(&identity)
            .expect("implicit Frantic Escape+0 atom");
        assert_eq!(catalog.spec(atom).unwrap().identity, identity);
    }

    #[test]
    fn an_unissued_atom_has_no_spec() {
        let catalog = CatalogBuilder::new().build();
        assert_eq!(catalog.spec(0), None);
    }

    #[test]
    fn per_fight_constants_survive_the_freeze() {
        let mut builder = CatalogBuilder::new();
        builder.set_game_build(Some(GameBuild::V0_111_0));
        assert_eq!(builder.build().game_build(), Some(GameBuild::V0_111_0));
    }

    #[test]
    fn card_types_are_the_cold_six_way_native_census() {
        let mut builder = CatalogBuilder::new();
        let strike = builder.intern(plain(CardId::StrikeIronclad)).unwrap();
        let defend = builder.intern(plain(CardId::DefendIronclad)).unwrap();
        let bash = builder.intern(plain(CardId::Bash)).unwrap();
        let catalog = builder.build();
        assert!(catalog.spec(strike).unwrap().is_attack);
        assert!(catalog.spec(bash).unwrap().is_attack);
        assert!(!catalog.spec(defend).unwrap().is_attack);
        assert!(catalog.spec(defend).unwrap().is_skill);
        assert_eq!(
            card_row(CardId::Sloth, 0).unwrap().card_type,
            CardType::Status
        );
        assert!(!card_row(CardId::Sloth, 0).unwrap().playable);
        // The generated enum is total and agrees with legacy convenience
        // predicates without deriving Attack from CanPlay.
        for row in crate::content_tables::CARD_ROWS.iter() {
            assert_eq!(
                row.is_power,
                row.card_type == CardType::Power,
                "{}",
                row.name
            );
            assert_eq!(
                row.is_skill,
                row.card_type == CardType::Skill,
                "{}",
                row.name
            );
            assert_eq!(
                row.is_status,
                row.card_type == CardType::Status,
                "{}",
                row.name
            );
        }
    }

    #[test]
    fn exact_status_projection_does_not_collapse_curses() {
        let wound = card_row(CardId::Wound, 0).unwrap();
        let bane = card_row(CardId::AscendersBane, 0).unwrap();
        assert!(wound.is_status);
        assert!(!bane.is_status);
        // This pre-existing convenience predicate is deliberately not the
        // native type axis: unplayable Status/Curse rows may leave it false.
        assert_eq!(wound.is_status_curse, bane.is_status_curse);
    }

    #[test]
    fn the_wire_vocabularies_are_ascending_and_round_trip() {
        for names in [
            GameBuild::NAMES.as_slice(),
            RewardPool::NAMES.as_slice(),
            RewardOdds::NAMES.as_slice(),
        ] {
            assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
        }
        for pool in RewardPool::ALL {
            assert_eq!(RewardPool::from_str(pool.as_str()), Some(pool));
        }
        for odds in RewardOdds::ALL {
            assert_eq!(RewardOdds::from_str(odds.as_str()), Some(odds));
        }
        for build in GameBuild::ALL {
            assert_eq!(GameBuild::from_str(build.as_str()), Some(build));
        }
        assert_eq!(RewardPool::from_str("Watcher"), None);
        assert_eq!(GameBuild::from_str("v0.110.1"), None);
    }

    #[test]
    fn generated_card_target_types_are_exactly_the_typed_vocabulary() {
        let generated: std::collections::BTreeSet<_> = crate::content_tables::CARD_ROWS
            .iter()
            .map(|row| row.target_type)
            .collect();
        let typed: std::collections::BTreeSet<_> = CardTargetType::ALL
            .into_iter()
            .map(|target| target.as_str())
            .collect();

        assert_eq!(generated, typed);
        assert_eq!(generated.len(), 8);
    }
}

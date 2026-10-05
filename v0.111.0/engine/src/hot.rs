//! The search-owned hot state (PORT_PLAN.md D3).
//!
//! # What this type is, and is not
//!
//! It is **not** a Rust transcription of Python's 517 `State` fields. It is
//! what the engine core executes: core scalars, the five card piles, the
//! monster roster, active powers, the continuation stack, the play-history
//! counters, and the xoshiro streams. Python's field-per-power shape is the *canonical* shape
//! (`canonical.rs`), and the canonical document is the only place it exists.
//!
//! Totality is achieved by refusal, not by width: `boundary.rs` walks every
//! key of a canonical document and either maps it here or refuses by name.
//! Coverage grows in reviewable diffs as R0.5+ claims fields; a field that is
//! silently dropped is the bug class this whole layer exists to prevent.
//!
//! # Layout rules, all mechanically pinned below
//!
//! * core scalars inline;
//! * piles are `Arc<Vec<HotCard>>` copy-on-write, [`HotCard`] exactly 8
//!   bytes and `Copy`;
//! * per-instance mutable card data lives in a copy-on-write side table
//!   keyed by uid, so the overwhelmingly common "nothing is modified" case
//!   costs one pointer;
//! * powers are sorted slot vectors, not fields ([`crate::powers`]);
//! * monsters are one copy-on-write vector with a dynamic count;
//! * the continuation stack is one shared [`Frame`] enum in a copy-on-write
//!   vector, replacing the kernel's twice-implemented per-encounter
//!   continuation enums;
//! * no owned text, no ordered/hashed maps, no per-node boxed chains
//!   anywhere in this file — `boundary.rs` has a contract test that reads
//!   this source and enforces it;
//! * inline sizes are `const`-asserted at the bottom. Exceeding a budget is
//!   a compile error on the pull request that does it.
//!
//! # Clone cost
//!
//! One `HotState::clone` with no pending selection is 12 atomic increments
//! (5 piles, monsters, powers, the card side table, the frame stack, the rng
//! block, the orb queue, and the card-event fan-out block) plus a memcpy of the
//! inline bytes. A live `Some(Arc<PendingSelection>)` adds one increment, for
//! 13. Nothing walks a pile, a roster, a power set, an orb queue, a listener
//! order, or a stream.

use std::ops::Deref;
use std::sync::{Arc, LazyLock};

use crate::catalog::{CardAtom, CardIdentity, RNG_STREAM_COUNT, RewardOdds, RewardPool};
use crate::decimal::DotNetDecimal;
use crate::frame::Frame;
use crate::ids::{CardId, MonsterKind, PotionId, PowerId};
use crate::pet::{PetStateError, SoloPetState};
use crate::powers::Slots;

/// Maximum members in the represented ordinary `AfterCardDrawn` walk.
const MAX_AFTER_CARD_DRAWN_POWERS: usize = 7;
/// Maximum members in the represented `AfterCardExhausted` walk.
const MAX_AFTER_CARD_EXHAUSTED_POWERS: usize = 2;
/// Maximum members in the represented `BeforeHandDraw` walk.
const MAX_BEFORE_HAND_DRAW_POWERS: usize = 4;
const MAX_AFTER_DAMAGE_GIVEN_POWERS: usize = 5;
const MAX_AFTER_BLOCK_GAINED_POWERS: usize = 2;
const MAX_AFTER_BLOCK_CLEARED_POWERS: usize = 2;
const MAX_AFTER_POWER_AMOUNT_CHANGED_POWERS: usize = 4;
/// Complete current-build concrete player-power membership of the ordinary
/// `AfterSideTurnStart` acquisition walk. Sandpit belongs to
/// `AfterSideTurnStartLate` and is deliberately not substituted here.
const MAX_AFTER_SIDE_TURN_START_POWERS: usize = 19;
const MAX_STAR_ENERGY_RESET_POWERS: usize = 2;
const AFTER_ENERGY_RESET_POWERS: [AfterEnergyResetPower; 6] = [
    AfterEnergyResetPower::Genesis,
    AfterEnergyResetPower::StarNextTurn,
    AfterEnergyResetPower::EnergyNextTurn,
    AfterEnergyResetPower::Radiance,
    AfterEnergyResetPower::LightningRod,
    AfterEnergyResetPower::Spinner,
];

/// The closed AfterEnergyReset listener vocabulary. Radiance is the one
/// potion-owned listener whose amount lives in the potion cold record rather
/// than `Slots<PowerId>`; retaining it here makes the six-item native list
/// explicit without inventing a sparse PowerId variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AfterEnergyResetPower {
    Genesis,
    StarNextTurn,
    EnergyNextTurn,
    Radiance,
    LightningRod,
    Spinner,
}

impl AfterEnergyResetPower {
    pub(crate) fn from_power(power: PowerId) -> Option<Self> {
        Some(match power {
            PowerId::Genesis => Self::Genesis,
            PowerId::StarNextTurn => Self::StarNextTurn,
            PowerId::EnergyNextTurn => Self::EnergyNextTurn,
            PowerId::LightningRod => Self::LightningRod,
            PowerId::Spinner => Self::Spinner,
            _ => return None,
        })
    }
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Genesis => "genesis",
            Self::StarNextTurn => "star_next_turn",
            Self::EnergyNextTurn => "energy_next_turn",
            Self::Radiance => "radiance",
            Self::LightningRod => "lightning_rod",
            Self::Spinner => "spinner",
        }
    }
}

/// Inline cold storage avoids widening every fanout COW allocation for a
/// bounded six-member native list.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct AfterEnergyResetOrder {
    powers: [AfterEnergyResetPower; AFTER_ENERGY_RESET_POWERS.len()],
    len: u8,
    explicit: bool,
}

impl AfterEnergyResetOrder {
    fn from_slice(order: &[AfterEnergyResetPower]) -> Self {
        let mut powers = [AfterEnergyResetPower::Genesis; AFTER_ENERGY_RESET_POWERS.len()];
        powers[..order.len()].copy_from_slice(order);
        Self {
            powers,
            len: order.len() as u8,
            explicit: true,
        }
    }

    fn as_slice(&self) -> &[AfterEnergyResetPower] {
        &self.powers[..usize::from(self.len)]
    }

    fn register(&mut self, power: AfterEnergyResetPower) -> bool {
        let is_new = !self.as_slice().contains(&power);
        if is_new {
            self.powers[usize::from(self.len)] = power;
            self.len += 1;
        }
        is_new
    }

    fn unregister(&mut self, power: AfterEnergyResetPower) {
        if let Some(index) = self
            .as_slice()
            .iter()
            .position(|candidate| *candidate == power)
        {
            let len = usize::from(self.len);
            self.powers.copy_within(index + 1..len, index);
            self.len -= 1;
            self.powers[len - 1] = AfterEnergyResetPower::Genesis;
        }
    }

    fn mark_legacy_inferred(&mut self) {
        self.explicit = false;
    }
}
const MAX_LOCAL_GENERATED_POWERS: usize = 4;
const MAX_RESULT_LOCATION_POWERS: usize = 4;
const WITHERING_CARDS_LEFT_MAX: u16 = 6;

// Compact private power-object state. Bits zero through two are Monologue's
// three listener registrations; bits three through five are the Ruined
// Helmet, Pale Blue Dot, and Unsettling Lamp latches. Bit six records the
// exact all-or-none Void Form hook bundle; bit seven is its deferred end-turn
// request.
const MONOLOGUE_HOOK_MASK: u8 = 0b0000_0111;
const RUINED_HELMET_USED_MASK: u8 = 0b0000_1000;
const PALE_BLUE_DOT_USED_MASK: u8 = 0b0001_0000;
const UNSETTLING_LAMP_AVAILABLE_MASK: u8 = 0b0010_0000;
const VOID_FORM_HOOKS_MASK: u8 = 0b0100_0000;
const VOID_FORM_END_TURN_REQUESTED_MASK: u8 = 0b1000_0000;

const AUTOMATION_LEFT_MAX: u8 = 15;
const PANACHE_LEFT_MAX: u8 = 7;
const AUTOMATION_LEFT_MASK: u8 = 0b0000_1111;
const PANACHE_LEFT_SHIFT: u32 = 4;
const PANACHE_LEFT_MASK: u8 = 0b0111_0000;
const TENDER_ACTIVE_MASK: u8 = 0b1000_0000;

const RINGING_LIVE_MASK: u8 = 0b0000_0001;
const BOUND_AFFLICTIONS_SHIFT: u32 = 1;
const BOUND_AFFLICTIONS_MASK: u8 = 0b0000_0110;
const BOUND_CARD_PLAYED_MASK: u8 = 0b0000_1000;
const HAND_DRILL_OWNED_MASK: u8 = 0b0001_0000;
const SNECKO_SKULL_OWNED_MASK: u8 = 0b0010_0000;
const THE_BOOT_OWNED_MASK: u8 = 0b0100_0000;
const TUNGSTEN_ROD_OWNED_MASK: u8 = 0b1000_0000;

// Compact Batch-5 hand-relic state. Persistent counters use zero as the
// absent sentinel and encode each live Python value as value + 1. The one
// full-width observation is Pocketwatch's prior-turn play count; Rust's
// already-admitted per-turn history is i16, so the exact nonnegative domain
// is 0..=i16::MAX. Bit 31 caches immutable Paper Krane ownership for the deep
// monster-attack path, which otherwise has no catalog parameter.
const FLOWER_SHIFT: u32 = 0;
const FLOWER_MASK: u32 = 0b11 << FLOWER_SHIFT;
const FAKE_FLOWER_SHIFT: u32 = 2;
const FAKE_FLOWER_MASK: u32 = 0b111 << FAKE_FLOWER_SHIFT;
const PENDULUM_SHIFT: u32 = 5;
const PENDULUM_MASK: u32 = 0b11 << PENDULUM_SHIFT;
const POLLINOUS_CORE_SHIFT: u32 = 7;
const POLLINOUS_CORE_MASK: u32 = 0b111 << POLLINOUS_CORE_SHIFT;
const EMBER_TEA_SHIFT: u32 = 10;
const EMBER_TEA_MASK: u32 = 0b111 << EMBER_TEA_SHIFT;
const ART_OF_WAR_CURRENT_MASK: u32 = 1 << 13;
const ART_OF_WAR_LAST_MASK: u32 = 1 << 14;
const PAELS_TEARS_LEFTOVER_MASK: u32 = 1 << 15;
const POCKETWATCH_LAST_SHIFT: u32 = 16;
const POCKETWATCH_LAST_MASK: u32 = 0x7fff << POCKETWATCH_LAST_SHIFT;
const PAPER_KRANE_OWNED_MASK: u32 = 1 << 31;
// Batch-6 relic latches. These are all independent booleans in Python's
// canonical State projection; immutable ownership remains in the Catalog.
const BOOMING_CONCH_ELITE_MASK: u32 = 1 << 0;
const DEMON_TONGUE_TRIGGERED_MASK: u32 = 1 << 1;
const EMOTION_DAMAGE_CURRENT_MASK: u32 = 1 << 2;
const EMOTION_DAMAGE_PREVIOUS_MASK: u32 = 1 << 3;
const FAKE_TEA_SET_CHARGED_MASK: u32 = 1 << 4;
const FUR_COAT_ACTIVE_MASK: u32 = 1 << 5;
const PERMAFROST_ARMED_MASK: u32 = 1 << 6;
const TEA_SET_CHARGED_MASK: u32 = 1 << 7;
const DEMON_TONGUE_OWNED_MASK: u32 = 1 << 8;
const EMOTION_CHIP_OWNED_MASK: u32 = 1 << 9;
const BOOK_REPAIR_KNIFE_OWNED_MASK: u32 = 1 << 10;
const VELVET_CHOKER_OWNED_MASK: u32 = 1 << 11;
const SPECTRUM_SHIFT_POOL_MASK: u32 = 1 << 12;
const CREATIVE_AI_POOL_MASK: u32 = 1 << 13;
const NUNCHAKU_SHIFT: u32 = 14;
const NUNCHAKU_MASK: u32 = 0b1111 << NUNCHAKU_SHIFT;
const KUNAI_SHIFT: u32 = 18;
const KUNAI_MASK: u32 = 0b11 << KUNAI_SHIFT;
const SHURIKEN_SHIFT: u32 = 20;
const SHURIKEN_MASK: u32 = 0b11 << SHURIKEN_SHIFT;
const ORNAMENTAL_FAN_SHIFT: u32 = 22;
const ORNAMENTAL_FAN_MASK: u32 = 0b11 << ORNAMENTAL_FAN_SHIFT;
const RAINBOW_RING_SHIFT: u32 = 24;
const RAINBOW_RING_MASK: u32 = 0b111 << RAINBOW_RING_SHIFT;
const METRONOME_SHIFT: u32 = 27;
const METRONOME_MASK: u32 = 0b111 << METRONOME_SHIFT;
const TEA_OF_DISCOURTESY_ACTIVE_MASK: u32 = 1 << 30;
const PHILOSOPHERS_STONE_OWNED_MASK: u32 = 1 << 31;
const RESULT_LOCATION_LEN_LOW_MASK: u8 = 0b0000_0011;
const STAR_ENERGY_RESET_LEN_SHIFT: u32 = 2;
const STAR_ENERGY_RESET_LEN_MASK: u8 = 0b0000_1100;
const LOCAL_GENERATED_LEN_SHIFT: u32 = 4;
const LOCAL_GENERATED_LEN_MASK: u8 = 0b0111_0000;
const RESULT_LOCATION_LEN_HIGH_MASK: u8 = 0b1000_0000;

const fn result_location_len(packed: u8) -> u8 {
    (packed & RESULT_LOCATION_LEN_LOW_MASK) | ((packed & RESULT_LOCATION_LEN_HIGH_MASK) >> 5)
}

const fn with_result_location_len(packed: u8, len: u8) -> u8 {
    (packed & !(RESULT_LOCATION_LEN_LOW_MASK | RESULT_LOCATION_LEN_HIGH_MASK))
        | (len & RESULT_LOCATION_LEN_LOW_MASK)
        | ((len & 0b100) << 5)
}
// Inline generated-power state. This byte replaces the former standalone
// Hello World provenance bool without widening HotState: bit zero publishes
// the exact Common pool, bit one preserves the closed current-minus-snapshot
// delta, and bit two mirrors the live persistent Calamity listener.
const HELLO_WORLD_GENERATION_POOL_MASK: u8 = 0b0000_0001;
const HELLO_WORLD_SNAPSHOT_DIRTY_MASK: u8 = 0b0000_0010;
const CALAMITY_HOOK_LIVE_MASK: u8 = 0b0000_0100;
const CALL_OF_THE_VOID_GENERATION_POOL_MASK: u8 = 0b0000_1000;
const ENTROPY_TURN_START_ORDINAL_SHIFT: u32 = 4;
const ENTROPY_TURN_START_ORDINAL_MASK: u8 = 0b0111_0000;
const GENERATED_POWER_FLAGS_MASK: u8 = HELLO_WORLD_GENERATION_POOL_MASK
    | HELLO_WORLD_SNAPSHOT_DIRTY_MASK
    | CALAMITY_HOOK_LIVE_MASK
    | CALL_OF_THE_VOID_GENERATION_POOL_MASK
    | ENTROPY_TURN_START_ORDINAL_MASK;
const INTERCEPT_COVERED_MASK: u8 = 0b0000_0011;
const INTERCEPT_COVERED_ORDER_SHIFT: u32 = 2;
const INTERCEPT_COVERED_ORDER_MASK: u8 = 0b0001_1100;
const IMITATION_LEARNING_ORDER_SHIFT: u32 = 5;
const IMITATION_LEARNING_ORDER_MASK: u8 = 0b1110_0000;
const MAX_AFTER_PLAYER_TURN_START_POWERS: usize = 8;
const MAX_STORED_AFTER_PLAYER_TURN_START_POWERS: usize = 7;
const MAX_AFTER_DAMAGE_RECEIVED_POWERS: usize = 3;
const AFTER_SIDE_TURN_START_LEN_MASK: u8 = 0b0001_1111;
const AFTER_PLAYER_TURN_START_LEN_SHIFT: u32 = 5;
const AFTER_PLAYER_TURN_START_LEN_MASK: u8 = 0b1110_0000;

/// Compact closed vocabulary for the four admitted result-location readers.
///
/// `PowerId` is a generated `u16`; storing these four known members as bytes
/// lets the widened order occupy the same four bytes as the former two-member
/// `[PowerId; 2]`, preserving the reviewed 256-byte fanout allocation.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ResultLocationPower {
    Corruption,
    Feral,
    Nostalgia,
    Rebound,
}

/// Compact closed vocabulary for the four admitted
/// `AfterPowerAmountChanged` listeners.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PowerAmountChangedPower {
    Vicious,
    Shroud,
    SleightOfFlesh,
    SwordSage,
}

impl PowerAmountChangedPower {
    fn from_power(power: PowerId) -> Option<Self> {
        match power {
            PowerId::Vicious => Some(Self::Vicious),
            PowerId::Shroud => Some(Self::Shroud),
            PowerId::SleightOfFlesh => Some(Self::SleightOfFlesh),
            PowerId::SwordSage => Some(Self::SwordSage),
            _ => None,
        }
    }

    const fn power(self) -> PowerId {
        match self {
            Self::Vicious => PowerId::Vicious,
            Self::Shroud => PowerId::Shroud,
            Self::SleightOfFlesh => PowerId::SleightOfFlesh,
            Self::SwordSage => PowerId::SwordSage,
        }
    }
}

/// Stack-only decoded view of the compact four-member listener order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PowerAmountChangedOrder {
    powers: [PowerId; MAX_AFTER_POWER_AMOUNT_CHANGED_POWERS],
    len: u8,
}

impl Deref for PowerAmountChangedOrder {
    type Target = [PowerId];

    fn deref(&self) -> &Self::Target {
        &self.powers[..usize::from(self.len)]
    }
}

impl<const N: usize> PartialEq<[PowerId; N]> for PowerAmountChangedOrder {
    fn eq(&self, other: &[PowerId; N]) -> bool {
        self.deref() == other
    }
}

impl<const N: usize> PartialEq<&[PowerId; N]> for PowerAmountChangedOrder {
    fn eq(&self, other: &&[PowerId; N]) -> bool {
        self.deref() == *other
    }
}

/// Stack-only decoded view of the complete compact eight-member listener order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AfterPlayerTurnStartOrder {
    powers: [PowerId; MAX_AFTER_PLAYER_TURN_START_POWERS],
    len: u8,
}

/// Compact closed vocabulary for the complete current-build ordinary
/// `AfterSideTurnStart` acquisition walk. Clarity has no generated `PowerId`,
/// so the carrier is a dedicated byte token rather than a partial PowerId
/// projection plus an out-of-band ordinal.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum AfterSideTurnStartToken {
    BiasedCognition,
    Blur,
    Clarity,
    Coolant,
    Countdown,
    DemonForm,
    DrawNextTurn,
    Feral,
    Furnace,
    Neurosurge,
    NoxiousFumes,
    Plating,
    Poison,
    PrepTime,
    Rampart,
    Reflect,
    ShadowStep,
    Slow,
    WraithForm,
}

impl AfterSideTurnStartToken {
    pub(crate) const ALL: [Self; MAX_AFTER_SIDE_TURN_START_POWERS] = [
        Self::BiasedCognition,
        Self::Blur,
        Self::Clarity,
        Self::Coolant,
        Self::Countdown,
        Self::DemonForm,
        Self::DrawNextTurn,
        Self::Feral,
        Self::Furnace,
        Self::Neurosurge,
        Self::NoxiousFumes,
        Self::Plating,
        Self::Poison,
        Self::PrepTime,
        Self::Rampart,
        Self::Reflect,
        Self::ShadowStep,
        Self::Slow,
        Self::WraithForm,
    ];

    pub(crate) const fn from_power(power: PowerId) -> Option<Self> {
        Some(match power {
            PowerId::BiasedCognition => Self::BiasedCognition,
            PowerId::Blur => Self::Blur,
            PowerId::Coolant => Self::Coolant,
            PowerId::Countdown => Self::Countdown,
            PowerId::DemonForm => Self::DemonForm,
            PowerId::DrawNextTurn => Self::DrawNextTurn,
            PowerId::Feral => Self::Feral,
            PowerId::Furnace => Self::Furnace,
            PowerId::Neurosurge => Self::Neurosurge,
            PowerId::NoxiousFumes => Self::NoxiousFumes,
            PowerId::Plating => Self::Plating,
            PowerId::Poison => Self::Poison,
            PowerId::PrepTime => Self::PrepTime,
            PowerId::Rampart => Self::Rampart,
            PowerId::Reflect => Self::Reflect,
            PowerId::ShadowStep => Self::ShadowStep,
            PowerId::Slow => Self::Slow,
            PowerId::WraithForm => Self::WraithForm,
            _ => return None,
        })
    }

    pub(crate) const fn power(self) -> Option<PowerId> {
        Some(match self {
            Self::BiasedCognition => PowerId::BiasedCognition,
            Self::Blur => PowerId::Blur,
            Self::Clarity => return None,
            Self::Coolant => PowerId::Coolant,
            Self::Countdown => PowerId::Countdown,
            Self::DemonForm => PowerId::DemonForm,
            Self::DrawNextTurn => PowerId::DrawNextTurn,
            Self::Feral => PowerId::Feral,
            Self::Furnace => PowerId::Furnace,
            Self::Neurosurge => PowerId::Neurosurge,
            Self::NoxiousFumes => PowerId::NoxiousFumes,
            Self::Plating => PowerId::Plating,
            Self::Poison => PowerId::Poison,
            Self::PrepTime => PowerId::PrepTime,
            Self::Rampart => PowerId::Rampart,
            Self::Reflect => PowerId::Reflect,
            Self::ShadowStep => PowerId::ShadowStep,
            Self::Slow => PowerId::Slow,
            Self::WraithForm => PowerId::WraithForm,
        })
    }

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::BiasedCognition => "biased_cognition",
            Self::Blur => "blur",
            Self::Clarity => "clarity",
            Self::Coolant => "coolant",
            Self::Countdown => "countdown",
            Self::DemonForm => "demon_form",
            Self::DrawNextTurn => "draw_next_turn",
            Self::Feral => "feral",
            Self::Furnace => "furnace",
            Self::Neurosurge => "neurosurge",
            Self::NoxiousFumes => "noxious_fumes",
            Self::Plating => "plating",
            Self::Poison => "poison",
            Self::PrepTime => "prep_time",
            Self::Rampart => "rampart",
            Self::Reflect => "reflect",
            Self::ShadowStep => "shadow_step",
            Self::Slow => "slow",
            Self::WraithForm => "wraith_form",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|token| token.as_str() == value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AfterSideTurnStartOrder {
    powers: [AfterSideTurnStartToken; MAX_AFTER_SIDE_TURN_START_POWERS],
    len: u8,
}

impl Deref for AfterSideTurnStartOrder {
    type Target = [AfterSideTurnStartToken];

    fn deref(&self) -> &Self::Target {
        &self.powers[..usize::from(self.len)]
    }
}

/// Closed player-power subset whose `AfterDamageReceived` callbacks can
/// co-fire on one powered enemy hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AfterDamageReceivedPower {
    FlameBarrier,
    Reflect,
    TheGambit,
}

impl AfterDamageReceivedPower {
    pub(crate) const ALL: [Self; MAX_AFTER_DAMAGE_RECEIVED_POWERS] =
        [Self::FlameBarrier, Self::Reflect, Self::TheGambit];

    pub(crate) const fn power(self) -> PowerId {
        match self {
            Self::FlameBarrier => PowerId::FlameBarrier,
            Self::Reflect => PowerId::Reflect,
            Self::TheGambit => PowerId::TheGambit,
        }
    }

    pub(crate) const fn from_power(power: PowerId) -> Option<Self> {
        match power {
            PowerId::FlameBarrier => Some(Self::FlameBarrier),
            PowerId::Reflect => Some(Self::Reflect),
            PowerId::TheGambit => Some(Self::TheGambit),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AfterDamageReceivedPowerOrder {
    powers: [AfterDamageReceivedPower; MAX_AFTER_DAMAGE_RECEIVED_POWERS],
    len: u8,
}

impl Deref for AfterDamageReceivedPowerOrder {
    type Target = [AfterDamageReceivedPower];

    fn deref(&self) -> &Self::Target {
        &self.powers[..usize::from(self.len)]
    }
}

const FACTORIAL_THROUGH_THREE: [usize; 4] = [1, 1, 2, 6];

fn encode_after_damage_received_order(order: &[AfterDamageReceivedPower]) -> Option<u8> {
    if order.len() > MAX_AFTER_DAMAGE_RECEIVED_POWERS
        || order
            .iter()
            .enumerate()
            .any(|(index, token)| order[..index].contains(token))
    {
        return None;
    }
    if order.len() <= 1 {
        return Some(0);
    }
    let mut available = [AfterDamageReceivedPower::FlameBarrier; MAX_AFTER_DAMAGE_RECEIVED_POWERS];
    let mut available_len = 0;
    for token in AfterDamageReceivedPower::ALL {
        if order.contains(&token) {
            available[available_len] = token;
            available_len += 1;
        }
    }
    let mut rank = 0_usize;
    for token in order.iter().copied() {
        let index = available[..available_len]
            .iter()
            .position(|candidate| *candidate == token)?;
        rank += index * FACTORIAL_THROUGH_THREE[available_len - 1];
        available.copy_within(index + 1..available_len, index);
        available_len -= 1;
    }
    u8::try_from(rank + 1).ok()
}

fn decode_after_damage_received_order(
    live: &[AfterDamageReceivedPower],
    code: u8,
) -> Option<AfterDamageReceivedPowerOrder> {
    if live.len() > MAX_AFTER_DAMAGE_RECEIVED_POWERS
        || live
            .iter()
            .enumerate()
            .any(|(index, token)| live[..index].contains(token))
        || live.len() <= 1 && code != 0
        || live.len() >= 2 && code == 0
    {
        return None;
    }
    let mut powers = [AfterDamageReceivedPower::FlameBarrier; MAX_AFTER_DAMAGE_RECEIVED_POWERS];
    if live.len() <= 1 {
        powers[..live.len()].copy_from_slice(live);
        return Some(AfterDamageReceivedPowerOrder {
            powers,
            len: live.len() as u8,
        });
    }
    let mut rank = usize::from(code - 1);
    if rank >= FACTORIAL_THROUGH_THREE[live.len()] {
        return None;
    }
    let mut available = [AfterDamageReceivedPower::FlameBarrier; MAX_AFTER_DAMAGE_RECEIVED_POWERS];
    available[..live.len()].copy_from_slice(live);
    let mut available_len = live.len();
    for destination in powers.iter_mut().take(live.len()) {
        let stride = FACTORIAL_THROUGH_THREE[available_len - 1];
        let index = rank / stride;
        rank %= stride;
        *destination = available[index];
        available.copy_within(index + 1..available_len, index);
        available_len -= 1;
    }
    Some(AfterDamageReceivedPowerOrder {
        powers,
        len: live.len() as u8,
    })
}

impl Deref for AfterPlayerTurnStartOrder {
    type Target = [PowerId];

    fn deref(&self) -> &Self::Target {
        &self.powers[..usize::from(self.len)]
    }
}

impl ResultLocationPower {
    fn from_power(power: PowerId) -> Option<Self> {
        match power {
            PowerId::Corruption => Some(Self::Corruption),
            PowerId::Feral => Some(Self::Feral),
            PowerId::Nostalgia => Some(Self::Nostalgia),
            PowerId::Rebound => Some(Self::Rebound),
            _ => None,
        }
    }

    const fn power(self) -> PowerId {
        match self {
            Self::Corruption => PowerId::Corruption,
            Self::Feral => PowerId::Feral,
            Self::Nostalgia => PowerId::Nostalgia,
            Self::Rebound => PowerId::Rebound,
        }
    }
}

/// One independently-instanced native `TheBombPower`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TheBombInstance {
    pub(crate) uid: u32,
    pub(crate) turns: i32,
    pub(crate) damage: i32,
}

/// The one keyed native `ToricToughnessPower`, including its lossless
/// retained `DynamicVars.Block` Decimal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ToricToughnessInstance {
    pub(crate) duration: i32,
    pub(crate) block: DotNetDecimal,
}

/// SlothPower's private number of owner CardPlay iterations started in the
/// current player turn. The public Amount remains in the ordinary power slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SlothInstance {
    pub(crate) cards_played_this_turn: i32,
}

/// One independently-instanced native `AutomationPower` acquired after the
/// first live one.
///
/// v0.111.0 (DLL `9cb4f1ad`): `AutomationPower::get_InstanceType` RVA
/// `0x9fa0e` IL_0001 returns 1 (`PowerInstanceType.Instanced`), and
/// `PowerCmd::FindExistingInstanceForStacking` RVA `0x1338d8` IL_0021-0036
/// answers `null` for that case, so every Automation play attaches a new
/// power object. `InitInternalData` RVA `0x9fa29` gives each object its own
/// `Data` whose `.ctor` (RVA `0x33558e` IL_0002-0004) starts `cardsLeft` at
/// 10. The FIRST live instance keeps its countdown in the packed
/// `automation_left` nibble and its amount is the scalar Automation power
/// minus these rows' amounts, so a one-instance state is byte-identical to
/// the pre-#3021 representation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct AutomationInstance {
    pub(crate) amount: i32,
    pub(crate) cards_left: i32,
}

/// One independently-instanced native `PanachePower`.
///
/// `already_applied` is the power object's private `Data.alreadyApplied`
/// bit.  It cannot be derived from liveness while the applying card's
/// `AfterCardPlayed` listener snapshot is parked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PanacheInstance {
    pub(crate) uid: u32,
    pub(crate) amount: i32,
    pub(crate) cards_left: i32,
    pub(crate) already_applied: bool,
}

/// One independently-instanced native `MonologuePower`.
///
/// `amount` is the fresh `PowerCmd.Apply` amount, while `power` is the
/// instance-local Strength dynamic variable written from the card's `Power`
/// variable after Apply returns.  `strength_applied` is this instance's
/// independent turn ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MonologueInstance {
    pub(crate) uid: u32,
    pub(crate) amount: i32,
    pub(crate) power: i32,
    pub(crate) strength_applied: i32,
}

/// Cold concrete power records sharing the vector header first paid for by
/// The Bomb. Bomb, Panache, and Monologue are truly instanced; Toric and Sloth
/// are keyed singletons. Keeping the tag explicit permits all five families
/// to coexist without encoding a false mutual exclusion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ColdPowerRecord {
    TheBomb(TheBombInstance),
    ToricToughness(ToricToughnessInstance),
    Sloth(SlothInstance),
    Panache(PanacheInstance),
    Monologue(MonologueInstance),
    /// A native AutomationPower object after the first live one (#3021).
    /// These rows keep their relative acquisition order and are otherwise
    /// independent of the other families' placement rules.
    Automation(AutomationInstance),
}

/// Typed acquisition token for the ordinary player-power turn-end walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BeforeSideTurnEndToken {
    ChainsOfBinding,
    Hailstorm,
    TheBomb(u32),
}

/// Stable native type token for one local-player power object registered in
/// the ordinary `AfterSideTurnEnd` hook.  The spelling is part of the
/// canonical Python/Rust boundary and deliberately does not reuse `PowerId`:
/// four admitted objects live outside that scalar vocabulary and Panache and
/// Monologue are native InstanceType-1 families.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum AfterSideTurnEndPowerToken {
    BorrowedTime = 0,
    Burst = 1,
    Constrict = 2,
    ConsumingShadow = 3,
    CorrosiveWave = 4,
    DarkEmbrace = 5,
    Doom = 6,
    DoubleDamage = 7,
    Duplication = 8,
    Hellraiser = 9,
    Juggling = 10,
    Monologue = 11,
    NoDraw = 12,
    NoEnergyGain = 13,
    OneTwoPunch = 14,
    PaleBlueDot = 15,
    Panache = 16,
    Rage = 17,
    Rebound = 18,
    RetainHand = 19,
    Ringing = 20,
    Ritual = 21,
    Shadowmeld = 22,
    Smoggy = 23,
    Tangled = 24,
    TemporaryDexterity = 25,
    TemporaryFocus = 26,
    TemporaryStrength = 27,
    Tender = 28,
}

impl AfterSideTurnEndPowerToken {
    pub(crate) const ALL: [Self; 29] = [
        Self::BorrowedTime,
        Self::Burst,
        Self::Constrict,
        Self::ConsumingShadow,
        Self::CorrosiveWave,
        Self::DarkEmbrace,
        Self::Doom,
        Self::DoubleDamage,
        Self::Duplication,
        Self::Hellraiser,
        Self::Juggling,
        Self::Monologue,
        Self::NoDraw,
        Self::NoEnergyGain,
        Self::OneTwoPunch,
        Self::PaleBlueDot,
        Self::Panache,
        Self::Rage,
        Self::Rebound,
        Self::RetainHand,
        Self::Ringing,
        Self::Ritual,
        Self::Shadowmeld,
        Self::Smoggy,
        Self::Tangled,
        Self::TemporaryDexterity,
        Self::TemporaryFocus,
        Self::TemporaryStrength,
        Self::Tender,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::BorrowedTime => "borrowed_time",
            Self::Burst => "burst",
            Self::Constrict => "constrict",
            Self::ConsumingShadow => "consuming_shadow",
            Self::CorrosiveWave => "corrosive_wave",
            Self::DarkEmbrace => "dark_embrace",
            Self::Doom => "doom",
            Self::DoubleDamage => "double_damage",
            Self::Duplication => "duplication",
            Self::Hellraiser => "hellraiser",
            Self::Juggling => "juggling",
            Self::Monologue => "monologue",
            Self::NoDraw => "no_draw",
            Self::NoEnergyGain => "no_energy_gain",
            Self::OneTwoPunch => "one_two_punch",
            Self::PaleBlueDot => "pale_blue_dot",
            Self::Panache => "panache",
            Self::Rage => "rage",
            Self::Rebound => "rebound",
            Self::RetainHand => "retain_hand",
            Self::Ringing => "ringing",
            Self::Ritual => "ritual",
            Self::Shadowmeld => "shadowmeld",
            Self::Smoggy => "smoggy",
            Self::Tangled => "tangled",
            Self::TemporaryDexterity => "temporary_dexterity",
            Self::TemporaryFocus => "temporary_focus",
            Self::TemporaryStrength => "temporary_strength",
            Self::Tender => "tender",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|token| token.as_str() == value)
    }

    /// Return the ordinary side-end object type for a scalar player-power
    /// carrier.  The remaining ten tokens use dedicated cold/packed carriers
    /// (or independently-instanced records) and are wired at their bespoke
    /// acquisition sites.
    pub(crate) const fn from_power(power: PowerId) -> Option<Self> {
        Some(match power {
            PowerId::BorrowedTime => Self::BorrowedTime,
            PowerId::Burst => Self::Burst,
            PowerId::ConsumingShadow => Self::ConsumingShadow,
            PowerId::CorrosiveWave => Self::CorrosiveWave,
            PowerId::DarkEmbrace => Self::DarkEmbrace,
            PowerId::Doom => Self::Doom,
            PowerId::DoubleDamage => Self::DoubleDamage,
            PowerId::Hellraiser => Self::Hellraiser,
            PowerId::Juggling => Self::Juggling,
            PowerId::NoDraw => Self::NoDraw,
            PowerId::OneTwoPunch => Self::OneTwoPunch,
            PowerId::PaleBlueDot => Self::PaleBlueDot,
            PowerId::Rage => Self::Rage,
            PowerId::Rebound => Self::Rebound,
            PowerId::RetainHand => Self::RetainHand,
            PowerId::Shadowmeld => Self::Shadowmeld,
            PowerId::Smoggy => Self::Smoggy,
            PowerId::Tangled => Self::Tangled,
            PowerId::TempDexterity => Self::TemporaryDexterity,
            _ => return None,
        })
    }

    fn from_ordinal(value: u32) -> Option<Self> {
        Self::ALL.get(value as usize).copied()
    }

    pub(crate) const fn is_instanced(self) -> bool {
        matches!(self, Self::Panache | Self::Monologue)
    }
}

/// One exact native object identity in local-player acquisition order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct AfterSideTurnEndPowerEntry {
    pub token: AfterSideTurnEndPowerToken,
    pub uid: u32,
}

/// Stable native type token for one local-player power object registered in
/// the ordinary `AfterCardPlayed` hook. The three InstanceType-1 families have
/// one row per object; every other token is a keyed singleton whose row and
/// uid survive restacks.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum AfterCardPlayedPowerToken {
    Withering = 0,
    Rupture = 1,
    Afterimage = 2,
    Rage = 3,
    Sneaky = 4,
    Calamity = 5,
    Subroutine = 6,
    Storm = 7,
    HauntPower = 8,
    DevourLife = 9,
    SerpentForm = 10,
    PaleBlueDot = 11,
    Smoggy = 12,
    BlackHole = 13,
    MasterPlanner = 14,
    Tender = 15,
    VoidForm = 16,
    Panache = 17,
    Monologue = 18,
}

impl AfterCardPlayedPowerToken {
    pub(crate) const ALL: [Self; 19] = [
        Self::Withering,
        Self::Rupture,
        Self::Afterimage,
        Self::Rage,
        Self::Sneaky,
        Self::Calamity,
        Self::Subroutine,
        Self::Storm,
        Self::HauntPower,
        Self::DevourLife,
        Self::SerpentForm,
        Self::PaleBlueDot,
        Self::Smoggy,
        Self::BlackHole,
        Self::MasterPlanner,
        Self::Tender,
        Self::VoidForm,
        Self::Panache,
        Self::Monologue,
    ];

    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Withering => "withering",
            Self::Rupture => "rupture",
            Self::Afterimage => "afterimage",
            Self::Rage => "rage",
            Self::Sneaky => "sneaky",
            Self::Calamity => "calamity",
            Self::Subroutine => "subroutine",
            Self::Storm => "storm",
            Self::HauntPower => "haunt_power",
            Self::DevourLife => "devour_life",
            Self::SerpentForm => "serpent_form",
            Self::PaleBlueDot => "pale_blue_dot",
            Self::Smoggy => "smoggy",
            Self::BlackHole => "black_hole",
            Self::MasterPlanner => "master_planner",
            Self::Tender => "tender",
            Self::VoidForm => "void_form",
            Self::Panache => "panache",
            Self::Monologue => "monologue",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|token| token.as_str() == value)
    }

    pub(crate) const fn from_power(power: PowerId) -> Option<Self> {
        Some(match power {
            PowerId::Rupture => Self::Rupture,
            PowerId::Afterimage => Self::Afterimage,
            PowerId::Rage => Self::Rage,
            PowerId::Sneaky => Self::Sneaky,
            PowerId::Calamity => Self::Calamity,
            PowerId::Subroutine => Self::Subroutine,
            PowerId::Storm => Self::Storm,
            PowerId::HauntPower => Self::HauntPower,
            PowerId::DevourLife => Self::DevourLife,
            PowerId::SerpentForm => Self::SerpentForm,
            PowerId::PaleBlueDot => Self::PaleBlueDot,
            PowerId::Smoggy => Self::Smoggy,
            PowerId::BlackHole => Self::BlackHole,
            PowerId::MasterPlanner => Self::MasterPlanner,
            PowerId::Tender => Self::Tender,
            PowerId::VoidForm => Self::VoidForm,
            _ => return None,
        })
    }

    pub(crate) const fn is_instanced(self) -> bool {
        matches!(self, Self::Withering | Self::Panache | Self::Monologue)
    }

    /// Imitation Learning is still outside the ordinary player-power ledger.
    /// These are exactly the already-live objects whose position relative to
    /// a newly acquired Imitation object can affect its nested Power play.
    pub(crate) const fn blocks_later_imitation_acquisition(self) -> bool {
        matches!(
            self,
            Self::Withering
                | Self::Afterimage
                | Self::Rage
                | Self::Sneaky
                | Self::Subroutine
                | Self::Storm
                | Self::DevourLife
                | Self::SerpentForm
                | Self::PaleBlueDot
                | Self::Smoggy
                | Self::BlackHole
                | Self::MasterPlanner
                | Self::Tender
                | Self::Panache
                | Self::Monologue
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct AfterCardPlayedPowerEntry {
    pub token: AfterCardPlayedPowerToken,
    pub uid: u32,
}

/// Private, synchronous native command receipts shared until a branch mutates.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct DamageExecution {
    /// Native Damage command stack. Entries retain identities whose lethal HP
    /// committed but whose cleanup has not begun. These cannot be imported.
    batches: Vec<Vec<u32>>,
    /// Death commands inside this execution, after ordinary cleanup but before
    /// their awaited listener/cascade suffix returns. Never a live-power proof.
    cleanups: Vec<u32>,
    /// Awaiting Panache invocations; repeated UIDs are reentrant calls.
    panache: Vec<u32>,
}

/// Cold, variable-width player-power state behind the existing Arc word.
///
/// The former field was `Arc<Vec<u32>>` for Fetch completion identities. R34
/// generalizes that already-paid pointer into one nested COW record so Bomb
/// instances, Toric's keyed Decimal record, the unbounded Bomb listener
/// order, and a separately nested potion belt do not widen HotState or
/// FanoutState. Search clones share every Arc layer until its cold writer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct ColdFanoutState {
    /// Native Player.IsActiveForHooks becomes false only after actual death.
    /// HP zero before ShouldDie and combat-ending victory do not imply it.
    player_hooks_deactivated: bool,
    /// `SurroundedPower._facing`, stored **plus one** so the derived
    /// `Default` (zero) is the absent sentinel rather than a live direction.
    ///
    /// Native v0.111.0 (DLL `9cb4f1ad`): the power is a PLAYER power —
    /// `Rocket/<AfterAddedToRoom>d__29::MoveNext` `0x367c94` IL_008b-IL_00ae
    /// applies `SurroundedPower` to `CombatState.GetOpponentsOf(rocket)`,
    /// not to the monster. Its backing field is `SurroundedPower::_facing`
    /// (`get_Facing` `0xa8bac`, `set_Facing` `0xa8bb4`) and the only writer is
    /// `FaceDirection` (`0xa8d6c`, body `0x34770c` IL_0027-IL_002e).
    ///
    /// Wire values match Python's `State.kaiser_facing` (`_update_kaiser_facing`, frozen Python, deleted #2827): `-1` absent, `0` the direction in which Crusher is the back
    /// attacker, `1` the direction in which Rocket is.
    kaiser_facing_plus_one: u8,
    /// `RunState.AscensionLevel`, stored as its distance **below**
    /// [`crate::encounters::MODELED_ASCENSION`] so the derived `Default`
    /// (zero) is that level — the tier every root was modeled at before
    /// #2539, and the wire default the boundary elides. Read through
    /// [`FanoutState::ascension`]; every ascension-tiered monster constant
    /// is selected by it (`encounters::tier`).
    ascension_below_modeled: u8,
    /// Whether this fight records physical power attachments for monster
    /// `StrengthPower` (#2693 S1).
    ///
    /// Mirrored once per fight from
    /// [`crate::catalog::Catalog::misery_is_reachable`] at boundary
    /// hydration, because the write path reaches `HotMonster` through callers
    /// that do not all carry a catalog. It lives behind the cold pointer
    /// rather than in `FanoutState` so the reviewed 312-byte fanout
    /// allocation and the 224-byte search state are both unmoved; the read is
    /// one extra indirection on a path that is already taking a monster
    /// copy-on-write.
    ///
    /// It is a *derived* per-fight constant and is deliberately **not**
    /// projected: a reload re-derives it from the same immutable catalog, so
    /// a cold copy compares equal to the state the engine built.
    ///
    /// It gates upkeep, not correctness. `Misery` is the ledger's only
    /// reader, so a fight that cannot reach it records no Strength row and
    /// projects byte-identically to the pre-#2693 engine. Absence of a row is
    /// a first-class "provenance not recorded", and every reader refuses on
    /// it exactly where it refused before.
    misery_attachment_upkeep: bool,
    /// Normality's `CardsPlayedThisTurn` projection: the owner's
    /// `CombatHistory.CardPlaysStarted` entries that `HappenedThisTurn`
    /// (Python `State.normality_card_plays_started_this_turn`).
    ///
    /// Maintained only in a fight whose catalog interns Normality
    /// ([`crate::catalog::Catalog::has_normality`]); everywhere else it stays
    /// at the canonical zero, so an unrelated fight never writes this cold
    /// record. It lives behind the nested cold pointer rather than in
    /// [`HotHistory`] because the inline history block and the 224-byte
    /// search state have no spare bytes, and only a Normality fight pays the
    /// copy-on-write. See `engine::play::normality_blocks_card_plays`.
    normality_card_plays_started_this_turn: i32,
    /// Complete native `Creature.Powers` acquisition order for the closed
    /// six-member AfterEnergyReset surface. `None` is deliberately distinct
    /// from `Some([])`: old canonical snapshots did not carry the cross-family
    /// order, while a combat-entry root with no listener proves known-empty.
    ///
    /// v0.111.0 DLL `Creature.ApplyPowerInternal` 0x11da0c IL006f appends a
    /// successful first attachment; stacks retain place and a later reapply
    /// appends again. `CombatState.Iterator` 0x3f9720 snapshots that list.
    after_energy_reset_order: Option<AfterEnergyResetOrder>,
    /// Private synchronous receipts behind one optional cold pointer so ordinary
    /// fanout copy-on-write does not pay for empty execution stacks.
    damage_execution: Option<Arc<DamageExecution>>,
    /// Hello World's delta beyond the inline first stack. Zero has no cold
    /// allocation/write on the existing one-application path.
    hello_world_snapshot_extra_delta: i32,
    fetch_finished_uids: Vec<u32>,
    power_records: Vec<ColdPowerRecord>,
    next_the_bomb_uid: u32,
    /// Spill for native side-end object ledgers of length two or greater.
    /// The one-entry case stays inline in `FanoutState`, so the common first
    /// application pays only the already-budgeted outer fanout COW.
    after_side_turn_end: Vec<AfterSideTurnEndPowerEntry>,
    /// Complete native ordinary AfterCardPlayed player-power object list in
    /// acquisition order. It shares the side-end allocator because objects
    /// participating in both hooks have one native identity.
    after_card_played: Vec<AfterCardPlayedPowerEntry>,
    /// Low word: deferred three-option The Hunt room rewards. High word:
    /// ending-gated inert TheHuntPower amount.
    the_hunt_state: u64,
    before_side_turn_end: Vec<BeforeSideTurnEndToken>,
    /// Exact nullable potion-belt topology and its immutable entry facts.
    /// A second Arc keeps unrelated fanout mutations from copying the belt.
    potion_belt: Arc<PotionBeltState>,
    /// Mutable state for the eighth 20-relic port. These counters change
    /// only at hook boundaries, so keeping them behind the existing nested
    /// COW pointer preserves the 224-byte search state and 312-byte fanout.
    batch_eight_relics: BatchEightRelicState,
    /// Mutable state for the ninth 20-relic port. Its retained card payloads
    /// make it substantially wider than batch eight, so a second Arc keeps
    /// unrelated fanout mutations from copying these cold bytes. Search clones
    /// still share both layers until a batch-nine relic actually writes.
    batch_nine_relics: Arc<BatchNineRelicState>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct BatchEightRelicState {
    beating_remnant_owned: bool,
    beating_remnant_damage_received: i32,
    brilliant_scarf_owned: bool,
    ectoplasm_owned: bool,
    lizard_tail_owned: bool,
    ruined_helmet_owned: bool,
    bone_tea_combats_left: u8,
    burning_sticks_used: bool,
    galactic_dust: u8,
    iron_club_cards: u8,
    kusarigama: u8,
    lizard_tail_used: bool,
    mini_regent_used: bool,
    pen_nib: u8,
    throwing_axe_available: bool,
    tuning_fork_skills: i32,
    vambrace_available: bool,
    vambrace_trigger_uid: Option<u32>,
    /// Immutable Undying Sigil ownership, cached for the catalogless monster
    /// attack snapshot (`monster_attack_hit_damage`), exactly as Paper Krane
    /// is cached in `relic_state`.
    undying_sigil_owned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct BatchNineRelicState {
    history_course_attack_current_turn: Option<FrozenAutoBatchEntry>,
    history_course_attack_previous_turn: Option<FrozenAutoBatchEntry>,
    puzzle_armed: bool,
    music_box_used_this_turn: bool,
    music_box_card_uid: Option<u32>,
    joss_paper_cards_exhausted: i32,
    joss_paper_ethereal_count: i32,
    pumpkin_candle_kindle_count: i32,
    paels_legion_cooldown: i8,
    paels_legion_trigger_uid: Option<u32>,
    paels_legion_triggered_last_turn: bool,
    paels_eye_used: bool,
    paels_eye_was_owner_part_last_player_turn: bool,
    /// Native `CombatTurnState.PlayersTakingExtraTurn` is non-empty (#3049).
    players_taking_extra_turn: bool,
    regalite_used_this_turn: bool,
    self_forming_clay_owned: bool,
    gremlin_horn_owned: bool,
    /// The queued deferred hook action of the running public transaction
    /// (#3387). Never serialized: the boundary refuses a state carrying it.
    deferred_hook_action: Option<Arc<DeferredHookAction>>,
    /// Immutable Red Skull ownership, cached so every player-HP writer can
    /// run its `AfterCurrentHpChanged` Strength transition without a
    /// catalog (#3044, `engine::damage::red_skull_after_player_hp_changed`).
    red_skull_owned: bool,
    /// Immutable Spiked Gauntlets ownership, cached for the catalogless
    /// energy-cost fold (#3437,
    /// `engine::play::spiked_gauntlets_surcharged_energy_cost`). It lives in
    /// this separately shared layer so batch eight's cold record, which the
    /// turn path copies, does not widen.
    spiked_gauntlets_owned: bool,
    /// Red Skull's native `StrengthApplied` latch disagrees with the HP
    /// quotient (#3044). Only the opening sets it — a Red Skull that ran
    /// before Planisphere's crossing heal — and the next player HP write
    /// consumes it. It is never serialized: the boundary refuses a state that
    /// still carries it.
    red_skull_latch_stale: bool,
    pending: Option<RelicPendingRecord>,
    /// Native `CombatState._nextCreatureId`, in monster-uid units (#3039).
    ///
    /// v0.111.0 DLL `9cb4f1ad`: `CombatState::AttachCreature` (RVA
    /// `0x13718f`) stamps `CombatId = _nextCreatureId` at `IL_0009`-`IL_0014`
    /// and increments the field at `IL_0019`-`IL_0022` for **every** attached
    /// creature: the player (`AddPlayer` `0x137059` `IL_0008`), each encounter
    /// enemy and each later `CreatureCmd.Add` spawn (`CreateCreature`
    /// `0x137074` `IL_007e`), and each pet `PlayerCmd.AddPet` creates
    /// (`<AddPet>d__14`1::MoveNext` `0x3ee68c` `IL_0054`). A monster's uid is
    /// its `CombatId` minus the solo player's id 0, so this stores the uid
    /// the next created creature takes.
    ///
    /// Zero is the canonical default and means "not tracked": the spawners
    /// then fall back to `max(monster uid) + 1`, which is exact exactly when
    /// no pet creature holds an id (the pre-#3039 behaviour, byte-identical
    /// for every pet-less fight). The opening seeds it only when a
    /// combat-start pet exists (`engine::seed_creature_uid_counter`).
    ///
    /// It lives behind batch nine's own Arc, beside Pael's Legion's pet
    /// state, because it is written only at the opening and at a creature
    /// creation: the inline cold record every fanout copy-on-write clones
    /// stays at its measured size.
    next_creature_uid: u32,
    /// How many passive relic pets (Byrdpip, Pael's Legion) this fight's
    /// player has (#3039). Each is a creature holding a native creature id
    /// (`<AddPet>d__14`1::MoveNext` `0x3ee68c` `IL_0054`).
    ///
    /// The pets are a projection of relic ownership (`boundary.rs`
    /// `PlayerSlot::RelicPets`, which `from_canonical` checks against the
    /// document's `relic_pets` roster), so this is mirrored once per fight at
    /// boundary hydration, exactly like `misery_attachment_upkeep`, and is
    /// deliberately **not** projected: a reload re-derives it from the same
    /// catalog. The spawners read it because they carry no catalog.
    relic_pet_creatures: u8,
    /// Execution-only: answers this state's lineage has consumed from an
    /// ActionReplay re-execution tape (#3114, `engine::puzzle`). A clone (a
    /// rehearsal probe) copies-on-write its own cursor, so a probe can never
    /// consume the committed run's answers. Zero in every published state;
    /// it has no wire form.
    replay_tape_cursor: u16,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RelicPendingKind {
    ChoicesParadox,
    GamblingChip,
    ToastyMittens,
    Toolbox,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct RelicPendingRecord {
    pub kind: RelicPendingKind,
    pub entries: Vec<FrozenAutoBatchEntry>,
}

impl Default for BatchNineRelicState {
    fn default() -> Self {
        Self {
            history_course_attack_current_turn: None,
            history_course_attack_previous_turn: None,
            puzzle_armed: false,
            music_box_used_this_turn: false,
            music_box_card_uid: None,
            joss_paper_cards_exhausted: -1,
            joss_paper_ethereal_count: 0,
            pumpkin_candle_kindle_count: -1,
            paels_legion_cooldown: -1,
            paels_legion_trigger_uid: None,
            paels_legion_triggered_last_turn: false,
            paels_eye_used: false,
            paels_eye_was_owner_part_last_player_turn: true,
            players_taking_extra_turn: false,
            regalite_used_this_turn: false,
            self_forming_clay_owned: false,
            gremlin_horn_owned: false,
            deferred_hook_action: None,
            red_skull_owned: false,
            spiked_gauntlets_owned: false,
            red_skull_latch_stale: false,
            pending: None,
            next_creature_uid: 0,
            relic_pet_creatures: 0,
            replay_tape_cursor: 0,
        }
    }
}

const POTION_BELT_SOZU_MASK: u8 = 0b0000_0001;
const POTION_BELT_BUCKLE_MASK: u8 = 0b0000_0010;
const POTION_BELT_BUCKLE_APPLIED_MASK: u8 = 0b0000_0100;
const POTION_BELT_STRICT_MASK: u8 = 0b0000_1000;
const POTION_BELT_POOL_PROOF_MASK: u8 = 0b0001_0000;
const POTION_BELT_UNCEASING_TOP_MASK: u8 = 0b0010_0000;
const POTION_BELT_NO_ENERGY_GAIN_MASK: u8 = 0b0100_0000;
const POTION_BELT_SLOT_FLAGS_MASK: u8 = POTION_BELT_SOZU_MASK
    | POTION_BELT_BUCKLE_MASK
    | POTION_BELT_BUCKLE_APPLIED_MASK
    | POTION_BELT_STRICT_MASK
    | POTION_BELT_POOL_PROOF_MASK;
const POTION_BELT_FLAGS_MASK: u8 =
    POTION_BELT_SLOT_FLAGS_MASK | POTION_BELT_UNCEASING_TOP_MASK | POTION_BELT_NO_ENERGY_GAIN_MASK;

/// Cold exact potion-belt state. `slots.len()` is the fixed native capacity;
/// nulls are retained and duplicate potion identities remain distinct by
/// slot. The pool-proof bit means the current-build fully-unlocked pool was
/// authenticated at entry rather than inferred from any dense mirror.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct PotionBeltState {
    slots: Vec<Option<PotionId>>,
    flags: u8,
    /// Potion-created player powers whose generated ids are deliberately not
    /// added to the frozen `PowerId` vocabulary. They share the belt's
    /// already-paid nested COW allocation and are canonical state.
    duplication: i32,
    clarity: i32,
    player_ritual: i32,
    gigantification: i32,
    radiance: i32,
    regen: i32,
    gigantification_bound: bool,
    /// Retained padding byte after Clarity's insertion ordinal moved into the
    /// complete compact listener lane.
    reserved_after_side_turn_start: u8,
    /// Zero for an absent/singleton hit-listener set; otherwise the
    /// factoradic permutation rank plus one for the complete live set.
    /// This consumes one more byte of existing padding without widening the
    /// 56-byte cold record.
    after_damage_received_order_code: u8,
}

/// The acquisition-ordered card-event listener surface and private counters.
///
/// Native stores the three order lists on `State` and the countdowns/latches
/// inside power objects. They change far less often than play-history, so one
/// copy-on-write block keeps ordinary search clones pointer-sized while fixed
/// arrays avoid a second allocation on the first listener application.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct FanoutState {
    after_card_drawn: [PowerId; MAX_AFTER_CARD_DRAWN_POWERS],
    after_card_exhausted: [PowerId; MAX_AFTER_CARD_EXHAUSTED_POWERS],
    before_hand_draw: [PowerId; MAX_BEFORE_HAND_DRAW_POWERS],
    after_damage_given: [PowerId; MAX_AFTER_DAMAGE_GIVEN_POWERS],
    after_block_gained: [PowerId; MAX_AFTER_BLOCK_GAINED_POWERS],
    after_block_cleared: [PowerId; MAX_AFTER_BLOCK_CLEARED_POWERS],
    after_power_amount_changed: [PowerAmountChangedPower; MAX_AFTER_POWER_AMOUNT_CHANGED_POWERS],
    after_side_turn_start: [AfterSideTurnStartToken; MAX_AFTER_SIDE_TURN_START_POWERS],
    after_player_turn_start: [PowerId; MAX_STORED_AFTER_PLAYER_TURN_START_POWERS],
    /// Aeonglass's owner-disjoint Withering Presence countdown. This reuses
    /// the retired inline BeforeSideTurnEnd word retained to freeze the
    /// reviewed 312-byte allocation: zero is absent and 1..=6 is live.
    withering_cards_left: [u16; 1],
    star_energy_reset: [PowerId; MAX_STAR_ENERGY_RESET_POWERS],
    local_generated: [PowerId; MAX_LOCAL_GENERATED_POWERS],
    result_location: [ResultLocationPower; MAX_RESULT_LOCATION_POWERS],
    /// MonologuePower's native `StrengthApplied` ledger.
    ///
    /// The private foundation admits only the exact singleton acquisition
    /// projection for each of its three hooks. Bits zero through two record
    /// `BeforeCardPlayed`, `AfterCardPlayed`, and `AfterSideTurnEnd`;
    /// respectively. Zero is the only valid inactive representation and
    /// `0b111` the only valid live representation.
    monologue_strength_applied: i32,
    /// VoidFormPower.Data.cardsPlayedThisTurn, including the current-build
    /// 999,999,999 first-application/restack sentinel.
    void_form_cards_played_this_turn: i32,
    private_power_flags: u8,
    after_card_drawn_len: u8,
    after_card_exhausted_len: u8,
    before_hand_draw_len: u8,
    after_damage_given_len: u8,
    after_block_gained_len: u8,
    after_block_cleared_len: u8,
    after_power_amount_changed_len: u8,
    after_side_turn_start_len: u8,
    /// Low two bits and bit seven jointly encode result-location length
    /// 0..=4. Bits two and three: AfterEnergyReset length 0..=2. Bits four
    /// through six: local-generated length 0..=4. Packing the formerly
    /// separate result-location byte makes room for the inline side-end uid
    /// without moving the hot Intercept metadata behind the nested COW.
    turn_and_generated_lens: u8,
    /// Low nibble: Automation's 0..=15 countdown. Bits four through six:
    /// Panache's 0..=7 countdown. Bit seven: the private Tender singleton.
    /// These closed ranges recover the full signed Tender counter without
    /// widening the reviewed FanoutState allocation.
    automation_panache_tender: u8,
    cacophony_left: u8,
    cacophony_resets_completed: i32,
    dark_embrace_ethereal: i32,
    ethereal_plays_finished_combat: i32,
    /// TenderPower.Data.CardsPlayedThisTurn. The active singleton bit is
    /// packed with the two bounded countdowns above; a zero bit requires a
    /// zero counter, while a live bit admits every nonnegative Int32 value.
    tender_cards_played: i32,
    /// Shared native object identity for the ordinary `AfterSideTurnEnd`
    /// hook. A single entry and the full-width next uid fit in the reclaimed
    /// outer bytes; two or more entries spill to `ColdFanoutState`.
    after_side_turn_end_inline: Option<AfterSideTurnEndPowerEntry>,
    next_after_side_turn_end_power_uid: u32,
    /// The singular local Osty plus its owner-turn attack history. This
    /// belongs behind the already-pointer-sized COW handle: growing the
    /// player-side pet must not widen the 224-byte search state.
    pet: SoloPetState,
    /// Exact physical Fetch sources that already finished this owner turn.
    cold: Arc<ColdFanoutState>,
    /// The one exact remote Player projection used by Unit C's ally cards.
    /// It remains behind this already-pointer-sized COW block so the hot
    /// search state stays at its reviewed 224-byte ceiling.
    multiplayer_ally: Option<MultiplayerAllyState>,
    /// Target-keyed Imitation Learning stacks and their frozen Power clones.
    imitation_learning: [i32; 2],
    imitation_clones: Arc<Vec<ImitationClone>>,
    /// Positive, three-aligned Amount of the one player ConstrictPower.
    ///
    /// The exact applier uid is the independently authenticated unique living
    /// Slithering Strangler. Keeping only native Amount here makes absence an
    /// O(1) player-side read without duplicating source identity.
    constrict_amount: i32,
    /// Bits zero and one are the local and exact remote Player's Intercept
    /// coverage. Bits two through four preserve their two-key acquisition
    /// order (encoded 0..=4); bits five through seven preserve Imitation
    /// Learning's independent two-key order (also 0..=4). This remains in the
    /// outer COW so Intercept cover/clear retains its one-allocation hot path.
    intercept_covered_state: u8,
}

impl Default for FanoutState {
    fn default() -> Self {
        // Entries beyond each published length are never read. Accuracy is a
        // deterministic filler, not a listener token.
        Self {
            after_card_drawn: [PowerId::Accuracy; MAX_AFTER_CARD_DRAWN_POWERS],
            after_card_exhausted: [PowerId::Accuracy; MAX_AFTER_CARD_EXHAUSTED_POWERS],
            before_hand_draw: [PowerId::Accuracy; MAX_BEFORE_HAND_DRAW_POWERS],
            after_damage_given: [PowerId::Accuracy; MAX_AFTER_DAMAGE_GIVEN_POWERS],
            after_block_gained: [PowerId::Accuracy; MAX_AFTER_BLOCK_GAINED_POWERS],
            after_block_cleared: [PowerId::Accuracy; MAX_AFTER_BLOCK_CLEARED_POWERS],
            after_power_amount_changed: [PowerAmountChangedPower::Vicious;
                MAX_AFTER_POWER_AMOUNT_CHANGED_POWERS],
            after_side_turn_start: [AfterSideTurnStartToken::BiasedCognition;
                MAX_AFTER_SIDE_TURN_START_POWERS],
            after_player_turn_start: [PowerId::Accuracy; MAX_STORED_AFTER_PLAYER_TURN_START_POWERS],
            withering_cards_left: [0; 1],
            star_energy_reset: [PowerId::Accuracy; MAX_STAR_ENERGY_RESET_POWERS],
            local_generated: [PowerId::Accuracy; MAX_LOCAL_GENERATED_POWERS],
            result_location: [ResultLocationPower::Corruption; MAX_RESULT_LOCATION_POWERS],
            monologue_strength_applied: 0,
            void_form_cards_played_this_turn: 0,
            private_power_flags: 0,
            after_card_drawn_len: 0,
            after_card_exhausted_len: 0,
            before_hand_draw_len: 0,
            after_damage_given_len: 0,
            after_block_gained_len: 0,
            after_block_cleared_len: 0,
            after_power_amount_changed_len: 0,
            after_side_turn_start_len: 0,
            turn_and_generated_lens: 0,
            automation_panache_tender: 10 | (5 << PANACHE_LEFT_SHIFT),
            cacophony_left: 33,
            cacophony_resets_completed: 0,
            dark_embrace_ethereal: 0,
            ethereal_plays_finished_combat: 0,
            tender_cards_played: 0,
            after_side_turn_end_inline: None,
            next_after_side_turn_end_power_uid: 0,
            pet: SoloPetState::default(),
            cold: Arc::new(ColdFanoutState::default()),
            multiplayer_ally: None,
            imitation_learning: [0; 2],
            imitation_clones: Arc::new(Vec::new()),
            constrict_amount: 0,
            intercept_covered_state: 0,
        }
    }
}

/// Exact physical card payload retained for the single represented remote
/// Player.
///
/// Ordinary remote cards are immutable identities. Sovereign Blade and The
/// Ball are the two admitted remote cards with one mutable `i32` dimension.
/// The lane stores Sovereign Blade's absolute Damage or The Ball's exact
/// nonnegative damage-growth amount; the identity domain separates them.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct MultiplayerAllyCard {
    pub identity: CardIdentity,
    mutable_i32: i32,
}

impl MultiplayerAllyCard {
    const IMMUTABLE: i32 = -1;

    pub(crate) fn immutable(identity: CardIdentity) -> Option<Self> {
        (!matches!(
            identity.id,
            crate::ids::CardId::SovereignBlade | crate::ids::CardId::TheBall
        ))
        .then_some(Self {
            identity,
            mutable_i32: Self::IMMUTABLE,
        })
    }

    pub(crate) fn sovereign_blade(identity: CardIdentity, damage: i32) -> Option<Self> {
        (matches!(identity.id, crate::ids::CardId::SovereignBlade)
            && identity.enchantment.is_none()
            && identity.upgrade <= 1
            && damage >= 0)
            .then_some(Self {
                identity,
                mutable_i32: damage,
            })
    }

    pub(crate) fn sovereign_blade_damage(self) -> Option<i32> {
        matches!(self.identity.id, crate::ids::CardId::SovereignBlade)
            .then_some(self.mutable_i32)
            .filter(|value| *value >= 0)
    }

    pub(crate) fn the_ball(identity: CardIdentity, growth: i32) -> Option<Self> {
        (matches!(identity.id, crate::ids::CardId::TheBall)
            && identity.enchantment.is_none()
            && identity.upgrade <= 1
            && growth >= 0)
            .then_some(Self {
                identity,
                mutable_i32: growth,
            })
    }

    pub(crate) fn the_ball_growth(self) -> Option<i32> {
        matches!(self.identity.id, crate::ids::CardId::TheBall)
            .then_some(self.mutable_i32)
            .filter(|value| *value >= 0)
    }

    pub(crate) fn add_forge_damage(&mut self, amount: i32) -> Option<()> {
        self.mutable_i32 = self
            .sovereign_blade_damage()
            .and_then(|damage| damage.checked_add(amount))?;
        Some(())
    }
}

/// The exact mutable quotient of the single represented remote Player.
///
/// This is intentionally not another [`HotState`]: the solver never predicts
/// teammate choices or turns. Only fields observed or mutated by represented
/// ally-card programs live here. Empty listener surfaces are validated
/// and discarded at the canonical boundary rather than guessed at runtime.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MultiplayerAllyState {
    pub key: u32,
    pub alive: bool,
    pub draw: Arc<Vec<MultiplayerAllyCard>>,
    pub hand: Arc<Vec<MultiplayerAllyCard>>,
    pub discard: Arc<Vec<MultiplayerAllyCard>>,
    pub shuffle_rng: Option<RngStreamState>,
    pub owner_generated_cards_combat: i32,
    pub energy: i32,
    pub no_energy_gain: bool,
    pub strength: i32,
    pub temp_strength: i32,
    pub temp_dexterity: i32,
    pub block: i32,
    pub draw_next_turn: i32,
    pub one_for_all: i32,
    /// Exact current-build Colorless unlock provenance used by Largesse.
    ///
    /// The wire carries the five sorted epoch names; the hot quotient needs
    /// only whether that closed set is present because no admitted body
    /// observes a partial remote Colorless pool.
    pub fully_unlocked_colorless_epochs: bool,
    /// Unique local-owner Hammer Time power. This fourth bool consumes the
    /// quotient's existing trailing padding; the frozen 120-byte carrier and
    /// 312-byte fanout ceiling do not move.
    pub(crate) hammer_time: bool,
    /// Frozen external teammate Power-card callback source identity.
    ///
    /// The target key was redundant with this carrier's authenticated `key`:
    /// every public accessor still exposes `(key, source)`, while storing only
    /// the source frees exactly four bytes for temporary Dexterity without
    /// moving the frozen 120-byte ally quotient.
    pub(crate) teammate_power_pending: Option<u32>,
}

impl Default for MultiplayerAllyState {
    fn default() -> Self {
        Self {
            key: 0,
            alive: true,
            draw: Arc::new(Vec::new()),
            hand: Arc::new(Vec::new()),
            discard: Arc::new(Vec::new()),
            shuffle_rng: None,
            owner_generated_cards_combat: 0,
            energy: 0,
            no_energy_gain: false,
            strength: 0,
            temp_strength: 0,
            temp_dexterity: 0,
            block: 0,
            draw_next_turn: 0,
            one_for_all: 0,
            fully_unlocked_colorless_epochs: false,
            hammer_time: false,
            teammate_power_pending: None,
        }
    }
}

/// Shared zero-cost view for solo states. The per-state remote quotient and
/// its three pile COW handles are allocated only when a party is represented.
static DEFAULT_MULTIPLAYER_ALLY: LazyLock<MultiplayerAllyState> =
    LazyLock::new(MultiplayerAllyState::default);

/// One frozen Imitation Learning clone correlated to its Power-card source.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImitationClone {
    pub target_key: u32,
    pub source_identity: u32,
    pub clone_uid: u32,
    pub identity: CardIdentity,
    /// MutableClone copies the complete physical payload before the source
    /// body runs. Keep both slot-presence bits and ordered side state frozen
    /// in the correlation record until the clone enters Play.
    pub flags: u16,
    pub instance: CardInstanceState,
}

/// Copy-on-write card-event listener state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct HotFanouts(Arc<FanoutState>);

impl HotFanouts {
    /// Canonical dataclass defaults.
    pub fn new() -> Self {
        // A generic canonical state does not prove that it is a combat entry.
        // The review-root adapter supplies the explicit known-empty witness for
        // genuine entries; otherwise preserve legacy absence as unknown.
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn shares_store_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// CombatHistory's count of completed effective-Ethereal physical plays.
    pub(crate) fn ethereal_plays_finished_combat(&self) -> i32 {
        self.0.ethereal_plays_finished_combat
    }

    pub(crate) fn set_ethereal_plays_finished_combat(&mut self, value: i32) -> bool {
        if value < 0 {
            return false;
        }
        Arc::make_mut(&mut self.0).ethereal_plays_finished_combat = value;
        true
    }

    pub(crate) fn record_ethereal_play_finished(&mut self) -> Result<(), ()> {
        let state = Arc::make_mut(&mut self.0);
        state.ethereal_plays_finished_combat = state
            .ethereal_plays_finished_combat
            .checked_add(1)
            .ok_or(())?;
        Ok(())
    }

    pub(crate) fn can_record_ethereal_plays(&self, count: i32) -> bool {
        count >= 0
            && self
                .0
                .ethereal_plays_finished_combat
                .checked_add(count)
                .is_some()
    }

    /// Count of deferred, unpopulated three-option The Hunt room rewards.
    pub(crate) fn the_hunt_reward_count(&self) -> i32 {
        self.0.cold.the_hunt_state as u32 as i32
    }

    /// Native TheHuntPower amount. It has no owned hook body; its generic
    /// AfterPowerAmountChanged walk is authenticated at the applying step.
    pub(crate) fn the_hunt_marker(&self) -> i32 {
        (self.0.cold.the_hunt_state >> 32) as u32 as i32
    }

    /// Replace the complete compact The Hunt state after boundary validation.
    pub(crate) fn set_the_hunt_state(&mut self, rewards: i32, marker: i32) -> bool {
        if rewards < 0 || marker < 0 {
            return false;
        }
        let state = Arc::make_mut(&mut self.0);
        Arc::make_mut(&mut state.cold).the_hunt_state =
            u64::from(rewards as u32) | (u64::from(marker as u32) << 32);
        true
    }

    /// Append one deferred reward descriptor and, while combat remains live,
    /// one inert TheHuntPower stack.
    pub(crate) fn record_the_hunt_kill(&mut self, apply_marker: bool) -> Result<(), ()> {
        let rewards = self.the_hunt_reward_count().checked_add(1).ok_or(())?;
        let marker = if apply_marker {
            self.the_hunt_marker().checked_add(1).ok_or(())?
        } else {
            self.the_hunt_marker()
        };
        let written = self.set_the_hunt_state(rewards, marker);
        debug_assert!(written);
        Ok(())
    }

    /// Native application order for ordinary draw listeners.
    pub(crate) fn after_card_drawn_order(&self) -> &[PowerId] {
        &self.0.after_card_drawn[..usize::from(self.0.after_card_drawn_len)]
    }

    /// Native application order for exhaust listeners.
    pub(crate) fn after_card_exhausted_order(&self) -> &[PowerId] {
        &self.0.after_card_exhausted[..usize::from(self.0.after_card_exhausted_len)]
    }

    /// Native application order for pre-hand-draw listeners.
    pub(crate) fn before_hand_draw_order(&self) -> &[PowerId] {
        &self.0.before_hand_draw[..usize::from(self.0.before_hand_draw_len)]
    }

    pub(crate) fn after_damage_given_order(&self) -> &[PowerId] {
        &self.0.after_damage_given[..usize::from(self.0.after_damage_given_len)]
    }
    pub(crate) fn after_block_gained_order(&self) -> &[PowerId] {
        &self.0.after_block_gained[..usize::from(self.0.after_block_gained_len)]
    }
    pub(crate) fn after_block_cleared_order(&self) -> &[PowerId] {
        &self.0.after_block_cleared[..usize::from(self.0.after_block_cleared_len)]
    }
    pub(crate) fn after_power_amount_changed_order(&self) -> PowerAmountChangedOrder {
        let len = self.0.after_power_amount_changed_len;
        let mut powers = [PowerId::Vicious; MAX_AFTER_POWER_AMOUNT_CHANGED_POWERS];
        for (target, source) in powers
            .iter_mut()
            .zip(self.0.after_power_amount_changed.iter())
            .take(usize::from(len))
        {
            *target = source.power();
        }
        PowerAmountChangedOrder { powers, len }
    }
    pub(crate) fn after_side_turn_start_order(&self) -> &[AfterSideTurnStartToken] {
        let len = self.0.after_side_turn_start_len & AFTER_SIDE_TURN_START_LEN_MASK;
        &self.0.after_side_turn_start[..usize::from(len)]
    }
    /// The seven non-Entropy members stored in their dedicated compact lane.
    /// Entropy's insertion ordinal lives in `HotState::generated_power_flags`.
    pub(crate) fn stored_after_player_turn_start_order(&self) -> AfterPlayerTurnStartOrder {
        let len = (self.0.after_side_turn_start_len & AFTER_PLAYER_TURN_START_LEN_MASK)
            >> AFTER_PLAYER_TURN_START_LEN_SHIFT;
        let mut powers = [PowerId::Accuracy; MAX_AFTER_PLAYER_TURN_START_POWERS];
        if usize::from(len) > MAX_STORED_AFTER_PLAYER_TURN_START_POWERS {
            return AfterPlayerTurnStartOrder { powers, len: 0 };
        }
        powers[..usize::from(len)]
            .copy_from_slice(&self.0.after_player_turn_start[..usize::from(len)]);
        AfterPlayerTurnStartOrder { powers, len }
    }
    pub(crate) fn after_player_turn_start_order_is_exact(&self) -> bool {
        let len = usize::from(
            (self.0.after_side_turn_start_len & AFTER_PLAYER_TURN_START_LEN_MASK)
                >> AFTER_PLAYER_TURN_START_LEN_SHIFT,
        );
        if len > MAX_STORED_AFTER_PLAYER_TURN_START_POWERS {
            return false;
        }
        let order = &self.0.after_player_turn_start[..len];
        len <= MAX_STORED_AFTER_PLAYER_TURN_START_POWERS
            && order.iter().all(|power| {
                matches!(
                    power,
                    PowerId::Loop
                        | PowerId::RollingBoulder
                        | PowerId::SummonNextTurn
                        | PowerId::Inferno
                        | PowerId::CrimsonMantle
                        | PowerId::ToolsOfTheTrade
                        | PowerId::Tyranny
                )
            })
            && order
                .iter()
                .enumerate()
                .all(|(index, power)| !order[..index].contains(power))
    }
    fn star_energy_reset_len(&self) -> u8 {
        (self.0.turn_and_generated_lens & STAR_ENERGY_RESET_LEN_MASK) >> STAR_ENERGY_RESET_LEN_SHIFT
    }
    fn local_generated_len(&self) -> u8 {
        (self.0.turn_and_generated_lens & LOCAL_GENERATED_LEN_MASK) >> LOCAL_GENERATED_LEN_SHIFT
    }
    pub(crate) fn before_side_turn_end_order(&self) -> &[BeforeSideTurnEndToken] {
        &self.0.cold.before_side_turn_end
    }

    pub(crate) fn after_side_turn_end_power_order(&self) -> &[AfterSideTurnEndPowerEntry] {
        self.0.after_side_turn_end_inline.as_ref().map_or_else(
            || self.0.cold.after_side_turn_end.as_slice(),
            std::slice::from_ref,
        )
    }

    pub(crate) fn synchronous_damage_is_active(&self) -> bool {
        self.0.cold.damage_execution.is_some()
    }

    pub(crate) fn damage_batch_is_active(&self) -> bool {
        self.0
            .cold
            .damage_execution
            .as_ref()
            .is_some_and(|execution| !execution.batches.is_empty())
    }

    pub(crate) fn monster_death_is_pending(&self, uid: u32) -> bool {
        self.0
            .cold
            .damage_execution
            .as_ref()
            .is_some_and(|execution| execution.batches.iter().any(|batch| batch.contains(&uid)))
    }

    pub(crate) fn enter_damage_batch(&mut self) {
        Arc::make_mut(
            Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold)
                .damage_execution
                .get_or_insert_with(Default::default),
        )
        .batches
        .push(Vec::new());
    }

    pub(crate) fn register_batch_death(&mut self, uid: u32) {
        let cold = Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold);
        Arc::make_mut(
            cold.damage_execution
                .as_mut()
                .expect("entered Damage batch"),
        )
        .batches
        .last_mut()
        .expect("entered Damage batch")
        .push(uid);
    }

    pub(crate) fn monster_death_cleanup_is_active(&self, uid: u32) -> bool {
        self.0
            .cold
            .damage_execution
            .as_ref()
            .is_some_and(|execution| execution.cleanups.contains(&uid))
    }

    pub(crate) fn cancel_pending_batch_death(&mut self, uid: u32) {
        if !self.monster_death_is_pending(uid) {
            return;
        }
        let cold = Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold);
        let execution = Arc::make_mut(cold.damage_execution.as_mut().expect("pending death"));
        for batch in &mut execution.batches {
            batch.retain(|pending| *pending != uid);
        }
    }

    pub(crate) fn begin_batch_death_cleanup(&mut self, uid: u32) {
        if !self.synchronous_damage_is_active() {
            return;
        }
        let cold = Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold);
        let execution = Arc::make_mut(cold.damage_execution.as_mut().expect("active execution"));
        for batch in &mut execution.batches {
            batch.retain(|pending| *pending != uid);
        }
        execution.cleanups.push(uid);
    }

    pub(crate) fn finish_batch_death_cleanup(&mut self, uid: u32) -> bool {
        let cold = Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold);
        let valid = cold
            .damage_execution
            .as_mut()
            .and_then(|execution| Arc::make_mut(execution).cleanups.pop())
            == Some(uid);
        Self::clear_finished_damage_execution(cold);
        valid
    }

    pub(crate) fn leave_damage_batch(&mut self) -> bool {
        let cold = Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold);
        let valid = cold
            .damage_execution
            .as_mut()
            .and_then(|execution| Arc::make_mut(execution).batches.pop())
            .is_some_and(|pending| pending.is_empty());
        Self::clear_finished_damage_execution(cold);
        valid
    }

    fn clear_finished_damage_execution(cold: &mut ColdFanoutState) {
        if cold.damage_execution.as_ref().is_some_and(|execution| {
            execution.batches.is_empty()
                && execution.panache.is_empty()
                && execution.cleanups.is_empty()
        }) {
            cold.damage_execution = None;
        }
    }

    pub(crate) fn player_hooks_deactivated(&self) -> bool {
        self.0.cold.player_hooks_deactivated
    }

    pub(crate) fn set_player_hooks_deactivated(&mut self, value: bool) {
        if self.player_hooks_deactivated() != value {
            Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold).player_hooks_deactivated = value;
        }
    }

    /// `SurroundedPower.Facing` for the live player, or `-1` when no
    /// `SurroundedPower` instance exists (`get_Facing` `0xa8bac`).
    pub(crate) fn kaiser_facing(&self) -> i8 {
        self.0.cold.kaiser_facing_plus_one as i8 - 1
    }

    /// Write `SurroundedPower.Facing`. Returns false for a value outside the
    /// two-member `SurroundedPower/Direction` enum and its absent sentinel,
    /// so the boundary can refuse an unrepresentable document by name.
    /// `RunState.AscensionLevel` of the fight (#2539). Constant for the fight;
    /// every `AscensionTier` monster constant is selected by it.
    pub fn ascension(&self) -> u8 {
        crate::encounters::MODELED_ASCENSION - self.0.cold.ascension_below_modeled
    }

    /// Write the fight's ascension. Returns false for a level outside
    /// `0..=MAX_ASCENSION`, so the boundary can refuse it by name.
    pub(crate) fn set_ascension(&mut self, value: u8) -> bool {
        if value > crate::encounters::MAX_ASCENSION {
            return false;
        }
        if self.ascension() != value {
            Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold).ascension_below_modeled =
                crate::encounters::MODELED_ASCENSION - value;
        }
        true
    }

    pub(crate) fn set_kaiser_facing(&mut self, value: i8) -> bool {
        if !(-1..=1).contains(&value) {
            return false;
        }
        if self.kaiser_facing() != value {
            Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold).kaiser_facing_plus_one =
                (value + 1) as u8;
        }
        true
    }

    pub(crate) fn after_card_played_power_order(&self) -> &[AfterCardPlayedPowerEntry] {
        &self.0.cold.after_card_played
    }

    /// Authoritative fixed-width potion slots. An empty slice is the
    /// canonical legacy/default state with no authenticated belt topology.
    pub(crate) fn potion_slots(&self) -> &[Option<PotionId>] {
        &self.0.cold.potion_belt.slots
    }

    pub(crate) fn potion_sozu(&self) -> bool {
        self.0.cold.potion_belt.flags & POTION_BELT_SOZU_MASK != 0
    }

    fn batch_eight(&self) -> &BatchEightRelicState {
        &self.0.cold.batch_eight_relics
    }

    fn mutate_batch_eight(&mut self) -> &mut BatchEightRelicState {
        let fanout = Arc::make_mut(&mut self.0);
        &mut Arc::make_mut(&mut fanout.cold).batch_eight_relics
    }

    pub(crate) fn beating_remnant_damage_received(&self) -> i32 {
        self.batch_eight().beating_remnant_damage_received
    }

    pub(crate) fn beating_remnant_owned(&self) -> bool {
        self.batch_eight().beating_remnant_owned
    }

    pub(crate) fn brilliant_scarf_owned(&self) -> bool {
        self.batch_eight().brilliant_scarf_owned
    }

    pub(crate) fn ectoplasm_owned(&self) -> bool {
        self.batch_eight().ectoplasm_owned
    }

    pub(crate) fn lizard_tail_owned(&self) -> bool {
        self.batch_eight().lizard_tail_owned
    }

    pub(crate) fn ruined_helmet_owned(&self) -> bool {
        self.batch_eight().ruined_helmet_owned
    }

    pub(crate) fn undying_sigil_owned(&self) -> bool {
        self.batch_eight().undying_sigil_owned
    }

    /// Hydrates the immutable Undying Sigil ownership cache. An unchanged
    /// value never forces a copy-on-write of the shared cold fanout.
    pub(crate) fn set_undying_sigil_owned(&mut self, value: bool) {
        if self.undying_sigil_owned() != value {
            self.mutate_batch_eight().undying_sigil_owned = value;
        }
    }

    pub(crate) fn set_batch_eight_deep_relic_ownership(
        &mut self,
        beating_remnant: bool,
        brilliant_scarf: bool,
        ectoplasm: bool,
        lizard_tail: bool,
        ruined_helmet: bool,
    ) {
        let state = self.mutate_batch_eight();
        state.beating_remnant_owned = beating_remnant;
        state.brilliant_scarf_owned = brilliant_scarf;
        state.ectoplasm_owned = ectoplasm;
        state.lizard_tail_owned = lizard_tail;
        state.ruined_helmet_owned = ruined_helmet;
    }

    pub(crate) fn set_beating_remnant_damage_received(&mut self, value: i32) -> bool {
        if !(0..=20).contains(&value) {
            return false;
        }
        self.mutate_batch_eight().beating_remnant_damage_received = value;
        true
    }

    pub(crate) fn bone_tea_combats_left(&self) -> u8 {
        self.batch_eight().bone_tea_combats_left
    }

    pub(crate) fn set_bone_tea_combats_left(&mut self, value: u8) -> bool {
        if value > 1 {
            return false;
        }
        self.mutate_batch_eight().bone_tea_combats_left = value;
        true
    }

    pub(crate) fn burning_sticks_used(&self) -> bool {
        self.batch_eight().burning_sticks_used
    }

    pub(crate) fn set_burning_sticks_used(&mut self, value: bool) {
        self.mutate_batch_eight().burning_sticks_used = value;
    }

    pub(crate) fn galactic_dust(&self) -> u8 {
        self.batch_eight().galactic_dust
    }

    pub(crate) fn set_galactic_dust(&mut self, value: u8) -> bool {
        if value > 9 {
            return false;
        }
        self.mutate_batch_eight().galactic_dust = value;
        true
    }

    pub(crate) fn iron_club_cards(&self) -> u8 {
        self.batch_eight().iron_club_cards
    }

    pub(crate) fn set_iron_club_cards(&mut self, value: u8) -> bool {
        if value > 3 {
            return false;
        }
        self.mutate_batch_eight().iron_club_cards = value;
        true
    }

    pub(crate) fn kusarigama(&self) -> u8 {
        self.batch_eight().kusarigama
    }

    pub(crate) fn set_kusarigama(&mut self, value: u8) -> bool {
        if value > 2 {
            return false;
        }
        self.mutate_batch_eight().kusarigama = value;
        true
    }

    pub(crate) fn lizard_tail_used(&self) -> bool {
        self.batch_eight().lizard_tail_used
    }

    pub(crate) fn set_lizard_tail_used(&mut self, value: bool) {
        self.mutate_batch_eight().lizard_tail_used = value;
    }

    pub(crate) fn mini_regent_used(&self) -> bool {
        self.batch_eight().mini_regent_used
    }

    pub(crate) fn set_mini_regent_used(&mut self, value: bool) {
        self.mutate_batch_eight().mini_regent_used = value;
    }

    pub(crate) fn pen_nib(&self) -> u8 {
        self.batch_eight().pen_nib
    }

    pub(crate) fn set_pen_nib(&mut self, value: u8) -> bool {
        if value > 9 {
            return false;
        }
        self.mutate_batch_eight().pen_nib = value;
        true
    }

    pub(crate) fn throwing_axe_available(&self) -> bool {
        self.batch_eight().throwing_axe_available
    }

    pub(crate) fn set_throwing_axe_available(&mut self, value: bool) {
        self.mutate_batch_eight().throwing_axe_available = value;
    }

    pub(crate) fn tuning_fork_skills(&self) -> i32 {
        self.batch_eight().tuning_fork_skills
    }

    pub(crate) fn set_tuning_fork_skills(&mut self, value: i32) -> bool {
        if value < 0 {
            return false;
        }
        self.mutate_batch_eight().tuning_fork_skills = value;
        true
    }

    pub(crate) fn vambrace_available(&self) -> bool {
        self.batch_eight().vambrace_available
    }

    pub(crate) fn set_vambrace_available(&mut self, value: bool) {
        self.mutate_batch_eight().vambrace_available = value;
    }

    pub(crate) fn vambrace_trigger_uid(&self) -> Option<u32> {
        self.batch_eight().vambrace_trigger_uid
    }

    pub(crate) fn set_vambrace_trigger_uid(&mut self, value: Option<u32>) {
        self.mutate_batch_eight().vambrace_trigger_uid = value;
    }

    fn batch_nine(&self) -> &BatchNineRelicState {
        &self.0.cold.batch_nine_relics
    }

    fn mutate_batch_nine(&mut self) -> &mut BatchNineRelicState {
        let fanout = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut fanout.cold);
        Arc::make_mut(&mut cold.batch_nine_relics)
    }

    pub(crate) fn history_course_attack_current_turn(&self) -> Option<&FrozenAutoBatchEntry> {
        self.batch_nine()
            .history_course_attack_current_turn
            .as_ref()
    }

    pub(crate) fn set_history_course_attack_current_turn(
        &mut self,
        value: Option<FrozenAutoBatchEntry>,
    ) {
        self.mutate_batch_nine().history_course_attack_current_turn = value;
    }

    pub(crate) fn history_course_attack_previous_turn(&self) -> Option<&FrozenAutoBatchEntry> {
        self.batch_nine()
            .history_course_attack_previous_turn
            .as_ref()
    }

    pub(crate) fn set_history_course_attack_previous_turn(
        &mut self,
        value: Option<FrozenAutoBatchEntry>,
    ) {
        self.mutate_batch_nine().history_course_attack_previous_turn = value;
    }

    pub(crate) fn roll_history_course_attack_turn(&mut self) {
        let state = self.mutate_batch_nine();
        state.history_course_attack_previous_turn = state.history_course_attack_current_turn.take();
    }

    pub(crate) fn puzzle_armed(&self) -> bool {
        self.batch_nine().puzzle_armed
    }

    pub(crate) fn set_puzzle_armed(&mut self, value: bool) {
        self.mutate_batch_nine().puzzle_armed = value;
    }

    pub(crate) fn replay_tape_cursor(&self) -> u16 {
        self.batch_nine().replay_tape_cursor
    }

    pub(crate) fn set_replay_tape_cursor(&mut self, value: u16) {
        if self.replay_tape_cursor() != value {
            self.mutate_batch_nine().replay_tape_cursor = value;
        }
    }

    pub(crate) fn music_box_used_this_turn(&self) -> bool {
        self.batch_nine().music_box_used_this_turn
    }

    pub(crate) fn set_music_box_used_this_turn(&mut self, value: bool) {
        self.mutate_batch_nine().music_box_used_this_turn = value;
    }

    pub(crate) fn music_box_card_uid(&self) -> Option<u32> {
        self.batch_nine().music_box_card_uid
    }

    pub(crate) fn set_music_box_card_uid(&mut self, value: Option<u32>) {
        self.mutate_batch_nine().music_box_card_uid = value;
    }

    pub(crate) fn joss_paper_cards_exhausted(&self) -> i32 {
        self.batch_nine().joss_paper_cards_exhausted
    }

    pub(crate) fn set_joss_paper_cards_exhausted(&mut self, value: i32) -> bool {
        if value < -1 {
            return false;
        }
        self.mutate_batch_nine().joss_paper_cards_exhausted = value;
        true
    }

    pub(crate) fn joss_paper_ethereal_count(&self) -> i32 {
        self.batch_nine().joss_paper_ethereal_count
    }

    pub(crate) fn set_joss_paper_ethereal_count(&mut self, value: i32) -> bool {
        if value < 0 {
            return false;
        }
        self.mutate_batch_nine().joss_paper_ethereal_count = value;
        true
    }

    pub(crate) fn pumpkin_candle_kindle_count(&self) -> i32 {
        self.batch_nine().pumpkin_candle_kindle_count
    }

    pub(crate) fn set_pumpkin_candle_kindle_count(&mut self, value: i32) -> bool {
        if value < -1 {
            return false;
        }
        self.mutate_batch_nine().pumpkin_candle_kindle_count = value;
        true
    }

    pub(crate) fn paels_legion_cooldown(&self) -> i8 {
        self.batch_nine().paels_legion_cooldown
    }

    pub(crate) fn set_paels_legion_cooldown(&mut self, value: i8) -> bool {
        if !(-1..=2).contains(&value) {
            return false;
        }
        self.mutate_batch_nine().paels_legion_cooldown = value;
        true
    }

    pub(crate) fn paels_legion_trigger_uid(&self) -> Option<u32> {
        self.batch_nine().paels_legion_trigger_uid
    }

    pub(crate) fn set_paels_legion_trigger_uid(&mut self, value: Option<u32>) {
        self.mutate_batch_nine().paels_legion_trigger_uid = value;
    }

    pub(crate) fn paels_legion_triggered_last_turn(&self) -> bool {
        self.batch_nine().paels_legion_triggered_last_turn
    }

    pub(crate) fn set_paels_legion_triggered_last_turn(&mut self, value: bool) {
        self.mutate_batch_nine().paels_legion_triggered_last_turn = value;
    }

    pub(crate) fn paels_eye_used(&self) -> bool {
        self.batch_nine().paels_eye_used
    }

    pub(crate) fn set_paels_eye_used(&mut self, value: bool) {
        self.mutate_batch_nine().paels_eye_used = value;
    }

    pub(crate) fn paels_eye_was_owner_part_last_player_turn(&self) -> bool {
        self.batch_nine().paels_eye_was_owner_part_last_player_turn
    }

    pub(crate) fn set_paels_eye_was_owner_part_last_player_turn(&mut self, value: bool) {
        self.mutate_batch_nine()
            .paels_eye_was_owner_part_last_player_turn = value;
    }

    /// Whether native `CombatTurnState.PlayersTakingExtraTurn` is non-empty:
    /// the list is cleared and refilled at every
    /// `CombatManager/<SwitchFromPlayerToEnemySide>d__141::MoveNext`
    /// (RVA `0x3f87dc`, `Clear` IL_0059-005e, `Add` IL_00fd-010a behind
    /// `Hook::ShouldTakeExtraTurn` IL_0095), so it holds for exactly the
    /// player turn a Pael's Eye extra turn plays (#3049).
    pub(crate) fn players_taking_extra_turn(&self) -> bool {
        self.batch_nine().players_taking_extra_turn
    }

    /// Writes only on change: every player side end reaches this, and an
    /// unconditional write would copy the shared cold bundle each turn.
    pub(crate) fn set_players_taking_extra_turn(&mut self, value: bool) {
        if self.players_taking_extra_turn() != value {
            self.mutate_batch_nine().players_taking_extra_turn = value;
        }
    }

    pub(crate) fn regalite_used_this_turn(&self) -> bool {
        self.batch_nine().regalite_used_this_turn
    }

    pub(crate) fn set_regalite_used_this_turn(&mut self, value: bool) {
        self.mutate_batch_nine().regalite_used_this_turn = value;
    }

    pub(crate) fn self_forming_clay_owned(&self) -> bool {
        self.batch_nine().self_forming_clay_owned
    }

    pub(crate) fn set_self_forming_clay_owned(&mut self, value: bool) {
        self.mutate_batch_nine().self_forming_clay_owned = value;
    }

    pub(crate) fn gremlin_horn_owned(&self) -> bool {
        self.batch_nine().gremlin_horn_owned
    }

    pub(crate) fn set_gremlin_horn_owned(&mut self, value: bool) {
        self.mutate_batch_nine().gremlin_horn_owned = value;
    }

    /// Whether a deferred hook action is queued (#3387).
    pub(crate) fn deferred_hook_action_is_queued(&self) -> bool {
        self.batch_nine().deferred_hook_action.is_some()
    }

    fn take_deferred_hook_action(&mut self) -> Option<Arc<DeferredHookAction>> {
        if !self.deferred_hook_action_is_queued() {
            return None;
        }
        self.mutate_batch_nine().deferred_hook_action.take()
    }

    #[cfg(test)]
    pub(crate) fn take_deferred_hook_action_for_test(&mut self) {
        self.take_deferred_hook_action();
    }

    fn queue_deferred_hook_action(&mut self, action: DeferredHookAction) {
        self.mutate_batch_nine().deferred_hook_action = Some(Arc::new(action));
    }

    pub(crate) fn spiked_gauntlets_owned(&self) -> bool {
        self.batch_nine().spiked_gauntlets_owned
    }

    /// Hydrates the immutable Spiked Gauntlets ownership cache (#3437). An
    /// unchanged value never forces a copy-on-write of the shared cold fanout.
    pub(crate) fn set_spiked_gauntlets_owned(&mut self, value: bool) {
        if self.spiked_gauntlets_owned() != value {
            self.mutate_batch_nine().spiked_gauntlets_owned = value;
        }
    }

    pub(crate) fn red_skull_owned(&self) -> bool {
        self.batch_nine().red_skull_owned
    }

    /// Hydrates the immutable Red Skull ownership cache. An unchanged value
    /// never forces a copy-on-write of the shared cold fanout.
    pub(crate) fn set_red_skull_owned(&mut self, value: bool) {
        if self.red_skull_owned() != value {
            self.mutate_batch_nine().red_skull_owned = value;
        }
    }

    pub(crate) fn red_skull_latch_stale(&self) -> bool {
        self.batch_nine().red_skull_latch_stale
    }

    pub(crate) fn set_red_skull_latch_stale(&mut self, value: bool) {
        if self.red_skull_latch_stale() != value {
            self.mutate_batch_nine().red_skull_latch_stale = value;
        }
    }

    pub(crate) fn batch_nine_relic_pending(&self) -> Option<&RelicPendingRecord> {
        self.batch_nine().pending.as_ref()
    }

    pub(crate) fn set_batch_nine_relic_pending(&mut self, value: Option<RelicPendingRecord>) {
        self.mutate_batch_nine().pending = value;
    }

    pub(crate) fn potion_belt_buckle(&self) -> bool {
        self.0.cold.potion_belt.flags & POTION_BELT_BUCKLE_MASK != 0
    }

    pub(crate) fn potion_belt_buckle_applied(&self) -> bool {
        self.0.cold.potion_belt.flags & POTION_BELT_BUCKLE_APPLIED_MASK != 0
    }

    pub(crate) fn strict_potions(&self) -> bool {
        self.0.cold.potion_belt.flags & POTION_BELT_STRICT_MASK != 0
    }

    pub(crate) fn fully_unlocked_potion_pool(&self) -> bool {
        self.0.cold.potion_belt.flags & POTION_BELT_POOL_PROOF_MASK != 0
    }

    pub(crate) fn unceasing_top(&self) -> bool {
        self.0.cold.potion_belt.flags & POTION_BELT_UNCEASING_TOP_MASK != 0
    }

    pub(crate) fn set_unceasing_top(&mut self, value: bool) {
        let fanout = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut fanout.cold);
        let belt = Arc::make_mut(&mut cold.potion_belt);
        if value {
            belt.flags |= POTION_BELT_UNCEASING_TOP_MASK;
        } else {
            belt.flags &= !POTION_BELT_UNCEASING_TOP_MASK;
        }
    }

    /// Install a complete belt atomically after boundary validation.
    pub(crate) fn set_potion_belt(
        &mut self,
        slots: Vec<Option<PotionId>>,
        sozu: bool,
        belt_buckle: bool,
        belt_buckle_applied: bool,
        strict: bool,
        fully_unlocked_pool: bool,
    ) -> bool {
        if slots.len() > usize::from(u8::MAX) + 1
            || (slots.is_empty()
                && (belt_buckle || belt_buckle_applied || strict || fully_unlocked_pool))
            || belt_buckle_applied && (!belt_buckle || slots.iter().any(Option::is_some))
        {
            return false;
        }
        let mut flags = 0;
        for (live, mask) in [
            (sozu, POTION_BELT_SOZU_MASK),
            (belt_buckle, POTION_BELT_BUCKLE_MASK),
            (belt_buckle_applied, POTION_BELT_BUCKLE_APPLIED_MASK),
            (strict, POTION_BELT_STRICT_MASK),
            (fully_unlocked_pool, POTION_BELT_POOL_PROOF_MASK),
        ] {
            if live {
                flags |= mask;
            }
        }
        let fanout = Arc::make_mut(&mut self.0);
        fanout.cold = Arc::new({
            let mut cold = (*fanout.cold).clone();
            cold.potion_belt = Arc::new(PotionBeltState {
                slots,
                flags,
                ..PotionBeltState::default()
            });
            cold
        });
        true
    }

    /// Every belt coherence check except the pool proof (#3347): capacity,
    /// flag bits, Belt Buckle latch, and the scalar counters. Which proof of
    /// the generation pool a materialised belt needs is decided with the
    /// catalog's recorded profile in hand, by
    /// [`crate::engine::potions::potion_belt_state_is_exact`].
    pub(crate) fn potion_belt_topology_is_exact(&self) -> bool {
        let belt = &self.0.cold.potion_belt;
        belt.flags & !POTION_BELT_FLAGS_MASK == 0
            && belt.slots.len() <= usize::from(u8::MAX) + 1
            // Python elides the entire empty belt topology, but still
            // projects Sozu because its reward-generation veto is meaningful
            // without a held slot. Every other slot-dependent flag continues
            // to require an authenticated nonempty topology.
            && (!belt.slots.is_empty()
                || belt.flags & (POTION_BELT_SLOT_FLAGS_MASK & !POTION_BELT_SOZU_MASK) == 0)
            && (!self.potion_belt_buckle_applied()
                || self.potion_belt_buckle() && belt.slots.iter().all(Option::is_none))
            && belt.duplication >= 0
            && belt.clarity >= 0
            && belt.player_ritual >= 0
            && belt.gigantification >= 0
            && belt.radiance >= 0
            && belt.regen >= 0
            && belt.after_damage_received_order_code <= 6
            && (!belt.gigantification_bound || belt.gigantification > 0)
            && belt.reserved_after_side_turn_start == 0
    }

    fn potion_belt_mut(&mut self) -> &mut PotionBeltState {
        let fanout = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut fanout.cold);
        Arc::make_mut(&mut cold.potion_belt)
    }

    pub(crate) fn remove_potion_at(&mut self, slot: usize) -> Option<PotionId> {
        self.potion_belt_mut().slots.get_mut(slot)?.take()
    }

    pub(crate) fn insert_potion_first_empty(&mut self, potion: PotionId) -> bool {
        let belt = self.potion_belt_mut();
        let Some(slot) = belt.slots.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        *slot = Some(potion);
        true
    }

    pub(crate) fn set_potion_belt_buckle_applied(&mut self, applied: bool) {
        let belt = self.potion_belt_mut();
        if applied {
            belt.flags |= POTION_BELT_BUCKLE_APPLIED_MASK;
        } else {
            belt.flags &= !POTION_BELT_BUCKLE_APPLIED_MASK;
        }
    }

    pub(crate) fn duplication(&self) -> i32 {
        self.0.cold.potion_belt.duplication
    }
    pub(crate) fn clarity(&self) -> i32 {
        self.0.cold.potion_belt.clarity
    }
    pub(crate) fn player_ritual(&self) -> i32 {
        self.0.cold.potion_belt.player_ritual
    }
    pub(crate) fn gigantification(&self) -> i32 {
        self.0.cold.potion_belt.gigantification
    }
    pub(crate) fn radiance(&self) -> i32 {
        self.0.cold.potion_belt.radiance
    }
    pub(crate) fn regen(&self) -> i32 {
        self.0.cold.potion_belt.regen
    }
    pub(crate) fn gigantification_bound(&self) -> bool {
        self.0.cold.potion_belt.gigantification_bound
    }

    pub(crate) fn set_duplication(&mut self, value: i32) -> bool {
        if value < 0 {
            return false;
        }
        self.potion_belt_mut().duplication = value;
        true
    }
    pub(crate) fn set_clarity(&mut self, value: i32) -> bool {
        if value < 0 {
            return false;
        }
        let old = self.clarity();
        if old == 0
            && value > 0
            && !self.register_after_side_turn_start_token(AfterSideTurnStartToken::Clarity)
        {
            return false;
        }
        if old > 0 && value == 0 {
            self.unregister_after_side_turn_start_token(AfterSideTurnStartToken::Clarity);
        }
        self.potion_belt_mut().clarity = value;
        true
    }
    fn after_damage_received_order_code(&self) -> u8 {
        self.0.cold.potion_belt.after_damage_received_order_code
    }
    fn set_after_damage_received_order_code(&mut self, code: u8) {
        self.potion_belt_mut().after_damage_received_order_code = code;
    }
    pub(crate) fn set_player_ritual(&mut self, value: i32) -> bool {
        if value < 0 {
            return false;
        }
        self.potion_belt_mut().player_ritual = value;
        true
    }
    pub(crate) fn no_energy_gain(&self) -> bool {
        self.0.cold.potion_belt.flags & POTION_BELT_NO_ENERGY_GAIN_MASK != 0
    }
    pub(crate) fn set_no_energy_gain(&mut self, value: bool) {
        let belt = self.potion_belt_mut();
        if value {
            belt.flags |= POTION_BELT_NO_ENERGY_GAIN_MASK;
        } else {
            belt.flags &= !POTION_BELT_NO_ENERGY_GAIN_MASK;
        }
    }
    pub(crate) fn set_gigantification(&mut self, value: i32) -> bool {
        let belt = self.potion_belt_mut();
        if value < 0 || value == 0 && belt.gigantification_bound {
            return false;
        }
        belt.gigantification = value;
        true
    }
    pub(crate) fn set_gigantification_bound(&mut self, value: bool) -> bool {
        let belt = self.potion_belt_mut();
        if value && belt.gigantification == 0 {
            return false;
        }
        belt.gigantification_bound = value;
        true
    }
    pub(crate) fn set_radiance(&mut self, value: i32) -> bool {
        if value < 0 {
            return false;
        }
        self.potion_belt_mut().radiance = value;
        true
    }
    pub(crate) fn set_regen(&mut self, value: i32) -> bool {
        if value < 0 {
            return false;
        }
        self.potion_belt_mut().regen = value;
        true
    }

    #[cfg(test)]
    pub(crate) fn potion_belt_shares_store_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0.cold.potion_belt, &other.0.cold.potion_belt)
    }

    #[cfg(test)]
    pub(crate) fn forge_potion_belt_flags_for_test(&mut self, flags: u8) {
        let fanout = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut fanout.cold);
        Arc::make_mut(&mut cold.potion_belt).flags = flags;
    }
    /// Aeonglass Withering Presence's exact owner-play countdown.
    pub(crate) fn withering_cards_left(&self) -> u16 {
        self.0.withering_cards_left[0]
    }

    /// Replace the bounded Withering countdown without widening FanoutState.
    pub(crate) fn set_withering_cards_left(&mut self, value: u16) -> bool {
        if value > WITHERING_CARDS_LEFT_MAX {
            return false;
        }
        Arc::make_mut(&mut self.0).withering_cards_left[0] = value;
        true
    }

    pub(crate) fn withering_cards_left_is_exact(&self) -> bool {
        self.withering_cards_left() <= WITHERING_CARDS_LEFT_MAX
    }
    pub(crate) fn the_bomb_instances(&self) -> impl Iterator<Item = TheBombInstance> + '_ {
        self.0
            .cold
            .power_records
            .iter()
            .filter_map(|record| match record {
                ColdPowerRecord::TheBomb(instance) => Some(*instance),
                ColdPowerRecord::ToricToughness(_)
                | ColdPowerRecord::Sloth(_)
                | ColdPowerRecord::Panache(_)
                | ColdPowerRecord::Monologue(_)
                | ColdPowerRecord::Automation(_) => None,
            })
    }
    pub(crate) fn panache_instances(&self) -> impl Iterator<Item = PanacheInstance> + '_ {
        self.0
            .cold
            .power_records
            .iter()
            .filter_map(|record| match record {
                ColdPowerRecord::Panache(instance) => Some(*instance),
                ColdPowerRecord::TheBomb(_)
                | ColdPowerRecord::ToricToughness(_)
                | ColdPowerRecord::Sloth(_)
                | ColdPowerRecord::Monologue(_)
                | ColdPowerRecord::Automation(_) => None,
            })
    }
    pub(crate) fn monologue_instances(&self) -> impl Iterator<Item = MonologueInstance> + '_ {
        self.0
            .cold
            .power_records
            .iter()
            .filter_map(|record| match record {
                ColdPowerRecord::Monologue(instance) => Some(*instance),
                ColdPowerRecord::TheBomb(_)
                | ColdPowerRecord::ToricToughness(_)
                | ColdPowerRecord::Sloth(_)
                | ColdPowerRecord::Panache(_)
                | ColdPowerRecord::Automation(_) => None,
            })
    }
    pub(crate) fn toric_toughness(&self) -> Option<ToricToughnessInstance> {
        self.0
            .cold
            .power_records
            .iter()
            .find_map(|record| match record {
                ColdPowerRecord::TheBomb(_) => None,
                ColdPowerRecord::ToricToughness(instance) => Some(*instance),
                ColdPowerRecord::Sloth(_)
                | ColdPowerRecord::Panache(_)
                | ColdPowerRecord::Monologue(_)
                | ColdPowerRecord::Automation(_) => None,
            })
    }
    pub(crate) fn sloth(&self) -> Option<SlothInstance> {
        self.0
            .cold
            .power_records
            .iter()
            .find_map(|record| match record {
                ColdPowerRecord::Sloth(instance) => Some(*instance),
                ColdPowerRecord::TheBomb(_)
                | ColdPowerRecord::ToricToughness(_)
                | ColdPowerRecord::Panache(_)
                | ColdPowerRecord::Monologue(_)
                | ColdPowerRecord::Automation(_) => None,
            })
    }
    pub(crate) fn the_bomb_instances_are_empty(&self) -> bool {
        self.the_bomb_instances().next().is_none()
    }
    pub(crate) fn the_bomb_instance(&self, uid: u32) -> Option<TheBombInstance> {
        self.the_bomb_instances()
            .find(|instance| instance.uid == uid)
    }
    pub(crate) fn next_the_bomb_uid(&self) -> u32 {
        self.0.cold.next_the_bomb_uid
    }
    /// See `BatchNineRelicState::relic_pet_creatures` (#3039).
    pub(crate) fn relic_pet_creatures(&self) -> u8 {
        self.batch_nine().relic_pet_creatures
    }

    pub(crate) fn set_relic_pet_creatures(&mut self, count: u8) {
        if self.batch_nine().relic_pet_creatures == count {
            return;
        }
        self.mutate_batch_nine().relic_pet_creatures = count;
    }

    /// See `BatchNineRelicState::next_creature_uid` (#3039); zero is untracked.
    pub(crate) fn next_creature_uid(&self) -> u32 {
        self.batch_nine().next_creature_uid
    }
    pub(crate) fn next_after_side_turn_end_power_uid(&self) -> u32 {
        self.0.next_after_side_turn_end_power_uid
    }
    pub(crate) fn star_energy_reset_order(&self) -> &[PowerId] {
        &self.0.star_energy_reset[..usize::from(self.star_energy_reset_len())]
    }
    /// The complete native application order, when provenance established it.
    /// `None` is a legacy checkpoint whose cross-family order was absent.
    pub(crate) fn after_energy_reset_order(&self) -> Option<&[AfterEnergyResetPower]> {
        self.0
            .cold
            .after_energy_reset_order
            .as_ref()
            .map(AfterEnergyResetOrder::as_slice)
    }
    pub(crate) fn after_energy_reset_order_is_explicit(&self) -> bool {
        self.0
            .cold
            .after_energy_reset_order
            .as_ref()
            .is_some_and(|order| order.explicit)
    }
    pub(crate) fn local_generated_power_order(&self) -> &[PowerId] {
        &self.0.local_generated[..usize::from(self.local_generated_len())]
    }
    /// Validate the two compact cold-metadata carriers without decoding an
    /// impossible two-key order. Boundary/admission call this before any
    /// getter that assumes the private writers' closed 0..=4 vocabulary.
    pub(crate) fn cold_packed_metadata_is_exact(&self) -> bool {
        let lengths = self.0.turn_and_generated_lens;
        let star_reset = (lengths & STAR_ENERGY_RESET_LEN_MASK) >> STAR_ENERGY_RESET_LEN_SHIFT;
        let local_generated = (lengths & LOCAL_GENERATED_LEN_MASK) >> LOCAL_GENERATED_LEN_SHIFT;
        if usize::from(result_location_len(lengths)) > MAX_RESULT_LOCATION_POWERS
            || usize::from(star_reset) > MAX_STAR_ENERGY_RESET_POWERS
            || usize::from(local_generated) > MAX_LOCAL_GENERATED_POWERS
            || !self.after_player_turn_start_order_is_exact()
            || !self.the_bomb_state_is_exact()
            || !self.toric_toughness_state_is_exact()
            || self
                .after_block_cleared_order()
                .iter()
                .filter(|power| **power == PowerId::ToricToughness)
                .count()
                != usize::from(self.toric_toughness().is_some())
        {
            return false;
        }

        if self.0.after_side_turn_end_inline.is_some()
            && !self.0.cold.after_side_turn_end.is_empty()
            || self.0.after_side_turn_end_inline.is_none()
                && self.0.cold.after_side_turn_end.len() == 1
        {
            return false;
        }

        let packed = self.0.intercept_covered_state;
        let intercept_mask = packed & INTERCEPT_COVERED_MASK;
        let intercept_order =
            (packed & INTERCEPT_COVERED_ORDER_MASK) >> INTERCEPT_COVERED_ORDER_SHIFT;
        let imitation_order =
            (packed & IMITATION_LEARNING_ORDER_MASK) >> IMITATION_LEARNING_ORDER_SHIFT;
        let imitation_mask = self
            .0
            .imitation_learning
            .iter()
            .enumerate()
            .try_fold(0u8, |mask, (key, amount)| {
                (*amount >= 0).then_some(mask | (u8::from(*amount > 0) << key))
            });
        imitation_mask.is_some_and(|mask| {
            two_key_order_mask(intercept_order) == Some(intercept_mask)
                && two_key_order_mask(imitation_order) == Some(mask)
        })
    }
    pub(crate) fn result_location_power_order(&self) -> ResultLocationPowerOrder {
        let len = result_location_len(self.0.turn_and_generated_lens);
        let mut values = [PowerId::Accuracy; MAX_RESULT_LOCATION_POWERS];
        for (output, compact) in values
            .iter_mut()
            .zip(self.0.result_location[..usize::from(len)].iter())
        {
            *output = compact.power();
        }
        ResultLocationPowerOrder { values, len }
    }

    /// MonologuePower's nonnegative native StrengthApplied ledger.
    pub(crate) fn monologue_strength_applied(&self) -> i32 {
        self.0.monologue_strength_applied
    }

    /// Whether all three private Monologue listener projections are the
    /// exact singleton `[Monologue]` acquisition order.
    pub(crate) fn monologue_hooks_are_registered(&self) -> bool {
        self.monologue_hook_flags() == MONOLOGUE_HOOK_MASK
    }

    /// Whether the compact Monologue order carrier is one of its two exact
    /// representations. This distinguishes a forged partial registration
    /// from the inactive and live singleton states.
    pub(crate) fn monologue_hook_flags_are_exact(&self) -> bool {
        matches!(self.monologue_hook_flags(), 0 | MONOLOGUE_HOOK_MASK)
    }

    fn monologue_hook_flags(&self) -> u8 {
        self.0.private_power_flags & MONOLOGUE_HOOK_MASK
    }

    pub(crate) fn set_monologue_strength_applied(&mut self, value: i32) -> bool {
        if value < 0 {
            return false;
        }
        Arc::make_mut(&mut self.0).monologue_strength_applied = value;
        true
    }

    /// Register all three Monologue hooks on the zero-to-positive edge.
    pub(crate) fn register_monologue_hooks(&mut self) -> bool {
        let flags = self.monologue_hook_flags();
        if flags != 0 {
            return flags == MONOLOGUE_HOOK_MASK;
        }
        Arc::make_mut(&mut self.0).private_power_flags |= MONOLOGUE_HOOK_MASK;
        true
    }

    /// Remove all three Monologue hook registrations with the power.
    #[cfg(test)]
    pub(crate) fn unregister_monologue_hooks(&mut self) {
        Arc::make_mut(&mut self.0).private_power_flags &= !MONOLOGUE_HOOK_MASK;
    }

    pub(crate) fn void_form_cards_played_this_turn(&self) -> i32 {
        self.0.void_form_cards_played_this_turn
    }

    pub(crate) fn set_void_form_cards_played_this_turn(&mut self, value: i32) -> bool {
        if value < 0 {
            return false;
        }
        Arc::make_mut(&mut self.0).void_form_cards_played_this_turn = value;
        true
    }

    pub(crate) fn void_form_hooks_are_registered(&self) -> bool {
        self.0.private_power_flags & VOID_FORM_HOOKS_MASK != 0
    }

    pub(crate) fn set_void_form_hooks_registered(&mut self, registered: bool) {
        let flags = &mut Arc::make_mut(&mut self.0).private_power_flags;
        if registered {
            *flags |= VOID_FORM_HOOKS_MASK;
        } else {
            *flags &= !VOID_FORM_HOOKS_MASK;
        }
    }

    pub(crate) fn void_form_end_turn_requested(&self) -> bool {
        self.0.private_power_flags & VOID_FORM_END_TURN_REQUESTED_MASK != 0
    }

    pub(crate) fn set_void_form_end_turn_requested(&mut self, requested: bool) {
        let flags = &mut Arc::make_mut(&mut self.0).private_power_flags;
        if requested {
            *flags |= VOID_FORM_END_TURN_REQUESTED_MASK;
        } else {
            *flags &= !VOID_FORM_END_TURN_REQUESTED_MASK;
        }
    }

    #[cfg(test)]
    pub(crate) fn forge_monologue_hook_flags_for_test(&mut self, flags: u8) {
        let state = Arc::make_mut(&mut self.0);
        state.private_power_flags =
            (state.private_power_flags & !MONOLOGUE_HOOK_MASK) | (flags & MONOLOGUE_HOOK_MASK);
    }

    pub(crate) fn ruined_helmet_used(&self) -> bool {
        self.0.private_power_flags & RUINED_HELMET_USED_MASK != 0
    }

    pub(crate) fn set_ruined_helmet_used(&mut self) {
        Arc::make_mut(&mut self.0).private_power_flags |= RUINED_HELMET_USED_MASK;
    }

    /// Replace one complete canonical listener order after boundary validation.
    pub(crate) fn set_after_card_drawn_order(&mut self, order: &[PowerId]) -> bool {
        set_fanout_order(&mut self.0, order, MAX_AFTER_CARD_DRAWN_POWERS, |state| {
            (&mut state.after_card_drawn, &mut state.after_card_drawn_len)
        })
    }

    /// Replace one complete canonical listener order after boundary validation.
    pub(crate) fn set_after_card_exhausted_order(&mut self, order: &[PowerId]) -> bool {
        set_fanout_order(
            &mut self.0,
            order,
            MAX_AFTER_CARD_EXHAUSTED_POWERS,
            |state| {
                (
                    &mut state.after_card_exhausted,
                    &mut state.after_card_exhausted_len,
                )
            },
        )
    }

    /// Replace one complete canonical listener order after boundary validation.
    pub(crate) fn set_before_hand_draw_order(&mut self, order: &[PowerId]) -> bool {
        set_fanout_order(&mut self.0, order, MAX_BEFORE_HAND_DRAW_POWERS, |state| {
            (&mut state.before_hand_draw, &mut state.before_hand_draw_len)
        })
    }

    pub(crate) fn set_after_damage_given_order(&mut self, order: &[PowerId]) -> bool {
        set_fanout_order(&mut self.0, order, MAX_AFTER_DAMAGE_GIVEN_POWERS, |state| {
            (
                &mut state.after_damage_given,
                &mut state.after_damage_given_len,
            )
        })
    }
    pub(crate) fn set_after_block_gained_order(&mut self, order: &[PowerId]) -> bool {
        set_fanout_order(&mut self.0, order, MAX_AFTER_BLOCK_GAINED_POWERS, |state| {
            (
                &mut state.after_block_gained,
                &mut state.after_block_gained_len,
            )
        })
    }
    pub(crate) fn set_after_block_cleared_order(&mut self, order: &[PowerId]) -> bool {
        set_fanout_order(
            &mut self.0,
            order,
            MAX_AFTER_BLOCK_CLEARED_POWERS,
            |state| {
                (
                    &mut state.after_block_cleared,
                    &mut state.after_block_cleared_len,
                )
            },
        )
    }
    pub(crate) fn set_after_power_amount_changed_order(&mut self, order: &[PowerId]) -> bool {
        if order.len() > MAX_AFTER_POWER_AMOUNT_CHANGED_POWERS {
            return false;
        }
        let mut encoded = [PowerAmountChangedPower::Vicious; MAX_AFTER_POWER_AMOUNT_CHANGED_POWERS];
        for (index, power) in order.iter().copied().enumerate() {
            let Some(power) = PowerAmountChangedPower::from_power(power) else {
                return false;
            };
            encoded[index] = power;
        }
        let state = Arc::make_mut(&mut self.0);
        state.after_power_amount_changed = encoded;
        state.after_power_amount_changed_len = order.len() as u8;
        true
    }
    pub(crate) fn set_after_side_turn_start_order(
        &mut self,
        order: &[AfterSideTurnStartToken],
    ) -> bool {
        if order.len() > MAX_AFTER_SIDE_TURN_START_POWERS {
            return false;
        }
        let state = Arc::make_mut(&mut self.0);
        state
            .after_side_turn_start
            .fill(AfterSideTurnStartToken::BiasedCognition);
        state.after_side_turn_start[..order.len()].copy_from_slice(order);
        let player_len = state.after_side_turn_start_len & AFTER_PLAYER_TURN_START_LEN_MASK;
        state.after_side_turn_start_len = order.len() as u8 | player_len;
        true
    }
    pub(crate) fn set_after_player_turn_start_order(&mut self, order: &[PowerId]) -> bool {
        if order.len() > MAX_STORED_AFTER_PLAYER_TURN_START_POWERS
            || order.iter().enumerate().any(|(index, power)| {
                !matches!(
                    power,
                    PowerId::Loop
                        | PowerId::RollingBoulder
                        | PowerId::SummonNextTurn
                        | PowerId::Inferno
                        | PowerId::CrimsonMantle
                        | PowerId::ToolsOfTheTrade
                        | PowerId::Tyranny
                ) || order[..index].contains(power)
            })
        {
            return false;
        }
        let state = Arc::make_mut(&mut self.0);
        state.after_player_turn_start.fill(PowerId::Accuracy);
        state.after_player_turn_start[..order.len()].copy_from_slice(order);
        let side_len = state.after_side_turn_start_len & AFTER_SIDE_TURN_START_LEN_MASK;
        state.after_side_turn_start_len =
            side_len | ((order.len() as u8) << AFTER_PLAYER_TURN_START_LEN_SHIFT);
        true
    }
    #[cfg(test)]
    pub(crate) fn legacy_turn_start_hand_choice_order_without_entropy(
        &self,
    ) -> AfterPlayerTurnStartOrder {
        let raw = self.stored_after_player_turn_start_order();
        let mut powers = [PowerId::Accuracy; MAX_AFTER_PLAYER_TURN_START_POWERS];
        let mut len = 0;
        for power in raw
            .iter()
            .copied()
            .filter(|power| matches!(power, PowerId::ToolsOfTheTrade | PowerId::Tyranny))
        {
            powers[len] = power;
            len += 1;
        }
        AfterPlayerTurnStartOrder {
            powers,
            len: len as u8,
        }
    }
    #[cfg(test)]
    pub(crate) fn set_turn_start_hand_choice_order(&mut self, order: &[PowerId]) -> bool {
        self.set_after_player_turn_start_order(order)
    }
    pub(crate) fn set_before_side_turn_end_order(
        &mut self,
        order: &[BeforeSideTurnEndToken],
    ) -> bool {
        let state = Arc::make_mut(&mut self.0);
        Arc::make_mut(&mut state.cold).before_side_turn_end = order.to_vec();
        true
    }

    pub(crate) fn set_the_bomb_instances(&mut self, instances: &[TheBombInstance]) {
        let state = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut state.cold);
        let tail = cold
            .power_records
            .iter()
            .copied()
            .filter(|record| !matches!(record, ColdPowerRecord::TheBomb(_)))
            .collect::<Vec<_>>();
        cold.power_records.clear();
        cold.power_records
            .extend(instances.iter().copied().map(ColdPowerRecord::TheBomb));
        cold.power_records.extend(tail);
    }

    pub(crate) fn set_next_the_bomb_uid(&mut self, next_uid: u32) {
        let state = Arc::make_mut(&mut self.0);
        Arc::make_mut(&mut state.cold).next_the_bomb_uid = next_uid;
    }

    pub(crate) fn set_next_creature_uid(&mut self, next_uid: u32) {
        if self.batch_nine().next_creature_uid == next_uid {
            return;
        }
        self.mutate_batch_nine().next_creature_uid = next_uid;
    }

    pub(crate) fn set_next_after_side_turn_end_power_uid(&mut self, next_uid: u32) {
        Arc::make_mut(&mut self.0).next_after_side_turn_end_power_uid = next_uid;
    }

    /// Replace the canonical live object ledger.  This validates only the
    /// ledger's self-contained identity grammar; the boundary performs the
    /// stronger ledger-to-live-object correspondence check after hydrating
    /// every player-power carrier.
    pub(crate) fn set_after_side_turn_end_power_order(
        &mut self,
        order: &[AfterSideTurnEndPowerEntry],
    ) -> bool {
        let next_uid = self.next_after_side_turn_end_power_uid();
        if order.iter().enumerate().any(|(index, entry)| {
            entry.uid >= next_uid
                || index > 0 && order[index - 1].uid >= entry.uid
                || !entry.token.is_instanced()
                    && order[..index]
                        .iter()
                        .any(|other| other.token == entry.token)
        }) {
            return false;
        }
        let state = Arc::make_mut(&mut self.0);
        match order {
            [] => {
                state.after_side_turn_end_inline = None;
                if !state.cold.after_side_turn_end.is_empty() {
                    Arc::make_mut(&mut state.cold).after_side_turn_end.clear();
                }
            }
            [entry] => {
                state.after_side_turn_end_inline = Some(*entry);
                if !state.cold.after_side_turn_end.is_empty() {
                    Arc::make_mut(&mut state.cold).after_side_turn_end.clear();
                }
            }
            _ => {
                state.after_side_turn_end_inline = None;
                Arc::make_mut(&mut state.cold).after_side_turn_end = order.to_vec();
            }
        }
        true
    }

    /// Replace the canonical ordinary AfterCardPlayed object ledger. Stronger
    /// carrier and cross-hook ownership checks run after boundary hydration.
    pub(crate) fn set_after_card_played_power_order(
        &mut self,
        order: &[AfterCardPlayedPowerEntry],
    ) -> bool {
        let next_uid = self.next_after_side_turn_end_power_uid();
        if order.iter().enumerate().any(|(index, entry)| {
            entry.uid >= next_uid
                || index > 0 && order[index - 1].uid >= entry.uid
                || order[..index].iter().any(|other| other.uid == entry.uid)
                || !entry.token.is_instanced()
                    && order[..index]
                        .iter()
                        .any(|other| other.token == entry.token)
        }) {
            return false;
        }
        Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold).after_card_played = order.to_vec();
        true
    }

    /// Register one keyed ordinary AfterCardPlayed object. A supplied uid is
    /// the identity already allocated for this same native object by another
    /// hook view; otherwise this hook owns the first-application allocation.
    pub(crate) fn register_after_card_played_power(
        &mut self,
        token: AfterCardPlayedPowerToken,
        shared_uid: Option<u32>,
    ) -> Result<u32, ()> {
        if matches!(
            token,
            AfterCardPlayedPowerToken::Panache | AfterCardPlayedPowerToken::Monologue
        ) || self
            .after_card_played_power_order()
            .iter()
            .any(|entry| entry.token == token)
        {
            return Err(());
        }
        let uid = if let Some(uid) = shared_uid {
            if uid >= self.next_after_side_turn_end_power_uid()
                || self
                    .after_card_played_power_order()
                    .iter()
                    .any(|entry| entry.uid == uid)
            {
                return Err(());
            }
            uid
        } else {
            if self.next_after_side_turn_end_power_uid() == u32::MAX {
                return Err(());
            }
            let state = Arc::make_mut(&mut self.0);
            let uid = state.next_after_side_turn_end_power_uid;
            state.next_after_side_turn_end_power_uid += 1;
            uid
        };
        Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold)
            .after_card_played
            .push(AfterCardPlayedPowerEntry { token, uid });
        Ok(uid)
    }

    pub(crate) fn unregister_after_card_played_power(
        &mut self,
        token: AfterCardPlayedPowerToken,
        uid: u32,
    ) -> Result<(), ()> {
        let order = self.after_card_played_power_order();
        let index = order
            .iter()
            .position(|entry| entry.token == token && entry.uid == uid)
            .ok_or(())?;
        Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold)
            .after_card_played
            .remove(index);
        Ok(())
    }

    pub(crate) fn after_card_played_power_uid(
        &self,
        token: AfterCardPlayedPowerToken,
    ) -> Option<u32> {
        (!token.is_instanced())
            .then(|| {
                self.after_card_played_power_order()
                    .iter()
                    .find(|entry| entry.token == token)
                    .map(|entry| entry.uid)
            })
            .flatten()
    }

    /// Allocate and append one keyed native object on first application.
    /// Restacks must retain the uid and acquisition ordinal and therefore do
    /// not call this helper.
    pub(crate) fn register_after_side_turn_end_power(
        &mut self,
        token: AfterSideTurnEndPowerToken,
    ) -> Result<u32, ()> {
        if token.is_instanced()
            || self
                .after_side_turn_end_power_order()
                .iter()
                .any(|entry| entry.token == token)
            || self.next_after_side_turn_end_power_uid() == u32::MAX
        {
            return Err(());
        }
        let state = Arc::make_mut(&mut self.0);
        let uid = state.next_after_side_turn_end_power_uid;
        state.next_after_side_turn_end_power_uid += 1;
        let entry = AfterSideTurnEndPowerEntry { token, uid };
        if state.after_side_turn_end_inline.is_none() && state.cold.after_side_turn_end.is_empty() {
            state.after_side_turn_end_inline = Some(entry);
        } else if let Some(first) = state.after_side_turn_end_inline.take() {
            let cold = Arc::make_mut(&mut state.cold);
            debug_assert!(cold.after_side_turn_end.is_empty());
            cold.after_side_turn_end.extend([first, entry]);
        } else {
            Arc::make_mut(&mut state.cold)
                .after_side_turn_end
                .push(entry);
        }
        Ok(uid)
    }

    pub(crate) fn unregister_after_side_turn_end_power(
        &mut self,
        token: AfterSideTurnEndPowerToken,
        uid: u32,
    ) -> Result<(), ()> {
        let order = self.after_side_turn_end_power_order();
        let index = order
            .iter()
            .position(|entry| entry.token == token && entry.uid == uid)
            .ok_or(())?;
        if order
            .iter()
            .any(|entry| entry.token == token && entry.uid != uid)
            && !token.is_instanced()
        {
            return Err(());
        }
        let state = Arc::make_mut(&mut self.0);
        if state.after_side_turn_end_inline.is_some() {
            debug_assert_eq!(index, 0);
            state.after_side_turn_end_inline = None;
        } else {
            let cold = Arc::make_mut(&mut state.cold);
            cold.after_side_turn_end.remove(index);
            if cold.after_side_turn_end.len() == 1 {
                state.after_side_turn_end_inline = cold.after_side_turn_end.pop();
            }
        }
        Ok(())
    }

    pub(crate) fn after_side_turn_end_power_uid(
        &self,
        token: AfterSideTurnEndPowerToken,
    ) -> Option<u32> {
        (!token.is_instanced())
            .then(|| {
                self.after_side_turn_end_power_order()
                    .iter()
                    .find(|entry| entry.token == token)
                    .map(|entry| entry.uid)
            })
            .flatten()
    }

    fn replace_instanced_player_power_records(
        &mut self,
        panache: Option<&[PanacheInstance]>,
        monologue: Option<&[MonologueInstance]>,
    ) {
        let state = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut state.cold);
        let mut instanced = cold
            .power_records
            .iter()
            .copied()
            .filter(|record| {
                matches!(
                    record,
                    ColdPowerRecord::Panache(_) | ColdPowerRecord::Monologue(_)
                )
            })
            .collect::<Vec<_>>();
        if let Some(instances) = panache {
            instanced.retain(|record| !matches!(record, ColdPowerRecord::Panache(_)));
            instanced.extend(instances.iter().copied().map(ColdPowerRecord::Panache));
        }
        if let Some(instances) = monologue {
            instanced.retain(|record| !matches!(record, ColdPowerRecord::Monologue(_)));
            instanced.extend(instances.iter().copied().map(ColdPowerRecord::Monologue));
        }
        instanced.sort_unstable_by_key(|record| match record {
            ColdPowerRecord::Panache(instance) => instance.uid,
            ColdPowerRecord::Monologue(instance) => instance.uid,
            _ => unreachable!("filtered instanced player-power record"),
        });
        cold.power_records.retain(|record| {
            !matches!(
                record,
                ColdPowerRecord::Panache(_) | ColdPowerRecord::Monologue(_)
            )
        });
        cold.power_records.extend(instanced);
    }

    #[cfg(test)]
    pub(crate) fn set_panache_instances(&mut self, instances: &[PanacheInstance]) -> bool {
        let monologue = self.monologue_instances().collect::<Vec<_>>();
        self.set_instanced_player_power_instances(instances, &monologue)
    }

    #[cfg(test)]
    pub(crate) fn set_monologue_instances(&mut self, instances: &[MonologueInstance]) -> bool {
        let panache = self.panache_instances().collect::<Vec<_>>();
        self.set_instanced_player_power_instances(&panache, instances)
    }

    /// Transactionally replace both native Instanced families that share one
    /// acquisition-uid domain.  Boundary hydration must install them as one
    /// unit because either legacy compatibility mirror may temporarily
    /// disagree while the other family has not yet been decoded.
    pub(crate) fn set_instanced_player_power_instances(
        &mut self,
        panache: &[PanacheInstance],
        monologue: &[MonologueInstance],
    ) -> bool {
        let panache_rows_are_valid = panache.iter().enumerate().all(|(index, instance)| {
            instance.uid < self.next_after_side_turn_end_power_uid()
                && matches!(instance.amount, 10 | 14)
                && (1..=5).contains(&instance.cards_left)
                && (index == 0 || panache[index - 1].uid < instance.uid)
        });
        let monologue_rows_are_valid = monologue.iter().enumerate().all(|(index, instance)| {
            instance.uid < self.next_after_side_turn_end_power_uid()
                && instance.amount == 1
                && instance.power == 1
                && instance.strength_applied >= 0
                && (index == 0 || monologue[index - 1].uid < instance.uid)
        });
        if !panache_rows_are_valid
            || !monologue_rows_are_valid
            || panache
                .iter()
                .try_fold(0_i32, |sum, instance| sum.checked_add(instance.amount))
                .is_none()
        {
            return false;
        }
        let mirror = match panache {
            [] => 5,
            [instance] => instance.cards_left,
            _ => 5,
        };
        let Some(applied) = monologue.iter().try_fold(0_i32, |sum, instance| {
            sum.checked_add(instance.strength_applied)
        }) else {
            return false;
        };
        if monologue
            .iter()
            .try_fold(0_i32, |sum, instance| sum.checked_add(instance.amount))
            .is_none()
        {
            return false;
        }
        let mut candidate = self.clone();
        candidate.replace_instanced_player_power_records(Some(panache), Some(monologue));
        if !candidate.try_set_panache_left(mirror) {
            return false;
        }
        let state = Arc::make_mut(&mut candidate.0);
        state.monologue_strength_applied = applied;
        if monologue.is_empty() {
            state.private_power_flags &= !MONOLOGUE_HOOK_MASK;
        } else {
            state.private_power_flags |= MONOLOGUE_HOOK_MASK;
        }
        if !candidate.instanced_player_power_records_are_exact() {
            return false;
        }
        *self = candidate;
        true
    }
    pub(crate) fn set_star_energy_reset_order(&mut self, order: &[PowerId]) -> bool {
        if order.len() > MAX_STAR_ENERGY_RESET_POWERS {
            return false;
        }
        let state = Arc::make_mut(&mut self.0);
        state.star_energy_reset[..order.len()].copy_from_slice(order);
        state.turn_and_generated_lens = (state.turn_and_generated_lens
            & !STAR_ENERGY_RESET_LEN_MASK)
            | ((order.len() as u8) << STAR_ENERGY_RESET_LEN_SHIFT);
        true
    }
    /// Boundary hydration restores a supplied complete reset order without
    /// consulting map iteration. The caller verifies its live membership and
    /// the two legacy subgroup projections atomically.
    pub(crate) fn set_after_energy_reset_order(&mut self, order: &[AfterEnergyResetPower]) -> bool {
        if order.len() > AFTER_ENERGY_RESET_POWERS.len()
            || order
                .iter()
                .any(|power| !AFTER_ENERGY_RESET_POWERS.contains(power))
            || order
                .iter()
                .enumerate()
                .any(|(index, power)| order[..index].contains(power))
        {
            return false;
        }
        Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold).after_energy_reset_order =
            Some(AfterEnergyResetOrder::from_slice(order));
        true
    }

    /// Only an internally reconstructed combat-entry root may declare its
    /// empty listener order known before any power application.
    pub(crate) fn initialize_after_energy_reset_order_at_entry(&mut self) {
        Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold).after_energy_reset_order =
            Some(AfterEnergyResetOrder::from_slice(&[]));
    }

    pub(crate) fn mark_after_energy_reset_order_legacy_unknown(&mut self) {
        Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold).after_energy_reset_order = None;
    }

    /// Successful first attachment appends; a restack preserves place. An
    /// unknown legacy checkpoint stays unknown rather than acquiring an
    /// invented relative order for this new listener. Adding a peer to an
    /// inferred legacy sequence makes the cross-family acquisition history
    /// observable, so only that novel attachment promotes it to wire-explicit.
    pub(crate) fn register_after_energy_reset(&mut self, power: AfterEnergyResetPower) -> bool {
        let state = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut state.cold);
        if let Some(order) = &mut cold.after_energy_reset_order
            && order.register(power)
        {
            order.explicit = true;
        }
        true
    }

    /// Call only after native removal actually detached the Type1 listener.
    /// Decrements and Illusion-vetoed owner death retain their acquisition
    /// position and must not clear this ledger.
    pub(crate) fn unregister_after_energy_reset(&mut self, power: AfterEnergyResetPower) {
        let cold = Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold);
        let clear_implicit_empty = if let Some(order) = &mut cold.after_energy_reset_order {
            order.unregister(power);
            !order.explicit && order.as_slice().is_empty()
        } else {
            false
        };
        if clear_implicit_empty {
            // Legacy absence cannot prove an empty listener ledger. Dropping
            // the inferred carrier makes warm removal reload exactly as the
            // old wire root, while explicit entry/supplied empty remains.
            cold.after_energy_reset_order = None;
        }
    }
    pub(crate) fn mark_after_energy_reset_order_legacy_inferred(&mut self) {
        if let Some(order) =
            &mut Arc::make_mut(&mut Arc::make_mut(&mut self.0).cold).after_energy_reset_order
        {
            order.mark_legacy_inferred();
        }
    }
    pub(crate) fn set_local_generated_power_order(&mut self, order: &[PowerId]) -> bool {
        if order.len() > MAX_LOCAL_GENERATED_POWERS {
            return false;
        }
        let state = Arc::make_mut(&mut self.0);
        state.local_generated[..order.len()].copy_from_slice(order);
        state.turn_and_generated_lens = (state.turn_and_generated_lens & !LOCAL_GENERATED_LEN_MASK)
            | ((order.len() as u8) << LOCAL_GENERATED_LEN_SHIFT);
        true
    }
    pub(crate) fn set_result_location_power_order(&mut self, order: &[PowerId]) -> bool {
        if order.len() > MAX_RESULT_LOCATION_POWERS {
            return false;
        }
        let mut compact = [ResultLocationPower::Corruption; MAX_RESULT_LOCATION_POWERS];
        for (output, power) in compact.iter_mut().zip(order.iter().copied()) {
            let Some(power) = ResultLocationPower::from_power(power) else {
                return false;
            };
            *output = power;
        }
        let state = Arc::make_mut(&mut self.0);
        state.result_location[..order.len()].copy_from_slice(&compact[..order.len()]);
        state.turn_and_generated_lens =
            with_result_location_len(state.turn_and_generated_lens, order.len() as u8);
        true
    }

    /// Register a zero-to-positive listener edge without moving re-stacks.
    pub(crate) fn register_after_card_drawn(&mut self, power: PowerId) -> bool {
        register_fanout_power(&mut self.0, power, |state| {
            (&mut state.after_card_drawn, &mut state.after_card_drawn_len)
        })
    }

    /// Register a zero-to-positive listener edge without moving re-stacks.
    pub(crate) fn register_after_card_exhausted(&mut self, power: PowerId) -> bool {
        register_fanout_power(&mut self.0, power, |state| {
            (
                &mut state.after_card_exhausted,
                &mut state.after_card_exhausted_len,
            )
        })
    }

    /// Register a zero-to-positive listener edge without moving re-stacks.
    pub(crate) fn register_before_hand_draw(&mut self, power: PowerId) -> bool {
        register_fanout_power(&mut self.0, power, |state| {
            (&mut state.before_hand_draw, &mut state.before_hand_draw_len)
        })
    }

    #[inline]
    pub(crate) fn unregister_before_hand_draw(&mut self, power: PowerId) {
        let state = Arc::make_mut(&mut self.0);
        let len = usize::from(state.before_hand_draw_len);
        if let Some(index) = state.before_hand_draw[..len]
            .iter()
            .position(|candidate| *candidate == power)
        {
            state.before_hand_draw.copy_within(index + 1..len, index);
            state.before_hand_draw_len -= 1;
        }
    }

    pub(crate) fn register_after_damage_given(&mut self, power: PowerId) -> bool {
        register_fanout_power(&mut self.0, power, |state| {
            (
                &mut state.after_damage_given,
                &mut state.after_damage_given_len,
            )
        })
    }
    pub(crate) fn register_after_block_gained(&mut self, power: PowerId) -> bool {
        register_fanout_power(&mut self.0, power, |state| {
            (
                &mut state.after_block_gained,
                &mut state.after_block_gained_len,
            )
        })
    }
    pub(crate) fn register_after_block_cleared(&mut self, power: PowerId) -> bool {
        register_fanout_power(&mut self.0, power, |state| {
            (
                &mut state.after_block_cleared,
                &mut state.after_block_cleared_len,
            )
        })
    }

    pub(crate) fn unregister_after_block_cleared(&mut self, power: PowerId) {
        let state = Arc::make_mut(&mut self.0);
        let len = usize::from(state.after_block_cleared_len);
        if let Some(index) = state.after_block_cleared[..len]
            .iter()
            .position(|candidate| *candidate == power)
        {
            state.after_block_cleared.copy_within(index + 1..len, index);
            state.after_block_cleared_len -= 1;
        }
    }
    pub(crate) fn register_after_power_amount_changed(&mut self, power: PowerId) -> bool {
        let Some(power) = PowerAmountChangedPower::from_power(power) else {
            return false;
        };
        let state = Arc::make_mut(&mut self.0);
        let occupied = usize::from(state.after_power_amount_changed_len);
        if state.after_power_amount_changed[..occupied].contains(&power) {
            return true;
        }
        if occupied == MAX_AFTER_POWER_AMOUNT_CHANGED_POWERS {
            return false;
        }
        state.after_power_amount_changed[occupied] = power;
        state.after_power_amount_changed_len += 1;
        true
    }

    pub(crate) fn can_register_after_power_amount_changed(&self, power: PowerId) -> bool {
        let Some(power) = PowerAmountChangedPower::from_power(power) else {
            return false;
        };
        let occupied = usize::from(self.0.after_power_amount_changed_len);
        self.0.after_power_amount_changed[..occupied].contains(&power)
            || occupied < MAX_AFTER_POWER_AMOUNT_CHANGED_POWERS
    }

    pub(crate) fn unregister_after_power_amount_changed(&mut self, power: PowerId) {
        let Some(power) = PowerAmountChangedPower::from_power(power) else {
            return;
        };
        let state = Arc::make_mut(&mut self.0);
        let len = usize::from(state.after_power_amount_changed_len);
        if let Some(index) = state.after_power_amount_changed[..len]
            .iter()
            .position(|candidate| *candidate == power)
        {
            state
                .after_power_amount_changed
                .copy_within(index + 1..len, index);
            state.after_power_amount_changed_len -= 1;
        }
    }
    fn register_after_side_turn_start_token(&mut self, token: AfterSideTurnStartToken) -> bool {
        let side_len = self.after_side_turn_start_order().len();
        if self.after_side_turn_start_order().contains(&token) {
            return true;
        }
        if side_len == MAX_AFTER_SIDE_TURN_START_POWERS {
            return false;
        }
        let state = Arc::make_mut(&mut self.0);
        state.after_side_turn_start[side_len] = token;
        let player_len = state.after_side_turn_start_len & AFTER_PLAYER_TURN_START_LEN_MASK;
        state.after_side_turn_start_len = (side_len + 1) as u8 | player_len;
        true
    }
    pub(crate) fn register_after_side_turn_start(&mut self, power: PowerId) -> bool {
        AfterSideTurnStartToken::from_power(power)
            .is_some_and(|token| self.register_after_side_turn_start_token(token))
    }
    fn unregister_after_side_turn_start_token(&mut self, token: AfterSideTurnStartToken) {
        let state = Arc::make_mut(&mut self.0);
        let len = usize::from(state.after_side_turn_start_len & AFTER_SIDE_TURN_START_LEN_MASK);
        if let Some(index) = state.after_side_turn_start[..len]
            .iter()
            .position(|candidate| *candidate == token)
        {
            state
                .after_side_turn_start
                .copy_within(index + 1..len, index);
            state.after_side_turn_start[len - 1] = AfterSideTurnStartToken::BiasedCognition;
            let player_len = state.after_side_turn_start_len & AFTER_PLAYER_TURN_START_LEN_MASK;
            state.after_side_turn_start_len = (len - 1) as u8 | player_len;
        }
    }
    pub(crate) fn unregister_after_side_turn_start(&mut self, power: PowerId) {
        if let Some(token) = AfterSideTurnStartToken::from_power(power) {
            self.unregister_after_side_turn_start_token(token);
        }
    }
    pub(crate) fn register_before_side_turn_end(&mut self, power: PowerId) -> bool {
        let token = match power {
            PowerId::ChainsOfBinding => BeforeSideTurnEndToken::ChainsOfBinding,
            PowerId::Hailstorm => BeforeSideTurnEndToken::Hailstorm,
            _ => return false,
        };
        let state = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut state.cold);
        if !cold.before_side_turn_end.contains(&token) {
            cold.before_side_turn_end.push(token);
        }
        true
    }
    pub(crate) fn register_star_energy_reset(&mut self, power: PowerId) -> bool {
        let state = Arc::make_mut(&mut self.0);
        let len = (state.turn_and_generated_lens & STAR_ENERGY_RESET_LEN_MASK)
            >> STAR_ENERGY_RESET_LEN_SHIFT;
        let Some(next_len) = append_fanout_power(&mut state.star_energy_reset, len, power) else {
            return false;
        };
        state.turn_and_generated_lens = (state.turn_and_generated_lens
            & !STAR_ENERGY_RESET_LEN_MASK)
            | (next_len << STAR_ENERGY_RESET_LEN_SHIFT);
        self.register_after_energy_reset(
            AfterEnergyResetPower::from_power(power).expect("star reset power is closed"),
        )
    }
    pub(crate) fn register_local_generated_power(&mut self, power: PowerId) -> bool {
        let state = Arc::make_mut(&mut self.0);
        let len =
            (state.turn_and_generated_lens & LOCAL_GENERATED_LEN_MASK) >> LOCAL_GENERATED_LEN_SHIFT;
        let Some(next_len) = append_fanout_power(&mut state.local_generated, len, power) else {
            return false;
        };
        state.turn_and_generated_lens = (state.turn_and_generated_lens & !LOCAL_GENERATED_LEN_MASK)
            | (next_len << LOCAL_GENERATED_LEN_SHIFT);
        true
    }
    pub(crate) fn register_result_location_power(&mut self, power: PowerId) -> bool {
        let Some(power) = ResultLocationPower::from_power(power) else {
            return false;
        };
        let state = Arc::make_mut(&mut self.0);
        let occupied = usize::from(result_location_len(state.turn_and_generated_lens));
        if state.result_location[..occupied].contains(&power) {
            return true;
        }
        if occupied == MAX_RESULT_LOCATION_POWERS {
            return false;
        }
        state.result_location[occupied] = power;
        state.turn_and_generated_lens =
            with_result_location_len(state.turn_and_generated_lens, occupied as u8 + 1);
        true
    }

    pub(crate) fn unregister_result_location_power(&mut self, power: PowerId) {
        let Some(power) = ResultLocationPower::from_power(power) else {
            return;
        };
        let state = Arc::make_mut(&mut self.0);
        let len = usize::from(result_location_len(state.turn_and_generated_lens));
        if let Some(index) = state.result_location[..len]
            .iter()
            .position(|candidate| *candidate == power)
        {
            state.result_location.copy_within(index + 1..len, index);
            state.turn_and_generated_lens =
                with_result_location_len(state.turn_and_generated_lens, len as u8 - 1);
        }
    }

    pub(crate) fn unregister_star_energy_reset(&mut self, power: PowerId) {
        let state = Arc::make_mut(&mut self.0);
        let packed_len = (state.turn_and_generated_lens & STAR_ENERGY_RESET_LEN_MASK)
            >> STAR_ENERGY_RESET_LEN_SHIFT;
        let len = usize::from(packed_len);
        if let Some(index) = state.star_energy_reset[..len]
            .iter()
            .position(|candidate| *candidate == power)
        {
            state.star_energy_reset.copy_within(index + 1..len, index);
            state.turn_and_generated_lens = (state.turn_and_generated_lens
                & !STAR_ENERGY_RESET_LEN_MASK)
                | ((packed_len - 1) << STAR_ENERGY_RESET_LEN_SHIFT);
        }
        if let Some(power) = AfterEnergyResetPower::from_power(power) {
            self.unregister_after_energy_reset(power);
        }
    }

    pub fn automation_left(&self) -> i32 {
        i32::from(self.0.automation_panache_tender & AUTOMATION_LEFT_MASK)
    }
    pub(crate) fn set_automation_left(&mut self, value: i32) {
        let value = u8::try_from(value).expect("Automation countdown stays in u8 range");
        assert!(
            value <= AUTOMATION_LEFT_MAX,
            "Automation countdown stays in packed four-bit range"
        );
        let state = Arc::make_mut(&mut self.0);
        state.automation_panache_tender =
            (state.automation_panache_tender & !AUTOMATION_LEFT_MASK) | value;
    }
    pub(crate) fn try_set_automation_left(&mut self, value: i32) -> bool {
        let Ok(value) = u8::try_from(value) else {
            return false;
        };
        if value > AUTOMATION_LEFT_MAX {
            return false;
        }
        let state = Arc::make_mut(&mut self.0);
        state.automation_panache_tender =
            (state.automation_panache_tender & !AUTOMATION_LEFT_MASK) | value;
        true
    }
    /// Automation objects after the first live one, in acquisition order.
    pub(crate) fn automation_later_instances(&self) -> Vec<AutomationInstance> {
        self.automation_later_rows().collect()
    }
    fn automation_later_rows(&self) -> impl Iterator<Item = AutomationInstance> + '_ {
        self.0
            .cold
            .power_records
            .iter()
            .filter_map(|record| match record {
                ColdPowerRecord::Automation(instance) => Some(*instance),
                _ => None,
            })
    }
    /// Checked total amount carried by [`Self::automation_later_instances`].
    pub(crate) fn automation_later_amount(&self) -> Option<i32> {
        self.automation_later_rows()
            .try_fold(0_i32, |sum, instance| sum.checked_add(instance.amount))
    }
    /// Replace the later Automation objects. Every row must carry a positive
    /// amount and a resting countdown in `1..=10`; the aggregate relation to
    /// the scalar power is authenticated by admission and by the draw
    /// listener, which own the scalar.
    pub(crate) fn set_automation_later_instances(
        &mut self,
        instances: &[AutomationInstance],
    ) -> bool {
        if instances
            .iter()
            .any(|instance| instance.amount < 1 || !(1..=10).contains(&instance.cards_left))
            || instances
                .iter()
                .try_fold(0_i32, |sum, instance| sum.checked_add(instance.amount))
                .is_none()
        {
            return false;
        }
        if self.automation_later_rows().eq(instances.iter().copied()) {
            return true;
        }
        let state = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut state.cold);
        cold.power_records
            .retain(|record| !matches!(record, ColdPowerRecord::Automation(_)));
        cold.power_records
            .extend(instances.iter().copied().map(ColdPowerRecord::Automation));
        true
    }
    pub fn cacophony_left(&self) -> i32 {
        i32::from(self.0.cacophony_left)
    }
    pub(crate) fn set_cacophony_left(&mut self, value: i32) {
        Arc::make_mut(&mut self.0).cacophony_left =
            u8::try_from(value).expect("Cacophony countdown stays in u8 range");
    }
    pub(crate) fn try_set_cacophony_left(&mut self, value: i32) -> bool {
        let Ok(value) = u8::try_from(value) else {
            return false;
        };
        Arc::make_mut(&mut self.0).cacophony_left = value;
        true
    }
    pub fn cacophony_resets_completed(&self) -> i32 {
        self.0.cacophony_resets_completed
    }
    pub(crate) fn set_cacophony_resets_completed(&mut self, value: i32) {
        Arc::make_mut(&mut self.0).cacophony_resets_completed = value;
    }
    pub fn dark_embrace_ethereal(&self) -> i32 {
        self.0.dark_embrace_ethereal
    }
    pub(crate) fn set_dark_embrace_ethereal(&mut self, value: i32) {
        Arc::make_mut(&mut self.0).dark_embrace_ethereal = value;
    }
    pub fn panache_left(&self) -> i32 {
        if self
            .0
            .cold
            .damage_execution
            .as_ref()
            .is_some_and(|execution| !execution.panache.is_empty())
        {
            let mut instances = self.panache_instances();
            if let (Some(instance), None) = (instances.next(), instances.next()) {
                return instance.cards_left;
            }
        }
        i32::from((self.0.automation_panache_tender & PANACHE_LEFT_MASK) >> PANACHE_LEFT_SHIFT)
    }
    #[cfg(test)]
    pub(crate) fn set_panache_left(&mut self, value: i32) {
        let value = u8::try_from(value).expect("Panache countdown stays in u8 range");
        assert!(
            value <= PANACHE_LEFT_MAX,
            "Panache countdown stays in packed three-bit range"
        );
        let state = Arc::make_mut(&mut self.0);
        state.automation_panache_tender =
            (state.automation_panache_tender & !PANACHE_LEFT_MASK) | (value << PANACHE_LEFT_SHIFT);
    }
    pub(crate) fn try_set_panache_left(&mut self, value: i32) -> bool {
        let Ok(value) = u8::try_from(value) else {
            return false;
        };
        if value > PANACHE_LEFT_MAX {
            return false;
        }
        let state = Arc::make_mut(&mut self.0);
        state.automation_panache_tender =
            (state.automation_panache_tender & !PANACHE_LEFT_MASK) | (value << PANACHE_LEFT_SHIFT);
        true
    }

    #[inline]
    pub(crate) fn tender_is_active(&self) -> bool {
        self.0.automation_panache_tender & TENDER_ACTIVE_MASK != 0
    }

    #[inline]
    pub(crate) fn tender_cards_played(&self) -> i32 {
        self.0.tender_cards_played
    }

    /// Replace the complete private Tender object state atomically.
    pub(crate) fn set_tender_state(&mut self, active: bool, cards_played: i32) -> bool {
        if cards_played < 0 || (!active && cards_played != 0) {
            return false;
        }
        let state = Arc::make_mut(&mut self.0);
        if active {
            state.automation_panache_tender |= TENDER_ACTIVE_MASK;
        } else {
            state.automation_panache_tender &= !TENDER_ACTIVE_MASK;
        }
        state.tender_cards_played = cards_played;
        true
    }

    #[cfg(test)]
    pub(crate) fn forge_tender_state(&mut self, active: bool, cards_played: i32) {
        let state = Arc::make_mut(&mut self.0);
        if active {
            state.automation_panache_tender |= TENDER_ACTIVE_MASK;
        } else {
            state.automation_panache_tender &= !TENDER_ACTIVE_MASK;
        }
        state.tender_cards_played = cards_played;
    }
    pub fn pale_blue_dot_used(&self) -> bool {
        self.0.private_power_flags & PALE_BLUE_DOT_USED_MASK != 0
    }
    pub(crate) fn set_pale_blue_dot_used(&mut self, value: bool) {
        let state = Arc::make_mut(&mut self.0);
        if value {
            state.private_power_flags |= PALE_BLUE_DOT_USED_MASK;
        } else {
            state.private_power_flags &= !PALE_BLUE_DOT_USED_MASK;
        }
    }

    /// Whether this fight records physical `StrengthPower` attachments.
    ///
    /// See [`ColdFanoutState::misery_attachment_upkeep`]: a per-fight
    /// constant mirrored from the catalog at hydration, not a projected
    /// state word.
    #[inline(always)]
    pub fn misery_attachment_upkeep(&self) -> bool {
        self.0.cold.misery_attachment_upkeep
    }

    /// Normality's owner `CardPlaysStarted`-this-turn count (see
    /// [`ColdFanoutState::normality_card_plays_started_this_turn`]).
    #[inline(always)]
    pub fn normality_card_plays_started_this_turn(&self) -> i32 {
        self.0.cold.normality_card_plays_started_this_turn
    }

    /// Write Normality's started-play count. Returns `false`, writing
    /// nothing, for a negative value. An unchanged value pays no
    /// copy-on-write clone, so the side-end reset of an untracked fight is
    /// free.
    pub(crate) fn set_normality_card_plays_started_this_turn(&mut self, value: i32) -> bool {
        if value < 0 {
            return false;
        }
        if self.0.cold.normality_card_plays_started_this_turn == value {
            return true;
        }
        let state = Arc::make_mut(&mut self.0);
        Arc::make_mut(&mut state.cold).normality_card_plays_started_this_turn = value;
        true
    }

    /// Mirror the catalog's attachment-reader reachability onto the state.
    ///
    /// Only boundary hydration and tests call this. It is idempotent and
    /// pays no copy-on-write clone when the value is already correct, so a
    /// fight without `Misery` allocates nothing here.
    pub(crate) fn set_misery_attachment_upkeep(&mut self, value: bool) {
        if self.0.cold.misery_attachment_upkeep == value {
            return;
        }
        let state = Arc::make_mut(&mut self.0);
        Arc::make_mut(&mut state.cold).misery_attachment_upkeep = value;
    }

    pub fn unsettling_lamp_available(&self) -> bool {
        self.0.private_power_flags & UNSETTLING_LAMP_AVAILABLE_MASK != 0
    }

    pub(crate) fn set_unsettling_lamp_available(&mut self, value: bool) {
        let state = Arc::make_mut(&mut self.0);
        if value {
            state.private_power_flags |= UNSETTLING_LAMP_AVAILABLE_MASK;
        } else {
            state.private_power_flags &= !UNSETTLING_LAMP_AVAILABLE_MASK;
        }
    }

    pub(crate) fn pet(&self) -> SoloPetState {
        self.0.pet
    }

    pub(crate) fn set_osty(&mut self, osty: Option<(i32, i32)>) -> Result<(), PetStateError> {
        Arc::make_mut(&mut self.0).pet.set_osty(osty)
    }

    pub(crate) fn set_osty_attacks_this_turn(&mut self, amount: i32) -> Result<(), PetStateError> {
        Arc::make_mut(&mut self.0).pet.set_attacks_this_turn(amount)
    }

    /// Hydrate the retained dead Osty from a document (#3674).
    ///
    /// This is a decode, not a creation. `OstyCmd/<Summon>d__0::MoveNext`
    /// (v0.111.0 RVA `0x3ee040`) creates the Osty creature once, through
    /// `PlayerCmd.AddPet<Osty>` at `IL_01e9`, and that is where
    /// `CombatState::AttachCreature` (`0x13718f` `IL_0009`-`IL_0022`) advanced
    /// `_nextCreatureId`. The retained corpse is that same creature, so the
    /// document's own `next_creature_uid` already counts it, exactly as it
    /// counts a live `ally` ([`Self::set_osty`], which never advanced the
    /// counter either).
    ///
    /// The loader used to route this through [`Self::mutate_pet`], whose
    /// absent-to-present edge is the engine's creation and advances a tracked
    /// counter. `next_creature_uid` sorts before `osty_corpse` in the entity
    /// bag, so the counter was already tracked when the corpse arrived, and
    /// every document holding both reloaded with the counter one too high:
    /// 78 census projections in 30 fights did not project back to themselves,
    /// and the next spawned enemy of a reloaded state would have taken a uid
    /// one above the session's (and above native's `.mcr` target id).
    pub(crate) fn set_osty_corpse(&mut self, corpse: bool) -> Result<(), PetStateError> {
        Arc::make_mut(&mut self.0).pet.set_corpse(corpse)
    }

    /// Run one pet operation. A fresh Osty creature takes a native creature
    /// id (#3039): `OstyCmd/<Summon>d__0::MoveNext` (RVA `0x3ee040`) looks
    /// the retained Osty up in `Allies` (`IL_00e3`-`IL_0103`) and revives it
    /// through `PlayerCombatState::AddPetInternal` (`IL_01d4`, no new
    /// creature) when one exists, and otherwise creates one through
    /// `PlayerCmd.AddPet<Osty>` (`IL_01e9`), whose `CreateCreature` attaches
    /// it and advances `_nextCreatureId`. The retained Osty is exactly
    /// [`SoloPetState::has_die_for_you`], so its false-to-true edge is that
    /// creation. A tracked counter advances with it; an untracked (zero)
    /// counter is left alone, and every spawner refuses by name while an
    /// Osty exists under it (`engine::monsters::allocate_creature_uid`).
    pub(crate) fn mutate_pet(
        &mut self,
        operation: impl FnOnce(&mut SoloPetState) -> Result<(), PetStateError>,
    ) -> Result<(), PetStateError> {
        let existed = self.0.pet.has_die_for_you();
        let tracked = self.batch_nine().next_creature_uid;
        if tracked != 0 && !existed {
            // Rehearse so a counter overflow publishes nothing.
            let mut pet = self.0.pet;
            operation(&mut pet)?;
            if pet.has_die_for_you() {
                let next = tracked
                    .checked_add(1)
                    .ok_or(PetStateError::CounterOverflow)?;
                self.set_next_creature_uid(next);
            }
            Arc::make_mut(&mut self.0).pet = pet;
            return Ok(());
        }
        operation(&mut Arc::make_mut(&mut self.0).pet)
    }

    pub(crate) fn fetch_finished(&self, uid: u32) -> bool {
        self.0.cold.fetch_finished_uids.contains(&uid)
    }

    pub(crate) fn record_fetch_finished(&mut self, uid: u32) {
        let state = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut state.cold);
        if !cold.fetch_finished_uids.contains(&uid) {
            cold.fetch_finished_uids.push(uid);
        }
    }

    pub(crate) fn reset_fetch_finished(&mut self) {
        let state = Arc::make_mut(&mut self.0);
        Arc::make_mut(&mut state.cold).fetch_finished_uids.clear();
    }

    pub(crate) fn the_bomb_state_is_exact(&self) -> bool {
        let cold = &self.0.cold;
        if self
            .the_bomb_instances()
            .enumerate()
            .any(|(index, instance)| {
                instance.uid >= cold.next_the_bomb_uid
                    || !(1..=3).contains(&instance.turns)
                    || !matches!(instance.damage, 40 | 50)
                    || self
                        .the_bomb_instances()
                        .take(index)
                        .any(|prior| prior.uid == instance.uid)
            })
        {
            return false;
        }
        let chains_count = cold
            .before_side_turn_end
            .iter()
            .filter(|token| matches!(token, BeforeSideTurnEndToken::ChainsOfBinding))
            .count();
        let hailstorm_count = cold
            .before_side_turn_end
            .iter()
            .filter(|token| matches!(token, BeforeSideTurnEndToken::Hailstorm))
            .count();
        if chains_count > 1 || hailstorm_count > 1 {
            return false;
        }
        let bomb_tokens = cold
            .before_side_turn_end
            .iter()
            .filter_map(|token| match token {
                BeforeSideTurnEndToken::ChainsOfBinding => None,
                BeforeSideTurnEndToken::Hailstorm => None,
                BeforeSideTurnEndToken::TheBomb(uid) => Some(*uid),
            });
        bomb_tokens.eq(self.the_bomb_instances().map(|instance| instance.uid))
    }

    fn toric_toughness_instance_is_exact(instance: ToricToughnessInstance) -> bool {
        if instance.duration <= 0 {
            return false;
        }
        let Some((numerator, denominator)) = instance.block.nonnegative_fraction() else {
            return false;
        };
        matches!(denominator, 1 | 2 | 4) && numerator < (2_147_483_648_u128 * denominator)
    }

    /// Authenticate the unique keyed Toric record and its reachable
    /// quarter-unit Decimal payload. Listener correspondence is validated by
    /// admission because its order lives only in `after_block_cleared`.
    pub(crate) fn cold_power_records_are_exact(&self) -> bool {
        let mut seen_toric = false;
        let mut seen_sloth = false;
        let mut reached_instanced = false;
        let mut previous_instance_uid = None;
        self.0.cold.power_records.iter().all(|record| match record {
            ColdPowerRecord::TheBomb(_) => !seen_toric && !seen_sloth && !reached_instanced,
            ColdPowerRecord::ToricToughness(instance) => {
                if seen_toric
                    || seen_sloth
                    || reached_instanced
                    || !Self::toric_toughness_instance_is_exact(*instance)
                {
                    return false;
                }
                seen_toric = true;
                true
            }
            ColdPowerRecord::Sloth(instance) => {
                if seen_sloth || reached_instanced || instance.cards_played_this_turn < 0 {
                    return false;
                }
                seen_sloth = true;
                true
            }
            ColdPowerRecord::Panache(instance) => {
                reached_instanced = true;
                let valid = instance.uid < self.0.next_after_side_turn_end_power_uid
                    && matches!(instance.amount, 10 | 14)
                    && (instance.cards_left <= 5
                        && (instance.cards_left >= 1
                            || self
                                .0
                                .cold
                                .damage_execution
                                .as_ref()
                                .map_or(0, |execution| {
                                    execution
                                        .panache
                                        .iter()
                                        .filter(|uid| **uid == instance.uid)
                                        .count()
                                }) as i64
                                >= 1_i64 - i64::from(instance.cards_left)))
                    && previous_instance_uid.is_none_or(|uid| uid < instance.uid);
                previous_instance_uid = Some(instance.uid);
                valid
            }
            ColdPowerRecord::Monologue(instance) => {
                reached_instanced = true;
                let valid = instance.uid < self.0.next_after_side_turn_end_power_uid
                    && instance.amount == 1
                    && instance.power == 1
                    && instance.strength_applied >= 0
                    && previous_instance_uid.is_none_or(|uid| uid < instance.uid);
                previous_instance_uid = Some(instance.uid);
                valid
            }
            ColdPowerRecord::Automation(instance) => {
                instance.amount >= 1 && (1..=10).contains(&instance.cards_left)
            }
        })
    }

    pub(crate) fn instanced_player_power_records_are_exact(&self) -> bool {
        if !self.cold_power_records_are_exact() {
            return false;
        }
        let Some(monologue_applied) =
            self.monologue_instances().try_fold(0_i32, |sum, instance| {
                sum.checked_add(instance.strength_applied)
            })
        else {
            return false;
        };
        let mut panache = self.panache_instances();
        let first = panache.next();
        let second = panache.next();
        let panache_mirror_is_exact = match (first, second) {
            (None, None) => self.panache_left() == 5,
            (Some(instance), None) => self.panache_left() == instance.cards_left,
            (Some(_), Some(_)) => self.panache_left() == 5,
            (None, Some(_)) => unreachable!("an iterator cannot yield its second item first"),
        };
        self.monologue_strength_applied() == monologue_applied
            && self.monologue_hooks_are_registered() == self.monologue_instances().next().is_some()
            && panache_mirror_is_exact
    }

    pub(crate) fn toric_toughness_state_is_exact(&self) -> bool {
        self.cold_power_records_are_exact()
    }

    pub(crate) fn set_toric_toughness(&mut self, instance: Option<ToricToughnessInstance>) -> bool {
        if instance.is_some_and(|instance| !Self::toric_toughness_instance_is_exact(instance)) {
            return false;
        }
        let state = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut state.cold);
        cold.power_records
            .retain(|record| !matches!(record, ColdPowerRecord::ToricToughness(_)));
        if let Some(instance) = instance {
            let insertion = cold
                .power_records
                .iter()
                .position(|record| {
                    matches!(
                        record,
                        ColdPowerRecord::Sloth(_)
                            | ColdPowerRecord::Panache(_)
                            | ColdPowerRecord::Monologue(_)
                    )
                })
                .unwrap_or(cold.power_records.len());
            cold.power_records
                .insert(insertion, ColdPowerRecord::ToricToughness(instance));
        }
        debug_assert!(self.cold_power_records_are_exact());
        true
    }

    pub(crate) fn set_sloth(&mut self, instance: Option<SlothInstance>) -> bool {
        if instance.is_some_and(|instance| instance.cards_played_this_turn < 0) {
            return false;
        }
        let state = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut state.cold);
        cold.power_records
            .retain(|record| !matches!(record, ColdPowerRecord::Sloth(_)));
        if let Some(instance) = instance {
            let insertion = cold
                .power_records
                .iter()
                .position(|record| {
                    matches!(
                        record,
                        ColdPowerRecord::Panache(_) | ColdPowerRecord::Monologue(_)
                    )
                })
                .unwrap_or(cold.power_records.len());
            cold.power_records
                .insert(insertion, ColdPowerRecord::Sloth(instance));
        }
        debug_assert!(self.cold_power_records_are_exact());
        true
    }

    pub(crate) fn increment_sloth(&mut self) -> Result<i32, ()> {
        let current = self.sloth().ok_or(())?;
        let next = current.cards_played_this_turn.checked_add(1).ok_or(())?;
        self.set_sloth(Some(SlothInstance {
            cards_played_this_turn: next,
        }))
        .then_some(next)
        .ok_or(())
    }

    /// Apply or restack the one keyed Toric instance in place. First
    /// application registers exactly one listener; restacks preserve its
    /// acquisition ordinal and overwrite the retained block payload.
    pub(crate) fn apply_toric_toughness(
        &mut self,
        duration: i32,
        block: DotNetDecimal,
    ) -> Result<i32, ()> {
        if !self.toric_toughness_state_is_exact() || duration <= 0 {
            return Err(());
        }
        let current = self.toric_toughness();
        if self
            .after_block_cleared_order()
            .contains(&PowerId::ToricToughness)
            != current.is_some()
        {
            return Err(());
        }
        let updated = current.map_or(Ok(duration), |instance| {
            instance.duration.checked_add(duration).ok_or(())
        })?;
        let next = ToricToughnessInstance {
            duration: updated,
            block,
        };
        if !Self::toric_toughness_instance_is_exact(next) {
            return Err(());
        }
        if current.is_none() && !self.register_after_block_cleared(PowerId::ToricToughness) {
            return Err(());
        }
        if !self.set_toric_toughness(Some(next)) {
            return Err(());
        }
        Ok(updated)
    }

    pub(crate) fn decrement_toric_toughness(&mut self) -> Result<i32, ()> {
        if !self.toric_toughness_state_is_exact() {
            return Err(());
        }
        let current = self.toric_toughness().ok_or(())?;
        let updated = current.duration - 1;
        if updated == 0 {
            if !self.set_toric_toughness(None) {
                return Err(());
            }
            self.unregister_after_block_cleared(PowerId::ToricToughness);
        } else if !self.set_toric_toughness(Some(ToricToughnessInstance {
            duration: updated,
            block: current.block,
        })) {
            return Err(());
        }
        Ok(updated)
    }

    /// Allocate one fresh native Panache object. InstanceType 1 makes every
    /// application a new listener even though StackType is Intensity.
    pub(crate) fn begin_panache_instance(&mut self, amount: i32) -> Result<u32, ()> {
        if !self.instanced_player_power_records_are_exact()
            || !matches!(amount, 10 | 14)
            || self.0.next_after_side_turn_end_power_uid == u32::MAX
        {
            return Err(());
        }
        let state = Arc::make_mut(&mut self.0);
        let uid = state.next_after_side_turn_end_power_uid;
        state.next_after_side_turn_end_power_uid += 1;
        let entry = AfterSideTurnEndPowerEntry {
            token: AfterSideTurnEndPowerToken::Panache,
            uid,
        };
        if state.after_side_turn_end_inline.is_none() && state.cold.after_side_turn_end.is_empty() {
            state.after_side_turn_end_inline = Some(entry);
        } else if let Some(first) = state.after_side_turn_end_inline.take() {
            let cold = Arc::make_mut(&mut state.cold);
            debug_assert!(cold.after_side_turn_end.is_empty());
            cold.after_side_turn_end.extend([first, entry]);
        } else {
            Arc::make_mut(&mut state.cold)
                .after_side_turn_end
                .push(entry);
        }
        Arc::make_mut(&mut state.cold)
            .after_card_played
            .push(AfterCardPlayedPowerEntry {
                token: AfterCardPlayedPowerToken::Panache,
                uid,
            });
        let cold = Arc::make_mut(&mut state.cold);
        cold.power_records
            .push(ColdPowerRecord::Panache(PanacheInstance {
                uid,
                amount,
                cards_left: 5,
                already_applied: false,
            }));
        // No scalar countdown can encode multiple native objects. Keep the
        // old field as a singleton/default compatibility mirror only.
        let previous = (state.automation_panache_tender & PANACHE_LEFT_MASK) >> PANACHE_LEFT_SHIFT;
        if previous != 5 {
            state.automation_panache_tender =
                (state.automation_panache_tender & !PANACHE_LEFT_MASK) | (5 << PANACHE_LEFT_SHIFT);
        }
        Ok(uid)
    }

    /// Allocate one fresh native Monologue object and all three of its hook
    /// memberships. `power` is the card-written Strength dynamic variable.
    pub(crate) fn begin_monologue_instance(&mut self, amount: i32, power: i32) -> Result<u32, ()> {
        if !self.instanced_player_power_records_are_exact()
            || amount != 1
            || power != 1
            || self.0.next_after_side_turn_end_power_uid == u32::MAX
        {
            return Err(());
        }
        let state = Arc::make_mut(&mut self.0);
        let uid = state.next_after_side_turn_end_power_uid;
        state.next_after_side_turn_end_power_uid += 1;
        let entry = AfterSideTurnEndPowerEntry {
            token: AfterSideTurnEndPowerToken::Monologue,
            uid,
        };
        if state.after_side_turn_end_inline.is_none() && state.cold.after_side_turn_end.is_empty() {
            state.after_side_turn_end_inline = Some(entry);
        } else if let Some(first) = state.after_side_turn_end_inline.take() {
            let cold = Arc::make_mut(&mut state.cold);
            debug_assert!(cold.after_side_turn_end.is_empty());
            cold.after_side_turn_end.extend([first, entry]);
        } else {
            Arc::make_mut(&mut state.cold)
                .after_side_turn_end
                .push(entry);
        }
        Arc::make_mut(&mut state.cold)
            .after_card_played
            .push(AfterCardPlayedPowerEntry {
                token: AfterCardPlayedPowerToken::Monologue,
                uid,
            });
        let cold = Arc::make_mut(&mut state.cold);
        cold.power_records
            .push(ColdPowerRecord::Monologue(MonologueInstance {
                uid,
                amount,
                power,
                strength_applied: 0,
            }));
        state.private_power_flags |= MONOLOGUE_HOOK_MASK;
        Ok(uid)
    }

    pub(crate) fn panache_instance(&self, uid: u32) -> Option<PanacheInstance> {
        self.panache_instances()
            .find(|instance| instance.uid == uid)
    }

    pub(crate) fn monologue_instance(&self, uid: u32) -> Option<MonologueInstance> {
        self.monologue_instances()
            .find(|instance| instance.uid == uid)
    }

    /// Native Panache 0x33fed8 decrements before Damage and resets only after
    /// its awaited command returns. Nested CardPlay callbacks can observe zero
    /// and negative counters; the synchronous invocation stack proves them.
    pub(crate) fn advance_panache_after_card_played(
        &mut self,
        uid: u32,
    ) -> Result<Option<i32>, ()> {
        if !self.instanced_player_power_records_are_exact() {
            return Err(());
        }
        let state = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut state.cold);
        let instance = cold
            .power_records
            .iter_mut()
            .find_map(|record| match record {
                ColdPowerRecord::Panache(instance) if instance.uid == uid => Some(instance),
                _ => None,
            })
            .ok_or(())?;
        let fired = if !instance.already_applied {
            instance.already_applied = true;
            None
        } else {
            instance.cards_left = instance.cards_left.checked_sub(1).ok_or(())?;
            (instance.cards_left <= 0).then_some(instance.amount)
        };
        if fired.is_some() {
            Arc::make_mut(cold.damage_execution.get_or_insert_with(Default::default))
                .panache
                .push(uid);
        }
        Self::sync_panache_mirror(state);
        Ok(fired)
    }

    pub(crate) fn finish_panache_after_damage(&mut self, uid: u32) -> Result<(), ()> {
        let state = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut state.cold);
        if cold
            .damage_execution
            .as_mut()
            .and_then(|execution| Arc::make_mut(execution).panache.pop())
            != Some(uid)
        {
            return Err(());
        }
        let instance = cold
            .power_records
            .iter_mut()
            .find_map(|record| match record {
                ColdPowerRecord::Panache(instance) if instance.uid == uid => Some(instance),
                _ => None,
            })
            .ok_or(())?;
        instance.cards_left = 5;
        Self::clear_finished_damage_execution(cold);
        Self::sync_panache_mirror(state);
        Ok(())
    }

    fn sync_panache_mirror(state: &mut FanoutState) {
        let mut counters = state
            .cold
            .power_records
            .iter()
            .filter_map(|record| match record {
                ColdPowerRecord::Panache(instance) => Some(instance.cards_left),
                _ => None,
            });
        let mirror = match (counters.next(), counters.next()) {
            (Some(left), None) => left.max(0) as u8,
            _ => 5,
        };
        state.automation_panache_tender =
            (state.automation_panache_tender & !PANACHE_LEFT_MASK) | (mirror << PANACHE_LEFT_SHIFT);
    }

    pub(crate) fn increment_monologue_strength_applied(&mut self, uid: u32) -> Result<i32, ()> {
        if !self.instanced_player_power_records_are_exact() {
            return Err(());
        }
        let instance = self.monologue_instance(uid).ok_or(())?;
        let next_instance_applied = instance
            .strength_applied
            .checked_add(instance.power)
            .ok_or(())?;
        let next_aggregate_applied = self
            .monologue_strength_applied()
            .checked_add(instance.power)
            .ok_or(())?;
        let state = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut state.cold);
        let instance = cold
            .power_records
            .iter_mut()
            .find_map(|record| match record {
                ColdPowerRecord::Monologue(instance) if instance.uid == uid => Some(instance),
                _ => None,
            })
            .ok_or(())?;
        instance.strength_applied = next_instance_applied;
        state.monologue_strength_applied = next_aggregate_applied;
        Ok(next_instance_applied)
    }

    pub(crate) fn remove_monologue_instance(&mut self, uid: u32) -> Result<MonologueInstance, ()> {
        if !self.instanced_player_power_records_are_exact() {
            return Err(());
        }
        let instance = self.monologue_instance(uid).ok_or(())?;
        let next_applied = self
            .monologue_strength_applied()
            .checked_sub(instance.strength_applied)
            .ok_or(())?;
        let ledger_index = self
            .after_side_turn_end_power_order()
            .iter()
            .position(|entry| {
                entry.token == AfterSideTurnEndPowerToken::Monologue && entry.uid == uid
            })
            .ok_or(())?;
        let ledger_is_inline = self.0.after_side_turn_end_inline.is_some();
        let after_card_played_index = self
            .after_card_played_power_order()
            .iter()
            .position(|entry| {
                entry.token == AfterCardPlayedPowerToken::Monologue && entry.uid == uid
            })
            .ok_or(())?;
        let power_record_index = self
            .0
            .cold
            .power_records
            .iter()
            .position(|record| {
                matches!(record, ColdPowerRecord::Monologue(instance) if instance.uid == uid)
            })
            .ok_or(())?;
        let state = Arc::make_mut(&mut self.0);
        if ledger_is_inline {
            debug_assert_eq!(ledger_index, 0);
            state.after_side_turn_end_inline = None;
        } else {
            let cold = Arc::make_mut(&mut state.cold);
            cold.after_side_turn_end.remove(ledger_index);
            if cold.after_side_turn_end.len() == 1 {
                state.after_side_turn_end_inline = cold.after_side_turn_end.pop();
            }
        }
        let cold = Arc::make_mut(&mut state.cold);
        cold.after_card_played.remove(after_card_played_index);
        let ColdPowerRecord::Monologue(instance) = cold.power_records.remove(power_record_index)
        else {
            unreachable!("matched Monologue record")
        };
        state.monologue_strength_applied = next_applied;
        if !cold
            .power_records
            .iter()
            .any(|record| matches!(record, ColdPowerRecord::Monologue(_)))
        {
            state.private_power_flags &= !MONOLOGUE_HOOK_MASK;
        }
        Ok(instance)
    }

    /// Allocate one native instanced Bomb and register its listener. The
    /// private damage payload is deliberately left at zero until the awaited
    /// Apply continuation completes; callers must immediately finish the
    /// transaction with [`Self::set_the_bomb_damage`].
    pub(crate) fn begin_the_bomb(&mut self, turns: i32) -> Result<u32, ()> {
        if !self.the_bomb_state_is_exact()
            || turns != 3
            || self.0.cold.next_the_bomb_uid == u32::MAX
        {
            return Err(());
        }
        let state = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut state.cold);
        let uid = cold.next_the_bomb_uid;
        cold.next_the_bomb_uid += 1;
        let insertion = cold
            .power_records
            .iter()
            .position(|record| !matches!(record, ColdPowerRecord::TheBomb(_)))
            .unwrap_or(cold.power_records.len());
        cold.power_records.insert(
            insertion,
            ColdPowerRecord::TheBomb(TheBombInstance {
                uid,
                turns,
                damage: 0,
            }),
        );
        cold.before_side_turn_end
            .push(BeforeSideTurnEndToken::TheBomb(uid));
        Ok(uid)
    }

    /// Complete The Bomb's post-Apply private payload write.
    pub(crate) fn set_the_bomb_damage(&mut self, uid: u32, damage: i32) -> Result<(), ()> {
        if !matches!(damage, 40 | 50) {
            return Err(());
        }
        let state = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut state.cold);
        let instance = cold
            .power_records
            .iter_mut()
            .find_map(|record| match record {
                ColdPowerRecord::TheBomb(instance) if instance.uid == uid => Some(instance),
                ColdPowerRecord::TheBomb(_)
                | ColdPowerRecord::ToricToughness(_)
                | ColdPowerRecord::Sloth(_)
                | ColdPowerRecord::Panache(_)
                | ColdPowerRecord::Monologue(_)
                | ColdPowerRecord::Automation(_) => None,
            })
            .ok_or(())?;
        if instance.damage != 0 || instance.turns != 3 {
            return Err(());
        }
        instance.damage = damage;
        if self.the_bomb_state_is_exact() {
            Ok(())
        } else {
            Err(())
        }
    }

    pub(crate) fn decrement_the_bomb(&mut self, uid: u32) -> Result<i32, ()> {
        if !self.the_bomb_state_is_exact() {
            return Err(());
        }
        let state = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut state.cold);
        let instance = cold
            .power_records
            .iter_mut()
            .find_map(|record| match record {
                ColdPowerRecord::TheBomb(instance) if instance.uid == uid => Some(instance),
                ColdPowerRecord::TheBomb(_)
                | ColdPowerRecord::ToricToughness(_)
                | ColdPowerRecord::Sloth(_)
                | ColdPowerRecord::Panache(_)
                | ColdPowerRecord::Monologue(_)
                | ColdPowerRecord::Automation(_) => None,
            })
            .ok_or(())?;
        if instance.turns <= 1 {
            return Err(());
        }
        instance.turns -= 1;
        Ok(instance.turns)
    }

    pub(crate) fn remove_the_bomb(&mut self, uid: u32) -> Result<TheBombInstance, ()> {
        if !self.the_bomb_state_is_exact() {
            return Err(());
        }
        let state = Arc::make_mut(&mut self.0);
        let cold = Arc::make_mut(&mut state.cold);
        let index = cold
            .power_records
            .iter()
            .position(|record| {
                matches!(record, ColdPowerRecord::TheBomb(instance) if instance.uid == uid)
            })
            .ok_or(())?;
        let token = cold
            .before_side_turn_end
            .iter()
            .position(|token| *token == BeforeSideTurnEndToken::TheBomb(uid))
            .ok_or(())?;
        cold.before_side_turn_end.remove(token);
        match cold.power_records.remove(index) {
            ColdPowerRecord::TheBomb(instance) => Ok(instance),
            ColdPowerRecord::ToricToughness(_)
            | ColdPowerRecord::Sloth(_)
            | ColdPowerRecord::Panache(_)
            | ColdPowerRecord::Monologue(_)
            | ColdPowerRecord::Automation(_) => Err(()),
        }
    }

    pub(crate) fn multiplayer_ally(&self) -> &MultiplayerAllyState {
        self.0
            .multiplayer_ally
            .as_ref()
            .unwrap_or(&DEFAULT_MULTIPLAYER_ALLY)
    }

    pub(crate) fn set_multiplayer_ally(&mut self, mut ally: MultiplayerAllyState) -> bool {
        let prior_hammer_time = self
            .0
            .multiplayer_ally
            .as_ref()
            .is_some_and(|current| current.hammer_time);
        let prior_pending = self.0.multiplayer_ally.as_ref().and_then(|current| {
            current
                .teammate_power_pending
                .map(|source| (current.key, source))
        });
        if ally
            .teammate_power_pending
            .is_some_and(|source| prior_pending.is_some_and(|prior| prior.1 != source))
        {
            return false;
        }
        if let Some((key, source)) = prior_pending {
            if key != ally.key {
                return false;
            }
            ally.teammate_power_pending = Some(source);
        }
        ally.hammer_time |= prior_hammer_time;
        Arc::make_mut(&mut self.0).multiplayer_ally = Some(ally);
        true
    }

    pub(crate) fn hammer_time(&self) -> bool {
        self.0
            .multiplayer_ally
            .as_ref()
            .is_some_and(|ally| ally.hammer_time)
    }

    pub(crate) fn set_hammer_time(&mut self, active: bool) {
        self.multiplayer_ally_mut().hammer_time = active;
    }

    pub(crate) fn multiplayer_ally_mut(&mut self) -> &mut MultiplayerAllyState {
        Arc::make_mut(&mut self.0)
            .multiplayer_ally
            .get_or_insert_with(MultiplayerAllyState::default)
    }

    pub(crate) fn imitation_learning(&self, key: u32) -> Option<i32> {
        usize::try_from(key)
            .ok()
            .filter(|index| *index < 2)
            .map(|index| self.0.imitation_learning[index])
    }

    pub(crate) fn imitation_learning_order(&self) -> [Option<u8>; 2] {
        unpack_two_key_order(
            (self.0.intercept_covered_state & IMITATION_LEARNING_ORDER_MASK)
                >> IMITATION_LEARNING_ORDER_SHIFT,
        )
    }

    pub(crate) fn set_imitation_learning(&mut self, key: u32, amount: i32) -> bool {
        let Ok(index) = usize::try_from(key) else {
            return false;
        };
        if index >= 2 || amount < 0 {
            return false;
        }
        let state = Arc::make_mut(&mut self.0);
        let prior = state.imitation_learning[index];
        let prior_order = (state.intercept_covered_state & IMITATION_LEARNING_ORDER_MASK)
            >> IMITATION_LEARNING_ORDER_SHIFT;
        if prior == 0 && amount > 0 {
            let Some(order) = append_two_key_order(prior_order, key as u8) else {
                return false;
            };
            state.intercept_covered_state = (state.intercept_covered_state
                & !IMITATION_LEARNING_ORDER_MASK)
                | (order << IMITATION_LEARNING_ORDER_SHIFT);
        } else if prior > 0 && amount == 0 {
            let Some(order) = remove_two_key_order(prior_order, key as u8) else {
                return false;
            };
            state.intercept_covered_state = (state.intercept_covered_state
                & !IMITATION_LEARNING_ORDER_MASK)
                | (order << IMITATION_LEARNING_ORDER_SHIFT);
        }
        state.imitation_learning[index] = amount;
        true
    }

    pub(crate) fn imitation_clones(&self) -> &[ImitationClone] {
        self.0.imitation_clones.as_slice()
    }

    pub(crate) fn imitation_clones_mut(&mut self) -> &mut Vec<ImitationClone> {
        Arc::make_mut(&mut Arc::make_mut(&mut self.0).imitation_clones)
    }

    #[inline]
    pub(crate) fn teammate_power_pending(&self, multiplayer_ally_key: u8) -> Option<(u32, u32)> {
        if multiplayer_ally_key == 0 {
            return None;
        }
        self.0
            .multiplayer_ally
            .as_ref()
            .and_then(|ally| ally.teammate_power_pending.map(|source| (ally.key, source)))
    }

    pub(crate) fn teammate_power_pending_raw(&self) -> Option<(u32, u32)> {
        self.0
            .multiplayer_ally
            .as_ref()
            .and_then(|ally| ally.teammate_power_pending.map(|source| (ally.key, source)))
    }

    pub(crate) fn set_teammate_power_pending(&mut self, pending: Option<(u32, u32)>) -> bool {
        let Some(ally) = self.0.multiplayer_ally.as_ref() else {
            return pending.is_none();
        };
        if pending.is_some_and(|pending| pending.0 != ally.key) {
            return false;
        }
        Arc::make_mut(&mut self.0)
            .multiplayer_ally
            .as_mut()
            .expect("validated represented ally")
            .teammate_power_pending = pending.map(|(_, source)| source);
        true
    }

    #[inline]
    pub(crate) fn constrict_amount(&self) -> i32 {
        self.0.constrict_amount
    }

    pub(crate) fn set_constrict_amount(&mut self, amount: i32) -> bool {
        if amount < 0 || amount % 3 != 0 {
            return false;
        }
        Arc::make_mut(&mut self.0).constrict_amount = amount;
        true
    }

    #[cfg(test)]
    pub(crate) fn forge_constrict_amount(&mut self, amount: i32) {
        Arc::make_mut(&mut self.0).constrict_amount = amount;
    }

    pub(crate) fn intercept_covered(&self, key: u32) -> bool {
        key < 2 && self.0.intercept_covered_state & (1 << key) != 0
    }

    pub(crate) fn intercept_covered_mask(&self) -> u8 {
        self.0.intercept_covered_state & INTERCEPT_COVERED_MASK
    }

    pub(crate) fn intercept_covered_order(&self) -> [Option<u8>; 2] {
        unpack_two_key_order(
            (self.0.intercept_covered_state & INTERCEPT_COVERED_ORDER_MASK)
                >> INTERCEPT_COVERED_ORDER_SHIFT,
        )
    }

    pub(crate) fn cover_with_intercept(&mut self, key: u32) -> bool {
        if key >= 2 {
            return false;
        }
        let state = Arc::make_mut(&mut self.0);
        let bit = 1 << key;
        if state.intercept_covered_state & bit != 0 {
            return true;
        }
        let prior_order = (state.intercept_covered_state & INTERCEPT_COVERED_ORDER_MASK)
            >> INTERCEPT_COVERED_ORDER_SHIFT;
        let Some(order) = append_two_key_order(prior_order, key as u8) else {
            return false;
        };
        state.intercept_covered_state = (state.intercept_covered_state
            & !(INTERCEPT_COVERED_MASK | INTERCEPT_COVERED_ORDER_MASK))
            | ((state.intercept_covered_state & INTERCEPT_COVERED_MASK) | bit)
            | (order << INTERCEPT_COVERED_ORDER_SHIFT);
        true
    }

    pub(crate) fn set_intercept_covered_mask(&mut self, mask: u8) -> bool {
        if mask & !0b11 != 0 {
            return false;
        }
        let state = Arc::make_mut(&mut self.0);
        let mut next_order = 0;
        let prior_order = (state.intercept_covered_state & INTERCEPT_COVERED_ORDER_MASK)
            >> INTERCEPT_COVERED_ORDER_SHIFT;
        for key in unpack_two_key_order(prior_order)
            .into_iter()
            .flatten()
            .chain([0, 1])
        {
            let bit = 1 << key;
            if mask & bit != 0 {
                next_order = append_two_key_order(next_order, key).unwrap_or(next_order);
            }
        }
        state.intercept_covered_state = (state.intercept_covered_state
            & IMITATION_LEARNING_ORDER_MASK)
            | mask
            | (next_order << INTERCEPT_COVERED_ORDER_SHIFT);
        true
    }
}

/// Stack-owned decoded view of the compact result-location listener order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResultLocationPowerOrder {
    values: [PowerId; MAX_RESULT_LOCATION_POWERS],
    len: u8,
}

impl std::ops::Deref for ResultLocationPowerOrder {
    type Target = [PowerId];

    fn deref(&self) -> &Self::Target {
        &self.values[..usize::from(self.len)]
    }
}

impl<const N: usize> PartialEq<[PowerId; N]> for ResultLocationPowerOrder {
    fn eq(&self, other: &[PowerId; N]) -> bool {
        &self.values[..usize::from(self.len)] == other
    }
}

impl<const N: usize> PartialEq<&[PowerId; N]> for ResultLocationPowerOrder {
    fn eq(&self, other: &&[PowerId; N]) -> bool {
        &self.values[..usize::from(self.len)] == *other
    }
}

fn unpack_two_key_order(order: u8) -> [Option<u8>; 2] {
    match order {
        0 => [None, None],
        1 => [Some(0), None],
        2 => [Some(1), None],
        3 => [Some(0), Some(1)],
        4 => [Some(1), Some(0)],
        _ => unreachable!("two-key order is written only by checked helpers"),
    }
}

fn two_key_order_mask(order: u8) -> Option<u8> {
    match order {
        0 => Some(0),
        1 => Some(0b01),
        2 => Some(0b10),
        3 | 4 => Some(0b11),
        _ => None,
    }
}

fn append_two_key_order(order: u8, key: u8) -> Option<u8> {
    match (order, key) {
        (0, 0) => Some(1),
        (0, 1) => Some(2),
        (1, 1) => Some(3),
        (2, 0) => Some(4),
        (1, 0) | (3..=4, 0) | (2..=4, 1) => Some(order),
        _ => None,
    }
}

fn remove_two_key_order(order: u8, key: u8) -> Option<u8> {
    match (order, key) {
        (1, 0) | (2, 1) => Some(0),
        (3, 0) | (4, 0) => Some(2),
        (3, 1) | (4, 1) => Some(1),
        _ => None,
    }
}

fn set_fanout_order<const N: usize>(
    state: &mut Arc<FanoutState>,
    order: &[PowerId],
    maximum: usize,
    select: impl FnOnce(&mut FanoutState) -> (&mut [PowerId; N], &mut u8),
) -> bool {
    if order.len() > maximum || order.len() > N {
        return false;
    }
    let inner = Arc::make_mut(state);
    let (slots, len) = select(inner);
    slots[..order.len()].copy_from_slice(order);
    *len = order.len() as u8;
    true
}

fn register_fanout_power<const N: usize>(
    state: &mut Arc<FanoutState>,
    power: PowerId,
    select: impl FnOnce(&mut FanoutState) -> (&mut [PowerId; N], &mut u8),
) -> bool {
    let inner = Arc::make_mut(state);
    let (slots, len) = select(inner);
    let Some(next_len) = append_fanout_power(slots, *len, power) else {
        return false;
    };
    *len = next_len;
    true
}

fn append_fanout_power<const N: usize>(
    slots: &mut [PowerId; N],
    len: u8,
    power: PowerId,
) -> Option<u8> {
    let occupied = usize::from(len);
    if slots[..occupied].contains(&power) {
        return Some(len);
    }
    if occupied == N {
        return None;
    }
    slots[occupied] = power;
    Some(len + 1)
}

/// `OrbCmd::AddSlots` caps every live queue at ten slots.
pub const MAX_ORB_SLOTS: u8 = 10;

macro_rules! wire_enum {
    // Per-variant attributes are optional, so every pre-#2693 call site that
    // passes none expands exactly as before. They exist so a vocabulary whose
    // members each carry their own native citation — see
    // [`AttachedPowerModel`] — can put the RVAs on the variant they belong
    // to rather than piling them into the enum's own doc block.
    ($(#[$meta:meta])* $name:ident {
        $($(#[$vmeta:meta])* ($variant:ident, $wire:literal)),+ $(,)?
    }) => {
        $(#[$meta])*
        #[repr(u8)]
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
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
    /// The five ordered card piles.
    ///
    /// `tools/project_state.py::PILE_FIELDS`. Discriminants follow the
    /// ascending-name convention of the generated ids, so a pile doubles as
    /// an index into [`HotPiles`].
    PileId {
        (Discard, "discard"),
        (Draw, "draw"),
        (Exhaust, "exhaust"),
        (Hand, "hand"),
        (Play, "play"),
    }
}

wire_enum! {
    /// The five v0.111.0 orb kinds (`combat_sim._VALID_ORBS`).
    ///
    /// The canonical names are kept in ascending order so boundary parsing is
    /// a binary search. Native random-orb pool order is a separate generated
    /// content concern and must never be inferred from these discriminants.
    OrbKind {
        (Dark, "DARK"),
        (Frost, "FROST"),
        (Glass, "GLASS"),
        (Lightning, "LIGHTNING"),
        (Plasma, "PLASMA"),
    }
}

impl OrbKind {
    /// Whether this kind carries mutable per-instance state on the wire.
    pub const fn carries_amount(self) -> bool {
        matches!(self, Self::Dark | Self::Glass)
    }
}

/// One acquisition-ordered player power that channels at `AfterEnergyReset`.
///
/// Lightning Rod and Spinner share the same native listener walk. Their
/// order is observable when the queue is full, so the hot orb block carries
/// the exact application order instead of imposing enum order.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum OrbResetPower {
    /// `LightningRodPower`.
    LightningRod,
    /// `SpinnerPower`.
    Spinner,
}

/// One ordered orb-queue member, in eight bytes.
///
/// Dark carries its raw evoke accumulator and Glass its raw passive amount.
/// Lightning, Frost, and Plasma carry canonical `null`; their stored integer
/// is therefore zero and never exposed as an amount. The kind itself is the
/// exact tag, so no extra option discriminant is needed in the hot loop.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct HotOrb {
    amount: i32,
    kind: OrbKind,
}

impl HotOrb {
    /// Decode one canonical `(kind, amount)` pair.
    pub fn from_parts(kind: OrbKind, amount: Option<i32>) -> Option<Self> {
        match (kind.carries_amount(), amount) {
            (true, Some(amount)) if amount >= 0 => Some(Self { amount, kind }),
            (false, None) => Some(Self { amount: 0, kind }),
            _ => None,
        }
    }

    /// The orb's interned kind.
    pub const fn kind(self) -> OrbKind {
        self.kind
    }

    /// Dark/Glass's mutable raw amount, or `None` for the pure-value kinds.
    pub const fn amount(self) -> Option<i32> {
        if self.kind.carries_amount() {
            Some(self.amount)
        } else {
            None
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct OrbQueueState {
    orbs: Vec<HotOrb>,
    reset_order: Vec<OrbResetPower>,
    next_random_orb_progress_uid: u32,
    next_consuming_shadow_side_end_auth_uid: u32,
    temp_focus: i32,
    lightning_channeled: i32,
    orbit_energy_spent: i32,
    orbit_trigger_count: i32,
    base_slots: u8,
    slots: u8,
}

impl Default for OrbQueueState {
    fn default() -> Self {
        Self {
            orbs: Vec::new(),
            reset_order: Vec::new(),
            next_random_orb_progress_uid: 0,
            next_consuming_shadow_side_end_auth_uid: 0,
            temp_focus: 0,
            lightning_channeled: -1,
            orbit_energy_spent: 0,
            orbit_trigger_count: 0,
            base_slots: 0,
            slots: 0,
        }
    }
}

/// The live slot counts, ordered orb queue, and temporary Focus, copy-on-write.
///
/// The queue is one pointer in [`HotState`]. Capacity writes and per-orb
/// passive updates clone only this block; ordinary state clones pay one
/// atomic increment and never walk the ordered list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct HotOrbs(Arc<OrbQueueState>);

impl HotOrbs {
    /// Empty non-Defect defaults: no base/live slots and no queue members.
    pub fn new() -> Self {
        Self::default()
    }

    /// The character-derived immutable base slot count.
    pub fn base_slots(&self) -> u8 {
        self.0.base_slots
    }

    /// The live queue capacity.
    pub fn slots(&self) -> u8 {
        self.0.slots
    }

    /// Queue members in native leftmost-to-rightmost order.
    pub fn as_slice(&self) -> &[HotOrb] {
        self.0.orbs.as_slice()
    }

    /// `State.temp_focus`, the live TemporaryFocusPower contribution.
    pub fn temp_focus(&self) -> i32 {
        self.0.temp_focus
    }

    /// Voltaic's combat-wide Lightning-channel count, or `-1` while absent.
    pub fn lightning_channeled(&self) -> i32 {
        self.0.lightning_channeled
    }

    /// OrbitPower's lifetime energy-spent data counter.
    pub fn orbit_energy_spent(&self) -> i32 {
        self.0.orbit_energy_spent
    }

    /// OrbitPower's already-paid four-energy trigger counter.
    pub fn orbit_trigger_count(&self) -> i32 {
        self.0.orbit_trigger_count
    }

    /// Canonical allocator for authenticated RandomOrb iteration receipts.
    pub fn next_random_orb_progress_uid(&self) -> u32 {
        self.0.next_random_orb_progress_uid
    }

    /// Canonical allocator for Consuming Shadow side-end dispatcher epochs.
    pub fn next_consuming_shadow_side_end_auth_uid(&self) -> u32 {
        self.0.next_consuming_shadow_side_end_auth_uid
    }

    /// Native application order for the two orb `AfterEnergyReset` powers.
    pub(crate) fn reset_order(&self) -> &[OrbResetPower] {
        self.0.reset_order.as_slice()
    }

    /// Whether every canonical orb field is at its dataclass default.
    pub fn is_vacant(&self) -> bool {
        self.0.base_slots == 0
            && self.0.slots == 0
            && self.0.orbs.is_empty()
            && self.0.temp_focus == 0
            && self.0.lightning_channeled == -1
            && self.0.orbit_energy_spent == 0
            && self.0.orbit_trigger_count == 0
            && self.0.next_random_orb_progress_uid == 0
            && self.0.next_consuming_shadow_side_end_auth_uid == 0
            && self.0.reset_order.is_empty()
    }

    /// Boundary construction: replace the immutable base-slot projection.
    pub(crate) fn set_base_slots(&mut self, slots: u8) {
        Arc::make_mut(&mut self.0).base_slots = slots;
    }

    /// Boundary construction: replace the live capacity projection.
    pub(crate) fn set_slots(&mut self, slots: u8) {
        Arc::make_mut(&mut self.0).slots = slots;
    }

    /// Boundary construction: replace the complete ordered queue.
    pub(crate) fn set_orbs(&mut self, orbs: Vec<HotOrb>) {
        Arc::make_mut(&mut self.0).orbs = orbs;
    }

    /// Write the net TemporaryFocusPower contribution.
    pub(crate) fn set_temp_focus(&mut self, amount: i32) {
        Arc::make_mut(&mut self.0).temp_focus = amount;
    }

    /// Boundary/history write for Voltaic's exact sentinel-backed counter.
    pub(crate) fn set_lightning_channeled(&mut self, amount: i32) {
        Arc::make_mut(&mut self.0).lightning_channeled = amount;
    }

    /// Replace OrbitPower's two private Data counters atomically.
    pub(crate) fn set_orbit_counters(&mut self, energy_spent: i32, trigger_count: i32) {
        let state = Arc::make_mut(&mut self.0);
        state.orbit_energy_spent = energy_spent;
        state.orbit_trigger_count = trigger_count;
    }

    /// Replace the two canonical orb-lifecycle allocators.
    pub(crate) fn set_lifecycle_counters(
        &mut self,
        random_orb: u32,
        consuming_shadow_side_end: u32,
    ) {
        let state = Arc::make_mut(&mut self.0);
        state.next_random_orb_progress_uid = random_orb;
        state.next_consuming_shadow_side_end_auth_uid = consuming_shadow_side_end;
    }

    /// Allocate one authenticated RandomOrb iteration identity.
    pub(crate) fn allocate_random_orb_progress_uid(&mut self) -> Option<u32> {
        let state = Arc::make_mut(&mut self.0);
        let uid = state.next_random_orb_progress_uid;
        state.next_random_orb_progress_uid = uid.checked_add(1)?;
        Some(uid)
    }

    /// Allocate one Consuming Shadow side-end dispatcher identity.
    pub(crate) fn allocate_consuming_shadow_side_end_auth_uid(&mut self) -> Option<u32> {
        let state = Arc::make_mut(&mut self.0);
        let uid = state.next_consuming_shadow_side_end_auth_uid;
        state.next_consuming_shadow_side_end_auth_uid = uid.checked_add(1)?;
        Some(uid)
    }

    /// Boundary construction: replace the complete reset-listener order.
    pub(crate) fn set_reset_order(&mut self, order: Vec<OrbResetPower>) {
        Arc::make_mut(&mut self.0).reset_order = order;
    }

    /// Register a newly created reset listener without moving a re-stack.
    pub(crate) fn register_reset_power(&mut self, power: OrbResetPower) {
        if !self.0.reset_order.contains(&power) {
            Arc::make_mut(&mut self.0).reset_order.push(power);
        }
    }

    /// Remove a depleted duration listener from the acquisition walk.
    pub(crate) fn unregister_reset_power(&mut self, power: OrbResetPower) {
        Arc::make_mut(&mut self.0)
            .reset_order
            .retain(|candidate| *candidate != power);
    }

    /// Append one orb at the native right edge of the queue.
    pub(crate) fn push(&mut self, orb: HotOrb) {
        Arc::make_mut(&mut self.0).orbs.push(orb);
    }

    /// Native OrbQueue.Clear0x1184de clears members and live Capacity only.
    /// No evoke/remove callbacks; immutable base capacity and counters survive.
    pub(crate) fn clear_on_owner_death(&mut self) {
        let queue = Arc::make_mut(&mut self.0);
        queue.orbs.clear();
        queue.slots = 0;
    }

    /// Remove one live queue member by its frozen native position.
    pub(crate) fn remove(&mut self, index: usize) -> Option<HotOrb> {
        if index >= self.0.orbs.len() {
            return None;
        }
        Some(Arc::make_mut(&mut self.0).orbs.remove(index))
    }

    /// Replace one live member after a mutable passive finishes.
    pub(crate) fn replace(&mut self, index: usize, orb: HotOrb) -> bool {
        if index >= self.0.orbs.len() {
            return false;
        }
        Arc::make_mut(&mut self.0).orbs[index] = orb;
        true
    }
}

wire_enum! {
    /// The nine xoshiro streams.
    ///
    /// `tools/project_state.py::RNG_FIELDS`. Each stream's live words and
    /// counter live in the copy-on-write [`HotRng`] block below; a stream
    /// doubles as an index into it.
    RngStream {
        (Ai, "ai"),
        (CombatOrbs, "combat_orbs"),
        (EnergyCosts, "energy_costs"),
        (Generation, "generation"),
        (Niche, "niche"),
        (PotionGeneration, "potion_generation"),
        (Rng, "rng"),
        (Sel, "sel"),
        (Targets, "targets"),
    }
}

const _: () = assert!(RngStream::COUNT == RNG_STREAM_COUNT);

wire_enum! {
    /// Closed vocabulary for the admitted forced monster moves.
    ///
    /// Every member is a key of the registry `_validate_monster_stun_request` (frozen Python, deleted #2827) checks, whose bodies are all `("none", ())` — the dynamic state
    /// consumes one enemy action and then hands control to
    /// `forced_follow_up`. #2481 slot 3 adds Corpse Slug's Ravenous stun.
    /// Beetle SNORE/WAKE instead cover its intrinsic sleep counter and
    /// deferred wake body; both return to its single compiled Rollout row.
    MonsterOverride {
        (None, ""),
        (Dizzy, "DIZZY"),
        (RavenousStun, "RAVENOUS_STUN_MOVE"),
        (BeetleSnore, "SNORE"),
        (Stunned, "STUNNED"),
        (BeastStun, "STUN_MOVE"),
        (Terror, "TERROR"),
        (BeetleWake, "WAKE"),
        (LagavulinWakeUp, "WAKE_UP_MOVE"),
    }
}

wire_enum! {
    /// Closed vocabulary for dynamic forced-move successors.
    ///
    /// A follow-up is the ordinary MoveState the stun interrupted, so every
    /// member must be a move name in its owner's generated loop
    /// (`_monster_loop_state_index`, frozen Python, deleted #2827). #2481 slot 3 adds Corpse Slug's
    /// three telegraphs: Ravenous stuns every surviving sibling and parks
    /// whichever move that sibling was already showing, so all three are
    /// reachable.
    ///
    /// #2647 slice A adds the generic parked telegraph
    /// (`crate::engine::monsters::parked_telegraph`), which resolves a member
    /// by looking the owner's `loop_pos` row up in the generated loop rather
    /// than from a per-kind table. The four Bowlbug-roster names below are the
    /// telegraphs that grows reachable: `build_bowlbugs_weak` puts one of
    /// `BowlbugEgg`/`BowlbugNectar` beside the Rock, and Whistle
    /// (`CreatureCmd::Stun(target, null)`) can park either bug's current move.
    /// `every_monster_follow_up_name_exists_in_a_generated_loop` pins that no
    /// member names a move the generated tables do not carry.
    MonsterFollowUp {
        (None, ""),
        (AmalgamBeam, "BEAM_MOVE"),
        (BeastCry, "BEAST_CRY_MOVE"),
        (EggBite, "BITE"),
        (NectarBuff, "BUFF"),
        (QueenBurnBright, "BURN_BRIGHT_FOR_ME_MOVE"),
        (QueenEnrage, "ENRAGE_MOVE"),
        (HopperEscape, "ESCAPE_MOVE"),
        (QueenExecution, "EXECUTION_MOVE"),
        (SlugGlomp, "GLOMP_MOVE"),
        (SlugGoop, "GOOP_MOVE"),
        // Living Shield's completed SHIELD_SLAM defers its conditional
        // successor until PrepareForNextTurn, after its later Operator peer
        // has acted. This is a receipt, not an interrupting forced move.
        (LivingShieldSlam, "LIVING_SHIELD_SLAM_RECEIPT"),
        (HopperNab, "NAB_MOVE"),
        (QueenOffWithYourHead, "OFF_WITH_YOUR_HEAD_MOVE"),
        (QueenPuppetStrings, "PUPPET_STRINGS_MOVE"),
        // #2647 — an awake Slumbering Beetle's Imbalanced self-stun parks
        // ROLL_OUT_MOVE (`crate::engine::monsters::beetle_rollout_stun_is_exact`).
        // The row is the catalog-private `BEETLE_ROLLOUT`, not a generated
        // loop row, so no generic owner can resolve this name.
        (BeetleRollOut, "ROLL_OUT_MOVE"),
        (LagavulinSlash, "SLASH_MOVE"),
        (AmalgamStrongTackle, "STRONG_TACKLE_MOVE"),
        (AmalgamTackleTwo, "TACKLE_2_MOVE"),
        (AmalgamTackleThree, "TACKLE_3_MOVE"),
        (AmalgamTackleFour, "TACKLE_4_MOVE"),
        (NectarThrash, "THRASH"),
        (NectarThrashTwo, "THRASH2"),
        // #2647 — Bowlbug Silk's two rows (`BowlbugSilk::
        // GenerateMoveStateMachine` `0xb0044` IL_0012-IL_007b: two plain
        // `MoveState`s, each the other's `FollowUpState`, no branch state).
        // A `Misery` clone of `ImbalancedPower` can land on the Silk beside a
        // Bowlbug Rock (`BOWLBUGS_NORMAL`, `SLUMBERING_BEETLE_NORMAL`), and
        // its THRASH is a two-hit attack, so either row can be parked.
        (SilkThrash, "THRASH_MOVE"),
        (SilkToxicSpit, "TOXIC_SPIT_MOVE"),
        (SlugWhipSlap, "WHIP_SLAP_MOVE"),
        (QueenYoureMine, "YOU_ARE_MINE_MOVE"),
    }
}

wire_enum! {
    /// One entry of a monster's `misery_debuff_order` acquisition list.
    ///
    /// Vocabulary: `_validated_misery_debuff_order_projection` checks
    /// `combat_sim._MISERY_DEBUFF_TOKENS` (frozen Python, deleted #2827) — the twelve
    /// scalar debuffs plus the three multi-instance ones. It is a closed
    /// vocabulary, not an id axis, so it lives here rather than in the
    /// generated ids: `Misery` copies a target's debuffs in the order they
    /// were acquired, and that order is observable state.
    MiseryToken {
        (Conqueror, "conqueror"),
        (Debilitate, "debilitate"),
        (Demise, "demise"),
        (Doom, "doom"),
        (Flanking, "flanking"),
        (Hang, "hang"),
        (Knockdown, "knockdown"),
        (Oblivion, "oblivion"),
        (Poison, "poison"),
        (Shrink, "shrink"),
        (SicEm, "sic_em"),
        (Strangle, "strangle"),
        (TagTeam, "tag_team"),
        (Vuln, "vuln"),
        (Weak, "weak"),
    }
}

wire_enum! {
    /// The native `PowerModel` classes a physical attachment can name.
    ///
    /// Current-build authority is `sts2.dll` SHA-256
    /// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
    ///
    /// **Why this is not [`PowerId`].** `Creature::_powers`
    /// (`Creature::get_Powers` `0x11d8ae`) holds native `PowerModel`
    /// *instances*, so an attachment's identity is the model class. `PowerId`
    /// is a different axis: `tools/generate_content.py::power_ids` generates
    /// it from `combat_sim.MODELED_POWER_FIELDS`, a census of Python
    /// `State`/`Monster` **dataclass field names** — `tools/dll_content.py`'s
    /// `MODELING_ANNEX` says so in terms ("216 modeled fields against 283
    /// POWER entries, and the mapping is not one-to-one"). `ImbalancedPower`
    /// has no Python field at all: frozen `count_target_powers_for_rend` (frozen Python, deleted #2827) counts Bowlbug Rock's instance *by kind*. So the model
    /// vocabulary cannot be borrowed from the generated ids, and minting it
    /// there would mean editing a frozen `combat_sim.py` (#1282 D1-D4) to
    /// name a field Python does not have.
    ///
    /// It is therefore a closed hand-authored vocabulary like [`MiseryToken`]
    /// and [`MonsterFollowUp`], declared in ascending wire order so boundary
    /// parsing stays a binary search.
    ///
    /// **Closed on purpose.** A variant is added only with a consumer. Every
    /// member below takes the base `PowerModel::get_InstanceType` `0x83751`
    /// value `0` — no class here overrides it, checked against all 24
    /// `get_InstanceType` bodies in the assembly — so all are *singletons*
    /// per creature and `PowerCmd::FindExistingInstanceForStacking`
    /// `0x1338d8` IL_0058 resolves them by id alone, never by applier. The
    /// applier is carried regardless, because it is written once at fresh
    /// attach and survives cloning (see [`AttachmentRecord`]).
    ///
    /// **The temporary-Strength wrapper family (#2693 S2).** Eight of the
    /// members are concrete `TemporaryStrengthPower` subclasses. They are
    /// *derived*, not transcribed: the assembly's TypeDef table has exactly
    /// fourteen direct subclasses of `TemporaryStrengthPower` and no deeper
    /// ones, and `TemporaryStrengthPower::get_Type` `0xa9ac3` returns 2 iff
    /// `get_IsPositive` is false. Five of the fourteen inherit the base
    /// `get_IsPositive` `0xa9ada` (= true) and are therefore Type 1, outside
    /// `Misery`'s filter and player-side besides — `CoordinatePower`,
    /// `FeedingFrenzyPower`, `FlexPotionPower`, `ReptileTrinketPower`,
    /// `SetupStrikePower`. A sixth, `Mocks.MockTemporaryStrengthLossPower`
    /// `0xab90c`, overrides it to false but a whole-assembly `MethodSpec`
    /// census finds **no** `Apply` instantiation over it anywhere in the
    /// shipping assembly. The remaining eight each have exactly one native
    /// application site, cited on the variant. See
    /// [`AttachedPowerModel::is_temporary_strength_wrapper`] for what the
    /// family shares.
    ///
    /// Player-side temporary Strength is deliberately **not** on the ledger:
    /// `Misery` targets an enemy (`Misery/<OnPlay>d__3::MoveNext` `0x3ad358`
    /// IL_0029-IL_0039 requires `cardPlay.Target`), so a player row would be
    /// an unwitnessed field.
    AttachedPowerModel {
        /// `CrushUnderPower` (`get_IsPositive` `0xa101b` = false). Applied by
        /// `CrushUnder/<OnPlay>d__6::MoveNext` `0x395cac` IL_01d5, once per
        /// surviving target of the card's all-enemy attack.
        (CrushUnder, "crush_under"),
        /// `DarkShacklesPower` (`get_IsPositive` `0xa147a` = false). Applied
        /// by `DarkShackles/<OnPlay>d__8::MoveNext` `0x396954` IL_00b3-IL_00e6
        /// with `DynamicVars["StrengthLoss"].BaseValue` and
        /// `card.Owner.Player.Creature` as the applier.
        (DarkShackles, "dark_shackles"),
        /// `DyingStarPower` (`get_IsPositive` `0xa1e92` = false). Applied by
        /// `DyingStar/<OnPlay>d__10::MoveNext` `0x39ac44` IL_01af.
        (DyingStar, "dying_star"),
        /// `EnfeeblingTouchPower` (`get_IsPositive` `0xa20b2` = false).
        /// Applied by `EnfeeblingTouch/<OnPlay>d__8::MoveNext` `0x39bb64`
        /// IL_00e6.
        (EnfeeblingTouch, "enfeebling_touch"),
        /// `ImbalancedPower`. `get_Type` `0xa3bd3` returns 2 and
        /// `get_StackType` `0xa3bd6` returns 2, so it is a concrete Type-2
        /// power that `Misery`'s `TypeForCurrentAmount == 2` filter
        /// (`Misery/<>c::<OnPlay>b__3_0` `0x3ad30a`) always selects, and it
        /// carries no scalar amount of its own.
        ///
        /// A whole-assembly IL census (every `MethodDef` body, operands
        /// token-resolved) finds exactly one writer:
        /// `BowlbugRock/<AfterAddedToRoom>d__22::MoveNext` `0x353e30`
        /// IL_007f-IL_0097 calls `PowerCmd.Apply<ImbalancedPower>` with
        /// `System.Decimal::One` as the amount and the Rock's **own**
        /// `Creature` as the applier. There is no `ModifyAmount` site and no
        /// second `Apply<ImbalancedPower>` anywhere in the assembly, so
        /// amount 1 is the only value any native path can produce until
        /// `Misery` clones one onto a creature that already carries it.
        (Imbalanced, "imbalanced"),
        /// `ManglePower` (`get_IsPositive` `0xa491a` = false). Applied by
        /// `Mangle/<OnPlay>d__6::MoveNext` `0x3ab670` IL_013c.
        (Mangle, "mangle"),
        /// `MonarchsGazeStrengthDownPower` (`get_IsPositive` `0xa4ad4` =
        /// false). The one member applied by a *player power* rather than a
        /// card or potion play: `MonarchsGazePower/<AfterDamageGiven>d__4::
        /// MoveNext` `0x33e544` IL_0042-IL_0061 passes the listener's own
        /// `PowerModel::get_Owner` as the applier, which for a player-owned
        /// power is the player's `Creature`, and a literal null card source.
        (MonarchsGazeStrengthDown, "monarchs_gaze_strength_down"),
        /// `PiercingWailPower` (`get_IsPositive` `0xa5aa0` = false). Applied
        /// by `PiercingWail/<OnPlay>d__8::MoveNext` `0x3b253c` IL_00f3.
        (PiercingWail, "piercing_wail"),
        /// `ShacklingPotionPower` (`get_IsPositive` `0xa74d9` = false). The
        /// one member applied by a potion: `ShacklingPotion/<OnUse>d__10::
        /// MoveNext` `0x34ffb8` IL_007b-IL_00ae applies
        /// `DynamicVars.Strength.IntValue` to the frozen `HittableEnemies`
        /// list with `potion.Owner.Player.Creature` as the applier.
        (ShacklingPotion, "shackling_potion"),
        /// `StrengthPower`. `get_Type` `0xa8943` returns 1, `get_StackType`
        /// `0xa8946` returns 1 and `get_AllowNegative` `0xa8949` returns
        /// true, so `PowerModel::GetTypeForAmount` `0x83a94` flips it to
        /// Type 2 exactly while its amount is negative — which is what makes
        /// its *sign*, not its presence, decide whether `Misery` copies it.
        ///
        /// Present as the ledger's second model because the order and
        /// identity witnesses need two distinct models to say anything: a
        /// single-model ledger cannot show that a sort by model name would be
        /// visible, and cannot distinguish a model-keyed lookup from a
        /// position-keyed one.
        ///
        /// **#2693 S1 migrated monster Strength onto it.** The row carries
        /// POSITION and APPLIER; the `PowerId::Strength` scalar stays the
        /// value of record and the row's amount mirrors it, so the two can
        /// never disagree about how much Strength a creature has. A monster
        /// whose nonzero scalar carries no row has *unrecorded provenance* —
        /// a legacy checkpoint, or a fight that cannot reach `Misery` — and
        /// every reader refuses on it exactly where it refused before. See
        /// [`crate::engine::damage::write_monster_strength`] for the single
        /// writer and
        /// [`crate::engine::damage::misery_scalar_state_is_exact`] for the
        /// reader. Player Strength is deliberately NOT on the ledger: nothing
        /// clones it, so it would be an unwitnessed field.
        (Strength, "strength"),
    }
}

impl AttachedPowerModel {
    /// Is this a concrete `TemporaryStrengthPower` subclass whose `Sign` is
    /// negative — a temporary-Strength **loss** wrapper (#2693 S2)?
    ///
    /// Current-build authority is `sts2.dll` SHA-256
    /// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
    /// Everything the family shares is read, not assumed:
    ///
    /// * **The native `Amount` is POSITIVE and the effect is negative.**
    ///   `TemporaryStrengthPower::get_Sign` `0xa9add` is `IsPositive ? 1 :
    ///   -1`, and the nested application is `Sign * amount`
    ///   (`<BeforeApplied>d__20::MoveNext` `0x348d20` IL_0028-IL_004b). So a
    ///   loss wrapper of five is stored as `amount: 5` and moves Strength by
    ///   `-5`. An aggregate signed scalar cannot say which wrapper supplied
    ///   which part of that, which is the whole reason these rows exist.
    /// * **Always Type 2 while attached.** `get_Type` `0xa9ac3` returns 2 for
    ///   a non-positive wrapper, `get_StackType` `0xa9ad0` returns 1, and the
    ///   base `PowerModel::get_AllowNegative` `0x83a7d` returns false and no
    ///   subclass overrides it — so `GetTypeForAmount` `0x83a94` skips the
    ///   `AllowNegative` arm at IL_002e, skips IL_0063's negative test
    ///   (the amount is positive) and falls through at IL_0072 to `Type` = 2.
    ///   `Misery`'s `TypeForCurrentAmount == 2` filter `0x3ad30a` therefore
    ///   selects every live one, whatever its amount — unlike
    ///   [`AttachedPowerModel::Strength`], whose sign decides.
    /// * **Removed at any non-positive amount**, not at exactly zero:
    ///   `ShouldRemoveDueToAmount` `0x83b0d` IL_0001-IL_0010 is the
    ///   `!AllowNegative` arm for this family.
    /// * **Singleton per model, not per applier.** All eight inherit the base
    ///   `get_InstanceType` `0x83751` = 0, so
    ///   `PowerCmd::FindExistingInstanceForStacking` `0x1338d8` IL_0058
    ///   resolves them through `Creature::GetPower(Id)` `0x11d94c` — by id
    ///   alone. Two applications of the *same* model stack into one record
    ///   (`ModifyAmount` `0x3f032c`, a value and never a position); two
    ///   *different* models are two records at two positions, and
    ///   `Creature::ApplyPowerInternal` `0x11da0c` IL_0038-IL_0062 throws on
    ///   a second instance of one model.
    /// * **`InternallyAppliedPower` is `StrengthPower` for all eight**, since
    ///   none overrides `TemporaryStrengthPower::get_InternallyAppliedPower`
    ///   `0xa9ad3`. That is what makes `Misery`'s fold (`0x3ad335`) able to
    ///   find the Strength entry to adjust — #2693 S3's job, not this one's.
    pub const fn is_temporary_strength_wrapper(self) -> bool {
        // Exhaustive on purpose: a new variant must decide whether it joins
        // this family rather than defaulting out of it.
        match self {
            Self::CrushUnder
            | Self::DarkShackles
            | Self::DyingStar
            | Self::EnfeeblingTouch
            | Self::Mangle
            | Self::MonarchsGazeStrengthDown
            | Self::PiercingWail
            | Self::ShacklingPotion => true,
            Self::Imbalanced | Self::Strength => false,
        }
    }
}

/// One xoshiro stream's **live** state: the words and the draw counter.
///
/// R0.4 put the seed words in the catalog on the theory that search never
/// re-seeds mid-fight. That is true of the *seed* but not of the *state*: a
/// reshuffle advances `s.rng`'s four words in place (`combat_sim._rng_from`
/// starts (frozen Python, deleted #2827) and `_rng_tuple` serializes them), and the canonical
/// document carries the advanced
/// words. Keeping them catalog-side would have re-emitted the entry words
/// after every draw — a silent divergence on the first reshuffle. R0.5
/// therefore moves the whole stream state hot.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RngStreamState {
    /// The four xoshiro256** words, as they stand now.
    pub words: [u64; 4],
    /// How many draws this stream has issued.
    pub counter: u64,
}

/// The nine live streams, copy-on-write.
///
/// 360 bytes behind one pointer: streams are read on every draw but written
/// only by the handful of transitions that actually consume randomness (a
/// reshuffle, an AI roll), so the copy-on-write clone is paid where the work
/// already is, and every other transition pays one atomic increment. Inline
/// it would have cost 360 bytes of [`HotState`] against the then-200-byte
/// budget.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HotRng(Arc<[RngStreamState; RNG_STREAM_COUNT]>);

impl Default for HotRng {
    fn default() -> Self {
        Self(Arc::new([RngStreamState::default(); RNG_STREAM_COUNT]))
    }
}

impl HotRng {
    /// Every stream unseeded and undrawn.
    pub fn new() -> Self {
        Self::default()
    }

    /// One stream's live state.
    pub fn get(&self, stream: RngStream) -> RngStreamState {
        self.0[stream as usize]
    }

    /// Overwrite one stream, cloning shared storage exactly once.
    pub fn set(&mut self, stream: RngStream, state: RngStreamState) {
        Arc::make_mut(&mut self.0)[stream as usize] = state;
    }

    /// Whether a stream is at the canonical all-zero default.
    pub fn is_vacant(&self, stream: RngStream) -> bool {
        self.get(stream) == RngStreamState::default()
    }

    /// Whether the xoshiro state is a valid nonzero cycle state.
    ///
    /// The draw counter is observational metadata and cannot make the
    /// all-zero absorbing state valid.
    pub fn has_nonzero_words(&self, stream: RngStream) -> bool {
        self.get(stream).words != [0; 4]
    }
}

/// Who applied a physical power, by native creature identity.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
///
/// Native `PowerModel._applier` is a `Creature` reference, and
/// `PowerCmd/<>c__DisplayClass3_0::<FindExistingInstanceForStacking>b__0`
/// `0x3ef7ca` IL_000d compares it with `ceq` — *reference* identity, not a
/// name or a slot. Three cases are therefore distinguishable and must stay
/// distinguishable here: never written (`None`), the local player, and one
/// specific monster. A monster is identified by its `uid`, which is creation
/// order and never reused, so a replacement standing in a dead applier's slot
/// is a different `Applier` rather than the same one — exactly what `ceq`
/// would report. Collapsing the three would silently merge two native
/// instances at the `InstanceType == 2` stacking lookup
/// (`PowerCmd::FindExistingInstanceForStacking` `0x1338d8` IL_0038-IL_0055)
/// and would mis-answer the `ICombatState.ContainsCreature(applier)` liveness
/// gate on given-side modifiers
/// (`PowerCmd/<ModifyAmount>d__6::MoveNext` `0x3f032c` IL_0119).
///
/// All three arise in practice since #2693 S1, and which one a command
/// records is a native fact rather than a convention: a player card or potion
/// passes `card.Owner.Player.Creature` (`Malaise/<OnPlay>d__9::MoveNext`
/// `0x3ab428` IL_00fc-IL_0109), an enemy move or monster-owned power passes
/// its own `Creature` (`RitualPower/<AfterSideTurnEnd>d__11::MoveNext`
/// `0x342a94` IL_0052-IL_0071), and a relic passes a literal **null**
/// (`PhilosophersStone::AfterCreatureAddedToCombat` `0x994f0`
/// IL_004b-IL_004e; `Brimstone/<AfterSideTurnStart>d__8::MoveNext` `0x32098c`
/// IL_012d-IL_0130).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Applier {
    /// `_applier` was never written — the native field is null.
    None,
    /// The local player creature.
    Player,
    /// One monster, by its never-reused creation-order `uid`.
    Monster(u32),
    /// The instance is real and its position is known, but **which creature
    /// applied it was never observed** (#2693 S4).
    ///
    /// This is the one variant that is not a native identity. It exists for
    /// the instance a monster walks into a root already carrying — an
    /// `initial_powers` entry, an ascension effect, or any write that happened
    /// before the checkpoint the document records — where the ledger is empty,
    /// so the instance's *position* is uniquely determined (it precedes
    /// everything else that could be recorded) while its applier is simply
    /// absent from the evidence. The alternative was to record nothing, which
    /// is what #2693 forbids: an unrecorded scalar refuses when `Misery`
    /// reads it, and refusing late inside an admitted root is the defect.
    ///
    /// **Where it is exact.** `StrengthPower` is the only family this variant
    /// can carry, and it takes the base `PowerModel::get_InstanceType`
    /// `0x83751` = 0, so `PowerCmd::FindExistingInstanceForStacking`
    /// `0x1338d8` resolves it at IL_0058 by model id and never looks at the
    /// applier. The whole assembly has exactly two given-side amount
    /// modifiers — `SneckoSkull::ModifyPowerAmountGivenAdditive` `0x9bb8d`,
    /// which returns 0 for anything that is not a `PoisonPower`, and
    /// `UnsettlingLamp::ModifyPowerAmountGivenMultiplicative` `0x9d5a8` —
    /// and both are gated on `ICombatState.ContainsCreature(applier)`
    /// (`PowerCmd/<Apply>d__2::MoveNext` `0x3efbac` IL_01d3-IL_01ec). Refuse
    /// a reachable Lamp and the applier of a Strength instance is unobserved
    /// *in native too*, which is what makes "unknown" exact rather than
    /// approximate. [`crate::engine::admission`] carries that refusal, the
    /// same rule #2693 S1 wrote for a monster or relic applier.
    Unknown,
}

/// One physically attached power instance: model identity, applier identity
/// and the live native `Amount`.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
///
/// * **`applier` is written once.** `PowerCmd/<Apply>d__2::MoveNext`
///   `0x3efbac` does the stacking lookup at IL_00a3 and routes a *found*
///   instance to `PowerCmd::ModifyAmount` at IL_00c6; only the not-found
///   (fresh-attach) fall-through reaches `set_Applier` at IL_013d, before the
///   `BeforeApplied` hook at IL_02d9 and `ApplyInternal` at IL_0360. A later
///   stacking application therefore never rewrites it.
/// * **`applier` survives cloning.** `PowerModel::AfterCloned` `0x84093`
///   nulls `Flashed`, `DisplayAmountChanged`, `Removed`, `PulsingStarted`,
///   `PulsingStopped` and `_owner` — and leaves `_applier` alone.
/// * **`amount` is the live native `Amount`, and is never zero.**
///   `PowerModel::ApplyInternal` `0x84012` IL_0001-IL_000e returns before
///   setting the owner or attaching when the modified amount is zero, and
///   `PowerModel::ShouldRemoveDueToAmount` `0x83b0d` removes at exactly zero
///   for an `AllowNegative` power (IL_0012-IL_0023) and at any non-positive
///   amount otherwise (IL_0001-IL_0010). A zero-amount attachment is not a
///   native state, so the boundary refuses one.
///
/// The plan's fourth `flags: u8` word is deliberately absent: this stage has
/// no reader for it, and an unwitnessed field in a hashed, projected record
/// is a worse starting point for the migrations than adding it with its first
/// consumer.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct AttachmentRecord {
    /// The attached instance's native `PowerModel` class.
    pub power: AttachedPowerModel,
    /// Who applied it, by native creature identity.
    pub applier: Applier,
    /// The live native `Amount`; never zero.
    pub amount: i32,
}

/// A monster's acquisition-ordered Misery ledger, represented distinct
/// Knockdown instances, and ordered physical power attachments, copy-on-write
/// behind one hot word.
///
/// Acquisition order is not a set: `Misery` replays it, and Knockdown's
/// InstanceType-1 token repeats once per concrete power. The corresponding
/// positive amounts are retained in application order; the applier is the
/// one admitted local-player identity and therefore needs no second word.
///
/// [`MiseryEntry::Attachment`] generalises that to the native record the
/// remaining Misery families need — see [`AttachmentRecord`] and
/// [`MiseryOrder::push_attachment`] for the order contract and its citations.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
enum MiseryEntry {
    Token(MiseryToken),
    KnockdownAmount(i32),
    Attachment(AttachmentRecord),
}

/// One position of the unified `Creature.Powers` walk.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `Misery/<OnPlay>d__3::MoveNext` `0x3ad358` IL_003e-IL_009c builds its
/// dictionary from **one** ordered `Target.Powers` walk, so scalars and
/// attachments are a single sequence rather than two lists that happen to
/// share a monster. [`MiseryOrder::walk`] is that sequence; a Knockdown
/// amount rides behind its own token (`MiseryOrder::push_knockdown`) and is
/// therefore not a position of its own.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MiseryLedgerEntry {
    /// One scalar acquisition token, oldest first.
    Token(MiseryToken),
    /// One physically attached power instance.
    Attachment(AttachmentRecord),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct MiseryOrder(Arc<Vec<MiseryEntry>>);

/// Borrowed acquisition-token view over the compact shared Misery entries.
#[derive(Copy, Clone)]
pub struct MiseryTokens<'a>(&'a [MiseryEntry]);

impl MiseryTokens<'_> {
    /// Iterate acquisition tokens in native order.
    pub fn iter(&self) -> Self {
        *self
    }

    /// Number of acquisition tokens.
    pub fn len(&self) -> usize {
        self.0
            .iter()
            .filter(|entry| matches!(entry, MiseryEntry::Token(_)))
            .count()
    }

    /// Whether no acquisition token is present.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether the order contains the requested token.
    pub fn contains(&self, token: &MiseryToken) -> bool {
        self.0
            .iter()
            .any(|entry| matches!(entry, MiseryEntry::Token(value) if value == token))
    }
}

impl<'a> Iterator for MiseryTokens<'a> {
    type Item = &'a MiseryToken;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((entry, remaining)) = self.0.split_first() {
            self.0 = remaining;
            if let MiseryEntry::Token(token) = entry {
                return Some(token);
            }
        }
        None
    }
}

impl std::fmt::Debug for MiseryTokens<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl<const N: usize> PartialEq<[MiseryToken; N]> for MiseryTokens<'_> {
    fn eq(&self, other: &[MiseryToken; N]) -> bool {
        self.iter().copied().eq(other.iter().copied())
    }
}

impl<const N: usize> PartialEq<&[MiseryToken; N]> for MiseryTokens<'_> {
    fn eq(&self, other: &&[MiseryToken; N]) -> bool {
        self.iter().copied().eq(other.iter().copied())
    }
}

impl PartialEq<&[MiseryToken]> for MiseryTokens<'_> {
    fn eq(&self, other: &&[MiseryToken]) -> bool {
        self.iter().copied().eq(other.iter().copied())
    }
}

/// Borrowed ordered Knockdown-amount view over the compact shared entries.
#[derive(Copy, Clone)]
pub struct KnockdownAmounts<'a>(&'a [MiseryEntry]);

impl KnockdownAmounts<'_> {
    /// Iterate amounts in application order.
    pub fn iter(&self) -> Self {
        *self
    }

    /// Number of represented distinct instances.
    pub fn len(&self) -> usize {
        self.0
            .iter()
            .filter(|entry| matches!(entry, MiseryEntry::KnockdownAmount(_)))
            .count()
    }

    /// Whether no distinct instance is present.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Read one amount by its application-order index.
    pub fn get(&self, index: usize) -> Option<&i32> {
        self.iter().nth(index)
    }
}

impl<'a> Iterator for KnockdownAmounts<'a> {
    type Item = &'a i32;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((entry, remaining)) = self.0.split_first() {
            self.0 = remaining;
            if let MiseryEntry::KnockdownAmount(amount) = entry {
                return Some(amount);
            }
        }
        None
    }
}

impl std::fmt::Debug for KnockdownAmounts<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl<const N: usize> PartialEq<[i32; N]> for KnockdownAmounts<'_> {
    fn eq(&self, other: &[i32; N]) -> bool {
        self.iter().copied().eq(other.iter().copied())
    }
}

impl<const N: usize> PartialEq<&[i32; N]> for KnockdownAmounts<'_> {
    fn eq(&self, other: &&[i32; N]) -> bool {
        self.iter().copied().eq(other.iter().copied())
    }
}

impl PartialEq<&[i32]> for KnockdownAmounts<'_> {
    fn eq(&self, other: &&[i32]) -> bool {
        self.iter().copied().eq(other.iter().copied())
    }
}

/// Borrowed ordered physical-attachment view over the compact shared entries.
#[derive(Copy, Clone)]
pub struct Attachments<'a>(&'a [MiseryEntry]);

impl Attachments<'_> {
    /// Iterate attachments in `Creature.Powers` order.
    pub fn iter(&self) -> Self {
        *self
    }

    /// Number of live attachments.
    pub fn len(&self) -> usize {
        self.0
            .iter()
            .filter(|entry| matches!(entry, MiseryEntry::Attachment(_)))
            .count()
    }

    /// Whether nothing is attached.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Read one attachment by its ledger position.
    pub fn get(&self, index: usize) -> Option<&AttachmentRecord> {
        self.iter().nth(index)
    }
}

impl<'a> Iterator for Attachments<'a> {
    type Item = &'a AttachmentRecord;

    fn next(&mut self) -> Option<Self::Item> {
        while let Some((entry, remaining)) = self.0.split_first() {
            self.0 = remaining;
            if let MiseryEntry::Attachment(record) = entry {
                return Some(record);
            }
        }
        None
    }
}

impl std::fmt::Debug for Attachments<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_list().entries(self.iter()).finish()
    }
}

impl<const N: usize> PartialEq<[AttachmentRecord; N]> for Attachments<'_> {
    fn eq(&self, other: &[AttachmentRecord; N]) -> bool {
        self.iter().copied().eq(other.iter().copied())
    }
}

impl<const N: usize> PartialEq<&[AttachmentRecord; N]> for Attachments<'_> {
    fn eq(&self, other: &&[AttachmentRecord; N]) -> bool {
        self.iter().copied().eq(other.iter().copied())
    }
}

impl PartialEq<&[AttachmentRecord]> for Attachments<'_> {
    fn eq(&self, other: &&[AttachmentRecord]) -> bool {
        self.iter().copied().eq(other.iter().copied())
    }
}

impl MiseryOrder {
    /// An empty order — the canonical default.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from an ordered token list.
    pub fn from_tokens(tokens: Vec<MiseryToken>) -> Self {
        Self(Arc::new(
            tokens.into_iter().map(MiseryEntry::Token).collect(),
        ))
    }

    /// Hydrate only the canonical acquisition-order component while
    /// retaining independently decoded distinct-instance payloads.
    ///
    /// Returns `false` — the caller refuses — when an already-decoded
    /// attachment sits after more tokens than the replacement list has; see
    /// [`MiseryOrder::set_parts`].
    #[must_use]
    pub fn set_tokens(&mut self, tokens: Vec<MiseryToken>) -> bool {
        let amounts = self.knockdown().copied().collect::<Vec<_>>();
        self.set_parts(tokens, amounts)
    }

    /// The tokens, oldest first.
    pub fn as_slice(&self) -> MiseryTokens<'_> {
        MiseryTokens(self.0.as_slice())
    }

    /// Whether nothing has been acquired.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Concrete Knockdown amounts, oldest instance first.
    pub fn knockdown(&self) -> KnockdownAmounts<'_> {
        KnockdownAmounts(self.0.as_slice())
    }

    /// Hydrate the canonical distinct-instance payload independently from
    /// the acquisition-order field. Admission later proves exact one-to-one
    /// correspondence with repeated [`MiseryToken::Knockdown`] entries.
    ///
    /// The token list is unchanged here, so no attachment position can be
    /// stranded and [`MiseryOrder::set_parts`] cannot refuse.
    pub fn set_knockdown(&mut self, amounts: Vec<i32>) {
        let tokens = self.as_slice().copied().collect::<Vec<_>>();
        let retained = self.set_parts(tokens, amounts);
        debug_assert!(retained, "an unchanged token list strands no position");
    }

    /// Append one freshly acquired token.
    ///
    /// `combat_sim._record_misery_debuff_application` (frozen Python, deleted #2827) appends only
    /// for a *new* instance; a stacking application leaves the order alone.
    pub fn push(&mut self, token: MiseryToken) {
        Arc::make_mut(&mut self.0).push(MiseryEntry::Token(token));
    }

    /// Append one distinct local-player Knockdown instance and its matching
    /// Misery acquisition token in one COW publication.
    pub fn push_knockdown(&mut self, amount: i32) {
        let entries = Arc::make_mut(&mut self.0);
        entries.push(MiseryEntry::Token(MiseryToken::Knockdown));
        entries.push(MiseryEntry::KnockdownAmount(amount));
    }

    /// Physical attachments in `Creature.Powers` order, oldest first.
    pub fn attachments(&self) -> Attachments<'_> {
        Attachments(self.0.as_slice())
    }

    /// The unified `Creature.Powers` walk: tokens and attachments interleaved
    /// at their recorded positions.
    ///
    /// See [`MiseryLedgerEntry`] for why this is one sequence and not two.
    /// Nothing is allocated — the ledger already stores the entries in this
    /// order, so the walk is a filter over the shared vector.
    pub fn walk(&self) -> impl Iterator<Item = MiseryLedgerEntry> + '_ {
        self.0.iter().filter_map(|entry| match entry {
            MiseryEntry::Token(token) => Some(MiseryLedgerEntry::Token(*token)),
            MiseryEntry::KnockdownAmount(_) => None,
            MiseryEntry::Attachment(record) => Some(MiseryLedgerEntry::Attachment(*record)),
        })
    }

    /// Append one freshly attached power.
    ///
    /// Current-build authority is `sts2.dll` SHA-256
    /// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
    /// `Creature::ApplyPowerInternal` `0x11da0c` IL_0063-IL_006f is a plain
    /// `List.Add` onto `Creature::_powers`, and `Creature::get_Powers`
    /// `0x11d8ae` hands that list back without sorting or copying. Attachment
    /// order is therefore *arrival* order, which is what `Misery`'s
    /// `ToDictionary` over `Target.Powers`
    /// (`Misery/<OnPlay>d__3::MoveNext` `0x3ad358` IL_003e-IL_009c) freezes
    /// into its replay order. Appending is the only way a position is
    /// created; see [`MiseryOrder::set_attachment_amount`] and
    /// [`MiseryOrder::remove_attachment`] for the other two lifecycle edges.
    ///
    /// The append lands after every acquisition token present right now, so
    /// attach-then-acquire and acquire-then-attach are different ledgers that
    /// project to different documents. That difference is *observable*, not
    /// bookkeeping: Misery snapshots one ordered walk of `Creature.Powers`
    /// (`Misery/<OnPlay>d__3::MoveNext` `0x3ad358` IL_003e-IL_009c) and
    /// replays the clones in that order, so a recipient's Artifact consumes
    /// on whichever debuff comes first.
    pub fn push_attachment(&mut self, record: AttachmentRecord) {
        Arc::make_mut(&mut self.0).push(MiseryEntry::Attachment(record));
    }

    /// The ledger position of the instance a native stacking lookup would
    /// find, or `None` for a fresh attach.
    ///
    /// Current-build authority is `sts2.dll` SHA-256
    /// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
    /// `PowerCmd::FindExistingInstanceForStacking` `0x1338d8` switches on
    /// `InstanceType` (IL_0021) — 0 takes `Creature::GetPower(Id)` at
    /// IL_0058 (the singleton arm), 1 returns null at IL_0034 (never stacks),
    /// and 2 takes `GetPowerInstances(Id).FirstOrDefault(applier-equal)` at
    /// IL_0038-IL_0055. Every family this stage can name has the base
    /// `PowerModel::get_InstanceType` `0x83751` value 0, so the applier is
    /// carried but not yet consulted; the applier-keyed arm is deliberately
    /// spelled out here so the InstanceType-2 families cannot silently
    /// inherit a singleton lookup when they arrive.
    pub fn find_attachment(&self, power: AttachedPowerModel, applier: Applier) -> Option<usize> {
        self.attachments()
            .position(|record| record.power == power && record.applier == applier)
    }

    /// Restack one attachment in place.
    ///
    /// Current-build authority is `sts2.dll` SHA-256
    /// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
    /// `PowerCmd/<ModifyAmount>d__6::MoveNext` `0x3f032c` reaches
    /// `PowerModel::SetAmount` at IL_01d6, and `SetAmount` `0x83f8c` writes
    /// `_amount` (IL_0033) without ever touching `Creature::_powers`. A
    /// stacking application therefore changes a value, never a position.
    /// `SetAmount` also clamps to +/-999999999 (IL_0013-IL_0022) and returns
    /// early on a zero delta (IL_002d-IL_0030). Neither is modeled *here*:
    /// each family's own command owns them, because the clamp interacts with
    /// that family's overflow rule — `apply_card_monster_imbalanced_clone`
    /// clamps its stacked sum, and the Strength row's amount is the scalar's,
    /// which admission bounds to the same interval.
    ///
    /// Returns `false` when `index` names no attachment, or when `amount` is
    /// zero — a zero-amount attachment is not a native state (see
    /// [`AttachmentRecord`]).
    pub fn set_attachment_amount(&mut self, index: usize, amount: i32) -> bool {
        if amount == 0 {
            return false;
        }
        let Some(position) = self.attachment_entry_index(index) else {
            return false;
        };
        if let MiseryEntry::Attachment(record) = &mut Arc::make_mut(&mut self.0)[position] {
            record.amount = amount;
            return true;
        }
        false
    }

    /// Detach one attachment, closing its position without moving the rest.
    ///
    /// Current-build authority is `sts2.dll` SHA-256
    /// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
    /// `PowerModel::RemoveInternal` `0x84048` IL_001f calls
    /// `Creature::RemovePowerInternal` `0x11db0b`, whose IL_0015-IL_001c is a
    /// plain `List.Remove` on `Creature::_powers` — an in-place erase that
    /// leaves every surviving instance's relative order intact. A later
    /// re-application is an ordinary fresh attach and appends at the end
    /// (`Creature::ApplyPowerInternal` `0x11da0c` IL_0063-IL_006f), so a
    /// power that leaves and comes back loses its old position rather than
    /// reclaiming it. `ShouldRemoveDueToAmount` `0x83b0d` is the gate that
    /// decides when this runs.
    pub fn remove_attachment(&mut self, index: usize) -> bool {
        let Some(position) = self.attachment_entry_index(index) else {
            return false;
        };
        Arc::make_mut(&mut self.0).remove(position);
        true
    }

    /// Every attachment paired with the number of acquisition tokens that
    /// precede it in the unified ledger.
    ///
    /// This is the *derived* wire position: it is computed from the entry
    /// vector rather than stored, so there is exactly one source of truth for
    /// order and no stored index can disagree with the layout. A token
    /// removal shifts later attachments implicitly, because nothing was
    /// written down to become stale.
    pub fn placed_attachments(&self) -> Vec<(usize, AttachmentRecord)> {
        let mut tokens = 0usize;
        let mut placed = Vec::new();
        for entry in self.0.iter() {
            match entry {
                MiseryEntry::Token(_) => tokens += 1,
                MiseryEntry::KnockdownAmount(_) => {}
                MiseryEntry::Attachment(record) => placed.push((tokens, *record)),
            }
        }
        placed
    }

    /// Hydrate the attachment payload at its recorded unified positions.
    ///
    /// Returns `false` — the caller refuses — when the positions are not
    /// non-decreasing, or when one exceeds the live token count. Projection
    /// emits them in entry order and the preceding-token count is
    /// non-decreasing along the vector, so a violation is a malformed
    /// document rather than a state this ledger can hold.
    ///
    /// Callers hydrate `misery_debuff_order` before `power_attachments`: the
    /// canonical entity bag is walked in ascending key order, and
    /// `misery_debuff_order` sorts before `power_attachments`. The boundary
    /// test `the_attachment_slot_hydrates_after_the_token_slots` pins that,
    /// so the token count this validates against is always already complete.
    #[must_use]
    pub fn set_attachments(&mut self, placed: Vec<(usize, AttachmentRecord)>) -> bool {
        let tokens = self.as_slice().len();
        if placed.windows(2).any(|pair| pair[0].0 > pair[1].0)
            || placed.iter().any(|(position, _)| *position > tokens)
        {
            return false;
        }
        let tokens = self.as_slice().copied().collect::<Vec<_>>();
        let amounts = self.knockdown().copied().collect::<Vec<_>>();
        self.rebuild(tokens, amounts, placed);
        true
    }

    /// Translate an attachment position into an index into the shared entries.
    fn attachment_entry_index(&self, index: usize) -> Option<usize> {
        self.0
            .iter()
            .enumerate()
            .filter(|(_, entry)| matches!(entry, MiseryEntry::Attachment(_)))
            .map(|(position, _)| position)
            .nth(index)
    }

    /// Replace the token/Knockdown components, retaining each attachment at
    /// its own unified position.
    ///
    /// Returns `false` when an existing attachment sits after more tokens
    /// than the replacement list has. Sweeping it elsewhere would be an
    /// approximation of an order native treats as observable, so this
    /// refuses instead (I5). Within hydration the case cannot arise — the
    /// token slots are written first, while the ledger is still empty — and
    /// no engine path replaces a token list wholesale today.
    #[must_use]
    fn set_parts(&mut self, tokens: Vec<MiseryToken>, amounts: Vec<i32>) -> bool {
        let placed = self.placed_attachments();
        if placed.iter().any(|(position, _)| *position > tokens.len()) {
            return false;
        }
        self.rebuild(tokens, amounts, placed);
        true
    }

    /// Lay the three components out in one canonical entry vector.
    ///
    /// The layout is total and position-determined, so two ledgers holding
    /// the same facts are always `==`: attachments recorded after `k` tokens
    /// follow the `k`th token and its Knockdown amount, and `k == 0` leads.
    /// A Knockdown amount always rides immediately behind its own token —
    /// `push_knockdown` publishes the pair atomically — so no attachment can
    /// ever land between them.
    fn rebuild(
        &mut self,
        tokens: Vec<MiseryToken>,
        amounts: Vec<i32>,
        placed: Vec<(usize, AttachmentRecord)>,
    ) {
        let mut amounts = amounts.into_iter();
        let mut next_placed = 0usize;
        let mut entries = Vec::with_capacity(tokens.len().saturating_add(placed.len()));
        for index in 0..=tokens.len() {
            if index > 0 {
                let token = tokens[index - 1];
                entries.push(MiseryEntry::Token(token));
                if token == MiseryToken::Knockdown
                    && let Some(amount) = amounts.next()
                {
                    entries.push(MiseryEntry::KnockdownAmount(amount));
                }
            }
            while let Some((position, record)) = placed.get(next_placed).copied() {
                if position != index {
                    break;
                }
                entries.push(MiseryEntry::Attachment(record));
                next_placed += 1;
            }
        }
        entries.extend(amounts.map(MiseryEntry::KnockdownAmount));
        entries.extend(
            placed[next_placed..]
                .iter()
                .map(|(_, record)| MiseryEntry::Attachment(*record)),
        );
        *Arc::make_mut(&mut self.0) = entries;
    }

    /// Drop every entry for one wholly removed power
    /// (`_remove_misery_debuff_token`, frozen Python, deleted #2827).
    ///
    /// Attachments are a separate native record with their own removal gate
    /// (`PowerModel::ShouldRemoveDueToAmount` `0x83b0d`) and are untouched
    /// here: a scalar token's disappearance says nothing about a physically
    /// attached instance. Their unified *positions* do move, by one per
    /// removed token that preceded them — and correctly so, because a
    /// position is derived from the entry vector rather than stored, so
    /// there is no recorded index left to go stale.
    pub fn remove_all(&mut self, token: MiseryToken) {
        if self.as_slice().contains(&token) {
            Arc::make_mut(&mut self.0).retain(|entry| match entry {
                MiseryEntry::Token(value) => *value != token,
                MiseryEntry::KnockdownAmount(_) => token != MiseryToken::Knockdown,
                MiseryEntry::Attachment(_) => true,
            });
        }
    }
}

/// `HotCard::flags` bit: the card is a selection reference
/// (`PhysicalCardPick`), not a pile resident.
pub const CARD_FLAG_PICK: u16 = 1 << 0;

/// `HotCard::flags` bit: the instance carries the modeled slot-7 physical
/// payload (`_with_default_physical_card_state`, frozen Python, deleted #2827).
///
/// `start_combat` attaches an empty `PHYSICAL_CARD_STATE` row to every card in
/// `_PHYSICAL_COST_CARD_IDS` — Stomp, Claw, Modded, Thrash and eighteen others
/// — before anything has modified it. The ordered local Energy-cost rows, the
/// exact temporary `(0, true, true)` Star-free row, and damage growth live in
/// the side table below; one bit says the slot itself is present so an empty
/// row still round-trips. Other Star rows and the remaining slot-7 tails
/// continue to refuse at the boundary.
pub const CARD_FLAG_DEFAULT_PHYSICAL_STATE: u16 = 1 << 1;

/// The pile entry is a legacy semantic tuple without physical identity.
/// Monster status injection uses this form until the next native identity
/// normalization pass allocates it from `next_card_uid`.
pub const CARD_FLAG_LEGACY: u16 = 1 << 2;

/// The instance carries Sovereign Blade's required slot-6 payload. Its
/// nonnegative damage reuses `CardInstanceState::damage_growth`: slot-7
/// damage growth is refused on this identity, so the two meanings never
/// coexist. The bit preserves the distinct valid damage-zero payload without
/// growing the 24-byte side-table row.
pub const CARD_FLAG_SOVEREIGN_BLADE_STATE: u16 = 1 << 3;

/// The exact opaque-tail row `(CARD_AFFLICTION, RINGING, 1)`. Its payload is
/// fixed, so one card flag preserves the valid zero/nonzero distinction
/// without growing every side-table entry.
pub const CARD_FLAG_RINGING: u16 = 1 << 4;

/// The instance carries Genetic Algorithm's exact opaque-tail row
/// `(GENETIC_ALGORITHM_STATE, IncreasedBlock, DeckVersionRow)`.
///
/// The flag preserves the valid explicit `(0, null)` row without forcing a
/// side-table entry. The packed payload itself lives in
/// [`CardInstanceState::genetic_algorithm`].
pub const CARD_FLAG_GENETIC_ALGORITHM_STATE: u16 = 1 << 5;

/// The exact opaque-tail row `(CARD_AFFLICTION, BOUND, 3)`.
pub const CARD_FLAG_BOUND: u16 = 1 << 6;

/// The exact opaque-tail row `(CARD_AFFLICTION, HEXED, 2)`.
pub const CARD_FLAG_HEXED: u16 = 1 << 7;

/// The exact opaque-tail row `(CARD_DUPE, true)`. Native result routing
/// removes dupes before considering forced or effective Exhaust.
pub const CARD_FLAG_DUPE: u16 = 1 << 8;

/// Execution-only exact effective-Exhaust witness. Public canonical import
/// continues to refuse arbitrary local Exhaust because no admitted modeled
/// producer writes it; focused routing tests use this bit to pin native's
/// keyword branch without widening the public source closure.
pub const CARD_FLAG_LOCAL_EXHAUST: u16 = 1 << 9;

/// Derived Melancholy identity marker for catalog-free actual-death hooks.
/// Import and fresh generation derive it from CardId; admission/projection
/// reject disagreement. It is not an independently settable canonical field.
pub const CARD_FLAG_MELANCHOLY: u16 = 1 << 10;

/// Placeholder uid carried only while [`CARD_FLAG_LEGACY`] is set.
pub const LEGACY_CARD_UID: u32 = u32::MAX;

/// Every flag bit defined so far — anything else set is a construction bug.
pub const CARD_FLAGS_KNOWN: u16 = CARD_FLAG_PICK
    | CARD_FLAG_DEFAULT_PHYSICAL_STATE
    | CARD_FLAG_LEGACY
    | CARD_FLAG_SOVEREIGN_BLADE_STATE
    | CARD_FLAG_RINGING
    | CARD_FLAG_GENETIC_ALGORITHM_STATE
    | CARD_FLAG_BOUND
    | CARD_FLAG_HEXED
    | CARD_FLAG_DUPE
    | CARD_FLAG_LOCAL_EXHAUST
    | CARD_FLAG_MELANCHOLY;

/// One physical card, in eight bytes.
///
/// `uid` is the physical identity Python's allocator issues; `atom` indexes
/// the per-fight catalog, which owns the `(id, upgrade, enchantment)` spec
/// and the precomputed row predicates. Mutable per-instance data lives in
/// [`CardStates`], keyed by the same uid.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HotCard {
    /// Physical identity.
    pub uid: u32,
    /// Index into the per-fight catalog.
    pub atom: CardAtom,
    /// [`CARD_FLAG_PICK`] and future single-bit card facts.
    pub flags: u16,
}

/// One eight-byte word in the continuation store.
///
/// Record headers are self-describing; body words are interpreted only
/// through their authenticated parent/subrecord header. Simple card bodies
/// use the exact [`HotCard`] layout, frozen-batch entries add a closed
/// semantic [`CardInstanceState`] subrecord, and Strangle bodies use
/// `(uid, amount)`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct PendingWord {
    body: u32,
    meta: u32,
}

impl PendingWord {
    const SCHEMA: u8 = 1;

    fn header(kind: WordRecordKind, body_count: usize) -> Option<Self> {
        Some(Self {
            body: body_count.try_into().ok()?,
            meta: u32::from(kind as u8) | (u32::from(Self::SCHEMA) << 8),
        })
    }

    fn parse_header(self) -> Option<(WordRecordKind, u32)> {
        let kind = WordRecordKind::from_u8(self.meta as u8)?;
        let schema = ((self.meta >> 8) & 0xff) as u8;
        let reserved = self.meta >> 16;
        (schema == Self::SCHEMA && reserved == 0).then_some((kind, self.body))
    }

    fn from_card(card: HotCard) -> Self {
        Self {
            body: card.uid,
            meta: u32::from(card.atom) | (u32::from(card.flags) << 16),
        }
    }

    fn from_strangle(uid: u32, amount: i32) -> Self {
        Self {
            body: uid,
            meta: amount as u32,
        }
    }

    fn strangle(self) -> (u32, i32) {
        (self.body, self.meta as i32)
    }

    fn from_physical_listener(listener: PhysicalAfterCardPlayedListener) -> Self {
        Self {
            body: listener.uid,
            meta: listener.id as u32,
        }
    }

    fn physical_listener(self) -> Option<PhysicalAfterCardPlayedListener> {
        let id = CardId::ALL.get(self.meta as usize).copied()?;
        matches!(id, CardId::BansheesCry | CardId::Pinpoint)
            .then_some(PhysicalAfterCardPlayedListener { uid: self.body, id })
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum WordRecordKind {
    PendingChoice = 1,
    SelectionCards = 2,
    StrangleLatches = 3,
    CardPlay = 4,
    OblivionLatches = 5,
    EidolonReceipt = 6,
    PhysicalAfterCardPlayedListeners = 7,
    Phase = 8,
    LiveListener = 9,
    FrozenAutoBatch = 10,
    ActionReplay = 11,
    FrozenAutoBatchEntry = 12,
    TurnStartHandChoice = 13,
    EnemyPhase = 14,
    PotionFinish = 15,
    Draw = 16,
    AfterCardDrawnPower = 17,
    CardFinish = 18,
    AfterPowerAmountChanged = 19,
    MonologueLatches = 20,
    AfterCardPlayedPowerSnapshot = 21,
    InstancedPowerCardReceipt = 22,
    AfterSideTurnEndPower = 23,
    AfterCardExhaustedPower = 24,
    BeforeHandDrawPower = 25,
}

impl WordRecordKind {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::PendingChoice),
            2 => Some(Self::SelectionCards),
            3 => Some(Self::StrangleLatches),
            4 => Some(Self::CardPlay),
            5 => Some(Self::OblivionLatches),
            6 => Some(Self::EidolonReceipt),
            7 => Some(Self::PhysicalAfterCardPlayedListeners),
            8 => Some(Self::Phase),
            9 => Some(Self::LiveListener),
            10 => Some(Self::FrozenAutoBatch),
            11 => Some(Self::ActionReplay),
            12 => Some(Self::FrozenAutoBatchEntry),
            13 => Some(Self::TurnStartHandChoice),
            14 => Some(Self::EnemyPhase),
            15 => Some(Self::PotionFinish),
            16 => Some(Self::Draw),
            17 => Some(Self::AfterCardDrawnPower),
            18 => Some(Self::CardFinish),
            19 => Some(Self::AfterPowerAmountChanged),
            20 => Some(Self::MonologueLatches),
            21 => Some(Self::AfterCardPlayedPowerSnapshot),
            22 => Some(Self::InstancedPowerCardReceipt),
            23 => Some(Self::AfterSideTurnEndPower),
            24 => Some(Self::AfterCardExhaustedPower),
            25 => Some(Self::BeforeHandDrawPower),
            _ => None,
        }
    }
}

/// Exact suspended point inside `DrawInternal`.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawStage {
    EarlyHook = 0,
    OrdinaryHook = 1,
    AfterShuffle = 2,
}

impl DrawStage {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::EarlyHook => "early_hook",
            Self::OrdinaryHook => "ordinary_hook",
            Self::AfterShuffle => "after_shuffle",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match value {
            "early_hook" => Some(Self::EarlyHook),
            "ordinary_hook" => Some(Self::OrdinaryHook),
            "after_shuffle" => Some(Self::AfterShuffle),
            _ => None,
        }
    }

    fn from_ordinal(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::EarlyHook),
            1 => Some(Self::OrdinaryHook),
            2 => Some(Self::AfterShuffle),
            _ => None,
        }
    }
}

/// Closed post-Draw return sites admitted through the R52D2g card-body slice.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawCaller {
    PotionEpilogue = 0,
    ClarityPotion = 1,
    SneckoOil = 2,
    SlyAfterDraw = 3,
    AfterCardDrawnPower = 4,
    TurnStart = 5,
    AshwaterExhaust = 6,
    UnceasingTopPotion = 7,
    UnceasingTopTurnStart = 8,
    UnceasingTopFinish = 9,
    DistilledChaosGather = 10,
    CardPlay = 11,
    DarkEmbraceSideEnd = 12,
    AfterPowerAmountChanged = 13,
    AfterCardExhaustedPower = 14,
    Pillage = 15,
    RestlessnessFinal = 16,
    RestlessnessOneRemaining = 17,
    RestlessnessTwoRemaining = 18,
    ForegoneBeforeHandDraw = 19,
    /// One of Centennial Puzzle's three awaited one-card `CardPileCmd.Draw`
    /// calls (`CentennialPuzzle/<AfterDamageReceived>d__10::MoveNext` RVA
    /// `0x321758` IL_007f-0116). The caller's suffix is owned by the
    /// ActionReplay receipt, not by a frame (#3114): see
    /// `engine::puzzle::receipt_owned_draw`.
    CentennialPuzzle = 20,
    /// The Swift enchantment's one awaited `CardPileCmd.Draw(Amount)`
    /// (`Swift/<OnPlay>d__4::MoveNext` RVA `0x3885c0` IL_001d-0053), owned
    /// like the Puzzle's by the ActionReplay receipt (#3115).
    SwiftEnchantment = 21,
    /// Joss Paper's one awaited `CardPileCmd.Draw(CardsExhausted / 5)`
    /// (`JossPaper/<DrawIfThresholdMet>d__27::MoveNext` RVA `0x327888`
    /// IL_0058-0094), reached from its `AfterCardExhausted` and
    /// `AfterSideTurnEnd` bodies and owned like the Puzzle's by the
    /// ActionReplay receipt (#3201).
    JossPaper = 22,
    /// Gremlin Horn's awaited `CardPileCmd.Draw(choiceContext, Cards, Owner,
    /// false)` (`GremlinHorn/<AfterDeath>d__6::MoveNext` RVA `0x326170`
    /// IL_00bc-00d9), the listener's tail (#3387). A choice it begins is
    /// deferred into a queued `GenericHookGameAction`; the Draw frame is then
    /// published at the base of the stack once the enclosing action has
    /// finished (`engine::hook_action`), and its completion owns nothing.
    GremlinHorn = 23,
}

/// Closed producer continuations which can await an ordinary
/// `AfterCardExhausted` Dark Embrace Draw. Ten wire tags distinguish fourteen
/// logical producer routes multiplexed through twelve owner-capable entry
/// expressions (thirteen source literals including the synchronous wrapper).
/// Result-bearing Pillage and Restlessness Draws remain distinct Draw callers
/// rather than exhaust return modes.
///
/// `Glowwater` (#2480) is the first producer that is a potion body rather
/// than a card step: `GlowwaterPotion/<OnUse>d__10::MoveNext` `0x34e764`
/// awaits one `CardCmd::Exhaust` per card of its frozen Hand snapshot.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AfterCardExhaustedReturnKind {
    ReturnOnly = 0,
    CardFinish = 1,
    PuritySelection = 2,
    GenericSelection = 3,
    BurningPact = 4,
    SecondWind = 5,
    FiendFire = 6,
    Stoke = 7,
    Flak = 8,
    Glowwater = 9,
}

impl AfterCardExhaustedReturnKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ReturnOnly => "return_only",
            Self::CardFinish => "card_finish",
            Self::PuritySelection => "purity_selection",
            Self::GenericSelection => "generic_selection",
            Self::BurningPact => "burning_pact",
            Self::SecondWind => "second_wind",
            Self::FiendFire => "fiend_fire",
            Self::Stoke => "stoke",
            Self::Flak => "flak",
            Self::Glowwater => "glowwater",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match value {
            "return_only" => Some(Self::ReturnOnly),
            "card_finish" => Some(Self::CardFinish),
            "purity_selection" => Some(Self::PuritySelection),
            "generic_selection" => Some(Self::GenericSelection),
            "burning_pact" => Some(Self::BurningPact),
            "second_wind" => Some(Self::SecondWind),
            "fiend_fire" => Some(Self::FiendFire),
            "stoke" => Some(Self::Stoke),
            "flak" => Some(Self::Flak),
            "glowwater" => Some(Self::Glowwater),
            _ => None,
        }
    }

    fn from_ordinal(value: u8) -> Option<Self> {
        Self::ALL.get(value as usize).copied()
    }

    const ALL: [Self; 10] = [
        Self::ReturnOnly,
        Self::CardFinish,
        Self::PuritySelection,
        Self::GenericSelection,
        Self::BurningPact,
        Self::SecondWind,
        Self::FiendFire,
        Self::Stoke,
        Self::Flak,
        Self::Glowwater,
    ];
}

/// Frozen native object-iterator state plus the exact finite producer cursor.
/// `remaining` contains physical card UIDs only; callback payloads are always
/// re-resolved from the live card object by UID after an awaited child.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AfterCardExhaustedPowerRecord {
    pub listeners: Vec<PowerId>,
    pub cursor: u32,
    pub card_uid: u32,
    /// Hook-entry pile-order snapshot of every physical Midnight object.
    pub midnight_uids: Vec<u32>,
    pub ordinary_suffix_invoked: bool,
    pub return_kind: AfterCardExhaustedReturnKind,
    pub source_uid: Option<u32>,
    pub step_index: u32,
    pub amount: i32,
    pub count: u32,
    pub target: Option<(i32, u32, Option<i32>)>,
    pub flags: u32,
    pub remaining: Vec<u32>,
    pub generated: Vec<CardId>,
    pub generation_cursor: u32,
}

impl AfterCardExhaustedPowerRecord {
    pub(crate) fn for_return(return_kind: AfterCardExhaustedReturnKind) -> Self {
        Self {
            listeners: Vec::new(),
            cursor: 0,
            card_uid: 0,
            midnight_uids: Vec::new(),
            ordinary_suffix_invoked: false,
            return_kind,
            source_uid: None,
            step_index: 0,
            amount: 0,
            count: 0,
            target: None,
            flags: 0,
            remaining: Vec::new(),
            generated: Vec::new(),
            generation_cursor: 0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AfterCardExhaustedPowerView<'a> {
    listeners: &'a [PendingWord],
    midnight_uids: &'a [PendingWord],
    remaining: &'a [PendingWord],
    generated: &'a [PendingWord],
    pub cursor: u32,
    pub card_uid: u32,
    pub ordinary_suffix_invoked: bool,
    pub return_kind: AfterCardExhaustedReturnKind,
    pub source_uid: Option<u32>,
    pub step_index: u32,
    pub amount: i32,
    pub count: u32,
    pub target: Option<(i32, u32, Option<i32>)>,
    pub flags: u32,
    pub generation_cursor: u32,
}

impl AfterCardExhaustedPowerView<'_> {
    pub(crate) fn listeners(&self) -> impl ExactSizeIterator<Item = PowerId> + '_ {
        self.listeners
            .iter()
            .map(|word| PowerId::ALL[word.body as usize])
    }

    pub(crate) fn remaining(&self) -> impl ExactSizeIterator<Item = u32> + '_ {
        self.remaining.iter().map(|word| word.body)
    }

    pub(crate) fn midnight_uids(&self) -> impl ExactSizeIterator<Item = u32> + '_ {
        self.midnight_uids.iter().map(|word| word.body)
    }

    pub(crate) fn generated(&self) -> impl ExactSizeIterator<Item = CardId> + '_ {
        self.generated
            .iter()
            .map(|word| CardId::ALL[word.body as usize])
    }

    pub(crate) fn to_owned(self) -> AfterCardExhaustedPowerRecord {
        AfterCardExhaustedPowerRecord {
            listeners: self.listeners().collect(),
            cursor: self.cursor,
            card_uid: self.card_uid,
            midnight_uids: self.midnight_uids().collect(),
            ordinary_suffix_invoked: self.ordinary_suffix_invoked,
            return_kind: self.return_kind,
            source_uid: self.source_uid,
            step_index: self.step_index,
            amount: self.amount,
            count: self.count,
            target: self.target,
            flags: self.flags,
            remaining: self.remaining().collect(),
            generated: self.generated().collect(),
            generation_cursor: self.generation_cursor,
        }
    }
}

/// Result-routed `CardModel.OnPlayWrapper` cursor retained only while
/// Unceasing Top's CheckForEmptyHand Draw can suspend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CardFinishRecord {
    pub uid: u32,
    pub payload: FrozenAutoBatchEntry,
    pub glam: u32,
    pub result_route: CardResultRoute,
    pub source: CardPlaySource,
    pub is_power_auto: bool,
    pub routed_uid: Option<u32>,
    pub stage: crate::frame::CardFinishStage,
}

impl DrawCaller {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::PotionEpilogue => "potion_epilogue",
            Self::ClarityPotion => "clarity_potion",
            Self::SneckoOil => "snecko_oil",
            Self::SlyAfterDraw => "sly_after_draw",
            Self::AfterCardDrawnPower => "after_card_drawn_power_order",
            Self::TurnStart => "turn_start",
            Self::AshwaterExhaust => "ashwater_exhaust",
            Self::UnceasingTopPotion => "unceasing_top_potion",
            Self::UnceasingTopTurnStart => "unceasing_top_turn_start",
            Self::UnceasingTopFinish => "unceasing_top_finish",
            Self::DistilledChaosGather => "distilled_chaos_gather",
            // Python's no-result Draw handler has no locals. The surrounding
            // CardPlay frame, rather than the Draw frame, owns the suffix.
            Self::CardPlay => "none",
            Self::DarkEmbraceSideEnd => "dark_embrace_side_end",
            Self::AfterPowerAmountChanged => "after_power_amount_changed_power_order",
            Self::AfterCardExhaustedPower => "after_card_exhausted_power_order",
            Self::Pillage => "pillage",
            Self::RestlessnessFinal => "restlessness_final",
            Self::RestlessnessOneRemaining => "restlessness_one_remaining",
            Self::RestlessnessTwoRemaining => "restlessness_two_remaining",
            Self::ForegoneBeforeHandDraw => "foregone_before_hand_draw",
            Self::CentennialPuzzle => "centennial_puzzle",
            Self::SwiftEnchantment => "swift_enchantment",
            Self::JossPaper => "joss_paper",
            Self::GremlinHorn => "gremlin_horn",
        }
    }

    pub(crate) fn from_str(value: &str) -> Option<Self> {
        match value {
            "potion_epilogue" => Some(Self::PotionEpilogue),
            "clarity_potion" => Some(Self::ClarityPotion),
            "snecko_oil" => Some(Self::SneckoOil),
            "sly_after_draw" => Some(Self::SlyAfterDraw),
            "after_card_drawn_power_order" => Some(Self::AfterCardDrawnPower),
            "turn_start" => Some(Self::TurnStart),
            "ashwater_exhaust" => Some(Self::AshwaterExhaust),
            "unceasing_top_potion" => Some(Self::UnceasingTopPotion),
            "unceasing_top_turn_start" => Some(Self::UnceasingTopTurnStart),
            "unceasing_top_finish" => Some(Self::UnceasingTopFinish),
            "distilled_chaos_gather" => Some(Self::DistilledChaosGather),
            "none" => Some(Self::CardPlay),
            "dark_embrace_side_end" => Some(Self::DarkEmbraceSideEnd),
            "after_power_amount_changed_power_order" => Some(Self::AfterPowerAmountChanged),
            "after_card_exhausted_power_order" => Some(Self::AfterCardExhaustedPower),
            "pillage" => Some(Self::Pillage),
            "restlessness_final" => Some(Self::RestlessnessFinal),
            "restlessness_one_remaining" => Some(Self::RestlessnessOneRemaining),
            "restlessness_two_remaining" => Some(Self::RestlessnessTwoRemaining),
            "foregone_before_hand_draw" => Some(Self::ForegoneBeforeHandDraw),
            "centennial_puzzle" => Some(Self::CentennialPuzzle),
            "swift_enchantment" => Some(Self::SwiftEnchantment),
            "joss_paper" => Some(Self::JossPaper),
            "gremlin_horn" => Some(Self::GremlinHorn),
            _ => None,
        }
    }

    fn from_ordinal(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::PotionEpilogue),
            1 => Some(Self::ClarityPotion),
            2 => Some(Self::SneckoOil),
            3 => Some(Self::SlyAfterDraw),
            4 => Some(Self::AfterCardDrawnPower),
            5 => Some(Self::TurnStart),
            6 => Some(Self::AshwaterExhaust),
            7 => Some(Self::UnceasingTopPotion),
            8 => Some(Self::UnceasingTopTurnStart),
            9 => Some(Self::UnceasingTopFinish),
            10 => Some(Self::DistilledChaosGather),
            11 => Some(Self::CardPlay),
            12 => Some(Self::DarkEmbraceSideEnd),
            13 => Some(Self::AfterPowerAmountChanged),
            14 => Some(Self::AfterCardExhaustedPower),
            15 => Some(Self::Pillage),
            16 => Some(Self::RestlessnessFinal),
            17 => Some(Self::RestlessnessOneRemaining),
            18 => Some(Self::RestlessnessTwoRemaining),
            19 => Some(Self::ForegoneBeforeHandDraw),
            20 => Some(Self::CentennialPuzzle),
            21 => Some(Self::SwiftEnchantment),
            22 => Some(Self::JossPaper),
            23 => Some(Self::GremlinHorn),
            _ => None,
        }
    }
}

/// Frozen ordinary player-power listener walk for one physical drawn card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AfterCardDrawnPowerRecord {
    pub listeners: Vec<PowerId>,
    pub cursor: u32,
    pub card_uid: u32,
    pub from_hand_draw: bool,
    pub cacophony_reset: bool,
    pub cacophony_reset_generation: i32,
}

/// Frozen player-owned AfterPowerAmountChanged walk for one positive,
/// permanent Type-2 Vulnerable application to an exact monster identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AfterPowerAmountChangedRecord {
    pub listeners: Vec<PowerId>,
    pub cursor: u32,
    pub amount: i32,
    pub target: (i32, u32, Option<i32>),
}

/// Retained native object state for one member of the frozen ordinary
/// `AfterSideTurnEnd` snapshot.  Four signed payload lanes cover the complete
/// current-build 29-type census without widening `Frame`; token-specific
/// construction and validation give each lane its exact meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AfterSideTurnEndPowerSnapshot {
    pub object: AfterSideTurnEndPowerEntry,
    pub payload: [i32; 4],
}

/// Frozen ordinary hook iterator.  `cursor` is precommitted before invoking a
/// callback.  A parked Dark Embrace child records its old captured object uid
/// even if that object has meanwhile been removed from the live ledger.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AfterSideTurnEndPowerRecord {
    pub listeners: Vec<AfterSideTurnEndPowerSnapshot>,
    pub cursor: u32,
    pub dark_pending_uid: Option<u32>,
    /// Every ordinary listener after the local player-power subwalk,
    /// including relic/global and later-owner callbacks, has been invoked.
    /// Only the Hook/WhenAll continuation remains blocked on a parked child.
    pub ordinary_suffix_invoked: bool,
    /// Entry witness consumed only by the post-Hook enemy-phase suffix.
    pub conqueror_reachable_at_entry: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AfterSideTurnEndPowerView<'a> {
    listeners: &'a [PendingWord],
    pub cursor: u32,
    pub dark_pending_uid: Option<u32>,
    pub ordinary_suffix_invoked: bool,
    pub conqueror_reachable_at_entry: bool,
}

impl AfterSideTurnEndPowerView<'_> {
    pub(crate) fn listeners(
        &self,
    ) -> impl ExactSizeIterator<Item = AfterSideTurnEndPowerSnapshot> + '_ {
        self.listeners.chunks_exact(3).map(|words| {
            let object = AfterSideTurnEndPowerEntry {
                token: AfterSideTurnEndPowerToken::from_ordinal(words[0].meta)
                    .expect("validated AfterSideTurnEnd token"),
                uid: words[0].body,
            };
            AfterSideTurnEndPowerSnapshot {
                object,
                payload: [
                    words[1].body as i32,
                    words[1].meta as i32,
                    words[2].body as i32,
                    words[2].meta as i32,
                ],
            }
        })
    }

    #[cfg(test)]
    pub(crate) fn to_owned(self) -> AfterSideTurnEndPowerRecord {
        AfterSideTurnEndPowerRecord {
            listeners: self.listeners().collect(),
            cursor: self.cursor,
            dark_pending_uid: self.dark_pending_uid,
            ordinary_suffix_invoked: self.ordinary_suffix_invoked,
            conqueror_reachable_at_entry: self.conqueror_reachable_at_entry,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AfterPowerAmountChangedView<'a> {
    listeners: &'a [PendingWord],
    pub cursor: u32,
    pub amount: i32,
    pub target: (i32, u32, Option<i32>),
}

impl AfterPowerAmountChangedView<'_> {
    pub(crate) fn listeners(&self) -> impl ExactSizeIterator<Item = PowerId> + '_ {
        self.listeners
            .iter()
            .map(|word| PowerId::ALL[word.body as usize])
    }

    pub(crate) fn to_owned(self) -> AfterPowerAmountChangedRecord {
        AfterPowerAmountChangedRecord {
            listeners: self.listeners().collect(),
            cursor: self.cursor,
            amount: self.amount,
            target: self.target,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AfterCardDrawnPowerView<'a> {
    listeners: &'a [PendingWord],
    pub cursor: u32,
    pub card_uid: u32,
    pub from_hand_draw: bool,
    pub cacophony_reset: bool,
    pub cacophony_reset_generation: i32,
}

impl AfterCardDrawnPowerView<'_> {
    pub(crate) fn listeners(&self) -> impl ExactSizeIterator<Item = PowerId> + '_ {
        self.listeners
            .iter()
            .map(|word| PowerId::ALL[word.body as usize])
    }

    pub(crate) fn to_owned(self) -> AfterCardDrawnPowerRecord {
        AfterCardDrawnPowerRecord {
            listeners: self.listeners().collect(),
            cursor: self.cursor,
            card_uid: self.card_uid,
            from_hand_draw: self.from_hand_draw,
            cacophony_reset: self.cacophony_reset,
            cacophony_reset_generation: self.cacophony_reset_generation,
        }
    }
}

/// One immutable element of DrawInternal's result prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrawEntry {
    Uid(HotCard),
    Payload(HotCard),
}

impl DrawEntry {
    pub(crate) const fn card(self) -> HotCard {
        match self {
            Self::Uid(card) | Self::Payload(card) => card,
        }
    }
}

/// Complete immutable cursor for one suspended `CardPileCmd.Draw`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DrawRecord {
    pub requested: u32,
    pub completed: u32,
    pub drawn: Vec<DrawEntry>,
    pub from_hand_draw: bool,
    pub stage: DrawStage,
    pub card_uid: Option<u32>,
    pub caller: DrawCaller,
    /// Frozen native rarity/ModelId-ordered physical options owned only by an
    /// AfterShuffle Stratagem selector. Action ordinals use a separate
    /// payload-sorted view. Empty for every other Draw stage.
    pub shuffle_candidates: Vec<HotCard>,
    /// Calculated Gamble's paired program: the frozen discard/draw count
    /// plus the frozen Sly siblings in discard order. Set only by the
    /// Gamble owned tail (#2667); `None` for every other Draw.
    pub gamble_paired: Option<GamblePaired>,
}

/// Calculated Gamble's paired discard/draw program (#2667).
///
/// `DiscardAndDraw` freezes the hand count before the discards, so the
/// paired Draw count is dynamic. The parked record carries the frozen count
/// plus the Sly siblings collected during the paired discard walk, in
/// discard order; the resume replays the Sly batch over the live-resolved
/// objects after the Draw fully returns.
///
/// Each sibling rides one trailing word (`body` uid, `meta` catalog atom).
/// The atom is not trusted for identity — the resume refuses when the live
/// card's atom no longer matches, so a transform reusing the projection uid
/// cannot inherit the captured Sly callback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GamblePaired {
    pub count: u32,
    pub sly: Vec<GambleSly>,
}

/// One frozen Calculated Gamble Sly sibling: projection uid plus the catalog
/// atom observed when the paired discard walk collected it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GambleSly {
    pub uid: u32,
    pub atom: u16,
}

/// Borrowed authenticated Draw record view.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DrawView<'a> {
    pub requested: u32,
    pub completed: u32,
    drawn: &'a [PendingWord],
    pub from_hand_draw: bool,
    pub stage: DrawStage,
    pub card_uid: Option<u32>,
    pub caller: DrawCaller,
    shuffle_candidates: &'a [PendingWord],
    gamble_paired: Option<GamblePairedView<'a>>,
}

/// Borrowed Calculated Gamble paired-program suffix: the frozen count plus
/// the raw sibling words in discard order.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GamblePairedView<'a> {
    pub count: u32,
    pub sly: &'a [PendingWord],
}

impl DrawView<'_> {
    pub(crate) fn drawn(&self) -> impl ExactSizeIterator<Item = DrawEntry> + '_ {
        self.drawn.iter().copied().map(|word| {
            let payload = word.meta & (1 << 31) != 0;
            let meta = word.meta & !(1 << 31);
            let card = HotCard {
                uid: word.body,
                atom: meta as u16,
                flags: (meta >> 16) as u16,
            };
            if payload {
                DrawEntry::Payload(card)
            } else {
                DrawEntry::Uid(card)
            }
        })
    }

    pub(crate) fn shuffle_candidates(&self) -> impl ExactSizeIterator<Item = HotCard> + '_ {
        self.shuffle_candidates.iter().copied().map(|word| HotCard {
            uid: word.body,
            atom: word.meta as u16,
            flags: (word.meta >> 16) as u16,
        })
    }

    /// The parked Calculated Gamble paired program, if the suspending body
    /// froze one. The sibling atoms are observed values, not identity: the
    /// resume rechecks each against the live card.
    pub(crate) fn gamble_paired(&self) -> Option<GamblePaired> {
        let paired = self.gamble_paired?;
        Some(GamblePaired {
            count: paired.count,
            sly: paired
                .sly
                .iter()
                .map(|word| GambleSly {
                    uid: word.body,
                    atom: word.meta as u16,
                })
                .collect(),
        })
    }

    pub(crate) fn to_owned(self) -> DrawRecord {
        DrawRecord {
            requested: self.requested,
            completed: self.completed,
            drawn: self.drawn().collect(),
            from_hand_draw: self.from_hand_draw,
            stage: self.stage,
            card_uid: self.card_uid,
            caller: self.caller,
            shuffle_candidates: self.shuffle_candidates().collect(),
            gamble_paired: self.gamble_paired(),
        }
    }
}

/// Exact body cursor retained by one resumable potion wrapper.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PotionBodyStage {
    Synchronous = 0,
    Selecting = 1,
    AfterChild = 2,
    AfterShuffle = 3,
    /// A skippable one-of-three generated-card choice. The option payload is
    /// a frozen ordered list of catalog atoms, not physical-card UIDs: no
    /// generated object exists until an answer is accepted.
    GenerationSelecting = 4,
    /// Orobic Acid's serial plural generated-card callback cursor.
    OrobicAfterChild = 5,
    /// Ashwater's current Exhaust listener walk has completed Feel No Pain
    /// before entering Dark Embrace's potentially resumable Draw.
    AshwaterAfterFnp = 6,
}

impl PotionBodyStage {
    fn from_ordinal(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Synchronous),
            1 => Some(Self::Selecting),
            2 => Some(Self::AfterChild),
            3 => Some(Self::AfterShuffle),
            4 => Some(Self::GenerationSelecting),
            5 => Some(Self::OrobicAfterChild),
            6 => Some(Self::AshwaterAfterFnp),
            _ => None,
        }
    }
}

/// Frames-owned immutable potion option snapshot and wrapper stage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PotionFinishRecord {
    pub name: PotionId,
    pub stage: crate::frame::PotionFinishStage,
    pub body_stage: PotionBodyStage,
    /// Closed body-local physical uid, used only by Ashwater's serial
    /// Exhaust await. `None` is canonical for every other potion stage.
    pub current_uid: Option<u32>,
    /// Number of leading candidate entries owned by the current-body local
    /// tuple (Ashwater's frozen Midnight snapshot).
    pub aux: u32,
    pub candidates: Vec<HotCard>,
    /// Frozen ordered level-zero generated-card identities for one of the
    /// four skippable generation potions.
    pub generation_options: Vec<CardAtom>,
}

/// Borrowed authenticated view of a potion finish record.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PotionFinishView<'a> {
    pub name: PotionId,
    pub stage: crate::frame::PotionFinishStage,
    pub body_stage: PotionBodyStage,
    pub current_uid: Option<u32>,
    pub aux: u32,
    candidates: &'a [PendingWord],
    generation_options: &'a [PendingWord],
}

impl PotionFinishView<'_> {
    pub(crate) fn candidates(&self) -> impl ExactSizeIterator<Item = HotCard> + '_ {
        self.candidates.iter().copied().map(|word| HotCard {
            uid: word.body,
            atom: word.meta as u16,
            flags: (word.meta >> 16) as u16,
        })
    }

    pub(crate) fn generation_options(&self) -> impl ExactSizeIterator<Item = CardAtom> + '_ {
        self.generation_options
            .iter()
            .map(|word| word.body as CardAtom)
    }

    pub(crate) fn to_owned(self) -> PotionFinishRecord {
        PotionFinishRecord {
            name: self.name,
            stage: self.stage,
            body_stage: self.body_stage,
            current_uid: self.current_uid,
            aux: self.aux,
            candidates: self.candidates().collect(),
            generation_options: self.generation_options().collect(),
        }
    }
}

/// A stable word-record marker. `u32::MAX` is the sole absent value.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct WordRecordIndex(u32);

impl WordRecordIndex {
    const NONE: Self = Self(u32::MAX);

    #[cfg(test)]
    pub(crate) const fn from_raw_for_test(value: u32) -> Self {
        Self(value)
    }

    fn from_offset(offset: usize) -> Option<Self> {
        let value: u32 = offset.try_into().ok()?;
        (value != u32::MAX).then_some(Self(value))
    }

    fn offset(self) -> Option<usize> {
        (self != Self::NONE).then_some(self.0 as usize)
    }
}

/// The exact resumable stage of one persistent `CardPlayFrame`.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum CardPlayStage {
    StartBody = 0,
    Body = 1,
    AfterBody = 2,
    AfterImitation = 3,
    AfterEnchantment = 4,
    AfterNormalHook = 5,
}

impl CardPlayStage {
    fn from_ordinal(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::StartBody,
            1 => Self::Body,
            2 => Self::AfterBody,
            3 => Self::AfterImitation,
            4 => Self::AfterEnchantment,
            5 => Self::AfterNormalHook,
            _ => return None,
        })
    }
}

/// Closed source vocabulary retained by Python's `CardPlayFrame.source`.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum CardPlaySource {
    Manual = 0,
    Auto = 1,
    Hellraiser = 2,
    Stampede = 3,
    DrawPileFlip = 4,
    SlyDiscard = 5,
    BeatDown = 6,
    Catastrophe = 7,
    Eidolon = 8,
    KnifeTrap = 9,
    Uproar = 10,
    Decisions = 11,
}

impl CardPlaySource {
    fn from_ordinal(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Manual,
            1 => Self::Auto,
            2 => Self::Hellraiser,
            3 => Self::Stampede,
            4 => Self::DrawPileFlip,
            5 => Self::SlyDiscard,
            6 => Self::BeatDown,
            7 => Self::Catastrophe,
            8 => Self::Eidolon,
            9 => Self::KnifeTrap,
            10 => Self::Uproar,
            11 => Self::Decisions,
            _ => return None,
        })
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum CardPlayChoiceKind {
    None = 0,
    SignedInt = 1,
    HotCard = 2,
}

impl CardPlayChoiceKind {
    fn from_ordinal(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::None,
            1 => Self::SignedInt,
            2 => Self::HotCard,
            _ => return None,
        })
    }
}

#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum CardPlayLatchKind {
    Empty = 0,
    Raw = 1,
    Post = 2,
}

/// One physical player-card listener frozen before nested Imitation AutoPlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PhysicalAfterCardPlayedListener {
    pub uid: u32,
    pub id: CardId,
}

/// One frozen native-order listener in the admitted AutoPost snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutoPostListener {
    /// The unique owner-local Stampede power model.
    Stampede,
    /// One exact physical Howl from Beyond listener captured by uid.
    Howl { uid: u32 },
    /// One physical I Am Invincible draw-top listener.
    Invincible { uid: u32 },
}

impl AutoPostListener {
    pub(crate) fn card_uid(self) -> Option<u32> {
        match self {
            Self::Stampede => None,
            Self::Howl { uid } | Self::Invincible { uid } => Some(uid),
        }
    }
}

/// Owned exact payload for the admitted `PhaseFrame` quotient.
///
/// R19 intentionally admits only `auto_post` / `normal` / `end_turn` with
/// no remaining subphases. Those constants are therefore represented by the
/// record kind itself rather than repeated in every word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AutoPostPhaseRecord {
    /// Completed frozen-listener count.
    pub cursor: u32,
    /// The one native-order snapshot: Stampede power, then physical cards in frozen pile order.
    pub listeners: Vec<AutoPostListener>,
}

/// Borrowed exact view of one admitted AutoPost phase record.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AutoPostPhaseView<'a> {
    /// Completed frozen-listener count.
    pub cursor: u32,
    listener_words: &'a [PendingWord],
}

impl<'a> AutoPostPhaseView<'a> {
    pub(crate) fn listener_count(self) -> usize {
        self.listener_words.len()
    }

    pub(crate) fn listener(self, index: usize) -> Option<AutoPostListener> {
        self.listeners().nth(index)
    }

    /// Decode the frozen listener suffix in native order.
    pub(crate) fn listeners(self) -> impl Iterator<Item = AutoPostListener> + 'a {
        self.listener_words.iter().map(|word| match word.meta {
            0 => AutoPostListener::Stampede,
            1 => AutoPostListener::Howl { uid: word.body },
            2 => AutoPostListener::Invincible { uid: word.body },
            _ => unreachable!("validated AutoPost listener word"),
        })
    }

    pub(crate) fn to_owned(self) -> AutoPostPhaseRecord {
        AutoPostPhaseRecord {
            cursor: self.cursor,
            listeners: self.listeners().collect(),
        }
    }
}

/// The one parked AutoPre phase required by the restricted Mayhem quotient.
///
/// The canonical frame has already precommitted its sole normal listener and
/// retains only the empty late subphase plus the ordinary-actions terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AutoPreMayhemPhaseRecord;

/// Owned AutoPre/Early snapshot of stable physical Bombardment uids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AutoPreBombardmentPhaseRecord {
    pub cursor: u32,
    pub listeners: Vec<u32>,
}

/// Borrowed exact view over the existing Frames word arena.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AutoPreBombardmentPhaseView<'a> {
    pub cursor: u32,
    listener_words: &'a [PendingWord],
}

impl<'a> AutoPreBombardmentPhaseView<'a> {
    pub(crate) fn listeners(self) -> impl Iterator<Item = u32> + 'a {
        self.listener_words.iter().map(|word| word.body)
    }

    pub(crate) fn to_owned(self) -> AutoPreBombardmentPhaseRecord {
        AutoPreBombardmentPhaseRecord {
            cursor: self.cursor,
            listeners: self.listener_words.iter().map(|word| word.body).collect(),
        }
    }
}

/// Owned exact payload for Stampede's one admitted `LiveListenerFrame`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StampedeLiveRecord {
    /// Completed loop-iteration count.
    pub cursor: u32,
    /// Python's canonical invocation-amount witness.
    ///
    /// Current native execution live-reads the power Amount at every
    /// back-edge. R19 admits only the quotient in which no child can replace
    /// or modify that power model, so this witness must keep agreeing with the
    /// live amount and is never used as a substitute for the native read.
    pub iterations: u32,
}

/// Owned exact payload for Catastrophe's bounded live Draw-query loop.
///
/// Native re-reads the active physical Catastrophe after each awaited child,
/// so the continuation stores only the completed-iteration cursor and the
/// source UID. The current-build maximum is always three; the live card level
/// decides whether iteration two is entered after a prior child returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CatastropheLiveRecord {
    pub cursor: u32,
    pub source_uid: u32,
}

/// Closed provenance retained by the authenticated frozen AutoBatch wire.
///
/// The first two variants are the shared Draw/Sly command owners. The three
/// card-specific variants are the only current native bodies that snapshot a
/// plural physical-card collection before serial AutoPlay. Singleton live
/// owners (Catastrophe and Uproar) retain their own parent cursor instead of
/// smuggling an unrelated batch source into this vocabulary. Decisions is a
/// fixed sequence of three awaited commands on the same physical reference,
/// represented by three identical capture payloads; all other sources require
/// distinct physical UIDs.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrozenAutoBatchSource {
    DrawPileFlip = 0,
    SlyDiscard = 1,
    BeatDown = 2,
    Eidolon = 3,
    KnifeTrap = 4,
    /// DecisionsDecisions 0x3977c0: three full commands, one captured object.
    Decisions = 5,
}

impl FrozenAutoBatchSource {
    fn from_ordinal(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::DrawPileFlip),
            1 => Some(Self::SlyDiscard),
            2 => Some(Self::BeatDown),
            3 => Some(Self::Eidolon),
            4 => Some(Self::KnifeTrap),
            5 => Some(Self::Decisions),
            _ => None,
        }
    }

    pub(crate) const fn force_exhaust_is_exact(self, force_exhaust: bool) -> bool {
        match self {
            Self::DrawPileFlip => true,
            Self::SlyDiscard | Self::BeatDown | Self::KnifeTrap | Self::Decisions => !force_exhaust,
            Self::Eidolon => force_exhaust,
        }
    }

    pub(crate) const fn stop_on_ending(self) -> bool {
        matches!(self, Self::SlyDiscard | Self::BeatDown | Self::KnifeTrap)
    }
}

/// One complete capture-time physical card in a frozen AutoPlay batch.
///
/// The card-state snapshot is deliberately owned by the continuation arena.
/// A later child can mutate or remove this uid before the batch reaches it;
/// projection must still emit the object Python froze when the batch was
/// gathered rather than rereading the current global side table.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct FrozenAutoBatchEntry {
    pub card: HotCard,
    pub state: CardInstanceState,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TurnStartHandChoiceKind {
    Loop = 0,
    RollingBoulder = 1,
    SummonNextTurn = 2,
    Inferno = 3,
    CrimsonMantle = 4,
    ToolsOfTheTrade = 5,
    Tyranny = 6,
    Entropy = 7,
}

impl TurnStartHandChoiceKind {
    fn from_ordinal(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Loop),
            1 => Some(Self::RollingBoulder),
            2 => Some(Self::SummonNextTurn),
            3 => Some(Self::Inferno),
            4 => Some(Self::CrimsonMantle),
            5 => Some(Self::ToolsOfTheTrade),
            6 => Some(Self::Tyranny),
            7 => Some(Self::Entropy),
            _ => None,
        }
    }

    pub(crate) const fn power(self) -> PowerId {
        match self {
            Self::Loop => PowerId::Loop,
            Self::RollingBoulder => PowerId::RollingBoulder,
            Self::SummonNextTurn => PowerId::SummonNextTurn,
            Self::Inferno => PowerId::Inferno,
            Self::CrimsonMantle => PowerId::CrimsonMantle,
            Self::ToolsOfTheTrade => PowerId::ToolsOfTheTrade,
            Self::Tyranny => PowerId::Tyranny,
            Self::Entropy => PowerId::Entropy,
        }
    }

    pub(crate) const fn from_power(power: PowerId) -> Option<Self> {
        match power {
            PowerId::Loop => Some(Self::Loop),
            PowerId::RollingBoulder => Some(Self::RollingBoulder),
            PowerId::SummonNextTurn => Some(Self::SummonNextTurn),
            PowerId::Inferno => Some(Self::Inferno),
            PowerId::CrimsonMantle => Some(Self::CrimsonMantle),
            PowerId::ToolsOfTheTrade => Some(Self::ToolsOfTheTrade),
            PowerId::Tyranny => Some(Self::Tyranny),
            PowerId::Entropy => Some(Self::Entropy),
            _ => None,
        }
    }

    pub(crate) const fn selects_hand(self) -> bool {
        matches!(self, Self::ToolsOfTheTrade | Self::Tyranny | Self::Entropy)
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TurnStartHandChoicePhase {
    Selecting = 0,
    Effect = 1,
}

impl TurnStartHandChoicePhase {
    fn from_ordinal(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Selecting),
            1 => Some(Self::Effect),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TurnStartHandChoiceRecord {
    pub listener_order: Vec<TurnStartHandChoiceKind>,
    pub appended_suffix: Vec<TurnStartHandChoiceKind>,
    pub kind: TurnStartHandChoiceKind,
    pub amount: u32,
    pub next_listener: u32,
    pub cursor: u32,
    pub phase: TurnStartHandChoicePhase,
    pub entries: Vec<FrozenAutoBatchEntry>,
}

/// The one represented suspended `BeforeHandDraw` owner: Foregone Conclusion.
///
/// `listeners` is the complete native snapshot, with `cursor` already past
/// Foregone when the player is choosing. `candidates` is the native modal
/// view (rarity then ModelId), frozen only after its possible shuffle and
/// nested AfterShuffle work completed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BeforeHandDrawPowerRecord {
    pub listeners: Vec<PowerId>,
    pub cursor: u32,
    pub amount: u32,
    pub candidates: Vec<FrozenAutoBatchEntry>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct BeforeHandDrawPowerView<'a> {
    pub cursor: u32,
    pub amount: u32,
    listener_words: &'a [PendingWord],
    candidate_words: &'a [PendingWord],
    candidate_count: usize,
}

impl<'a> BeforeHandDrawPowerView<'a> {
    pub(crate) fn listeners(self) -> impl ExactSizeIterator<Item = PowerId> + 'a {
        self.listener_words.iter().map(|word| {
            PowerId::ALL
                .get(word.body as usize)
                .copied()
                .expect("validated power")
        })
    }

    pub(crate) fn candidates(self) -> impl ExactSizeIterator<Item = FrozenAutoBatchEntry> + 'a {
        FrozenAutoBatchEntryIter {
            words: self.candidate_words,
            remaining: self.candidate_count,
        }
    }

    pub(crate) fn to_owned(self) -> BeforeHandDrawPowerRecord {
        BeforeHandDrawPowerRecord {
            listeners: self.listeners().collect(),
            cursor: self.cursor,
            amount: self.amount,
            candidates: self.candidates().collect(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct TurnStartHandChoiceView<'a> {
    pub listener_order: [Option<TurnStartHandChoiceKind>; 8],
    pub listener_count: usize,
    pub appended_suffix: [Option<TurnStartHandChoiceKind>; 8],
    pub appended_suffix_count: usize,
    pub kind: TurnStartHandChoiceKind,
    pub amount: u32,
    pub next_listener: u32,
    pub cursor: u32,
    pub phase: TurnStartHandChoicePhase,
    entry_words: &'a [PendingWord],
    entry_count: usize,
}

impl<'a> TurnStartHandChoiceView<'a> {
    pub(crate) fn entries(self) -> impl ExactSizeIterator<Item = FrozenAutoBatchEntry> + 'a {
        FrozenAutoBatchEntryIter {
            words: self.entry_words,
            remaining: self.entry_count,
        }
    }

    pub(crate) fn order(self) -> std::vec::IntoIter<TurnStartHandChoiceKind> {
        self.listener_order[..self.listener_count]
            .iter()
            .copied()
            .flatten()
            .collect::<Vec<_>>()
            .into_iter()
    }

    pub(crate) fn appended(self) -> std::vec::IntoIter<TurnStartHandChoiceKind> {
        self.appended_suffix[..self.appended_suffix_count]
            .iter()
            .copied()
            .flatten()
            .collect::<Vec<_>>()
            .into_iter()
    }

    pub(crate) fn to_owned(self) -> TurnStartHandChoiceRecord {
        TurnStartHandChoiceRecord {
            listener_order: self.order().collect(),
            appended_suffix: self.appended().collect(),
            kind: self.kind,
            amount: self.amount,
            next_listener: self.next_listener,
            cursor: self.cursor,
            phase: self.phase,
            entries: self.entries().collect(),
        }
    }
}

impl FrozenAutoBatchEntry {
    pub(crate) fn from_current(card: HotCard, card_states: &CardStates) -> Self {
        Self {
            card,
            state: card_states.get(card.uid),
        }
    }
}

/// Owned frozen-card payload for one restricted `FrozenAutoBatchFrame`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FrozenAutoBatchRecord {
    pub entries: Vec<FrozenAutoBatchEntry>,
    pub cursor: u32,
    /// Nonzero only while Distilled Chaos is still gathering its fixed
    /// draw-top batch. The value is the native Repeat bound (three); the
    /// already-gathered prefix lives in `entries`, and playback begins only
    /// after this field is cleared.
    pub gather_target: u8,
    pub force_exhaust: bool,
    pub source: FrozenAutoBatchSource,
}

impl FrozenAutoBatchRecord {
    pub(crate) fn from_current(
        cards: &[HotCard],
        card_states: &CardStates,
        cursor: u32,
        force_exhaust: bool,
        source: FrozenAutoBatchSource,
    ) -> Self {
        Self {
            entries: cards
                .iter()
                .copied()
                .map(|card| FrozenAutoBatchEntry::from_current(card, card_states))
                .collect(),
            cursor,
            gather_target: 0,
            force_exhaust,
            source,
        }
    }
}

/// Closed initiating action retained by an independently rooted replay frame.
///
/// This mirrors the external actions that can start a frozen replay
/// trajectory. The potion form keeps exact belt-slot and target identity in
/// the already-paid two action words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActionReplayRootAction {
    Play {
        uid: u32,
        target: Option<u8>,
        selection_uid: Option<u32>,
    },
    UsePotion {
        slot: u8,
        target: Option<u8>,
    },
    EndTurn,
}

/// One typed external selection answer already consumed by a replayed action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActionReplayAnswer {
    CardUid(u32),
    OptionIndex(u32),
}

/// Word ranges of one validated closed replay record; see
/// `Frames::action_replay_layout`.
struct ActionReplayLayout {
    action: ActionReplayRootAction,
    json: std::ops::Range<usize>,
    predecessor_len: usize,
    answers: std::ops::Range<usize>,
}

/// Owned depth-one replay witness for a whole external action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActionReplayRecord {
    /// Exact canonical predecessor bytes, with canonical zero padding in the
    /// word arena. The predecessor is structurally continuation-free.
    pub predecessor_json: Vec<u8>,
    pub action: ActionReplayRootAction,
    pub answers: Vec<ActionReplayAnswer>,
}

/// Exact suspended suffix of the enemy-side ACT snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EnemyPhaseRecord {
    pub actor_uid: u32,
    pub stage: u8,
    pub remaining_uids: Vec<u32>,
}

/// Borrowed authenticated view of one arena-backed AutoBatch record.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FrozenAutoBatchView<'a> {
    pub cursor: u32,
    pub gather_target: u8,
    pub force_exhaust: bool,
    pub source: FrozenAutoBatchSource,
    entry_words: &'a [PendingWord],
    entry_count: usize,
}

impl<'a> FrozenAutoBatchView<'a> {
    pub(crate) fn entries(self) -> impl ExactSizeIterator<Item = FrozenAutoBatchEntry> + 'a {
        FrozenAutoBatchEntryIter {
            words: self.entry_words,
            remaining: self.entry_count,
        }
    }

    pub(crate) fn cards(self) -> impl ExactSizeIterator<Item = HotCard> + 'a {
        self.entries().map(|entry| entry.card)
    }

    pub(crate) fn uids(self) -> impl ExactSizeIterator<Item = u32> + 'a {
        self.entries().map(|entry| entry.card.uid)
    }

    pub(crate) fn uid(self, index: usize) -> Option<u32> {
        self.entries().nth(index).map(|entry| entry.card.uid)
    }

    pub(crate) fn len(self) -> usize {
        self.entry_count
    }

    pub(crate) fn to_owned(self) -> FrozenAutoBatchRecord {
        FrozenAutoBatchRecord {
            entries: self.entries().collect(),
            cursor: self.cursor,
            gather_target: self.gather_target,
            force_exhaust: self.force_exhaust,
            source: self.source,
        }
    }
}

struct FrozenAutoBatchEntryIter<'a> {
    words: &'a [PendingWord],
    remaining: usize,
}

impl Iterator for FrozenAutoBatchEntryIter<'_> {
    type Item = FrozenAutoBatchEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }
        let (entry, consumed) = Frames::decode_frozen_auto_batch_entry(self.words)?;
        self.words = self.words.get(consumed..)?;
        self.remaining -= 1;
        Some(entry)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for FrozenAutoBatchEntryIter<'_> {}

impl CardPlayLatchKind {
    fn from_ordinal(value: u8) -> Option<Self> {
        Some(match value {
            0 => Self::Empty,
            1 => Self::Raw,
            2 => Self::Post,
            _ => return None,
        })
    }
}

/// Owned exact payload used to append or replace one persistent CardPlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CardPlayRecord {
    pub uid: u32,
    pub stage: CardPlayStage,
    pub source: CardPlaySource,
    pub result_route: CardResultRoute,
    pub choice_kind: CardPlayChoiceKind,
    pub choice_i32: i32,
    pub first_choice: Option<HotCard>,
    pub target: Option<(i32, u32, Option<i32>)>,
    pub latch_kind: CardPlayLatchKind,
    pub pen_double: bool,
    pub force_exhaust: bool,
    pub is_power_auto: bool,
    pub active_play_member: bool,
    pub pending_choice: bool,
    pub lamp_owner: bool,
    pub plays: u32,
    pub play_index: u32,
    pub x_value: i64,
    pub spent: i32,
    pub star_spent: u32,
    pub glam: u32,
    pub next_step: u32,
    pub afterimage: i32,
    pub subroutine: i32,
    pub storm: i32,
    pub serpent: i32,
    pub rupture_batch: i32,
    pub panache: bool,
    pub rupture_registered: bool,
    pub calamity: bool,
    pub monologue: bool,
    pub tender: bool,
    pub selection_amount: i32,
    pub selection_kind: Option<PendingSelectionKind>,
    pub source_pile: PileId,
    pub expos: Option<u32>,
    pub selection_cards: Vec<HotCard>,
    pub oblivion_latches: Vec<(u32, i32)>,
    pub strangle_latches: Vec<(u32, i32)>,
    /// Monologue's per-instance BeforeCardPlayed dictionary values, keyed by
    /// native power-object identity in acquisition order.
    pub monologue_latches: Vec<(u32, i32)>,
    /// Shared object-uid allocator watermark captured immediately before
    /// BeforeCardPlayed, plus the one Panache/Monologue uid created by this
    /// CardPlay body (if any).  The pair authenticates both Monologue's full
    /// pre-body dictionary and Panache's otherwise-private fresh false bit.
    pub instanced_power_uid_at_before: Option<u32>,
    pub instanced_power_created_uid: Option<u32>,
    /// The ordinary AfterCardPlayed hook-entry snapshot. Uids share the
    /// monotonic Panache/Monologue object-identity domain. `cursor` is the
    /// number of callbacks entered; a pending Monologue uid identifies the
    /// one callback whose awaited Strength command has not yet committed its
    /// private ledger suffix.
    pub after_card_played_power_snapshot: Vec<u32>,
    pub after_card_played_power_snapshot_captured: bool,
    pub after_card_played_power_cursor: u32,
    pub after_card_played_power_phase: u8,
    pub after_card_played_pending_monologue_uid: Option<u32>,
    pub physical_after_card_played_listeners: Vec<PhysicalAfterCardPlayedListener>,
}

#[cfg(test)]
impl CardPlayRecord {
    pub(crate) fn pending_for_test(uid: u32) -> Self {
        Self {
            uid,
            stage: CardPlayStage::Body,
            source: CardPlaySource::Manual,
            result_route: CardResultRoute::DiscardBottom,
            choice_kind: CardPlayChoiceKind::None,
            choice_i32: 0,
            first_choice: None,
            target: None,
            latch_kind: CardPlayLatchKind::Raw,
            pen_double: false,
            force_exhaust: false,
            is_power_auto: false,
            active_play_member: true,
            pending_choice: true,
            lamp_owner: false,
            plays: 1,
            play_index: 0,
            x_value: 0,
            spent: 0,
            star_spent: 0,
            glam: 0,
            next_step: 1,
            afterimage: 0,
            subroutine: 0,
            storm: 0,
            serpent: 0,
            rupture_batch: 0,
            panache: false,
            rupture_registered: false,
            calamity: false,
            monologue: false,
            tender: false,
            selection_amount: 0,
            selection_kind: Some(PendingSelectionKind::Program),
            source_pile: PileId::Play,
            expos: None,
            selection_cards: Vec::new(),
            oblivion_latches: Vec::new(),
            strangle_latches: Vec::new(),
            monologue_latches: Vec::new(),
            instanced_power_uid_at_before: None,
            instanced_power_created_uid: None,
            after_card_played_power_snapshot: Vec::new(),
            after_card_played_power_snapshot_captured: false,
            after_card_played_power_cursor: 0,
            after_card_played_power_phase: 0,
            after_card_played_pending_monologue_uid: None,
            physical_after_card_played_listeners: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CardPlayView<'a> {
    pub uid: u32,
    pub stage: CardPlayStage,
    pub source: CardPlaySource,
    pub result_route: CardResultRoute,
    pub choice_kind: CardPlayChoiceKind,
    pub choice_i32: i32,
    pub first_choice: Option<HotCard>,
    pub target: Option<(i32, u32, Option<i32>)>,
    pub latch_kind: CardPlayLatchKind,
    pub pen_double: bool,
    pub force_exhaust: bool,
    pub is_power_auto: bool,
    pub active_play_member: bool,
    pub pending_choice: bool,
    pub lamp_owner: bool,
    pub plays: u32,
    pub play_index: u32,
    pub x_value: i64,
    pub spent: i32,
    pub star_spent: u32,
    pub glam: u32,
    pub next_step: u32,
    pub afterimage: i32,
    pub subroutine: i32,
    pub storm: i32,
    pub serpent: i32,
    pub rupture_batch: i32,
    pub panache: bool,
    pub rupture_registered: bool,
    pub calamity: bool,
    pub monologue: bool,
    pub tender: bool,
    pub after_card_played_power_cursor: u32,
    pub after_card_played_power_phase: u8,
    pub after_card_played_power_snapshot_captured: bool,
    pub after_card_played_pending_monologue_uid: Option<u32>,
    pub instanced_power_uid_at_before: Option<u32>,
    pub instanced_power_created_uid: Option<u32>,
    pub selection_amount: i32,
    pub selection_kind: Option<PendingSelectionKind>,
    pub source_pile: PileId,
    pub expos: Option<u32>,
    selection_words: &'a [PendingWord],
    oblivion_words: &'a [PendingWord],
    strangle_words: &'a [PendingWord],
    monologue_words: &'a [PendingWord],
    after_card_played_power_words: &'a [PendingWord],
    physical_listener_words: &'a [PendingWord],
}

impl<'a> CardPlayView<'a> {
    pub(crate) fn route(
        self,
    ) -> Result<(PileId, PendingSelectionKind), PendingSelectionRoutingError> {
        self.selection_kind
            .map(|kind| (self.source_pile, kind))
            .ok_or(PendingSelectionRoutingError(0xff))
    }

    pub(crate) fn selection_cards(self) -> &'a [HotCard] {
        // SAFETY: `HotCard` and `PendingWord` are both `repr(C)`, eight-byte,
        // four-byte-aligned pairs whose second word is exactly the packed
        // `(atom, flags)` halves. The authenticated SelectionCards subrecord
        // is written only by `PendingWord::from_card`.
        unsafe {
            std::slice::from_raw_parts(
                self.selection_words.as_ptr().cast(),
                self.selection_words.len(),
            )
        }
    }

    pub(crate) fn strangle_latches(self) -> impl ExactSizeIterator<Item = (u32, i32)> + 'a {
        self.strangle_words
            .iter()
            .copied()
            .map(PendingWord::strangle)
    }

    pub(crate) fn oblivion_latches(self) -> impl ExactSizeIterator<Item = (u32, i32)> + 'a {
        self.oblivion_words
            .iter()
            .copied()
            .map(PendingWord::strangle)
    }

    pub(crate) fn monologue_latches(self) -> impl ExactSizeIterator<Item = (u32, i32)> + 'a {
        self.monologue_words
            .iter()
            .copied()
            .map(PendingWord::strangle)
    }

    pub(crate) fn after_card_played_power_snapshot(
        self,
    ) -> impl ExactSizeIterator<Item = u32> + 'a {
        self.after_card_played_power_words
            .iter()
            .map(|word| word.body)
    }

    pub(crate) fn physical_after_card_played_listeners(
        self,
    ) -> impl ExactSizeIterator<Item = PhysicalAfterCardPlayedListener> + 'a {
        self.physical_listener_words.iter().copied().map(|word| {
            word.physical_listener()
                .expect("authenticated physical-listener word")
        })
    }

    pub(crate) fn to_owned(self) -> CardPlayRecord {
        CardPlayRecord {
            uid: self.uid,
            stage: self.stage,
            source: self.source,
            result_route: self.result_route,
            choice_kind: self.choice_kind,
            choice_i32: self.choice_i32,
            first_choice: self.first_choice,
            target: self.target,
            latch_kind: self.latch_kind,
            pen_double: self.pen_double,
            force_exhaust: self.force_exhaust,
            is_power_auto: self.is_power_auto,
            active_play_member: self.active_play_member,
            pending_choice: self.pending_choice,
            lamp_owner: self.lamp_owner,
            plays: self.plays,
            play_index: self.play_index,
            x_value: self.x_value,
            spent: self.spent,
            star_spent: self.star_spent,
            glam: self.glam,
            next_step: self.next_step,
            afterimage: self.afterimage,
            subroutine: self.subroutine,
            storm: self.storm,
            serpent: self.serpent,
            rupture_batch: self.rupture_batch,
            panache: self.panache,
            rupture_registered: self.rupture_registered,
            calamity: self.calamity,
            monologue: self.monologue,
            tender: self.tender,
            selection_amount: self.selection_amount,
            selection_kind: self.selection_kind,
            source_pile: self.source_pile,
            expos: self.expos,
            selection_cards: self.selection_cards().to_vec(),
            oblivion_latches: self.oblivion_latches().collect(),
            strangle_latches: self.strangle_latches().collect(),
            monologue_latches: self.monologue_latches().collect(),
            instanced_power_uid_at_before: self.instanced_power_uid_at_before,
            instanced_power_created_uid: self.instanced_power_created_uid,
            after_card_played_power_snapshot: self.after_card_played_power_snapshot().collect(),
            after_card_played_power_snapshot_captured: self
                .after_card_played_power_snapshot_captured,
            after_card_played_power_cursor: self.after_card_played_power_cursor,
            after_card_played_power_phase: self.after_card_played_power_phase,
            after_card_played_pending_monologue_uid: self.after_card_played_pending_monologue_uid,
            physical_after_card_played_listeners: self
                .physical_after_card_played_listeners()
                .collect(),
        }
    }
}

/// One local Energy-cost operation, in native append order.
///
/// `CardEnergyCost.LocalCostModifier` stores `(type, amount, expiration,
/// reduce_only)` rows. `Set` and `Add` do not commute, so this is deliberately
/// a sequence rather than a folded scalar.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct LocalCostModifier {
    /// Absolute replacement or relative addition.
    pub kind: LocalCostModifierKind,
    /// The signed integer operand.
    pub amount: i64,
    /// When the row is removed.
    pub expiration: LocalCostExpiration,
    /// Keep the lower of the incoming and candidate values.
    pub reduce_only: bool,
}

/// The two native local Energy-cost operations.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum LocalCostModifierKind {
    /// `LOCAL_COST_SET = 1`.
    Set = 1,
    /// `LOCAL_COST_ADD = 2`.
    Add = 2,
}

impl LocalCostModifierKind {
    /// Decode the canonical integer wire value.
    pub fn from_wire(value: i64) -> Option<Self> {
        match value {
            1 => Some(Self::Set),
            2 => Some(Self::Add),
            _ => None,
        }
    }
}

/// Native local-cost expiration flags.
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum LocalCostExpiration {
    /// `LOCAL_COST_THIS_COMBAT = 0`.
    ThisCombat = 0,
    /// `LOCAL_COST_THIS_TURN = 2`.
    ThisTurn = 2,
    /// `LOCAL_COST_UNTIL_PLAYED = 4`.
    UntilPlayed = 4,
    /// Both turn and played cleanup remove the row.
    ThisTurnOrPlayed = 6,
}

impl LocalCostExpiration {
    /// Decode the canonical integer wire value.
    pub fn from_wire(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::ThisCombat),
            2 => Some(Self::ThisTurn),
            4 => Some(Self::UntilPlayed),
            6 => Some(Self::ThisTurnOrPlayed),
            _ => None,
        }
    }

    /// Whether the native bitmask contains `flag`.
    fn contains(self, flag: Self) -> bool {
        (self as u8 & flag as u8) != 0
    }
}

/// One card's ordered, copy-on-write local-cost rows.
///
/// The list is independently shared inside the already-copy-on-write card
/// side table. Cloning a search state still increments only the side-table
/// pointer; a row list is copied only when that exact card is modified.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct CardInstanceAux {
    rows: Vec<LocalCostModifier>,
    thrash_growth: Option<crate::decimal::DotNetDecimalBits>,
    thrash_growth_is_fraction: bool,
    free_star_cost_this_combat: bool,
    // Empty keeps the common combat-prefix/transient-suffix encoding compact.
    // Nonempty preserves the complete native order for interleaved/repeated rows.
    star_expirations: Vec<LocalCostExpiration>,
    local_ethereal: bool,
    transient_sly: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct LocalCostModifiers(Arc<CardInstanceAux>);

impl Default for LocalCostModifiers {
    fn default() -> Self {
        // Missing side-table reads manufacture a default CardInstanceState
        // frequently. Share its empty immutable payload so adding Thrash's
        // cold Decimal slot does not increase ordinary allocation bytes.
        static EMPTY: LazyLock<Arc<CardInstanceAux>> =
            LazyLock::new(|| Arc::new(CardInstanceAux::default()));
        Self(Arc::clone(&EMPTY))
    }
}

impl LocalCostModifiers {
    /// Build a list from canonical order.
    pub fn from_rows(rows: Vec<LocalCostModifier>) -> Self {
        Self(Arc::new(CardInstanceAux {
            rows,
            thrash_growth: None,
            thrash_growth_is_fraction: false,
            free_star_cost_this_combat: false,
            star_expirations: Vec::new(),
            local_ethereal: false,
            transient_sly: false,
        }))
    }

    /// The rows in native application order.
    pub fn as_slice(&self) -> &[LocalCostModifier] {
        self.0.rows.as_slice()
    }

    /// Whether the ordered Energy-cost row list is empty.
    ///
    /// This deliberately does not inspect the independent Star-cost list;
    /// use [`Self::free_star_cost_this_combat`] for its combat-long prefix.
    pub fn is_empty(&self) -> bool {
        self.0.rows.is_empty()
    }

    /// Whether the exact Star-cost list contains a combat-long absolute-zero row.
    pub fn free_star_cost_this_combat(&self) -> bool {
        self.0.free_star_cost_this_combat
    }

    /// Exact ordered native absolute-zero Star rows, including repeated and
    /// interleaved combat-long writes from Touch of Insanity.
    pub(crate) fn star_cost_expirations(
        &self,
        transient: u8,
    ) -> impl Iterator<Item = LocalCostExpiration> + '_ {
        let compact = self.0.star_expirations.is_empty();
        std::iter::repeat_n(
            LocalCostExpiration::ThisCombat,
            usize::from(compact && self.0.free_star_cost_this_combat),
        )
        .chain(std::iter::repeat_n(
            LocalCostExpiration::ThisTurnOrPlayed,
            if compact { usize::from(transient) } else { 0 },
        ))
        .chain(self.0.star_expirations.iter().copied())
    }

    /// Import/canonicalize one native Star list without dropping duplicate rows.
    pub(crate) fn set_star_cost_expirations(&mut self, rows: &[LocalCostExpiration]) -> Option<u8> {
        if rows.iter().any(|row| {
            !matches!(
                row,
                LocalCostExpiration::ThisCombat | LocalCostExpiration::ThisTurnOrPlayed
            )
        }) {
            return None;
        }
        let transient = u8::try_from(
            rows.iter()
                .filter(|row| **row == LocalCostExpiration::ThisTurnOrPlayed)
                .count(),
        )
        .ok()?;
        let combat = rows.len() - usize::from(transient);
        let compact =
            combat == 0 || combat == 1 && rows.first() == Some(&LocalCostExpiration::ThisCombat);
        let payload = Arc::make_mut(&mut self.0);
        payload.free_star_cost_this_combat = combat > 0;
        payload.star_expirations = if compact { Vec::new() } else { rows.to_vec() };
        Some(transient)
    }

    fn star_encoding_is_exact(&self, transient: u8) -> bool {
        if self.0.star_expirations.is_empty() {
            return true;
        }
        let rows = &self.0.star_expirations;
        let combat = rows
            .iter()
            .filter(|row| **row == LocalCostExpiration::ThisCombat)
            .count();
        let temporary = rows
            .iter()
            .filter(|row| **row == LocalCostExpiration::ThisTurnOrPlayed)
            .count();
        combat + temporary == rows.len()
            && temporary == usize::from(transient)
            && self.0.free_star_cost_this_combat
            && (combat > 1 || combat == 1 && rows.first() != Some(&LocalCostExpiration::ThisCombat))
    }

    /// Replace only Energy rows while preserving every cold slot-7 member.
    ///
    /// Adaptive Strike rewrites a cloned Energy-cost list after the source
    /// may already carry Metamorphosis's combat-long Star row. Rebuilding the
    /// COW carrier from rows would silently discard that independent native
    /// list member.
    pub(crate) fn replace_rows(&mut self, rows: Vec<LocalCostModifier>) {
        Arc::make_mut(&mut self.0).rows = rows;
    }

    /// Initialize a fresh combat-long Star prefix. Existing physical cards
    /// append through CardStates, which preserves their full ordered list.
    pub(crate) fn set_free_star_cost_this_combat(&mut self) -> Option<()> {
        let payload = Arc::make_mut(&mut self.0);
        if payload.free_star_cost_this_combat {
            return None;
        }
        payload.free_star_cost_this_combat = true;
        Some(())
    }

    fn exact_damage_growth(&self) -> Option<crate::decimal::DotNetDecimal> {
        self.0
            .thrash_growth
            .map(crate::decimal::DotNetDecimal::from_bits)?
            .ok()
    }

    fn set_exact_damage_growth(&mut self, value: crate::decimal::DotNetDecimal, is_fraction: bool) {
        let payload = Arc::make_mut(&mut self.0);
        payload.thrash_growth = Some(value.bits());
        payload.thrash_growth_is_fraction = is_fraction;
    }

    fn clear_exact_damage_growth(&mut self) {
        if self.0.thrash_growth.is_none() {
            return;
        }
        let payload = Arc::make_mut(&mut self.0);
        payload.thrash_growth = None;
        payload.thrash_growth_is_fraction = false;
    }

    /// Append one operation without folding or reordering it.
    pub(crate) fn push(&mut self, modifier: LocalCostModifier) {
        Arc::make_mut(&mut self.0).rows.push(modifier);
    }

    /// Remove rows whose native expiration bitmask contains `flag`.
    pub(crate) fn cleanup(&mut self, flag: LocalCostExpiration) {
        if self.0.rows.iter().any(|row| row.expiration.contains(flag)) {
            Arc::make_mut(&mut self.0)
                .rows
                .retain(|row| !row.expiration.contains(flag));
        }
        if self
            .0
            .star_expirations
            .iter()
            .any(|expiration| expiration.contains(flag))
        {
            let rows: Vec<_> = self
                .0
                .star_expirations
                .iter()
                .copied()
                .filter(|expiration| !expiration.contains(flag))
                .collect();
            self.set_star_cost_expirations(&rows)
                .expect("removing Star rows preserves representability");
        }
    }

    /// Clamp native `Set` rows after a printed-cost-lowering upgrade.
    ///
    /// `CardEnergyCost.UpgradeBy` preserves row order and every non-`Set`
    /// field, but lowers an absolute replacement above the new printed base.
    /// Keeping the operation on this type prevents upgrade callers from
    /// rebuilding or folding the non-commutative row list.
    pub(crate) fn clamp_sets_above(&mut self, new_base: i64) {
        if self
            .0
            .rows
            .iter()
            .any(|row| row.kind == LocalCostModifierKind::Set && row.amount > new_base)
        {
            for row in &mut Arc::make_mut(&mut self.0).rows {
                if row.kind == LocalCostModifierKind::Set && row.amount > new_base {
                    row.amount = new_base;
                }
            }
        }
    }

    /// Apply every row in append order without the global cost pipeline's
    /// final nonnegative clamp.
    ///
    /// Global energy-cost hooks need the signed result because a negative
    /// local value bypasses their walk entirely. Callers that only need the
    /// historical local projection should use [`Self::resolve`].
    pub(crate) fn resolve_signed(&self, base: i64) -> i64 {
        let mut value = i128::from(base);
        for row in self.as_slice() {
            let candidate = match row.kind {
                LocalCostModifierKind::Set => i128::from(row.amount),
                LocalCostModifierKind::Add => value + i128::from(row.amount),
            };
            value = if row.reduce_only {
                value.min(candidate)
            } else {
                candidate
            };
        }
        value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
    }

    /// Apply every row in append order, then the native nonnegative clamp.
    pub fn resolve(&self, base: i64) -> i64 {
        self.resolve_signed(base).max(0)
    }
}

/// Genetic Algorithm's exact per-instance mutable state, packed into one
/// word so the generic side-table row grows by one machine word rather than
/// by separate `i32` and `Option<u32>` fields.
///
/// Bits 0..=30 store nonnegative `IncreasedBlock`; bits 31..=62 store the
/// optional master-deck row; bit 63 records row presence. The separate card
/// flag records whether the opaque tail exists at all, so zero remains the
/// valid explicit `(0, null)` payload.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct GeneticAlgorithmState(u64);

impl GeneticAlgorithmState {
    const GROWTH_MASK: u64 = (1_u64 << 31) - 1;
    const ROW_SHIFT: u32 = 31;
    const ROW_PRESENT: u64 = 1_u64 << 63;

    /// Build one exactly representable native payload.
    pub fn from_parts(growth: i32, deck_row: Option<u32>) -> Option<Self> {
        if growth < 0 {
            return None;
        }
        let row = deck_row.map_or(0, |value| u64::from(value) << Self::ROW_SHIFT);
        let present = deck_row.map_or(0, |_| Self::ROW_PRESENT);
        Some(Self(u64::from(growth as u32) | row | present))
    }

    /// Accumulated `IncreasedBlock` for this combat object.
    pub fn growth(self) -> i32 {
        (self.0 & Self::GROWTH_MASK) as i32
    }

    /// Stable master-deck row for a linked original, or `None` for a mutable
    /// clone/generated instance.
    pub fn deck_row(self) -> Option<u32> {
        (self.0 & Self::ROW_PRESENT != 0)
            .then(|| ((self.0 >> Self::ROW_SHIFT) & u64::from(u32::MAX)) as u32)
    }

    /// Replace only accumulated growth, retaining the exact DeckVersion link.
    pub fn with_growth(self, growth: i32) -> Option<Self> {
        Self::from_parts(growth, self.deck_row())
    }

    /// Native `MutableClone` retains local growth but clears `DeckVersion`.
    pub fn without_deck_row(self) -> Self {
        Self::from_parts(self.growth(), None).expect("stored growth is nonnegative")
    }
}

/// Four-byte carrier for an optional nonnegative enchantment amount.
///
/// Zero means absent and every present amount is stored one greater. This
/// preserves both `Some(0)` and `Some(i32::MAX)` while recovering four bytes
/// from Rust's eight-byte `Option<i32>` representation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct OptionalNonNegativeI32(u32);

impl OptionalNonNegativeI32 {
    /// Encode an optional amount, refusing negative values.
    pub fn from_option(value: Option<i32>) -> Option<Self> {
        match value {
            None => Some(Self(0)),
            Some(value) if value >= 0 => Some(Self((value as u32) + 1)),
            Some(_) => None,
        }
    }

    /// Decode the exact optional amount.
    pub fn get(self) -> Option<i32> {
        (self.0 != 0).then(|| (self.0 - 1) as i32)
    }

    /// Whether an amount is present, including explicit zero.
    pub fn is_some(self) -> bool {
        self.0 != 0
    }
}

/// Four-byte carrier for any physical card's nullable BaseReplayCount.
///
/// Native's absent tuple member and explicit zero compare differently. Raw
/// zero is absent, raw negative one is explicit zero, and a positive raw value
/// is the exact present count. No other negative state can be constructed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BaseReplayCount(i32);

impl BaseReplayCount {
    /// Encode a nullable nonnegative replay count.
    pub fn from_option(value: Option<i32>) -> Option<Self> {
        match value {
            None => Some(Self(0)),
            Some(0) => Some(Self(-1)),
            Some(value) if value > 0 => Some(Self(value)),
            Some(_) => None,
        }
    }

    /// Decode the exact nullable count, preserving explicit zero.
    pub fn get(self) -> Option<i32> {
        match self.0 {
            0 => None,
            -1 => Some(0),
            value if value > 0 => Some(value),
            _ => unreachable!("BaseReplayCount has no public raw constructor"),
        }
    }
}

/// Mutable per-instance card data.
///
/// Ethereal, Retain, and permanent Sly are the modeled combat-local keywords;
/// Retain is also the one modeled transient keyword. Sovereign Blade's mutable damage
/// is the modeled slot-6 payload. Generic nullable BaseReplayCount is the sixth
/// member of the native slot-7 physical-state tuple.
/// Other keyword kinds, unsupported Star rows and the opaque tail remain
/// outside this slice and fail closed at the boundary.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CardInstanceState {
    /// Physical slot 5: the mutable Momentum/Vigorous counter
    /// (`combat_sim.card_enchantment_state`). Its enchantment id is fixed by
    /// the card's identity in the catalog, so only the amount is stored.
    pub enchantment_state: OptionalNonNegativeI32,
    /// Physical BaseReplayCount, with absent distinct from explicit zero.
    pub(crate) base_replay_count: BaseReplayCount,
    /// Physical slot 7's nonnegative per-instance damage-growth payload.
    pub damage_growth: i32,
    /// Physical slot 7's ordered local Energy-cost operations.
    pub local_cost_modifiers: LocalCostModifiers,
    /// Number of exact temporary Star-cost rows `(0, true, true)` written by
    /// `SetToFreeThisTurn`. Native appends rather than folds these rows, so a
    /// replayed Bullet Time must preserve their cardinality. Every row expires
    /// at turn end or when played; fixed negative Star costs ignore them.
    pub free_star_cost_this_turn_or_played_rows: u8,
    /// Genetic Algorithm's accumulated Block and nullable DeckVersion row.
    pub genetic_algorithm: GeneticAlgorithmState,
    /// Combat-local `CardKeyword.Retain` on this physical instance.
    pub local_retain: bool,
    /// Combat-local `CardKeyword.Sly` on this physical instance.
    pub local_sly: bool,
    /// Turn-scoped `CardKeyword.Retain` on this physical instance.
    pub transient_retain: bool,
}

impl CardInstanceState {
    pub(crate) fn transient_sly(&self) -> bool {
        self.local_cost_modifiers.0.transient_sly
    }

    pub(crate) fn set_transient_sly(&mut self, value: bool) {
        if self.transient_sly() != value {
            Arc::make_mut(&mut self.local_cost_modifiers.0).transient_sly = value;
        }
    }

    /// Native CardModel.get_IsSlyThisTurn (0x7cd5d): canonical/local Sly or
    /// HasSingleTurnSly. Callers supply the immutable canonical keyword.
    pub(crate) fn is_sly(&self, canonical: bool) -> bool {
        canonical || self.local_sly || self.transient_sly()
    }

    /// Lexicographic order of the canonical sorted Retain/Sly tuple.
    pub(crate) fn transient_keyword_rank(&self) -> u8 {
        match (self.transient_retain, self.transient_sly()) {
            (false, false) => 0,
            (true, false) => 1,
            (true, true) => 2,
            (false, true) => 3,
        }
    }

    pub(crate) fn local_ethereal(&self) -> bool {
        self.local_cost_modifiers.0.local_ethereal
    }

    pub(crate) fn set_local_ethereal(&mut self, value: bool) {
        Arc::make_mut(&mut self.local_cost_modifiers.0).local_ethereal = value;
    }
    /// Exact nullable physical BaseReplayCount, preserving explicit zero.
    pub fn base_replay_count(&self) -> Option<i32> {
        self.base_replay_count.get()
    }

    /// Set nullable physical BaseReplayCount, refusing negative values.
    pub fn set_base_replay_count(&mut self, value: Option<i32>) -> Option<()> {
        self.base_replay_count = BaseReplayCount::from_option(value)?;
        Some(())
    }

    /// Exact slot-7 damage growth, including Thrash's retained Decimal.
    pub(crate) fn exact_damage_growth(&self) -> Option<crate::decimal::DotNetDecimal> {
        match self.local_cost_modifiers.exact_damage_growth() {
            None => Some(crate::decimal::DotNetDecimal::from_i64(i64::from(
                self.damage_growth,
            ))),
            Some(value) if self.damage_growth == 0 => Some(value),
            Some(_) => None,
        }
    }

    /// Whether slot 7 uses Thrash's cold Decimal carrier rather than Int32.
    pub(crate) fn has_exact_damage_growth_aux(&self) -> bool {
        self.local_cost_modifiers.0.thrash_growth.is_some()
    }

    /// Whether the canonical slot was a Python Fraction/Decimal rather than
    /// an integer. This remains significant when the denominator is one.
    pub(crate) fn exact_damage_growth_is_fraction(&self) -> bool {
        self.local_cost_modifiers.0.thrash_growth.is_some()
            && self.local_cost_modifiers.0.thrash_growth_is_fraction
    }

    /// Replace slot-7 growth without widening the frozen side-table row.
    pub(crate) fn set_exact_damage_growth(
        &mut self,
        value: crate::decimal::DotNetDecimal,
    ) -> Option<()> {
        if value < crate::decimal::DotNetDecimal::zero() {
            return None;
        }
        let bits = value.bits();
        if bits.negative {
            return None;
        }
        if bits.scale == 0
            && bits.hi == 0
            && bits.mid == 0
            && let Ok(integer) = i32::try_from(bits.lo)
        {
            self.damage_growth = integer;
            self.local_cost_modifiers.clear_exact_damage_growth();
        } else {
            self.damage_growth = 0;
            self.local_cost_modifiers
                .set_exact_damage_growth(value, false);
        }
        Some(())
    }

    /// Replace slot-7 growth with a native Decimal/Fraction value, preserving
    /// Decimal provenance even when its reduced denominator is one.
    pub(crate) fn set_fraction_damage_growth(
        &mut self,
        value: crate::decimal::DotNetDecimal,
    ) -> Option<()> {
        let bits = value.bits();
        if value < crate::decimal::DotNetDecimal::zero() || bits.negative {
            return None;
        }
        self.damage_growth = 0;
        self.local_cost_modifiers
            .set_exact_damage_growth(value, true);
        Some(())
    }

    /// Whether this instance carries nothing and needs no side-table entry.
    pub fn is_vacant(&self) -> bool {
        *self == Self::default()
    }

    /// Lexicographic rank of Python's sorted local-keyword tuple.
    ///
    /// The modeled vocabulary is Ethereal/Retain/Sly. These eight ranks are
    /// precisely Python's lexicographic order over every sorted subset.
    pub(crate) fn local_keyword_rank(&self) -> u8 {
        match (self.local_ethereal(), self.local_retain, self.local_sly) {
            (false, false, false) => 0,
            (true, false, false) => 1,
            (true, true, false) => 2,
            (true, true, true) => 3,
            (true, false, true) => 4,
            (false, true, false) => 5,
            (false, true, true) => 6,
            (false, false, true) => 7,
        }
    }
}

/// One ordered, copy-on-write card pile.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct HotPile(Arc<Vec<HotCard>>);

impl HotPile {
    /// An empty pile.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from an ordered card list.
    pub fn from_cards(cards: Vec<HotCard>) -> Self {
        Self(Arc::new(cards))
    }

    /// The ordered cards. Order is load-bearing everywhere.
    pub fn as_slice(&self) -> &[HotCard] {
        self.0.as_slice()
    }

    /// Number of cards.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the pile holds no cards.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Mutable access, cloning shared storage exactly once.
    pub fn make_mut(&mut self) -> &mut Vec<HotCard> {
        Arc::make_mut(&mut self.0)
    }
}

/// The five piles, indexed by [`PileId`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct HotPiles([HotPile; PileId::COUNT]);

impl HotPiles {
    /// One pile.
    pub fn get(&self, pile: PileId) -> &HotPile {
        &self.0[pile as usize]
    }

    /// One pile, mutably.
    pub fn get_mut(&mut self, pile: PileId) -> &mut HotPile {
        &mut self.0[pile as usize]
    }

    /// Replace one pile wholesale.
    pub fn set(&mut self, pile: PileId, cards: HotPile) {
        self.0[pile as usize] = cards;
    }
}

/// One complete persistent Thieving Hopper master-deck row.
///
/// `card.uid` is the native DeckVersion row ordinal as well as the combat
/// object's initial physical uid. The full card and instance payload are
/// retained because the sole stolen row is absent from all five combat piles.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct HopperDeckRow {
    pub card: HotCard,
    pub state: CardInstanceState,
}

/// Native MapPointHistory loot facts for the one fixed Hopper theft.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum HopperLootKind {
    Stolen,
    Returned,
}

/// One exact ordered loot-history entry, including the absent row payload.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct HopperLootEvent {
    pub kind: HopperLootKind,
    pub row: HopperDeckRow,
}

/// Hopper's cold persistent DeckVersion quotient.
///
/// The deterministic encounter can steal at most one row. Its history is
/// therefore exactly empty, `Stolen(row)`, or `Stolen(row), Returned(row)`;
/// admission authenticates that closed grammar and the live Swipe owner.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct HopperDeckState {
    pub master: Vec<HopperDeckRow>,
    pub history: Vec<HopperLootEvent>,
}

/// One physical card downgraded by Magi Knight's shared Dampen power.
///
/// Native stores the original upgrade level in insertion order. The current
/// admitted catalog has only L0/L1 rows, but retaining the exact byte keeps
/// the cold state faithful and makes a widened generated table fail closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DampenCardRow {
    pub uid: u32,
    pub old_level: u8,
    /// Exact pre-Dampen atom, retained so the native AfterDeath listener can
    /// restore without reopening a catalogless public damage/death seam.
    pub old_atom: CardAtom,
    /// Exact L0 atom installed by the first application. Transform/remove
    /// paths are refused while tracked; this pins that proof at restoration.
    pub dampened_atom: CardAtom,
}

/// Dampen's one-caster, ordered downgrade snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DampenState {
    pub caster_uid: u32,
    pub cards: Vec<DampenCardRow>,
}

/// Bolas/Thrumming Hatchet's exact prior-owner-turn physical identity state.
///
/// Native records completed plays by `CardModel` reference, rolls that set at
/// the next owner turn, and freezes the live all-piles listener enumeration
/// before any `BeforeHandDraw` body runs.  Stable physical UIDs are the
/// canonical reference witness.  The three vectors retain insertion/pile
/// order because both replay de-duplication and listener execution order are
/// observable.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct SelfReturnState {
    pub current_turn: Vec<u32>,
    pub previous_turn: Vec<u32>,
    pub before_hand_draw: Vec<u32>,
}

impl SelfReturnState {
    pub(crate) fn is_empty(&self) -> bool {
        self.current_turn.is_empty()
            && self.previous_turn.is_empty()
            && self.before_hand_draw.is_empty()
    }
}

/// The owner-disjoint cold encounter-card payload.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum EncounterCardState {
    Hopper(HopperDeckState),
    Dampen(DampenState),
}

/// The cold allocation behind [`CardStates`]. Hopper and Dampen are encounter
/// disjoint, so one tagged optional shared arc carries either payload without
/// changing the one-word hot handle or the fixed 32-byte per-card row.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct CardStateStore {
    /// Removed DUPE Strike objects retained only by an in-flight Draw.
    removed_draw: Vec<FrozenAutoBatchEntry>,
    entries: Vec<(u32, CardInstanceState)>,
    encounter: Option<Arc<EncounterCardState>>,
    /// Cross-encounter-card-family state that can coexist with Hopper or
    /// Dampen without widening the one-word [`CardStates`] hot handle.
    self_return: Option<Arc<SelfReturnState>>,
    /// The Scythe's non-Hopper `DeckVersion` links, `(uid, deck_row)`
    /// ascending by uid (#2941). See [`CardStates::scythe_deck_links`].
    scythe_links: Option<Arc<Vec<(u32, u32)>>>,
}

/// The copy-on-write side table of modified card instances, ascending by uid.
///
/// Most states modify no card, and that case is one pointer and one atomic
/// increment. Lookup is a binary search; there is no ordered map and no
/// per-card allocation.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct CardStates(Arc<CardStateStore>);

impl CardStates {
    /// An empty side table.
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn shares_store_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    #[cfg(test)]
    pub(crate) fn self_return_shares_store_with(&self, other: &Self) -> bool {
        match (&self.0.self_return, &other.0.self_return) {
            (Some(left), Some(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    /// Build from an arbitrary list, sorting by uid.
    ///
    /// Returns `None` on a repeated uid or a vacant entry: both would make
    /// the table's meaning depend on insertion order.
    pub fn from_unsorted(mut entries: Vec<(u32, CardInstanceState)>) -> Option<Self> {
        entries.sort_unstable_by_key(|entry| entry.0);
        if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return None;
        }
        if entries.iter().any(|entry| entry.1.is_vacant()) {
            return None;
        }
        Some(Self(Arc::new(CardStateStore {
            entries,
            removed_draw: Vec::new(),
            encounter: None,
            self_return: None,
            scythe_links: None,
        })))
    }

    /// Sparse retained Draw objects, separate from the live instance census.
    pub(crate) fn removed_draw_objects(&self) -> &[FrozenAutoBatchEntry] {
        &self.0.removed_draw
    }

    pub(crate) fn removed_draw_object(&self, uid: u32) -> Option<&FrozenAutoBatchEntry> {
        self.0
            .removed_draw
            .binary_search_by_key(&uid, |entry| entry.card.uid)
            .ok()
            .map(|index| &self.0.removed_draw[index])
    }

    pub(crate) fn removed_draw_object_mut(
        &mut self,
        uid: u32,
    ) -> Option<&mut FrozenAutoBatchEntry> {
        let index = self
            .0
            .removed_draw
            .binary_search_by_key(&uid, |entry| entry.card.uid)
            .ok()?;
        Some(&mut Arc::make_mut(&mut self.0).removed_draw[index])
    }

    /// Caller authenticates removal provenance and absence from live piles.
    pub(crate) fn retain_removed_draw_object(&mut self, entry: FrozenAutoBatchEntry) -> Option<()> {
        match self
            .0
            .removed_draw
            .binary_search_by_key(&entry.card.uid, |row| row.card.uid)
        {
            Ok(index) => (self.0.removed_draw[index] == entry).then_some(()),
            Err(index) => {
                Arc::make_mut(&mut self.0).removed_draw.insert(index, entry);
                Some(())
            }
        }
    }

    pub(crate) fn prune_removed_draw_objects(&mut self, retained: &[u32]) {
        if self
            .0
            .removed_draw
            .iter()
            .all(|entry| retained.contains(&entry.card.uid))
        {
            return;
        }
        Arc::make_mut(&mut self.0)
            .removed_draw
            .retain(|entry| retained.contains(&entry.card.uid));
    }

    /// Ascending entries.
    pub fn as_slice(&self) -> &[(u32, CardInstanceState)] {
        self.0.entries.as_slice()
    }

    /// Whether no card is modified — the common case.
    pub fn is_empty(&self) -> bool {
        self.0.entries.is_empty()
            && self.0.removed_draw.is_empty()
            && self.0.encounter.is_none()
            && self.0.self_return.is_none()
            && self.0.scythe_links.is_none()
    }

    /// Number of modified cards.
    pub fn len(&self) -> usize {
        self.0.entries.len()
    }

    /// The instance data for `uid`, defaulted when the card is unmodified.
    pub fn get(&self, uid: u32) -> CardInstanceState {
        match self.0.entries.binary_search_by_key(&uid, |entry| entry.0) {
            Ok(index) => self.0.entries[index].1.clone(),
            Err(_) => CardInstanceState::default(),
        }
    }

    /// Borrow the live entry for a uid without incrementing an inner list's
    /// reference count.
    pub fn get_ref(&self, uid: u32) -> Option<&CardInstanceState> {
        self.0
            .entries
            .binary_search_by_key(&uid, |entry| entry.0)
            .ok()
            .map(|index| &self.0.entries[index].1)
    }

    /// Write `uid`; a vacant value removes the entry.
    pub fn set(&mut self, uid: u32, state: CardInstanceState) {
        let entries = &mut Arc::make_mut(&mut self.0).entries;
        match entries.binary_search_by_key(&uid, |entry| entry.0) {
            Ok(index) => {
                if state.is_vacant() {
                    entries.remove(index);
                } else {
                    entries[index].1 = state;
                }
            }
            Err(index) => {
                if !state.is_vacant() {
                    entries.insert(index, (uid, state));
                }
            }
        }
    }

    /// Append one ordered local-cost row to an exact physical card.
    pub fn append_local_cost_modifier(&mut self, uid: u32, modifier: LocalCostModifier) {
        let entries = &mut Arc::make_mut(&mut self.0).entries;
        match entries.binary_search_by_key(&uid, |entry| entry.0) {
            Ok(index) => entries[index].1.local_cost_modifiers.push(modifier),
            Err(index) => {
                let mut state = CardInstanceState::default();
                state.local_cost_modifiers.push(modifier);
                entries.insert(index, (uid, state));
            }
        }
    }

    /// Append native `CardModel.SetToFreeThisTurn` state to one exact card.
    ///
    /// The Energy setter returns before writing when the unmodified base is
    /// negative; the Star setter always appends its row. Callers decide
    /// whether an Energy-X card is eligible for the outer mechanic.
    pub fn set_to_free_this_turn(&mut self, uid: u32, energy_base: i64) -> Option<()> {
        let mut state = self.get(uid);
        let rows = state
            .free_star_cost_this_turn_or_played_rows
            .checked_add(1)?;
        if energy_base >= 0 {
            state.local_cost_modifiers.push(LocalCostModifier {
                kind: LocalCostModifierKind::Set,
                amount: 0,
                expiration: LocalCostExpiration::ThisTurnOrPlayed,
                reduce_only: false,
            });
        }
        if !state.local_cost_modifiers.0.star_expirations.is_empty() {
            Arc::make_mut(&mut state.local_cost_modifiers.0)
                .star_expirations
                .push(LocalCostExpiration::ThisTurnOrPlayed);
        }
        state.free_star_cost_this_turn_or_played_rows = rows;
        self.set(uid, state);
        Some(())
    }

    /// Append native CardModel.SetToFreeThisCombat (RVA 0x7d3df) without moving it ahead of earlier rows.
    /// Energy skips negative printed costs; Star always appends absolute zero.
    pub fn set_to_free_this_combat(&mut self, uid: u32, energy_base: i64) -> Option<()> {
        let mut state = self.get(uid);
        if state.free_star_cost_this_turn_or_played_rows != 0
            || state.local_cost_modifiers.free_star_cost_this_combat()
        {
            let mut rows: Vec<_> = state
                .local_cost_modifiers
                .star_cost_expirations(state.free_star_cost_this_turn_or_played_rows)
                .collect();
            rows.push(LocalCostExpiration::ThisCombat);
            state
                .local_cost_modifiers
                .set_star_cost_expirations(&rows)?;
        } else {
            state
                .local_cost_modifiers
                .set_free_star_cost_this_combat()?;
        }
        if energy_base >= 0 {
            state.local_cost_modifiers.push(LocalCostModifier {
                kind: LocalCostModifierKind::Set,
                amount: 0,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            });
        }
        self.set(uid, state);
        Some(())
    }

    /// Append Snecko Oil's temporary absolute Energy-cost row.
    pub(crate) fn set_snecko_oil_cost(&mut self, uid: u32, amount: i64) -> Option<()> {
        if !(0..=3).contains(&amount) {
            return None;
        }
        let mut state = self.get(uid);
        state.local_cost_modifiers.push(LocalCostModifier {
            kind: LocalCostModifierKind::Set,
            amount,
            expiration: LocalCostExpiration::ThisTurnOrPlayed,
            reduce_only: false,
        });
        self.set(uid, state);
        Some(())
    }

    /// Append Confused's combat-long absolute Energy-cost row.
    pub(crate) fn set_confused_cost(&mut self, uid: u32, amount: i64) -> Option<()> {
        if !(0..=3).contains(&amount) {
            return None;
        }
        let mut state = self.get(uid);
        state.local_cost_modifiers.push(LocalCostModifier {
            kind: LocalCostModifierKind::Set,
            amount,
            expiration: LocalCostExpiration::ThisCombat,
            reduce_only: false,
        });
        self.set(uid, state);
        Some(())
    }

    /// Set one exact instance's combat-local Retain keyword.
    pub fn set_local_retain(&mut self, uid: u32) {
        let mut state = self.get(uid);
        state.local_retain = true;
        self.set(uid, state);
    }

    /// Set one exact instance's combat-local Sly keyword.
    pub fn set_local_sly(&mut self, uid: u32) {
        let mut state = self.get(uid);
        state.local_sly = true;
        self.set(uid, state);
    }

    /// Set one exact instance's turn-scoped Retain keyword.
    pub fn set_transient_retain(&mut self, uid: u32) {
        let mut state = self.get(uid);
        state.transient_retain = true;
        self.set(uid, state);
    }

    /// CardCmd.ApplySingleTurnSly (0x12fe38) delegates to
    /// CardModel.GiveSingleTurnSly (0x7d549), a physical-instance boolean write.
    pub fn set_transient_sly(&mut self, uid: u32) {
        let mut state = self.get(uid);
        state.set_transient_sly(true);
        self.set(uid, state);
    }

    /// CardModel.EndOfTurnCleanup (0x7dbc4) clears both single-turn keywords
    /// after the hand flush, including cards outside Hand.
    pub fn cleanup_transient_keywords(&mut self) {
        if !self
            .0
            .entries
            .iter()
            .any(|entry| entry.1.transient_retain || entry.1.transient_sly())
        {
            return;
        }
        let entries = &mut Arc::make_mut(&mut self.0).entries;
        for (_, state) in entries.iter_mut() {
            state.transient_retain = false;
            state.set_transient_sly(false);
        }
        entries.retain(|(_, state)| !state.is_vacant());
    }

    /// Add one nonnegative physical damage-growth delta to an exact card.
    ///
    /// Python integers are unbounded. `None` therefore means the delta was
    /// negative or the represented `i32` range would overflow; callers turn
    /// that into a typed refusal instead of wrapping or partially writing.
    pub fn add_damage_growth(&mut self, uid: u32, delta: i32) -> Option<i32> {
        if delta < 0 {
            return None;
        }
        match self.0.entries.binary_search_by_key(&uid, |entry| entry.0) {
            Ok(index) => {
                if self.0.entries[index]
                    .1
                    .local_cost_modifiers
                    .0
                    .thrash_growth
                    .is_some()
                {
                    return None;
                }
                let growth = self.0.entries[index].1.damage_growth.checked_add(delta)?;
                let entries = &mut Arc::make_mut(&mut self.0).entries;
                entries[index].1.damage_growth = growth;
                Some(growth)
            }
            Err(index) => {
                if delta > 0 {
                    let state = CardInstanceState {
                        damage_growth: delta,
                        ..CardInstanceState::default()
                    };
                    let entries = &mut Arc::make_mut(&mut self.0).entries;
                    entries.insert(index, (uid, state));
                }
                Some(delta)
            }
        }
    }

    /// Run native local-cost cleanup over every modified card.
    pub fn cleanup_local_cost_modifiers(&mut self, flag: LocalCostExpiration) {
        let clears_temporary_star = matches!(
            flag,
            LocalCostExpiration::ThisTurn | LocalCostExpiration::UntilPlayed
        );
        let has_matching_row = self.0.entries.iter().any(|entry| {
            entry
                .1
                .local_cost_modifiers
                .as_slice()
                .iter()
                .any(|row| row.expiration.contains(flag))
                || (clears_temporary_star && entry.1.free_star_cost_this_turn_or_played_rows != 0)
        });
        if !has_matching_row {
            return;
        }
        let entries = &mut Arc::make_mut(&mut self.0).entries;
        for (_, state) in entries.iter_mut() {
            state.local_cost_modifiers.cleanup(flag);
            if clears_temporary_star {
                state.free_star_cost_this_turn_or_played_rows = 0;
            }
        }
        entries.retain(|(_, state)| !state.is_vacant());
    }

    /// Run native local-cost cleanup for one routed card.
    pub fn cleanup_card_local_cost_modifiers(
        &mut self,
        uid: u32,
        flag: LocalCostExpiration,
    ) -> bool {
        let Some(index) = self
            .0
            .entries
            .binary_search_by_key(&uid, |entry| entry.0)
            .ok()
        else {
            return false;
        };
        let clears_temporary_star = matches!(
            flag,
            LocalCostExpiration::ThisTurn | LocalCostExpiration::UntilPlayed
        );
        let has_matching_row = self.0.entries[index]
            .1
            .local_cost_modifiers
            .as_slice()
            .iter()
            .any(|row| row.expiration.contains(flag))
            || (clears_temporary_star
                && self.0.entries[index]
                    .1
                    .free_star_cost_this_turn_or_played_rows
                    != 0);
        if !has_matching_row {
            return false;
        }
        let entries = &mut Arc::make_mut(&mut self.0).entries;
        entries[index].1.local_cost_modifiers.cleanup(flag);
        if clears_temporary_star {
            entries[index].1.free_star_cost_this_turn_or_played_rows = 0;
        }
        if entries[index].1.is_vacant() {
            entries.remove(index);
        }
        true
    }

    /// The optional Hopper-only cold DeckVersion quotient.
    pub(crate) fn hopper(&self) -> Option<&HopperDeckState> {
        match self.0.encounter.as_deref() {
            Some(EncounterCardState::Hopper(deck)) => Some(deck),
            _ => None,
        }
    }

    /// Mutable Hopper quotient, cloning both shared layers only when needed.
    pub(crate) fn hopper_mut(&mut self) -> Option<&mut HopperDeckState> {
        match Arc::make_mut(Arc::make_mut(&mut self.0).encounter.as_mut()?) {
            EncounterCardState::Hopper(deck) => Some(deck),
            EncounterCardState::Dampen(_) => None,
        }
    }

    /// Replace Hopper's complete cold quotient atomically.
    pub(crate) fn set_hopper(&mut self, value: Option<HopperDeckState>) {
        Arc::make_mut(&mut self.0).encounter =
            value.map(|deck| Arc::new(EncounterCardState::Hopper(deck)));
    }

    /// The optional Dampen-only cold downgrade snapshot.
    pub(crate) fn dampen(&self) -> Option<&DampenState> {
        match self.0.encounter.as_deref() {
            Some(EncounterCardState::Dampen(state)) => Some(state),
            _ => None,
        }
    }

    /// Number of cards in the exact Dampen snapshot (smoke/evidence only).
    pub fn dampen_tracked_len(&self) -> usize {
        self.dampen().map_or(0, |state| state.cards.len())
    }

    /// Replace Dampen's complete cold quotient atomically.
    pub(crate) fn set_dampen(&mut self, value: Option<DampenState>) {
        Arc::make_mut(&mut self.0).encounter =
            value.map(|state| Arc::new(EncounterCardState::Dampen(state)));
    }

    /// The Scythe's `DeckVersion` links in an ordinary (non-Hopper) fight,
    /// `(uid, deck_row)` ascending by uid (#2941).
    ///
    /// `Player::PopulateCombatState` RVA `0x117a90` links every combat copy,
    /// a `CloneCard` of its run-deck object, back to that object
    /// (IL_002c-IL_0036 `set_DeckVersion`), and the master growth the canonical
    /// document spells as `scythe_deck_growth` is that object's
    /// `IncreasedDamage`. It always equals the linked live copy's growth:
    /// the combat copy is a clone of the deck card, and the only writer,
    /// `TheScythe/<OnPlay>d__15::MoveNext` RVA `0x3c2ed0`, applies the same
    /// `Increase` local (IL_00d3-IL_00e8) to the live copy (`BuffFromPlay`
    /// IL_00eb) and to its `DeckVersion` (IL_00f0-IL_0102). So only the link
    /// is stored; the growth is read from the live copy. Keyed by uid, the
    /// link does not follow a clone, exactly as `CardModel::AfterCloned`
    /// RVA `0x7d31c` nulls `DeckVersion` (IL_0039-IL_003b). A Thieving
    /// Hopper fight carries its links as master rows instead
    /// ([`Self::hopper`]), and the two never coexist.
    pub(crate) fn scythe_deck_links(&self) -> &[(u32, u32)] {
        self.0.scythe_links.as_deref().map_or(&[], Vec::as_slice)
    }

    /// The `DeckVersion` row `uid` is linked to, if any (#2941).
    pub(crate) fn scythe_deck_row(&self, uid: u32) -> Option<u32> {
        let links = self.scythe_deck_links();
        links
            .binary_search_by_key(&uid, |link| link.0)
            .ok()
            .map(|index| links[index].1)
    }

    /// Replace the complete link list. `None` on a repeated uid or row;
    /// empty canonicalizes to absence.
    pub(crate) fn set_scythe_deck_links(&mut self, mut links: Vec<(u32, u32)>) -> Option<()> {
        links.sort_unstable_by_key(|link| link.0);
        if links.windows(2).any(|pair| pair[0].0 == pair[1].0)
            || links
                .iter()
                .enumerate()
                .any(|(index, link)| links[..index].iter().any(|prior| prior.1 == link.1))
        {
            return None;
        }
        Arc::make_mut(&mut self.0).scythe_links = (!links.is_empty()).then(|| Arc::new(links));
        Some(())
    }

    /// Borrow the exact Batch139 UID/history/listener quotient.
    pub(crate) fn self_return(&self) -> Option<&SelfReturnState> {
        self.0.self_return.as_deref()
    }

    /// Mutate the Batch139 quotient through both COW layers.
    pub(crate) fn self_return_mut(&mut self) -> &mut SelfReturnState {
        let store = Arc::make_mut(&mut self.0);
        let value = store
            .self_return
            .get_or_insert_with(|| Arc::new(SelfReturnState::default()));
        Arc::make_mut(value)
    }

    /// Replace the complete Batch139 quotient, canonicalizing empty state to
    /// absence so ordinary fights retain their prior allocation/equality
    /// profile.
    pub(crate) fn set_self_return(&mut self, value: SelfReturnState) {
        Arc::make_mut(&mut self.0).self_return = (!value.is_empty()).then(|| Arc::new(value));
    }

    /// Record one completed physical self-return card with native Any-style
    /// de-duplication across replay bodies.
    pub(crate) fn record_self_return_finished(&mut self, uid: u32) {
        let state = self.self_return_mut();
        if !state.current_turn.contains(&uid) {
            state.current_turn.push(uid);
        }
    }
}

/// A relocatable Draw-child frame segment detached from the stack (#3387,
/// [`Frames::detach_hook_segment`]). Record indices are relative to the
/// segment's first word.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct HookFrameSegment {
    frames: Vec<Frame>,
    words: Vec<PendingWord>,
}

/// One queued `GenericHookGameAction` whose body is a suspended Gremlin Horn
/// Draw (#3387, `engine::hook_action`).
///
/// Transient: it exists only between the Draw's detach and the end of the
/// same public transaction, and the boundary refuses a state carrying one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DeferredHookAction {
    segment: HookFrameSegment,
    /// The pending choice, its record index relative to the segment.
    pending: PendingSelection,
    /// The Hand and Draw piles (with each card's instance state) when the
    /// choice began: the piles native reads the option list from once the
    /// hook action starts.
    hand: Vec<(HotCard, CardInstanceState)>,
    draw: Vec<(HotCard, CardInstanceState)>,
    /// The Play cards of the segment's own suspended card plays (Hellraiser
    /// AutoPlays), in Play order: once the enclosing action has finished,
    /// the Play pile must hold exactly these (#3387).
    play: Vec<(HotCard, CardInstanceState)>,
}

/// The complete choice-bound continuation store.
///
/// Both vectors share one COW owner, so [`Frames`] remains one pointer and a
/// clone increments exactly one strong count. Vector equality ignores spare
/// capacity, keeping canonical/search equality content-based.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct FrameStore {
    frames: Vec<Frame>,
    words: Vec<PendingWord>,
}

/// The copy-on-write continuation stack and its stable word records.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Frames(Arc<FrameStore>);

impl Frames {
    /// An empty stack.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from a bottom-to-top frame list.
    pub fn from_frames(frames: Vec<Frame>) -> Self {
        Self(Arc::new(FrameStore {
            frames,
            words: Vec::new(),
        }))
    }

    /// The frames, bottom to top.
    pub fn as_slice(&self) -> &[Frame] {
        self.0.frames.as_slice()
    }

    /// Stack depth.
    pub fn len(&self) -> usize {
        self.0.frames.len()
    }

    /// Whether the stack is empty.
    pub fn is_empty(&self) -> bool {
        self.0.frames.is_empty()
    }

    /// The top frame.
    pub fn top(&self) -> Option<Frame> {
        self.0.frames.last().copied()
    }

    /// Push, cloning shared storage exactly once.
    pub fn push(&mut self, frame: Frame) {
        Arc::make_mut(&mut self.0).frames.push(frame);
    }

    /// Pop, cloning shared storage exactly once.
    pub fn pop(&mut self) -> Option<Frame> {
        Arc::make_mut(&mut self.0).frames.pop()
    }

    fn encode_card_finish(
        record: &CardFinishRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        if record.uid == u32::MAX
            || record.payload.card.uid != record.uid
            || record.routed_uid == Some(u32::MAX)
        {
            return None;
        }
        let entry_len = Self::encode_frozen_auto_batch_entry(&record.payload, None)?;
        let body_count = 3_usize.checked_add(entry_len)?;
        let encoded_len = 1_usize.checked_add(body_count)?;
        PendingWord::header(WordRecordKind::CardFinish, body_count)?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        words.push(PendingWord::header(WordRecordKind::CardFinish, body_count)?);
        words.push(PendingWord {
            body: record.uid,
            meta: record.stage as u32
                | ((record.result_route as u32) << 8)
                | ((record.source as u32) << 16)
                | (u32::from(record.is_power_auto) << 24),
        });
        words.push(PendingWord {
            body: record.routed_uid.unwrap_or(u32::MAX),
            meta: record.glam,
        });
        words.push(PendingWord { body: 0, meta: 0 });
        Self::encode_frozen_auto_batch_entry(&record.payload, Some(words))?;
        Some(encoded_len)
    }

    pub(crate) fn push_card_finish(
        &mut self,
        record: &CardFinishRecord,
    ) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_card_finish(record, None)?;
        let start = self.0.words.len();
        let index = WordRecordIndex::from_offset(start)?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_card_finish(record, Some(&mut store.words))?;
        store.frames.push(Frame::CardFinish { record: index });
        Some(index)
    }

    pub(crate) fn card_finish(&self, index: WordRecordIndex) -> Option<CardFinishRecord> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::CardFinish || count < 4 {
            return None;
        }
        let end = start.checked_add(1)?.checked_add(count as usize)?;
        let fixed = self.0.words.get(start + 1..start + 4)?;
        let control = fixed[0].meta;
        if control >> 25 != 0 || fixed[2] != (PendingWord { body: 0, meta: 0 }) {
            return None;
        }
        let stage = crate::frame::CardFinishStage::ALL
            .get((control & 0xff) as usize)
            .copied()?;
        let result_route = CardResultRoute::from_ordinal(((control >> 8) & 0xff) as u8)?;
        let source = CardPlaySource::from_ordinal(((control >> 16) & 0xff) as u8)?;
        let (payload, entry_len) =
            Self::decode_frozen_auto_batch_entry(self.0.words.get(start + 4..end)?)?;
        if start + 4 + entry_len != end {
            return None;
        }
        let record = CardFinishRecord {
            uid: fixed[0].body,
            payload,
            glam: fixed[1].meta,
            result_route,
            source,
            is_power_auto: control & (1 << 24) != 0,
            routed_uid: (fixed[1].body != u32::MAX).then_some(fixed[1].body),
            stage,
        };
        Self::encode_card_finish(&record, None)?;
        Some(record)
    }

    pub(crate) fn pop_top_card_finish(&mut self) -> Option<CardFinishRecord> {
        let Frame::CardFinish { record } = self.top()? else {
            return None;
        };
        let owned = self.card_finish(record)?;
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    fn encode_potion_finish(
        record: &PotionFinishRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        let generation_choice = record.body_stage == PotionBodyStage::GenerationSelecting
            && record.stage == crate::frame::PotionFinishStage::Effect
            && matches!(
                record.name,
                PotionId::AttackPotion
                    | PotionId::SkillPotion
                    | PotionId::PowerPotion
                    | PotionId::ColorlessPotion
            );
        let generation_child = record.body_stage == PotionBodyStage::AfterChild
            && record.stage == crate::frame::PotionFinishStage::Effect
            && matches!(
                record.name,
                PotionId::AttackPotion
                    | PotionId::SkillPotion
                    | PotionId::PowerPotion
                    | PotionId::ColorlessPotion
            )
            && record.current_uid.is_some()
            && record.aux == 0
            && record.candidates.is_empty()
            && record.generation_options.is_empty();
        let orobic_child = record.body_stage == PotionBodyStage::OrobicAfterChild
            && record.stage == crate::frame::PotionFinishStage::Effect
            && record.name == PotionId::OrobicAcid
            && record.current_uid.is_some()
            && usize::try_from(record.aux).is_ok_and(|cursor| cursor <= 3)
            && record.candidates.is_empty()
            && record.generation_options.len() == 3;
        let ashwater_child = record.name == PotionId::Ashwater
            && record.stage == crate::frame::PotionFinishStage::Effect
            && matches!(
                record.body_stage,
                PotionBodyStage::AfterChild | PotionBodyStage::AshwaterAfterFnp
            )
            && record.current_uid.is_some()
            && usize::try_from(record.aux).is_ok_and(|aux| aux <= record.candidates.len());
        let candidate_is_invalid = record.candidates.iter().enumerate().any(|(index, card)| {
            let same_segment_start =
                if ashwater_child && index >= usize::try_from(record.aux).unwrap_or(usize::MAX) {
                    usize::try_from(record.aux).unwrap_or(0)
                } else {
                    0
                };
            card.uid == u32::MAX
                || card.flags & !CARD_FLAGS_KNOWN != 0
                || record.candidates[same_segment_start..index]
                    .iter()
                    .any(|prior| prior.uid == card.uid)
        });
        let generation_options_invalid =
            record
                .generation_options
                .iter()
                .enumerate()
                .any(|(index, atom)| {
                    *atom == u16::MAX || record.generation_options[..index].contains(atom)
                });
        if record.current_uid == Some(u32::MAX)
            || candidate_is_invalid
            || generation_options_invalid
            || (record.body_stage == PotionBodyStage::Selecting
                && record.stage != crate::frame::PotionFinishStage::Effect)
            || (!ashwater_child
                && !generation_child
                && !orobic_child
                && (record.current_uid.is_some() || record.aux != 0))
            || (!record.candidates.is_empty()
                && !(record.body_stage == PotionBodyStage::Selecting
                    || record.body_stage == PotionBodyStage::AfterShuffle
                        && record.stage == crate::frame::PotionFinishStage::Effect
                        && record.name == PotionId::BottledPotential
                    || ashwater_child
                    || record.body_stage == PotionBodyStage::AfterChild
                        && record.stage == crate::frame::PotionFinishStage::Effect
                        && matches!(record.name, PotionId::GamblersBrew | PotionId::Ashwater)))
            || if generation_choice {
                record.current_uid.is_some()
                    || record.aux != 0
                    || !record.candidates.is_empty()
                    || record.generation_options.len() != 3
            } else if orobic_child {
                false
            } else {
                !record.generation_options.is_empty()
            }
        {
            return None;
        }
        let body_count = 2_usize
            .checked_add(record.candidates.len())?
            .checked_add(record.generation_options.len())?;
        let encoded_len = 1_usize.checked_add(body_count)?;
        PendingWord::header(WordRecordKind::PotionFinish, body_count)?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        words.reserve(encoded_len);
        words.push(PendingWord::header(
            WordRecordKind::PotionFinish,
            body_count,
        )?);
        words.push(PendingWord {
            body: record.name as u32,
            meta: u32::from(record.stage as u8) | (u32::from(record.body_stage as u8) << 8),
        });
        words.push(PendingWord {
            body: record.current_uid.unwrap_or(u32::MAX),
            meta: record.aux,
        });
        words.extend(
            record
                .candidates
                .iter()
                .copied()
                .map(PendingWord::from_card),
        );
        words.extend(
            record
                .generation_options
                .iter()
                .copied()
                .map(|atom| PendingWord {
                    body: u32::from(atom),
                    meta: 0,
                }),
        );
        Some(encoded_len)
    }

    pub(crate) fn push_potion_finish(
        &mut self,
        record: &PotionFinishRecord,
    ) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_potion_finish(record, None)?;
        let start = self.0.words.len();
        let final_len = start.checked_add(encoded_len)?;
        if final_len > u32::MAX as usize {
            return None;
        }
        let index = WordRecordIndex::from_offset(start)?;
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_potion_finish(record, Some(&mut store.words))?;
        store.frames.push(Frame::PotionFinish { record: index });
        Some(index)
    }

    pub(crate) fn potion_finish(&self, index: WordRecordIndex) -> Option<PotionFinishView<'_>> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::PotionFinish || count < 2 {
            return None;
        }
        let end = start.checked_add(1)?.checked_add(count as usize)?;
        let control = *self.0.words.get(start + 1)?;
        if control.body >= PotionId::COUNT as u32 || control.meta & !0x0000_ffff != 0 {
            return None;
        }
        let name = PotionId::ALL[control.body as usize];
        let stage = crate::frame::PotionFinishStage::ALL
            .get((control.meta & 0xff) as usize)
            .copied()?;
        let body_stage = PotionBodyStage::from_ordinal(((control.meta >> 8) & 0xff) as u8)?;
        let locals = *self.0.words.get(start + 2)?;
        let current_uid = (locals.body != u32::MAX).then_some(locals.body);
        let aux = locals.meta;
        let payload = self.0.words.get(start + 3..end)?;
        let (candidates, generation_options) = if matches!(
            body_stage,
            PotionBodyStage::GenerationSelecting | PotionBodyStage::OrobicAfterChild
        ) {
            if payload
                .iter()
                .any(|word| word.meta != 0 || word.body >= u16::MAX as u32)
            {
                return None;
            }
            (&[][..], payload)
        } else {
            (payload, &[][..])
        };
        let view = PotionFinishView {
            name,
            stage,
            body_stage,
            current_uid,
            aux,
            candidates,
            generation_options,
        };
        Self::encode_potion_finish(&view.to_owned(), None)
            .filter(|len| *len == 1 + count as usize)?;
        Some(view)
    }

    pub(crate) fn replace_top_potion_finish(&mut self, record: &PotionFinishRecord) -> Option<()> {
        let Frame::PotionFinish { record: index } = self.top()? else {
            return None;
        };
        self.potion_finish(index)?;
        let encoded_len = Self::encode_potion_finish(record, None)?;
        let start = index.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.words.reserve(encoded_len);
        Self::encode_potion_finish(record, Some(&mut store.words))?;
        Some(())
    }

    pub(crate) fn pop_top_potion_finish(&mut self) -> Option<PotionFinishRecord> {
        let Frame::PotionFinish { record } = self.top()? else {
            return None;
        };
        let owned = self.potion_finish(record)?.to_owned();
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    fn encode_draw(
        record: &DrawRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        let in_flight = matches!(record.stage, DrawStage::EarlyHook | DrawStage::OrdinaryHook);
        if record.requested == 0
            || record.completed >= record.requested
            || record.drawn.len()
                != usize::try_from(record.completed)
                    .ok()?
                    .checked_add(usize::from(in_flight))?
            || in_flight != record.card_uid.is_some()
            || record.card_uid.is_some_and(|uid| {
                record
                    .drawn
                    .last()
                    .is_none_or(|entry| entry.card().uid != uid)
            })
            || record.drawn.iter().any(|entry| {
                let card = entry.card();
                card.flags & !CARD_FLAGS_KNOWN != 0
                    || matches!(entry, DrawEntry::Uid(_)) && card.uid == u32::MAX
                    || matches!(entry, DrawEntry::Payload(_))
                        && (card.uid != LEGACY_CARD_UID || card.flags & CARD_FLAG_LEGACY == 0)
            })
            || (record.stage == DrawStage::AfterShuffle) != !record.shuffle_candidates.is_empty()
            || record
                .shuffle_candidates
                .iter()
                .enumerate()
                .any(|(index, card)| {
                    card.uid == LEGACY_CARD_UID
                        || card.flags & !CARD_FLAGS_KNOWN != 0
                        || record.shuffle_candidates[..index]
                            .iter()
                            .any(|prior| prior.uid == card.uid)
                })
            || record.gamble_paired.as_ref().is_some_and(|paired| {
                // The paired program rides a CardPlay-owned Draw whose frozen
                // count is the draw it was issued with. Siblings are a
                // discard-ordered uid set drawn from one frozen hand, so the
                // ten-card hand cap bounds them.
                record.caller != DrawCaller::CardPlay
                    || paired.count != record.requested
                    || paired.sly.len() > crate::engine::draw::MAX_CARDS_IN_HAND
                    || paired
                        .sly
                        .iter()
                        .any(|sibling| sibling.uid == u32::MAX || sibling.uid == LEGACY_CARD_UID)
                    || paired.sly.iter().enumerate().any(|(index, sibling)| {
                        paired.sly[..index]
                            .iter()
                            .any(|prior| prior.uid == sibling.uid)
                    })
            })
        {
            return None;
        }
        let paired_len = match record.gamble_paired.as_ref() {
            None => 0,
            Some(paired) => 1_usize.checked_add(paired.sly.len())?,
        };
        let body_count = 2_usize
            .checked_add(record.drawn.len())?
            .checked_add(record.shuffle_candidates.len())?
            .checked_add(paired_len)?;
        let encoded_len = 1_usize.checked_add(body_count)?;
        PendingWord::header(WordRecordKind::Draw, body_count)?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        words.reserve(encoded_len);
        words.push(PendingWord::header(WordRecordKind::Draw, body_count)?);
        words.push(PendingWord {
            body: record.requested,
            meta: record.completed,
        });
        // The paired-program tag rides the control word's spare high byte:
        // bit 24 marks presence, bits 25-31 carry the sibling count. The
        // sibling words themselves trail the shuffle candidates so the
        // self-delimiting Draw prefix parses unchanged.
        let paired_tag = record
            .gamble_paired
            .as_ref()
            .map(|paired| 1_u32 | ((paired.sly.len() as u32) << 1))
            .unwrap_or(0);
        words.push(PendingWord {
            body: record.card_uid.unwrap_or(u32::MAX),
            meta: u32::from(record.from_hand_draw)
                | (u32::from(record.stage as u8) << 8)
                | (u32::from(record.caller as u8) << 16)
                | (paired_tag << 24),
        });
        words.extend(record.drawn.iter().copied().map(|entry| {
            let mut word = PendingWord::from_card(entry.card());
            if matches!(entry, DrawEntry::Payload(_)) {
                word.meta |= 1 << 31;
            }
            word
        }));
        words.extend(
            record
                .shuffle_candidates
                .iter()
                .copied()
                .map(PendingWord::from_card),
        );
        if let Some(paired) = record.gamble_paired.as_ref() {
            words.push(PendingWord {
                body: paired.count,
                meta: 0,
            });
            words.extend(paired.sly.iter().map(|sibling| PendingWord {
                body: sibling.uid,
                meta: u32::from(sibling.atom),
            }));
        }
        Some(encoded_len)
    }

    pub(crate) fn push_draw(&mut self, record: &DrawRecord) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_draw(record, None)?;
        let start = self.0.words.len();
        let final_len = start.checked_add(encoded_len)?;
        if final_len > u32::MAX as usize {
            return None;
        }
        let index = WordRecordIndex::from_offset(start)?;
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_draw(record, Some(&mut store.words))?;
        store.frames.push(Frame::Draw { record: index });
        Some(index)
    }

    pub(crate) fn draw(&self, index: WordRecordIndex) -> Option<DrawView<'_>> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::Draw || count < 2 {
            return None;
        }
        let end = start.checked_add(1)?.checked_add(count as usize)?;
        let counters = *self.0.words.get(start + 1)?;
        let control = *self.0.words.get(start + 2)?;
        if control.meta & 0xff > 1 {
            return None;
        }
        // The paired-program tag rides the control high byte: bit 24 marks
        // presence, bits 25-31 the sibling count. No other high bits exist.
        let paired_tag = (control.meta >> 24) as u8;
        let paired_sly_len = usize::from(paired_tag >> 1);
        if paired_tag & 1 == 0 {
            if paired_tag != 0 {
                return None;
            }
        } else if paired_sly_len > crate::engine::draw::MAX_CARDS_IN_HAND {
            return None;
        }
        let paired_len = if paired_tag & 1 == 0 {
            0
        } else {
            1_usize.checked_add(paired_sly_len)?
        };
        let shuffle_end = end.checked_sub(paired_len)?;
        let stage = DrawStage::from_ordinal(((control.meta >> 8) & 0xff) as u8)?;
        let drawn_len = usize::try_from(counters.meta)
            .ok()?
            .checked_add(usize::from(matches!(
                stage,
                DrawStage::EarlyHook | DrawStage::OrdinaryHook
            )))?;
        let drawn_end = (start + 3).checked_add(drawn_len)?;
        if drawn_end > shuffle_end {
            return None;
        }
        let gamble_paired = if paired_len == 0 {
            None
        } else {
            let sly_start = shuffle_end.checked_add(1)?;
            let sly_end = sly_start.checked_add(paired_sly_len)?;
            if sly_end != end {
                return None;
            }
            let count_word = self.0.words.get(shuffle_end)?;
            if count_word.meta != 0 {
                return None;
            }
            Some(GamblePairedView {
                count: count_word.body,
                sly: self.0.words.get(sly_start..sly_end)?,
            })
        };
        let view = DrawView {
            requested: counters.body,
            completed: counters.meta,
            drawn: self.0.words.get(start + 3..drawn_end)?,
            from_hand_draw: control.meta & 0xff != 0,
            stage,
            card_uid: (control.body != u32::MAX).then_some(control.body),
            caller: DrawCaller::from_ordinal(((control.meta >> 16) & 0xff) as u8)?,
            shuffle_candidates: self.0.words.get(drawn_end..shuffle_end)?,
            gamble_paired,
        };
        Self::encode_draw(&view.to_owned(), None).filter(|len| *len == 1 + count as usize)?;
        Some(view)
    }

    pub(crate) fn replace_top_draw(&mut self, record: &DrawRecord) -> Option<()> {
        let Frame::Draw { record: index } = self.top()? else {
            return None;
        };
        self.draw(index)?;
        let encoded_len = Self::encode_draw(record, None)?;
        let start = index.offset()?;
        let final_len = start.checked_add(encoded_len)?;
        if final_len > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.words.reserve(encoded_len);
        Self::encode_draw(record, Some(&mut store.words))?;
        Some(())
    }

    pub(crate) fn pop_top_draw(&mut self) -> Option<DrawRecord> {
        let Frame::Draw { record } = self.top()? else {
            return None;
        };
        let owned = self.draw(record)?.to_owned();
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    fn encode_after_card_exhausted_power(
        record: &AfterCardExhaustedPowerRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        let listener_is_supported =
            |power: PowerId| matches!(power, PowerId::DarkEmbrace | PowerId::FeelNoPain);
        let lanes_except_source_are_default = record.amount == 0
            && record.count == 0
            && record.target.is_none()
            && record.flags == 0
            && record.remaining.is_empty()
            && record.generated.is_empty()
            && record.generation_cursor == 0;
        let lanes_are_default = record.source_uid.is_none()
            && record.step_index == 0
            && lanes_except_source_are_default;
        let return_is_valid = match record.return_kind {
            AfterCardExhaustedReturnKind::ReturnOnly => {
                lanes_are_default
                    || record.source_uid.is_some()
                        && record.step_index > 0
                        && lanes_except_source_are_default
            }
            AfterCardExhaustedReturnKind::CardFinish => lanes_are_default,
            AfterCardExhaustedReturnKind::PuritySelection
            | AfterCardExhaustedReturnKind::GenericSelection => {
                record.source_uid.is_some()
                    && record.step_index > 0
                    && record.amount == 0
                    && record.count == 0
                    && record.target.is_none()
                    && record.flags < PileId::ALL.len() as u32
                    && record.generated.is_empty()
                    && record.generation_cursor == 0
            }
            AfterCardExhaustedReturnKind::BurningPact => {
                record.source_uid.is_some()
                    && record.step_index > 0
                    && record.amount == 0
                    && record.count > 0
                    && record.target.is_none()
                    && record.flags <= 1
                    && record.remaining.is_empty()
                    && record.generated.is_empty()
                    && record.generation_cursor == 0
            }
            AfterCardExhaustedReturnKind::SecondWind => {
                record.source_uid.is_some()
                    && record.step_index > 0
                    && record.amount > 0
                    && record.count == 0
                    && record.target.is_none()
                    && record.flags <= 1
                    && record.generated.is_empty()
                    && record.generation_cursor == 0
            }
            AfterCardExhaustedReturnKind::FiendFire => {
                record.source_uid.is_some()
                    && record.step_index > 0
                    && record.amount > 0
                    && record.count > 0
                    && record.target.is_some()
                    && record.flags <= 1
                    && record.generated.is_empty()
                    && record.generation_cursor == 0
            }
            AfterCardExhaustedReturnKind::Stoke => {
                let upgrade = record.flags & 0xff;
                let phase = record.flags >> 8;
                record.source_uid.is_some()
                    && record.step_index > 0
                    && record.amount == 0
                    && record.count > 0
                    && record.target.is_none()
                    && upgrade <= 1
                    && phase <= 1
                    && record.flags & 0xffff_0000 == 0
                    && (phase == 0 && record.generated.is_empty() && record.generation_cursor == 0
                        || phase == 1
                            && record.generated.len() == usize::try_from(record.count).ok()?
                            && usize::try_from(record.generation_cursor).ok()?
                                <= record.generated.len())
            }
            AfterCardExhaustedReturnKind::Flak => {
                record.source_uid.is_some()
                    && record.step_index > 0
                    && record.amount > 0
                    && record.count > 0
                    && record.target.is_none()
                    && record.flags <= 1
                    && record.generated.is_empty()
                    && record.generation_cursor == 0
            }
            // Glowwater Potion's producer is the potion body, not a card, so
            // it carries no source/step cursor — only the frozen tail of its
            // `Hand.Cards.ToList()` snapshot in `remaining`. The `Draw(10)`
            // that follows the walk is an ordinary potion-epilogue Draw owned
            // by the PotionFinish frame underneath, not by this record.
            AfterCardExhaustedReturnKind::Glowwater => {
                record.source_uid.is_none()
                    && record.step_index == 0
                    && record.amount == 0
                    && record.count == 0
                    && record.target.is_none()
                    && record.flags == 0
                    && record.generated.is_empty()
                    && record.generation_cursor == 0
            }
        };
        if !return_is_valid
            || record.card_uid == u32::MAX
            || record
                .midnight_uids
                .iter()
                .copied()
                .enumerate()
                .any(|(index, uid)| uid == u32::MAX || record.midnight_uids[..index].contains(&uid))
            || record.source_uid == Some(u32::MAX)
            || record.listeners.is_empty()
            || usize::try_from(record.cursor).ok()? > record.listeners.len()
            || record
                .listeners
                .iter()
                .copied()
                .enumerate()
                .any(|(index, power)| {
                    !listener_is_supported(power) || record.listeners[..index].contains(&power)
                })
            || record
                .remaining
                .iter()
                .copied()
                .enumerate()
                .any(|(index, uid)| uid == u32::MAX || record.remaining[..index].contains(&uid))
            || record.target.is_some_and(|(actor, uid, slot)| {
                actor < 0 || uid == u32::MAX || slot.is_some_and(|slot| slot < 0)
            })
        {
            return None;
        }
        let listener_len = record.listeners.len();
        let midnight_len = record.midnight_uids.len();
        let remaining_len = record.remaining.len();
        let generated_len = record.generated.len();
        let body_count = 9_usize
            .checked_add(listener_len)?
            .checked_add(midnight_len)?
            .checked_add(remaining_len)?
            .checked_add(generated_len)?;
        let encoded_len = 1_usize.checked_add(body_count)?;
        PendingWord::header(WordRecordKind::AfterCardExhaustedPower, body_count)?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        words.reserve(encoded_len);
        words.push(PendingWord::header(
            WordRecordKind::AfterCardExhaustedPower,
            body_count,
        )?);
        words.push(PendingWord {
            body: record.card_uid,
            meta: record.cursor,
        });
        words.push(PendingWord {
            body: u32::try_from(midnight_len).ok()?,
            meta: u32::from(record.ordinary_suffix_invoked) | ((record.return_kind as u32) << 8),
        });
        words.push(PendingWord {
            body: record.source_uid.unwrap_or(u32::MAX),
            meta: record.step_index,
        });
        words.push(PendingWord {
            body: record.amount as u32,
            meta: record.count,
        });
        let (target_actor, target_uid, target_slot) = record
            .target
            .map_or((u32::MAX, u32::MAX, u32::MAX), |(actor, uid, slot)| {
                (actor as u32, uid, slot.map_or(u32::MAX, |slot| slot as u32))
            });
        words.push(PendingWord {
            body: target_actor,
            meta: target_uid,
        });
        words.push(PendingWord {
            body: target_slot,
            meta: record.flags,
        });
        words.push(PendingWord {
            body: u32::try_from(remaining_len).ok()?,
            meta: u32::try_from(generated_len).ok()?,
        });
        words.push(PendingWord {
            body: record.generation_cursor,
            meta: u32::try_from(listener_len).ok()?,
        });
        words.push(PendingWord { body: 0, meta: 0 });
        words.extend(record.listeners.iter().map(|power| PendingWord {
            body: *power as u32,
            meta: 0,
        }));
        words.extend(record.midnight_uids.iter().map(|uid| PendingWord {
            body: *uid,
            meta: 0,
        }));
        words.extend(record.remaining.iter().map(|uid| PendingWord {
            body: *uid,
            meta: 0,
        }));
        words.extend(record.generated.iter().map(|card| PendingWord {
            body: *card as u32,
            meta: 0,
        }));
        Some(encoded_len)
    }

    pub(crate) fn push_after_card_exhausted_power(
        &mut self,
        record: &AfterCardExhaustedPowerRecord,
    ) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_after_card_exhausted_power(record, None)?;
        let start = self.0.words.len();
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let index = WordRecordIndex::from_offset(start)?;
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_after_card_exhausted_power(record, Some(&mut store.words))?;
        store
            .frames
            .push(Frame::AfterCardExhaustedPower { record: index });
        Some(index)
    }

    pub(crate) fn after_card_exhausted_power(
        &self,
        index: WordRecordIndex,
    ) -> Option<AfterCardExhaustedPowerView<'_>> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::AfterCardExhaustedPower || count < 10 {
            return None;
        }
        let end = start.checked_add(1)?.checked_add(count as usize)?;
        let fixed = self.0.words.get(start + 1..start + 10)?;
        if fixed[1].meta & !0x0000_ff01 != 0 {
            return None;
        }
        let listener_len = usize::try_from(fixed[7].meta).ok()?;
        let midnight_len = usize::try_from(fixed[1].body).ok()?;
        let remaining_len = usize::try_from(fixed[6].body).ok()?;
        let generated_len = usize::try_from(fixed[6].meta).ok()?;
        if fixed[8].body != 0 || fixed[8].meta != 0 {
            return None;
        }
        let listeners_end = (start + 10).checked_add(listener_len)?;
        let midnight_end = listeners_end.checked_add(midnight_len)?;
        let remaining_end = midnight_end.checked_add(remaining_len)?;
        let generated_end = remaining_end.checked_add(generated_len)?;
        if generated_end != end {
            return None;
        }
        let listeners = self.0.words.get(start + 10..listeners_end)?;
        let midnight_uids = self.0.words.get(listeners_end..midnight_end)?;
        let remaining = self.0.words.get(midnight_end..remaining_end)?;
        let generated = self.0.words.get(remaining_end..generated_end)?;
        if listeners.iter().any(|word| {
            word.meta != 0
                || PowerId::ALL.get(word.body as usize).is_none_or(|power| {
                    !matches!(power, PowerId::DarkEmbrace | PowerId::FeelNoPain)
                })
        }) || midnight_uids.iter().any(|word| word.meta != 0)
            || remaining.iter().any(|word| word.meta != 0)
            || generated
                .iter()
                .any(|word| word.meta != 0 || CardId::ALL.get(word.body as usize).is_none())
        {
            return None;
        }
        let target = match (fixed[4].body, fixed[4].meta, fixed[5].body) {
            (u32::MAX, u32::MAX, u32::MAX) => None,
            (actor, uid, slot) if actor != u32::MAX && uid != u32::MAX => {
                Some((actor as i32, uid, (slot != u32::MAX).then_some(slot as i32)))
            }
            _ => return None,
        };
        let view = AfterCardExhaustedPowerView {
            listeners,
            midnight_uids,
            remaining,
            generated,
            cursor: fixed[0].meta,
            card_uid: fixed[0].body,
            ordinary_suffix_invoked: fixed[1].meta & 1 != 0,
            return_kind: AfterCardExhaustedReturnKind::from_ordinal(
                ((fixed[1].meta >> 8) & 0xff) as u8,
            )?,
            source_uid: (fixed[2].body != u32::MAX).then_some(fixed[2].body),
            step_index: fixed[2].meta,
            amount: fixed[3].body as i32,
            count: fixed[3].meta,
            target,
            flags: fixed[5].meta,
            generation_cursor: fixed[7].body,
        };
        Self::encode_after_card_exhausted_power(&view.to_owned(), None)
            .filter(|len| *len == 1 + count as usize)?;
        Some(view)
    }

    pub(crate) fn replace_top_after_card_exhausted_power(
        &mut self,
        record: &AfterCardExhaustedPowerRecord,
    ) -> Option<()> {
        let Frame::AfterCardExhaustedPower { record: index } = self.top()? else {
            return None;
        };
        self.after_card_exhausted_power(index)?;
        let encoded_len = Self::encode_after_card_exhausted_power(record, None)?;
        let start = index.offset()?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.words.reserve(encoded_len);
        Self::encode_after_card_exhausted_power(record, Some(&mut store.words))?;
        Some(())
    }

    pub(crate) fn pop_top_after_card_exhausted_power(
        &mut self,
    ) -> Option<AfterCardExhaustedPowerRecord> {
        let Frame::AfterCardExhaustedPower { record } = self.top()? else {
            return None;
        };
        let owned = self.after_card_exhausted_power(record)?.to_owned();
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    fn encode_after_card_drawn_power(
        record: &AfterCardDrawnPowerRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        let listener_is_supported = |power: PowerId| {
            matches!(
                power,
                PowerId::Speedster
                    | PowerId::CorrosiveWave
                    | PowerId::Automation
                    | PowerId::Cacophony
                    | PowerId::Pagestorm
                    | PowerId::Iteration
                    | PowerId::ChainsOfBinding
            )
        };
        if record.listeners.is_empty()
            || usize::try_from(record.cursor).ok()? > record.listeners.len()
            || record.card_uid == u32::MAX
            || record.cacophony_reset_generation < 0
            || record.listeners.iter().copied().any(|power| {
                !listener_is_supported(power)
                    || record
                        .listeners
                        .iter()
                        .filter(|other| **other == power)
                        .count()
                        != 1
            })
            || record.cacophony_reset
                && (record.cursor == 0
                    || record.listeners.get(record.cursor as usize - 1)
                        != Some(&PowerId::Cacophony))
        {
            return None;
        }
        let body_count = 2_usize.checked_add(record.listeners.len())?;
        let encoded_len = 1_usize.checked_add(body_count)?;
        PendingWord::header(WordRecordKind::AfterCardDrawnPower, body_count)?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        words.reserve(encoded_len);
        words.push(PendingWord::header(
            WordRecordKind::AfterCardDrawnPower,
            body_count,
        )?);
        words.push(PendingWord {
            body: record.card_uid,
            meta: record.cursor,
        });
        words.push(PendingWord {
            body: record.cacophony_reset_generation as u32,
            meta: u32::from(record.from_hand_draw) | (u32::from(record.cacophony_reset) << 1),
        });
        words.extend(record.listeners.iter().map(|power| PendingWord {
            body: *power as u32,
            meta: 0,
        }));
        Some(encoded_len)
    }

    pub(crate) fn push_after_card_drawn_power(
        &mut self,
        record: &AfterCardDrawnPowerRecord,
    ) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_after_card_drawn_power(record, None)?;
        let start = self.0.words.len();
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let index = WordRecordIndex::from_offset(start)?;
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_after_card_drawn_power(record, Some(&mut store.words))?;
        store
            .frames
            .push(Frame::AfterCardDrawnPower { record: index });
        Some(index)
    }

    pub(crate) fn after_card_drawn_power(
        &self,
        index: WordRecordIndex,
    ) -> Option<AfterCardDrawnPowerView<'_>> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::AfterCardDrawnPower || count < 3 {
            return None;
        }
        let end = start.checked_add(1)?.checked_add(count as usize)?;
        let cursor = *self.0.words.get(start + 1)?;
        let control = *self.0.words.get(start + 2)?;
        if control.meta & !3 != 0 {
            return None;
        }
        let listeners = self.0.words.get(start + 3..end)?;
        if listeners.iter().any(|word| {
            word.meta != 0
                || PowerId::ALL.get(word.body as usize).is_none_or(|power| {
                    !matches!(
                        power,
                        PowerId::Speedster
                            | PowerId::CorrosiveWave
                            | PowerId::Automation
                            | PowerId::Cacophony
                            | PowerId::Pagestorm
                            | PowerId::Iteration
                            | PowerId::ChainsOfBinding
                    )
                })
        }) {
            return None;
        }
        let view = AfterCardDrawnPowerView {
            listeners,
            cursor: cursor.meta,
            card_uid: cursor.body,
            from_hand_draw: control.meta & 1 != 0,
            cacophony_reset: control.meta & 2 != 0,
            cacophony_reset_generation: control.body as i32,
        };
        Self::encode_after_card_drawn_power(&view.to_owned(), None)
            .filter(|len| *len == 1 + count as usize)?;
        Some(view)
    }

    pub(crate) fn replace_top_after_card_drawn_power(
        &mut self,
        record: &AfterCardDrawnPowerRecord,
    ) -> Option<()> {
        let Frame::AfterCardDrawnPower { record: index } = self.top()? else {
            return None;
        };
        self.after_card_drawn_power(index)?;
        let encoded_len = Self::encode_after_card_drawn_power(record, None)?;
        let start = index.offset()?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.words.reserve(encoded_len);
        Self::encode_after_card_drawn_power(record, Some(&mut store.words))?;
        Some(())
    }

    pub(crate) fn pop_top_after_card_drawn_power(&mut self) -> Option<AfterCardDrawnPowerRecord> {
        let Frame::AfterCardDrawnPower { record } = self.top()? else {
            return None;
        };
        let owned = self.after_card_drawn_power(record)?.to_owned();
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    fn encode_after_side_turn_end_power(
        record: &AfterSideTurnEndPowerRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        if record.listeners.is_empty()
            || usize::try_from(record.cursor).ok()? > record.listeners.len()
            || record
                .listeners
                .iter()
                .enumerate()
                .any(|(index, listener)| {
                    listener.object.uid == u32::MAX
                        || index > 0
                            && record.listeners[index - 1].object.uid >= listener.object.uid
                        || !listener.object.token.is_instanced()
                            && record.listeners[..index]
                                .iter()
                                .any(|other| other.object.token == listener.object.token)
                })
        {
            return None;
        }
        if let Some(uid) = record.dark_pending_uid {
            let position = record.listeners.iter().position(|listener| {
                listener.object.uid == uid
                    && listener.object.token == AfterSideTurnEndPowerToken::DarkEmbrace
            })?;
            if position >= record.cursor as usize {
                return None;
            }
        }
        if record.ordinary_suffix_invoked
            && (record.cursor as usize != record.listeners.len()
                || record.dark_pending_uid.is_none())
        {
            return None;
        }
        let body_count = 2_usize.checked_add(record.listeners.len().checked_mul(3)?)?;
        let encoded_len = body_count.checked_add(1)?;
        PendingWord::header(WordRecordKind::AfterSideTurnEndPower, body_count)?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        words.reserve(encoded_len);
        words.push(PendingWord::header(
            WordRecordKind::AfterSideTurnEndPower,
            body_count,
        )?);
        words.push(PendingWord {
            body: record.cursor,
            meta: u32::from(record.ordinary_suffix_invoked)
                | (u32::from(record.dark_pending_uid.is_some()) << 1)
                | (u32::from(record.conqueror_reachable_at_entry) << 2),
        });
        words.push(PendingWord {
            body: record.dark_pending_uid.unwrap_or(u32::MAX),
            meta: 0,
        });
        for listener in &record.listeners {
            words.push(PendingWord {
                body: listener.object.uid,
                meta: listener.object.token as u32,
            });
            words.push(PendingWord {
                body: listener.payload[0] as u32,
                meta: listener.payload[1] as u32,
            });
            words.push(PendingWord {
                body: listener.payload[2] as u32,
                meta: listener.payload[3] as u32,
            });
        }
        Some(encoded_len)
    }

    pub(crate) fn push_after_side_turn_end_power(
        &mut self,
        record: &AfterSideTurnEndPowerRecord,
    ) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_after_side_turn_end_power(record, None)?;
        let start = self.0.words.len();
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let index = WordRecordIndex::from_offset(start)?;
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_after_side_turn_end_power(record, Some(&mut store.words))?;
        store
            .frames
            .push(Frame::AfterSideTurnEndPower { record: index });
        Some(index)
    }

    pub(crate) fn after_side_turn_end_power(
        &self,
        index: WordRecordIndex,
    ) -> Option<AfterSideTurnEndPowerView<'_>> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::AfterSideTurnEndPower || count < 5 || (count - 2) % 3 != 0 {
            return None;
        }
        let end = start.checked_add(1)?.checked_add(count as usize)?;
        let control = *self.0.words.get(start + 1)?;
        let dark = *self.0.words.get(start + 2)?;
        if control.meta & !7 != 0 || dark.meta != 0 {
            return None;
        }
        let listeners = self.0.words.get(start + 3..end)?;
        let dark_pending_uid = if control.meta & 2 != 0 {
            Some(dark.body)
        } else if dark.body == u32::MAX {
            None
        } else {
            return None;
        };
        let view = AfterSideTurnEndPowerView {
            listeners,
            cursor: control.body,
            dark_pending_uid,
            ordinary_suffix_invoked: control.meta & 1 != 0,
            conqueror_reachable_at_entry: control.meta & 4 != 0,
        };
        let listener_count = listeners.len() / 3;
        if usize::try_from(view.cursor).ok()? > listener_count {
            return None;
        }
        let mut prior_uid = None;
        for (index, words) in listeners.chunks_exact(3).enumerate() {
            let token = AfterSideTurnEndPowerToken::from_ordinal(words[0].meta)?;
            let uid = words[0].body;
            if uid == u32::MAX || prior_uid.is_some_and(|prior| prior >= uid) {
                return None;
            }
            if !token.is_instanced()
                && listeners[..index * 3]
                    .chunks_exact(3)
                    .any(|prior| prior[0].meta == words[0].meta)
            {
                return None;
            }
            prior_uid = Some(uid);
        }
        if let Some(uid) = view.dark_pending_uid {
            let position = listeners.chunks_exact(3).position(|words| {
                words[0].body == uid
                    && words[0].meta == AfterSideTurnEndPowerToken::DarkEmbrace as u32
            })?;
            if position >= view.cursor as usize {
                return None;
            }
        }
        if view.ordinary_suffix_invoked
            && (view.cursor as usize != listener_count || view.dark_pending_uid.is_none())
        {
            return None;
        }
        Some(view)
    }

    pub(crate) fn set_after_side_turn_end_power_cursor(
        &mut self,
        index: WordRecordIndex,
        cursor: u32,
    ) -> Option<()> {
        let view = self.after_side_turn_end_power(index)?;
        if usize::try_from(cursor).ok()? > view.listeners().len() {
            return None;
        }
        let control_index = index.offset()?.checked_add(1)?;
        self.0.words.get(control_index)?;
        Arc::make_mut(&mut self.0).words[control_index].body = cursor;
        Some(())
    }

    pub(crate) fn set_after_side_turn_end_power_payload(
        &mut self,
        index: WordRecordIndex,
        listener_index: usize,
        payload: [i32; 4],
    ) -> Option<()> {
        let view = self.after_side_turn_end_power(index)?;
        view.listeners().nth(listener_index)?;
        let payload_index = index
            .offset()?
            .checked_add(4)?
            .checked_add(listener_index.checked_mul(3)?)?;
        let end = payload_index.checked_add(2)?;
        self.0.words.get(payload_index..end)?;
        let words = &mut Arc::make_mut(&mut self.0).words[payload_index..end];
        words[0] = PendingWord {
            body: payload[0] as u32,
            meta: payload[1] as u32,
        };
        words[1] = PendingWord {
            body: payload[2] as u32,
            meta: payload[3] as u32,
        };
        Some(())
    }

    pub(crate) fn set_after_side_turn_end_power_dark_pending_uid(
        &mut self,
        index: WordRecordIndex,
        dark_pending_uid: Option<u32>,
    ) -> Option<()> {
        let view = self.after_side_turn_end_power(index)?;
        if let Some(uid) = dark_pending_uid {
            let position = view.listeners().position(|listener| {
                listener.object.uid == uid
                    && listener.object.token == AfterSideTurnEndPowerToken::DarkEmbrace
            })?;
            if position >= view.cursor as usize {
                return None;
            }
        }
        let control_index = index.offset()?.checked_add(1)?;
        let dark_index = control_index.checked_add(1)?;
        self.0.words.get(dark_index)?;
        let store = Arc::make_mut(&mut self.0);
        if dark_pending_uid.is_some() {
            store.words[control_index].meta |= 2;
        } else {
            store.words[control_index].meta &= !2;
        }
        store.words[dark_index].body = dark_pending_uid.unwrap_or(u32::MAX);
        Some(())
    }

    pub(crate) fn set_after_side_turn_end_power_ordinary_suffix_invoked(
        &mut self,
        index: WordRecordIndex,
    ) -> Option<()> {
        let view = self.after_side_turn_end_power(index)?;
        if view.cursor as usize != view.listeners().len() || view.dark_pending_uid.is_none() {
            return None;
        }
        let control_index = index.offset()?.checked_add(1)?;
        self.0.words.get(control_index)?;
        Arc::make_mut(&mut self.0).words[control_index].meta |= 1;
        Some(())
    }

    /// Pop a completed ordinary owner without materializing its captured
    /// listener vector. The post-Hook continuation needs only the frozen
    /// Conqueror entry witness.
    pub(crate) fn pop_top_after_side_turn_end_power_completion(&mut self) -> Option<bool> {
        let Frame::AfterSideTurnEndPower { record } = self.top()? else {
            return None;
        };
        let view = self.after_side_turn_end_power(record)?;
        let conqueror_reachable_at_entry = view.conqueror_reachable_at_entry;
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(conqueror_reachable_at_entry)
    }

    #[cfg(test)]
    pub(crate) fn pop_top_after_side_turn_end_power(
        &mut self,
    ) -> Option<AfterSideTurnEndPowerRecord> {
        let Frame::AfterSideTurnEndPower { record } = self.top()? else {
            return None;
        };
        let owned = self.after_side_turn_end_power(record)?.to_owned();
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    fn encode_after_power_amount_changed(
        record: &AfterPowerAmountChangedRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        let listener_is_supported = |power: PowerId| {
            matches!(
                power,
                PowerId::Vicious | PowerId::Shroud | PowerId::SleightOfFlesh | PowerId::SwordSage
            )
        };
        let (slot, uid, fallback) = record.target;
        if record.listeners.is_empty()
            || usize::try_from(record.cursor).ok()? > record.listeners.len()
            || record.amount <= 0
            || uid == u32::MAX
            || fallback.is_some_and(|index| index < 0)
            || record.listeners.iter().copied().any(|power| {
                !listener_is_supported(power)
                    || record
                        .listeners
                        .iter()
                        .filter(|other| **other == power)
                        .count()
                        != 1
            })
        {
            return None;
        }
        let body_count = 3_usize.checked_add(record.listeners.len())?;
        let encoded_len = 1_usize.checked_add(body_count)?;
        PendingWord::header(WordRecordKind::AfterPowerAmountChanged, body_count)?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        words.reserve(encoded_len);
        words.push(PendingWord::header(
            WordRecordKind::AfterPowerAmountChanged,
            body_count,
        )?);
        words.push(PendingWord {
            body: record.cursor,
            meta: record.amount as u32,
        });
        words.push(PendingWord {
            body: slot as u32,
            meta: uid,
        });
        words.push(PendingWord {
            body: fallback.map_or(u32::MAX, |index| index as u32),
            meta: 0,
        });
        words.extend(record.listeners.iter().map(|power| PendingWord {
            body: *power as u32,
            meta: 0,
        }));
        Some(encoded_len)
    }

    pub(crate) fn push_after_power_amount_changed(
        &mut self,
        record: &AfterPowerAmountChangedRecord,
    ) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_after_power_amount_changed(record, None)?;
        let start = self.0.words.len();
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let index = WordRecordIndex::from_offset(start)?;
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_after_power_amount_changed(record, Some(&mut store.words))?;
        store
            .frames
            .push(Frame::AfterPowerAmountChanged { record: index });
        Some(index)
    }

    pub(crate) fn after_power_amount_changed(
        &self,
        index: WordRecordIndex,
    ) -> Option<AfterPowerAmountChangedView<'_>> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::AfterPowerAmountChanged || count < 4 {
            return None;
        }
        let end = start.checked_add(1)?.checked_add(count as usize)?;
        let control = *self.0.words.get(start + 1)?;
        let target = *self.0.words.get(start + 2)?;
        let fallback = *self.0.words.get(start + 3)?;
        if fallback.meta != 0 {
            return None;
        }
        let listeners = self.0.words.get(start + 4..end)?;
        if listeners.iter().any(|word| {
            word.meta != 0
                || PowerId::ALL.get(word.body as usize).is_none_or(|power| {
                    !matches!(
                        power,
                        PowerId::Vicious
                            | PowerId::Shroud
                            | PowerId::SleightOfFlesh
                            | PowerId::SwordSage
                    )
                })
        }) {
            return None;
        }
        let view = AfterPowerAmountChangedView {
            listeners,
            cursor: control.body,
            amount: control.meta as i32,
            target: (
                target.body as i32,
                target.meta,
                (fallback.body != u32::MAX).then_some(fallback.body as i32),
            ),
        };
        Self::encode_after_power_amount_changed(&view.to_owned(), None)
            .filter(|len| *len == 1 + count as usize)?;
        Some(view)
    }

    pub(crate) fn replace_top_after_power_amount_changed(
        &mut self,
        record: &AfterPowerAmountChangedRecord,
    ) -> Option<()> {
        let Frame::AfterPowerAmountChanged { record: index } = self.top()? else {
            return None;
        };
        self.after_power_amount_changed(index)?;
        let encoded_len = Self::encode_after_power_amount_changed(record, None)?;
        let start = index.offset()?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.words.reserve(encoded_len);
        Self::encode_after_power_amount_changed(record, Some(&mut store.words))?;
        Some(())
    }

    pub(crate) fn pop_top_after_power_amount_changed(
        &mut self,
    ) -> Option<AfterPowerAmountChangedRecord> {
        let Frame::AfterPowerAmountChanged { record } = self.top()? else {
            return None;
        };
        let owned = self.after_power_amount_changed(record)?.to_owned();
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    #[cold]
    #[inline(never)]
    fn encode_action_replay(
        record: &ActionReplayRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        if record.predecessor_json.is_empty() {
            return None;
        }
        let json_words = record.predecessor_json.len().checked_add(7)? / 8;
        let body_count = 3_usize
            .checked_add(json_words)?
            .checked_add(record.answers.len())?;
        let encoded_len = 1_usize.checked_add(body_count)?;
        PendingWord::header(WordRecordKind::ActionReplay, body_count)?;
        let (action_word, selection_word) = match record.action {
            ActionReplayRootAction::Play {
                uid,
                target,
                selection_uid,
            } => {
                if selection_uid == Some(u32::MAX) {
                    return None;
                }
                (
                    PendingWord {
                        body: uid,
                        meta: u32::from(target.map_or(0, |value| u16::from(value) + 1)) << 8,
                    },
                    PendingWord {
                        body: selection_uid.unwrap_or(u32::MAX),
                        meta: 0,
                    },
                )
            }
            ActionReplayRootAction::EndTurn => (
                PendingWord { body: 0, meta: 1 },
                PendingWord {
                    body: u32::MAX,
                    meta: 0,
                },
            ),
            ActionReplayRootAction::UsePotion { slot, target } => (
                PendingWord {
                    body: u32::from(slot),
                    meta: 2 | (u32::from(target.map_or(0, |value| u16::from(value) + 1)) << 8),
                },
                PendingWord {
                    body: u32::MAX,
                    meta: 0,
                },
            ),
        };
        let answer_count: u32 = record.answers.len().try_into().ok()?;
        let predecessor_len: u32 = record.predecessor_json.len().try_into().ok()?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        words.reserve(encoded_len);
        words.push(PendingWord::header(
            WordRecordKind::ActionReplay,
            body_count,
        )?);
        words.push(action_word);
        words.push(selection_word);
        words.push(PendingWord {
            body: predecessor_len,
            meta: answer_count,
        });
        for chunk in record.predecessor_json.chunks(8) {
            let mut bytes = [0_u8; 8];
            bytes[..chunk.len()].copy_from_slice(chunk);
            words.push(PendingWord {
                body: u32::from_le_bytes(bytes[..4].try_into().ok()?),
                meta: u32::from_le_bytes(bytes[4..].try_into().ok()?),
            });
        }
        words.extend(record.answers.iter().copied().map(|answer| match answer {
            ActionReplayAnswer::CardUid(value) => PendingWord {
                body: value,
                meta: 0,
            },
            ActionReplayAnswer::OptionIndex(value) => PendingWord {
                body: value,
                meta: 1,
            },
        }));
        Some(encoded_len)
    }

    /// Install the sole bottom-of-stack independently rooted action replay.
    #[cold]
    #[inline(never)]
    pub(crate) fn push_action_replay(
        &mut self,
        record: &ActionReplayRecord,
    ) -> Option<WordRecordIndex> {
        if !self.is_empty() || !self.0.words.is_empty() {
            return None;
        }
        let encoded_len = Self::encode_action_replay(record, None)?;
        if encoded_len > u32::MAX as usize {
            return None;
        }
        let index = WordRecordIndex::from_offset(0)?;
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_action_replay(record, Some(&mut store.words))?;
        store.frames.push(Frame::ActionReplay { record: index });
        Some(index)
    }

    /// Validate the closed replay record at `index`, including canonical zero
    /// padding, without copying its predecessor document or answers.
    ///
    /// Every check [`Self::action_replay`] makes is made here, and that
    /// decoder is built on this one, so `action_replay_layout(i).is_some()`
    /// iff `action_replay(i).is_some()`. Continuation validators ask this
    /// question several times per public transition; materializing the
    /// predecessor JSON for each ask dominated selection-heavy fights (#3420).
    fn action_replay_layout(&self, index: WordRecordIndex) -> Option<ActionReplayLayout> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::ActionReplay || count < 4 {
            return None;
        }
        let end = start.checked_add(1)?.checked_add(count as usize)?;
        let action_word = *self.0.words.get(start + 1)?;
        let selection_word = *self.0.words.get(start + 2)?;
        let lengths = *self.0.words.get(start + 3)?;
        if selection_word.meta != 0 {
            return None;
        }
        let action = match action_word.meta & 0xff {
            0 if action_word.meta & !0x0001_ff00 == 0 => {
                let encoded_target = (action_word.meta >> 8) & 0x1ff;
                if encoded_target > 256 {
                    return None;
                }
                ActionReplayRootAction::Play {
                    uid: action_word.body,
                    target: encoded_target.checked_sub(1).map(|value| value as u8),
                    selection_uid: (selection_word.body != u32::MAX).then_some(selection_word.body),
                }
            }
            1 if action_word == (PendingWord { body: 0, meta: 1 })
                && selection_word.body == u32::MAX =>
            {
                ActionReplayRootAction::EndTurn
            }
            2 if action_word.body <= u32::from(u8::MAX)
                && action_word.meta & !0x0001_ff02 == 0
                && selection_word.body == u32::MAX =>
            {
                let encoded_target = (action_word.meta >> 8) & 0x1ff;
                if encoded_target > 256 {
                    return None;
                }
                ActionReplayRootAction::UsePotion {
                    slot: action_word.body as u8,
                    target: encoded_target.checked_sub(1).map(|value| value as u8),
                }
            }
            _ => return None,
        };
        let predecessor_len = usize::try_from(lengths.body).ok()?;
        if predecessor_len == 0 {
            return None;
        }
        let answer_count = usize::try_from(lengths.meta).ok()?;
        let json_words = predecessor_len.checked_add(7)? / 8;
        if 3_usize.checked_add(json_words)?.checked_add(answer_count)? != count as usize {
            return None;
        }
        let json = start + 4..start + 4 + json_words;
        let json_slice = self.0.words.get(json.clone())?;
        // Canonical zero padding: the bytes past `predecessor_len` all sit in
        // the last JSON word, laid out `body` then `meta`, little-endian.
        let last = json_slice.last()?;
        let mut last_bytes = [0_u8; 8];
        last_bytes[..4].copy_from_slice(&last.body.to_le_bytes());
        last_bytes[4..].copy_from_slice(&last.meta.to_le_bytes());
        let used_in_last = predecessor_len - (json_words - 1) * 8;
        if last_bytes[used_in_last..].iter().any(|byte| *byte != 0) {
            return None;
        }
        let answers = start + 4 + json_words..end;
        if self
            .0
            .words
            .get(answers.clone())?
            .iter()
            .any(|word| word.meta > 1)
        {
            return None;
        }
        Some(ActionReplayLayout {
            action,
            json,
            predecessor_len,
            answers,
        })
    }

    /// The root action of the valid closed replay record at `index`; `None`
    /// exactly when [`Self::action_replay`] is. For callers that need only
    /// the action or the record's validity, this copies nothing.
    pub(crate) fn action_replay_action(
        &self,
        index: WordRecordIndex,
    ) -> Option<ActionReplayRootAction> {
        self.action_replay_layout(index).map(|layout| layout.action)
    }

    /// Decode the closed replay record, including canonical zero padding.
    #[cold]
    #[inline(never)]
    pub(crate) fn action_replay(&self, index: WordRecordIndex) -> Option<ActionReplayRecord> {
        let layout = self.action_replay_layout(index)?;
        let json_slice = self.0.words.get(layout.json)?;
        let mut predecessor_json = Vec::with_capacity(json_slice.len() * 8);
        for word in json_slice {
            predecessor_json.extend_from_slice(&word.body.to_le_bytes());
            predecessor_json.extend_from_slice(&word.meta.to_le_bytes());
        }
        predecessor_json.truncate(layout.predecessor_len);
        let answers = self
            .0
            .words
            .get(layout.answers)?
            .iter()
            .map(|word| match word.meta {
                0 => Some(ActionReplayAnswer::CardUid(word.body)),
                1 => Some(ActionReplayAnswer::OptionIndex(word.body)),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        Some(ActionReplayRecord {
            predecessor_json,
            action: layout.action,
            answers,
        })
    }

    fn rebase_frame_record_after(frame: Frame, insertion: usize, delta: usize) -> Option<Frame> {
        let rebase = |record: WordRecordIndex| {
            let offset = record.offset()?;
            WordRecordIndex::from_offset(if offset >= insertion {
                offset.checked_add(delta)?
            } else {
                offset
            })
        };
        Some(match frame {
            Frame::ActionReplay { record } => Frame::ActionReplay {
                record: rebase(record)?,
            },
            Frame::CardPlay { record } => Frame::CardPlay {
                record: rebase(record)?,
            },
            Frame::Phase { record } => Frame::Phase {
                record: rebase(record)?,
            },
            Frame::LiveListener { record } => Frame::LiveListener {
                record: rebase(record)?,
            },
            Frame::FrozenAutoBatch { record } => Frame::FrozenAutoBatch {
                record: rebase(record)?,
            },
            Frame::TurnStartHandChoice { record } => Frame::TurnStartHandChoice {
                record: rebase(record)?,
            },
            Frame::EnemyPhase { record } => Frame::EnemyPhase {
                record: rebase(record)?,
            },
            Frame::PotionFinish { record } => Frame::PotionFinish {
                record: rebase(record)?,
            },
            Frame::Draw { record } => Frame::Draw {
                record: rebase(record)?,
            },
            Frame::AfterCardDrawnPower { record } => Frame::AfterCardDrawnPower {
                record: rebase(record)?,
            },
            Frame::AfterCardExhaustedPower { record } => Frame::AfterCardExhaustedPower {
                record: rebase(record)?,
            },
            Frame::AfterPowerAmountChanged { record } => Frame::AfterPowerAmountChanged {
                record: rebase(record)?,
            },
            Frame::AfterSideTurnEndPower { record } => Frame::AfterSideTurnEndPower {
                record: rebase(record)?,
            },
            Frame::DarkEmbraceSideEnd { record } => Frame::DarkEmbraceSideEnd {
                record: rebase(record)?,
            },
            Frame::CardFinish { record } => Frame::CardFinish {
                record: rebase(record)?,
            },
            Frame::BeforeHandDrawPower { record } => Frame::BeforeHandDrawPower {
                record: rebase(record)?,
            },
            other => other,
        })
    }

    /// The word arena's length: the offset the next pushed record will take.
    pub(crate) fn word_len(&self) -> usize {
        self.0.words.len()
    }

    /// The record of a frame that owns its own words, with that frame's
    /// kind accepted in a detached hook-action segment (#3387): the Draw
    /// child grammar only (`rooted_draw_child_suffix_is_exact`).
    fn hook_segment_record(frame: Frame) -> Option<WordRecordIndex> {
        match frame {
            Frame::Draw { record }
            | Frame::CardPlay { record }
            | Frame::AfterCardDrawnPower { record }
            | Frame::AfterCardExhaustedPower { record }
            | Frame::AfterPowerAmountChanged { record }
            | Frame::CardFinish { record } => Some(record),
            _ => None,
        }
    }

    fn with_hook_segment_record(frame: Frame, record: WordRecordIndex) -> Option<Frame> {
        Some(match frame {
            Frame::Draw { .. } => Frame::Draw { record },
            Frame::CardPlay { .. } => Frame::CardPlay { record },
            Frame::AfterCardDrawnPower { .. } => Frame::AfterCardDrawnPower { record },
            Frame::AfterCardExhaustedPower { .. } => Frame::AfterCardExhaustedPower { record },
            Frame::AfterPowerAmountChanged { .. } => Frame::AfterPowerAmountChanged { record },
            Frame::CardFinish { .. } => Frame::CardFinish { record },
            _ => return None,
        })
    }

    /// Detach every frame at or above `depth`, with the words from
    /// `word_base` on, as a relocatable segment (#3387,
    /// `engine::hook_action`). The store is stack-like
    /// (`continuation_store_is_valid` requires records to be contiguous in
    /// stack order), so the segment owns exactly those words. Every segment
    /// frame must own its record and be of the Draw child grammar; the
    /// records are stored relative to the segment.
    #[cold]
    #[inline(never)]
    pub(crate) fn detach_hook_segment(
        &mut self,
        depth: usize,
        word_base: usize,
    ) -> Option<HookFrameSegment> {
        if depth >= self.0.frames.len() || word_base > self.0.words.len() {
            return None;
        }
        let frames = self.0.frames[depth..]
            .iter()
            .copied()
            .map(|frame| {
                let offset = Self::hook_segment_record(frame)?.offset()?;
                let relative = WordRecordIndex::from_offset(offset.checked_sub(word_base)?)?;
                Self::with_hook_segment_record(frame, relative)
            })
            .collect::<Option<Vec<_>>>()?;
        let store = Arc::make_mut(&mut self.0);
        let words = store.words.split_off(word_base);
        store.frames.truncate(depth);
        Some(HookFrameSegment { frames, words })
    }

    /// Re-attach a detached segment on top of the stack, returning the word
    /// offset its records were rebased onto.
    #[cold]
    #[inline(never)]
    pub(crate) fn attach_hook_segment(&mut self, segment: &HookFrameSegment) -> Option<usize> {
        let base = self.0.words.len();
        if base.checked_add(segment.words.len())? > u32::MAX as usize {
            return None;
        }
        let frames = segment
            .frames
            .iter()
            .copied()
            .map(|frame| {
                let offset = Self::hook_segment_record(frame)?.offset()?;
                let rebased = WordRecordIndex::from_offset(offset.checked_add(base)?)?;
                Self::with_hook_segment_record(frame, rebased)
            })
            .collect::<Option<Vec<_>>>()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.extend_from_slice(&segment.words);
        store.frames.extend(frames);
        Some(base)
    }

    /// Prepend the sole replay root beneath an already-authenticated parked
    /// continuation stack. Every record-bearing frame is rebased before the
    /// new store is published; the HotState wrapper owns the separate pending
    /// index and completes the same atomic transaction.
    #[cold]
    #[inline(never)]
    fn install_action_replay_bottom(&mut self, record: &ActionReplayRecord) -> Option<usize> {
        if self
            .0
            .frames
            .iter()
            .any(|frame| matches!(frame, Frame::ActionReplay { .. }))
        {
            return None;
        }
        let encoded_len = Self::encode_action_replay(record, None)?;
        let total_words = self.0.words.len().checked_add(encoded_len)?;
        if total_words > u32::MAX as usize {
            return None;
        }
        let rebased_frames = self
            .0
            .frames
            .iter()
            .copied()
            .map(|frame| Self::rebase_frame_record_after(frame, 0, encoded_len))
            .collect::<Option<Vec<_>>>()?;
        let mut prefix = Vec::new();
        prefix.try_reserve(encoded_len).ok()?;
        Self::encode_action_replay(record, Some(&mut prefix))?;
        if prefix.len() != encoded_len {
            return None;
        }
        let mut store = (*self.0).clone();
        store.words.try_reserve(encoded_len).ok()?;
        prefix.extend_from_slice(&store.words);
        store.words = prefix;
        store.frames.try_reserve(1).ok()?;
        store.frames.insert(
            0,
            Frame::ActionReplay {
                record: WordRecordIndex::from_offset(0)?,
            },
        );
        for (destination, rebased) in store.frames[1..].iter_mut().zip(rebased_frames) {
            *destination = rebased;
        }
        self.0 = Arc::new(store);
        Some(encoded_len)
    }

    /// Append one successful typed answer to the prefix replay record.
    ///
    /// Growing the first record shifts every later word record by one. All
    /// frame offsets are preflighted and rebased before the single COW store
    /// mutation. The caller must similarly rebase `PendingSelection` using
    /// the returned insertion offset before publishing the new store.
    #[cold]
    #[inline(never)]
    fn append_action_replay_answer(
        &mut self,
        index: WordRecordIndex,
        answer: ActionReplayAnswer,
    ) -> Option<usize> {
        if self.0.frames.first().copied() != Some(Frame::ActionReplay { record: index })
            || index.offset() != Some(0)
        {
            return None;
        }
        let existing = self.action_replay(index)?;
        let (_, old_count) = self.0.words.first().copied()?.parse_header()?;
        let insertion = 1_usize.checked_add(old_count as usize)?;
        if insertion > self.0.words.len() || self.0.words.len() >= u32::MAX as usize {
            return None;
        }
        let new_count = old_count.checked_add(1)?;
        let new_answer_count: u32 = existing.answers.len().checked_add(1)?.try_into().ok()?;
        let rebased_frames = self
            .0
            .frames
            .iter()
            .copied()
            .map(|frame| Self::rebase_frame_record_after(frame, insertion, 1))
            .collect::<Option<Vec<_>>>()?;
        let answer_word = match answer {
            ActionReplayAnswer::CardUid(value) => PendingWord {
                body: value,
                meta: 0,
            },
            ActionReplayAnswer::OptionIndex(value) => PendingWord {
                body: value,
                meta: 1,
            },
        };
        let new_header = PendingWord::header(
            WordRecordKind::ActionReplay,
            usize::try_from(new_count).ok()?,
        )?;
        let mut store = (*self.0).clone();
        store.words.try_reserve(1).ok()?;
        store.words.insert(insertion, answer_word);
        store.words[0] = new_header;
        store.words[3].meta = new_answer_count;
        store.frames = rebased_frames;
        self.0 = Arc::new(store);
        Some(insertion)
    }

    /// Pop the replay root only after every child frame has drained.
    #[cold]
    #[inline(never)]
    pub(crate) fn pop_top_action_replay(&mut self) -> Option<ActionReplayRecord> {
        let Frame::ActionReplay { record } = self.top()? else {
            return None;
        };
        if self.len() != 1 || record.offset() != Some(0) {
            return None;
        }
        let owned = self.action_replay(record)?;
        let store = Arc::make_mut(&mut self.0);
        store.frames.clear();
        store.words.clear();
        Some(owned)
    }

    fn encode_enemy_phase(
        record: &EnemyPhaseRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        if record.stage != 0 || record.remaining_uids.contains(&record.actor_uid) {
            return None;
        }
        let body_count = 1_usize.checked_add(record.remaining_uids.len())?;
        let encoded_len = 1_usize.checked_add(body_count)?;
        PendingWord::header(WordRecordKind::EnemyPhase, body_count)?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        words.reserve(encoded_len);
        words.push(PendingWord::header(WordRecordKind::EnemyPhase, body_count)?);
        words.push(PendingWord {
            body: record.actor_uid,
            meta: u32::from(record.stage),
        });
        words.extend(record.remaining_uids.iter().map(|uid| PendingWord {
            body: *uid,
            meta: 0,
        }));
        Some(encoded_len)
    }

    pub(crate) fn push_enemy_phase(
        &mut self,
        record: &EnemyPhaseRecord,
    ) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_enemy_phase(record, None)?;
        let start = self.0.words.len();
        let final_len = start.checked_add(encoded_len)?;
        if final_len > u32::MAX as usize {
            return None;
        }
        let index = WordRecordIndex::from_offset(start)?;
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_enemy_phase(record, Some(&mut store.words))?;
        store.frames.push(Frame::EnemyPhase { record: index });
        Some(index)
    }

    pub(crate) fn enemy_phase(&self, index: WordRecordIndex) -> Option<EnemyPhaseRecord> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::EnemyPhase || count < 1 {
            return None;
        }
        let end = start.checked_add(1)?.checked_add(count as usize)?;
        let control = *self.0.words.get(start + 1)?;
        if control.meta != 0 {
            return None;
        }
        let remaining_uids = self
            .0
            .words
            .get(start + 2..end)?
            .iter()
            .map(|word| (word.meta == 0).then_some(word.body))
            .collect::<Option<Vec<_>>>()?;
        let record = EnemyPhaseRecord {
            actor_uid: control.body,
            stage: 0,
            remaining_uids,
        };
        Self::encode_enemy_phase(&record, None).filter(|len| *len == 1 + count as usize)?;
        Some(record)
    }

    pub(crate) fn pop_top_enemy_phase(&mut self) -> Option<EnemyPhaseRecord> {
        let Frame::EnemyPhase { record } = self.top()? else {
            return None;
        };
        let owned = self.enemy_phase(record)?;
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    pub(crate) fn replace_top_enemy_phase(&mut self, record: &EnemyPhaseRecord) -> Option<()> {
        let Frame::EnemyPhase { record: index } = self.top()? else {
            return None;
        };
        self.enemy_phase(index)?;
        let encoded_len = Self::encode_enemy_phase(record, None)?;
        let start = index.offset()?;
        let final_len = start.checked_add(encoded_len)?;
        if final_len > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.words.reserve(encoded_len);
        Self::encode_enemy_phase(record, Some(&mut store.words))?;
        Some(())
    }

    fn encode_auto_post_phase(
        record: &AutoPostPhaseRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        let listener_count = record.listeners.len();
        if listener_count == 0
            || usize::try_from(record.cursor).ok()? > listener_count
            || record
                .listeners
                .iter()
                .enumerate()
                .any(|(index, listener)| match listener {
                    AutoPostListener::Stampede => index != 0,
                    AutoPostListener::Howl { uid } | AutoPostListener::Invincible { uid } => record
                        .listeners[..index]
                        .iter()
                        .any(|prior| prior.card_uid() == Some(*uid)),
                })
        {
            return None;
        }
        let encoded_len = 2_usize.checked_add(listener_count)?;
        PendingWord::header(WordRecordKind::Phase, encoded_len - 1)?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        words.reserve(encoded_len);
        words.push(
            PendingWord::header(WordRecordKind::Phase, encoded_len - 1)
                .expect("preflighted Phase header"),
        );
        words.push(PendingWord {
            body: record.cursor,
            meta: 0,
        });
        words.extend(record.listeners.iter().map(|listener| match *listener {
            AutoPostListener::Stampede => PendingWord { body: 0, meta: 0 },
            AutoPostListener::Howl { uid } => PendingWord { body: uid, meta: 1 },
            AutoPostListener::Invincible { uid } => PendingWord { body: uid, meta: 2 },
        }));
        Some(encoded_len)
    }

    /// Append the admitted AutoPost phase atomically.
    pub(crate) fn push_auto_post_phase(
        &mut self,
        record: &AutoPostPhaseRecord,
    ) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_auto_post_phase(record, None)?;
        let start = self.0.words.len();
        let index = WordRecordIndex::from_offset(start)?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_auto_post_phase(record, Some(&mut store.words))?;
        store.frames.push(Frame::Phase { record: index });
        Some(index)
    }

    /// Decode one admitted AutoPost phase record.
    pub(crate) fn auto_post_phase(&self, index: WordRecordIndex) -> Option<AutoPostPhaseView<'_>> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::Phase || count < 2 {
            return None;
        }
        let end = start.checked_add(1)?.checked_add(count as usize)?;
        let fixed = *self.0.words.get(start + 1)?;
        if fixed.meta != 0 {
            return None;
        }
        let listener_words = self.0.words.get(start + 2..end)?;
        if listener_words.is_empty()
            || usize::try_from(fixed.body).ok()? > listener_words.len()
            || listener_words
                .iter()
                .enumerate()
                .any(|(position, word)| match word.meta {
                    0 => position != 0 || word.body != 0,
                    1 | 2 => listener_words[..position]
                        .iter()
                        .any(|prior| matches!(prior.meta, 1 | 2) && prior.body == word.body),
                    _ => true,
                })
        {
            return None;
        }
        Some(AutoPostPhaseView {
            cursor: fixed.body,
            listener_words,
        })
    }

    /// Replace only the authenticated top AutoPost phase.
    pub(crate) fn replace_top_auto_post_phase(
        &mut self,
        record: &AutoPostPhaseRecord,
    ) -> Option<()> {
        let Frame::Phase { record: index } = self.top()? else {
            return None;
        };
        self.auto_post_phase(index)?;
        let encoded_len = Self::encode_auto_post_phase(record, None)?;
        let start = index.offset()?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        Self::encode_auto_post_phase(record, Some(&mut store.words))?;
        Some(())
    }

    /// Pop only the authenticated top AutoPost phase and its word suffix.
    pub(crate) fn pop_top_auto_post_phase(&mut self) -> Option<AutoPostPhaseRecord> {
        let Frame::Phase { record } = self.top()? else {
            return None;
        };
        let owned = self.auto_post_phase(record)?.to_owned();
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    #[cold]
    #[inline(never)]
    fn encode_auto_pre_mayhem_phase(destination: Option<&mut Vec<PendingWord>>) -> Option<usize> {
        let Some(words) = destination else {
            return Some(2);
        };
        words.reserve(2);
        words.push(PendingWord::header(WordRecordKind::Phase, 1)?);
        // body=1 is the precommitted sole Mayhem listener; meta=1 keeps this
        // record disjoint from every AutoPost header/body combination.
        words.push(PendingWord { body: 1, meta: 1 });
        Some(2)
    }

    /// Append the exact parked AutoPre/normal Mayhem phase atomically.
    #[cold]
    #[inline(never)]
    pub(crate) fn push_auto_pre_mayhem_phase(&mut self) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_auto_pre_mayhem_phase(None)?;
        let start = self.0.words.len();
        let index = WordRecordIndex::from_offset(start)?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_auto_pre_mayhem_phase(Some(&mut store.words))?;
        store.frames.push(Frame::Phase { record: index });
        Some(index)
    }

    /// Decode only the fixed AutoPre/normal Mayhem phase quotient.
    #[cold]
    #[inline(never)]
    pub(crate) fn auto_pre_mayhem_phase(
        &self,
        index: WordRecordIndex,
    ) -> Option<AutoPreMayhemPhaseRecord> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::Phase || count != 1 {
            return None;
        }
        (*self.0.words.get(start + 1)? == PendingWord { body: 1, meta: 1 })
            .then_some(AutoPreMayhemPhaseRecord)
    }

    /// Pop only the authenticated fixed AutoPre/normal Mayhem phase.
    #[cold]
    #[inline(never)]
    pub(crate) fn pop_top_auto_pre_mayhem_phase(&mut self) -> Option<AutoPreMayhemPhaseRecord> {
        let Frame::Phase { record } = self.top()? else {
            return None;
        };
        let owned = self.auto_pre_mayhem_phase(record)?;
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    #[cold]
    #[inline(never)]
    fn encode_auto_pre_history_course_phase(
        dupe_uid: u32,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        if dupe_uid == 0 || dupe_uid == LEGACY_CARD_UID {
            return None;
        }
        let Some(words) = destination else {
            return Some(2);
        };
        words.reserve(2);
        words.push(PendingWord::header(WordRecordKind::Phase, 1)?);
        // meta=3 keeps this single-word record disjoint from the Mayhem
        // quotient ({1, 1}); the AutoPost and Bombardment grammars need at
        // least two body words.
        words.push(PendingWord {
            body: dupe_uid,
            meta: 3,
        });
        Some(2)
    }

    /// Append the AutoPre/Normal History Course phase that owns one awaited
    /// `CardCmd.AutoPlay` of the dupe `dupe_uid` (#3309).
    #[cold]
    #[inline(never)]
    pub(crate) fn push_auto_pre_history_course_phase(
        &mut self,
        dupe_uid: u32,
    ) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_auto_pre_history_course_phase(dupe_uid, None)?;
        let start = self.0.words.len();
        let index = WordRecordIndex::from_offset(start)?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_auto_pre_history_course_phase(dupe_uid, Some(&mut store.words))?;
        store.frames.push(Frame::Phase { record: index });
        Some(index)
    }

    /// Decode the History Course AutoPre phase: the dupe's uid.
    #[cold]
    #[inline(never)]
    pub(crate) fn auto_pre_history_course_phase(&self, index: WordRecordIndex) -> Option<u32> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::Phase || count != 1 {
            return None;
        }
        let word = *self.0.words.get(start + 1)?;
        (word.meta == 3 && word.body != 0 && word.body != LEGACY_CARD_UID).then_some(word.body)
    }

    /// Pop only the authenticated top History Course AutoPre phase.
    #[cold]
    #[inline(never)]
    pub(crate) fn pop_top_auto_pre_history_course_phase(&mut self) -> Option<u32> {
        let Frame::Phase { record } = self.top()? else {
            return None;
        };
        let dupe_uid = self.auto_pre_history_course_phase(record)?;
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(dupe_uid)
    }

    fn encode_auto_pre_bombardment_phase(
        record: &AutoPreBombardmentPhaseRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        if record.listeners.is_empty()
            || usize::try_from(record.cursor).ok()? > record.listeners.len()
            || record
                .listeners
                .iter()
                .enumerate()
                .any(|(index, uid)| *uid == 0 || record.listeners[..index].contains(uid))
        {
            return None;
        }
        let encoded_len = record.listeners.len().checked_add(2)?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        words.reserve(encoded_len);
        words.push(PendingWord::header(
            WordRecordKind::Phase,
            encoded_len.checked_sub(1)?,
        )?);
        // meta=2 distinguishes AutoPre/Early from the frozen Mayhem and
        // AutoPost grammars. Each listener word uses meta=2 as a stable UID.
        words.push(PendingWord {
            body: record.cursor,
            meta: 2,
        });
        words.extend(
            record
                .listeners
                .iter()
                .copied()
                .map(|body| PendingWord { body, meta: 2 }),
        );
        Some(encoded_len)
    }

    pub(crate) fn push_auto_pre_bombardment_phase(
        &mut self,
        listeners: &[u32],
    ) -> Option<WordRecordIndex> {
        let record = AutoPreBombardmentPhaseRecord {
            cursor: 0,
            listeners: listeners.to_vec(),
        };
        let encoded_len = Self::encode_auto_pre_bombardment_phase(&record, None)?;
        let start = self.0.words.len();
        let index = WordRecordIndex::from_offset(start)?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_auto_pre_bombardment_phase(&record, Some(&mut store.words))?;
        store.frames.push(Frame::Phase { record: index });
        Some(index)
    }

    pub(crate) fn auto_pre_bombardment_phase(
        &self,
        index: WordRecordIndex,
    ) -> Option<AutoPreBombardmentPhaseView<'_>> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::Phase || count < 2 {
            return None;
        }
        let end = start.checked_add(1)?.checked_add(count as usize)?;
        let fixed = *self.0.words.get(start + 1)?;
        let listener_words = self.0.words.get(start + 2..end)?;
        if fixed.meta != 2
            || listener_words.is_empty()
            || usize::try_from(fixed.body).ok()? > listener_words.len()
            || listener_words.iter().enumerate().any(|(position, word)| {
                word.meta != 2
                    || word.body == 0
                    || listener_words[..position]
                        .iter()
                        .any(|prior| prior.body == word.body)
            })
        {
            return None;
        }
        Some(AutoPreBombardmentPhaseView {
            cursor: fixed.body,
            listener_words,
        })
    }

    pub(crate) fn replace_top_auto_pre_bombardment_phase(
        &mut self,
        record: &AutoPreBombardmentPhaseRecord,
    ) -> Option<()> {
        let Frame::Phase { record: index } = self.top()? else {
            return None;
        };
        self.auto_pre_bombardment_phase(index)?;
        let encoded_len = Self::encode_auto_pre_bombardment_phase(record, None)?;
        let start = index.offset()?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        Self::encode_auto_pre_bombardment_phase(record, Some(&mut store.words))?;
        debug_assert_eq!(store.words.len(), start + encoded_len);
        Some(())
    }

    pub(crate) fn pop_top_auto_pre_bombardment_phase(
        &mut self,
    ) -> Option<AutoPreBombardmentPhaseRecord> {
        let Frame::Phase { record } = self.top()? else {
            return None;
        };
        let owned = self.auto_pre_bombardment_phase(record)?.to_owned();
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    fn encode_stampede_live(
        record: StampedeLiveRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        if record.iterations == 0 || record.cursor > record.iterations {
            return None;
        }
        let Some(words) = destination else {
            return Some(2);
        };
        words.reserve(2);
        words.push(PendingWord::header(WordRecordKind::LiveListener, 1)?);
        words.push(PendingWord {
            body: record.cursor,
            meta: record.iterations,
        });
        Some(2)
    }

    /// Append Stampede's admitted live-loop atomically.
    pub(crate) fn push_stampede_live(
        &mut self,
        record: StampedeLiveRecord,
    ) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_stampede_live(record, None)?;
        let start = self.0.words.len();
        let index = WordRecordIndex::from_offset(start)?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_stampede_live(record, Some(&mut store.words))?;
        store.frames.push(Frame::LiveListener { record: index });
        Some(index)
    }

    /// Decode Stampede's admitted live-loop record.
    pub(crate) fn stampede_live(&self, index: WordRecordIndex) -> Option<StampedeLiveRecord> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::LiveListener || count != 1 {
            return None;
        }
        let word = *self.0.words.get(start + 1)?;
        (word.meta > 0 && word.body <= word.meta).then_some(StampedeLiveRecord {
            cursor: word.body,
            iterations: word.meta,
        })
    }

    /// Replace only the authenticated top Stampede live loop.
    pub(crate) fn replace_top_stampede_live(&mut self, record: StampedeLiveRecord) -> Option<()> {
        let Frame::LiveListener { record: index } = self.top()? else {
            return None;
        };
        self.stampede_live(index)?;
        Self::encode_stampede_live(record, None)?;
        let start = index.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        Self::encode_stampede_live(record, Some(&mut store.words))?;
        Some(())
    }

    /// Pop only the authenticated top Stampede live loop.
    pub(crate) fn pop_top_stampede_live(&mut self) -> Option<StampedeLiveRecord> {
        let Frame::LiveListener { record } = self.top()? else {
            return None;
        };
        let owned = self.stampede_live(record)?;
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    fn encode_catastrophe_live(
        record: CatastropheLiveRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        const ITERATIONS: u32 = 3;
        if record.cursor > ITERATIONS || record.source_uid == u32::MAX {
            return None;
        }
        let Some(words) = destination else {
            return Some(3);
        };
        words.reserve(3);
        words.push(PendingWord::header(WordRecordKind::LiveListener, 2)?);
        words.push(PendingWord {
            body: record.cursor,
            meta: ITERATIONS,
        });
        words.push(PendingWord {
            body: record.source_uid,
            meta: 0,
        });
        Some(3)
    }

    /// Append Catastrophe's bounded live-query loop atomically.
    pub(crate) fn push_catastrophe_live(
        &mut self,
        record: CatastropheLiveRecord,
    ) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_catastrophe_live(record, None)?;
        let start = self.0.words.len();
        let index = WordRecordIndex::from_offset(start)?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_catastrophe_live(record, Some(&mut store.words))?;
        store.frames.push(Frame::LiveListener { record: index });
        Some(index)
    }

    /// Decode Catastrophe's admitted live-query loop record.
    pub(crate) fn catastrophe_live(&self, index: WordRecordIndex) -> Option<CatastropheLiveRecord> {
        const ITERATIONS: u32 = 3;
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::LiveListener || count != 2 {
            return None;
        }
        let cursor = *self.0.words.get(start + 1)?;
        let source = *self.0.words.get(start + 2)?;
        (cursor.meta == ITERATIONS
            && cursor.body <= ITERATIONS
            && source.meta == 0
            && source.body != u32::MAX)
            .then_some(CatastropheLiveRecord {
                cursor: cursor.body,
                source_uid: source.body,
            })
    }

    /// Replace only the authenticated top Catastrophe live loop.
    pub(crate) fn replace_top_catastrophe_live(
        &mut self,
        record: CatastropheLiveRecord,
    ) -> Option<()> {
        let Frame::LiveListener { record: index } = self.top()? else {
            return None;
        };
        self.catastrophe_live(index)?;
        Self::encode_catastrophe_live(record, None)?;
        let start = index.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        Self::encode_catastrophe_live(record, Some(&mut store.words))?;
        Some(())
    }

    /// Pop only the authenticated top Catastrophe live loop.
    pub(crate) fn pop_top_catastrophe_live(&mut self) -> Option<CatastropheLiveRecord> {
        let Frame::LiveListener { record } = self.top()? else {
            return None;
        };
        let owned = self.catastrophe_live(record)?;
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    #[cold]
    #[inline(never)]
    fn genetic_algorithm_from_raw(raw: u64) -> Option<GeneticAlgorithmState> {
        let growth = (raw & GeneticAlgorithmState::GROWTH_MASK) as i32;
        let row_payload = ((raw >> GeneticAlgorithmState::ROW_SHIFT) & u64::from(u32::MAX)) as u32;
        let deck_row = if raw & GeneticAlgorithmState::ROW_PRESENT != 0 {
            Some(row_payload)
        } else {
            if row_payload != 0 {
                return None;
            }
            None
        };
        let reconstructed = GeneticAlgorithmState::from_parts(growth, deck_row)?;
        (reconstructed.0 == raw).then_some(reconstructed)
    }

    #[cold]
    #[inline(never)]
    fn encode_frozen_auto_batch_entry(
        entry: &FrozenAutoBatchEntry,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        const FIXED_BODY_WORDS: usize = 6;

        let card = entry.card;
        let state = &entry.state;
        if card.flags & !CARD_FLAGS_KNOWN != 0 || card.flags & CARD_FLAG_LEGACY != 0 {
            return None;
        }
        let enchantment_state = state.enchantment_state.0;
        if enchantment_state > (i32::MAX as u32).checked_add(1)? {
            return None;
        }
        let base_replay_count = state.base_replay_count.0;
        if !matches!(base_replay_count, -1..=i32::MAX) {
            return None;
        }
        if state.damage_growth < 0 {
            return None;
        }
        let decimal_bits = state.local_cost_modifiers.0.thrash_growth;
        if decimal_bits.is_some() && state.damage_growth != 0
            || state.local_cost_modifiers.0.thrash_growth_is_fraction && decimal_bits.is_none()
        {
            return None;
        }
        if let Some(bits) = decimal_bits
            && (bits.negative || crate::decimal::DotNetDecimal::from_bits(bits).is_err())
        {
            return None;
        }
        if let Some(bits) = decimal_bits
            && !state.local_cost_modifiers.0.thrash_growth_is_fraction
            && bits.scale == 0
            && bits.hi == 0
            && bits.mid == 0
            && i32::try_from(bits.lo).is_ok()
        {
            return None;
        }
        Self::genetic_algorithm_from_raw(state.genetic_algorithm.0)?;
        if !state
            .local_cost_modifiers
            .star_encoding_is_exact(state.free_star_cost_this_turn_or_played_rows)
        {
            return None;
        }
        let rows = state.local_cost_modifiers.as_slice();
        let star_rows = &state.local_cost_modifiers.0.star_expirations;
        let row_words = rows.len().checked_add(star_rows.len())?.checked_mul(2)?;
        let body_count = FIXED_BODY_WORDS.checked_add(row_words)?;
        let encoded_len = 1_usize.checked_add(body_count)?;
        PendingWord::header(WordRecordKind::FrozenAutoBatchEntry, body_count)?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        words.reserve(encoded_len);
        words.push(PendingWord::header(
            WordRecordKind::FrozenAutoBatchEntry,
            body_count,
        )?);
        words.push(PendingWord::from_card(card));
        words.push(PendingWord {
            body: enchantment_state,
            meta: base_replay_count as u32,
        });
        let control = u32::from(state.free_star_cost_this_turn_or_played_rows)
            | (u32::from(state.local_retain) << 8)
            | (u32::from(state.local_sly) << 9)
            | (u32::from(state.transient_retain) << 10)
            | (u32::from(decimal_bits.is_some()) << 11)
            | (u32::from(state.local_cost_modifiers.0.thrash_growth_is_fraction) << 12)
            | (u32::from(state.local_cost_modifiers.free_star_cost_this_combat()) << 13)
            | (u32::from(state.local_ethereal()) << 14)
            | (u32::from(state.transient_sly()) << 15);
        words.push(PendingWord {
            body: state.damage_growth as u32,
            meta: control,
        });
        words.push(PendingWord {
            body: state.genetic_algorithm.0 as u32,
            meta: (state.genetic_algorithm.0 >> 32) as u32,
        });
        let decimal_bits = decimal_bits.unwrap_or(crate::decimal::DotNetDecimalBits {
            lo: 0,
            mid: 0,
            hi: 0,
            negative: false,
            scale: 0,
        });
        words.push(PendingWord {
            body: decimal_bits.lo,
            meta: decimal_bits.mid,
        });
        words.push(PendingWord {
            body: decimal_bits.hi,
            meta: decimal_bits.scale,
        });
        for row in rows {
            words.push(PendingWord {
                body: row.amount as u64 as u32,
                meta: ((row.amount as u64) >> 32) as u32,
            });
            words.push(PendingWord {
                body: u32::from(row.kind as u8)
                    | (u32::from(row.expiration as u8) << 8)
                    | (u32::from(row.reduce_only) << 16),
                meta: 0,
            });
        }
        // Kind 3 is an internal Star-row tag; it is never an Energy operation.
        for expiration in star_rows {
            words.push(PendingWord { body: 0, meta: 0 });
            words.push(PendingWord {
                body: 3 | (u32::from(*expiration as u8) << 8),
                meta: 0,
            });
        }
        Some(encoded_len)
    }

    #[cold]
    #[inline(never)]
    fn decode_frozen_auto_batch_entry(
        words: &[PendingWord],
    ) -> Option<(FrozenAutoBatchEntry, usize)> {
        const FIXED_BODY_WORDS: usize = 6;
        const CONTROL_MASK: u32 = 0xffff;

        let (kind, count) = words.first()?.parse_header()?;
        let count = usize::try_from(count).ok()?;
        if kind != WordRecordKind::FrozenAutoBatchEntry
            || count < FIXED_BODY_WORDS
            || !(count - FIXED_BODY_WORDS).is_multiple_of(2)
        {
            return None;
        }
        let encoded_len = 1_usize.checked_add(count)?;
        let body = words.get(1..encoded_len)?;
        let card_word = body[0];
        let card = HotCard {
            uid: card_word.body,
            atom: card_word.meta as u16,
            flags: (card_word.meta >> 16) as u16,
        };
        if card.flags & !CARD_FLAGS_KNOWN != 0 || card.flags & CARD_FLAG_LEGACY != 0 {
            return None;
        }
        let scalar = body[1];
        if scalar.body > (i32::MAX as u32).checked_add(1)? {
            return None;
        }
        let base_replay_count = scalar.meta as i32;
        if !matches!(base_replay_count, -1..=i32::MAX) {
            return None;
        }
        let damage = body[2];
        if damage.meta & !CONTROL_MASK != 0 || damage.body > i32::MAX as u32 {
            return None;
        }
        let has_decimal = damage.meta & (1 << 11) != 0;
        let decimal_is_fraction = damage.meta & (1 << 12) != 0;
        if decimal_is_fraction && !has_decimal || has_decimal && damage.body != 0 {
            return None;
        }
        let decimal_lo_mid = body[4];
        let decimal_hi_scale = body[5];
        let decimal_bits = crate::decimal::DotNetDecimalBits {
            lo: decimal_lo_mid.body,
            mid: decimal_lo_mid.meta,
            hi: decimal_hi_scale.body,
            negative: false,
            scale: decimal_hi_scale.meta,
        };
        let decimal = if has_decimal {
            Some(crate::decimal::DotNetDecimal::from_bits(decimal_bits).ok()?)
        } else {
            if decimal_bits
                != (crate::decimal::DotNetDecimalBits {
                    lo: 0,
                    mid: 0,
                    hi: 0,
                    negative: false,
                    scale: 0,
                })
            {
                return None;
            }
            None
        };
        let mut rows = Vec::with_capacity((count - FIXED_BODY_WORDS) / 2);
        let mut star_rows = Vec::new();
        for pair in body[FIXED_BODY_WORDS..].chunks_exact(2) {
            let amount = ((u64::from(pair[0].meta) << 32) | u64::from(pair[0].body)) as i64;
            let control = pair[1];
            if control.meta != 0 || control.body & !0x1_ffff != 0 {
                return None;
            }
            let expiration = LocalCostExpiration::from_wire(i64::from((control.body >> 8) & 0xff))?;
            if control.body & 0xff == 3 {
                if amount != 0 || control.body & (1 << 16) != 0 {
                    return None;
                }
                star_rows.push(expiration);
                continue;
            }
            if !star_rows.is_empty() {
                return None;
            }
            let kind = LocalCostModifierKind::from_wire(i64::from(control.body & 0xff))?;
            rows.push(LocalCostModifier {
                kind,
                amount,
                expiration,
                reduce_only: control.body & (1 << 16) != 0,
            });
        }
        let genetic_raw = u64::from(body[3].body) | (u64::from(body[3].meta) << 32);
        let mut local_cost_modifiers = LocalCostModifiers::from_rows(rows);
        if damage.meta & (1 << 13) != 0 {
            local_cost_modifiers.set_free_star_cost_this_combat()?;
        }
        if !star_rows.is_empty()
            && (local_cost_modifiers.set_star_cost_expirations(&star_rows)? != damage.meta as u8
                || local_cost_modifiers.free_star_cost_this_combat()
                    != (damage.meta & (1 << 13) != 0))
        {
            return None;
        }
        if damage.meta & (1 << 14) != 0 {
            Arc::make_mut(&mut local_cost_modifiers.0).local_ethereal = true;
        }
        let mut state = CardInstanceState {
            enchantment_state: OptionalNonNegativeI32(scalar.body),
            base_replay_count: BaseReplayCount(base_replay_count),
            damage_growth: damage.body as i32,
            local_cost_modifiers,
            free_star_cost_this_turn_or_played_rows: damage.meta as u8,
            genetic_algorithm: Self::genetic_algorithm_from_raw(genetic_raw)?,
            local_retain: damage.meta & (1 << 8) != 0,
            local_sly: damage.meta & (1 << 9) != 0,
            transient_retain: damage.meta & (1 << 10) != 0,
        };
        state.set_transient_sly(damage.meta & (1 << 15) != 0);
        if let Some(decimal) = decimal {
            if decimal_is_fraction {
                state.set_fraction_damage_growth(decimal)?;
            } else {
                state.set_exact_damage_growth(decimal)?;
            }
        }
        let entry = FrozenAutoBatchEntry { card, state };
        let mut canonical_words = Vec::with_capacity(encoded_len);
        if Self::encode_frozen_auto_batch_entry(&entry, Some(&mut canonical_words))? != encoded_len
            || canonical_words.as_slice() != words.get(..encoded_len)?
        {
            return None;
        }
        Some((entry, encoded_len))
    }

    #[cold]
    #[inline(never)]
    fn encode_frozen_auto_batch(
        record: &FrozenAutoBatchRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        let gathering = record.gather_target != 0;
        if (!gathering && record.entries.is_empty())
            || !gathering && usize::try_from(record.cursor).ok()? > record.entries.len()
            || gathering
                && (record.gather_target != 3
                    || record.cursor > u32::from(record.gather_target)
                    || record.entries.len() > usize::try_from(record.cursor).ok()?
                    || record.entries.len() >= usize::from(record.gather_target)
                    || record.source != FrozenAutoBatchSource::DrawPileFlip
                    || record.force_exhaust)
            || !record.source.force_exhaust_is_exact(record.force_exhaust)
            || record.source == FrozenAutoBatchSource::Decisions
                && (record.entries.len() != 3
                    || record
                        .entries
                        .iter()
                        .any(|entry| entry != &record.entries[0]))
            || record.source != FrozenAutoBatchSource::Decisions
                && record.entries.iter().enumerate().any(|(index, entry)| {
                    record.entries[..index]
                        .iter()
                        .any(|prior| prior.card.uid == entry.card.uid)
                })
        {
            return None;
        }
        let mut body_count = 1_usize;
        for entry in &record.entries {
            body_count =
                body_count.checked_add(Self::encode_frozen_auto_batch_entry(entry, None)?)?;
        }
        let encoded_len = 1_usize.checked_add(body_count)?;
        PendingWord::header(WordRecordKind::FrozenAutoBatch, body_count)?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        words.reserve(encoded_len);
        words.push(PendingWord::header(
            WordRecordKind::FrozenAutoBatch,
            body_count,
        )?);
        words.push(PendingWord {
            body: record.cursor,
            meta: u32::from(record.source as u8)
                | (u32::from(record.force_exhaust) << 8)
                | (u32::from(record.gather_target) << 16),
        });
        for entry in &record.entries {
            Self::encode_frozen_auto_batch_entry(entry, Some(words))?;
        }
        Some(encoded_len)
    }

    /// Append one syntactically valid frozen-card AutoBatch atomically.
    #[cold]
    #[inline(never)]
    pub(crate) fn push_frozen_auto_batch(
        &mut self,
        record: &FrozenAutoBatchRecord,
    ) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_frozen_auto_batch(record, None)?;
        let start = self.0.words.len();
        let index = WordRecordIndex::from_offset(start)?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        Self::encode_frozen_auto_batch(record, Some(&mut store.words))?;
        store.frames.push(Frame::FrozenAutoBatch { record: index });
        Some(index)
    }

    /// Decode one syntactically authenticated frozen-card AutoBatch record.
    /// Semantic ownership is deliberately a separate cold boundary proof.
    #[cold]
    #[inline(never)]
    pub(crate) fn frozen_auto_batch(
        &self,
        index: WordRecordIndex,
    ) -> Option<FrozenAutoBatchView<'_>> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::FrozenAutoBatch || count < 1 {
            return None;
        }
        let end = start.checked_add(1)?.checked_add(count as usize)?;
        let fixed = *self.0.words.get(start + 1)?;
        let source = FrozenAutoBatchSource::from_ordinal(fixed.meta & 0xff)?;
        let force_exhaust = fixed.meta & (1 << 8) != 0;
        let gather_target = ((fixed.meta >> 16) & 0xff) as u8;
        if fixed.meta & !0x00ff_01ff != 0 || !source.force_exhaust_is_exact(force_exhaust) {
            return None;
        }
        let entry_words = self.0.words.get(start + 2..end)?;
        let mut remaining = entry_words;
        let mut uids = Vec::new();
        let mut first_entry = None;
        while !remaining.is_empty() {
            let (entry, consumed) = Self::decode_frozen_auto_batch_entry(remaining)?;
            if source == FrozenAutoBatchSource::Decisions {
                if first_entry.as_ref().is_some_and(|first| first != &entry) {
                    return None;
                }
                first_entry = Some(entry.clone());
            }
            if source != FrozenAutoBatchSource::Decisions && uids.contains(&entry.card.uid) {
                return None;
            }
            uids.push(entry.card.uid);
            remaining = remaining.get(consumed..)?;
        }
        let gathering = gather_target != 0;
        if source == FrozenAutoBatchSource::Decisions && (uids.len() != 3 || gathering)
            || (!gathering && uids.is_empty())
            || !gathering && usize::try_from(fixed.body).ok()? > uids.len()
            || gathering
                && (gather_target != 3
                    || fixed.body > u32::from(gather_target)
                    || uids.len() > usize::try_from(fixed.body).ok()?
                    || uids.len() >= usize::from(gather_target)
                    || source != FrozenAutoBatchSource::DrawPileFlip
                    || force_exhaust)
        {
            return None;
        }
        Some(FrozenAutoBatchView {
            cursor: fixed.body,
            gather_target,
            force_exhaust,
            source,
            entry_words,
            entry_count: uids.len(),
        })
    }

    /// Replace only the authenticated top frozen AutoBatch record.
    #[cold]
    #[inline(never)]
    pub(crate) fn replace_top_frozen_auto_batch(
        &mut self,
        record: &FrozenAutoBatchRecord,
    ) -> Option<()> {
        let Frame::FrozenAutoBatch { record: index } = self.top()? else {
            return None;
        };
        self.frozen_auto_batch(index)?;
        let encoded_len = Self::encode_frozen_auto_batch(record, None)?;
        let start = index.offset()?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        Self::encode_frozen_auto_batch(record, Some(&mut store.words))?;
        Some(())
    }

    /// Pop only the authenticated top frozen AutoBatch and its word suffix.
    #[cold]
    #[inline(never)]
    pub(crate) fn pop_top_frozen_auto_batch(&mut self) -> Option<FrozenAutoBatchRecord> {
        let Frame::FrozenAutoBatch { record } = self.top()? else {
            return None;
        };
        let owned = self.frozen_auto_batch(record)?.to_owned();
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    #[cold]
    #[inline(never)]
    fn encode_turn_start_hand_choice(
        record: &TurnStartHandChoiceRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        let order_len = record.listener_order.len();
        let appended_len = record.appended_suffix.len();
        let retained_order = record
            .listener_order
            .iter()
            .enumerate()
            .filter_map(|(index, kind)| {
                (u32::try_from(index).ok()? >= record.next_listener
                    || *kind != TurnStartHandChoiceKind::SummonNextTurn)
                    .then_some(*kind)
            })
            .collect::<Vec<_>>();
        let entry_count = u32::try_from(record.entries.len()).ok()?;
        if !(1..=8).contains(&order_len)
            || retained_order.len().checked_add(appended_len)? > 8
            || record.listener_order[0..order_len]
                .iter()
                .enumerate()
                .any(|(index, kind)| record.listener_order[..index].contains(kind))
            || record
                .appended_suffix
                .iter()
                .enumerate()
                .any(|(index, kind)| {
                    retained_order.contains(kind) || record.appended_suffix[..index].contains(kind)
                })
            || !record.appended_suffix.is_empty()
                && !(record.kind == TurnStartHandChoiceKind::ToolsOfTheTrade
                    && record.phase == TurnStartHandChoicePhase::Effect
                    || usize::try_from(record.next_listener.saturating_sub(1))
                        .ok()
                        .and_then(|completed| record.listener_order.get(..completed))
                        .is_some_and(|completed| {
                            completed.contains(&TurnStartHandChoiceKind::ToolsOfTheTrade)
                        }))
            || record.amount == 0
            || !(1..=u32::try_from(order_len).ok()?).contains(&record.next_listener)
            || record.kind != record.listener_order[record.next_listener as usize - 1]
            || record.entries.is_empty()
            || record.entries.iter().enumerate().any(|(index, entry)| {
                record.entries[..index]
                    .iter()
                    .any(|prior| prior.card.uid == entry.card.uid)
            })
            || record.phase == TurnStartHandChoicePhase::Selecting
                && (!record.kind.selects_hand()
                    || record.cursor != 0
                    || record.entries.len() <= record.amount as usize)
            || record.phase == TurnStartHandChoicePhase::Effect
                && (entry_count > record.amount
                    || record.kind == TurnStartHandChoiceKind::ToolsOfTheTrade
                        && record.cursor != entry_count
                    || record.kind == TurnStartHandChoiceKind::Tyranny
                        && record.cursor > entry_count
                    || record.kind == TurnStartHandChoiceKind::Entropy
                        && record.cursor > entry_count
                    || !record.kind.selects_hand())
        {
            return None;
        }
        let mut body_count = 3_usize;
        for entry in &record.entries {
            body_count =
                body_count.checked_add(Self::encode_frozen_auto_batch_entry(entry, None)?)?;
        }
        let encoded_len = body_count.checked_add(1)?;
        PendingWord::header(WordRecordKind::TurnStartHandChoice, body_count)?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        let control = u32::from(record.phase as u8)
            | (u32::from(record.kind as u8) << 8)
            | (u32::try_from(order_len).ok()? << 16);
        let mut packed_order = 0_u32;
        for (index, kind) in record.listener_order.iter().enumerate() {
            packed_order |= u32::from(*kind as u8) << (index * 3);
        }
        let mut packed_appended = 0_u32;
        for (index, kind) in record.appended_suffix.iter().enumerate() {
            packed_appended |= u32::from(*kind as u8) << (index * 3);
        }
        packed_appended |= u32::try_from(appended_len).ok()? << 24;
        words.reserve(encoded_len);
        words.push(PendingWord::header(
            WordRecordKind::TurnStartHandChoice,
            body_count,
        )?);
        words.push(PendingWord {
            body: record.cursor,
            meta: control,
        });
        words.push(PendingWord {
            body: record.amount,
            meta: record.next_listener,
        });
        words.push(PendingWord {
            body: packed_order,
            meta: packed_appended,
        });
        for entry in &record.entries {
            Self::encode_frozen_auto_batch_entry(entry, Some(words))?;
        }
        Some(encoded_len)
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn push_turn_start_hand_choice(
        &mut self,
        record: &TurnStartHandChoiceRecord,
    ) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_turn_start_hand_choice(record, None)?;
        let start = self.0.words.len();
        let index = WordRecordIndex::from_offset(start)?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        Self::encode_turn_start_hand_choice(record, Some(&mut store.words))?;
        store
            .frames
            .push(Frame::TurnStartHandChoice { record: index });
        Some(index)
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn turn_start_hand_choice(
        &self,
        index: WordRecordIndex,
    ) -> Option<TurnStartHandChoiceView<'_>> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::TurnStartHandChoice || count < 10 {
            return None;
        }
        let end = start.checked_add(1)?.checked_add(count as usize)?;
        let fixed = *self.0.words.get(start + 1)?;
        let phase = TurnStartHandChoicePhase::from_ordinal(fixed.meta & 0xff)?;
        let selected_kind = TurnStartHandChoiceKind::from_ordinal((fixed.meta >> 8) & 0xff)?;
        let listener_count = usize::try_from((fixed.meta >> 16) & 0xf).ok()?;
        if !(1..=8).contains(&listener_count) {
            return None;
        }
        let order_word = *self.0.words.get(start + 3)?;
        let appended_suffix_count = usize::try_from((order_word.meta >> 24) & 0xf).ok()?;
        if appended_suffix_count > 8
            || order_word.meta >> 28 != 0
            || order_word.body >> (listener_count * 3) != 0
            || (order_word.meta & 0x00ff_ffff) >> (appended_suffix_count * 3) != 0
        {
            return None;
        }
        let mut listener_order = [None; 8];
        for (index, slot) in listener_order.iter_mut().take(listener_count).enumerate() {
            *slot = Some(TurnStartHandChoiceKind::from_ordinal(
                (order_word.body >> (index * 3)) & 0x7,
            )?);
        }
        if listener_order[..listener_count]
            .iter()
            .enumerate()
            .any(|(index, kind)| listener_order[..index].contains(kind))
        {
            return None;
        }
        let mut appended_suffix = [None; 8];
        for (index, slot) in appended_suffix
            .iter_mut()
            .take(appended_suffix_count)
            .enumerate()
        {
            *slot = Some(TurnStartHandChoiceKind::from_ordinal(
                (order_word.meta >> (index * 3)) & 0x7,
            )?);
        }
        let exact_control = u32::from(phase as u8)
            | (u32::from(selected_kind as u8) << 8)
            | (u32::try_from(listener_count).ok()? << 16);
        if fixed.meta != exact_control {
            return None;
        }
        let scalar = *self.0.words.get(start + 2)?;
        let entry_words = self.0.words.get(start + 4..end)?;
        let mut remaining = entry_words;
        let mut entries = Vec::new();
        while !remaining.is_empty() {
            let (entry, consumed) = Self::decode_frozen_auto_batch_entry(remaining)?;
            if entries
                .iter()
                .any(|prior: &FrozenAutoBatchEntry| prior.card.uid == entry.card.uid)
            {
                return None;
            }
            entries.push(entry);
            remaining = remaining.get(consumed..)?;
        }
        let view = TurnStartHandChoiceView {
            listener_order,
            listener_count,
            appended_suffix,
            appended_suffix_count,
            kind: selected_kind,
            amount: scalar.body,
            next_listener: scalar.meta,
            cursor: fixed.body,
            phase,
            entry_words,
            entry_count: entries.len(),
        };
        Self::encode_turn_start_hand_choice(&view.to_owned(), None)?;
        Some(view)
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn replace_top_turn_start_hand_choice(
        &mut self,
        record: &TurnStartHandChoiceRecord,
    ) -> Option<()> {
        let Frame::TurnStartHandChoice { record: index } = self.top()? else {
            return None;
        };
        self.turn_start_hand_choice(index)?;
        let encoded_len = Self::encode_turn_start_hand_choice(record, None)?;
        let start = index.offset()?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        Self::encode_turn_start_hand_choice(record, Some(&mut store.words))?;
        Some(())
    }

    /// Authenticate one listener appended by a Tools-owned Sly CardPlay.
    ///
    /// The proof occupies spare bits in the existing turn-start word record,
    /// so rewriting a non-top parent never changes any later frame offset.
    pub(crate) fn record_turn_start_hand_choice_sly_append(
        &mut self,
        power: PowerId,
    ) -> Option<()> {
        let appended = TurnStartHandChoiceKind::from_power(power)?;
        let parents = self
            .0
            .frames
            .iter()
            .enumerate()
            .filter_map(|(position, frame)| {
                let Frame::TurnStartHandChoice { record } = *frame else {
                    return None;
                };
                self.turn_start_hand_choice(record)
                    .filter(|view| {
                        view.phase == TurnStartHandChoicePhase::Effect
                            && view.kind == TurnStartHandChoiceKind::ToolsOfTheTrade
                    })
                    .map(|view| (position, record, view.to_owned()))
            })
            .collect::<Vec<_>>();
        if parents.is_empty() {
            return Some(());
        }
        if parents.len() != 1 {
            return None;
        }
        let (parent_position, parent_index, mut parent) =
            parents.into_iter().next().expect("one Tools parent");
        let selected_uids = parent
            .entries
            .iter()
            .map(|entry| entry.card.uid)
            .collect::<Vec<_>>();
        if !crate::engine::play::active_tools_sly_descendant(&selected_uids) {
            let top_position = self.0.frames.len().checked_sub(1)?;
            let Frame::CardPlay {
                record: child_index,
            } = self.0.frames[top_position]
            else {
                return Some(());
            };
            let child_uid = self.card_play(child_index)?.uid;
            let mut position = parent_position + 1;
            let mut batched_root_uid = None;
            if let Some(Frame::FrozenAutoBatch { record }) = self.0.frames.get(position).copied() {
                let batch = self.frozen_auto_batch(record)?;
                let child_position = usize::try_from(batch.cursor).ok()?.checked_sub(1)?;
                if batch.source != FrozenAutoBatchSource::SlyDiscard || batch.force_exhaust {
                    return None;
                }
                batched_root_uid = batch.uid(child_position);
                position += 1;
            }
            let Some(Frame::CardPlay { record }) = self.0.frames.get(position).copied() else {
                return None;
            };
            let root = self.card_play(record)?;
            if root.source != CardPlaySource::SlyDiscard
                || batched_root_uid.is_some_and(|uid| uid != root.uid)
                || root.force_exhaust
                || !parent
                    .entries
                    .iter()
                    .any(|entry| entry.card.uid == root.uid)
            {
                return None;
            }
            position += 1;
            while position <= top_position {
                let Some(Frame::FrozenAutoBatch {
                    record: batch_record,
                }) = self.0.frames.get(position).copied()
                else {
                    return None;
                };
                let Some(Frame::CardPlay {
                    record: play_record,
                }) = self.0.frames.get(position + 1).copied()
                else {
                    return None;
                };
                let batch = self.frozen_auto_batch(batch_record)?;
                let play = self.card_play(play_record)?;
                let child_position = usize::try_from(batch.cursor).ok()?.checked_sub(1)?;
                if batch.uid(child_position) != Some(play.uid)
                    || match batch.source {
                        FrozenAutoBatchSource::DrawPileFlip => {
                            play.source != CardPlaySource::DrawPileFlip
                                || play.force_exhaust != batch.force_exhaust
                        }
                        FrozenAutoBatchSource::SlyDiscard => {
                            play.source != CardPlaySource::SlyDiscard
                                || play.force_exhaust
                                || batch.force_exhaust
                        }
                        FrozenAutoBatchSource::BeatDown => {
                            play.source != CardPlaySource::BeatDown || play.force_exhaust
                        }
                        FrozenAutoBatchSource::Eidolon => {
                            play.source != CardPlaySource::Eidolon || !play.force_exhaust
                        }
                        FrozenAutoBatchSource::KnifeTrap => {
                            play.source != CardPlaySource::KnifeTrap || play.force_exhaust
                        }
                        FrozenAutoBatchSource::Decisions => {
                            play.source != CardPlaySource::Decisions || play.force_exhaust
                        }
                    }
                {
                    return None;
                }
                position += 2;
            }
            if position != top_position + 1 || child_uid != self.card_play(child_index)?.uid {
                return None;
            }
        }
        parent.appended_suffix.push(appended);
        let mut encoded = Vec::new();
        let encoded_len = Self::encode_turn_start_hand_choice(&parent, Some(&mut encoded))?;
        let start = parent_index.offset()?;
        let old_len = self.0.words.get(start)?.parse_header()?.1 as usize + 1;
        if encoded_len != old_len || encoded.len() != old_len {
            return None;
        }
        Arc::make_mut(&mut self.0).words[start..start + old_len].copy_from_slice(&encoded);
        Some(())
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn pop_top_turn_start_hand_choice(&mut self) -> Option<TurnStartHandChoiceRecord> {
        let Frame::TurnStartHandChoice { record } = self.top()? else {
            return None;
        };
        let owned = self.turn_start_hand_choice(record)?.to_owned();
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    #[cold]
    #[inline(never)]
    fn encode_before_hand_draw_power(
        record: &BeforeHandDrawPowerRecord,
        destination: Option<&mut Vec<PendingWord>>,
    ) -> Option<usize> {
        let listener_count = u32::try_from(record.listeners.len()).ok()?;
        let candidate_count = u32::try_from(record.candidates.len()).ok()?;
        let listener_is_supported = |power| {
            matches!(
                power,
                PowerId::CreativeAi
                    | PowerId::CallOfTheVoid
                    | PowerId::InfiniteBlades
                    | PowerId::Foregone
                    | PowerId::HelloWorld
                    | PowerId::SentryMode
                    | PowerId::SpectrumShift
            )
        };
        if !(1..=8).contains(&record.listeners.len())
            || !record.listeners.contains(&PowerId::Foregone)
            || record.listeners.iter().enumerate().any(|(index, power)| {
                !listener_is_supported(*power) || record.listeners[..index].contains(power)
            })
            || record.cursor == 0
            || record.cursor > listener_count
            || record.listeners[(record.cursor - 1) as usize] != PowerId::Foregone
            || record.amount == 0
            || !record.candidates.is_empty() && record.candidates.len() <= record.amount as usize
            || record.candidates.iter().enumerate().any(|(index, entry)| {
                entry.card.uid == LEGACY_CARD_UID
                    || record.candidates[..index]
                        .iter()
                        .any(|prior| prior.card.uid == entry.card.uid)
            })
        {
            return None;
        }
        let mut body_count = 2_usize.checked_add(record.listeners.len())?;
        for candidate in &record.candidates {
            body_count =
                body_count.checked_add(Self::encode_frozen_auto_batch_entry(candidate, None)?)?;
        }
        let encoded_len = body_count.checked_add(1)?;
        PendingWord::header(WordRecordKind::BeforeHandDrawPower, body_count)?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        words.reserve(encoded_len);
        words.push(PendingWord::header(
            WordRecordKind::BeforeHandDrawPower,
            body_count,
        )?);
        words.push(PendingWord {
            body: record.cursor,
            meta: listener_count,
        });
        words.push(PendingWord {
            body: record.amount,
            meta: candidate_count,
        });
        for power in &record.listeners {
            words.push(PendingWord {
                body: *power as u32,
                meta: 0,
            });
        }
        for candidate in &record.candidates {
            Self::encode_frozen_auto_batch_entry(candidate, Some(words))?;
        }
        Some(encoded_len)
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn push_before_hand_draw_power(
        &mut self,
        record: &BeforeHandDrawPowerRecord,
    ) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_before_hand_draw_power(record, None)?;
        let start = self.0.words.len();
        let index = WordRecordIndex::from_offset(start)?;
        if start.checked_add(encoded_len)? > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        Self::encode_before_hand_draw_power(record, Some(&mut store.words))?;
        store
            .frames
            .push(Frame::BeforeHandDrawPower { record: index });
        Some(index)
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn before_hand_draw_power(
        &self,
        index: WordRecordIndex,
    ) -> Option<BeforeHandDrawPowerView<'_>> {
        let start = index.offset()?;
        let (kind, count) = self.0.words.get(start)?.parse_header()?;
        if kind != WordRecordKind::BeforeHandDrawPower || count < 3 {
            return None;
        }
        let end = start.checked_add(1)?.checked_add(count as usize)?;
        let header = *self.0.words.get(start + 1)?;
        let scalar = *self.0.words.get(start + 2)?;
        let listener_count = usize::try_from(header.meta).ok()?;
        let candidate_count = usize::try_from(scalar.meta).ok()?;
        if !(1..=8).contains(&listener_count) || header.body == 0 || header.body > header.meta {
            return None;
        }
        let listener_words = self.0.words.get(start + 3..start + 3 + listener_count)?;
        if listener_words
            .iter()
            .any(|word| word.meta != 0 || PowerId::ALL.get(word.body as usize).is_none())
        {
            return None;
        }
        let candidate_words = self.0.words.get(start + 3 + listener_count..end)?;
        let mut remaining = candidate_words;
        let mut seen = Vec::new();
        while !remaining.is_empty() {
            let (entry, consumed) = Self::decode_frozen_auto_batch_entry(remaining)?;
            if entry.card.uid == LEGACY_CARD_UID || seen.contains(&entry.card.uid) {
                return None;
            }
            seen.push(entry.card.uid);
            remaining = remaining.get(consumed..)?;
        }
        if seen.len() != candidate_count || !seen.is_empty() && seen.len() <= scalar.body as usize {
            return None;
        }
        let view = BeforeHandDrawPowerView {
            cursor: header.body,
            amount: scalar.body,
            listener_words,
            candidate_words,
            candidate_count,
        };
        Self::encode_before_hand_draw_power(&view.to_owned(), None)?;
        Some(view)
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn pop_top_before_hand_draw_power(&mut self) -> Option<BeforeHandDrawPowerRecord> {
        let Frame::BeforeHandDrawPower { record } = self.top()? else {
            return None;
        };
        let owned = self.before_hand_draw_power(record)?.to_owned();
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    fn encode_card_play(
        record: &CardPlayRecord,
        destination: Option<&mut Vec<PendingWord>>,
        allow_transient_after_normal_lamp_owner: bool,
    ) -> Option<usize> {
        if !(1..=256).contains(&record.plays)
            || record.play_index >= record.plays
            || record.spent < 0
            || i16::try_from(record.spent).is_err()
            || record.source == CardPlaySource::Manual && !record.active_play_member
            || record.source == CardPlaySource::Manual && record.force_exhaust
            || matches!(
                record.source,
                CardPlaySource::Auto
                    | CardPlaySource::Hellraiser
                    | CardPlaySource::DrawPileFlip
                    | CardPlaySource::SlyDiscard
                    | CardPlaySource::BeatDown
                    | CardPlaySource::Catastrophe
                    | CardPlaySource::Eidolon
                    | CardPlaySource::KnifeTrap
                    | CardPlaySource::Uproar
                    | CardPlaySource::Decisions
            ) && record.choice_kind != CardPlayChoiceKind::None
            || record.source == CardPlaySource::Manual
                && record.first_choice.is_none()
                && record.target.is_some()
                && record.choice_kind != CardPlayChoiceKind::SignedInt
            || (record.choice_kind == CardPlayChoiceKind::HotCard) != record.first_choice.is_some()
            || record.choice_kind != CardPlayChoiceKind::SignedInt && record.choice_i32 != 0
            || record.expos == Some(u32::MAX)
            || record.afterimage < 0
            || record.subroutine < 0
            || record.storm < 0
            || record.serpent < 0
            || record.rupture_batch < 0
            || !record.rupture_registered && record.rupture_batch != 0
            || record.expos.is_some()
                && (record.choice_kind != CardPlayChoiceKind::HotCard
                    || record.source != CardPlaySource::Manual
                    || record.source_pile != PileId::Play)
            || record.pending_choice != record.selection_kind.is_some()
            || matches!(record.stage, CardPlayStage::StartBody)
                && (record.latch_kind != CardPlayLatchKind::Empty || record.next_step != 0)
            || matches!(record.stage, CardPlayStage::Body)
                && record.latch_kind != CardPlayLatchKind::Raw
            || matches!(
                record.stage,
                CardPlayStage::AfterBody | CardPlayStage::AfterEnchantment
            ) && record.latch_kind != CardPlayLatchKind::Raw
            || matches!(
                record.stage,
                CardPlayStage::AfterImitation | CardPlayStage::AfterNormalHook
            ) && record.latch_kind != CardPlayLatchKind::Post
            || record.lamp_owner
                && matches!(
                    record.stage,
                    CardPlayStage::StartBody | CardPlayStage::AfterNormalHook
                )
                && !(allow_transient_after_normal_lamp_owner
                    && record.stage == CardPlayStage::AfterNormalHook)
            || record.stage != CardPlayStage::AfterImitation
                && !record.physical_after_card_played_listeners.is_empty()
            || record.stage != CardPlayStage::AfterImitation
                && record.after_card_played_power_snapshot_captured
            || matches!(
                record.stage,
                CardPlayStage::StartBody | CardPlayStage::AfterNormalHook
            ) && (record.instanced_power_uid_at_before.is_some()
                || record.instanced_power_created_uid.is_some())
            || record.instanced_power_created_uid.is_some_and(|created| {
                record
                    .instanced_power_uid_at_before
                    .is_none_or(|watermark| created < watermark)
            })
            || usize::try_from(record.after_card_played_power_cursor).map_or(true, |cursor| {
                cursor > record.after_card_played_power_snapshot.len()
            })
            || record.after_card_played_power_cursor >= (1 << 29)
            || record.after_card_played_power_phase > 3
            || !record.after_card_played_power_snapshot_captured
                && (record.after_card_played_power_cursor != 0
                    || record.after_card_played_power_phase != 0
                    || !record.after_card_played_power_snapshot.is_empty()
                    || record.after_card_played_pending_monologue_uid.is_some())
            || record.after_card_played_power_snapshot_captured
                && matches!(record.after_card_played_power_phase, 0 | 1)
                && (record.after_card_played_power_cursor != 0
                    || record.after_card_played_pending_monologue_uid.is_some())
            || record.after_card_played_power_snapshot_captured
                && record.after_card_played_power_phase == 3
                && (record.after_card_played_pending_monologue_uid.is_some()
                    || !record.monologue_latches.is_empty()
                    || record.monologue)
            || record
                .after_card_played_pending_monologue_uid
                .is_some_and(|uid| {
                    record.after_card_played_power_phase != 2
                        || record.after_card_played_power_cursor == 0
                        || usize::try_from(record.after_card_played_power_cursor - 1)
                            .ok()
                            .and_then(|cursor| record.after_card_played_power_snapshot.get(cursor))
                            .copied()
                            != Some(uid)
                        || record
                            .monologue_latches
                            .iter()
                            .any(|(latched_uid, _)| *latched_uid == uid)
                })
            || record.monologue != !record.monologue_latches.is_empty()
            || record
                .monologue_latches
                .iter()
                .enumerate()
                .any(|(index, (uid, power))| {
                    *power != 1
                        || index > 0 && record.monologue_latches[index - 1].0 >= *uid
                        || record.after_card_played_power_snapshot_captured
                            && record
                                .after_card_played_power_snapshot
                                .binary_search(uid)
                                .ok()
                                .is_none_or(|position| {
                                    record.after_card_played_power_phase == 2
                                        && position
                                            < usize::try_from(record.after_card_played_power_cursor)
                                                .unwrap_or(usize::MAX)
                                })
                })
            || record
                .after_card_played_power_snapshot
                .iter()
                .enumerate()
                .any(|(index, uid)| {
                    index > 0 && record.after_card_played_power_snapshot[index - 1] >= *uid
                })
            || record
                .physical_after_card_played_listeners
                .iter()
                .enumerate()
                .any(|(index, listener)| {
                    !matches!(listener.id, CardId::BansheesCry | CardId::Pinpoint)
                        || record.physical_after_card_played_listeners[..index]
                            .iter()
                            .any(|prior| prior.uid == listener.uid)
                })
        {
            return None;
        }
        let optional_words = usize::from(!record.selection_cards.is_empty())
            .checked_add(record.selection_cards.len())?
            .checked_add(usize::from(!record.strangle_latches.is_empty()))?
            .checked_add(record.strangle_latches.len())?
            .checked_add(usize::from(!record.oblivion_latches.is_empty()))?
            .checked_add(record.oblivion_latches.len())?
            .checked_add(usize::from(!record.monologue_latches.is_empty()))?
            .checked_add(record.monologue_latches.len())?
            .checked_add(2_usize * usize::from(record.instanced_power_uid_at_before.is_some()))?
            .checked_add(usize::from(
                record.after_card_played_power_snapshot_captured,
            ))?
            .checked_add(usize::from(
                record.after_card_played_power_snapshot_captured,
            ))?
            .checked_add(record.after_card_played_power_snapshot.len())?
            .checked_add(usize::from(
                !record.physical_after_card_played_listeners.is_empty(),
            ))?
            .checked_add(record.physical_after_card_played_listeners.len())?;
        let outer_body = 13_usize.checked_add(optional_words)?;
        let encoded_len = 1_usize.checked_add(outer_body)?;
        PendingWord::header(WordRecordKind::CardPlay, outer_body)?;
        let Some(words) = destination else {
            return Some(encoded_len);
        };
        words.reserve(encoded_len);
        words.push(
            PendingWord::header(WordRecordKind::CardPlay, outer_body)
                .expect("preflighted CardPlay header"),
        );
        let target_shape = match record.target {
            None => 0_u32,
            Some((_, _, None)) => 1,
            Some((_, _, Some(_))) => 2,
        };
        let has_optional = !record.selection_cards.is_empty()
            || !record.oblivion_latches.is_empty()
            || !record.strangle_latches.is_empty()
            || !record.monologue_latches.is_empty()
            || record.instanced_power_uid_at_before.is_some()
            || record.after_card_played_power_snapshot_captured
            || !record.physical_after_card_played_listeners.is_empty();
        let control = u32::from(record.stage as u8)
            | (u32::from(record.source as u8) << 3)
            | (u32::from(record.result_route as u8) << 8)
            | (u32::from(record.choice_kind as u8) << 12)
            | (target_shape << 14)
            | (u32::from(record.latch_kind as u8) << 16)
            | (u32::from(record.pen_double) << 18)
            | (u32::from(record.force_exhaust) << 19)
            | (u32::from(record.is_power_auto) << 20)
            | (u32::from(record.active_play_member) << 21)
            | (u32::from(record.pending_choice) << 22)
            | (u32::from(record.lamp_owner) << 23);
        words.push(PendingWord {
            body: record.uid,
            meta: control,
        });
        words.push(PendingWord {
            body: record.plays,
            meta: record.play_index,
        });
        words.push(PendingWord {
            body: record.x_value as u64 as u32,
            meta: ((record.x_value as u64) >> 32) as u32,
        });
        words.push(PendingWord {
            body: record.spent as u32,
            meta: record.star_spent,
        });
        words.push(PendingWord {
            body: record.glam,
            meta: record.next_step,
        });
        let (target_slot, target_uid, target_fallback) =
            record.target.map_or((0, 0, 0), |(slot, uid, fallback)| {
                (slot, uid, fallback.unwrap_or(0))
            });
        words.push(PendingWord {
            body: target_slot as u32,
            meta: target_uid,
        });
        words.push(PendingWord {
            body: target_fallback as u32,
            meta: record.choice_i32 as u32,
        });
        words.push(
            record
                .first_choice
                .map_or(PendingWord::default(), PendingWord::from_card),
        );
        words.push(PendingWord {
            body: record.afterimage as u32,
            meta: record.subroutine as u32,
        });
        words.push(PendingWord {
            body: record.storm as u32,
            meta: record.serpent as u32,
        });
        let latch_flags = u32::from(record.panache)
            | (u32::from(record.rupture_registered) << 1)
            | (u32::from(record.calamity) << 2)
            | (u32::from(record.monologue) << 3)
            | (u32::from(record.tender) << 4);
        words.push(PendingWord {
            body: record.rupture_batch as u32,
            meta: latch_flags,
        });
        let selection_ordinal = record.selection_kind.map_or(255, |kind| kind as u8);
        let selection_control = u32::from(selection_ordinal)
            | (u32::from(record.source_pile as u8) << 8)
            | (u32::from(has_optional) << 11);
        words.push(PendingWord {
            body: record.selection_amount as u32,
            meta: selection_control,
        });
        words.push(PendingWord {
            body: record.expos.unwrap_or(u32::MAX),
            meta: 0,
        });
        if !record.selection_cards.is_empty() {
            words.push(
                PendingWord::header(WordRecordKind::SelectionCards, record.selection_cards.len())
                    .expect("preflighted SelectionCards header"),
            );
            words.extend(
                record
                    .selection_cards
                    .iter()
                    .copied()
                    .map(PendingWord::from_card),
            );
        }
        if !record.strangle_latches.is_empty() {
            words.push(
                PendingWord::header(
                    WordRecordKind::StrangleLatches,
                    record.strangle_latches.len(),
                )
                .expect("preflighted StrangleLatches header"),
            );
            words.extend(
                record
                    .strangle_latches
                    .iter()
                    .copied()
                    .map(|(uid, amount)| PendingWord::from_strangle(uid, amount)),
            );
        }
        if !record.oblivion_latches.is_empty() {
            words.push(
                PendingWord::header(
                    WordRecordKind::OblivionLatches,
                    record.oblivion_latches.len(),
                )
                .expect("preflighted OblivionLatches header"),
            );
            words.extend(
                record
                    .oblivion_latches
                    .iter()
                    .copied()
                    .map(|(uid, amount)| PendingWord::from_strangle(uid, amount)),
            );
        }
        if !record.monologue_latches.is_empty() {
            words.push(
                PendingWord::header(
                    WordRecordKind::MonologueLatches,
                    record.monologue_latches.len(),
                )
                .expect("preflighted MonologueLatches header"),
            );
            words.extend(
                record
                    .monologue_latches
                    .iter()
                    .copied()
                    .map(|(uid, power)| PendingWord::from_strangle(uid, power)),
            );
        }
        if let Some(watermark) = record.instanced_power_uid_at_before {
            words.push(
                PendingWord::header(WordRecordKind::InstancedPowerCardReceipt, 1)
                    .expect("preflighted InstancedPowerCardReceipt header"),
            );
            words.push(PendingWord {
                body: watermark,
                meta: record.instanced_power_created_uid.unwrap_or(u32::MAX),
            });
        }
        if record.after_card_played_power_snapshot_captured {
            words.push(
                PendingWord::header(
                    WordRecordKind::AfterCardPlayedPowerSnapshot,
                    record.after_card_played_power_snapshot.len() + 1,
                )
                .expect("preflighted AfterCardPlayedPowerSnapshot header"),
            );
            words.push(PendingWord {
                body: record.after_card_played_power_cursor
                    | (u32::from(record.after_card_played_power_phase) << 29),
                meta: record
                    .after_card_played_pending_monologue_uid
                    .unwrap_or(u32::MAX),
            });
            words.extend(
                record
                    .after_card_played_power_snapshot
                    .iter()
                    .map(|uid| PendingWord {
                        body: *uid,
                        meta: 0,
                    }),
            );
        }
        if !record.physical_after_card_played_listeners.is_empty() {
            words.push(
                PendingWord::header(
                    WordRecordKind::PhysicalAfterCardPlayedListeners,
                    record.physical_after_card_played_listeners.len(),
                )
                .expect("preflighted PhysicalAfterCardPlayedListeners header"),
            );
            words.extend(
                record
                    .physical_after_card_played_listeners
                    .iter()
                    .copied()
                    .map(PendingWord::from_physical_listener),
            );
        }
        Some(encoded_len)
    }

    /// Append one complete record-bearing CardPlay atomically.
    pub(crate) fn push_card_play(&mut self, record: &CardPlayRecord) -> Option<WordRecordIndex> {
        let encoded_len = Self::encode_card_play(record, None, false)?;
        let start = self.0.words.len();
        let index = WordRecordIndex::from_offset(start)?;
        let final_len = start.checked_add(encoded_len)?;
        if final_len > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.reserve(encoded_len);
        store.frames.reserve(1);
        let before_words = store.words.len();
        let appended = Self::encode_card_play(record, Some(&mut store.words), false)
            .expect("preflighted CardPlay encoding");
        debug_assert_eq!(appended, encoded_len);
        debug_assert_eq!(store.words.len() - before_words, encoded_len);
        store.frames.push(Frame::CardPlay { record: index });
        Some(index)
    }

    pub(crate) fn card_play(&self, index: WordRecordIndex) -> Option<CardPlayView<'_>> {
        self.card_play_inner(index, false)
    }

    fn card_play_inner(
        &self,
        index: WordRecordIndex,
        allow_transient_after_normal_lamp_owner: bool,
    ) -> Option<CardPlayView<'_>> {
        let start = index.offset()?;
        let words = &self.0.words;
        let (kind, outer_count) = words.get(start)?.parse_header()?;
        if kind != WordRecordKind::CardPlay || outer_count < 13 {
            return None;
        }
        let end = start.checked_add(1)?.checked_add(outer_count as usize)?;
        let fixed = words.get(start + 1..start + 14)?;
        let control = fixed[0].meta;
        if control >> 24 != 0 || fixed[12].meta != 0 {
            return None;
        }
        let stage = CardPlayStage::from_ordinal((control & 0x07) as u8)?;
        let source = CardPlaySource::from_ordinal(((control >> 3) & 0x1f) as u8)?;
        let result_route = CardResultRoute::from_ordinal(((control >> 8) & 0x0f) as u8)?;
        let choice_kind = CardPlayChoiceKind::from_ordinal(((control >> 12) & 0x03) as u8)?;
        let target_shape = ((control >> 14) & 0x03) as u8;
        if target_shape > 2 {
            return None;
        }
        let latch_kind = CardPlayLatchKind::from_ordinal(((control >> 16) & 0x03) as u8)?;
        let plays = fixed[1].body;
        let play_index = fixed[1].meta;
        let spent = fixed[3].body as i32;
        if !(1..=256).contains(&plays)
            || play_index >= plays
            || spent < 0
            || i16::try_from(spent).is_err()
        {
            return None;
        }
        let x_value = (u64::from(fixed[2].meta) << 32 | u64::from(fixed[2].body)) as i64;
        let choice_i32 = fixed[6].meta as i32;
        let first_choice = (choice_kind == CardPlayChoiceKind::HotCard).then_some(HotCard {
            uid: fixed[7].body,
            atom: fixed[7].meta as u16,
            flags: (fixed[7].meta >> 16) as u16,
        });
        if (choice_kind != CardPlayChoiceKind::HotCard && fixed[7] != PendingWord::default())
            || (choice_kind == CardPlayChoiceKind::HotCard && fixed[7] == PendingWord::default())
            || (choice_kind != CardPlayChoiceKind::SignedInt && choice_i32 != 0)
        {
            return None;
        }
        let target = match target_shape {
            0 if fixed[5] == PendingWord::default() && fixed[6].body == 0 => None,
            1 if fixed[6].body == 0 => Some((fixed[5].body as i32, fixed[5].meta, None)),
            2 => Some((
                fixed[5].body as i32,
                fixed[5].meta,
                Some(fixed[6].body as i32),
            )),
            _ => return None,
        };
        let latch_flags = fixed[10].meta;
        if latch_flags >> 5 != 0
            || (fixed[8].body as i32) < 0
            || (fixed[8].meta as i32) < 0
            || (fixed[9].body as i32) < 0
            || (fixed[9].meta as i32) < 0
            || (fixed[10].body as i32) < 0
            || latch_flags & 2 == 0 && fixed[10].body != 0
        {
            return None;
        }
        let selection_control = fixed[11].meta;
        if selection_control >> 12 != 0 {
            return None;
        }
        let selection_kind = match selection_control as u8 {
            255 => None,
            ordinal => Some(
                PendingSelectionKind::ALL
                    .get(usize::from(ordinal))
                    .copied()?,
            ),
        };
        let source_pile = PileId::ALL
            .get(((selection_control >> 8) & 0x07) as usize)
            .copied()?;
        let has_optional = selection_control & (1 << 11) != 0;
        let mut cursor = start + 14;
        let mut selection_words = &words[0..0];
        let mut strangle_words = &words[0..0];
        let mut oblivion_words = &words[0..0];
        let mut monologue_words = &words[0..0];
        let mut instanced_power_uid_at_before = None;
        let mut instanced_power_created_uid = None;
        let mut after_card_played_power_words = &words[0..0];
        let mut after_card_played_power_cursor = 0_u32;
        let mut after_card_played_power_phase = 0_u8;
        let mut after_card_played_power_snapshot_captured = false;
        let mut after_card_played_pending_monologue_uid = None;
        let mut physical_listener_words = &words[0..0];
        if cursor < end {
            let (subkind, count) = words.get(cursor)?.parse_header()?;
            if subkind == WordRecordKind::SelectionCards {
                if count == 0 {
                    return None;
                }
                let body_start = cursor.checked_add(1)?;
                let body_end = body_start.checked_add(count as usize)?;
                selection_words = words.get(body_start..body_end)?;
                cursor = body_end;
            }
        }
        if cursor < end {
            let (subkind, count) = words.get(cursor)?.parse_header()?;
            if subkind == WordRecordKind::StrangleLatches {
                if count == 0 {
                    return None;
                }
                let body_start = cursor.checked_add(1)?;
                let body_end = body_start.checked_add(count as usize)?;
                strangle_words = words.get(body_start..body_end)?;
                cursor = body_end;
            }
        }
        if cursor < end {
            let (subkind, count) = words.get(cursor)?.parse_header()?;
            if subkind == WordRecordKind::OblivionLatches {
                if count == 0 {
                    return None;
                }
                let body_start = cursor.checked_add(1)?;
                let body_end = body_start.checked_add(count as usize)?;
                oblivion_words = words.get(body_start..body_end)?;
                cursor = body_end;
            }
        }
        if cursor < end {
            let (subkind, count) = words.get(cursor)?.parse_header()?;
            if subkind == WordRecordKind::MonologueLatches {
                if count == 0 {
                    return None;
                }
                let body_start = cursor.checked_add(1)?;
                let body_end = body_start.checked_add(count as usize)?;
                monologue_words = words.get(body_start..body_end)?;
                if monologue_words.iter().enumerate().any(|(index, word)| {
                    word.meta != 1 || index > 0 && monologue_words[index - 1].body >= word.body
                }) {
                    return None;
                }
                cursor = body_end;
            }
        }
        if cursor < end {
            let (subkind, count) = words.get(cursor)?.parse_header()?;
            if subkind == WordRecordKind::InstancedPowerCardReceipt {
                if count != 1 {
                    return None;
                }
                let receipt = *words.get(cursor.checked_add(1)?)?;
                instanced_power_uid_at_before = Some(receipt.body);
                instanced_power_created_uid = (receipt.meta != u32::MAX).then_some(receipt.meta);
                if instanced_power_created_uid.is_some_and(|uid| uid < receipt.body) {
                    return None;
                }
                cursor = cursor.checked_add(2)?;
            }
        }
        if cursor < end {
            let (subkind, count) = words.get(cursor)?.parse_header()?;
            if subkind == WordRecordKind::AfterCardPlayedPowerSnapshot {
                if count < 1 {
                    return None;
                }
                after_card_played_power_snapshot_captured = true;
                let body_start = cursor.checked_add(1)?;
                let body_end = body_start.checked_add(count as usize)?;
                let body = words.get(body_start..body_end)?;
                let control = *body.first()?;
                after_card_played_power_cursor = control.body & ((1 << 29) - 1);
                after_card_played_power_phase = (control.body >> 29) as u8;
                after_card_played_pending_monologue_uid =
                    (control.meta != u32::MAX).then_some(control.meta);
                after_card_played_power_words = &body[1..];
                if after_card_played_power_words
                    .iter()
                    .enumerate()
                    .any(|(index, word)| {
                        word.meta != 0
                            || index > 0
                                && after_card_played_power_words[index - 1].body >= word.body
                    })
                {
                    return None;
                }
                let snapshot_cursor = usize::try_from(after_card_played_power_cursor).ok()?;
                if after_card_played_power_phase > 3
                    || snapshot_cursor > after_card_played_power_words.len()
                    || after_card_played_pending_monologue_uid.is_some_and(|uid| {
                        after_card_played_power_phase != 2
                            || snapshot_cursor == 0
                            || after_card_played_power_words[snapshot_cursor - 1].body != uid
                            || monologue_words.iter().any(|word| word.body == uid)
                    })
                {
                    return None;
                }
                cursor = body_end;
            }
        }
        if cursor < end {
            let (subkind, count) = words.get(cursor)?.parse_header()?;
            if subkind != WordRecordKind::PhysicalAfterCardPlayedListeners || count == 0 {
                return None;
            }
            let body_start = cursor.checked_add(1)?;
            let body_end = body_start.checked_add(count as usize)?;
            physical_listener_words = words.get(body_start..body_end)?;
            if physical_listener_words
                .iter()
                .enumerate()
                .any(|(index, word)| {
                    word.physical_listener().is_none()
                        || physical_listener_words[..index]
                            .iter()
                            .any(|prior| prior.body == word.body)
                })
            {
                return None;
            }
            cursor = body_end;
        }
        if cursor != end
            || has_optional
                != (!selection_words.is_empty()
                    || !oblivion_words.is_empty()
                    || !strangle_words.is_empty()
                    || !monologue_words.is_empty()
                    || instanced_power_uid_at_before.is_some()
                    || after_card_played_power_snapshot_captured
                    || !physical_listener_words.is_empty())
            || (latch_flags & 8 != 0) != !monologue_words.is_empty()
            || (control & (1 << 22) != 0) != selection_kind.is_some()
            || source == CardPlaySource::Manual && control & (1 << 21) == 0
            || source == CardPlaySource::Manual && control & (1 << 19) != 0
            || matches!(
                source,
                CardPlaySource::Auto
                    | CardPlaySource::Hellraiser
                    | CardPlaySource::DrawPileFlip
                    | CardPlaySource::SlyDiscard
                    | CardPlaySource::BeatDown
                    | CardPlaySource::Catastrophe
                    | CardPlaySource::Eidolon
                    | CardPlaySource::KnifeTrap
                    | CardPlaySource::Uproar
                    | CardPlaySource::Decisions
            ) && choice_kind != CardPlayChoiceKind::None
            || source == CardPlaySource::Manual
                && first_choice.is_none()
                && target.is_some()
                && choice_kind != CardPlayChoiceKind::SignedInt
            || matches!(stage, CardPlayStage::StartBody)
                && (latch_kind != CardPlayLatchKind::Empty || fixed[4].meta != 0)
            || matches!(stage, CardPlayStage::Body) && latch_kind != CardPlayLatchKind::Raw
            || matches!(
                stage,
                CardPlayStage::AfterBody | CardPlayStage::AfterEnchantment
            ) && latch_kind != CardPlayLatchKind::Raw
            || matches!(
                stage,
                CardPlayStage::AfterImitation | CardPlayStage::AfterNormalHook
            ) && latch_kind != CardPlayLatchKind::Post
            || control & (1 << 23) != 0
                && matches!(
                    stage,
                    CardPlayStage::StartBody | CardPlayStage::AfterNormalHook
                )
                && !(allow_transient_after_normal_lamp_owner
                    && stage == CardPlayStage::AfterNormalHook)
            || stage != CardPlayStage::AfterImitation && !physical_listener_words.is_empty()
            || stage != CardPlayStage::AfterImitation && after_card_played_power_snapshot_captured
            || matches!(
                stage,
                CardPlayStage::StartBody | CardPlayStage::AfterNormalHook
            ) && (instanced_power_uid_at_before.is_some()
                || instanced_power_created_uid.is_some())
            || after_card_played_power_snapshot_captured
                && matches!(after_card_played_power_phase, 0 | 1)
                && (after_card_played_power_cursor != 0
                    || after_card_played_pending_monologue_uid.is_some())
            || after_card_played_power_snapshot_captured
                && after_card_played_power_phase == 3
                && (after_card_played_pending_monologue_uid.is_some()
                    || !monologue_words.is_empty()
                    || latch_flags & 8 != 0)
            || monologue_words.iter().any(|latch| {
                after_card_played_power_snapshot_captured
                    && after_card_played_power_words
                        .binary_search_by_key(&latch.body, |word| word.body)
                        .ok()
                        .is_none_or(|position| {
                            after_card_played_power_phase == 2
                                && position
                                    < usize::try_from(after_card_played_power_cursor)
                                        .unwrap_or(usize::MAX)
                        })
            })
        {
            return None;
        }
        Some(CardPlayView {
            uid: fixed[0].body,
            stage,
            source,
            result_route,
            choice_kind,
            choice_i32,
            first_choice,
            target,
            latch_kind,
            pen_double: control & (1 << 18) != 0,
            force_exhaust: control & (1 << 19) != 0,
            is_power_auto: control & (1 << 20) != 0,
            active_play_member: control & (1 << 21) != 0,
            pending_choice: control & (1 << 22) != 0,
            lamp_owner: control & (1 << 23) != 0,
            plays,
            play_index,
            x_value,
            spent,
            star_spent: fixed[3].meta,
            glam: fixed[4].body,
            next_step: fixed[4].meta,
            afterimage: fixed[8].body as i32,
            subroutine: fixed[8].meta as i32,
            storm: fixed[9].body as i32,
            serpent: fixed[9].meta as i32,
            rupture_batch: fixed[10].body as i32,
            panache: latch_flags & 1 != 0,
            rupture_registered: latch_flags & 2 != 0,
            calamity: latch_flags & 4 != 0,
            monologue: latch_flags & 8 != 0,
            tender: latch_flags & 16 != 0,
            after_card_played_power_cursor,
            after_card_played_power_phase,
            after_card_played_power_snapshot_captured,
            after_card_played_pending_monologue_uid,
            instanced_power_uid_at_before,
            instanced_power_created_uid,
            selection_amount: fixed[11].body as i32,
            selection_kind,
            source_pile,
            expos: (fixed[12].body != u32::MAX).then_some(fixed[12].body),
            selection_words,
            oblivion_words,
            strangle_words,
            monologue_words,
            after_card_played_power_words,
            physical_listener_words,
        })
    }

    pub(crate) fn pending_choice_is_valid(&self, pending: &PendingSelection) -> bool {
        self.top()
            == Some(Frame::CardPlay {
                record: pending.frame_record,
            })
            && self
                .card_play(pending.frame_record)
                .is_some_and(|record| record.uid == pending.frame_uid && record.pending_choice)
            && self.continuation_store_is_valid(Some(pending))
    }

    pub(crate) fn pending_turn_start_hand_choice_is_valid(
        &self,
        pending: &PendingSelection,
    ) -> bool {
        pending.frame_uid == TURN_START_HAND_CHOICE_PENDING_UID
            && self.top()
                == Some(Frame::TurnStartHandChoice {
                    record: pending.frame_record,
                })
            && self
                .turn_start_hand_choice(pending.frame_record)
                .is_some_and(|record| record.phase == TurnStartHandChoicePhase::Selecting)
            && self.continuation_store_is_valid(Some(pending))
    }

    /// Authenticate every frame-owned record in bottom-to-top order.
    pub(crate) fn continuation_store_is_valid(&self, pending: Option<&PendingSelection>) -> bool {
        if pending.is_some_and(PendingSelection::is_relic_selection) {
            return self.0.frames.is_empty() && self.0.words.is_empty();
        }
        let mut cursor = 0_usize;
        let mut uids = std::collections::BTreeSet::new();
        let mut pending_frames = 0_usize;
        let mut turn_start_pending_frames = 0_usize;
        let mut enemy_pending_frames = 0_usize;
        let mut potion_pending_frames = 0_usize;
        let mut generation_potion_pending_frames = 0_usize;
        let mut stratagem_pending_frames = 0_usize;
        let mut foregone_pending_frames = 0_usize;
        let mut replay_frames = 0_usize;
        for (position, frame) in self.0.frames.iter().enumerate() {
            let record = match *frame {
                Frame::ActionReplay { record }
                | Frame::CardPlay { record }
                | Frame::Phase { record }
                | Frame::LiveListener { record }
                | Frame::FrozenAutoBatch { record }
                | Frame::TurnStartHandChoice { record }
                | Frame::EnemyPhase { record }
                | Frame::PotionFinish { record }
                | Frame::Draw { record }
                | Frame::AfterCardDrawnPower { record }
                | Frame::AfterCardExhaustedPower { record }
                | Frame::AfterPowerAmountChanged { record }
                | Frame::AfterSideTurnEndPower { record }
                | Frame::CardFinish { record }
                | Frame::BeforeHandDrawPower { record } => record,
                _ => continue,
            };
            if record.offset() != Some(cursor) {
                return false;
            }
            match *frame {
                Frame::ActionReplay { .. } => {
                    replay_frames += 1;
                    if position != 0 || self.action_replay_action(record).is_none() {
                        return false;
                    }
                }
                Frame::CardPlay { .. } => {
                    let Some(view) = self.card_play(record) else {
                        return false;
                    };
                    if !uids.insert(view.uid) {
                        return false;
                    }
                    pending_frames += usize::from(view.pending_choice);
                }
                Frame::Phase { .. } => {
                    if self.auto_post_phase(record).is_none()
                        && self.auto_pre_mayhem_phase(record).is_none()
                        && self.auto_pre_bombardment_phase(record).is_none()
                        && self.auto_pre_history_course_phase(record).is_none()
                    {
                        return false;
                    }
                }
                Frame::LiveListener { .. } => {
                    if self.stampede_live(record).is_none()
                        && self.catastrophe_live(record).is_none()
                    {
                        return false;
                    }
                }
                Frame::FrozenAutoBatch { .. } => {
                    if self.frozen_auto_batch(record).is_none() {
                        return false;
                    }
                }
                Frame::TurnStartHandChoice { .. } => {
                    let Some(view) = self.turn_start_hand_choice(record) else {
                        return false;
                    };
                    turn_start_pending_frames +=
                        usize::from(view.phase == TurnStartHandChoicePhase::Selecting);
                    if view.phase == TurnStartHandChoicePhase::Selecting
                        && position + 1 != self.0.frames.len()
                    {
                        return false;
                    }
                }
                Frame::BeforeHandDrawPower { .. } => {
                    let Some(view) = self.before_hand_draw_power(record) else {
                        return false;
                    };
                    let pre_shuffle_child = view.candidates().len() == 0
                        && self
                            .0
                            .frames
                            .get(position + 1)
                            .is_some_and(|frame| match *frame {
                                Frame::Draw { record } => self.draw(record).is_some_and(|draw| {
                                    draw.caller == DrawCaller::ForegoneBeforeHandDraw
                                        && draw.stage == DrawStage::AfterShuffle
                                }),
                                _ => false,
                            });
                    if !(pre_shuffle_child || position + 1 == self.0.frames.len()) {
                        return false;
                    }
                    foregone_pending_frames += usize::from(view.candidates().len() != 0);
                }
                Frame::EnemyPhase { .. } => {
                    let Some(_) = self.enemy_phase(record) else {
                        return false;
                    };
                    enemy_pending_frames += 1;
                    if position + 1 != self.0.frames.len() {
                        return false;
                    }
                }
                Frame::PotionFinish { .. } => {
                    let Some(view) = self.potion_finish(record) else {
                        return false;
                    };
                    potion_pending_frames +=
                        usize::from(view.body_stage == PotionBodyStage::Selecting);
                    generation_potion_pending_frames +=
                        usize::from(view.body_stage == PotionBodyStage::GenerationSelecting);
                    stratagem_pending_frames +=
                        usize::from(view.body_stage == PotionBodyStage::AfterShuffle);
                    if matches!(
                        view.body_stage,
                        PotionBodyStage::Selecting
                            | PotionBodyStage::AfterShuffle
                            | PotionBodyStage::GenerationSelecting
                    ) && position + 1 != self.0.frames.len()
                    {
                        return false;
                    }
                }
                Frame::Draw { .. } => {
                    let Some(view) = self.draw(record) else {
                        return false;
                    };
                    stratagem_pending_frames += usize::from(view.stage == DrawStage::AfterShuffle);
                    if view.stage == DrawStage::AfterShuffle && position + 1 != self.0.frames.len()
                    {
                        return false;
                    }
                }
                Frame::AfterCardDrawnPower { .. } => {
                    if self.after_card_drawn_power(record).is_none() {
                        return false;
                    }
                }
                Frame::AfterCardExhaustedPower { .. } => {
                    if self.after_card_exhausted_power(record).is_none() {
                        return false;
                    }
                }
                Frame::AfterPowerAmountChanged { .. } => {
                    if self.after_power_amount_changed(record).is_none() {
                        return false;
                    }
                }
                Frame::AfterSideTurnEndPower { .. } => {
                    if self.after_side_turn_end_power(record).is_none() {
                        return false;
                    }
                }
                Frame::CardFinish { .. } => {
                    if self.card_finish(record).is_none() {
                        return false;
                    }
                }
                _ => unreachable!("record-bearing frame matched above"),
            }
            let Some((_, count)) = self.0.words[cursor].parse_header() else {
                return false;
            };
            let Some(next) = cursor.checked_add(1 + count as usize) else {
                return false;
            };
            cursor = next;
        }
        if cursor != self.0.words.len() {
            return false;
        }
        if replay_frames > 1 {
            return false;
        }
        let rootless_tyranny_stratagem = replay_frames == 0
            && matches!(
                self.0.frames.as_slice(),
                [
                    Frame::TurnStartHandChoice {
                        record: parent_record,
                    },
                    Frame::AfterCardExhaustedPower {
                        record: owner_record,
                    },
                    Frame::Draw { record: draw_record },
                ] if *draw_record == pending.map_or(WordRecordIndex(u32::MAX), |p| p.frame_record)
                    && self.turn_start_hand_choice(*parent_record).is_some_and(|parent| {
                        parent.kind == TurnStartHandChoiceKind::Tyranny
                            && parent.phase == TurnStartHandChoicePhase::Effect
                    })
                    && self.after_card_exhausted_power(*owner_record).is_some_and(|owner| {
                        owner.return_kind == AfterCardExhaustedReturnKind::ReturnOnly
                            && owner.source_uid.is_none()
                            && owner.step_index == 0
                    })
                    && self.draw(*draw_record).is_some_and(|draw| {
                        draw.caller == DrawCaller::AfterCardExhaustedPower
                            && draw.stage == DrawStage::AfterShuffle
                    })
            );
        let rootless_foregone_stratagem = replay_frames == 0
            && matches!(
                self.0.frames.as_slice(),
                [
                    Frame::BeforeHandDrawPower { record: parent_record },
                    Frame::Draw { record: draw_record },
                ] if *draw_record == pending.map_or(WordRecordIndex(u32::MAX), |p| p.frame_record)
                    && self.before_hand_draw_power(*parent_record).is_some_and(|parent| parent.candidates().len() == 0)
                    && self.draw(*draw_record).is_some_and(|draw| draw.caller == DrawCaller::ForegoneBeforeHandDraw && draw.stage == DrawStage::AfterShuffle)
            );
        // A Centennial Puzzle Draw parks on Stratagem at an instant with no
        // ActionReplay root only while `engine::puzzle::receipt_owned_draw`
        // re-executes the root's own receipt (#3114): the root is installed
        // when that transaction publishes. The grammar and admission still
        // require the root or the inline proof; this is only the store shape.
        let rootless_puzzle_stratagem = replay_frames == 0
            && self.0.frames.iter().any(|frame| match *frame {
                Frame::Draw { record } => self
                    .draw(record)
                    .is_some_and(|draw| crate::engine::puzzle::is_receipt_owned(draw.caller)),
                // The History Course dupe's awaited AutoPlay is receipt-owned
                // the same way (#3309).
                Frame::Phase { record } => self.auto_pre_history_course_phase(record).is_some(),
                _ => false,
            });
        // Gremlin Horn's Draw parks on Stratagem inside its AfterDeath listener
        // with no root yet: that choice is detached into the deferred hook
        // action and re-attached on the transaction's root (#3387,
        // `engine::hook_action`). The grammar and admission still require the
        // root; this is only the store shape.
        let rootless_gremlin_horn_stratagem = replay_frames == 0
            && self.0.frames.iter().any(|frame| match *frame {
                Frame::Draw { record } => self
                    .draw(record)
                    .is_some_and(|draw| draw.caller == DrawCaller::GremlinHorn),
                _ => false,
            });
        match pending {
            Some(pending) => {
                (pending_frames == 1
                    && turn_start_pending_frames == 0
                    && enemy_pending_frames == 0
                    && potion_pending_frames == 0
                    && generation_potion_pending_frames == 0
                    && stratagem_pending_frames == 0
                    && foregone_pending_frames == 0
                    && self.top()
                        == Some(Frame::CardPlay {
                            record: pending.frame_record,
                        })
                    && self
                        .card_play(pending.frame_record)
                        .is_some_and(|view| view.uid == pending.frame_uid && view.pending_choice))
                    || (pending_frames == 0
                        && turn_start_pending_frames == 1
                        && enemy_pending_frames == 0
                        && potion_pending_frames == 0
                        && generation_potion_pending_frames == 0
                        && stratagem_pending_frames == 0
                        && foregone_pending_frames == 0
                        && pending.frame_uid == TURN_START_HAND_CHOICE_PENDING_UID
                        && self.top()
                            == Some(Frame::TurnStartHandChoice {
                                record: pending.frame_record,
                            }))
                    || (pending_frames == 0
                        && turn_start_pending_frames == 0
                        && enemy_pending_frames == 1
                        && potion_pending_frames == 0
                        && generation_potion_pending_frames == 0
                        && stratagem_pending_frames == 0
                        && foregone_pending_frames == 0
                        && self.top()
                            == Some(Frame::EnemyPhase {
                                record: pending.frame_record,
                            })
                        && self
                            .enemy_phase(pending.frame_record)
                            .is_some_and(|record| {
                                record.actor_uid == pending.frame_uid && record.stage == 0
                            }))
                    || (pending_frames == 0
                        && turn_start_pending_frames == 0
                        && enemy_pending_frames == 0
                        && potion_pending_frames == 1
                        && generation_potion_pending_frames == 0
                        && stratagem_pending_frames == 0
                        && foregone_pending_frames == 0
                        && replay_frames == 1
                        && self.0.frames.first().is_some_and(|frame| {
                            let Frame::ActionReplay { record } = *frame else {
                                return false;
                            };
                            matches!(
                                self.action_replay_action(record),
                                Some(ActionReplayRootAction::UsePotion { target: None, .. })
                            )
                        })
                        && pending.frame_uid == POTION_SELECTION_PENDING_UID
                        && self.top()
                            == Some(Frame::PotionFinish {
                                record: pending.frame_record,
                            })
                        && self
                            .potion_finish(pending.frame_record)
                            .is_some_and(|record| {
                                record.stage == crate::frame::PotionFinishStage::Effect
                                    && record.body_stage == PotionBodyStage::Selecting
                            }))
                    || (pending_frames == 0
                        && turn_start_pending_frames == 0
                        && enemy_pending_frames == 0
                        && potion_pending_frames == 0
                        && generation_potion_pending_frames == 1
                        && stratagem_pending_frames == 0
                        && foregone_pending_frames == 0
                        && replay_frames == 1
                        && self.0.frames.first().is_some_and(|frame| {
                            let Frame::ActionReplay { record } = *frame else {
                                return false;
                            };
                            matches!(
                                self.action_replay_action(record),
                                Some(ActionReplayRootAction::UsePotion { target: None, .. })
                            )
                        })
                        && pending.frame_uid == GENERATION_POTION_PENDING_UID
                        && self.top()
                            == Some(Frame::PotionFinish {
                                record: pending.frame_record,
                            })
                        && self
                            .potion_finish(pending.frame_record)
                            .is_some_and(|record| {
                                record.stage == crate::frame::PotionFinishStage::Effect
                                    && record.body_stage == PotionBodyStage::GenerationSelecting
                            }))
                    || (pending_frames == 0
                        && turn_start_pending_frames == 0
                        && enemy_pending_frames == 0
                        && potion_pending_frames == 0
                        && generation_potion_pending_frames == 0
                        && stratagem_pending_frames == 1
                        && foregone_pending_frames == 0
                        && (replay_frames == 1
                            || rootless_tyranny_stratagem
                            || rootless_foregone_stratagem
                            || rootless_puzzle_stratagem
                            || rootless_gremlin_horn_stratagem)
                        && pending.frame_uid == STRATAGEM_SELECTION_PENDING_UID
                        && match self.top() {
                            Some(Frame::PotionFinish { record })
                                if record == pending.frame_record =>
                            {
                                self.potion_finish(record).is_some_and(|view| {
                                    view.name == PotionId::BottledPotential
                                        && view.stage == crate::frame::PotionFinishStage::Effect
                                        && view.body_stage == PotionBodyStage::AfterShuffle
                                })
                            }
                            Some(Frame::Draw { record }) if record == pending.frame_record => self
                                .draw(record)
                                .is_some_and(|view| view.stage == DrawStage::AfterShuffle),
                            _ => false,
                        })
                    || (pending_frames == 0
                        && turn_start_pending_frames == 0
                        && enemy_pending_frames == 0
                        && potion_pending_frames == 0
                        && generation_potion_pending_frames == 0
                        && stratagem_pending_frames == 0
                        && foregone_pending_frames == 1
                        && pending.frame_uid == FOREGONE_SELECTION_PENDING_UID
                        && self.top()
                            == Some(Frame::BeforeHandDrawPower {
                                record: pending.frame_record,
                            })
                        && self.before_hand_draw_power(pending.frame_record).is_some())
            }
            None => {
                pending_frames == 0
                    && turn_start_pending_frames == 0
                    && enemy_pending_frames == 0
                    && potion_pending_frames == 0
                    && generation_potion_pending_frames == 0
                    && stratagem_pending_frames == 0
                    && foregone_pending_frames == 0
            }
        }
    }

    pub(crate) fn replace_top_card_play(&mut self, record: &CardPlayRecord) -> Option<()> {
        self.replace_top_card_play_inner(record, false, false)
    }

    /// Replace one same-width CardPlay record inside a validation-only clone.
    /// Execution mutators remain top-only; this cold helper exists solely so
    /// rooted APC authentication can validate stale generated-card targets
    /// against an inert witness without publishing or executing the clone.
    pub(crate) fn replace_card_play_validation_witness(
        &mut self,
        index: WordRecordIndex,
        record: &CardPlayRecord,
    ) -> Option<()> {
        self.card_play(index)?;
        let mut encoded = Vec::new();
        let encoded_len = Self::encode_card_play(record, Some(&mut encoded), false)?;
        let start = index.offset()?;
        let old_len = self.0.words.get(start)?.parse_header()?.1 as usize + 1;
        if encoded_len != old_len || encoded.len() != old_len {
            return None;
        }
        Arc::make_mut(&mut self.0).words[start..start + old_len].copy_from_slice(&encoded);
        Some(())
    }

    /// Clear the unique CardPlay creation receipt for an instanced power uid.
    /// This may target a suspended outer CardPlay while a nested clone owns
    /// the top frame; clearing `created_uid` preserves record width because
    /// the allocator-watermark subrecord remains present.
    pub(crate) fn clear_instanced_power_created_uid(&mut self, uid: u32) -> Option<()> {
        let mut matched = None;
        for frame in self.0.frames.iter().copied() {
            let Frame::CardPlay { record } = frame else {
                continue;
            };
            let view = self.card_play(record)?;
            if view.instanced_power_created_uid != Some(uid) {
                continue;
            }
            if matched.is_some() {
                return None;
            }
            matched = Some((record, view.to_owned()));
        }
        let Some((index, mut record)) = matched else {
            return Some(());
        };
        record.instanced_power_created_uid = None;
        self.replace_card_play_validation_witness(index, &record)
    }

    /// Repair a persisted CardPlay owner when a later native command moves
    /// its physical card while the play remains suspended.
    ///
    /// Canonical `active_play` stores Play-pile indices; Rust derives those
    /// indices from stable UIDs and therefore updates the membership bit and
    /// physical owner pile together. Re-encoding the exact record first also
    /// refuses source kinds whose lifecycle cannot be represented after such
    /// a move.
    pub(crate) fn repair_card_play_after_move(
        &mut self,
        uid: u32,
        source: PileId,
        destination: PileId,
    ) -> Option<()> {
        if source == destination {
            return Some(());
        }
        let mut matched = None;
        for frame in self.0.frames.iter().copied() {
            let Frame::CardPlay { record } = frame else {
                continue;
            };
            let view = self.card_play(record)?;
            if view.uid != uid {
                continue;
            }
            if matched.is_some()
                || view.source_pile != source
                || view.active_play_member != (source == PileId::Play)
            {
                return None;
            }
            matched = Some((record, view.to_owned()));
        }
        let Some((index, mut record)) = matched else {
            return Some(());
        };
        record.active_play_member = destination == PileId::Play;
        record.source_pile = destination;
        let mut encoded = Vec::new();
        let encoded_len = Self::encode_card_play(&record, Some(&mut encoded), false)?;
        let start = index.offset()?;
        let old_len = self.0.words.get(start)?.parse_header()?.1 as usize + 1;
        if encoded_len != old_len || encoded.len() != old_len {
            return None;
        }
        Arc::make_mut(&mut self.0).words[start..start + old_len].copy_from_slice(&encoded);
        Some(())
    }

    /// Precommit the one transient CardPlay state immediately before its
    /// owning Unsettling Lamp latch is consumed. The stage dispatcher must
    /// clear the owner and replace this record again before returning.
    pub(crate) fn replace_top_card_play_before_lamp_consumption(
        &mut self,
        record: &CardPlayRecord,
    ) -> Option<()> {
        if record.stage != CardPlayStage::AfterNormalHook || !record.lamp_owner {
            return None;
        }
        self.replace_top_card_play_inner(record, false, true)
    }

    /// Complete the transient Lamp-consumption transition with the exact
    /// non-owning AfterNormalHook record.
    pub(crate) fn replace_top_card_play_after_lamp_consumption(
        &mut self,
        record: &CardPlayRecord,
    ) -> Option<()> {
        if record.stage != CardPlayStage::AfterNormalHook || record.lamp_owner {
            return None;
        }
        self.replace_top_card_play_inner(record, true, false)
    }

    fn replace_top_card_play_inner(
        &mut self,
        record: &CardPlayRecord,
        allow_current_transient_after_normal_lamp_owner: bool,
        allow_new_transient_after_normal_lamp_owner: bool,
    ) -> Option<()> {
        let Frame::CardPlay { record: index } = self.top()? else {
            return None;
        };
        self.card_play_inner(index, allow_current_transient_after_normal_lamp_owner)?;
        let encoded_len =
            Self::encode_card_play(record, None, allow_new_transient_after_normal_lamp_owner)?;
        let start = index.offset()?;
        let final_len = start.checked_add(encoded_len)?;
        if final_len > u32::MAX as usize {
            return None;
        }
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.words.reserve(encoded_len);
        let before_words = store.words.len();
        let appended = Self::encode_card_play(
            record,
            Some(&mut store.words),
            allow_new_transient_after_normal_lamp_owner,
        )
        .expect("preflighted CardPlay encoding");
        debug_assert_eq!(appended, encoded_len);
        debug_assert_eq!(store.words.len() - before_words, encoded_len);
        Some(())
    }

    #[cold]
    #[inline(never)]
    pub(crate) fn pop_top_card_play(&mut self) -> Option<CardPlayRecord> {
        let Frame::CardPlay { record } = self.top()? else {
            return None;
        };
        let owned = self.card_play(record)?.to_owned();
        let start = record.offset()?;
        let store = Arc::make_mut(&mut self.0);
        store.words.truncate(start);
        store.frames.pop();
        Some(owned)
    }

    #[cfg(test)]
    pub(crate) fn push_pending_for_test(&mut self, record: &CardPlayRecord) -> PendingSelection {
        let frame_record = self
            .push_card_play(record)
            .expect("valid test CardPlay record");
        PendingSelection {
            frame_uid: record.uid,
            frame_record,
        }
    }

    #[cfg(test)]
    pub(crate) fn corrupt_selection_ordinal_for_test(
        &mut self,
        pending: &PendingSelection,
        ordinal: u8,
    ) {
        let start = pending.frame_record.offset().expect("test record offset");
        let store = Arc::make_mut(&mut self.0);
        let selection = store.words.get_mut(start + 12).expect("CardPlay W12");
        selection.meta = (selection.meta & !0xff) | u32::from(ordinal);
    }

    #[cfg(test)]
    pub(crate) fn corrupt_card_play_force_exhaust_for_test(&mut self, pending: &PendingSelection) {
        let start = pending.frame_record.offset().expect("test record offset");
        Arc::make_mut(&mut self.0).words[start + 1].meta |= 1 << 19;
    }

    #[cfg(test)]
    pub(crate) fn corrupt_card_play_counts_for_test(
        &mut self,
        pending: &PendingSelection,
        plays: u32,
        play_index: u32,
    ) {
        let start = pending.frame_record.offset().expect("test record offset");
        let word = &mut Arc::make_mut(&mut self.0).words[start + 2];
        word.body = plays;
        word.meta = play_index;
    }

    #[cfg(test)]
    pub(crate) fn corrupt_card_play_spent_for_test(
        &mut self,
        pending: &PendingSelection,
        spent: i32,
    ) {
        let start = pending.frame_record.offset().expect("test record offset");
        Arc::make_mut(&mut self.0).words[start + 4].body = spent as u32;
    }

    #[cfg(test)]
    pub(crate) fn shares_store_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Closed payload for one externally selected card-play continuation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PendingSelection {
    pub frame_uid: u32,
    pub(crate) frame_record: WordRecordIndex,
}

pub(crate) const TURN_START_HAND_CHOICE_PENDING_UID: u32 = u32::MAX;
pub(crate) const POTION_SELECTION_PENDING_UID: u32 = u32::MAX - 1;
pub(crate) const STRATAGEM_SELECTION_PENDING_UID: u32 = u32::MAX - 2;
pub(crate) const GENERATION_POTION_PENDING_UID: u32 = u32::MAX - 3;
pub(crate) const RELIC_SELECTION_PENDING_UID: u32 = u32::MAX - 4;
pub(crate) const FOREGONE_SELECTION_PENDING_UID: u32 = u32::MAX - 5;

impl PendingSelection {
    pub(crate) const fn relic_selection() -> Self {
        Self {
            frame_uid: RELIC_SELECTION_PENDING_UID,
            frame_record: WordRecordIndex::NONE,
        }
    }

    pub(crate) fn is_relic_selection(&self) -> bool {
        self.frame_uid == RELIC_SELECTION_PENDING_UID && self.frame_record == WordRecordIndex::NONE
    }

    pub(crate) fn record<'a>(&self, frames: &'a Frames) -> Option<CardPlayView<'a>> {
        frames
            .pending_choice_is_valid(self)
            .then(|| frames.card_play(self.frame_record))
            .flatten()
    }

    pub(crate) fn turn_start_hand_choice_record<'a>(
        &self,
        frames: &'a Frames,
    ) -> Option<TurnStartHandChoiceView<'a>> {
        frames
            .pending_turn_start_hand_choice_is_valid(self)
            .then(|| frames.turn_start_hand_choice(self.frame_record))
            .flatten()
    }

    pub(crate) fn enemy_phase_record(&self, frames: &Frames) -> Option<EnemyPhaseRecord> {
        (frames.top()
            == Some(Frame::EnemyPhase {
                record: self.frame_record,
            })
            && self.frame_uid != TURN_START_HAND_CHOICE_PENDING_UID)
            .then(|| frames.enemy_phase(self.frame_record))
            .flatten()
            .filter(|record| record.actor_uid == self.frame_uid)
    }

    pub(crate) fn potion_finish_record<'a>(
        &self,
        frames: &'a Frames,
    ) -> Option<PotionFinishView<'a>> {
        (self.frame_uid == POTION_SELECTION_PENDING_UID
            && frames.top()
                == Some(Frame::PotionFinish {
                    record: self.frame_record,
                })
            && frames.continuation_store_is_valid(Some(self)))
        .then(|| frames.potion_finish(self.frame_record))
        .flatten()
        .filter(|record| record.body_stage != PotionBodyStage::GenerationSelecting)
    }

    pub(crate) fn generation_potion_record<'a>(
        &self,
        frames: &'a Frames,
    ) -> Option<PotionFinishView<'a>> {
        (self.frame_uid == GENERATION_POTION_PENDING_UID
            && frames.top()
                == Some(Frame::PotionFinish {
                    record: self.frame_record,
                })
            && frames.continuation_store_is_valid(Some(self)))
        .then(|| frames.potion_finish(self.frame_record))
        .flatten()
        .filter(|record| record.body_stage == PotionBodyStage::GenerationSelecting)
    }

    pub(crate) fn stratagem_potion_record<'a>(
        &self,
        frames: &'a Frames,
    ) -> Option<PotionFinishView<'a>> {
        (self.frame_uid == STRATAGEM_SELECTION_PENDING_UID
            && frames.top()
                == Some(Frame::PotionFinish {
                    record: self.frame_record,
                })
            && frames.continuation_store_is_valid(Some(self)))
        .then(|| frames.potion_finish(self.frame_record))
        .flatten()
        .filter(|record| record.body_stage == PotionBodyStage::AfterShuffle)
    }

    pub(crate) fn stratagem_draw_record<'a>(&self, frames: &'a Frames) -> Option<DrawView<'a>> {
        (self.frame_uid == STRATAGEM_SELECTION_PENDING_UID
            && frames.top()
                == Some(Frame::Draw {
                    record: self.frame_record,
                })
            && frames.continuation_store_is_valid(Some(self)))
        .then(|| frames.draw(self.frame_record))
        .flatten()
        .filter(|record| record.stage == DrawStage::AfterShuffle)
    }

    pub(crate) fn foregone_before_hand_draw_record<'a>(
        &self,
        frames: &'a Frames,
    ) -> Option<BeforeHandDrawPowerView<'a>> {
        (self.frame_uid == FOREGONE_SELECTION_PENDING_UID
            && frames.top()
                == Some(Frame::BeforeHandDrawPower {
                    record: self.frame_record,
                })
            && frames.continuation_store_is_valid(Some(self)))
        .then(|| frames.before_hand_draw_power(self.frame_record))
        .flatten()
    }
}

/// Complete native result location frozen before GeneratePlayCount/body work.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CardResultRoute {
    RemoveBottom = 0,
    ExhaustBottom = 1,
    ExhaustTop = 2,
    DiscardBottom = 3,
    DrawTop = 4,
    HandTop = 5,
    HandBottom = 6,
    RemoteDrawRandom = 7,
}

impl CardResultRoute {
    const fn from_ordinal(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::RemoveBottom),
            1 => Some(Self::ExhaustBottom),
            2 => Some(Self::ExhaustTop),
            3 => Some(Self::DiscardBottom),
            4 => Some(Self::DrawTop),
            5 => Some(Self::HandTop),
            6 => Some(Self::HandBottom),
            7 => Some(Self::RemoteDrawRandom),
            _ => None,
        }
    }
}

/// A packed pending-selection route outside the closed six-bit vocabulary.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct PendingSelectionRoutingError(u8);

impl PendingSelectionRoutingError {
    pub(crate) const fn invalid_record() -> Self {
        Self(0xff)
    }

    /// The rejected packed byte.
    pub(crate) const fn raw(self) -> u8 {
        self.0
    }
}

/// Which exact continuation vocabulary the pending answer uses.
macro_rules! pending_selection_kinds {
    ($(#[$meta:meta])* $name:ident { $($(#[$variant_meta:meta])* $variant:ident),+ $(,)? }) => {
        $(#[$meta])*
        #[repr(u8)]
        #[derive(Copy, Clone, Debug, PartialEq, Eq)]
        pub enum $name {
            $($(#[$variant_meta])* $variant),+
        }

        impl $name {
            /// Number of variants in the packed routing vocabulary.
            pub const COUNT: usize = [$(stringify!($variant)),+].len();
            /// Every variant, in packed discriminant order.
            pub const ALL: [$name; Self::COUNT] = [$($name::$variant),+];
        }
    };
}

pending_selection_kinds! {
    /// Which exact continuation vocabulary the pending answer uses.
    PendingSelectionKind {
        /// The historical Burning Pact replay seam selects one Hand uid.
        Replay,
        /// A generated `select` step resumes by deterministic option ordinal.
        Program,
        /// Purity's ordered zero-through-limit exact Hand exhaustion.
        Purity,
        /// Seeker Strike's exact-one Draw shortlist move-to-Hand choice.
        SeekerStrike,
        /// Abundance's exact-one generated upgraded Power-card choice.
        Abundance,
        /// Discovery's exact-one generated L0 owner-card choice.
        Discovery,
        /// Dredge or Neow's Fury's live-space Discard-to-Hand selector.
        HandCap,
        /// Quasar's ordered three-card screen followed by its skippable answer.
        Quasar,
        /// Glimmer's exact-one live-Hand move to Draw/Top after its awaited Draw.
        Glimmer,
        /// Splash's exact-one generated source-level owner-excluded Attack choice.
        Splash,
        /// Tutor's exact-one live owner Draw-pile physical-card choice.
        Tutor,
    }
}

const _: () = assert!(PileId::COUNT == 5);
const _: () = assert!(PendingSelectionKind::COUNT == 11);

/// One monster.
///
/// `kind` is interned; the 56 power-named `Monster` fields collapse into
/// `powers`; everything else refuses at the boundary until it is modeled.
/// Waterfall Giant's four private scalar/latch fields are carried inline:
/// they change move damage, the forced first-death form, and therefore the
/// terminal result. `steam_pressure` itself remains an ordinary power slot.
///
/// There is deliberately no `Default`: `kind` and `hp` are exactly the two
/// `combat_sim.Monster` fields with no dataclass default, so a monster
/// cannot be conjured without them — [`HotMonster::new`] takes both and
/// leaves the rest at their canonical zero-defaults.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HotMonster {
    /// Active powers, ascending by id.
    pub powers: Slots<PowerId>,
    /// `Monster.misery_debuff_order` — debuff acquisition order.
    pub misery_debuff_order: MiseryOrder,
    /// `Monster.owner_powered_damage_results_this_turn` — how many powered
    /// damage results the owning player has landed on this creature during
    /// the current player turn (`combat_sim.damage_monster` (frozen Python, deleted #2827); reset in
    /// `begin_player_turn`).
    pub owner_powered_damage_results_this_turn: i32,
    /// `Monster.nonowner_same_side_powered_damage_results_this_turn` — how
    /// many powered damage results a player-owned pet has landed on this
    /// creature during the current player turn (`combat_sim.damage_monster` (frozen Python, deleted #2827); reset beside the owner counter in `begin_player_turn`).
    pub nonowner_same_side_powered_damage_results_this_turn: i32,
    /// Forced move state. Terror Eel's `STUNNED` -> `TERROR` chain and the
    /// Tunneler's one-turn `DIZZY` interrupt are admitted alongside
    /// Ceremonial Beast's cosmetic `STUN_MOVE`; the empty string is the
    /// canonical default.
    pub override_state: MonsterOverride,
    /// Dynamic successor paired with Ceremonial Beast's cosmetic stun.
    /// Native carries this independently from `override`, so the boundary
    /// does too and admission validates the exact pair.
    pub forced_follow_up: MonsterFollowUp,
    /// Current hit points.
    pub hp: i32,
    /// Maximum hit points.
    pub max_hp: i32,
    /// Current block.
    pub block: i32,
    /// `Monster.poison_uid` — the solver-only identity of this creature's one
    /// live `PoisonPower` instance, or -1 when it has none.
    ///
    /// Allocated lazily by `_ensure_poison_instance_uid` (frozen Python, deleted #2827) on the
    /// application that makes a previously-unpoisoned creature live, and reset
    /// to -1 when the turn-start tick takes the amount to zero. It exists
    /// because Outbreak and Test Subject key a parked trigger to the exact
    /// power instance; nothing in this slice reads it back, but it is a
    /// projected field, so carrying it is not optional.
    pub poison_uid: i32,
    /// The Lost's exact solo-player `PossessStrengthPower` debit. Zero is the
    /// empty dictionary; a negative value projects as `((0, debit),)`.
    pub possess_strength_debit: i32,
    /// The Forgotten's exact solo-player `PossessSpeedPower` debit. Zero is
    /// the empty dictionary; a negative value projects as `((0, debit),)`.
    pub possess_speed_debit: i32,
    /// Fabricator's shared `_lastSpawned` bot-kind discriminator. `None`
    /// projects as the native empty-string default; only the four exact bot
    /// kinds are admitted at the boundary.
    pub last_spawned: Option<MonsterKind>,
    /// CurlUpPower's private `Data.playedCard` identity. `None` projects as
    /// native `-1`; a live uid is carried only while that exact physical card
    /// is suspended between its powered hit and matching AfterCardPlayed.
    /// Carried as an `i32` with `-1` for "no latch", matching both the native
    /// projection and this struct's existing `poison_uid` idiom. An
    /// `Option<u32>` costs 8 bytes here and pushed `HotMonster` from 80 to 96
    /// (#1618); every per-transition roster clone paid that.
    pub curl_up_card_uid: i32,
    /// Packed owner-local latches. Bit 0 is Louse Progenitor's native
    /// `_curled` state or Frog Knight's owner-disjoint `HasBeetleCharged`.
    /// Bit 1 records whether Demise was acquired after an already-live
    /// Intangible listener. Bit 2 records whether Demise was acquired before
    /// an already-live Ritual listener. Bit 3 is Skittish's once-per-turn
    /// latch, bit 4 a live CrabRagePower, bit 5 Scroll of Biting's
    /// repeated-CHEW latch (#3026), and bit 6 Scroll of Biting's
    /// enemy-phase-internal "CHEW performed, `rand` not yet traversed"
    /// receipt (#3304). Typed accessors keep those meanings independent
    /// while preserving HotMonster's frozen 96-byte width.
    owner_latches: u8,
    /// Waterfall Giant's current Pressure Gun base damage. Two-Tailed Rat's
    /// owner-disjoint signed TurnsUntilSummonable value reuses this integer
    /// lane through the typed accessors below; boundary/admission never expose
    /// both interpretations for one owner.
    pub pressure_gun_damage: i32,
    /// Owner-disjoint signed payload. Waterfall Giant stores the damage
    /// frozen by ABOUT for the following EXPLODE attack; the retained dead
    /// Gremlin Merc stores Heist's returned-gold history amount. Typed
    /// accessors and admission keep the two interpretations disjoint.
    steam_eruption_damage: i32,
    /// Owner-disjoint private monster state. Waterfall Giant stores its
    /// non-negative buildup index in bits 0..30 and ABOUT form latch in bit
    /// 31; Test Subject stores its non-negative signed-Int32
    /// `extra_multi_claw_count` in bits 0..30; Queen stores the Amalgam-death
    /// and retained-Burn latches in bits 0 and 1; Gremlin Merc stores the
    /// encounter's gold-was-stolen latch in bit 0. Boundary/admission expose
    /// only the interpretation selected by `kind`, so no owner can observe
    /// another's payload. Sharing one word avoids moving every roster clone
    /// into the allocator's next size class.
    waterfall_phase: u32,
    /// Random-AI intent projection, encoded as generated move-table indexes.
    ///
    /// The current intent, trailing-three StateLog, and the admitted machine's
    /// ordered UseOnlyOnce log occupy one word. No content name reaches the hot path;
    /// the boundary translates names through `content_tables::RANDOM_MOVES`.
    /// See [`RandomAiState`] for the exact packing.
    pub random_ai: RandomAiState,
    /// Position in this monster's move loop.
    pub loop_pos: i32,
    /// Board position (`Monster.slot`) — the display/targeting index, which
    /// is emphatically *not* `uid`.
    pub slot: i32,
    /// A freshly spawned Wriggler's one do-nothing `SPAWNED` move.
    pub spawn_noop: bool,
    /// `Monster.ritual_fresh` — Ritual's owner-scoped
    /// `WasJustAppliedByEnemy` latch. The first enemy-side end after an
    /// application clears this bit instead of granting Strength.
    pub ritual_fresh: bool,
    /// Owner-disjoint compact revival state. Illusions use 0 while live and 1
    /// while waiting to spend `REVIVE_MOVE`. Test Subject uses bits 0..1 for
    /// its completed-respawn count, bit 2 for Adaptable's retained-corpse
    /// latch, and bit 3 for Nemesis's alternating-Intangible latch. Typed
    /// accessors below keep those logical canonical fields independent.
    pub revive_stage: u8,
    /// Creation-order identity (`Monster.uid`) — never a roster index.
    pub uid: u32,
    /// Interned monster kind.
    pub kind: MonsterKind,
}

impl HotMonster {
    const LOUSE_CURLED_MASK: u8 = 0b0000_0001;
    const DEMISE_AFTER_INTANGIBLE_MASK: u8 = 0b0000_0010;
    const DEMISE_BEFORE_RITUAL_MASK: u8 = 0b0000_0100;
    /// SkittishPower's `Data.hasGainedBlockThisTurn` once-per-turn latch
    /// (#2481 slot 3).
    const SKITTISH_USED_MASK: u8 = 0b0000_1000;
    /// A live `CrabRagePower` instance on this owner (#2654).
    const CRAB_RAGE_MASK: u8 = 0b0001_0000;
    /// Scroll of Biting's current CHEW was selected by its own post-CHEW
    /// `rand`, so its two trailing StateLog entries are CHEW, CHEW (#3026).
    const SCROLL_CHEW_REPEATED_MASK: u8 = 0b0010_0000;
    /// Scroll of Biting performed CHEW this enemy phase and its `rand`
    /// successor waits for `PrepareForNextTurn` (#3304). Written by
    /// `moves::normal::attack_scroll_branch`, consumed (always cleared) by
    /// `engine::turn::prepare_random_ai` before `EndTurn` returns, so it is
    /// never a stable-state fact and the boundary carries no field for it.
    const SCROLL_BRANCH_PENDING_MASK: u8 = 0b0100_0000;
    const OWNER_LATCHES_MASK: u8 = Self::LOUSE_CURLED_MASK
        | Self::DEMISE_AFTER_INTANGIBLE_MASK
        | Self::DEMISE_BEFORE_RITUAL_MASK
        | Self::SKITTISH_USED_MASK
        | Self::CRAB_RAGE_MASK
        | Self::SCROLL_CHEW_REPEATED_MASK
        | Self::SCROLL_BRANCH_PENDING_MASK;
    const TEST_SUBJECT_RESPAWNS_MASK: u8 = 0b0000_0011;
    const TEST_SUBJECT_ADAPTABLE_REVIVING_MASK: u8 = 0b0000_0100;
    const TEST_SUBJECT_NEMESIS_INTANGIBLE_MASK: u8 = 0b0000_1000;
    const QUEEN_AMALGAM_DEAD_MASK: u32 = 0b01;
    const QUEEN_BURN_BRIGHT_RETAINED_MASK: u32 = 0b10;
    const GREMLIN_MERC_GOLD_WAS_STOLEN_MASK: u32 = 0b01;

    pub(crate) fn knowledge_demon_curse_counter(&self) -> Option<u8> {
        (self.kind == MonsterKind::KnowledgeDemon && self.revive_stage <= 3)
            .then_some(self.revive_stage)
    }

    pub(crate) fn set_knowledge_demon_curse_counter(&mut self, value: u8) -> bool {
        if self.kind != MonsterKind::KnowledgeDemon || value > 3 {
            return false;
        }
        self.revive_stage = value;
        true
    }

    /// A monster at every canonical zero-default but the two fields Python
    /// requires.
    pub fn new(kind: MonsterKind, hp: i32) -> Self {
        Self {
            powers: Slots::new(),
            misery_debuff_order: MiseryOrder::new(),
            owner_powered_damage_results_this_turn: 0,
            nonowner_same_side_powered_damage_results_this_turn: 0,
            override_state: MonsterOverride::None,
            forced_follow_up: MonsterFollowUp::None,
            hp,
            max_hp: 0,
            block: 0,
            poison_uid: -1,
            possess_strength_debit: 0,
            possess_speed_debit: 0,
            last_spawned: None,
            curl_up_card_uid: -1,
            owner_latches: 0,
            pressure_gun_damage: 0,
            steam_eruption_damage: 0,
            waterfall_phase: 0,
            random_ai: RandomAiState::new(),
            loop_pos: 0,
            slot: 0,
            spawn_noop: false,
            ritual_fresh: false,
            revive_stage: 0,
            uid: 0,
            kind,
        }
    }

    /// Louse Progenitor's `_curled` latch or Frog Knight's owner-disjoint
    /// Beetle Charged latch.
    pub(crate) fn louse_curled(&self) -> bool {
        self.owner_latches & Self::LOUSE_CURLED_MASK != 0
    }

    pub(crate) fn set_louse_curled(&mut self, value: bool) {
        if value {
            self.owner_latches |= Self::LOUSE_CURLED_MASK;
        } else {
            self.owner_latches &= !Self::LOUSE_CURLED_MASK;
        }
    }

    /// A live `CrabRagePower` instance on this Kaiser Crab — the
    /// `Monster.crab_rage` field of the frozen oracle.
    ///
    /// Native v0.111.0 (DLL `9cb4f1ad`): `Crusher/<AfterAddedToRoom>d__36::MoveNext`
    /// `0x3570b8` IL_0100-IL_0112 and `Rocket/<AfterAddedToRoom>d__29::MoveNext`
    /// `0x367c94` IL_0197 apply one `CrabRagePower` to their own `Creature`.
    /// The instance removes itself in `CrabRagePower/<AfterDeath>d__8::MoveNext`
    /// `0x337f54` IL_0154 (`PowerCmd.Remove`), and the owner's own death
    /// removes it through `Creature.RemoveAllPowersAfterDeath` `0x11dbac`
    /// (`CrabRagePower` inherits the base `ShouldPowerBeRemovedAfterOwnerDeath`
    /// `0x840dd`, which returns 1).
    pub(crate) fn crab_rage(&self) -> bool {
        self.owner_latches & Self::CRAB_RAGE_MASK != 0
    }

    pub(crate) fn set_crab_rage(&mut self, value: bool) {
        if value {
            self.owner_latches |= Self::CRAB_RAGE_MASK;
        } else {
            self.owner_latches &= !Self::CRAB_RAGE_MASK;
        }
    }

    /// True when this owner's live Demise listener was acquired after its
    /// live Intangible listener. Intensity re-stacks preserve this bit.
    pub(crate) fn demise_after_intangible(&self) -> bool {
        self.owner_latches & Self::DEMISE_AFTER_INTANGIBLE_MASK != 0
    }

    pub(crate) fn set_demise_after_intangible(&mut self, value: bool) {
        if value {
            self.owner_latches |= Self::DEMISE_AFTER_INTANGIBLE_MASK;
        } else {
            self.owner_latches &= !Self::DEMISE_AFTER_INTANGIBLE_MASK;
        }
    }

    /// True when this owner's live Demise listener was acquired before its
    /// live Ritual listener. Intensity re-stacks preserve this bit.
    pub(crate) fn demise_before_ritual(&self) -> bool {
        self.owner_latches & Self::DEMISE_BEFORE_RITUAL_MASK != 0
    }

    pub(crate) fn set_demise_before_ritual(&mut self, value: bool) {
        if value {
            self.owner_latches |= Self::DEMISE_BEFORE_RITUAL_MASK;
        } else {
            self.owner_latches &= !Self::DEMISE_BEFORE_RITUAL_MASK;
        }
    }

    /// SkittishPower's once-per-turn latch — native
    /// `SkittishPower/Data.hasGainedBlockThisTurn` (`get_` RVA `0xa7a9a`,
    /// `set_` RVA `0xa7aa7`), mirrored by Python `Monster.skittish_used`. It is
    /// per-power-instance internal data rather than an Amount, which is why it
    /// needs a latch of its own instead of folding into the power slot.
    pub(crate) fn skittish_used(&self) -> bool {
        self.owner_latches & Self::SKITTISH_USED_MASK != 0
    }

    pub(crate) fn set_skittish_used(&mut self, value: bool) {
        if value {
            self.owner_latches |= Self::SKITTISH_USED_MASK;
        } else {
            self.owner_latches &= !Self::SKITTISH_USED_MASK;
        }
    }

    /// Scroll of Biting's repeated-CHEW latch (#3026). The Scroll's hot state
    /// is a fixed-loop position without a StateLog; this one bit is the part
    /// of its native StateLog that `RandomBranchState::GetStateWeight`
    /// (RVA `0x79380`) reads for CHEW's `maxRepeats = 2` branch: whether the
    /// two trailing entries are both CHEW. Set only while the Scroll is parked
    /// on a CHEW its own post-CHEW `rand` selected; see
    /// `engine::turn::roll_scroll_of_biting_branch`.
    pub(crate) fn scroll_chew_repeated(&self) -> bool {
        self.owner_latches & Self::SCROLL_CHEW_REPEATED_MASK != 0
    }

    pub(crate) fn set_scroll_chew_repeated(&mut self, value: bool) {
        if value {
            self.owner_latches |= Self::SCROLL_CHEW_REPEATED_MASK;
        } else {
            self.owner_latches &= !Self::SCROLL_CHEW_REPEATED_MASK;
        }
    }

    /// Scroll of Biting's pending post-CHEW `rand` traversal (#3304): CHEW
    /// completed this enemy phase and `PrepareForNextTurn` has not yet rolled
    /// its successor. See `engine::turn::prepare_random_ai`.
    pub(crate) fn scroll_branch_pending(&self) -> bool {
        self.owner_latches & Self::SCROLL_BRANCH_PENDING_MASK != 0
    }

    pub(crate) fn set_scroll_branch_pending(&mut self, value: bool) {
        if value {
            self.owner_latches |= Self::SCROLL_BRANCH_PENDING_MASK;
        } else {
            self.owner_latches &= !Self::SCROLL_BRANCH_PENDING_MASK;
        }
    }

    /// The packed byte has no unowned bits at a public boundary.
    pub(crate) fn owner_latches_are_exact(&self) -> bool {
        self.owner_latches & !Self::OWNER_LATCHES_MASK == 0
    }

    /// Waterfall Giant's complete non-negative buildup index.
    pub fn pressure_buildup_idx(&self) -> i32 {
        (self.waterfall_phase & i32::MAX as u32) as i32
    }

    /// Set the buildup index while preserving the independent ABOUT latch.
    /// Negative values are not representable in the native lifecycle.
    pub fn set_pressure_buildup_idx(&mut self, value: i32) -> bool {
        let Ok(value) = u32::try_from(value) else {
            return false;
        };
        self.waterfall_phase = (self.waterfall_phase & (1_u32 << 31)) | value;
        true
    }

    /// Whether the Waterfall Giant has completed its first-death transform.
    pub fn is_about_to_blow(&self) -> bool {
        self.waterfall_phase & (1_u32 << 31) != 0
    }

    /// Set the ABOUT latch while preserving the complete buildup index.
    pub fn set_is_about_to_blow(&mut self, value: bool) {
        if value {
            self.waterfall_phase |= 1_u32 << 31;
        } else {
            self.waterfall_phase &= i32::MAX as u32;
        }
    }

    /// Test Subject's completed form-transition count (0, 1, or 2).
    pub fn test_subject_respawns(&self) -> u8 {
        self.revive_stage & Self::TEST_SUBJECT_RESPAWNS_MASK
    }

    /// Replace Test Subject's form-transition count while preserving its two
    /// independent lifecycle latches. Values above the complete three-form
    /// range are not representable.
    pub fn set_test_subject_respawns(&mut self, value: u8) -> bool {
        if value > 2 {
            return false;
        }
        self.revive_stage = (self.revive_stage & !Self::TEST_SUBJECT_RESPAWNS_MASK) | value;
        true
    }

    /// Whether Adaptable retained this Test Subject corpse for its next
    /// enemy action.
    pub fn test_subject_adaptable_reviving(&self) -> bool {
        self.revive_stage & Self::TEST_SUBJECT_ADAPTABLE_REVIVING_MASK != 0
    }

    /// Replace Adaptable's retained-corpse latch without changing form or
    /// Nemesis state.
    pub fn set_test_subject_adaptable_reviving(&mut self, value: bool) {
        if value {
            self.revive_stage |= Self::TEST_SUBJECT_ADAPTABLE_REVIVING_MASK;
        } else {
            self.revive_stage &= !Self::TEST_SUBJECT_ADAPTABLE_REVIVING_MASK;
        }
    }

    /// Whether Nemesis's next side-end state is the Intangible-on half of its
    /// alternating lifecycle.
    pub fn test_subject_nemesis_apply_intangible(&self) -> bool {
        self.revive_stage & Self::TEST_SUBJECT_NEMESIS_INTANGIBLE_MASK != 0
    }

    /// Replace Nemesis's alternating-Intangible latch without changing form
    /// or Adaptable state.
    pub fn set_test_subject_nemesis_apply_intangible(&mut self, value: bool) {
        if value {
            self.revive_stage |= Self::TEST_SUBJECT_NEMESIS_INTANGIBLE_MASK;
        } else {
            self.revive_stage &= !Self::TEST_SUBJECT_NEMESIS_INTANGIBLE_MASK;
        }
    }

    /// Test Subject's signed-Int32 Multi Claw growth counter.
    pub fn test_subject_extra_multi_claw_count(&self) -> i32 {
        (self.waterfall_phase & i32::MAX as u32) as i32
    }

    /// Replace Test Subject's Multi Claw growth counter. The native field is
    /// a non-negative signed Int32 at every admitted command boundary.
    pub fn set_test_subject_extra_multi_claw_count(&mut self, value: i32) -> bool {
        let Ok(value) = u32::try_from(value) else {
            return false;
        };
        self.waterfall_phase = value;
        true
    }

    /// Bygone Effigy's `SlowPower` private cards-played counter.
    ///
    /// Native keeps the instance's Amount fixed at 1 and carries the count in
    /// the power's `SlowAmount` DynamicVar (`SlowPower::AfterCardPlayed` RVA
    /// `0xa7dac` increments it); Python mirrors that in its `Monster.slow`
    /// field, which `player_attack` (frozen Python, deleted #2827) reads as the damage multiplier.
    /// Owner-disjoint from every other tenant of this word — the
    /// Effigy is neither a Waterfall Giant, a Test Subject, the Queen nor a
    /// Gremlin Merc — so it reuses the whole non-negative lane rather than
    /// widening `HotMonster` past its reviewed 96-byte cap.
    pub fn bygone_effigy_slow(&self) -> i32 {
        (self.waterfall_phase & i32::MAX as u32) as i32
    }

    /// Replace the Effigy's cards-played counter. The native `SlowAmount`
    /// DynamicVar is non-negative at every command boundary: it only ever
    /// increments by one per card played and resets to zero at the owner's
    /// own side start.
    pub fn set_bygone_effigy_slow(&mut self, value: i32) -> bool {
        let Ok(value) = u32::try_from(value) else {
            return false;
        };
        self.waterfall_phase = value;
        true
    }

    /// Skulking Colony's hardened-shell side-turn HP-loss window.
    ///
    /// Python `Monster.shell_window` accumulates the HP the Colony has already
    /// lost during the current side turn; `damage_monster` (frozen Python, deleted #2827) caps each
    /// further loss at `HARDENED_SHELL - shell_window`. Owner-disjoint from
    /// every other tenant of this word for the same reason as the Effigy
    /// counter above.
    pub fn skulking_colony_shell_window(&self) -> i32 {
        (self.waterfall_phase & i32::MAX as u32) as i32
    }

    /// Replace the Colony's shell window. Non-negative at every command
    /// boundary: it accumulates realised HP loss and resets to zero at the
    /// owner's own side start.
    pub fn set_skulking_colony_shell_window(&mut self, value: i32) -> bool {
        let Ok(value) = u32::try_from(value) else {
            return false;
        };
        self.waterfall_phase = value;
        true
    }

    /// Queen's latched observation that her fixed Torch Head Amalgam died.
    pub fn queen_amalgam_dead(&self) -> bool {
        self.waterfall_phase & Self::QUEEN_AMALGAM_DEAD_MASK != 0
    }

    /// Replace Queen's Amalgam-death latch without disturbing retained Burn.
    pub fn set_queen_amalgam_dead(&mut self, value: bool) {
        if value {
            self.waterfall_phase |= Self::QUEEN_AMALGAM_DEAD_MASK;
        } else {
            self.waterfall_phase &= !Self::QUEEN_AMALGAM_DEAD_MASK;
        }
    }

    /// Whether a stunned Queen retained the already-selected Burn follow-up.
    pub fn queen_burn_bright_retained(&self) -> bool {
        self.waterfall_phase & Self::QUEEN_BURN_BRIGHT_RETAINED_MASK != 0
    }

    /// Replace Queen's retained-Burn latch without disturbing death state.
    pub fn set_queen_burn_bright_retained(&mut self, value: bool) {
        if value {
            self.waterfall_phase |= Self::QUEEN_BURN_BRIGHT_RETAINED_MASK;
        } else {
            self.waterfall_phase &= !Self::QUEEN_BURN_BRIGHT_RETAINED_MASK;
        }
    }

    /// Whether the shared private word contains only Queen's two latch bits.
    pub(crate) fn queen_private_state_is_exact(&self) -> bool {
        self.waterfall_phase
            & !(Self::QUEEN_AMALGAM_DEAD_MASK | Self::QUEEN_BURN_BRIGHT_RETAINED_MASK)
            == 0
    }

    /// The encounter history latch written only after Surprise has published
    /// both Gremlin children.
    pub(crate) fn gremlin_merc_gold_was_stolen(&self) -> bool {
        self.kind == MonsterKind::GremlinMerc
            && self.waterfall_phase & Self::GREMLIN_MERC_GOLD_WAS_STOLEN_MASK != 0
    }

    /// Replace the retained Merc's stolen-gold history latch.
    pub(crate) fn set_gremlin_merc_gold_was_stolen(&mut self, value: bool) -> bool {
        if self.kind != MonsterKind::GremlinMerc {
            return false;
        }
        if value {
            self.waterfall_phase |= Self::GREMLIN_MERC_GOLD_WAS_STOLEN_MASK;
        } else {
            self.waterfall_phase &= !Self::GREMLIN_MERC_GOLD_WAS_STOLEN_MASK;
        }
        true
    }

    /// Whether the private packed word contains only the Merc latch bit.
    pub(crate) fn gremlin_merc_private_state_is_exact(&self) -> bool {
        self.kind == MonsterKind::GremlinMerc
            && self.waterfall_phase & !Self::GREMLIN_MERC_GOLD_WAS_STOLEN_MASK == 0
            && self.steam_eruption_damage >= 0
    }

    /// Aeonglass's private Strength-ramp count.
    pub(crate) fn aeonglass_additional_strength(&self) -> i32 {
        if self.kind == MonsterKind::Aeonglass {
            self.pressure_gun_damage
        } else {
            0
        }
    }

    pub(crate) fn set_aeonglass_additional_strength(&mut self, value: i32) -> bool {
        if self.kind != MonsterKind::Aeonglass || value < 0 {
            return false;
        }
        self.pressure_gun_damage = value;
        true
    }

    /// Aeonglass's private all-Wither fake-upgrade count.
    pub(crate) fn aeonglass_wither_upgrade_count(&self) -> i32 {
        if self.kind == MonsterKind::Aeonglass {
            self.steam_eruption_damage
        } else {
            0
        }
    }

    pub(crate) fn set_aeonglass_wither_upgrade_count(&mut self, value: i32) -> bool {
        if self.kind != MonsterKind::Aeonglass || value < 0 {
            return false;
        }
        self.steam_eruption_damage = value;
        true
    }

    pub(crate) fn aeonglass_private_state_is_exact(&self) -> bool {
        self.kind == MonsterKind::Aeonglass
            && self.pressure_gun_damage >= 0
            && self.steam_eruption_damage >= 0
            && self.waterfall_phase == 0
            && self.revive_stage == 0
    }

    /// Heist's exact returned-gold projection on the retained Merc.
    pub(crate) fn gremlin_merc_returned_gold(&self) -> i32 {
        if self.kind == MonsterKind::GremlinMerc {
            self.steam_eruption_damage
        } else {
            0
        }
    }

    /// Replace Heist's returned-gold projection on its retained Merc owner.
    pub(crate) fn set_gremlin_merc_returned_gold(&mut self, value: i32) -> bool {
        if self.kind != MonsterKind::GremlinMerc || value < 0 {
            return false;
        }
        self.steam_eruption_damage = value;
        true
    }

    /// Exact stolen DeckVersion uid, encoded as uid+1 in the owner-disjoint
    /// private word so every valid non-sentinel physical uid is representable.
    pub(crate) fn hopper_swipe_uid(&self) -> Option<u32> {
        (self.kind == MonsterKind::ThievingHopper && self.waterfall_phase != 0)
            .then(|| self.waterfall_phase - 1)
    }

    /// Replace the Hopper Swipe identity without exposing another owner's
    /// packed interpretation.
    pub(crate) fn set_hopper_swipe_uid(&mut self, value: Option<u32>) -> bool {
        if self.kind != MonsterKind::ThievingHopper || value == Some(u32::MAX) {
            return false;
        }
        self.waterfall_phase = value.map_or(0, |uid| uid + 1);
        true
    }

    /// Whether the Hopper-only packed lanes contain no forbidden bits.
    pub(crate) fn hopper_private_state_is_exact(&self) -> bool {
        self.kind == MonsterKind::ThievingHopper
            && self.revive_stage == 0
            && self.pressure_gun_damage == 0
            && self.steam_eruption_damage == 0
    }

    /// Two-Tailed Rat's signed TurnsUntilSummonable counter.
    pub fn rat_turns_until_summonable(&self) -> i32 {
        self.pressure_gun_damage
    }

    /// Replace the Rat-only TurnsUntilSummonable counter.
    pub fn set_rat_turns_until_summonable(&mut self, value: i32) {
        self.pressure_gun_damage = value;
    }

    /// Two-Tailed Rat's synchronized completed-call count.
    pub fn rat_call_for_backup_count(&self) -> u8 {
        self.random_ai.rat_call_for_backup_count()
    }

    /// Replace the Rat-only completed-call count.
    pub fn set_rat_call_for_backup_count(&mut self, value: u8) -> bool {
        self.random_ai.set_rat_call_for_backup_count(value)
    }

    /// Whether this Rat already rolled its spawn-time starter this side.
    pub fn rat_spawn_fresh(&self) -> bool {
        self.random_ai.rat_spawn_fresh()
    }

    /// Replace the Rat-only spawn-time starter latch.
    pub fn set_rat_spawn_fresh(&mut self, value: bool) {
        self.random_ai.set_rat_spawn_fresh(value);
    }

    /// Waterfall Giant's frozen Steam Eruption damage. Zero is the unfrozen
    /// value outside ABOUT/EXPLODE.
    pub(crate) fn waterfall_steam_eruption_damage(&self) -> i32 {
        if self.kind == MonsterKind::WaterfallGiant {
            self.steam_eruption_damage
        } else {
            0
        }
    }

    /// Replace Waterfall Giant's frozen Steam Eruption damage.
    pub(crate) fn set_waterfall_steam_eruption_damage(&mut self, damage: i32) -> bool {
        if self.kind != MonsterKind::WaterfallGiant {
            return false;
        }
        self.steam_eruption_damage = damage;
        true
    }

    /// Whether the private Waterfall payload is physically present. Public
    /// admission uses this raw predicate to reject a forged alien owner.
    pub(crate) fn waterfall_steam_payload_is_set(&self) -> bool {
        self.steam_eruption_damage != 0
    }

    #[cfg(test)]
    pub(crate) fn forge_waterfall_steam_payload(&mut self, value: i32) {
        self.steam_eruption_damage = value;
    }
}

/// One random-AI monster's complete compact command-boundary state.
///
/// Each move is its one-based index in the generated per-kind random table;
/// zero means absent.  Three bits per index cover the current maximum table
/// width (four) without baking any monster or move identity into the layout.
/// StateLog retains the native trailing-three window. The separate ordered
/// UseOnlyOnce log retains up to the complete generated table width:
///
/// * bits 0..2: current intent;
/// * bits 3..4 + 5..13: StateLog length and three entries;
/// * bits 14..16 + 17..28: UseOnlyOnce-log length and up to four entries.
/// * bits 29..30: Two-Tailed Rat CallForBackupCount;
/// * bit 31: Two-Tailed Rat spawn-time fresh-roll latch.
///
/// The boundary rejects tables or lists that do not fit. Admission then
/// validates the exact reachable machine shape before any action can mutate
/// state or consume the shared AI stream.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct RandomAiState(u32);

impl RandomAiState {
    const INDEX_BITS: u32 = 3;
    const INDEX_MASK: u32 = (1 << Self::INDEX_BITS) - 1;
    const LOG_LEN_SHIFT: u32 = 3;
    const LOG_SHIFT: u32 = 5;
    const ONCE_LEN_SHIFT: u32 = 14;
    const ONCE_SHIFT: u32 = 17;
    const RAT_CALL_COUNT_SHIFT: u32 = 29;
    const RAT_CALL_COUNT_MASK: u32 = 0b11 << Self::RAT_CALL_COUNT_SHIFT;
    const RAT_FRESH_MASK: u32 = 1 << 31;
    const LOG_MAX_ENTRIES: usize = 3;
    const ONCE_MAX_ENTRIES: usize = 4;

    /// Empty canonical state.
    pub const fn new() -> Self {
        Self(0)
    }

    /// Whether all three canonical fields are at their defaults.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Current generated-table index.
    pub fn next(self) -> Option<u8> {
        Self::decode_index(self.0)
    }

    /// Replace the current generated-table index.
    pub fn set_next(&mut self, index: Option<u8>) -> bool {
        let Some(encoded) = Self::encode_index(index) else {
            return false;
        };
        self.0 = (self.0 & !Self::INDEX_MASK) | encoded;
        true
    }

    /// Number of entries in the trailing StateLog.
    pub fn log_len(self) -> usize {
        ((self.0 >> Self::LOG_LEN_SHIFT) & 0b11) as usize
    }

    /// One trailing StateLog table index.
    pub fn log_at(self, position: usize) -> Option<u8> {
        (position < self.log_len()).then(|| {
            let encoded = self.0 >> (Self::LOG_SHIFT + position as u32 * Self::INDEX_BITS);
            Self::decode_index(encoded).expect("encoded random-AI log indexes are nonzero")
        })
    }

    /// Replace the complete trailing StateLog.
    pub fn set_log(&mut self, values: &[u8]) -> bool {
        self.set_sequence(
            Self::LOG_LEN_SHIFT,
            Self::LOG_SHIFT,
            Self::LOG_MAX_ENTRIES,
            values,
        )
    }

    /// Append a rolled/followed intent, retaining the trailing three entries.
    pub fn push_log(&mut self, value: u8) -> bool {
        let mut values = [0; Self::LOG_MAX_ENTRIES];
        let len = self.log_len();
        for (position, slot) in values.iter_mut().enumerate().take(len) {
            *slot = self.log_at(position).expect("position is below length");
        }
        if len < Self::LOG_MAX_ENTRIES {
            values[len] = value;
            self.set_log(&values[..len + 1])
        } else {
            self.set_log(&[values[1], values[2], value])
        }
    }

    /// Number of entries in the ordered UseOnlyOnce log.
    pub fn once_len(self) -> usize {
        ((self.0 >> Self::ONCE_LEN_SHIFT) & 0b111) as usize
    }

    /// One UseOnlyOnce-log table index.
    pub fn once_at(self, position: usize) -> Option<u8> {
        (position < self.once_len()).then(|| {
            let encoded = self.0 >> (Self::ONCE_SHIFT + position as u32 * Self::INDEX_BITS);
            Self::decode_index(encoded).expect("encoded UseOnlyOnce indexes are nonzero")
        })
    }

    /// Replace the complete ordered UseOnlyOnce log.
    pub fn set_once(&mut self, values: &[u8]) -> bool {
        self.set_sequence(
            Self::ONCE_LEN_SHIFT,
            Self::ONCE_SHIFT,
            Self::ONCE_MAX_ENTRIES,
            values,
        )
    }

    /// Append one newly used UseOnlyOnce intent.
    pub fn push_once(&mut self, value: u8) -> bool {
        let len = self.once_len();
        if len == Self::ONCE_MAX_ENTRIES {
            return false;
        }
        let mut values = [0; Self::ONCE_MAX_ENTRIES];
        for (position, slot) in values.iter_mut().enumerate().take(len) {
            *slot = self.once_at(position).expect("position is below length");
        }
        values[len] = value;
        self.set_once(&values[..len + 1])
    }

    /// Two-Tailed Rat's synchronized CallForBackupCount.
    pub fn rat_call_for_backup_count(self) -> u8 {
        ((self.0 & Self::RAT_CALL_COUNT_MASK) >> Self::RAT_CALL_COUNT_SHIFT) as u8
    }

    /// Replace the Rat-only call count while preserving its AI machine.
    pub fn set_rat_call_for_backup_count(&mut self, value: u8) -> bool {
        if value > 3 {
            return false;
        }
        self.0 = (self.0 & !Self::RAT_CALL_COUNT_MASK)
            | (u32::from(value) << Self::RAT_CALL_COUNT_SHIFT);
        true
    }

    /// Whether the Rat's spawn-time starter roll happened this side.
    pub fn rat_spawn_fresh(self) -> bool {
        self.0 & Self::RAT_FRESH_MASK != 0
    }

    /// Replace the Rat-only fresh-roll latch while preserving its AI machine.
    pub fn set_rat_spawn_fresh(&mut self, value: bool) {
        if value {
            self.0 |= Self::RAT_FRESH_MASK;
        } else {
            self.0 &= !Self::RAT_FRESH_MASK;
        }
    }

    fn set_sequence(
        &mut self,
        len_shift: u32,
        value_shift: u32,
        max_entries: usize,
        values: &[u8],
    ) -> bool {
        if values.len() > max_entries {
            return false;
        }
        let len_bits = if max_entries == Self::LOG_MAX_ENTRIES {
            2
        } else {
            3
        };
        let width = len_bits + max_entries as u32 * Self::INDEX_BITS;
        let mask = ((1_u32 << width) - 1) << len_shift;
        let mut encoded = (values.len() as u32) << len_shift;
        for (position, value) in values.iter().copied().enumerate() {
            let Some(value) = Self::encode_index(Some(value)) else {
                return false;
            };
            encoded |= value << (value_shift + position as u32 * Self::INDEX_BITS);
        }
        self.0 = (self.0 & !mask) | encoded;
        true
    }

    fn encode_index(index: Option<u8>) -> Option<u32> {
        match index {
            None => Some(0),
            Some(index) if u32::from(index) < Self::INDEX_MASK => Some(u32::from(index) + 1),
            Some(_) => None,
        }
    }

    fn decode_index(encoded: u32) -> Option<u8> {
        let value = encoded & Self::INDEX_MASK;
        (value != 0).then(|| (value - 1) as u8)
    }
}

/// The play-history counters the engine core maintains.
///
/// Python keeps these as ordinary `State` fields; they are grouped here only
/// so the engine's bookkeeping reads as one unit and so the turn rollover is
/// a single obvious assignment rather than nine scattered ones. They stay
/// **inline** — every card play writes several of them, so a copy-on-write
/// indirection would allocate once per transition to save 28 bytes.
///
/// Widths are `i16` for the per-turn counters (a turn that plays 32,768 cards
/// is not reachable, and the boundary refuses rather than wrapping if one
/// ever arrives) and `i32` for the combat-scoped ones.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct HotHistory {
    /// `State.card_plays_finished_combat`.
    pub card_plays_finished_combat: i32,
    /// `State.player_unblocked_damage_results_combat`.
    pub player_unblocked_damage_results_combat: i32,
    /// `State.owner_cards_exhausted_combat` — how many cards this player has
    /// exhausted during the whole combat (`_card_exhausted`, frozen Python, deleted #2827).
    pub owner_cards_exhausted_combat: i32,
    /// `State.owner_generated_cards_combat` — owner-created cards whose
    /// `CardGenerated` history entry has begun during this combat.
    pub owner_generated_cards_combat: i32,
    /// `State.round_number` — advances with the enemy phase, defaults to 1.
    pub round_number: i16,
    /// `State.plays_this_turn` (EchoForm's this-turn window).
    pub plays_this_turn: i16,
    /// `State.manual_card_plays_finished_this_turn`.
    pub manual_card_plays_finished_this_turn: i16,
    /// `State.owner_card_plays_finished_this_turn`.
    pub owner_card_plays_finished_this_turn: i16,
    /// `State.owner_attack_plays_started_this_turn`.
    pub owner_attack_plays_started_this_turn: i16,
    /// `State.zero_energy_attack_plays_started_this_turn` — the subset of the
    /// counter above whose play spent no energy (`_advance_card_play_frame`, frozen Python, deleted #2827). An X-cost card spends everything it had, so it lands here
    /// exactly when the player entered the play at zero energy.
    pub zero_energy_attack_plays_started_this_turn: i16,
    /// `State.attack_skill_plays_started_this_turn`.
    pub attack_skill_plays_started_this_turn: i16,
    /// `State.attack_plays_finished_this_turn`.
    pub attack_plays_finished_this_turn: i16,
    /// `State.shiv_plays_finished_this_turn` — owner Shiv plays whose
    /// `CardPlayFinished` history entry has occurred during this player turn.
    pub shiv_plays_finished_this_turn: i16,
    /// `State.skill_plays_finished_this_turn`.
    pub skill_plays_finished_this_turn: i16,
    /// `State.energy_spent_this_turn`.
    pub energy_spent_this_turn: i16,
    /// `State.stars_gained_this_turn` — positive successful Star gains in the
    /// current owner turn, recorded before any AfterStarsGained listener.
    pub stars_gained_this_turn: i16,
    /// `State.card_block_gains` — Unmovable's this-turn history window.
    pub card_block_gains: i16,
    /// `State.non_hand_draws_this_turn` — every drawn card that did **not**
    /// come from the turn-start hand draw (`_draw_one_iteration`, frozen Python, deleted #2827).
    pub non_hand_draws_this_turn: i16,
    /// `State.status_draws_this_turn` — Status cards drawn during this turn.
    pub status_draws_this_turn: i16,
    /// `State.discarded_cards_this_turn` — `CardCmd.DiscardAndDraw`'s
    /// per-card history entry (`_discard_and_draw`, frozen Python, deleted #2827).
    pub discarded_cards_this_turn: i16,
    /// `State.over` — combat has ended.
    pub over: bool,
    /// `State.player_unblocked_damage_this_turn`.
    pub player_unblocked_damage_this_turn: bool,
    /// `State.owner_card_exhausted_this_turn` (`_card_exhausted`, frozen Python, deleted #2827).
    pub owner_card_exhausted_this_turn: bool,
    /// `State.doom_applied_by_player_this_turn` — Death's Door's owner-turn
    /// history latch. Neurosurge is the first admitted writer.
    pub doom_applied_by_player_this_turn: bool,
}

impl HotHistory {
    /// The `combat_sim.State` dataclass defaults for these fields.
    pub fn at_defaults() -> Self {
        Self {
            card_plays_finished_combat: 0,
            player_unblocked_damage_results_combat: 0,
            owner_cards_exhausted_combat: 0,
            owner_generated_cards_combat: 0,
            round_number: 1,
            plays_this_turn: 0,
            manual_card_plays_finished_this_turn: 0,
            owner_card_plays_finished_this_turn: 0,
            owner_attack_plays_started_this_turn: 0,
            zero_energy_attack_plays_started_this_turn: 0,
            attack_skill_plays_started_this_turn: 0,
            attack_plays_finished_this_turn: 0,
            shiv_plays_finished_this_turn: 0,
            skill_plays_finished_this_turn: 0,
            energy_spent_this_turn: 0,
            stars_gained_this_turn: 0,
            card_block_gains: 0,
            non_hand_draws_this_turn: 0,
            status_draws_this_turn: 0,
            discarded_cards_this_turn: 0,
            over: false,
            player_unblocked_damage_this_turn: false,
            owner_card_exhausted_this_turn: false,
            doom_applied_by_player_this_turn: false,
        }
    }
}

/// The search-owned combat state.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HotState {
    /// The nine live xoshiro streams, words and counters.
    pub rng: HotRng,
    /// The five ordered piles.
    pub piles: HotPiles,
    /// The roster, in creation order.
    pub monsters: Arc<Vec<HotMonster>>,
    /// The player's active powers, ascending by id.
    pub powers: Slots<PowerId>,
    /// Modified card instances, ascending by uid.
    pub card_states: CardStates,
    /// The continuation stack, bottom to top.
    pub frames: Frames,
    /// An external selection paired with the top continuation frame.
    pub pending: Option<Arc<PendingSelection>>,
    /// Character base slots, live capacity, and the ordered orb queue.
    pub orbs: HotOrbs,
    /// Card-event listener acquisition order and private power counters.
    pub fanouts: HotFanouts,
    /// `State.hp`.
    pub hp: i32,
    /// `State.max_hp`.
    pub max_hp: i32,
    /// `State.block`.
    pub block: i32,
    /// `State.gold`.
    pub gold: i32,
    /// `State.cards_drawn_combat`.
    pub cards_drawn_combat: i32,
    /// `State.ps_strikes` — the Perfected Strike multiplier count.
    pub ps_strikes: i32,
    /// `State.next_card_uid` — the deterministic physical-card allocator.
    pub next_card_uid: u32,
    /// `State.next_generated_hook_uid` — the generated-card callback epoch
    /// allocator. The current admitted surface completes each callback
    /// synchronously, but the monotonic cursor remains canonical state.
    pub next_generated_hook_uid: i32,
    /// `State.next_poison_uid` — the allocator behind
    /// [`HotMonster::poison_uid`].
    pub next_poison_uid: i32,
    /// `State.energy`.
    pub energy: i16,
    /// `State.stars` — the player's current spendable Star balance.
    pub stars: i16,
    /// `State.innate_min_draw` — the number of Innate cards moved to the
    /// draw-pile top during combat setup, and therefore the turn-one minimum
    /// hand draw.
    pub innate_min_draw: u8,
    /// `State.turn`.
    pub turn: i16,
    /// `State.inky_attack_damage`.
    pub inky_attack_damage: i16,
    /// `State.regalite_block_amount`.
    pub regalite_block_amount: u8,
    /// `State.temp_strength` — the owner's positive
    /// `TemporaryStrengthPower` contribution, removed at owner side end.
    pub temp_strength: i32,
    /// `State.player_phase` — the `PlayerCombatState.Phase` projection.
    pub player_phase: u8,
    /// `State.exact_piles` — the fight has been promoted to **exact** pile
    /// identity (`_commit_live_card_piles`, frozen Python, deleted #2827).
    ///
    /// Off, a pile is a multiset of payloads and `legal_actions` deduplicates
    /// plays by payload alone. On, the pile is an ordered list of physical
    /// objects and the dedup key becomes the played payload *plus the ordered
    /// payloads of the rest of the hand* — so two equal cards separated by a
    /// third stop collapsing into one action. It is a one-way promotion: every
    /// writer sets it, nothing clears it.
    pub exact_piles: bool,
    /// `State.player_side_active` — which side `CombatManager` has switched
    /// to. True through the whole player side, false from the monster side's
    /// start (`_finish_side_switch_after_disintegration`, frozen Python, deleted #2827) until the
    /// next `begin_player_turn`. Only observable at a terminal reached inside the enemy
    /// phase, which is exactly where the differential found it missing.
    pub player_side_active: bool,
    /// Compact card-affliction and deep damage-path flags. Bit zero is
    /// Ringing; bits one and two are Chains of Binding's successful-affliction
    /// count; bit three is its ordinary-Bound-card-play latch. Bits four
    /// through seven are immutable Hand Drill, Snecko Skull, The Boot, and
    /// Tungsten Rod ownership flags. Those four readers avoid plumbing the
    /// cold catalog through every nested damage and debuff command.
    card_affliction_state: u8,
    /// Compact mutable state for the Batch-5 counter relics. Keeping this
    /// inline makes their turn hooks allocation-free; narrowing the closed
    /// multiplayer key below from `u32` to `u8` recovers the same four-byte
    /// lane, so `HotState` remains at its reviewed 224-byte ceiling.
    relic_state: u32,
    /// Compact mutable state for Batch-6 relics. Narrowing
    /// `innate_min_draw` to its physically reachable byte domain pays for
    /// this lane without increasing the 224-byte search state.
    batch_six_relic_state: u32,
    /// `State.reward_card_pool`.
    pub reward_card_pool: Option<RewardPool>,
    /// `State.entropy_card_pool` — the lowercase owner-pool provenance
    /// published by explicitly fully-unlocked combat construction.
    pub entropy_card_pool: Option<RewardPool>,
    /// Compact generated-power state: exact Hello World pool provenance and
    /// closed turn-start snapshot delta, the live persistent Calamity callback,
    /// and Entropy's three-bit insertion ordinal in the unified turn-start
    /// order. Bit seven remains forbidden.
    generated_power_flags: u8,
    /// Whether the exact fully-unlocked character + Colorless epoch set is
    /// published on `State.splash_unlock_epochs`.
    pub fully_unlocked_card_pool_epochs: bool,
    /// `State.reward_card_rarity_odds`.
    pub reward_card_rarity_odds: Option<RewardOdds>,
    /// The one exact default-live remote Player admitted by the synthesized
    /// multiplayer entry vehicle. The key is stable; no teammate turns,
    /// choices, piles, resources, pets, or callbacks are inferred. Zero is
    /// the absent sentinel; the synthesized teammate's nonzero key fits in
    /// existing `HotState` padding, preserving the measured 224-byte layout.
    pub multiplayer_ally_key: u8,
    /// The play-history counters the engine maintains.
    pub history: HotHistory,
}

impl HotState {
    /// Recover an absent legacy reset-order witness only where scalar and
    /// subgroup facts determine it. It stays wire-implicit until a later
    /// attachment makes an otherwise-unrecoverable relative order observable.
    /// This keeps warm mutation and cold hydration observationally identical
    /// without inventing cross-family history or changing old root digests.
    pub(crate) fn normalize_after_energy_reset_order_if_unique(&mut self) {
        if self.fanouts.after_energy_reset_order().is_some() {
            return;
        }
        let candidates = [
            AfterEnergyResetPower::Genesis,
            AfterEnergyResetPower::StarNextTurn,
            AfterEnergyResetPower::EnergyNextTurn,
            AfterEnergyResetPower::Radiance,
            AfterEnergyResetPower::LightningRod,
            AfterEnergyResetPower::Spinner,
        ];
        let is_live = |power| match power {
            AfterEnergyResetPower::Radiance => self.fanouts.radiance() > 0,
            AfterEnergyResetPower::Genesis => self.powers.value(PowerId::Genesis) > 0,
            AfterEnergyResetPower::StarNextTurn => self.powers.value(PowerId::StarNextTurn) > 0,
            AfterEnergyResetPower::EnergyNextTurn => self.powers.value(PowerId::EnergyNextTurn) > 0,
            AfterEnergyResetPower::LightningRod => self.powers.value(PowerId::LightningRod) > 0,
            AfterEnergyResetPower::Spinner => self.powers.value(PowerId::Spinner) > 0,
        };
        // The common legacy-empty path runs on ordinary turn boundaries. Do
        // not allocate a temporary ledger merely to discover that there is no
        // listener to normalize.
        if candidates.iter().filter(|power| is_live(**power)).count() == 0 {
            return;
        }
        let live = candidates
            .into_iter()
            .filter(|power| is_live(*power))
            .collect::<Vec<_>>();
        let recovered = if live.len() == 1 {
            Some(live)
        } else if !live.is_empty()
            && live.iter().all(|power| {
                matches!(
                    power,
                    AfterEnergyResetPower::Genesis | AfterEnergyResetPower::StarNextTurn
                )
            })
        {
            let order = self
                .fanouts
                .star_energy_reset_order()
                .iter()
                .map(|power| {
                    AfterEnergyResetPower::from_power(*power)
                        .expect("star reset projection is closed")
                })
                .collect::<Vec<_>>();
            (order.len() == live.len() && order.iter().all(|power| live.contains(power)))
                .then_some(order)
        } else if !live.is_empty()
            && live.iter().all(|power| {
                matches!(
                    power,
                    AfterEnergyResetPower::LightningRod | AfterEnergyResetPower::Spinner
                )
            })
        {
            let order = self
                .orbs
                .reset_order()
                .iter()
                .map(|power| match power {
                    OrbResetPower::LightningRod => AfterEnergyResetPower::LightningRod,
                    OrbResetPower::Spinner => AfterEnergyResetPower::Spinner,
                })
                .collect::<Vec<_>>();
            (order.len() == live.len() && order.iter().all(|power| live.contains(power)))
                .then_some(order)
        } else {
            None
        };
        if let Some(order) = recovered {
            let accepted = self.fanouts.set_after_energy_reset_order(&order);
            debug_assert!(accepted);
            self.fanouts.mark_after_energy_reset_order_legacy_inferred();
        }
    }

    fn relic_counter(&self, mask: u32, shift: u32) -> i16 {
        let encoded = ((self.relic_state & mask) >> shift) as i16;
        encoded - 1
    }

    fn set_relic_counter(&mut self, mask: u32, shift: u32, value: i16, maximum: i16) -> bool {
        if value < -1 || value > maximum {
            return false;
        }
        let encoded = u32::try_from(value + 1).expect("nonnegative relic counter encoding");
        self.relic_state = (self.relic_state & !mask) | (encoded << shift);
        true
    }

    pub(crate) fn relic_state_is_exact(&self) -> bool {
        self.flower() <= 2
            && self.fake_flower() <= 4
            && self.pendulum() <= 2
            && self.pollinous_core() <= 3
            && self.ember_tea_combats_left() <= 5
    }

    pub(crate) fn flower(&self) -> i16 {
        self.relic_counter(FLOWER_MASK, FLOWER_SHIFT)
    }

    pub(crate) fn set_flower(&mut self, value: i16) -> bool {
        self.set_relic_counter(FLOWER_MASK, FLOWER_SHIFT, value, 2)
    }

    pub(crate) fn fake_flower(&self) -> i16 {
        self.relic_counter(FAKE_FLOWER_MASK, FAKE_FLOWER_SHIFT)
    }

    pub(crate) fn set_fake_flower(&mut self, value: i16) -> bool {
        self.set_relic_counter(FAKE_FLOWER_MASK, FAKE_FLOWER_SHIFT, value, 4)
    }

    pub(crate) fn pendulum(&self) -> i16 {
        self.relic_counter(PENDULUM_MASK, PENDULUM_SHIFT)
    }

    pub(crate) fn set_pendulum(&mut self, value: i16) -> bool {
        self.set_relic_counter(PENDULUM_MASK, PENDULUM_SHIFT, value, 2)
    }

    pub(crate) fn pollinous_core(&self) -> i16 {
        self.relic_counter(POLLINOUS_CORE_MASK, POLLINOUS_CORE_SHIFT)
    }

    pub(crate) fn set_pollinous_core(&mut self, value: i16) -> bool {
        self.set_relic_counter(POLLINOUS_CORE_MASK, POLLINOUS_CORE_SHIFT, value, 3)
    }

    pub(crate) fn ember_tea_combats_left(&self) -> i16 {
        self.relic_counter(EMBER_TEA_MASK, EMBER_TEA_SHIFT)
    }

    /// Ember Tea's saved `CombatsLeft`, `-1` (unowned) or `0..=5`.
    ///
    /// The domain is native's (v0.111.0 `sts2.dll` SHA-256 `9cb4f1ad…12b4`,
    /// #3381): `EmberTea::.ctor` (RVA `0x92fdf`) stores `ldc.i4.5` into
    /// `_combatsLeft` at IL_0002-IL_0003, and the one in-assembly writer
    /// besides it, `set_CombatsLeft` (`0x92f44`, IL_0014), is called only by
    /// `<AfterRoomEntered>d__17::MoveNext` (`0x3239cc`) with `CombatsLeft - 1`
    /// (IL_00bd-IL_00c8), after the `IsUsedUp` (`CombatsLeft > 0` false,
    /// `0x92eca`) early return at IL_001e-IL_0025. So the counter starts at 5,
    /// only decrements, and never goes below 0. The untouched charge 5 is what
    /// a save taken before the relic's first combat records, and the pre-hook
    /// entry document carries it; the three-bit field encodes `value + 1` up to
    /// 7, so 5 (encoded 6) fits. It was capped at 4 before #3381, which refused
    /// every fight entered with a fresh Ember Tea.
    pub(crate) fn set_ember_tea_combats_left(&mut self, value: i16) -> bool {
        self.set_relic_counter(EMBER_TEA_MASK, EMBER_TEA_SHIFT, value, 5)
    }

    pub(crate) fn art_of_war_current_attack(&self) -> bool {
        self.relic_state & ART_OF_WAR_CURRENT_MASK != 0
    }

    pub(crate) fn set_art_of_war_current_attack(&mut self, value: bool) {
        if value {
            self.relic_state |= ART_OF_WAR_CURRENT_MASK;
        } else {
            self.relic_state &= !ART_OF_WAR_CURRENT_MASK;
        }
    }

    pub(crate) fn art_of_war_last_attack(&self) -> bool {
        self.relic_state & ART_OF_WAR_LAST_MASK != 0
    }

    pub(crate) fn set_art_of_war_last_attack(&mut self, value: bool) {
        if value {
            self.relic_state |= ART_OF_WAR_LAST_MASK;
        } else {
            self.relic_state &= !ART_OF_WAR_LAST_MASK;
        }
    }

    pub(crate) fn paels_tears_had_leftover_energy(&self) -> bool {
        self.relic_state & PAELS_TEARS_LEFTOVER_MASK != 0
    }

    pub(crate) fn set_paels_tears_had_leftover_energy(&mut self, value: bool) {
        if value {
            self.relic_state |= PAELS_TEARS_LEFTOVER_MASK;
        } else {
            self.relic_state &= !PAELS_TEARS_LEFTOVER_MASK;
        }
    }

    pub(crate) fn pocketwatch_last_plays(&self) -> i16 {
        ((self.relic_state & POCKETWATCH_LAST_MASK) >> POCKETWATCH_LAST_SHIFT) as i16
    }

    pub(crate) fn set_pocketwatch_last_plays(&mut self, value: i16) -> bool {
        if value < 0 {
            return false;
        }
        self.relic_state = (self.relic_state & !POCKETWATCH_LAST_MASK)
            | (u32::from(value as u16) << POCKETWATCH_LAST_SHIFT);
        true
    }

    pub(crate) fn paper_krane_owned(&self) -> bool {
        self.relic_state & PAPER_KRANE_OWNED_MASK != 0
    }

    fn batch_six_relic_flag(&self, mask: u32) -> bool {
        self.batch_six_relic_state & mask != 0
    }

    fn set_batch_six_relic_flag(&mut self, mask: u32, value: bool) {
        if value {
            self.batch_six_relic_state |= mask;
        } else {
            self.batch_six_relic_state &= !mask;
        }
    }

    pub(crate) fn booming_conch_elite(&self) -> bool {
        self.batch_six_relic_flag(BOOMING_CONCH_ELITE_MASK)
    }

    pub(crate) fn set_booming_conch_elite(&mut self, value: bool) {
        self.set_batch_six_relic_flag(BOOMING_CONCH_ELITE_MASK, value);
    }

    pub(crate) fn demon_tongue_triggered(&self) -> bool {
        self.batch_six_relic_flag(DEMON_TONGUE_TRIGGERED_MASK)
    }

    pub(crate) fn set_demon_tongue_triggered(&mut self, value: bool) {
        self.set_batch_six_relic_flag(DEMON_TONGUE_TRIGGERED_MASK, value);
    }

    pub(crate) fn emotion_damage_current_turn(&self) -> bool {
        self.batch_six_relic_flag(EMOTION_DAMAGE_CURRENT_MASK)
    }

    pub(crate) fn set_emotion_damage_current_turn(&mut self, value: bool) {
        self.set_batch_six_relic_flag(EMOTION_DAMAGE_CURRENT_MASK, value);
    }

    pub(crate) fn emotion_damage_previous_turn(&self) -> bool {
        self.batch_six_relic_flag(EMOTION_DAMAGE_PREVIOUS_MASK)
    }

    pub(crate) fn set_emotion_damage_previous_turn(&mut self, value: bool) {
        self.set_batch_six_relic_flag(EMOTION_DAMAGE_PREVIOUS_MASK, value);
    }

    pub(crate) fn fake_tea_set_charged(&self) -> bool {
        self.batch_six_relic_flag(FAKE_TEA_SET_CHARGED_MASK)
    }

    pub(crate) fn set_fake_tea_set_charged(&mut self, value: bool) {
        self.set_batch_six_relic_flag(FAKE_TEA_SET_CHARGED_MASK, value);
    }

    pub(crate) fn fur_coat_active(&self) -> bool {
        self.batch_six_relic_flag(FUR_COAT_ACTIVE_MASK)
    }

    pub(crate) fn set_fur_coat_active(&mut self, value: bool) {
        self.set_batch_six_relic_flag(FUR_COAT_ACTIVE_MASK, value);
    }

    pub(crate) fn permafrost_armed(&self) -> bool {
        self.batch_six_relic_flag(PERMAFROST_ARMED_MASK)
    }

    pub(crate) fn set_permafrost_armed(&mut self, value: bool) {
        self.set_batch_six_relic_flag(PERMAFROST_ARMED_MASK, value);
    }

    pub(crate) fn tea_set_charged(&self) -> bool {
        self.batch_six_relic_flag(TEA_SET_CHARGED_MASK)
    }

    pub(crate) fn set_tea_set_charged(&mut self, value: bool) {
        self.set_batch_six_relic_flag(TEA_SET_CHARGED_MASK, value);
    }

    pub(crate) fn demon_tongue_owned(&self) -> bool {
        self.batch_six_relic_flag(DEMON_TONGUE_OWNED_MASK)
    }

    pub(crate) fn emotion_chip_owned(&self) -> bool {
        self.batch_six_relic_flag(EMOTION_CHIP_OWNED_MASK)
    }

    pub(crate) fn book_repair_knife_owned(&self) -> bool {
        self.batch_six_relic_flag(BOOK_REPAIR_KNIFE_OWNED_MASK)
    }

    pub(crate) fn velvet_choker_owned(&self) -> bool {
        self.batch_six_relic_flag(VELVET_CHOKER_OWNED_MASK)
    }

    pub fn spectrum_shift_generation_pool(&self) -> bool {
        self.batch_six_relic_flag(SPECTRUM_SHIFT_POOL_MASK)
    }

    pub fn set_spectrum_shift_generation_pool(&mut self, value: bool) {
        self.set_batch_six_relic_flag(SPECTRUM_SHIFT_POOL_MASK, value);
    }

    pub fn creative_ai_generation_pool(&self) -> bool {
        self.batch_six_relic_flag(CREATIVE_AI_POOL_MASK)
    }

    pub fn set_creative_ai_generation_pool(&mut self, value: bool) {
        self.set_batch_six_relic_flag(CREATIVE_AI_POOL_MASK, value);
    }

    fn packed_batch_seven_counter(&self, mask: u32, shift: u32) -> u8 {
        ((self.batch_six_relic_state & mask) >> shift) as u8
    }

    fn set_packed_batch_seven_counter(
        &mut self,
        value: u8,
        maximum: u8,
        mask: u32,
        shift: u32,
    ) -> bool {
        if value > maximum {
            return false;
        }
        self.batch_six_relic_state =
            (self.batch_six_relic_state & !mask) | (u32::from(value) << shift);
        true
    }

    pub(crate) fn nunchaku(&self) -> u8 {
        self.packed_batch_seven_counter(NUNCHAKU_MASK, NUNCHAKU_SHIFT)
    }

    pub(crate) fn set_nunchaku(&mut self, value: u8) -> bool {
        self.set_packed_batch_seven_counter(value, 9, NUNCHAKU_MASK, NUNCHAKU_SHIFT)
    }

    pub(crate) fn kunai(&self) -> u8 {
        self.packed_batch_seven_counter(KUNAI_MASK, KUNAI_SHIFT)
    }

    pub(crate) fn set_kunai(&mut self, value: u8) -> bool {
        self.set_packed_batch_seven_counter(value, 2, KUNAI_MASK, KUNAI_SHIFT)
    }

    pub(crate) fn shuriken(&self) -> u8 {
        self.packed_batch_seven_counter(SHURIKEN_MASK, SHURIKEN_SHIFT)
    }

    pub(crate) fn set_shuriken(&mut self, value: u8) -> bool {
        self.set_packed_batch_seven_counter(value, 2, SHURIKEN_MASK, SHURIKEN_SHIFT)
    }

    pub(crate) fn ornamental_fan(&self) -> u8 {
        self.packed_batch_seven_counter(ORNAMENTAL_FAN_MASK, ORNAMENTAL_FAN_SHIFT)
    }

    pub(crate) fn set_ornamental_fan(&mut self, value: u8) -> bool {
        self.set_packed_batch_seven_counter(value, 2, ORNAMENTAL_FAN_MASK, ORNAMENTAL_FAN_SHIFT)
    }

    pub(crate) fn rainbow_ring_types_this_turn(&self) -> u8 {
        self.packed_batch_seven_counter(RAINBOW_RING_MASK, RAINBOW_RING_SHIFT)
    }

    pub(crate) fn set_rainbow_ring_types_this_turn(&mut self, value: u8) -> bool {
        self.set_packed_batch_seven_counter(value, 7, RAINBOW_RING_MASK, RAINBOW_RING_SHIFT)
    }

    pub(crate) fn metronome(&self) -> u8 {
        self.packed_batch_seven_counter(METRONOME_MASK, METRONOME_SHIFT)
    }

    pub(crate) fn set_metronome(&mut self, value: u8) -> bool {
        self.set_packed_batch_seven_counter(value, 7, METRONOME_MASK, METRONOME_SHIFT)
    }

    pub(crate) fn tea_of_discourtesy_active(&self) -> bool {
        self.batch_six_relic_flag(TEA_OF_DISCOURTESY_ACTIVE_MASK)
    }

    pub(crate) fn set_tea_of_discourtesy_active(&mut self, value: bool) {
        self.set_batch_six_relic_flag(TEA_OF_DISCOURTESY_ACTIVE_MASK, value);
    }

    pub(crate) fn philosophers_stone_owned(&self) -> bool {
        self.batch_six_relic_flag(PHILOSOPHERS_STONE_OWNED_MASK)
    }

    pub(crate) fn set_philosophers_stone_owned(&mut self, value: bool) {
        self.set_batch_six_relic_flag(PHILOSOPHERS_STONE_OWNED_MASK, value);
    }

    pub(crate) fn set_batch_six_deep_relic_ownership(
        &mut self,
        demon_tongue: bool,
        emotion_chip: bool,
        book_repair_knife: bool,
        velvet_choker: bool,
    ) {
        self.set_batch_six_relic_flag(DEMON_TONGUE_OWNED_MASK, demon_tongue);
        self.set_batch_six_relic_flag(EMOTION_CHIP_OWNED_MASK, emotion_chip);
        self.set_batch_six_relic_flag(BOOK_REPAIR_KNIFE_OWNED_MASK, book_repair_knife);
        self.set_batch_six_relic_flag(VELVET_CHOKER_OWNED_MASK, velvet_choker);
    }

    pub(crate) fn set_paper_krane_owned(&mut self, value: bool) {
        if value {
            self.relic_state |= PAPER_KRANE_OWNED_MASK;
        } else {
            self.relic_state &= !PAPER_KRANE_OWNED_MASK;
        }
    }

    pub(crate) fn card_affliction_state_is_exact(&self) -> bool {
        true
    }

    #[cfg(test)]
    pub(crate) fn forge_card_affliction_state_for_test(&mut self, raw: u8) {
        self.card_affliction_state = raw;
    }

    pub(crate) fn ringing(&self) -> bool {
        self.card_affliction_state & RINGING_LIVE_MASK != 0
    }

    pub(crate) fn set_ringing(&mut self, live: bool) {
        if live {
            self.card_affliction_state |= RINGING_LIVE_MASK;
        } else {
            self.card_affliction_state &= !RINGING_LIVE_MASK;
        }
    }

    pub(crate) fn bound_afflictions_this_turn(&self) -> u8 {
        (self.card_affliction_state & BOUND_AFFLICTIONS_MASK) >> BOUND_AFFLICTIONS_SHIFT
    }

    pub(crate) fn set_bound_afflictions_this_turn(&mut self, count: u8) -> bool {
        if count > 3 {
            return false;
        }
        self.card_affliction_state = (self.card_affliction_state & !BOUND_AFFLICTIONS_MASK)
            | (count << BOUND_AFFLICTIONS_SHIFT);
        true
    }

    pub(crate) fn bound_card_played(&self) -> bool {
        self.card_affliction_state & BOUND_CARD_PLAYED_MASK != 0
    }

    pub(crate) fn set_bound_card_played(&mut self, played: bool) {
        if played {
            self.card_affliction_state |= BOUND_CARD_PLAYED_MASK;
        } else {
            self.card_affliction_state &= !BOUND_CARD_PLAYED_MASK;
        }
    }

    pub(crate) fn hand_drill_owned(&self) -> bool {
        self.card_affliction_state & HAND_DRILL_OWNED_MASK != 0
    }

    pub(crate) fn snecko_skull_owned(&self) -> bool {
        self.card_affliction_state & SNECKO_SKULL_OWNED_MASK != 0
    }

    pub(crate) fn the_boot_owned(&self) -> bool {
        self.card_affliction_state & THE_BOOT_OWNED_MASK != 0
    }

    pub(crate) fn tungsten_rod_owned(&self) -> bool {
        self.card_affliction_state & TUNGSTEN_ROD_OWNED_MASK != 0
    }

    pub(crate) fn set_deep_relic_ownership(
        &mut self,
        hand_drill: bool,
        snecko_skull: bool,
        the_boot: bool,
        tungsten_rod: bool,
    ) {
        let flags = (u8::from(hand_drill) * HAND_DRILL_OWNED_MASK)
            | (u8::from(snecko_skull) * SNECKO_SKULL_OWNED_MASK)
            | (u8::from(the_boot) * THE_BOOT_OWNED_MASK)
            | (u8::from(tungsten_rod) * TUNGSTEN_ROD_OWNED_MASK);
        self.card_affliction_state = (self.card_affliction_state & 0b0000_1111) | flags;
    }

    pub(crate) fn generated_power_flags_are_exact(&self) -> bool {
        if self.generated_power_flags & !GENERATED_POWER_FLAGS_MASK != 0
            || self.fanouts.0.cold.hello_world_snapshot_extra_delta < 0
            || (self.hello_world_snapshot_is_current()
                && self.fanouts.0.cold.hello_world_snapshot_extra_delta != 0)
        {
            return false;
        }
        let ordinal = usize::from(
            (self.generated_power_flags & ENTROPY_TURN_START_ORDINAL_MASK)
                >> ENTROPY_TURN_START_ORDINAL_SHIFT,
        );
        if self.powers.value(PowerId::Entropy) > 0 {
            self.fanouts.after_player_turn_start_order_is_exact()
                && ordinal <= self.fanouts.stored_after_player_turn_start_order().len()
        } else {
            ordinal == 0
        }
    }

    /// Reconstruct native's complete eight-token `AfterPlayerTurnStart`
    /// player-power order. Seven non-Entropy ids occupy the existing fanout
    /// tail; Entropy's 0..=7 insertion ordinal occupies bits 4..=6 of the
    /// pre-existing generated-power byte. Its live power distinguishes an
    /// absent listener from ordinal zero.
    pub(crate) fn after_player_turn_start_order(&self) -> Option<AfterPlayerTurnStartOrder> {
        if !self.fanouts.after_player_turn_start_order_is_exact()
            || !self.generated_power_flags_are_exact()
        {
            return None;
        }
        let stored = self.fanouts.stored_after_player_turn_start_order();
        // Most ordinary fights have no player power at all. The validation
        // above remains mandatory; once it succeeds, avoid rebuilding the
        // empty fixed-width order.
        if stored.is_empty() && self.powers.is_empty() {
            return Some(AfterPlayerTurnStartOrder {
                powers: [PowerId::Accuracy; MAX_AFTER_PLAYER_TURN_START_POWERS],
                len: 0,
            });
        }
        let entropy_live = self.powers.value(PowerId::Entropy) > 0;
        let entropy_ordinal = usize::from(
            (self.generated_power_flags & ENTROPY_TURN_START_ORDINAL_MASK)
                >> ENTROPY_TURN_START_ORDINAL_SHIFT,
        );
        if entropy_live && entropy_ordinal > stored.len() {
            return None;
        }
        let mut powers = [PowerId::Accuracy; MAX_AFTER_PLAYER_TURN_START_POWERS];
        let mut cursor = 0;
        for (index, power) in stored.iter().copied().enumerate() {
            if entropy_live && index == entropy_ordinal {
                powers[cursor] = PowerId::Entropy;
                cursor += 1;
            }
            powers[cursor] = power;
            cursor += 1;
        }
        if entropy_live && entropy_ordinal == stored.len() {
            powers[cursor] = PowerId::Entropy;
            cursor += 1;
        }
        Some(AfterPlayerTurnStartOrder {
            powers,
            len: cursor as u8,
        })
    }

    fn after_side_turn_start_token_is_live(&self, token: AfterSideTurnStartToken) -> bool {
        match token.power() {
            Some(power) => self.powers.value(power) > 0,
            None => self.fanouts.clarity() > 0,
        }
    }

    /// Reconstruct the complete native first-application order. Every live
    /// concrete member must occur exactly once and no inactive member may be
    /// retained.
    pub(crate) fn after_side_turn_start_power_order(&self) -> Option<AfterSideTurnStartOrder> {
        let stored = self.fanouts.after_side_turn_start_order();
        // The overwhelming ordinary-state case has no player powers and no
        // Clarity potion effect. It is exact by construction: Slots omits
        // zero-valued powers, and the empty stored order therefore accounts
        // for every member of this listener family. Avoid re-checking all
        // eighteen possible tokens on every turn boundary of an unpowered
        // combat; the full validation below remains the path for every state
        // that can carry a relevant or unrelated player power.
        if stored.is_empty() && self.powers.is_empty() && self.fanouts.clarity() == 0 {
            return Some(AfterSideTurnStartOrder {
                powers: [AfterSideTurnStartToken::BiasedCognition;
                    MAX_AFTER_SIDE_TURN_START_POWERS],
                len: 0,
            });
        }
        if stored.len() > MAX_AFTER_SIDE_TURN_START_POWERS
            || stored.iter().enumerate().any(|(index, token)| {
                stored[..index].contains(token) || !self.after_side_turn_start_token_is_live(*token)
            })
            || AfterSideTurnStartToken::ALL.iter().any(|token| {
                self.after_side_turn_start_token_is_live(*token) != stored.contains(token)
            })
        {
            return None;
        }
        let mut powers =
            [AfterSideTurnStartToken::BiasedCognition; MAX_AFTER_SIDE_TURN_START_POWERS];
        powers[..stored.len()].copy_from_slice(stored);
        Some(AfterSideTurnStartOrder {
            powers,
            len: stored.len() as u8,
        })
    }

    /// Replace the complete authenticated order. Restacks use registration
    /// methods and therefore never call this replacement path.
    pub(crate) fn set_after_side_turn_start_power_order(
        &mut self,
        order: &[AfterSideTurnStartToken],
    ) -> bool {
        if order.len() > MAX_AFTER_SIDE_TURN_START_POWERS
            || order
                .iter()
                .enumerate()
                .any(|(index, token)| order[..index].contains(token))
            || AfterSideTurnStartToken::ALL.iter().any(|token| {
                self.after_side_turn_start_token_is_live(*token) != order.contains(token)
            })
        {
            return false;
        }
        self.fanouts.set_after_side_turn_start_order(order)
    }

    /// Reconstruct the complete native first-application order for the
    /// closed Flame Barrier / Reflect / The Gambit listener family.
    pub(crate) fn after_damage_received_power_order(
        &self,
    ) -> Option<AfterDamageReceivedPowerOrder> {
        let mut live = [AfterDamageReceivedPower::FlameBarrier; MAX_AFTER_DAMAGE_RECEIVED_POWERS];
        let mut len = 0;
        for token in AfterDamageReceivedPower::ALL {
            if self.powers.value(token.power()) > 0 {
                live[len] = token;
                len += 1;
            }
        }
        decode_after_damage_received_order(
            &live[..len],
            self.fanouts.after_damage_received_order_code(),
        )
    }

    pub(crate) fn after_damage_received_order_is_reachable(&self) -> bool {
        self.fanouts.after_damage_received_order_code() != 0
            || AfterDamageReceivedPower::ALL
                .into_iter()
                .any(|token| self.powers.get(token.power()).is_some())
    }

    /// Replace the complete authenticated order at a canonical boundary.
    pub(crate) fn set_after_damage_received_power_order(
        &mut self,
        order: &[AfterDamageReceivedPower],
    ) -> bool {
        for token in AfterDamageReceivedPower::ALL {
            if order.contains(&token) != (self.powers.value(token.power()) > 0) {
                return false;
            }
        }
        let Some(code) = encode_after_damage_received_order(order) else {
            return false;
        };
        self.fanouts.set_after_damage_received_order_code(code);
        true
    }

    /// Record a new native power instance immediately before its scalar is
    /// published. Existing instances are restacks and retain their position.
    pub(crate) fn register_after_damage_received_power(&mut self, power: PowerId) -> bool {
        let Some(token) = AfterDamageReceivedPower::from_power(power) else {
            return false;
        };
        if let Some(slot) = self.powers.get(power) {
            return slot.value > 0 && self.after_damage_received_power_order().is_some();
        }
        let Some(current) = self.after_damage_received_power_order() else {
            return false;
        };
        let mut order = [AfterDamageReceivedPower::FlameBarrier; MAX_AFTER_DAMAGE_RECEIVED_POWERS];
        let len = current.len();
        order[..len].copy_from_slice(&current);
        order[len] = token;
        let Some(code) = encode_after_damage_received_order(&order[..=len]) else {
            return false;
        };
        self.fanouts.set_after_damage_received_order_code(code);
        true
    }

    /// Remove one live listener from the stored order before its scalar is
    /// cleared. A later first application therefore appends at the tail.
    pub(crate) fn unregister_after_damage_received_power(&mut self, power: PowerId) -> bool {
        let Some(token) = AfterDamageReceivedPower::from_power(power) else {
            return false;
        };
        let Some(current) = self.after_damage_received_power_order() else {
            return false;
        };
        let Some(index) = current.iter().position(|candidate| *candidate == token) else {
            return self.powers.value(power) <= 0;
        };
        let mut order = [AfterDamageReceivedPower::FlameBarrier; MAX_AFTER_DAMAGE_RECEIVED_POWERS];
        let mut len = 0;
        for candidate in current.iter().copied() {
            if candidate != token {
                order[len] = candidate;
                len += 1;
            }
        }
        debug_assert_eq!(current.len() - 1, len);
        debug_assert!(index < current.len());
        let Some(code) = encode_after_damage_received_order(&order[..len]) else {
            return false;
        };
        self.fanouts.set_after_damage_received_order_code(code);
        true
    }

    pub(crate) fn set_after_player_turn_start_order(&mut self, order: &[PowerId]) -> bool {
        if order.len() > MAX_AFTER_PLAYER_TURN_START_POWERS
            || order.iter().enumerate().any(|(index, power)| {
                TurnStartHandChoiceKind::from_power(*power).is_none()
                    || order[..index].contains(power)
            })
        {
            return false;
        }
        let entropy = order.iter().position(|power| *power == PowerId::Entropy);
        let stored = order
            .iter()
            .copied()
            .filter(|power| *power != PowerId::Entropy)
            .collect::<Vec<_>>();
        if stored.len() > MAX_STORED_AFTER_PLAYER_TURN_START_POWERS
            || !self.fanouts.set_after_player_turn_start_order(&stored)
        {
            return false;
        }
        self.generated_power_flags &= !ENTROPY_TURN_START_ORDINAL_MASK;
        if let Some(ordinal) = entropy {
            self.generated_power_flags |= (ordinal as u8) << ENTROPY_TURN_START_ORDINAL_SHIFT;
        }
        true
    }

    pub(crate) fn register_after_player_turn_start(&mut self, power: PowerId) -> bool {
        if TurnStartHandChoiceKind::from_power(power).is_none() {
            return false;
        }
        let Some(current) = self.after_player_turn_start_order() else {
            return false;
        };
        if current.contains(&power) {
            return true;
        }
        if current.len() == MAX_AFTER_PLAYER_TURN_START_POWERS {
            return false;
        }
        let mut order = [PowerId::Accuracy; MAX_AFTER_PLAYER_TURN_START_POWERS];
        order[..current.len()].copy_from_slice(&current);
        order[current.len()] = power;
        if self
            .frames
            .record_turn_start_hand_choice_sly_append(power)
            .is_none()
        {
            return false;
        }
        self.set_after_player_turn_start_order(&order[..current.len() + 1])
    }

    pub(crate) fn unregister_after_player_turn_start(&mut self, power: PowerId) -> bool {
        let Some(current) = self.after_player_turn_start_order() else {
            return false;
        };
        let Some(index) = current.iter().position(|candidate| *candidate == power) else {
            return true;
        };
        let mut order = [PowerId::Accuracy; MAX_AFTER_PLAYER_TURN_START_POWERS];
        order[..current.len()].copy_from_slice(&current);
        order.copy_within(index + 1..current.len(), index);
        self.set_after_player_turn_start_order(&order[..current.len() - 1])
    }

    pub(crate) fn hello_world_generation_pool_is_exact(&self) -> bool {
        self.generated_power_flags & HELLO_WORLD_GENERATION_POOL_MASK != 0
    }

    pub(crate) fn publish_hello_world_generation_pool(&mut self) {
        self.generated_power_flags |= HELLO_WORLD_GENERATION_POOL_MASK;
    }

    /// Recover the exact native HelloWorldPower.AmountOnTurnStart.
    /// Creature.BeforeTurnStart (v111 RVA 0x11dbe0 IL0024–0029) snapshots
    /// the amount; PowerModel's independent field getter/setter are
    /// 0x838cb/0x838d3. HelloWorldPower.BeforeHandDraw 0x33bfd0 reads that
    /// snapshot, and mid-turn applications change only the live amount.
    /// Delta zero/one stay inline; larger deltas spill behind the existing
    /// cold COW handle, preserving HotState and FanoutState byte ceilings.
    pub(crate) fn hello_world_amount_on_turn_start(&self, current: i32) -> Option<i32> {
        let extra = self.fanouts.0.cold.hello_world_snapshot_extra_delta;
        let dirty = self.generated_power_flags & HELLO_WORLD_SNAPSHOT_DIRTY_MASK != 0;
        if extra < 0 || (!dirty && extra != 0) {
            return None;
        }
        let delta = i32::from(dirty).checked_add(extra)?;
        current.checked_sub(delta).filter(|snapshot| *snapshot >= 0)
    }

    pub(crate) fn set_hello_world_amount_on_turn_start(
        &mut self,
        current: i32,
        snapshot: i32,
    ) -> bool {
        let Some(delta) = current
            .checked_sub(snapshot)
            .filter(|delta| *delta >= 0 && snapshot >= 0)
        else {
            return false;
        };
        let extra = (delta - 1).max(0);
        if self.fanouts.0.cold.hello_world_snapshot_extra_delta != extra {
            Arc::make_mut(&mut Arc::make_mut(&mut self.fanouts.0).cold)
                .hello_world_snapshot_extra_delta = extra;
        }
        if delta > 0 {
            self.generated_power_flags |= HELLO_WORLD_SNAPSHOT_DIRTY_MASK;
        } else {
            self.generated_power_flags &= !HELLO_WORLD_SNAPSHOT_DIRTY_MASK;
        }
        true
    }

    pub(crate) fn hello_world_snapshot_is_current(&self) -> bool {
        self.generated_power_flags & HELLO_WORLD_SNAPSHOT_DIRTY_MASK == 0
    }

    pub(crate) fn freeze_hello_world_amount_on_turn_start(&mut self) {
        if self.fanouts.0.cold.hello_world_snapshot_extra_delta != 0 {
            Arc::make_mut(&mut Arc::make_mut(&mut self.fanouts.0).cold)
                .hello_world_snapshot_extra_delta = 0;
        }
        self.generated_power_flags &= !HELLO_WORLD_SNAPSHOT_DIRTY_MASK;
    }

    pub(crate) fn calamity_hook_is_live(&self) -> bool {
        self.generated_power_flags & CALAMITY_HOOK_LIVE_MASK != 0
    }

    pub(crate) fn call_of_the_void_generation_pool_is_exact(&self) -> bool {
        self.generated_power_flags & CALL_OF_THE_VOID_GENERATION_POOL_MASK != 0
    }

    pub(crate) fn publish_call_of_the_void_generation_pool(&mut self) {
        self.generated_power_flags |= CALL_OF_THE_VOID_GENERATION_POOL_MASK;
    }

    pub(crate) fn set_calamity_hook_live(&mut self, live: bool) {
        if live {
            self.generated_power_flags |= CALAMITY_HOOK_LIVE_MASK;
        } else {
            self.generated_power_flags &= !CALAMITY_HOOK_LIVE_MASK;
        }
    }

    /// Install one independently rooted replay beneath the complete current
    /// parked stack, rebasing the separate pending CardPlay index atomically.
    #[cold]
    #[inline(never)]
    pub(crate) fn install_action_replay_root(&mut self, record: &ActionReplayRecord) -> Option<()> {
        let mut frames = self.frames.clone();
        let delta = frames.install_action_replay_bottom(record)?;
        let pending = match self.pending.as_deref().cloned() {
            Some(mut pending) => {
                let offset = pending.frame_record.offset()?;
                pending.frame_record = WordRecordIndex::from_offset(offset.checked_add(delta)?)?;
                Some(pending)
            }
            None => None,
        };
        if !frames.continuation_store_is_valid(pending.as_ref()) {
            return None;
        }
        self.frames = frames;
        self.pending = pending.map(Arc::new);
        Some(())
    }

    /// Append one replay answer while atomically rebasing the separately
    /// owned pending CardPlay record index.
    #[cold]
    #[inline(never)]
    pub(crate) fn append_action_replay_answer(
        &mut self,
        record: WordRecordIndex,
        answer: ActionReplayAnswer,
    ) -> Option<()> {
        let mut frames = self.frames.clone();
        let insertion = frames.append_action_replay_answer(record, answer)?;
        let pending = match self.pending.as_deref().cloned() {
            Some(mut pending) => {
                let offset = pending.frame_record.offset()?;
                pending.frame_record = WordRecordIndex::from_offset(if offset >= insertion {
                    offset.checked_add(1)?
                } else {
                    offset
                })?;
                Some(pending)
            }
            None => None,
        };
        if !frames.continuation_store_is_valid(pending.as_ref()) {
            return None;
        }
        self.frames = frames;
        self.pending = pending.map(Arc::new);
        Some(())
    }

    fn hook_choice_piles(&self, pile: PileId) -> Vec<(HotCard, CardInstanceState)> {
        self.piles
            .get(pile)
            .as_slice()
            .iter()
            .map(|card| (*card, self.card_states.get(card.uid)))
            .collect()
    }

    /// Detach the suspended Draw at `depth` (its words from `word_base`) and
    /// the pending choice above it into the queued deferred hook action
    /// (#3387). `None` leaves `self` untouched: the pending choice must be
    /// the segment's, and no action may already be queued.
    #[cold]
    #[inline(never)]
    pub(crate) fn queue_deferred_hook_draw(
        &mut self,
        depth: usize,
        word_base: usize,
    ) -> Option<()> {
        if self.fanouts.deferred_hook_action_is_queued() {
            return None;
        }
        let mut pending = self.pending.as_deref()?.clone();
        let offset = pending.frame_record.offset()?;
        pending.frame_record = WordRecordIndex::from_offset(offset.checked_sub(word_base)?)?;
        let mut frames = self.frames.clone();
        let segment = frames.detach_hook_segment(depth, word_base)?;
        if pending.frame_record.offset()? >= segment.words.len() {
            return None;
        }
        // The segment's own card plays keep their cards in Play while the
        // enclosing action finishes (#3387): each must be there, once.
        let child_uids = self.frames.as_slice()[depth..]
            .iter()
            .filter_map(|frame| match *frame {
                Frame::CardPlay { record } => Some(self.frames.card_play(record).map(|p| p.uid)),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        let play = self
            .hook_choice_piles(PileId::Play)
            .into_iter()
            .filter(|(card, _)| child_uids.contains(&card.uid))
            .collect::<Vec<_>>();
        if play.len() != child_uids.len() {
            return None;
        }
        let action = DeferredHookAction {
            segment,
            pending,
            hand: self.hook_choice_piles(PileId::Hand),
            draw: self.hook_choice_piles(PileId::Draw),
            play,
        };
        self.frames = frames;
        self.pending = None;
        self.fanouts.queue_deferred_hook_action(action);
        Some(())
    }

    /// Whether the Hand and Draw piles still read as they did when the
    /// queued hook action's choice began (#3387).
    pub(crate) fn deferred_hook_choice_piles_are_unmoved(&self) -> bool {
        self.fanouts
            .batch_nine()
            .deferred_hook_action
            .as_deref()
            .is_some_and(|action| {
                action.hand == self.hook_choice_piles(PileId::Hand)
                    && action.draw == self.hook_choice_piles(PileId::Draw)
            })
    }

    /// Whether the Play pile holds exactly the queued hook action's own
    /// suspended card plays, as they were when its choice began: the
    /// enclosing action's card has left and nothing else moved (#3387).
    pub(crate) fn deferred_hook_play_pile_is_the_segments(&self) -> bool {
        self.fanouts
            .batch_nine()
            .deferred_hook_action
            .as_deref()
            .is_some_and(|action| action.play == self.hook_choice_piles(PileId::Play))
    }

    /// Re-attach the queued deferred hook action on top of the stack and
    /// restore its pending choice, emptying the queue (#3387).
    #[cold]
    #[inline(never)]
    pub(crate) fn publish_deferred_hook_draw(&mut self) -> Option<()> {
        if self.pending.is_some() {
            return None;
        }
        let action = self.fanouts.batch_nine().deferred_hook_action.clone()?;
        let mut frames = self.frames.clone();
        let base = frames.attach_hook_segment(&action.segment)?;
        let mut pending = action.pending.clone();
        let offset = pending.frame_record.offset()?;
        pending.frame_record = WordRecordIndex::from_offset(offset.checked_add(base)?)?;
        if !frames.continuation_store_is_valid(Some(&pending)) {
            return None;
        }
        self.fanouts.take_deferred_hook_action();
        self.frames = frames;
        self.pending = Some(Arc::new(pending));
        Some(())
    }

    /// A state carrying every canonical zero-default, and nothing else.
    ///
    /// The defaults are the dataclass defaults of `combat_sim.State`, which
    /// is what the canonical document's zero-default elision means: an absent
    /// key is this value. `boundary.rs` owns the single table those numbers
    /// come from; this constructor and that table are checked against each
    /// other by test.
    pub fn at_defaults() -> Self {
        Self {
            rng: HotRng::new(),
            piles: HotPiles::default(),
            monsters: Arc::new(Vec::new()),
            powers: Slots::new(),
            card_states: CardStates::new(),
            frames: Frames::new(),
            pending: None,
            orbs: HotOrbs::new(),
            fanouts: HotFanouts::new(),
            hp: 0,
            max_hp: 999,
            block: 0,
            gold: 0,
            cards_drawn_combat: 0,
            ps_strikes: 0,
            next_card_uid: 0,
            next_generated_hook_uid: 0,
            next_poison_uid: 0,
            energy: 3,
            stars: 0,
            innate_min_draw: 0,
            turn: 1,
            inky_attack_damage: 1,
            regalite_block_amount: 6,
            temp_strength: 0,
            player_phase: 0,
            exact_piles: false,
            player_side_active: true,
            card_affliction_state: 0,
            relic_state: 0,
            batch_six_relic_state: 0,
            reward_card_pool: None,
            entropy_card_pool: None,
            generated_power_flags: 0,
            fully_unlocked_card_pool_epochs: false,
            reward_card_rarity_odds: None,
            multiplayer_ally_key: 0,
            history: HotHistory::at_defaults(),
        }
    }

    /// The roster, mutably, cloning shared storage exactly once.
    pub fn monsters_mut(&mut self) -> &mut Vec<HotMonster> {
        Arc::make_mut(&mut self.monsters)
    }

    /// Whether any monster is still alive (`combat_sim.State.alive`).
    pub fn any_monster_alive(&self) -> bool {
        self.monsters.iter().any(|monster| monster.hp > 0)
    }

    /// Authenticate the unique physical owner of one persisted CardPlay.
    ///
    /// This is the shared read-only gate for every API which can expose or
    /// resume a continuation. Keeping it beside the hot state prevents legal
    /// enumeration, admission, and the execution guard from drifting onto
    /// different notions of a live active card.
    pub(crate) fn card_play_physical_owner(
        &self,
        record: CardPlayView<'_>,
    ) -> Option<(PileId, usize, HotCard)> {
        if record.uid >= self.next_card_uid {
            return None;
        }
        let mut owner = None;
        for pile in PileId::ALL {
            for (index, card) in self.piles.get(pile).as_slice().iter().copied().enumerate() {
                if card.uid != record.uid {
                    continue;
                }
                if owner.replace((pile, index, card)).is_some() {
                    return None;
                }
            }
        }
        let owner = owner?;
        if owner.0 != record.source_pile
            || record.active_play_member != (owner.0 == PileId::Play)
            || record.source == CardPlaySource::Manual && owner.0 != PileId::Play
        {
            return None;
        }
        Some(owner)
    }

    /// Validate the structural and physical ownership of the pending top play.
    pub(crate) fn pending_card_play(
        &self,
        pending: &PendingSelection,
    ) -> Option<(CardPlayView<'_>, HotCard)> {
        let record = pending.record(&self.frames)?;
        let (_, _, card) = self.card_play_physical_owner(record)?;
        Some((record, card))
    }
}

// --- compile-time size pins (PORT_PLAN.md D3, §7) --------------------------
//
// Budgets are set from the measured R0.4 layout with headroom for the fields
// R0.5 claims, and are checked here so exceeding one is a compile error on
// the pull request that does it. The v0.110.1 kernel's 144-byte Byrdonis hot
// state is the aspiration; its 656-byte text-laden Queen state
// (`versions/v0.110.1/rust/src/queen.rs:1047`) is the cautionary tale.

/// Exactly eight bytes, per D3. Not a budget — an equality.
const _: () = assert!(size_of::<HotCard>() == 8);
/// Damage growth occupies padding already present in the side-table payload.
/// Genetic Algorithm's exact persistent relation consumes one explicit word;
/// all other card-instance payloads retain the same one-row COW table.
const _: () = assert!(size_of::<CardInstanceState>() == 32);
/// The exact Thrash Decimal is out-of-line behind the existing cost carrier.
const _: () = assert!(size_of::<LocalCostModifiers>() == size_of::<usize>());
/// The complete pre-reviewed inline-state reserve described below.
const HOT_STATE_SIZE_BUDGET: usize = 224;
/// The current layout measurement. Spending the reserve updates this detector,
/// but never the independently reviewed ceiling above.
const HOT_STATE_MEASURED: usize = 224;
/// #1486 raised the reviewed ceiling from 208 to 224 for two pointer-sized
/// consumers. The selection payload (#1363) spent one word through
/// `Option<Arc<PendingSelection>>`; Rust's non-null `Arc` niche keeps that
/// nullable handle to one word independently of its closed payload. Batch K
/// then spent the final eight-byte alignment step on Ringing's canonical
/// player latch, taking the measured layout from 216 to the 224-byte ceiling.
/// There is therefore no unspent ally-roster word here.
///
/// The later exact consumers did not raise or repack this inline budget. The
/// solo Osty slot (#1675) and the represented remote Player (#1361/#1562) live
/// inside [`FanoutState`] behind the already-present pointer-sized
/// [`HotFanouts`] handle. #1674's `multiplayer_ally_key` fit existing sub-word
/// padding. That out-of-line choice preserves this 224-byte layout, but its
/// larger fanout allocation remains paid and measured by the performance
/// floor; it is not free headroom. A future inline field must recover space or
/// obtain a separately reviewed ceiling increase rather than relying on the
/// old reservation.
///
/// Before those two final steps, 208 measured after #1406 added the
/// one-pointer card-event fan-out block to #1394's 200, which added the
/// one-pointer orb queue to #1374's 184 and itself followed slice 2's 168,
/// from R0.5's 152 (itself down from R0.4's 184: the nine streams' 72 bytes of
/// draw counters left for the copy-on-write [`HotRng`] block, and
/// [`HotHistory`] moved in). #1480's Stars balance and turn-history counter
/// fit in existing padding, so the measured size remained 208.
///
/// The compile-time pin is intentionally a ceiling; the runtime test remains
/// an equality against `HOT_STATE_MEASURED`. A planned owner spending the
/// reserve therefore updates one measurement constant, while unexpected
/// growth still fails immediately and exceeding the reserve is a compile
/// error.
const _: () = assert!(HOT_STATE_MEASURED <= HOT_STATE_SIZE_BUDGET);
const _: () = assert!(size_of::<HotState>() <= HOT_STATE_SIZE_BUDGET);
/// One ordered orb member and one copy-on-write queue handle.
const _: () = assert!(size_of::<HotOrb>() == 8);
const _: () = assert!(size_of::<HotOrbs>() == size_of::<usize>());
/// One more pointer for the complete card-event listener surface.
const _: () = assert!(size_of::<HotFanouts>() == size_of::<usize>());
/// Unit C Batch B raised the closed fanout block from 120 to 256 bytes. One
/// For All adds the remote Player's independent full-width Power amount;
/// alignment raises that party-only COW allocation to 264 bytes. The
/// complete delta is the exact remote-Player pile/RNG/resource quotient, two
/// target-keyed Imitation stacks, the COW frozen-clone list, plus the two-bit
/// Intercept/Covered set. The pending callback now lives with its exact remote
/// owner: adding its full tuple grows that cold quotient from 112 to 120 bytes,
/// while the vacated twelve-byte lane carries Constrict's amount-only player
/// state. The enclosing block therefore remains 312 bytes. The Hunt then
/// adds one packed word for its independent deferred-reward count and inert,
/// ending-gated marker, taking the allocation from 264 to 272 bytes. Furnace's
/// current-build-complete 18-member represented `AfterSideTurnStart` order
/// adds thirty-six payload bytes plus alignment for the remote quotient,
/// taking the allocation to 312 bytes. Monologue's COW-private Strength ledger
/// consumes the remaining three bytes of tail padding after its three hooks,
/// Ruined Helmet, Pale Blue Dot, and Unsettling Lamp are folded into one
/// private flag byte. The block therefore stays at 312 bytes and behind the
/// existing one-word `HotFanouts` handle: a solo state and all of its search
/// clones keep sharing the default allocation, while parsing a party state is
/// what detaches and populates the remote quotient. This allocation trade is
/// explicit even though `HotState` itself remains 224 bytes. Lethality's
/// combat-wide Ethereal completion count fit the pre-Hunt allocation
/// block by storing Automation's admitted 1..=10 countdown and Panache's
/// admitted 1..=5 countdown as `u8`; their public accessors remain signed and
/// the boundary/admission pair still rejects every out-of-domain value. R50
/// spends the two remaining alignment bytes by growing `AfterBlockCleared`
/// from one `PowerId` to two; the allocation remains exactly 312 bytes.
///
/// The exact pins here and on [`FrameStore`] hold pointer-width fields, so
/// each is pinned per target width (#3469): the measured 64-bit layout, and
/// the wasm32 layout the browser build compiles (`FanoutState` 296,
/// `MultiplayerAllyState` 112, `FrameStore` 24, measured on
/// `wasm32-unknown-unknown`). A widening still trips the pin on both.
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<FanoutState>() == 312);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(size_of::<FanoutState>() == 296);
const _: () = assert!(size_of::<ColdPowerRecord>() == 24);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<MultiplayerAllyState>() == 120);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Option<MultiplayerAllyState>>() == 120);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(size_of::<MultiplayerAllyState>() == 112);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(size_of::<Option<MultiplayerAllyState>>() == 112);
/// 96 measured after #1675 adds the distinct nonowner-same-side powered-result
/// counter to the prior 88-byte layout. Both history counters are complete
/// state and can coexist on one target. #1560 Batch F added the two signed
/// Possess owner debits to #1491's 72-byte layout. Those scalars are likewise
/// complete state: reconstructing a
/// source debit from the player's later aggregate would be wrong. #1491 added
/// one packed random-AI state word to #1472's 64-byte Waterfall layout. The
/// word crossed the roster element's eight-byte alignment boundary, so its
/// measured cost was eight bytes rather than its four-byte field width. #1472
/// added Waterfall Giant's three private `i32` values and one form latch to
/// slice 2's 56, from R0.5's 48 (itself from R0.4's 40:
/// `misery_debuff_order` and the owner-damage-result counter); slice 2 adds
/// `poison_uid`. The non-negative buildup index and form latch share one word,
/// keeping the roster allocation below the next size class. #1675 consumes
/// the reviewed 96-byte cap without growing the pointer-sized `HotState`.
const _: () = assert!(size_of::<HotMonster>() <= 96);
/// The play-history block, inline. Not a separate D3 budget — it is part of
/// [`HotState`]'s — but pinned so a careless widening is visible.
///
/// **Raised from 32 to 48 at slice 2 (#1367), measured 44.** Five `State`
/// counters the slice's new primitives write joined it: the 0-cost attack
/// tally (#1314), the non-hand draw tally and the discard tally that a
/// step-callable Draw / `_discard_and_draw` maintain, and the two exhaust
/// counters. Each is a field Python projects, so *not* carrying it is a
/// divergence on the first transition that touches it, not a saving. The
/// enclosing [`HotState`] budget is checked separately, and the block stays
/// inline for the reason it always was:
/// every card play writes several of these, so an indirection would allocate
/// once per transition to save 48 bytes. Issue #1374 adds the canonical
/// per-turn Shiv-finished counter; the block is now 52 bytes, absorbed by
/// existing tail padding so the enclosing state remains 184 bytes. Issue
/// #1406 spends the last byte of that same padding on Neurosurge's Doom
/// history latch; both measured sizes remain unchanged.
const _: () = assert!(size_of::<HotHistory>() <= 52);
/// Copy-on-write handles are one pointer each.
const _: () = assert!(size_of::<HotRng>() == size_of::<usize>());
const _: () = assert!(size_of::<MiseryOrder>() == size_of::<usize>());
/// One pointer per pile, five piles.
const _: () = assert!(size_of::<HotPiles>() == 5 * size_of::<usize>());
/// Copy-on-write handles are one pointer each.
const _: () = assert!(size_of::<CardStates>() == size_of::<usize>());
const _: () = assert!(size_of::<Frames>() == size_of::<usize>());
/// One compact record index replaces the former pair of fat COW slices;
/// variable words live behind the already-present one-pointer Frames handle.
const _: () = assert!(size_of::<PendingSelection>() == 8);
const _: () = assert!(size_of::<PendingWord>() == 8);
const _: () = assert!(align_of::<PendingWord>() == align_of::<HotCard>());
const _: () = assert!(size_of::<WordRecordIndex>() == 4);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<FrameStore>() == 48);
#[cfg(target_pointer_width = "32")]
const _: () = assert!(size_of::<FrameStore>() == 24);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Frame, TurnStartStage};
    use crate::powers::SlotWire;

    /// Every `wire_enum!` vocabulary parses back every name it can emit.
    ///
    /// `from_str` binary-searches `NAMES`, so an out-of-order variant breaks
    /// *other* variants' parsing while round-tripping fine itself. Walking all
    /// of them makes the next insertion fail here, at the assumption, rather
    /// than as an unrelated `unknown monster override` at some boundary.
    #[test]
    fn every_wire_vocabulary_is_ascending_and_round_trips() {
        macro_rules! check {
            ($($ty:ty),+ $(,)?) => {$(
                assert!(
                    <$ty>::NAMES.windows(2).all(|pair| pair[0] < pair[1]),
                    "{} NAMES is not ascending, so from_str's binary search is unsound",
                    stringify!($ty)
                );
                for variant in <$ty>::ALL {
                    assert_eq!(
                        <$ty>::from_str(variant.as_str()),
                        Some(variant),
                        "{}::{:?} does not parse back from {:?}",
                        stringify!($ty),
                        variant,
                        variant.as_str()
                    );
                }
            )+};
        }
        // Every `wire_enum!` in the crate, not a subset: the guard is only
        // worth having if a new vocabulary is covered by default.
        check!(
            PileId,
            OrbKind,
            RngStream,
            MonsterOverride,
            MonsterFollowUp,
            MiseryToken,
            crate::catalog::GameBuild,
            crate::catalog::RewardPool,
            crate::catalog::RewardOdds,
            crate::catalog::CardTargetType,
        );
    }

    #[test]
    fn the_measured_sizes_are_what_the_budgets_claim() {
        assert_eq!(size_of::<PowerId>(), 2);
        assert_eq!(size_of::<HotCard>(), 8);
        assert_eq!(size_of::<OptionalNonNegativeI32>(), 4);
        assert_eq!(size_of::<BaseReplayCount>(), 4);
        assert_eq!(size_of::<CardInstanceState>(), 32);
        assert_eq!(size_of::<HotState>(), HOT_STATE_MEASURED);
        assert_eq!(size_of::<HotOrb>(), 8);
        assert_eq!(size_of::<HotOrbs>(), 8);
        assert_eq!(size_of::<FanoutState>(), 312);
        assert_eq!(size_of::<PotionBeltState>(), 56);
        assert_eq!(size_of::<ColdPowerRecord>(), 24);
        assert_eq!(size_of::<MultiplayerAllyState>(), 120);
        assert_eq!(size_of::<Option<MultiplayerAllyState>>(), 120);
        assert_eq!(
            size_of::<HotMonster>(),
            96,
            "the second exact powered-result history counter consumes the reviewed cap"
        );
        assert_eq!(size_of::<HotHistory>(), 52);
        assert_eq!(size_of::<PendingWord>(), 8);
        assert_eq!(size_of::<WordRecordIndex>(), 4);
        assert_eq!(size_of::<FrameStore>(), 48);
        assert_eq!(size_of::<PendingSelection>(), 8);
        assert_eq!(size_of::<Frames>(), 8);
        assert_eq!(size_of::<CardStates>(), 8);
        assert_eq!(size_of::<HotRng>(), 8);
        assert_eq!(size_of::<MiseryOrder>(), 8);
        assert_eq!(size_of::<crate::engine::Action>(), 12);
        assert_eq!(size_of::<Frame>(), 8);
        assert_eq!(DrawCaller::CardPlay as u8, 11);
        assert_eq!(DrawCaller::CardPlay.as_str(), "none");
        assert_eq!(DrawCaller::from_str("none"), Some(DrawCaller::CardPlay));
        assert_eq!(DrawCaller::from_ordinal(11), Some(DrawCaller::CardPlay));
        assert_eq!(DrawCaller::DarkEmbraceSideEnd as u8, 12);
        assert_eq!(
            DrawCaller::DarkEmbraceSideEnd.as_str(),
            "dark_embrace_side_end"
        );
        assert_eq!(
            DrawCaller::from_str("dark_embrace_side_end"),
            Some(DrawCaller::DarkEmbraceSideEnd)
        );
        assert_eq!(
            DrawCaller::from_ordinal(12),
            Some(DrawCaller::DarkEmbraceSideEnd)
        );
    }

    #[test]
    fn hit_power_factoradic_order_lifecycle_and_nested_cow_are_exact() {
        use AfterDamageReceivedPower::{FlameBarrier, Reflect, TheGambit};
        let permutations = [
            [FlameBarrier, Reflect, TheGambit],
            [FlameBarrier, TheGambit, Reflect],
            [Reflect, FlameBarrier, TheGambit],
            [Reflect, TheGambit, FlameBarrier],
            [TheGambit, FlameBarrier, Reflect],
            [TheGambit, Reflect, FlameBarrier],
        ];
        for order in permutations {
            let mut state = HotState::at_defaults();
            for token in order {
                state.powers.set(token.power(), SlotWire::Int, 1);
            }
            assert!(state.set_after_damage_received_power_order(&order));
            assert_eq!(&*state.after_damage_received_power_order().unwrap(), &order);
        }

        let mut left = HotState::at_defaults();
        assert!(left.register_after_damage_received_power(PowerId::Reflect));
        left.powers.set(PowerId::Reflect, SlotWire::Int, 1);
        assert!(left.register_after_damage_received_power(PowerId::FlameBarrier));
        left.powers.set(PowerId::FlameBarrier, SlotWire::Int, 4);
        let mut right = left.clone();
        assert!(Arc::ptr_eq(
            &left.fanouts.0.cold.potion_belt,
            &right.fanouts.0.cold.potion_belt
        ));
        assert!(right.unregister_after_damage_received_power(PowerId::Reflect));
        right.powers.set(PowerId::Reflect, SlotWire::Int, 0);
        assert!(!Arc::ptr_eq(
            &left.fanouts.0.cold.potion_belt,
            &right.fanouts.0.cold.potion_belt
        ));
        assert_ne!(left, right);
        assert_eq!(
            &*left.after_damage_received_power_order().unwrap(),
            &[Reflect, FlameBarrier]
        );
        assert!(right.register_after_damage_received_power(PowerId::Reflect));
        right.powers.set(PowerId::Reflect, SlotWire::Int, 1);
        assert_eq!(
            &*right.after_damage_received_power_order().unwrap(),
            &[FlameBarrier, Reflect]
        );
    }

    #[test]
    fn potion_belt_is_nested_cow_and_keeps_frozen_layouts() {
        let mut root = HotFanouts::new();
        assert!(root.set_potion_belt(
            vec![Some(PotionId::FirePotion), None, Some(PotionId::FirePotion)],
            true,
            false,
            false,
            true,
            true,
        ));
        let mut successor = root.clone();
        assert!(root.shares_store_with(&successor));
        assert!(root.potion_belt_shares_store_with(&successor));

        assert!(successor.set_withering_cards_left(1));
        assert!(!root.shares_store_with(&successor));
        assert!(root.potion_belt_shares_store_with(&successor));
        assert_eq!(successor.potion_slots(), root.potion_slots());

        assert!(successor.set_duplication(2));
        assert!(!root.potion_belt_shares_store_with(&successor));
        assert_eq!(root.duplication(), 0);
        assert_eq!(successor.duplication(), 2);
        assert_eq!(root.potion_slots()[0], Some(PotionId::FirePotion));

        assert!(successor.set_potion_belt(
            vec![None, Some(PotionId::WeakPotion)],
            false,
            true,
            false,
            false,
            true,
        ));
        assert!(!root.potion_belt_shares_store_with(&successor));
        assert_eq!(root.potion_slots()[0], Some(PotionId::FirePotion));
        assert_eq!(successor.potion_slots(), [None, Some(PotionId::WeakPotion)]);
        assert_eq!(successor.duplication(), 0);

        successor.forge_potion_belt_flags_for_test(0x80);
        assert!(!successor.potion_belt_topology_is_exact());
        assert_eq!(size_of::<FanoutState>(), 312);
        assert_eq!(size_of::<HotState>(), HOT_STATE_MEASURED);
    }

    #[test]
    fn potion_action_replay_uses_the_third_two_word_action_tag() {
        let record = ActionReplayRecord {
            predecessor_json: br#"{"player":{},"schema":"sts-sim-canonical-v2"}"#.to_vec(),
            action: ActionReplayRootAction::UsePotion {
                slot: 254,
                target: Some(255),
            },
            answers: Vec::new(),
        };
        let mut frames = Frames::new();
        let index = frames.push_action_replay(&record).unwrap();
        assert_eq!(frames.action_replay(index), Some(record));
    }

    fn test_card_play(uid: u32) -> CardPlayRecord {
        CardPlayRecord {
            uid,
            stage: CardPlayStage::Body,
            source: CardPlaySource::Manual,
            result_route: CardResultRoute::DiscardBottom,
            choice_kind: CardPlayChoiceKind::None,
            choice_i32: 0,
            first_choice: None,
            target: None,
            latch_kind: CardPlayLatchKind::Raw,
            pen_double: false,
            force_exhaust: false,
            is_power_auto: false,
            active_play_member: true,
            pending_choice: true,
            lamp_owner: false,
            plays: 1,
            play_index: 0,
            x_value: 0,
            spent: 0,
            star_spent: 0,
            glam: 0,
            next_step: 1,
            afterimage: 0,
            subroutine: 0,
            storm: 0,
            serpent: 0,
            rupture_batch: 0,
            panache: false,
            rupture_registered: false,
            calamity: false,
            monologue: false,
            tender: false,
            selection_amount: 0,
            selection_kind: Some(PendingSelectionKind::Program),
            source_pile: PileId::Play,
            expos: None,
            selection_cards: Vec::new(),
            oblivion_latches: Vec::new(),
            strangle_latches: Vec::new(),
            monologue_latches: Vec::new(),
            instanced_power_uid_at_before: None,
            instanced_power_created_uid: None,
            after_card_played_power_snapshot: Vec::new(),
            after_card_played_power_snapshot_captured: false,
            after_card_played_power_cursor: 0,
            after_card_played_power_phase: 0,
            after_card_played_pending_monologue_uid: None,
            physical_after_card_played_listeners: Vec::new(),
        }
    }

    fn pending_for_record(frame_uid: u32, frame_record: WordRecordIndex) -> PendingSelection {
        PendingSelection {
            frame_uid,
            frame_record,
        }
    }

    #[test]
    fn action_replay_arena_round_trips_and_atomically_rebases_every_record_index() {
        let predecessor_json = br#"{"player":{},"schema":"sts-sim-canonical-v2"}"#.to_vec();
        let replay = ActionReplayRecord {
            predecessor_json: predecessor_json.clone(),
            action: ActionReplayRootAction::Play {
                uid: 7,
                target: Some(2),
                selection_uid: Some(9),
            },
            answers: Vec::new(),
        };
        let mut state = HotState::at_defaults();
        let replay_index = state.frames.push_action_replay(&replay).unwrap();
        let phase_index = state
            .frames
            .push_auto_post_phase(&AutoPostPhaseRecord {
                cursor: 0,
                listeners: vec![AutoPostListener::Stampede],
            })
            .unwrap();
        let live_index = state
            .frames
            .push_stampede_live(StampedeLiveRecord {
                cursor: 0,
                iterations: 1,
            })
            .unwrap();
        let batch_index = state
            .frames
            .push_frozen_auto_batch(&FrozenAutoBatchRecord {
                entries: vec![FrozenAutoBatchEntry {
                    card: HotCard {
                        uid: 11,
                        atom: 1,
                        flags: 0,
                    },
                    state: CardInstanceState::default(),
                }],
                cursor: 1,
                gather_target: 0,
                force_exhaust: false,
                source: FrozenAutoBatchSource::DrawPileFlip,
            })
            .unwrap();
        let after_power_index = state
            .frames
            .push_after_power_amount_changed(&AfterPowerAmountChangedRecord {
                listeners: vec![PowerId::Vicious],
                cursor: 1,
                amount: 2,
                target: (0, 3, None),
            })
            .unwrap();
        let draw_index = state
            .frames
            .push_draw(&DrawRecord {
                requested: 1,
                completed: 0,
                drawn: vec![DrawEntry::Uid(HotCard {
                    uid: 12,
                    atom: 1,
                    flags: 0,
                })],
                from_hand_draw: false,
                stage: DrawStage::EarlyHook,
                card_uid: Some(12),
                caller: DrawCaller::AfterPowerAmountChanged,
                shuffle_candidates: Vec::new(),
                gamble_paired: None,
            })
            .unwrap();
        let card_index = state.frames.push_card_play(&test_card_play(7)).unwrap();
        state.pending = Some(Arc::new(pending_for_record(7, card_index)));
        assert!(
            state
                .frames
                .continuation_store_is_valid(state.pending.as_deref())
        );
        let before = [
            phase_index,
            live_index,
            batch_index,
            after_power_index,
            draw_index,
            card_index,
        ]
        .map(|index| index.offset().unwrap());

        state
            .append_action_replay_answer(replay_index, ActionReplayAnswer::OptionIndex(4))
            .unwrap();
        let after: Vec<usize> = state
            .frames
            .as_slice()
            .iter()
            .filter_map(|frame| match *frame {
                Frame::ActionReplay { .. } => None,
                Frame::Phase { record }
                | Frame::LiveListener { record }
                | Frame::FrozenAutoBatch { record }
                | Frame::AfterPowerAmountChanged { record }
                | Frame::Draw { record }
                | Frame::CardPlay { record } => record.offset(),
                _ => None,
            })
            .collect();
        assert_eq!(after, before.map(|offset| offset + 1));
        assert_eq!(
            state.pending.as_deref().unwrap().frame_record.offset(),
            Some(before[5] + 1)
        );
        assert_eq!(
            state.frames.action_replay(replay_index).unwrap(),
            ActionReplayRecord {
                predecessor_json,
                action: replay.action,
                answers: vec![ActionReplayAnswer::OptionIndex(4)],
            }
        );
        assert!(
            state
                .frames
                .continuation_store_is_valid(state.pending.as_deref())
        );

        let checkpoint = state.clone();
        let wrong_index = WordRecordIndex::from_raw_for_test(1);
        assert!(
            state
                .append_action_replay_answer(wrong_index, ActionReplayAnswer::CardUid(3))
                .is_none()
        );
        assert_eq!(state, checkpoint, "a failed append is completely atomic");

        let mut stale_pending = state.clone();
        Arc::make_mut(stale_pending.pending.as_mut().unwrap()).frame_record =
            WordRecordIndex::from_raw_for_test(1);
        let checkpoint = stale_pending.clone();
        assert!(
            stale_pending
                .append_action_replay_answer(replay_index, ActionReplayAnswer::CardUid(5))
                .is_none()
        );
        assert_eq!(
            stale_pending, checkpoint,
            "a stale separate PendingSelection index rolls the splice back"
        );
    }

    #[test]
    fn action_replay_prepend_is_atomic_across_all_record_indexes() {
        let replay = ActionReplayRecord {
            predecessor_json: br#"{"player":{},"schema":"sts-sim-canonical-v2"}"#.to_vec(),
            action: ActionReplayRootAction::EndTurn,
            answers: Vec::new(),
        };
        let mut state = HotState::at_defaults();
        let phase_index = state
            .frames
            .push_auto_post_phase(&AutoPostPhaseRecord {
                cursor: 0,
                listeners: vec![AutoPostListener::Stampede],
            })
            .unwrap();
        let live_index = state
            .frames
            .push_stampede_live(StampedeLiveRecord {
                cursor: 0,
                iterations: 1,
            })
            .unwrap();
        let batch_index = state
            .frames
            .push_frozen_auto_batch(&FrozenAutoBatchRecord {
                entries: vec![FrozenAutoBatchEntry {
                    card: HotCard {
                        uid: 11,
                        atom: 1,
                        flags: 0,
                    },
                    state: CardInstanceState::default(),
                }],
                cursor: 1,
                gather_target: 0,
                force_exhaust: false,
                source: FrozenAutoBatchSource::DrawPileFlip,
            })
            .unwrap();
        let after_power_index = state
            .frames
            .push_after_power_amount_changed(&AfterPowerAmountChangedRecord {
                listeners: vec![PowerId::Vicious],
                cursor: 1,
                amount: 2,
                target: (0, 3, None),
            })
            .unwrap();
        let draw_index = state
            .frames
            .push_draw(&DrawRecord {
                requested: 1,
                completed: 0,
                drawn: vec![DrawEntry::Uid(HotCard {
                    uid: 12,
                    atom: 1,
                    flags: 0,
                })],
                from_hand_draw: false,
                stage: DrawStage::EarlyHook,
                card_uid: Some(12),
                caller: DrawCaller::AfterPowerAmountChanged,
                shuffle_candidates: Vec::new(),
                gamble_paired: None,
            })
            .unwrap();
        let card_index = state.frames.push_card_play(&test_card_play(7)).unwrap();
        state.pending = Some(Arc::new(pending_for_record(7, card_index)));
        let before = [
            phase_index,
            live_index,
            batch_index,
            after_power_index,
            draw_index,
            card_index,
        ]
        .map(|index| index.offset().unwrap());
        let expected_delta = Frames::encode_action_replay(&replay, None).unwrap();
        state.install_action_replay_root(&replay).unwrap();
        assert_eq!(
            state.frames.as_slice().first(),
            Some(&Frame::ActionReplay {
                record: WordRecordIndex::from_offset(0).unwrap(),
            })
        );
        let after: Vec<_> = state
            .frames
            .as_slice()
            .iter()
            .skip(1)
            .filter_map(|frame| match *frame {
                Frame::Phase { record }
                | Frame::LiveListener { record }
                | Frame::FrozenAutoBatch { record }
                | Frame::AfterPowerAmountChanged { record }
                | Frame::Draw { record }
                | Frame::CardPlay { record } => record.offset(),
                _ => None,
            })
            .collect();
        assert_eq!(
            after,
            before.map(|offset| offset + expected_delta),
            "all six child record variants shift by the complete root length"
        );
        assert_eq!(
            state.pending.as_deref().unwrap().frame_record.offset(),
            Some(before[5] + expected_delta)
        );
        assert!(
            state
                .frames
                .continuation_store_is_valid(state.pending.as_deref())
        );

        let checkpoint = state.clone();
        assert!(state.install_action_replay_root(&replay).is_none());
        assert_eq!(state, checkpoint, "a second root is wholly atomic");

        let mut overflowing = HotState::at_defaults();
        overflowing.frames.push(Frame::CardPlay {
            record: WordRecordIndex::from_raw_for_test(u32::MAX - 1),
        });
        let checkpoint = overflowing.clone();
        assert!(overflowing.install_action_replay_root(&replay).is_none());
        assert_eq!(
            overflowing, checkpoint,
            "record-index overflow cannot partially prepend"
        );
    }

    #[test]
    fn action_replay_rejects_nonzero_json_padding_and_malformed_bounds() {
        let record = ActionReplayRecord {
            predecessor_json: b"x".to_vec(),
            action: ActionReplayRootAction::EndTurn,
            answers: vec![ActionReplayAnswer::CardUid(3)],
        };
        let mut frames = Frames::new();
        let index = frames.push_action_replay(&record).unwrap();
        assert_eq!(frames.action_replay(index), Some(record.clone()));

        let mut bad_padding = frames.clone();
        Arc::make_mut(&mut bad_padding.0).words[4].meta = 1;
        assert!(bad_padding.action_replay(index).is_none());
        assert!(!bad_padding.continuation_store_is_valid(None));

        let mut unknown_header = frames.clone();
        Arc::make_mut(&mut unknown_header.0).words[0].meta |= 0xff;
        assert!(unknown_header.action_replay(index).is_none());
        assert!(!unknown_header.continuation_store_is_valid(None));

        let mut reserved_header = frames.clone();
        Arc::make_mut(&mut reserved_header.0).words[0].meta |= 1 << 31;
        assert!(reserved_header.action_replay(index).is_none());
        assert!(!reserved_header.continuation_store_is_valid(None));

        let mut unknown_action = frames.clone();
        Arc::make_mut(&mut unknown_action.0).words[1].meta = 3;
        assert!(unknown_action.action_replay(index).is_none());
        assert!(!unknown_action.continuation_store_is_valid(None));

        let mut reserved_action = frames.clone();
        Arc::make_mut(&mut reserved_action.0).words[1].meta |= 1 << 31;
        assert!(reserved_action.action_replay(index).is_none());
        assert!(!reserved_action.continuation_store_is_valid(None));

        let potion = ActionReplayRecord {
            predecessor_json: b"x".to_vec(),
            action: ActionReplayRootAction::UsePotion {
                slot: u8::MAX,
                target: Some(u8::MAX),
            },
            answers: Vec::new(),
        };
        let mut potion_frames = Frames::new();
        let potion_index = potion_frames.push_action_replay(&potion).unwrap();
        assert_eq!(potion_frames.action_replay(potion_index), Some(potion));

        let mut potion_slot_overflow = potion_frames.clone();
        Arc::make_mut(&mut potion_slot_overflow.0).words[1].body = 256;
        assert!(potion_slot_overflow.action_replay(potion_index).is_none());
        assert!(!potion_slot_overflow.continuation_store_is_valid(None));

        let mut potion_target_overflow = potion_frames.clone();
        Arc::make_mut(&mut potion_target_overflow.0).words[1].meta = 2 | (257 << 8);
        assert!(potion_target_overflow.action_replay(potion_index).is_none());
        assert!(!potion_target_overflow.continuation_store_is_valid(None));

        let mut potion_selection = potion_frames.clone();
        Arc::make_mut(&mut potion_selection.0).words[2].body = 0;
        assert!(potion_selection.action_replay(potion_index).is_none());
        assert!(!potion_selection.continuation_store_is_valid(None));

        let mut unknown_answer = frames.clone();
        Arc::make_mut(&mut unknown_answer.0).words[5].meta = 2;
        assert!(unknown_answer.action_replay(index).is_none());
        assert!(!unknown_answer.continuation_store_is_valid(None));

        let mut trailing_word = frames.clone();
        Arc::make_mut(&mut trailing_word.0)
            .words
            .push(PendingWord::default());
        assert_eq!(trailing_word.action_replay(index), Some(record.clone()));
        assert!(!trailing_word.continuation_store_is_valid(None));

        let mut bad_length = frames.clone();
        Arc::make_mut(&mut bad_length.0).words[3].body = u32::MAX;
        assert!(bad_length.action_replay(index).is_none());
        assert!(!bad_length.continuation_store_is_valid(None));

        let mut empty_predecessor = frames.clone();
        let empty_store = Arc::make_mut(&mut empty_predecessor.0);
        empty_store.words[0] = PendingWord::header(WordRecordKind::ActionReplay, 4).unwrap();
        empty_store.words[3].body = 0;
        empty_store.words.remove(4);
        let checkpoint = empty_predecessor.clone();
        assert!(empty_predecessor.action_replay(index).is_none());
        assert!(!empty_predecessor.continuation_store_is_valid(None));
        assert_eq!(empty_predecessor, checkpoint, "validation is read-only");

        let mut uninjective_selection = record.clone();
        uninjective_selection.action = ActionReplayRootAction::Play {
            uid: 7,
            target: None,
            selection_uid: Some(u32::MAX),
        };
        let mut rejected = Frames::new();
        assert!(
            rejected
                .push_action_replay(&uninjective_selection)
                .is_none()
        );
        assert!(rejected.is_empty());
        assert!(rejected.0.words.is_empty());
    }

    #[test]
    fn action_replay_action_agrees_with_the_full_decoder_at_every_padding_width() {
        let set_byte = |frames: &mut Frames, word: usize, byte: usize, value: u8| {
            let word = &mut Arc::make_mut(&mut frames.0).words[word];
            let half = if byte < 4 {
                &mut word.body
            } else {
                &mut word.meta
            };
            let mut bytes = half.to_le_bytes();
            bytes[byte % 4] = value;
            *half = u32::from_le_bytes(bytes);
        };
        let action = ActionReplayRootAction::Play {
            uid: 7,
            target: Some(1),
            selection_uid: Some(9),
        };
        for len in 1_usize..=17 {
            let record = ActionReplayRecord {
                predecessor_json: (1..=len).map(|byte| byte as u8).collect(),
                action,
                answers: vec![
                    ActionReplayAnswer::CardUid(3),
                    ActionReplayAnswer::OptionIndex(0),
                ],
            };
            let mut frames = Frames::new();
            let index = frames.push_action_replay(&record).unwrap();
            assert_eq!(frames.action_replay(index), Some(record.clone()));
            assert_eq!(frames.action_replay_action(index), Some(action));

            let json_words = len.div_ceil(8);
            for position in 0..json_words * 8 {
                let mut drifted = frames.clone();
                set_byte(&mut drifted, 4 + position / 8, position % 8, 0xa5);
                let decoded = drifted.action_replay(index);
                assert_eq!(
                    drifted.action_replay_action(index),
                    decoded.as_ref().map(|replay| replay.action),
                    "len {len}, byte {position}"
                );
                // Content bytes are the predecessor's own; padding must be zero.
                assert_eq!(
                    decoded.is_some(),
                    position < len,
                    "len {len}, byte {position}"
                );
            }
        }
    }

    #[test]
    fn card_play_word_store_round_trips_cows_and_truncates() {
        let cards = [HotCard {
            uid: 9,
            atom: 3,
            flags: CARD_FLAG_PICK,
        }];
        let mut payload = test_card_play(7);
        payload.selection_cards = cards.to_vec();
        payload.strangle_latches = vec![(11, 2)];
        let mut frames = Frames::new();
        frames.push(Frame::JossSideEnd);
        let index = frames.push_card_play(&payload).unwrap();
        let pending = pending_for_record(7, index);
        let view = frames.card_play(index).unwrap();
        assert!(frames.pending_choice_is_valid(&pending));
        assert_eq!(view.selection_cards(), cards);
        assert_eq!(view.strangle_latches().collect::<Vec<_>>(), [(11, 2)]);

        let original = frames.clone();
        assert!(original.shares_store_with(&frames));
        assert_eq!(frames.pop_top_card_play().unwrap(), payload);
        assert!(!original.shares_store_with(&frames));
        assert_eq!(frames.as_slice(), &[Frame::JossSideEnd]);
        assert!(frames.0.words.is_empty());
    }

    #[test]
    fn instanced_power_snapshot_wire_is_exact_at_the_adjacent_record_boundary() {
        let mut record = test_card_play(7);
        record.pending_choice = false;
        record.selection_kind = None;
        record.stage = CardPlayStage::AfterImitation;
        record.latch_kind = CardPlayLatchKind::Post;
        record.instanced_power_uid_at_before = Some(3);
        record.instanced_power_created_uid = Some(12);
        record.after_card_played_power_snapshot_captured = true;
        record.after_card_played_power_snapshot = vec![3, 9, 12];
        record.after_card_played_power_cursor = 3;
        record.after_card_played_power_phase = 3;

        let mut frames = Frames::new();
        let card_index = frames.push_card_play(&record).unwrap();
        let phase_index = frames
            .push_auto_post_phase(&AutoPostPhaseRecord {
                cursor: 0,
                listeners: vec![AutoPostListener::Stampede],
            })
            .unwrap();

        let decoded = frames.card_play(card_index).unwrap();
        assert_eq!(decoded.instanced_power_uid_at_before, Some(3));
        assert_eq!(decoded.instanced_power_created_uid, Some(12));
        assert_eq!(decoded.to_owned(), record);
        assert!(frames.auto_post_phase(phase_index).is_some());
        assert_eq!(phase_index.offset(), Some(21));
        assert_eq!(frames.0.words[20].body, 12, "the final uid is payload");
        assert_eq!(
            frames.0.words[21].parse_header().unwrap().0,
            WordRecordKind::Phase
        );

        let mut forged = record.clone();
        forged.instanced_power_uid_at_before = Some(13);
        assert!(Frames::new().push_card_play(&forged).is_none());
    }

    #[test]
    fn card_play_power_snapshot_phase_matrix_rejects_every_unreachable_shape() {
        let valid = || {
            let mut record = test_card_play(7);
            record.pending_choice = false;
            record.selection_kind = None;
            record.stage = CardPlayStage::AfterImitation;
            record.latch_kind = CardPlayLatchKind::Post;
            record
        };

        let mut captured_empty = valid();
        captured_empty.after_card_played_power_snapshot_captured = true;
        captured_empty.after_card_played_power_phase = 1;
        assert!(Frames::new().push_card_play(&captured_empty).is_some());

        let mutations: [fn(&mut CardPlayRecord); 6] = [
            |record| record.after_card_played_power_phase = 1,
            |record| {
                record.after_card_played_power_snapshot_captured = true;
                record.after_card_played_power_snapshot = vec![3];
                record.after_card_played_power_cursor = 1;
            },
            |record| {
                record.after_card_played_power_snapshot_captured = true;
                record.after_card_played_power_phase = 1;
                record.after_card_played_pending_monologue_uid = Some(3);
            },
            |record| {
                record.after_card_played_power_snapshot_captured = true;
                record.after_card_played_power_snapshot = vec![3];
                record.after_card_played_power_phase = 2;
                record.after_card_played_power_cursor = 1;
                record.monologue = true;
                record.monologue_latches = vec![(3, 1)];
            },
            |record| {
                record.after_card_played_power_snapshot_captured = true;
                record.after_card_played_power_snapshot = vec![3];
                record.after_card_played_power_phase = 3;
                record.monologue = true;
                record.monologue_latches = vec![(3, 1)];
            },
            |record| {
                record.after_card_played_power_snapshot_captured = true;
                record.after_card_played_power_snapshot = vec![3];
                record.after_card_played_power_phase = 2;
                record.after_card_played_power_cursor = 1;
                record.after_card_played_pending_monologue_uid = Some(4);
            },
        ];
        for mutate in mutations {
            let mut record = valid();
            mutate(&mut record);
            assert!(Frames::new().push_card_play(&record).is_none());
        }
    }

    #[test]
    fn instanced_player_power_replacement_and_overflow_are_atomic() {
        let mut fanouts = HotFanouts::new();
        fanouts.set_next_after_side_turn_end_power_uid(8);
        let panache = [PanacheInstance {
            uid: 1,
            amount: 10,
            cards_left: 4,
            already_applied: true,
        }];
        let monologue = [MonologueInstance {
            uid: 3,
            amount: 1,
            power: 1,
            strength_applied: 2,
        }];
        assert!(fanouts.set_instanced_player_power_instances(&panache, &monologue));

        let invalid_rows = [
            (
                vec![PanacheInstance {
                    uid: 1,
                    amount: 11,
                    ..panache[0]
                }],
                monologue.to_vec(),
            ),
            (
                vec![PanacheInstance {
                    uid: 1,
                    cards_left: 0,
                    ..panache[0]
                }],
                monologue.to_vec(),
            ),
            (
                vec![PanacheInstance {
                    uid: 8,
                    ..panache[0]
                }],
                monologue.to_vec(),
            ),
            (
                panache.to_vec(),
                vec![MonologueInstance {
                    uid: 3,
                    amount: 2,
                    ..monologue[0]
                }],
            ),
            (
                panache.to_vec(),
                vec![MonologueInstance {
                    uid: 3,
                    power: 2,
                    ..monologue[0]
                }],
            ),
            (
                panache.to_vec(),
                vec![MonologueInstance {
                    uid: 3,
                    strength_applied: -1,
                    ..monologue[0]
                }],
            ),
            (
                vec![PanacheInstance {
                    uid: 3,
                    ..panache[0]
                }],
                monologue.to_vec(),
            ),
        ];
        for (panache_rows, monologue_rows) in invalid_rows {
            let before = fanouts.clone();
            assert!(fanouts.shares_store_with(&before));
            assert!(!fanouts.set_instanced_player_power_instances(&panache_rows, &monologue_rows));
            assert_eq!(fanouts, before);
            assert!(
                fanouts.shares_store_with(&before),
                "failure must not detach COW state"
            );
        }

        let mut overflow = HotFanouts::new();
        overflow.set_next_after_side_turn_end_power_uid(2);
        assert!(overflow.set_monologue_instances(&[MonologueInstance {
            uid: 1,
            amount: 1,
            power: 1,
            strength_applied: i32::MAX,
        }]));
        let before = overflow.clone();
        assert_eq!(overflow.increment_monologue_strength_applied(1), Err(()));
        assert_eq!(overflow, before);
        assert!(overflow.shares_store_with(&before));

        let before = fanouts.clone();
        assert!(!fanouts.set_monologue_instances(&[
            MonologueInstance {
                uid: 3,
                strength_applied: i32::MAX,
                ..monologue[0]
            },
            MonologueInstance {
                uid: 4,
                strength_applied: 1,
                ..monologue[0]
            },
        ]));
        assert_eq!(fanouts, before);
        assert!(fanouts.shares_store_with(&before));
    }

    #[test]
    fn card_play_word_store_preserves_frozen_imitation_listeners_and_rejects_impossible_scalars() {
        let listeners = vec![
            PhysicalAfterCardPlayedListener {
                uid: 91,
                id: CardId::BansheesCry,
            },
            PhysicalAfterCardPlayedListener {
                uid: 92,
                id: CardId::Pinpoint,
            },
        ];
        let mut record = test_card_play(7);
        record.pending_choice = false;
        record.selection_kind = None;
        record.stage = CardPlayStage::AfterImitation;
        record.latch_kind = CardPlayLatchKind::Post;
        record.physical_after_card_played_listeners = listeners.clone();
        let mut frames = Frames::new();
        let index = frames.push_card_play(&record).unwrap();
        assert_eq!(
            frames
                .card_play(index)
                .unwrap()
                .physical_after_card_played_listeners()
                .collect::<Vec<_>>(),
            listeners
        );
        assert_eq!(frames.card_play(index).unwrap().to_owned(), record);

        for mutation in [
            |record: &mut CardPlayRecord| record.plays = 0,
            |record: &mut CardPlayRecord| record.plays = 257,
            |record: &mut CardPlayRecord| record.play_index = record.plays,
            |record: &mut CardPlayRecord| record.spent = -1,
            |record: &mut CardPlayRecord| record.spent = i32::from(i16::MAX) + 1,
            |record: &mut CardPlayRecord| record.force_exhaust = true,
        ] {
            let mut malformed = test_card_play(8);
            mutation(&mut malformed);
            assert!(Frames::new().push_card_play(&malformed).is_none());
        }

        let mut wrong_stage = record.clone();
        wrong_stage.stage = CardPlayStage::AfterNormalHook;
        assert!(Frames::new().push_card_play(&wrong_stage).is_none());
        let mut duplicate = record.clone();
        duplicate
            .physical_after_card_played_listeners
            .push(listeners[0]);
        assert!(Frames::new().push_card_play(&duplicate).is_none());
        for stage in [CardPlayStage::StartBody, CardPlayStage::AfterNormalHook] {
            let mut impossible_lamp = test_card_play(9);
            impossible_lamp.pending_choice = false;
            impossible_lamp.selection_kind = None;
            impossible_lamp.stage = stage;
            impossible_lamp.latch_kind = match stage {
                CardPlayStage::StartBody => CardPlayLatchKind::Empty,
                CardPlayStage::AfterNormalHook => CardPlayLatchKind::Post,
                _ => unreachable!(),
            };
            impossible_lamp.lamp_owner = true;
            assert!(Frames::new().push_card_play(&impossible_lamp).is_none());
        }
    }

    #[test]
    fn frame_store_equality_ignores_capacity_and_construction_history() {
        let mut compact = Frames::new();
        compact.push(Frame::JossSideEnd);
        compact.push_card_play(&test_card_play(7)).unwrap();
        let roomy = Frames(Arc::new(FrameStore {
            frames: compact.0.frames.to_vec(),
            words: compact.0.words.to_vec(),
        }));
        assert_eq!(compact, roomy);
    }

    #[test]
    fn card_play_decoder_rejects_noncanonical_kind_expos_and_extents() {
        let mut valid = Frames::new();
        let index = valid.push_card_play(&test_card_play(7)).unwrap();
        let pending = pending_for_record(7, index);
        assert!(valid.pending_choice_is_valid(&pending));

        for ordinal in [11_u32, 254] {
            let mut mutation = valid.clone();
            let word = &mut Arc::make_mut(&mut mutation.0).words[12];
            word.meta = (word.meta & !0xff) | ordinal;
            assert!(mutation.card_play(index).is_none(), "ordinal {ordinal}");
        }
        let mut absent = test_card_play(8);
        absent.pending_choice = false;
        absent.selection_kind = None;
        let mut absent_frames = Frames::new();
        let absent_index = absent_frames.push_card_play(&absent).unwrap();
        assert_eq!(
            absent_frames
                .card_play(absent_index)
                .unwrap()
                .selection_kind,
            None
        );

        let mut bad_expos = test_card_play(9);
        bad_expos.expos = Some(u32::MAX);
        assert!(Frames::new().push_card_play(&bad_expos).is_none());

        for mutation in [
            |store: &mut FrameStore| store.words[0].meta ^= 1 << 8,
            |store: &mut FrameStore| store.words[0].meta |= 1 << 16,
            |store: &mut FrameStore| store.words[0].body = u32::MAX,
            |store: &mut FrameStore| store.words.push(PendingWord::default()),
        ] {
            let mut malformed = valid.clone();
            mutation(Arc::make_mut(&mut malformed.0));
            assert!(!malformed.continuation_store_is_valid(Some(&pending)));
        }

        let mut all_empty_present = valid.clone();
        Arc::make_mut(&mut all_empty_present.0).words[12].meta |= 1 << 11;
        assert!(all_empty_present.card_play(index).is_none());

        let wrong_uid = pending_for_record(8, index);
        assert!(!valid.pending_choice_is_valid(&wrong_uid));
        let mut non_top = valid.clone();
        non_top.push(Frame::JossSideEnd);
        assert!(!non_top.pending_choice_is_valid(&pending));

        let mut with_subrecords = test_card_play(10);
        with_subrecords.selection_cards.push(HotCard {
            uid: 11,
            atom: 2,
            flags: CARD_FLAG_PICK,
        });
        with_subrecords.strangle_latches.push((91, 3));
        let mut encoded = Frames::new();
        let encoded_index = encoded.push_card_play(&with_subrecords).unwrap();
        let encoded_pending = pending_for_record(10, encoded_index);
        assert!(encoded.pending_choice_is_valid(&encoded_pending));
        for mutation in [
            |store: &mut FrameStore| store.words[14].meta = store.words[16].meta,
            |store: &mut FrameStore| store.words[16].meta = store.words[14].meta,
            |store: &mut FrameStore| store.words[14].meta = (store.words[14].meta & !0xff) | 7,
            |store: &mut FrameStore| store.words[14].body = 0,
            |store: &mut FrameStore| store.words[0].body -= 1,
        ] {
            let mut malformed = encoded.clone();
            mutation(Arc::make_mut(&mut malformed.0));
            assert!(!malformed.continuation_store_is_valid(Some(&encoded_pending)));
        }

        let mut aliased = encoded.clone();
        aliased.push(Frame::CardPlay {
            record: encoded_index,
        });
        assert!(!aliased.continuation_store_is_valid(Some(&encoded_pending)));
    }

    #[test]
    fn phase_and_stampede_live_records_are_canonical_cow_suffixes() {
        let phase = AutoPostPhaseRecord {
            cursor: 1,
            listeners: vec![
                AutoPostListener::Stampede,
                AutoPostListener::Howl { uid: 17 },
            ],
        };
        let mut frames = Frames::new();
        let phase_index = frames.push_auto_post_phase(&phase).unwrap();
        assert_eq!(
            frames.auto_post_phase(phase_index).unwrap().to_owned(),
            phase
        );
        let live = StampedeLiveRecord {
            cursor: 1,
            iterations: 3,
        };
        let live_index = frames.push_stampede_live(live).unwrap();
        assert_eq!(frames.stampede_live(live_index), Some(live));
        assert!(frames.continuation_store_is_valid(None));

        let original = frames.clone();
        assert!(original.shares_store_with(&frames));
        frames
            .replace_top_stampede_live(StampedeLiveRecord {
                cursor: 2,
                iterations: 3,
            })
            .unwrap();
        assert!(!original.shares_store_with(&frames));
        assert_eq!(original.stampede_live(live_index), Some(live));
        assert_eq!(
            frames.pop_top_stampede_live(),
            Some(StampedeLiveRecord {
                cursor: 2,
                iterations: 3,
            })
        );
        assert_eq!(frames.pop_top_auto_post_phase(), Some(phase));
        assert!(frames.is_empty());
        assert!(frames.0.words.is_empty());

        for invalid in [
            AutoPostPhaseRecord {
                cursor: 0,
                listeners: Vec::new(),
            },
            AutoPostPhaseRecord {
                cursor: 0,
                listeners: vec![
                    AutoPostListener::Stampede,
                    AutoPostListener::Howl { uid: 1 },
                    AutoPostListener::Howl { uid: 1 },
                ],
            },
            AutoPostPhaseRecord {
                cursor: 2,
                listeners: vec![AutoPostListener::Stampede],
            },
        ] {
            assert!(Frames::new().push_auto_post_phase(&invalid).is_none());
        }
        for invalid in [
            StampedeLiveRecord {
                cursor: 0,
                iterations: 0,
            },
            StampedeLiveRecord {
                cursor: 2,
                iterations: 1,
            },
        ] {
            assert!(Frames::new().push_stampede_live(invalid).is_none());
        }

        let bombardment = AutoPreBombardmentPhaseRecord {
            cursor: 1,
            listeners: vec![41, 73],
        };
        let mut frames = Frames::new();
        let bombardment_index = frames
            .push_auto_pre_bombardment_phase(&bombardment.listeners)
            .unwrap();
        frames
            .replace_top_auto_pre_bombardment_phase(&bombardment)
            .unwrap();
        assert_eq!(
            frames
                .auto_pre_bombardment_phase(bombardment_index)
                .unwrap()
                .to_owned(),
            bombardment
        );
        assert!(frames.continuation_store_is_valid(None));
        let original = frames.clone();
        let finished = AutoPreBombardmentPhaseRecord {
            cursor: 2,
            listeners: vec![41, 73],
        };
        frames
            .replace_top_auto_pre_bombardment_phase(&finished)
            .unwrap();
        assert!(!original.shares_store_with(&frames));
        assert_eq!(frames.pop_top_auto_pre_bombardment_phase(), Some(finished));
        assert!(frames.is_empty());

        for listeners in [Vec::new(), vec![0], vec![7, 7]] {
            assert!(
                Frames::new()
                    .push_auto_pre_bombardment_phase(&listeners)
                    .is_none()
            );
        }
    }

    /// #3309: the History Course AutoPre phase names one dupe uid, is
    /// disjoint from the Mayhem quotient, and pops only as itself.
    #[test]
    fn history_course_phase_record_round_trips_and_is_disjoint_from_mayhem() {
        let mut frames = Frames::new();
        let mayhem = frames.push_auto_pre_mayhem_phase().unwrap();
        let history = frames.push_auto_pre_history_course_phase(41).unwrap();
        assert_eq!(frames.auto_pre_history_course_phase(history), Some(41));
        assert_eq!(frames.auto_pre_mayhem_phase(history), None);
        assert!(frames.auto_pre_bombardment_phase(history).is_none());
        assert!(frames.auto_post_phase(history).is_none());
        assert_eq!(frames.auto_pre_history_course_phase(mayhem), None);
        assert!(frames.continuation_store_is_valid(None));
        assert_eq!(frames.pop_top_auto_pre_mayhem_phase(), None);
        assert_eq!(frames.pop_top_auto_pre_history_course_phase(), Some(41));
        assert_eq!(frames.pop_top_auto_pre_history_course_phase(), None);
        assert_eq!(
            frames.pop_top_auto_pre_mayhem_phase(),
            Some(AutoPreMayhemPhaseRecord)
        );
        assert!(frames.is_empty());
        for uid in [0, LEGACY_CARD_UID] {
            assert!(
                Frames::new()
                    .push_auto_pre_history_course_phase(uid)
                    .is_none()
            );
        }
    }

    #[test]
    fn restricted_auto_pre_and_frozen_batch_records_are_canonical_cow_suffixes() {
        let mut frames = Frames::new();
        let phase_index = frames.push_auto_pre_mayhem_phase().unwrap();
        assert_eq!(
            frames.auto_pre_mayhem_phase(phase_index),
            Some(AutoPreMayhemPhaseRecord)
        );
        let mut full_state = CardInstanceState {
            enchantment_state: OptionalNonNegativeI32::from_option(Some(7)).unwrap(),
            base_replay_count: BaseReplayCount::from_option(Some(3)).unwrap(),
            damage_growth: 0,
            local_cost_modifiers: LocalCostModifiers::from_rows(vec![
                LocalCostModifier {
                    kind: LocalCostModifierKind::Set,
                    amount: -9,
                    expiration: LocalCostExpiration::ThisCombat,
                    reduce_only: false,
                },
                LocalCostModifier {
                    kind: LocalCostModifierKind::Add,
                    amount: i64::MAX,
                    expiration: LocalCostExpiration::ThisTurnOrPlayed,
                    reduce_only: true,
                },
            ]),
            free_star_cost_this_turn_or_played_rows: 2,
            genetic_algorithm: GeneticAlgorithmState::from_parts(17, Some(23)).unwrap(),
            local_retain: true,
            local_sly: true,
            transient_retain: true,
        };
        full_state.set_local_ethereal(true);
        full_state.set_transient_sly(true);
        full_state
            .set_fraction_damage_growth(crate::decimal::DotNetDecimal::ratio(7, 3).unwrap())
            .unwrap();
        full_state
            .local_cost_modifiers
            .set_free_star_cost_this_combat()
            .unwrap();
        let record = FrozenAutoBatchRecord {
            entries: vec![
                FrozenAutoBatchEntry {
                    card: HotCard {
                        uid: 7,
                        atom: 2,
                        flags: 0,
                    },
                    state: full_state,
                },
                FrozenAutoBatchEntry {
                    card: HotCard {
                        uid: 9,
                        atom: 3,
                        flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                    },
                    state: CardInstanceState::default(),
                },
            ],
            cursor: 1,
            gather_target: 0,
            force_exhaust: false,
            source: FrozenAutoBatchSource::DrawPileFlip,
        };
        let batch_index = frames.push_frozen_auto_batch(&record).unwrap();
        let batch = frames.frozen_auto_batch(batch_index).unwrap();
        assert_eq!(batch.uids().collect::<Vec<_>>(), [7, 9]);
        assert_eq!(batch.entries().collect::<Vec<_>>(), record.entries);
        assert_eq!(batch.cursor, 1);
        assert_eq!(batch.source, FrozenAutoBatchSource::DrawPileFlip);
        assert!(!batch.force_exhaust);
        assert!(frames.continuation_store_is_valid(None));

        for (source, force_exhaust) in [
            (FrozenAutoBatchSource::DrawPileFlip, false),
            (FrozenAutoBatchSource::DrawPileFlip, true),
            (FrozenAutoBatchSource::SlyDiscard, false),
            (FrozenAutoBatchSource::BeatDown, false),
            (FrozenAutoBatchSource::Eidolon, true),
            (FrozenAutoBatchSource::KnifeTrap, false),
        ] {
            let mut variant = record.clone();
            variant.source = source;
            variant.force_exhaust = force_exhaust;
            let mut variant_frames = Frames::new();
            let index = variant_frames.push_frozen_auto_batch(&variant).unwrap();
            assert_eq!(
                variant_frames.frozen_auto_batch(index).unwrap().to_owned(),
                variant
            );
        }

        let mut unit_fraction_record = record.clone();
        unit_fraction_record.entries[1]
            .state
            .set_fraction_damage_growth(crate::decimal::DotNetDecimal::from_i64(7))
            .unwrap();
        let mut unit_fraction_frames = Frames::new();
        let unit_fraction_index = unit_fraction_frames
            .push_frozen_auto_batch(&unit_fraction_record)
            .unwrap();
        assert_eq!(
            unit_fraction_frames
                .frozen_auto_batch(unit_fraction_index)
                .unwrap()
                .to_owned(),
            unit_fraction_record
        );

        let original = frames.clone();
        assert!(original.shares_store_with(&frames));
        frames.push(Frame::JossSideEnd);
        assert!(!original.shares_store_with(&frames));
        assert_eq!(original.as_slice().len(), 2);
        assert_eq!(frames.as_slice().len(), 3);

        for malformed in [
            FrozenAutoBatchRecord {
                entries: Vec::new(),
                ..record.clone()
            },
            FrozenAutoBatchRecord {
                entries: vec![record.entries[0].clone(), record.entries[0].clone()],
                ..record.clone()
            },
            FrozenAutoBatchRecord {
                cursor: 3,
                ..record.clone()
            },
            FrozenAutoBatchRecord {
                force_exhaust: true,
                source: FrozenAutoBatchSource::SlyDiscard,
                ..record.clone()
            },
        ] {
            assert!(Frames::new().push_frozen_auto_batch(&malformed).is_none());
        }

        let batch_start = batch_index.offset().unwrap();
        for mutation_index in 0..26 {
            let mut malformed = original.clone();
            let store = Arc::make_mut(&mut malformed.0);
            match mutation_index {
                0 => store.words[batch_start].meta ^= 1,
                1 => store.words[batch_start + 1].body = 3,
                2 => store.words[batch_start + 2].meta ^= 1,
                3 => store.words[batch_start + 3].meta |= 1 << 31,
                4 => store.words[batch_start + 4].body = 0x8000_0001,
                5 => store.words[batch_start + 4].meta = i32::MIN as u32,
                6 => store.words[batch_start + 5].body = i32::MAX as u32 + 1,
                7 => store.words[batch_start + 5].meta |= 1 << 31,
                8 => store.words[batch_start + 8].meta = 29,
                9 => store.words[batch_start + 10].body = 3,
                10 => store.words[batch_start + 10].meta = 1,
                11 => store.words[batch_start + 14].body = store.words[batch_start + 3].body,
                12 => store.words.push(PendingWord::default()),
                13 => store.words[batch_start].body -= 1,
                14 => store.words[batch_start].body += 1,
                15 => store.words[batch_start + 1].meta |= 1 << 31,
                16 => store.words[batch_start + 2].body = 5,
                17 => store.words[batch_start + 2].body += 1,
                18 => store.words[batch_start + 10].body = 1 << 8,
                19 => store.words[batch_start + 5].meta &= !(1 << 11),
                20 => store.words[batch_start + 5].body = 1,
                21 => store.words[batch_start + 18].body = 1,
                22 => store.words[batch_start].meta |= 1 << 16,
                23 => store.words[batch_start + 2].meta |= 1 << 16,
                24 => store.words[batch_start + 6].meta &= !(1 << 31),
                25 => {
                    store.words[batch_start + 5].meta &= !(1 << 12);
                    store.words[batch_start + 7] = PendingWord { body: 7, meta: 0 };
                    store.words[batch_start + 8] = PendingWord { body: 0, meta: 0 };
                }
                _ => unreachable!(),
            }
            assert!(
                !malformed.continuation_store_is_valid(None),
                "restricted record mutation {mutation_index}"
            );
        }
    }

    #[test]
    fn turn_start_hand_choice_record_preserves_full_aux_payload_and_same_record_replacement() {
        let state = CardInstanceState {
            local_cost_modifiers: LocalCostModifiers::from_rows(vec![LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: -3,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: true,
            }]),
            local_sly: true,
            free_star_cost_this_turn_or_played_rows: 2,
            ..CardInstanceState::default()
        };
        let entries = vec![
            FrozenAutoBatchEntry {
                card: HotCard {
                    uid: 41,
                    atom: 2,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
                },
                state: state.clone(),
            },
            FrozenAutoBatchEntry {
                card: HotCard {
                    uid: 42,
                    atom: 3,
                    flags: 0,
                },
                state: CardInstanceState::default(),
            },
        ];
        let selecting = TurnStartHandChoiceRecord {
            listener_order: vec![TurnStartHandChoiceKind::ToolsOfTheTrade],
            appended_suffix: Vec::new(),
            kind: TurnStartHandChoiceKind::ToolsOfTheTrade,
            amount: 1,
            next_listener: 1,
            cursor: 0,
            phase: TurnStartHandChoicePhase::Selecting,
            entries: entries.clone(),
        };
        let mut frames = Frames::new();
        let index = frames.push_turn_start_hand_choice(&selecting).unwrap();
        let pending = PendingSelection {
            frame_uid: TURN_START_HAND_CHOICE_PENDING_UID,
            frame_record: index,
        };
        assert_eq!(
            frames.turn_start_hand_choice(index).unwrap().to_owned(),
            selecting
        );
        assert!(frames.pending_turn_start_hand_choice_is_valid(&pending));
        for (mutation_index, mutation) in [
            |store: &mut FrameStore| store.words[1].meta |= 1 << 22,
            |store: &mut FrameStore| store.words[1].meta |= 1 << 31,
            |store: &mut FrameStore| store.words[2].meta = 2,
            |store: &mut FrameStore| store.words[2].body = 0,
        ]
        .into_iter()
        .enumerate()
        {
            let mut malformed = frames.clone();
            mutation(Arc::make_mut(&mut malformed.0));
            assert!(
                !malformed.continuation_store_is_valid(Some(&pending)),
                "turn-start choice record mutation {mutation_index}"
            );
        }

        let effect = TurnStartHandChoiceRecord {
            listener_order: vec![
                TurnStartHandChoiceKind::Loop,
                TurnStartHandChoiceKind::RollingBoulder,
                TurnStartHandChoiceKind::SummonNextTurn,
                TurnStartHandChoiceKind::Inferno,
                TurnStartHandChoiceKind::CrimsonMantle,
                TurnStartHandChoiceKind::ToolsOfTheTrade,
                TurnStartHandChoiceKind::Tyranny,
                TurnStartHandChoiceKind::Entropy,
            ],
            kind: TurnStartHandChoiceKind::Entropy,
            next_listener: 8,
            entries: vec![entries[0].clone()],
            cursor: 1,
            phase: TurnStartHandChoicePhase::Effect,
            ..selecting
        };
        frames.replace_top_turn_start_hand_choice(&effect).unwrap();
        assert_eq!(
            frames.top(),
            Some(Frame::TurnStartHandChoice { record: index })
        );
        assert_eq!(
            frames.turn_start_hand_choice(index).unwrap().to_owned(),
            effect
        );
        assert_eq!(frames.pop_top_turn_start_hand_choice().unwrap(), effect);

        let max_width_reapplied_summon = TurnStartHandChoiceRecord {
            listener_order: vec![
                TurnStartHandChoiceKind::SummonNextTurn,
                TurnStartHandChoiceKind::ToolsOfTheTrade,
                TurnStartHandChoiceKind::Tyranny,
                TurnStartHandChoiceKind::Loop,
                TurnStartHandChoiceKind::RollingBoulder,
                TurnStartHandChoiceKind::Inferno,
                TurnStartHandChoiceKind::CrimsonMantle,
                TurnStartHandChoiceKind::Entropy,
            ],
            appended_suffix: vec![TurnStartHandChoiceKind::SummonNextTurn],
            kind: TurnStartHandChoiceKind::ToolsOfTheTrade,
            amount: 2,
            next_listener: 2,
            cursor: 2,
            phase: TurnStartHandChoicePhase::Effect,
            entries,
        };
        let mut frames = Frames::new();
        let index = frames
            .push_turn_start_hand_choice(&max_width_reapplied_summon)
            .unwrap();
        assert_eq!(
            frames.turn_start_hand_choice(index).unwrap().to_owned(),
            max_width_reapplied_summon
        );
        let mut forbidden_high_bits = frames.clone();
        Arc::make_mut(&mut forbidden_high_bits.0).words[index.offset().unwrap() + 3].meta |=
            1 << 31;
        assert!(!forbidden_high_bits.continuation_store_is_valid(None));
        assert_eq!(
            frames.pop_top_turn_start_hand_choice().unwrap(),
            max_width_reapplied_summon
        );
        assert!(frames.is_empty());
    }

    #[test]
    fn auto_post_invincible_words_roundtrip_and_reject_cross_kind_duplicate_uids() {
        let phase = AutoPostPhaseRecord {
            cursor: 1,
            listeners: vec![
                AutoPostListener::Invincible { uid: 17 },
                AutoPostListener::Howl { uid: 18 },
            ],
        };
        let mut frames = Frames::new();
        let index = frames.push_auto_post_phase(&phase).unwrap();
        assert_eq!(frames.auto_post_phase(index).unwrap().to_owned(), phase);
        let mut duplicated = phase.clone();
        duplicated.listeners[1] = AutoPostListener::Howl { uid: 17 };
        assert!(frames.push_auto_post_phase(&duplicated).is_none());
        Arc::make_mut(&mut frames.0).words[3].body = 17;
        assert!(frames.auto_post_phase(index).is_none());
    }

    #[test]
    fn phase_and_stampede_live_decoders_reject_every_header_and_body_drift() {
        let phase = AutoPostPhaseRecord {
            cursor: 0,
            listeners: vec![
                AutoPostListener::Stampede,
                AutoPostListener::Howl { uid: 17 },
                AutoPostListener::Howl { uid: 18 },
            ],
        };
        let mut valid_phase = Frames::new();
        let phase_index = valid_phase.push_auto_post_phase(&phase).unwrap();
        assert_eq!(
            valid_phase.auto_post_phase(phase_index).unwrap().to_owned(),
            phase
        );
        for (mutation_index, mutation) in [
            |store: &mut FrameStore| store.words[0].meta ^= 1,
            |store: &mut FrameStore| store.words[0].meta ^= 1 << 8,
            |store: &mut FrameStore| store.words[0].meta |= 1 << 16,
            |store: &mut FrameStore| store.words[0].body -= 1,
            |store: &mut FrameStore| store.words[1].meta = 1,
            |store: &mut FrameStore| store.words[1].body = 4,
            |store: &mut FrameStore| store.words[2].body = 1,
            |store: &mut FrameStore| store.words[3].meta = 3,
            |store: &mut FrameStore| store.words[4].body = 17,
            |store: &mut FrameStore| store.words.push(PendingWord::default()),
        ]
        .into_iter()
        .enumerate()
        {
            let mut malformed = valid_phase.clone();
            mutation(Arc::make_mut(&mut malformed.0));
            assert!(
                !malformed.continuation_store_is_valid(None),
                "phase mutation {mutation_index}"
            );
        }

        let mut valid_live = Frames::new();
        let live_index = valid_live
            .push_stampede_live(StampedeLiveRecord {
                cursor: 1,
                iterations: 2,
            })
            .unwrap();
        assert_eq!(
            valid_live.stampede_live(live_index),
            Some(StampedeLiveRecord {
                cursor: 1,
                iterations: 2,
            })
        );
        for (mutation_index, mutation) in [
            |store: &mut FrameStore| store.words[0].meta ^= 1,
            |store: &mut FrameStore| store.words[0].meta ^= 1 << 8,
            |store: &mut FrameStore| store.words[0].meta |= 1 << 16,
            |store: &mut FrameStore| store.words[0].body = 2,
            |store: &mut FrameStore| store.words[1].meta = 0,
            |store: &mut FrameStore| store.words[1].body = 3,
            |store: &mut FrameStore| store.words.push(PendingWord::default()),
        ]
        .into_iter()
        .enumerate()
        {
            let mut malformed = valid_live.clone();
            mutation(Arc::make_mut(&mut malformed.0));
            assert!(
                !malformed.continuation_store_is_valid(None),
                "live mutation {mutation_index}"
            );
        }
    }

    #[cfg(feature = "allocation-counting")]
    #[test]
    fn ordinary_hot_state_clones_never_allocate_a_continuation_store() {
        use crate::allocation::thread_snapshot;

        let state = HotState::at_defaults();
        assert!(state.pending.is_none());
        let _warmed = state.clone();
        let (before_allocations, before_bytes) = thread_snapshot();
        let cloned = state.clone();
        let (after_allocations, after_bytes) = thread_snapshot();
        assert!(cloned.pending.is_none());
        assert_eq!(after_allocations, before_allocations);
        assert_eq!(after_bytes, before_bytes);
    }

    #[cfg(feature = "allocation-counting")]
    #[test]
    fn scalar_frames_do_not_allocate_words_and_real_choices_allocate_two_vectors() {
        use crate::allocation::thread_snapshot;

        let before_default = thread_snapshot();
        let empty = std::hint::black_box(Frames::new());
        let after_default = thread_snapshot();
        assert_eq!(after_default.0 - before_default.0, 1);
        assert!(empty.is_empty());
        assert_eq!(empty.0.words.capacity(), 0);

        let mut scalar = Frames::new();
        let before_scalar = thread_snapshot();
        scalar.push(Frame::JossSideEnd);
        let after_scalar = thread_snapshot();
        assert_eq!(after_scalar.0 - before_scalar.0, 1);
        assert_eq!(scalar.0.words.capacity(), 0);

        let mut choice = Frames::new();
        let card = HotCard {
            uid: 9,
            atom: 3,
            flags: CARD_FLAG_PICK,
        };
        let mut record = test_card_play(7);
        record.selection_cards.push(card);
        record.strangle_latches.push((11, 2));
        let before_choice = thread_snapshot();
        choice.push_card_play(&record).unwrap();
        let after_choice = thread_snapshot();
        assert_eq!(after_choice.0 - before_choice.0, 2);
        assert!(choice.0.frames.capacity() >= 1);
        assert!(choice.0.words.capacity() >= 5);
    }

    #[test]
    fn four_byte_optional_carriers_preserve_every_boundary_value() {
        for value in [None, Some(0), Some(1), Some(i32::MAX)] {
            let encoded = OptionalNonNegativeI32::from_option(value).unwrap();
            assert_eq!(encoded.get(), value);
        }
        assert!(OptionalNonNegativeI32::from_option(Some(-1)).is_none());

        for value in [None, Some(0), Some(1), Some(i32::MAX)] {
            let encoded = BaseReplayCount::from_option(value).unwrap();
            assert_eq!(encoded.get(), value);
        }
        assert!(BaseReplayCount::from_option(Some(-1)).is_none());

        let mut explicit_zero = CardInstanceState::default();
        explicit_zero.set_base_replay_count(Some(0)).unwrap();
        assert_eq!(explicit_zero.base_replay_count(), Some(0));
        assert!(!explicit_zero.is_vacant());
        assert_ne!(explicit_zero, CardInstanceState::default());

        fn hash(value: &CardInstanceState) -> u64 {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            value.hash(&mut hasher);
            hasher.finish()
        }
        let absent = CardInstanceState::default();
        assert_ne!(hash(&absent), hash(&explicit_zero));
        for amount in [0, i32::MAX] {
            let enchanted = CardInstanceState {
                enchantment_state: OptionalNonNegativeI32::from_option(Some(amount)).unwrap(),
                ..CardInstanceState::default()
            };
            assert_ne!(hash(&absent), hash(&enchanted));
            assert_eq!(enchanted.clone(), enchanted);
        }
    }

    #[test]
    fn sword_sage_listener_restacks_in_place_and_unregisters_among_peers() {
        let mut fanouts = HotFanouts::new();
        assert!(fanouts.set_after_power_amount_changed_order(&[
            PowerId::Shroud,
            PowerId::SwordSage,
            PowerId::Vicious,
        ]));
        assert!(fanouts.register_after_power_amount_changed(PowerId::SwordSage));
        assert_eq!(
            fanouts.after_power_amount_changed_order(),
            [PowerId::Shroud, PowerId::SwordSage, PowerId::Vicious]
        );
        fanouts.unregister_after_power_amount_changed(PowerId::SwordSage);
        assert_eq!(
            fanouts.after_power_amount_changed_order(),
            [PowerId::Shroud, PowerId::Vicious]
        );
        assert_eq!(size_of::<FanoutState>(), 312);
    }

    #[test]
    fn after_side_turn_start_power_capacity_matches_current_concrete_census() {
        // Current-build Hook.AfterSideTurnStart has nineteen concrete power
        // types. Clarity has no Rust PowerId, leaving these exact eighteen
        // representable members. Sandpit is a separate Late-hook member.
        let current = [
            PowerId::BiasedCognition,
            PowerId::Blur,
            PowerId::Coolant,
            PowerId::Countdown,
            PowerId::DemonForm,
            PowerId::DrawNextTurn,
            PowerId::Feral,
            PowerId::Furnace,
            PowerId::Neurosurge,
            PowerId::NoxiousFumes,
            PowerId::Plating,
            PowerId::Poison,
            PowerId::PrepTime,
            PowerId::Rampart,
            PowerId::Reflect,
            PowerId::ShadowStep,
            PowerId::Slow,
            PowerId::WraithForm,
        ];
        assert_eq!(current.len() + 1, MAX_AFTER_SIDE_TURN_START_POWERS);
        assert_eq!(
            AfterSideTurnStartToken::ALL.len(),
            MAX_AFTER_SIDE_TURN_START_POWERS
        );
        assert_eq!(
            current,
            crate::steps::regent_forge::AFTER_SIDE_TURN_START_POWER_CENSUS,
            "the carrier width and source-derived power census must move together"
        );
    }

    #[test]
    fn empty_after_side_turn_start_order_fast_path_preserves_validation() {
        let mut state = HotState::at_defaults();
        assert_eq!(
            state.after_side_turn_start_power_order().unwrap().len(),
            0,
            "the no-power case is a complete empty listener order"
        );

        // An unrelated live power must still take the validating path and
        // remains a valid empty listener order.
        state.powers.set(PowerId::Strength, SlotWire::Int, 1);
        assert_eq!(state.after_side_turn_start_power_order().unwrap().len(), 0);

        // A live member without its required acquisition record is malformed,
        // so the shortcut cannot accidentally accept it.
        state.powers.set(PowerId::BiasedCognition, SlotWire::Int, 1);
        assert!(state.after_side_turn_start_power_order().is_none());
    }

    #[test]
    fn empty_after_player_turn_start_order_fast_path_preserves_validation() {
        let mut state = HotState::at_defaults();
        assert_eq!(state.after_player_turn_start_order().unwrap().len(), 0);

        // An unrelated live power remains a valid empty listener order.
        state.powers.set(PowerId::Strength, SlotWire::Int, 1);
        assert_eq!(state.after_player_turn_start_order().unwrap().len(), 0);

        // A live ordered member without its acquisition record remains
        // malformed; it cannot take the empty-state shortcut.
        state.powers.set(PowerId::Loop, SlotWire::Int, 1);
        assert!(state.after_player_turn_start_order().is_some());
        assert!(!crate::engine::turn::after_player_turn_start_power_order_is_exact(&state));
    }

    #[test]
    fn full_after_side_and_player_turn_start_orders_coexist_and_rotate_cow() {
        let mut state = HotState::at_defaults();
        for token in AfterSideTurnStartToken::ALL {
            if let Some(power) = token.power() {
                state.powers.set(power, SlotWire::Int, 1);
            } else {
                assert!(state.fanouts.set_clarity(1));
            }
        }
        let side = AfterSideTurnStartToken::ALL;
        assert!(state.set_after_side_turn_start_power_order(&side));

        let player = [
            PowerId::Loop,
            PowerId::RollingBoulder,
            PowerId::SummonNextTurn,
            PowerId::Inferno,
            PowerId::CrimsonMantle,
            PowerId::ToolsOfTheTrade,
            PowerId::Tyranny,
            PowerId::Entropy,
        ];
        for power in player {
            state.powers.set(power, SlotWire::Int, 1);
        }
        assert!(state.set_after_player_turn_start_order(&player));
        assert_eq!(&*state.after_side_turn_start_power_order().unwrap(), &side);
        assert_eq!(&*state.after_player_turn_start_order().unwrap(), &player);

        let sibling = state.clone();
        let mut rotated = side;
        rotated.rotate_left(7);
        assert!(state.set_after_side_turn_start_power_order(&rotated));
        assert_eq!(
            &*state.after_side_turn_start_power_order().unwrap(),
            &rotated
        );
        assert_eq!(
            &*sibling.after_side_turn_start_power_order().unwrap(),
            &side
        );
        assert_eq!(&*state.after_player_turn_start_order().unwrap(), &player);
        assert_eq!(size_of::<FanoutState>(), 312);
        assert_eq!(size_of::<HotState>(), 224);
        assert_eq!(size_of::<crate::engine::Action>(), 12);
        assert_eq!(size_of::<Frame>(), 8);
    }

    #[test]
    fn after_side_registration_restacks_remove_and_reacquire_without_reordering() {
        let mut fanouts = HotFanouts::default();
        for token in AfterSideTurnStartToken::ALL {
            match token.power() {
                Some(power) => assert!(fanouts.register_after_side_turn_start(power)),
                None => assert!(fanouts.set_clarity(1)),
            }
        }
        assert_eq!(
            fanouts.after_side_turn_start_order(),
            AfterSideTurnStartToken::ALL
        );
        for token in AfterSideTurnStartToken::ALL {
            match token.power() {
                Some(power) => assert!(fanouts.register_after_side_turn_start(power)),
                None => assert!(fanouts.set_clarity(2)),
            }
        }
        assert_eq!(
            fanouts.after_side_turn_start_order(),
            AfterSideTurnStartToken::ALL
        );

        fanouts.unregister_after_side_turn_start(PowerId::Countdown);
        assert!(
            !fanouts
                .after_side_turn_start_order()
                .contains(&AfterSideTurnStartToken::Countdown)
        );
        assert!(fanouts.register_after_side_turn_start(PowerId::Countdown));
        assert_eq!(
            fanouts.after_side_turn_start_order().last(),
            Some(&AfterSideTurnStartToken::Countdown)
        );
    }

    #[test]
    fn clarity_order_restacks_removes_reacquires_and_detaches_nested_cow() {
        let mut state = HotState::at_defaults();
        state.powers.set(PowerId::DemonForm, SlotWire::Int, 3);
        assert!(
            state
                .fanouts
                .register_after_side_turn_start(PowerId::DemonForm)
        );
        let sibling = state.clone();
        assert!(state.fanouts.shares_store_with(&sibling.fanouts));
        assert!(
            state
                .fanouts
                .potion_belt_shares_store_with(&sibling.fanouts)
        );

        assert!(state.fanouts.set_clarity(2));
        assert!(!state.fanouts.shares_store_with(&sibling.fanouts));
        assert!(
            !state
                .fanouts
                .potion_belt_shares_store_with(&sibling.fanouts)
        );
        assert_eq!(
            &*state.after_side_turn_start_power_order().unwrap(),
            &[
                AfterSideTurnStartToken::DemonForm,
                AfterSideTurnStartToken::Clarity,
            ]
        );
        assert_ne!(state, sibling);

        // A restack changes only Amount, never the insertion ordinal.
        assert!(state.fanouts.set_clarity(5));
        assert_eq!(
            &*state.after_side_turn_start_power_order().unwrap(),
            &[
                AfterSideTurnStartToken::DemonForm,
                AfterSideTurnStartToken::Clarity,
            ]
        );
        assert!(state.fanouts.set_clarity(0));
        assert_eq!(
            &*state.after_side_turn_start_power_order().unwrap(),
            &[AfterSideTurnStartToken::DemonForm]
        );
        assert!(state.fanouts.set_clarity(1));
        assert_eq!(
            &*state.after_side_turn_start_power_order().unwrap(),
            &[
                AfterSideTurnStartToken::DemonForm,
                AfterSideTurnStartToken::Clarity,
            ]
        );
        assert_eq!(sibling.fanouts.clarity(), 0);
        assert_eq!(
            sibling.fanouts.after_side_turn_start_order(),
            [AfterSideTurnStartToken::DemonForm]
        );

        let mut stale = state;
        stale
            .fanouts
            .potion_belt_mut()
            .reserved_after_side_turn_start = 4;
        assert!(!stale.fanouts.potion_belt_topology_is_exact());
    }

    #[test]
    fn pending_selection_vocabulary_is_exactly_five_piles_and_eleven_kinds() {
        assert_eq!(
            PileId::ALL,
            [
                PileId::Discard,
                PileId::Draw,
                PileId::Exhaust,
                PileId::Hand,
                PileId::Play,
            ]
        );
        assert_eq!(PileId::COUNT, 5);
        assert_eq!(
            PendingSelectionKind::ALL,
            [
                PendingSelectionKind::Replay,
                PendingSelectionKind::Program,
                PendingSelectionKind::Purity,
                PendingSelectionKind::SeekerStrike,
                PendingSelectionKind::Abundance,
                PendingSelectionKind::Discovery,
                PendingSelectionKind::HandCap,
                PendingSelectionKind::Quasar,
                PendingSelectionKind::Glimmer,
                PendingSelectionKind::Splash,
                PendingSelectionKind::Tutor,
            ]
        );
        assert_eq!(PendingSelectionKind::COUNT, 11);
    }

    #[test]
    fn every_pending_route_consumer_uses_the_checked_decoder() {
        let consumers = [
            ("boundary", include_str!("boundary.rs")),
            ("admission", include_str!("engine/admission.rs")),
            ("engine", include_str!("engine/mod.rs")),
            ("play", include_str!("engine/play.rs")),
            ("selection", include_str!("engine/selection.rs")),
        ];
        for (name, source) in consumers {
            assert!(
                source.contains(".record(&state.frames)")
                    || source.contains("pending_choice_is_valid(pending)")
                    || source.contains("pending_card_play("),
                "{name} must authenticate the frame-owned pending record"
            );
            assert!(
                !source.contains("pending.kind()"),
                "{name} must not bypass the checked kind decoder"
            );
            assert!(
                !source.contains("pending.source_pile()"),
                "{name} must not bypass the checked pile decoder"
            );
        }
        let hot = include_str!("hot.rs");
        let stale_pile_index = ["PileId::ALL[usize::from(", "self.routing"].concat();
        let stale_kind_mask = ["(self.routing >> ", "3) & 7"].concat();
        assert!(!hot.contains(&stale_pile_index));
        assert!(!hot.contains(&stale_kind_mask));
    }

    #[test]
    fn solo_fanout_clones_share_the_default_cow_until_party_state_is_written() {
        let solo = HotState::at_defaults();
        let solo_clone = solo.clone();
        assert!(Arc::ptr_eq(&solo.fanouts.0, &solo_clone.fanouts.0));
        assert!(solo.fanouts.0.multiplayer_ally.is_none());
        assert_eq!(
            solo.fanouts.multiplayer_ally(),
            &MultiplayerAllyState::default()
        );
        assert!(solo.fanouts.0.multiplayer_ally.is_none());
        assert!(Arc::ptr_eq(&solo.fanouts.0, &solo_clone.fanouts.0));

        let mut party = solo_clone;
        assert!(party.fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            ..MultiplayerAllyState::default()
        }));
        assert!(!Arc::ptr_eq(&solo.fanouts.0, &party.fanouts.0));
        assert!(party.fanouts.0.multiplayer_ally.is_some());
        let party_clone = party.clone();
        assert!(Arc::ptr_eq(&party.fanouts.0, &party_clone.fanouts.0));
    }

    #[test]
    fn teammate_pending_requires_and_survives_its_exact_ally_replacement() {
        let mut fanouts = HotFanouts::new();
        assert!(!fanouts.set_teammate_power_pending(Some((1, 77))));
        assert!(fanouts.0.multiplayer_ally.is_none());
        assert!(fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            ..MultiplayerAllyState::default()
        }));
        assert!(!fanouts.set_teammate_power_pending(Some((0, 77))));
        assert!(fanouts.set_teammate_power_pending(Some((1, 77))));
        assert_eq!(fanouts.teammate_power_pending(0), None);
        assert_eq!(fanouts.teammate_power_pending(1), Some((1, 77)));

        assert!(fanouts.set_multiplayer_ally(MultiplayerAllyState {
            key: 1,
            energy: 9,
            temp_dexterity: 8,
            ..MultiplayerAllyState::default()
        }));
        assert_eq!(fanouts.multiplayer_ally().energy, 9);
        assert_eq!(fanouts.multiplayer_ally().temp_dexterity, 8);
        assert_eq!(fanouts.teammate_power_pending(1), Some((1, 77)));
        let before = fanouts.clone();
        assert!(!fanouts.set_multiplayer_ally(MultiplayerAllyState::default()));
        assert_eq!(fanouts, before);
        assert!(fanouts.set_teammate_power_pending(None));
        assert_eq!(fanouts.teammate_power_pending(1), None);
    }

    #[test]
    fn packed_random_ai_state_preserves_order_and_trailing_windows() {
        let mut state = RandomAiState::new();
        assert!(state.is_empty());
        assert!(state.set_next(Some(2)));
        assert!(state.set_log(&[2]));
        assert!(state.push_log(0));
        assert!(state.push_log(1));
        assert!(state.push_log(2));
        assert_eq!(state.next(), Some(2));
        assert_eq!(state.log_len(), 3);
        assert_eq!(
            (0..state.log_len())
                .map(|position| state.log_at(position).unwrap())
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert!(state.push_once(1));
        assert_eq!(state.once_len(), 1);
        assert_eq!(state.once_at(0), Some(1));
        assert!(state.set_once(&[0, 1, 2, 3]));
        assert_eq!(state.once_len(), 4);
        assert_eq!(state.once_at(3), Some(3));
        assert!(state.set_rat_call_for_backup_count(2));
        state.set_rat_spawn_fresh(true);
        assert_eq!(state.rat_call_for_backup_count(), 2);
        assert!(state.rat_spawn_fresh());
        assert_eq!(state.next(), Some(2));
        assert_eq!(state.log_at(2), Some(2));
        assert_eq!(state.once_at(3), Some(3));
        assert!(!state.set_rat_call_for_backup_count(4));
        assert_eq!(state.rat_call_for_backup_count(), 2);
        assert!(!state.set_log(&[0, 1, 2, 3]));
        assert!(!state.set_once(&[0, 1, 2, 3, 4]));
        assert!(!state.set_next(Some(7)));
    }

    #[test]
    fn a_pending_selection_handle_fits_its_one_word_reservation() {
        type PendingSelectionPayloadProbe = [u8; 64];

        assert_eq!(
            size_of::<Option<Arc<PendingSelectionPayloadProbe>>>(),
            size_of::<usize>()
        );
    }

    #[test]
    fn an_rng_write_leaves_an_existing_clone_alone() {
        let mut state = HotState::at_defaults();
        state.rng.set(
            RngStream::Rng,
            RngStreamState {
                words: [1, 2, 3, 4],
                counter: 9,
            },
        );
        let snapshot = state.clone();
        state.rng.set(
            RngStream::Rng,
            RngStreamState {
                words: [5, 6, 7, 8],
                counter: 10,
            },
        );
        assert_eq!(snapshot.rng.get(RngStream::Rng).counter, 9);
        assert_eq!(state.rng.get(RngStream::Rng).counter, 10);
        assert!(snapshot.rng.is_vacant(RngStream::Ai));
    }

    #[test]
    fn the_orb_queue_preserves_order_amounts_and_copy_on_write() {
        let mut queue = HotOrbs::new();
        queue.set_base_slots(3);
        queue.set_slots(3);
        queue.set_orbs(vec![
            HotOrb::from_parts(OrbKind::Lightning, None).unwrap(),
            HotOrb::from_parts(OrbKind::Dark, Some(17)).unwrap(),
            HotOrb::from_parts(OrbKind::Glass, Some(4)).unwrap(),
        ]);
        let snapshot = queue.clone();
        queue.set_slots(2);
        queue.set_temp_focus(-3);

        assert_eq!(snapshot.base_slots(), 3);
        assert_eq!(snapshot.slots(), 3);
        assert_eq!(snapshot.temp_focus(), 0);
        assert_eq!(queue.slots(), 2);
        assert_eq!(queue.temp_focus(), -3);
        assert_eq!(snapshot.as_slice()[0].kind(), OrbKind::Lightning);
        assert_eq!(snapshot.as_slice()[0].amount(), None);
        assert_eq!(snapshot.as_slice()[1].amount(), Some(17));
        assert_eq!(snapshot.as_slice()[2].amount(), Some(4));
        assert!(HotOrbs::new().is_vacant());
        assert!(!snapshot.is_vacant());
    }

    #[test]
    fn card_event_orders_and_private_counters_are_copy_on_write() {
        let mut state = HotState::at_defaults();
        let snapshot = state.clone();

        assert!(state.fanouts.register_after_card_drawn(PowerId::Automation));
        assert!(
            state
                .fanouts
                .register_after_card_exhausted(PowerId::FeelNoPain)
        );
        assert!(
            state
                .fanouts
                .register_before_hand_draw(PowerId::InfiniteBlades)
        );
        state.fanouts.set_automation_left(4);
        state.fanouts.set_cacophony_left(9);
        state.fanouts.set_panache_left(2);
        assert!(state.fanouts.set_ethereal_plays_finished_combat(7));
        state.fanouts.set_pale_blue_dot_used(true);

        assert_eq!(
            state.fanouts.after_card_drawn_order(),
            &[PowerId::Automation]
        );
        assert_eq!(
            state.fanouts.after_card_exhausted_order(),
            &[PowerId::FeelNoPain]
        );
        assert_eq!(
            state.fanouts.before_hand_draw_order(),
            &[PowerId::InfiniteBlades]
        );
        assert_eq!(state.fanouts.automation_left(), 4);
        assert_eq!(state.fanouts.cacophony_left(), 9);
        assert_eq!(state.fanouts.panache_left(), 2);
        assert_eq!(state.fanouts.ethereal_plays_finished_combat(), 7);
        assert!(state.fanouts.pale_blue_dot_used());

        assert!(snapshot.fanouts.after_card_drawn_order().is_empty());
        assert!(snapshot.fanouts.after_card_exhausted_order().is_empty());
        assert!(snapshot.fanouts.before_hand_draw_order().is_empty());
        assert_eq!(snapshot.fanouts.automation_left(), 10);
        assert_eq!(snapshot.fanouts.cacophony_left(), 33);
        assert_eq!(snapshot.fanouts.panache_left(), 5);
        assert_eq!(snapshot.fanouts.ethereal_plays_finished_combat(), 0);
        assert!(!snapshot.fanouts.pale_blue_dot_used());
    }

    #[test]
    fn before_hand_draw_unregister_is_ordered_and_copy_on_write() {
        let mut state = HotState::at_defaults();
        assert!(
            state
                .fanouts
                .register_before_hand_draw(PowerId::InfiniteBlades)
        );
        assert!(state.fanouts.register_before_hand_draw(PowerId::Foregone));
        assert!(state.fanouts.register_before_hand_draw(PowerId::HelloWorld));
        let snapshot = state.clone();

        state.fanouts.unregister_before_hand_draw(PowerId::Foregone);
        assert_eq!(
            state.fanouts.before_hand_draw_order(),
            &[PowerId::InfiniteBlades, PowerId::HelloWorld]
        );
        assert_eq!(
            snapshot.fanouts.before_hand_draw_order(),
            &[
                PowerId::InfiniteBlades,
                PowerId::Foregone,
                PowerId::HelloWorld
            ]
        );

        state.fanouts.unregister_before_hand_draw(PowerId::Foregone);
        assert_eq!(
            state.fanouts.before_hand_draw_order(),
            &[PowerId::InfiniteBlades, PowerId::HelloWorld]
        );
    }

    #[test]
    fn packed_private_power_flags_preserve_each_other_exactly() {
        let mut fanouts = HotFanouts::new();
        assert!(fanouts.register_monologue_hooks());
        fanouts.set_ruined_helmet_used();
        fanouts.set_pale_blue_dot_used(true);
        fanouts.set_unsettling_lamp_available(true);
        fanouts.set_void_form_hooks_registered(true);
        fanouts.set_void_form_end_turn_requested(true);

        assert!(fanouts.monologue_hooks_are_registered());
        assert!(fanouts.ruined_helmet_used());
        assert!(fanouts.pale_blue_dot_used());
        assert!(fanouts.unsettling_lamp_available());
        assert!(fanouts.void_form_hooks_are_registered());
        assert!(fanouts.void_form_end_turn_requested());

        fanouts.set_pale_blue_dot_used(false);
        fanouts.set_unsettling_lamp_available(false);
        assert!(fanouts.monologue_hooks_are_registered());
        assert!(fanouts.ruined_helmet_used());
        assert!(!fanouts.pale_blue_dot_used());
        assert!(!fanouts.unsettling_lamp_available());
        assert!(fanouts.void_form_hooks_are_registered());
        assert!(fanouts.void_form_end_turn_requested());

        fanouts.forge_monologue_hook_flags_for_test(0b001);
        assert!(!fanouts.monologue_hook_flags_are_exact());
        assert!(fanouts.ruined_helmet_used());
        assert!(fanouts.void_form_hooks_are_registered());
        assert!(fanouts.void_form_end_turn_requested());
        fanouts.unregister_monologue_hooks();
        assert!(fanouts.monologue_hook_flags_are_exact());
        assert!(fanouts.ruined_helmet_used());
        assert!(fanouts.void_form_hooks_are_registered());
        assert!(fanouts.void_form_end_turn_requested());

        fanouts.set_void_form_hooks_registered(false);
        fanouts.set_void_form_end_turn_requested(false);
        assert!(!fanouts.void_form_hooks_are_registered());
        assert!(!fanouts.void_form_end_turn_requested());
        assert!(fanouts.ruined_helmet_used());
    }

    #[test]
    fn compact_countdowns_and_ethereal_history_preserve_their_exact_domains() {
        let mut fanouts = HotFanouts::new();

        for (tender, count) in [(false, 0), (true, 17)] {
            assert!(fanouts.set_tender_state(tender, count));
            for automation in 0..=15 {
                for panache in 0..=7 {
                    assert!(fanouts.try_set_automation_left(automation));
                    assert!(fanouts.try_set_panache_left(panache));
                    assert_eq!(fanouts.automation_left(), automation);
                    assert_eq!(fanouts.panache_left(), panache);
                    assert_eq!(fanouts.tender_is_active(), tender);
                    assert_eq!(fanouts.tender_cards_played(), count);
                }
            }
        }
        for value in [-1, 16, 256, i32::MAX] {
            let before = fanouts.clone();
            assert!(!fanouts.try_set_automation_left(value));
            assert_eq!(fanouts, before);
        }
        for value in [-1, 8, 256, i32::MAX] {
            let before = fanouts.clone();
            assert!(!fanouts.try_set_panache_left(value));
            assert_eq!(fanouts, before);
        }
        for (setter, invalid) in [
            (
                HotFanouts::set_automation_left as fn(&mut HotFanouts, i32),
                16,
            ),
            (HotFanouts::set_panache_left as fn(&mut HotFanouts, i32), 8),
        ] {
            let before = fanouts.clone();
            assert!(
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    setter(&mut fanouts, invalid);
                }))
                .is_err()
            );
            assert_eq!(fanouts, before);
        }
        assert!(!fanouts.set_tender_state(false, 1));
        assert!(!fanouts.set_tender_state(true, -1));
        for value in [0, 1, 33, 255] {
            assert!(fanouts.try_set_cacophony_left(value));
            assert_eq!(fanouts.cacophony_left(), value);
        }
        for value in [-1, 256, i32::MAX] {
            let before = fanouts.clone();
            assert!(!fanouts.try_set_cacophony_left(value));
            assert_eq!(fanouts, before);
        }

        assert!(!fanouts.set_ethereal_plays_finished_combat(-1));
        assert_eq!(fanouts.ethereal_plays_finished_combat(), 0);
        assert!(fanouts.set_ethereal_plays_finished_combat(i32::MAX));
        assert!(!fanouts.can_record_ethereal_plays(1));
        assert_eq!(fanouts.record_ethereal_play_finished(), Err(()));
        assert_eq!(fanouts.ethereal_plays_finished_combat(), i32::MAX);
    }

    #[test]
    fn packed_cold_lengths_preserve_their_complete_cartesian_product() {
        let before = [BeforeSideTurnEndToken::Hailstorm];
        let star = [PowerId::LightningRod, PowerId::Spinner];
        let local = [
            PowerId::Accuracy,
            PowerId::Automation,
            PowerId::Cacophony,
            PowerId::Panache,
        ];
        let result_location = [
            PowerId::Corruption,
            PowerId::Feral,
            PowerId::Nostalgia,
            PowerId::Rebound,
        ];
        for before_len in 0..=before.len() {
            for star_len in 0..=star.len() {
                for local_len in 0..=local.len() {
                    for result_len in 0..=result_location.len() {
                        let mut fanouts = HotFanouts::new();
                        assert!(fanouts.set_before_side_turn_end_order(&before[..before_len]));
                        assert!(fanouts.set_star_energy_reset_order(&star[..star_len]));
                        assert!(fanouts.set_local_generated_power_order(&local[..local_len]));
                        assert!(
                            fanouts.set_result_location_power_order(&result_location[..result_len])
                        );
                        assert_eq!(fanouts.before_side_turn_end_order(), &before[..before_len]);
                        assert_eq!(fanouts.star_energy_reset_order(), &star[..star_len]);
                        assert_eq!(fanouts.local_generated_power_order(), &local[..local_len]);
                        let actual_result_location = fanouts.result_location_power_order();
                        assert_eq!(&*actual_result_location, &result_location[..result_len]);
                        assert!(fanouts.cold_packed_metadata_is_exact());
                    }
                }
            }
        }

        let mut fanouts = HotFanouts::new();
        let snapshot = fanouts.clone();
        assert!(!fanouts.set_star_energy_reset_order(&[
            PowerId::LightningRod,
            PowerId::Spinner,
            PowerId::Strength,
        ]));
        assert!(!fanouts.set_local_generated_power_order(&[
            PowerId::Accuracy,
            PowerId::Automation,
            PowerId::Cacophony,
            PowerId::Panache,
            PowerId::Strength,
        ]));
        assert_eq!(fanouts, snapshot);

        assert!(fanouts.set_before_side_turn_end_order(&[
            BeforeSideTurnEndToken::Hailstorm,
            BeforeSideTurnEndToken::Hailstorm,
        ]));
        assert!(!fanouts.cold_packed_metadata_is_exact());

        for raw in [
            0b0000_1100,
            0b0101_0000,
            0b1000_0001,
            0b1000_0010,
            0b1000_0011,
        ] {
            let mut forged = HotFanouts::new();
            Arc::make_mut(&mut forged.0).turn_and_generated_lens = raw;
            assert!(!forged.cold_packed_metadata_is_exact(), "raw={raw:#010b}");
        }
    }

    #[test]
    fn hello_world_full_width_snapshot_is_keyed_and_clone_isolated() {
        let mut state = HotState::at_defaults();
        assert!(state.set_hello_world_amount_on_turn_start(9, 3));
        let original = state.clone();
        assert!(state.set_hello_world_amount_on_turn_start(9, 4));
        assert_ne!(state, original);
        assert_eq!(original.hello_world_amount_on_turn_start(9), Some(3));
        state.freeze_hello_world_amount_on_turn_start();
        assert_eq!(state.hello_world_amount_on_turn_start(9), Some(9));
        assert_eq!(original.hello_world_amount_on_turn_start(9), Some(3));
        assert!(state.set_hello_world_amount_on_turn_start(i32::MAX, 0));
        assert_eq!(state.hello_world_amount_on_turn_start(i32::MAX), Some(0));
        let before = state.clone();
        assert!(!state.set_hello_world_amount_on_turn_start(i32::MAX, -1));
        assert_eq!(state, before);
        let mut forged = HotState::at_defaults();
        Arc::make_mut(&mut Arc::make_mut(&mut forged.fanouts.0).cold)
            .hello_world_snapshot_extra_delta = 2;
        assert!(!forged.generated_power_flags_are_exact());
        assert_eq!(forged.hello_world_amount_on_turn_start(9), None);
    }

    #[test]
    fn generated_power_flags_preserve_exact_cartesian_product_and_reject_forgeries() {
        for pool in [false, true] {
            for dirty in [false, true] {
                for calamity in [false, true] {
                    let mut state = HotState::at_defaults();
                    if pool {
                        state.publish_hello_world_generation_pool();
                    }
                    assert!(state.set_hello_world_amount_on_turn_start(i32::from(dirty), 0));
                    state.set_calamity_hook_live(calamity);
                    assert_eq!(state.hello_world_generation_pool_is_exact(), pool);
                    assert_eq!(state.hello_world_snapshot_is_current(), !dirty);
                    assert_eq!(
                        state.hello_world_amount_on_turn_start(i32::from(dirty)),
                        Some(0)
                    );
                    assert_eq!(state.calamity_hook_is_live(), calamity);
                    assert!(state.generated_power_flags_are_exact());
                }
            }
        }

        let mut snapshot = HotState::at_defaults();
        assert!(snapshot.set_hello_world_amount_on_turn_start(4, 4));
        assert_eq!(snapshot.hello_world_amount_on_turn_start(4), Some(4));
        assert!(snapshot.hello_world_snapshot_is_current());
        assert!(snapshot.set_hello_world_amount_on_turn_start(5, 4));
        assert_eq!(snapshot.hello_world_amount_on_turn_start(5), Some(4));
        assert!(!snapshot.hello_world_snapshot_is_current());
        let before = snapshot.clone();
        for (current, frozen) in [(4, 5), (-1, -1), (0, -1)] {
            assert!(!snapshot.set_hello_world_amount_on_turn_start(current, frozen));
            assert_eq!(snapshot, before);
        }
        snapshot.freeze_hello_world_amount_on_turn_start();
        assert!(snapshot.hello_world_snapshot_is_current());
        assert_eq!(snapshot.hello_world_amount_on_turn_start(5), Some(5));

        let peers = [
            PowerId::Loop,
            PowerId::RollingBoulder,
            PowerId::SummonNextTurn,
            PowerId::Inferno,
            PowerId::CrimsonMantle,
            PowerId::ToolsOfTheTrade,
            PowerId::Tyranny,
        ];
        for ordinal in 0..=peers.len() {
            let mut state = HotState::at_defaults();
            state
                .powers
                .set(PowerId::Entropy, crate::powers::SlotWire::Int, 1);
            let mut order = peers.to_vec();
            order.insert(ordinal, PowerId::Entropy);
            assert!(state.set_after_player_turn_start_order(&order));
            assert_eq!(&*state.after_player_turn_start_order().unwrap(), order);
            assert!(state.generated_power_flags_are_exact());

            assert!(state.unregister_after_player_turn_start(PowerId::SummonNextTurn));
            let without_summon = order
                .iter()
                .copied()
                .filter(|power| *power != PowerId::SummonNextTurn)
                .collect::<Vec<_>>();
            assert_eq!(
                &*state.after_player_turn_start_order().unwrap(),
                without_summon,
                "removing a preceding Summon decrements Entropy's live ordinal"
            );
            assert!(state.register_after_player_turn_start(PowerId::SummonNextTurn));
            let mut restacked = without_summon;
            restacked.push(PowerId::SummonNextTurn);
            assert_eq!(&*state.after_player_turn_start_order().unwrap(), restacked);
        }

        for raw in 0b0001_0000..=u8::MAX {
            let mut forged = HotState::at_defaults();
            forged.generated_power_flags = raw;
            assert!(!forged.generated_power_flags_are_exact(), "raw={raw:#010b}");
        }

        let mut forged = HotState::at_defaults();
        forged
            .powers
            .set(PowerId::Entropy, crate::powers::SlotWire::Int, 1);
        forged.generated_power_flags = 1 << ENTROPY_TURN_START_ORDINAL_SHIFT;
        assert!(!forged.generated_power_flags_are_exact());
        forged.generated_power_flags = 0b1000_0000;
        assert!(!forged.generated_power_flags_are_exact());
    }

    #[test]
    fn packed_two_player_orders_preserve_every_interaction_and_reject_forgeries() {
        fn expected(keys: &[u32]) -> [Option<u8>; 2] {
            let mut out = [None; 2];
            for (slot, key) in out.iter_mut().zip(keys) {
                *slot = Some(*key as u8);
            }
            out
        }

        let orders: [&[u32]; 5] = [&[], &[0], &[1], &[0, 1], &[1, 0]];
        for imitation in orders {
            for intercept in orders {
                let mut fanouts = HotFanouts::new();
                for key in imitation {
                    assert!(fanouts.set_imitation_learning(*key, 1));
                }
                for key in intercept {
                    assert!(fanouts.cover_with_intercept(*key));
                }
                assert_eq!(fanouts.imitation_learning_order(), expected(imitation));
                assert_eq!(fanouts.intercept_covered_order(), expected(intercept));
                assert!(fanouts.cold_packed_metadata_is_exact());

                let imitation_order = fanouts.imitation_learning_order();
                assert!(fanouts.set_intercept_covered_mask(0));
                assert_eq!(fanouts.imitation_learning_order(), imitation_order);
                for key in intercept {
                    assert!(fanouts.cover_with_intercept(*key));
                }

                let intercept_order = fanouts.intercept_covered_order();
                for key in imitation {
                    assert!(fanouts.set_imitation_learning(*key, 0));
                }
                assert_eq!(fanouts.intercept_covered_order(), intercept_order);
                assert!(fanouts.cold_packed_metadata_is_exact());
            }
        }

        let mut fanouts = HotFanouts::new();
        let snapshot = fanouts.clone();
        assert!(!fanouts.set_intercept_covered_mask(0b100));
        assert_eq!(fanouts, snapshot);
        for raw in [0b0001_0100, 0b1010_0000, 0b0000_0100, 0b0010_0001] {
            let mut forged = HotFanouts::new();
            Arc::make_mut(&mut forged.0).intercept_covered_state = raw;
            assert!(!forged.cold_packed_metadata_is_exact(), "raw={raw:#010b}");
        }
    }

    #[cfg(feature = "allocation-counting")]
    #[test]
    fn first_ethereal_completion_pays_one_fanout_cow_and_later_writes_pay_none() {
        use crate::allocation::thread_snapshot;

        let root = HotState::at_defaults();
        let mut successor = root.clone();
        let (before_allocations, before_bytes) = thread_snapshot();
        successor.fanouts.record_ethereal_play_finished().unwrap();
        let (after_first_allocations, after_first_bytes) = thread_snapshot();
        successor.fanouts.record_ethereal_play_finished().unwrap();
        let (after_second_allocations, after_second_bytes) = thread_snapshot();

        assert_eq!(after_first_allocations - before_allocations, 1);
        assert_eq!(
            after_first_bytes - before_bytes,
            size_of::<FanoutState>() as u64 + 2 * size_of::<usize>() as u64,
            "one ArcInner allocation carries the exact measured fanout payload"
        );
        assert_eq!(after_second_allocations, after_first_allocations);
        assert_eq!(after_second_bytes, after_first_bytes);
        assert_eq!(successor.fanouts.ethereal_plays_finished_combat(), 2);
        assert_eq!(root.fanouts.ethereal_plays_finished_combat(), 0);
    }

    #[cfg(feature = "allocation-counting")]
    #[test]
    fn first_monologue_write_pays_one_fanout_cow_and_later_writes_pay_none() {
        use crate::allocation::thread_snapshot;

        let root = HotState::at_defaults();
        let mut successor = root.clone();
        let (before_allocations, before_bytes) = thread_snapshot();
        assert!(successor.fanouts.register_monologue_hooks());
        let (after_first_allocations, after_first_bytes) = thread_snapshot();
        assert!(successor.fanouts.set_monologue_strength_applied(1));
        successor.fanouts.set_ruined_helmet_used();
        let (after_later_allocations, after_later_bytes) = thread_snapshot();

        assert_eq!(after_first_allocations - before_allocations, 1);
        assert_eq!(
            after_first_bytes - before_bytes,
            size_of::<FanoutState>() as u64 + 2 * size_of::<usize>() as u64
        );
        assert_eq!(after_later_allocations, after_first_allocations);
        assert_eq!(after_later_bytes, after_first_bytes);
        assert!(!root.fanouts.monologue_hooks_are_registered());
        assert_eq!(root.fanouts.monologue_strength_applied(), 0);
        assert!(!root.fanouts.ruined_helmet_used());
    }

    #[cfg(feature = "allocation-counting")]
    #[test]
    fn first_tender_write_pays_one_unchanged_fanout_cow_and_later_writes_pay_none() {
        use crate::allocation::thread_snapshot;

        let root = HotState::at_defaults();
        let mut successor = root.clone();
        let (before_allocations, before_bytes) = thread_snapshot();
        assert!(successor.fanouts.set_tender_state(true, 0));
        let (after_first_allocations, after_first_bytes) = thread_snapshot();
        assert!(successor.fanouts.set_tender_state(true, 1));
        successor.fanouts.set_panache_left(3);
        successor.fanouts.set_automation_left(4);
        let (after_later_allocations, after_later_bytes) = thread_snapshot();

        assert_eq!(after_first_allocations - before_allocations, 1);
        assert_eq!(
            after_first_bytes - before_bytes,
            size_of::<FanoutState>() as u64 + 2 * size_of::<usize>() as u64
        );
        assert_eq!(after_later_allocations, after_first_allocations);
        assert_eq!(after_later_bytes, after_first_bytes);
        assert!(!root.fanouts.tender_is_active());
        assert_eq!(root.fanouts.tender_cards_played(), 0);
        assert!(successor.fanouts.tender_is_active());
        assert_eq!(successor.fanouts.tender_cards_played(), 1);
        assert_eq!(successor.fanouts.panache_left(), 3);
        assert_eq!(successor.fanouts.automation_left(), 4);
    }

    #[test]
    fn the_misery_order_appends_and_removes_by_token() {
        let mut order = MiseryOrder::new();
        assert!(order.is_empty());
        order.push(MiseryToken::Vuln);
        order.push_knockdown(2);
        order.push(MiseryToken::Weak);
        order.push(MiseryToken::Vuln);
        let snapshot = order.clone();
        order.remove_all(MiseryToken::Vuln);
        assert_eq!(
            order.as_slice(),
            [MiseryToken::Knockdown, MiseryToken::Weak]
        );
        assert_eq!(order.knockdown(), [2]);
        assert_eq!(snapshot.as_slice().len(), 4);
        assert_eq!(snapshot.knockdown(), [2]);
        order.push_knockdown(3);
        assert_eq!(order.knockdown(), [2, 3]);
        assert_eq!(snapshot.knockdown(), [2], "the shared Arc is copy-on-write");
        order.remove_all(MiseryToken::Knockdown);
        assert!(order.knockdown().is_empty());
        assert_eq!(order.as_slice(), [MiseryToken::Weak]);
        // Removing an absent token leaves the shared storage untouched.
        let mut untouched = snapshot.clone();
        untouched.remove_all(MiseryToken::Doom);
        assert_eq!(untouched, snapshot);
    }

    fn attachment(power: AttachedPowerModel, applier: Applier, amount: i32) -> AttachmentRecord {
        AttachmentRecord {
            power,
            applier,
            amount,
        }
    }

    /// Attachment position is arrival order and nothing else.
    ///
    /// `Creature::ApplyPowerInternal` `0x11da0c` IL_0063-IL_006f appends, and
    /// `Creature::get_Powers` `0x11d8ae` returns `_powers` unsorted, so
    /// neither the power's identity nor its amount may influence where an
    /// instance sits. The ledger is what `Misery`'s `ToDictionary` over
    /// `Target.Powers` freezes as replay order, so a sort here would be a
    /// wrong answer, not a cosmetic one.
    #[test]
    fn an_attachment_ledger_is_never_reordered_by_amount_or_power() {
        let mut order = MiseryOrder::new();
        // Deliberately descending by model name ("strength" > "imbalanced")
        // and non-ascending by amount, so a sort on either key would move a
        // position: by model the Imbalanced row would come first, by amount
        // the -4 row would.
        order.push_attachment(attachment(AttachedPowerModel::Strength, Applier::Player, 3));
        order.push_attachment(attachment(AttachedPowerModel::Strength, Applier::None, -4));
        order.push_attachment(attachment(
            AttachedPowerModel::Imbalanced,
            Applier::Monster(7),
            9,
        ));
        let expected = [
            attachment(AttachedPowerModel::Strength, Applier::Player, 3),
            attachment(AttachedPowerModel::Strength, Applier::None, -4),
            attachment(AttachedPowerModel::Imbalanced, Applier::Monster(7), 9),
        ];
        assert_eq!(order.attachments(), expected);
        assert_eq!(order.attachments().len(), 3);

        // A restacking application changes a value, never a position
        // (`ModifyAmount` `0x3f032c` IL_01d6 -> `SetAmount` `0x83f8c`, which
        // never touches `Creature::_powers`).
        let snapshot = order.clone();
        assert!(order.set_attachment_amount(1, 12));
        assert_eq!(
            order
                .attachments()
                .map(|record| (record.power, record.applier))
                .collect::<Vec<_>>(),
            vec![
                (AttachedPowerModel::Strength, Applier::Player),
                (AttachedPowerModel::Strength, Applier::None),
                (AttachedPowerModel::Imbalanced, Applier::Monster(7)),
            ],
        );
        assert_eq!(order.attachments().get(1).unwrap().amount, 12);
        assert_eq!(
            snapshot.attachments().get(1).unwrap().amount,
            -4,
            "the shared Arc is copy-on-write",
        );

        // Zero is not a native live amount, so the mutator refuses it rather
        // than parking an instance the native lifecycle would have detached.
        let before = order.clone();
        assert!(!order.set_attachment_amount(1, 0));
        assert!(!order.set_attachment_amount(3, 5));
        assert_eq!(order, before);

        // Attachments and acquisition tokens are independent records: a
        // scalar token's removal says nothing about a physical instance.
        order.push(MiseryToken::Weak);
        order.remove_all(MiseryToken::Weak);
        assert!(order.as_slice().is_empty());
        assert_eq!(order.attachments().len(), 3);
    }

    /// Removal closes a position in place; a re-attach appends at the end.
    ///
    /// `PowerModel::ShouldRemoveDueToAmount` `0x83b0d` decides *when*
    /// (exactly zero for an `AllowNegative` power, any non-positive amount
    /// otherwise), `PowerModel::RemoveInternal` `0x84048` IL_001f calls
    /// `Creature::RemovePowerInternal` `0x11db0b`, and its IL_0015-IL_001c is
    /// a plain `List.Remove`. So the survivors keep their relative order and
    /// a power that comes back does not reclaim its old seat.
    #[test]
    fn removal_at_exactly_zero_removes_in_place_and_reattach_appends() {
        let mut order = MiseryOrder::new();
        order.push_attachment(attachment(AttachedPowerModel::Strength, Applier::Player, 2));
        order.push_attachment(attachment(
            AttachedPowerModel::Imbalanced,
            Applier::Player,
            1,
        ));
        order.push_attachment(attachment(
            AttachedPowerModel::Strength,
            Applier::Monster(7),
            1,
        ));

        // The lookup is keyed on model AND applier jointly: two Strength rows
        // coexist and each answers only for its own applier.
        assert_eq!(
            order.find_attachment(AttachedPowerModel::Strength, Applier::Player),
            Some(0)
        );
        assert_eq!(
            order.find_attachment(AttachedPowerModel::Strength, Applier::Monster(7)),
            Some(2)
        );

        let found = order
            .find_attachment(AttachedPowerModel::Imbalanced, Applier::Player)
            .expect("the stacking lookup finds the applier-matched instance");
        assert_eq!(found, 1);
        assert!(order.remove_attachment(found));
        assert_eq!(
            order.attachments(),
            [
                attachment(AttachedPowerModel::Strength, Applier::Player, 2),
                attachment(AttachedPowerModel::Strength, Applier::Monster(7), 1),
            ],
            "the survivors keep their relative order",
        );
        assert_eq!(
            order.find_attachment(AttachedPowerModel::Imbalanced, Applier::Player),
            None
        );

        order.push_attachment(attachment(
            AttachedPowerModel::Imbalanced,
            Applier::Player,
            3,
        ));
        assert_eq!(
            order.attachments(),
            [
                attachment(AttachedPowerModel::Strength, Applier::Player, 2),
                attachment(AttachedPowerModel::Strength, Applier::Monster(7), 1),
                attachment(AttachedPowerModel::Imbalanced, Applier::Player, 3),
            ],
            "a re-attach appends rather than reclaiming the old position",
        );
        assert!(!order.remove_attachment(3));
    }

    /// The applier is a creature *reference* in native
    /// (`<FindExistingInstanceForStacking>b__0` `0x3ef7ca` IL_000d is `ceq`),
    /// so absence, the player, and each individual monster are three
    /// different lookup keys — and a replacement carrying a different uid is
    /// a fourth.
    #[test]
    fn an_absent_applier_is_distinguishable_from_a_replacement_in_the_same_slot() {
        let appliers = [
            Applier::None,
            Applier::Player,
            Applier::Monster(4),
            Applier::Monster(9),
        ];
        let mut ledgers = Vec::new();
        for applier in appliers {
            let mut order = MiseryOrder::new();
            order.push_attachment(attachment(AttachedPowerModel::Strength, applier, 3));
            assert_eq!(
                order.find_attachment(AttachedPowerModel::Strength, applier),
                Some(0)
            );
            for other in appliers {
                if other != applier {
                    assert_eq!(
                        order.find_attachment(AttachedPowerModel::Strength, other),
                        None,
                        "{applier:?} must not answer a lookup for {other:?}",
                    );
                }
            }
            ledgers.push(order);
        }
        for (index, left) in ledgers.iter().enumerate() {
            for right in &ledgers[index + 1..] {
                assert_ne!(left, right);
            }
        }
    }

    #[test]
    fn the_history_defaults_start_the_round_counter_at_one() {
        let history = HotHistory::at_defaults();
        assert_eq!(history.round_number, 1);
        assert_eq!(history.card_plays_finished_combat, 0);
        assert!(!history.over);
        assert_eq!(HotState::at_defaults().history, history);
    }

    #[test]
    fn pile_and_stream_tables_are_ascending_and_index_by_discriminant() {
        assert!(PileId::NAMES.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(RngStream::NAMES.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(OrbKind::NAMES.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(MiseryToken::NAMES.windows(2).all(|pair| pair[0] < pair[1]));
        for (index, kind) in OrbKind::ALL.iter().enumerate() {
            assert_eq!(*kind as usize, index);
            assert_eq!(OrbKind::from_str(kind.as_str()), Some(*kind));
        }
        for token in MiseryToken::ALL {
            assert_eq!(MiseryToken::from_str(token.as_str()), Some(token));
        }
        assert_eq!(MiseryToken::from_str("thorns"), None);
        for (index, pile) in PileId::ALL.iter().enumerate() {
            assert_eq!(*pile as usize, index);
            assert_eq!(PileId::from_str(pile.as_str()), Some(*pile));
        }
        for (index, stream) in RngStream::ALL.iter().enumerate() {
            assert_eq!(*stream as usize, index);
            assert_eq!(RngStream::from_str(stream.as_str()), Some(*stream));
        }
        assert_eq!(PileId::from_str("deck"), None);
        assert_eq!(RngStream::from_str("shuffle"), None);
        assert_eq!(OrbKind::from_str("VOID"), None);
    }

    #[test]
    fn a_pile_write_leaves_an_existing_clone_alone() {
        let mut state = HotState::at_defaults();
        state.piles.set(
            PileId::Hand,
            HotPile::from_cards(vec![HotCard {
                uid: 0,
                atom: 0,
                flags: 0,
            }]),
        );
        let snapshot = state.clone();
        state.piles.get_mut(PileId::Hand).make_mut().push(HotCard {
            uid: 1,
            atom: 0,
            flags: 0,
        });
        assert_eq!(snapshot.piles.get(PileId::Hand).len(), 1);
        assert_eq!(state.piles.get(PileId::Hand).len(), 2);
        // The untouched piles are still shared with the snapshot.
        assert_eq!(
            snapshot.piles.get(PileId::Draw),
            state.piles.get(PileId::Draw)
        );
    }

    #[test]
    fn a_roster_write_leaves_an_existing_clone_alone() {
        let mut state = HotState::at_defaults();
        let mut toadpole = HotMonster::new(MonsterKind::Toadpole, 26);
        toadpole.max_hp = 26;
        state.monsters_mut().push(toadpole);
        let snapshot = state.clone();
        state.monsters_mut()[0].hp = 10;
        assert_eq!(snapshot.monsters[0].hp, 26);
        assert_eq!(state.monsters[0].hp, 10);
    }

    #[test]
    fn the_card_side_table_stays_sorted_and_vacates() {
        let mut states = CardStates::new();
        assert!(states.is_empty());
        let filled = CardInstanceState {
            enchantment_state: OptionalNonNegativeI32::from_option(Some(2)).unwrap(),
            ..CardInstanceState::default()
        };
        states.set(9, filled.clone());
        states.set(3, filled.clone());
        assert_eq!(
            states.as_slice().iter().map(|e| e.0).collect::<Vec<_>>(),
            vec![3, 9]
        );
        assert_eq!(states.get(9), filled);
        assert_eq!(states.get(4), CardInstanceState::default());
        states.set(3, CardInstanceState::default());
        assert_eq!(states.len(), 1);
    }

    #[test]
    fn generic_base_replay_rows_are_hashed_and_copy_on_write() {
        use std::hash::{Hash, Hasher};

        fn hash(states: &CardStates) -> u64 {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            states.as_slice().hash(&mut hasher);
            hasher.finish()
        }

        let mut root = CardStates::new();
        let mut replay = CardInstanceState::default();
        replay.set_base_replay_count(Some(2)).unwrap();
        root.set(41, replay);
        let snapshot = root.clone();
        let snapshot_hash = hash(&snapshot);

        let mut changed = root.get(41);
        changed.set_base_replay_count(Some(3)).unwrap();
        root.set(41, changed);

        assert_eq!(snapshot.get(41).base_replay_count(), Some(2));
        assert_eq!(root.get(41).base_replay_count(), Some(3));
        assert_eq!(hash(&snapshot), snapshot_hash);
        assert_ne!(hash(&root), snapshot_hash);
    }

    #[test]
    fn local_keywords_share_the_side_table_and_only_transient_keywords_expire() {
        let mut states = CardStates::new();
        let mut all_local = states.get(9);
        all_local.set_local_ethereal(true);
        all_local.local_retain = true;
        all_local.local_sly = true;
        states.set(9, all_local);
        states.set_transient_retain(9);
        states.set_transient_sly(9);
        states.set_transient_retain(3);
        let snapshot = states.clone();

        assert!(states.get(9).local_ethereal());
        assert!(states.get(9).local_retain);
        assert!(states.get(9).local_sly);
        assert_eq!(states.get(9).local_keyword_rank(), 3);
        assert!(states.get(9).transient_retain);
        assert!(states.get(3).transient_retain);
        states.cleanup_transient_keywords();

        assert_eq!(
            states
                .as_slice()
                .iter()
                .map(|entry| entry.0)
                .collect::<Vec<_>>(),
            [9]
        );
        assert!(states.get(9).local_ethereal());
        assert!(states.get(9).local_retain);
        assert!(states.get(9).local_sly);
        assert!(!states.get(9).transient_retain);
        assert!(!states.get(9).transient_sly());
        assert!(snapshot.get(9).transient_sly());
        assert!(states.get(3).is_vacant());
        assert!(snapshot.get(3).transient_retain, "cleanup is copy-on-write");
    }

    #[test]
    fn the_card_side_table_rejects_duplicates_and_vacant_entries() {
        let filled = CardInstanceState {
            enchantment_state: OptionalNonNegativeI32::from_option(Some(1)).unwrap(),
            ..CardInstanceState::default()
        };
        assert!(
            CardStates::from_unsorted(vec![(1, filled.clone()), (1, filled.clone())]).is_none()
        );
        assert!(CardStates::from_unsorted(vec![(1, CardInstanceState::default())]).is_none());
        assert!(CardStates::from_unsorted(vec![(2, filled.clone()), (1, filled)]).is_some());
    }

    #[test]
    fn local_cost_rows_apply_in_order_and_expire_by_bitmask() {
        let mut states = CardStates::new();
        states.append_local_cost_modifier(
            7,
            LocalCostModifier {
                kind: LocalCostModifierKind::Set,
                amount: 3,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            },
        );
        states.append_local_cost_modifier(
            7,
            LocalCostModifier {
                kind: LocalCostModifierKind::Add,
                amount: -1,
                expiration: LocalCostExpiration::ThisTurnOrPlayed,
                reduce_only: false,
            },
        );
        assert_eq!(
            states.get_ref(7).unwrap().local_cost_modifiers.resolve(9),
            2
        );
        assert!(states.cleanup_card_local_cost_modifiers(7, LocalCostExpiration::UntilPlayed));
        assert_eq!(
            states.get_ref(7).unwrap().local_cost_modifiers.resolve(9),
            3
        );
    }

    #[test]
    fn interleaved_star_rows_survive_frozen_payloads_without_aliasing_order() {
        let mut states = CardStates::new();
        states.set_to_free_this_turn(9, 2).unwrap();
        states.set_to_free_this_combat(9, 2).unwrap();
        states.set_to_free_this_turn(9, 2).unwrap();
        states.set_to_free_this_combat(9, 2).unwrap();
        let frozen = FrozenAutoBatchEntry {
            card: HotCard {
                uid: 9,
                atom: 0,
                flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            },
            state: states.get(9),
        };
        let mut words = Vec::new();
        let count = Frames::encode_frozen_auto_batch_entry(&frozen, Some(&mut words)).unwrap();
        assert_eq!(
            Frames::decode_frozen_auto_batch_entry(&words),
            Some((frozen.clone(), count))
        );
        let mut reordered = frozen.clone();
        reordered
            .state
            .local_cost_modifiers
            .set_star_cost_expirations(&[
                LocalCostExpiration::ThisCombat,
                LocalCostExpiration::ThisCombat,
                LocalCostExpiration::ThisTurnOrPlayed,
                LocalCostExpiration::ThisTurnOrPlayed,
            ])
            .unwrap();
        assert_ne!(
            reordered, frozen,
            "native append order participates in physical identity"
        );
        let mut other_words = Vec::new();
        Frames::encode_frozen_auto_batch_entry(&reordered, Some(&mut other_words)).unwrap();
        assert_ne!(words, other_words);
        // A forged exceptional row cannot claim a different transient count,
        // a nonzero Star amount, or an unknown expiry.
        let mut forged = frozen.clone();
        forged.state.free_star_cost_this_turn_or_played_rows = 1;
        assert!(Frames::encode_frozen_auto_batch_entry(&forged, None).is_none());
        let last = words.len() - 1;
        for (offset, replacement) in [
            (last - 1, PendingWord { body: 1, meta: 0 }),
            (
                last,
                PendingWord {
                    body: 3 | (2 << 8),
                    meta: 0,
                },
            ),
        ] {
            let mut forged = words.clone();
            forged[offset] = replacement;
            assert!(Frames::decode_frozen_auto_batch_entry(&forged).is_none());
        }
    }

    #[test]
    fn combat_free_cost_rows_preserve_append_order_and_survive_transient_cleanup() {
        let mut negative = CardStates::new();
        negative.set_to_free_this_combat(7, -1).unwrap();
        let negative_state = negative.get_ref(7).unwrap();
        assert!(
            negative_state
                .local_cost_modifiers
                .free_star_cost_this_combat()
        );
        assert!(negative_state.local_cost_modifiers.as_slice().is_empty());
        assert!(!negative_state.is_vacant(), "the Star-only carrier is live");

        negative.set_to_free_this_combat(7, -1).unwrap();
        assert_eq!(
            negative
                .get(7)
                .local_cost_modifiers
                .star_cost_expirations(0)
                .collect::<Vec<_>>(),
            [
                LocalCostExpiration::ThisCombat,
                LocalCostExpiration::ThisCombat
            ]
        );

        let mut interleaved = CardStates::new();
        interleaved.set_to_free_this_turn(9, -1).unwrap();
        interleaved.set_to_free_this_combat(9, -1).unwrap();
        let interleaved_state = interleaved.get(9);
        assert_eq!(
            interleaved_state
                .local_cost_modifiers
                .star_cost_expirations(1)
                .collect::<Vec<_>>(),
            [
                LocalCostExpiration::ThisTurnOrPlayed,
                LocalCostExpiration::ThisCombat
            ]
        );
        interleaved.cleanup_card_local_cost_modifiers(9, LocalCostExpiration::UntilPlayed);
        assert_eq!(
            interleaved
                .get(9)
                .local_cost_modifiers
                .star_cost_expirations(0)
                .collect::<Vec<_>>(),
            [LocalCostExpiration::ThisCombat]
        );

        let mut positive = CardStates::new();
        positive.set_to_free_this_combat(11, 2).unwrap();
        positive.set_to_free_this_turn(11, 2).unwrap();
        let before_cleanup = positive.get_ref(11).unwrap();
        assert!(
            before_cleanup
                .local_cost_modifiers
                .free_star_cost_this_combat()
        );
        assert_eq!(before_cleanup.free_star_cost_this_turn_or_played_rows, 1);
        assert_eq!(before_cleanup.local_cost_modifiers.as_slice().len(), 2);

        assert!(positive.cleanup_card_local_cost_modifiers(11, LocalCostExpiration::UntilPlayed));
        let after_cleanup = positive.get_ref(11).unwrap();
        assert!(
            after_cleanup
                .local_cost_modifiers
                .free_star_cost_this_combat()
        );
        assert_eq!(after_cleanup.free_star_cost_this_turn_or_played_rows, 0);
        assert_eq!(
            after_cleanup.local_cost_modifiers.as_slice(),
            &[LocalCostModifier {
                kind: LocalCostModifierKind::Set,
                amount: 0,
                expiration: LocalCostExpiration::ThisCombat,
                reduce_only: false,
            }]
        );
    }

    #[test]
    fn damage_growth_uses_the_sorted_cow_side_table_and_never_wraps() {
        let mut states = CardStates::new();
        assert_eq!(states.add_damage_growth(9, 4), Some(4));
        assert_eq!(states.add_damage_growth(3, 7), Some(7));
        assert_eq!(states.add_damage_growth(9, 5), Some(9));
        assert_eq!(
            states
                .as_slice()
                .iter()
                .map(|entry| entry.0)
                .collect::<Vec<_>>(),
            vec![3, 9]
        );
        assert_eq!(states.get(9).damage_growth, 9);

        let snapshot = states.clone();
        assert_eq!(states.add_damage_growth(9, i32::MAX), None);
        assert_eq!(states, snapshot, "overflow must not partially publish");
        assert_eq!(states.add_damage_growth(12, -1), None);
        assert_eq!(states, snapshot, "negative growth is not representable");
        assert_eq!(states.add_damage_growth(12, 0), Some(0));
        assert_eq!(states, snapshot, "a zero default does not create a row");
    }

    #[test]
    fn thrash_decimal_reuses_the_aux_arc_without_changing_native_cost_rows() {
        assert_eq!(size_of::<LocalCostModifiers>(), 8);
        assert_eq!(size_of::<CardInstanceState>(), 32);
        let default = CardInstanceState::default();
        assert!(default.is_vacant());
        assert_eq!(default, CardInstanceState::default());

        let exact = crate::decimal::DotNetDecimal::from_bits(crate::decimal::DotNetDecimalBits {
            lo: 5_775,
            mid: 0,
            hi: 0,
            negative: false,
            scale: 3,
        })
        .unwrap();
        let mut instance = CardInstanceState::default();
        instance.set_fraction_damage_growth(exact).unwrap();
        assert_eq!(instance.exact_damage_growth(), Some(exact));
        assert_eq!(instance.exact_damage_growth().unwrap().bits(), exact.bits());
        assert_eq!(instance.damage_growth, 0);
        assert!(instance.local_cost_modifiers.as_slice().is_empty());
        assert!(!instance.is_vacant());

        instance.local_cost_modifiers.push(LocalCostModifier {
            kind: LocalCostModifierKind::Set,
            amount: 4,
            expiration: LocalCostExpiration::ThisCombat,
            reduce_only: false,
        });
        instance.local_cost_modifiers.push(LocalCostModifier {
            kind: LocalCostModifierKind::Add,
            amount: -1,
            expiration: LocalCostExpiration::ThisTurnOrPlayed,
            reduce_only: false,
        });
        assert_eq!(instance.local_cost_modifiers.resolve(9), 3);
        instance.local_cost_modifiers.clamp_sets_above(2);
        assert_eq!(instance.local_cost_modifiers.resolve(9), 1);
        instance
            .local_cost_modifiers
            .cleanup(LocalCostExpiration::UntilPlayed);
        assert_eq!(instance.local_cost_modifiers.resolve(9), 2);
        assert_eq!(instance.exact_damage_growth(), Some(exact));

        instance
            .set_exact_damage_growth(crate::decimal::DotNetDecimal::from_i64(7))
            .unwrap();
        assert_eq!(instance.damage_growth, 7);
        assert_eq!(
            instance.exact_damage_growth(),
            Some(crate::decimal::DotNetDecimal::from_i64(7))
        );
        assert!(instance.local_cost_modifiers.0.thrash_growth.is_none());
    }

    #[test]
    fn integer_growth_cannot_alias_an_aux_decimal() {
        let exact = crate::decimal::DotNetDecimal::ratio(7, 3).unwrap();
        let mut instance = CardInstanceState::default();
        instance.set_fraction_damage_growth(exact).unwrap();
        let mut states = CardStates::new();
        states.set(9, instance);
        let before = states.clone();
        assert_eq!(states.add_damage_growth(9, 1), None);
        assert_eq!(states, before);
        assert_eq!(states.get(9).exact_damage_growth(), Some(exact));
    }

    #[cfg(feature = "allocation-counting")]
    #[test]
    fn refused_integer_growth_keeps_the_shared_side_table_without_allocating() {
        use crate::allocation::thread_snapshot;

        let exact = crate::decimal::DotNetDecimal::ratio(21, 5).unwrap();
        let mut instance = CardInstanceState::default();
        instance.set_fraction_damage_growth(exact).unwrap();
        let mut root = CardStates::default();
        root.set(9, instance);
        let mut successor = root.clone();
        assert!(Arc::ptr_eq(&root.0, &successor.0));

        let before = thread_snapshot();
        assert_eq!(successor.add_damage_growth(9, 1), None);
        let after = thread_snapshot();

        assert_eq!(after, before);
        assert!(Arc::ptr_eq(&root.0, &successor.0));
        assert_eq!(successor.get(9).exact_damage_growth(), Some(exact));
    }

    #[cfg(feature = "allocation-counting")]
    #[test]
    fn default_integer_growth_keeps_the_vacant_aux_arc_without_allocating() {
        use crate::allocation::thread_snapshot;

        // Constructing these first warms the shared LazyLock payload. Both
        // Int32 writes are no-op clears of its absent Decimal slot.
        let baseline = CardInstanceState::default();
        let mut zero = CardInstanceState::default();
        let mut seven = CardInstanceState::default();
        assert!(Arc::ptr_eq(
            &baseline.local_cost_modifiers.0,
            &zero.local_cost_modifiers.0
        ));

        let before = thread_snapshot();
        zero.set_exact_damage_growth(crate::decimal::DotNetDecimal::zero())
            .unwrap();
        seven
            .set_exact_damage_growth(crate::decimal::DotNetDecimal::from_i64(7))
            .unwrap();
        let after = thread_snapshot();

        assert_eq!(after, before);
        assert!(Arc::ptr_eq(
            &baseline.local_cost_modifiers.0,
            &zero.local_cost_modifiers.0
        ));
        assert!(Arc::ptr_eq(
            &baseline.local_cost_modifiers.0,
            &seven.local_cost_modifiers.0
        ));
        assert_eq!(zero.damage_growth, 0);
        assert_eq!(seven.damage_growth, 7);
    }

    #[test]
    fn temporary_star_rows_all_expire_with_their_energy_rows() {
        let mut states = CardStates::new();
        let instance = CardInstanceState {
            local_cost_modifiers: LocalCostModifiers::from_rows(vec![
                LocalCostModifier {
                    kind: LocalCostModifierKind::Set,
                    amount: 0,
                    expiration: LocalCostExpiration::ThisTurnOrPlayed,
                    reduce_only: false,
                },
                LocalCostModifier {
                    kind: LocalCostModifierKind::Set,
                    amount: 0,
                    expiration: LocalCostExpiration::ThisTurnOrPlayed,
                    reduce_only: false,
                },
            ]),
            free_star_cost_this_turn_or_played_rows: 2,
            ..CardInstanceState::default()
        };
        states.set(7, instance);

        assert!(states.cleanup_card_local_cost_modifiers(7, LocalCostExpiration::UntilPlayed));
        assert!(states.get(7).is_vacant());
    }

    #[test]
    fn the_frame_stack_is_copy_on_write() {
        let mut state = HotState::at_defaults();
        assert!(state.frames.is_empty());
        state.frames.push(Frame::JossSideEnd);
        let snapshot = state.clone();
        state.frames.push(Frame::TurnStart {
            stage: TurnStartStage::AfterRoyal,
            crimson_block: 2,
        });
        assert_eq!(snapshot.frames.len(), 1);
        assert_eq!(state.frames.len(), 2);
        assert_eq!(snapshot.frames.top(), Some(Frame::JossSideEnd));
        state.frames.pop();
        assert_eq!(state.frames.top(), Some(Frame::JossSideEnd));
    }

    #[test]
    fn test_subject_private_state_is_owner_packed_without_losing_independence() {
        let mut monster = HotMonster::new(MonsterKind::TestSubject, 111);
        assert_eq!(monster.test_subject_respawns(), 0);
        assert!(!monster.test_subject_adaptable_reviving());
        assert!(!monster.test_subject_nemesis_apply_intangible());
        assert_eq!(monster.test_subject_extra_multi_claw_count(), 0);

        assert!(monster.set_test_subject_respawns(2));
        monster.set_test_subject_adaptable_reviving(true);
        monster.set_test_subject_nemesis_apply_intangible(true);
        assert!(monster.set_test_subject_extra_multi_claw_count(i32::MAX));
        assert_eq!(monster.test_subject_respawns(), 2);
        assert!(monster.test_subject_adaptable_reviving());
        assert!(monster.test_subject_nemesis_apply_intangible());
        assert_eq!(monster.test_subject_extra_multi_claw_count(), i32::MAX);

        assert!(!monster.set_test_subject_respawns(3));
        assert_eq!(monster.test_subject_respawns(), 2);
        assert!(!monster.set_test_subject_extra_multi_claw_count(-1));
        assert_eq!(monster.test_subject_extra_multi_claw_count(), i32::MAX);

        monster.set_test_subject_adaptable_reviving(false);
        monster.set_test_subject_nemesis_apply_intangible(false);
        assert_eq!(monster.test_subject_respawns(), 2);
        assert!(!monster.test_subject_adaptable_reviving());
        assert!(!monster.test_subject_nemesis_apply_intangible());
    }

    #[test]
    fn a_default_state_carries_the_python_dataclass_defaults() {
        let state = HotState::at_defaults();
        assert_eq!(state.max_hp, 999);
        assert_eq!(state.energy, 3);
        assert_eq!(state.turn, 1);
        assert_eq!(state.inky_attack_damage, 1);
        assert_eq!(state.regalite_block_amount, 6);
        assert_eq!(state.hp, 0);
        assert!(state.powers.is_empty());
        assert!(state.monsters.is_empty());
    }

    #[test]
    fn the_bomb_sidecar_allocates_instances_in_order_and_is_copy_on_write() {
        let mut fanouts = HotFanouts::new();
        let first = fanouts.begin_the_bomb(3).unwrap();
        assert_eq!(first, 0);
        fanouts.set_the_bomb_damage(first, 40).unwrap();
        assert!(fanouts.register_before_side_turn_end(PowerId::Hailstorm));
        let second = fanouts.begin_the_bomb(3).unwrap();
        assert_eq!(second, 1);
        fanouts.set_the_bomb_damage(second, 50).unwrap();
        assert_eq!(fanouts.next_the_bomb_uid(), 2);
        assert_eq!(
            fanouts.before_side_turn_end_order(),
            &[
                BeforeSideTurnEndToken::TheBomb(0),
                BeforeSideTurnEndToken::Hailstorm,
                BeforeSideTurnEndToken::TheBomb(1),
            ]
        );
        assert_eq!(
            fanouts.the_bomb_instances().collect::<Vec<_>>(),
            &[
                TheBombInstance {
                    uid: 0,
                    turns: 3,
                    damage: 40,
                },
                TheBombInstance {
                    uid: 1,
                    turns: 3,
                    damage: 50,
                },
            ]
        );
        assert!(fanouts.the_bomb_state_is_exact());

        let snapshot = fanouts.clone();
        assert_eq!(fanouts.decrement_the_bomb(0), Ok(2));
        assert_eq!(fanouts.remove_the_bomb(1).unwrap().damage, 50);
        assert_eq!(snapshot.the_bomb_instances().next().unwrap().turns, 3);
        assert_eq!(snapshot.the_bomb_instances().count(), 2);
        assert_eq!(fanouts.the_bomb_instances().count(), 1);
        assert!(snapshot.the_bomb_state_is_exact());
        assert!(fanouts.the_bomb_state_is_exact());
    }

    #[test]
    fn the_bomb_sidecar_rejects_partial_and_malformed_lifecycle_writes() {
        let mut fanouts = HotFanouts::new();
        assert_eq!(fanouts.begin_the_bomb(2), Err(()));
        assert_eq!(fanouts.begin_the_bomb(3), Ok(0));
        assert!(!fanouts.the_bomb_state_is_exact());
        assert_eq!(fanouts.set_the_bomb_damage(0, 41), Err(()));
        assert_eq!(fanouts.set_the_bomb_damage(7, 40), Err(()));
        assert_eq!(fanouts.set_the_bomb_damage(0, 40), Ok(()));
        assert_eq!(fanouts.set_the_bomb_damage(0, 50), Err(()));
        assert_eq!(fanouts.decrement_the_bomb(0), Ok(2));
        assert_eq!(fanouts.decrement_the_bomb(0), Ok(1));
        assert_eq!(fanouts.decrement_the_bomb(0), Err(()));
        assert_eq!(fanouts.remove_the_bomb(0).unwrap().damage, 40);
        assert!(fanouts.the_bomb_instances_are_empty());
        assert!(fanouts.before_side_turn_end_order().is_empty());
    }

    #[test]
    fn synchronous_damage_receipts_and_negative_panache_are_clone_isolated() {
        let mut original = HotFanouts::new();
        let uid = original.begin_panache_instance(10).unwrap();
        assert_eq!(original.advance_panache_after_card_played(uid), Ok(None));
        for _ in 0..4 {
            assert_eq!(original.advance_panache_after_card_played(uid), Ok(None));
        }
        assert_eq!(
            original.advance_panache_after_card_played(uid),
            Ok(Some(10))
        );
        assert_eq!(
            original.advance_panache_after_card_played(uid),
            Ok(Some(10))
        );
        assert_eq!(original.panache_left(), -1);
        original.enter_damage_batch();
        original.register_batch_death(7);
        original.register_batch_death(9);
        let mut branch = original.clone();
        branch.begin_batch_death_cleanup(7);
        branch.begin_batch_death_cleanup(9);
        assert!(branch.monster_death_cleanup_is_active(7));
        assert!(branch.finish_batch_death_cleanup(9));
        assert!(branch.finish_batch_death_cleanup(7));
        assert!(branch.leave_damage_batch());
        branch.finish_panache_after_damage(uid).unwrap();
        assert!(
            branch.synchronous_damage_is_active(),
            "outer positive-count callback still active"
        );
        assert_eq!(branch.panache_left(), 5);
        branch.finish_panache_after_damage(uid).unwrap();
        assert!(!branch.synchronous_damage_is_active());
        assert_eq!(original.panache_left(), -1);
        assert!(original.monster_death_is_pending(7));
        assert!(original.monster_death_is_pending(9));
        assert!(original.instanced_player_power_records_are_exact());
    }

    #[test]
    fn toric_and_bomb_share_the_cold_store_without_aliasing_or_reordering() {
        let mut fanouts = HotFanouts::new();
        let retained = DotNetDecimal::from_nonnegative_fraction(15, 4).unwrap();
        assert_eq!(fanouts.apply_toric_toughness(2, retained), Ok(2));
        let bomb = fanouts.begin_the_bomb(3).unwrap();
        fanouts.set_the_bomb_damage(bomb, 40).unwrap();
        assert!(fanouts.toric_toughness_state_is_exact());
        assert!(fanouts.the_bomb_state_is_exact());
        assert_eq!(
            fanouts.after_block_cleared_order(),
            &[PowerId::ToricToughness]
        );

        let snapshot = fanouts.clone();
        assert_eq!(
            fanouts.apply_toric_toughness(2, DotNetDecimal::from_i64(6)),
            Ok(4)
        );
        assert_eq!(fanouts.toric_toughness().unwrap().duration, 4);
        assert_eq!(
            fanouts
                .toric_toughness()
                .unwrap()
                .block
                .nonnegative_fraction(),
            Some((6, 1))
        );
        assert_eq!(snapshot.toric_toughness().unwrap().duration, 2);
        assert_eq!(
            snapshot
                .toric_toughness()
                .unwrap()
                .block
                .nonnegative_fraction(),
            Some((15, 4))
        );
        assert_eq!(snapshot.the_bomb_instances().count(), 1);

        let before = fanouts.clone();
        assert!(!fanouts.set_toric_toughness(Some(ToricToughnessInstance {
            duration: -1,
            block: retained,
        })));
        assert_eq!(fanouts, before, "invalid direct writes are atomic");
    }

    #[test]
    fn toric_and_sloth_coexist_in_canonical_cold_order_from_both_acquisition_orders() {
        fn toric_then_sloth() -> HotFanouts {
            let mut fanouts = HotFanouts::new();
            fanouts
                .apply_toric_toughness(2, DotNetDecimal::from_i64(5))
                .unwrap();
            assert!(fanouts.set_sloth(Some(SlothInstance {
                cards_played_this_turn: 4,
            })));
            fanouts
        }
        fn sloth_then_toric() -> HotFanouts {
            let mut fanouts = HotFanouts::new();
            assert!(fanouts.set_sloth(Some(SlothInstance {
                cards_played_this_turn: 4,
            })));
            fanouts
                .apply_toric_toughness(2, DotNetDecimal::from_i64(5))
                .unwrap();
            fanouts
        }
        let left = toric_then_sloth();
        let right = sloth_then_toric();
        assert_eq!(left, right);
        assert!(left.cold_power_records_are_exact());
        assert_eq!(left.toric_toughness().unwrap().duration, 2);
        assert_eq!(left.sloth().unwrap().cards_played_this_turn, 4);

        let snapshot = left.clone();
        let mut changed = left;
        assert_eq!(changed.increment_sloth(), Ok(5));
        assert_eq!(snapshot.sloth().unwrap().cards_played_this_turn, 4);
        assert_eq!(changed.sloth().unwrap().cards_played_this_turn, 5);
        assert_eq!(snapshot.toric_toughness(), changed.toric_toughness());
    }

    #[test]
    fn known_card_flags_cover_every_defined_bit() {
        assert_eq!(CARD_FLAGS_KNOWN & CARD_FLAG_PICK, CARD_FLAG_PICK);
        assert_eq!(
            CARD_FLAGS_KNOWN & CARD_FLAG_DEFAULT_PHYSICAL_STATE,
            CARD_FLAG_DEFAULT_PHYSICAL_STATE
        );
        assert_eq!(CARD_FLAG_PICK & CARD_FLAG_DEFAULT_PHYSICAL_STATE, 0);
        assert_eq!(CARD_FLAGS_KNOWN & CARD_FLAG_LEGACY, CARD_FLAG_LEGACY);
        assert_eq!(
            CARD_FLAGS_KNOWN & CARD_FLAG_SOVEREIGN_BLADE_STATE,
            CARD_FLAG_SOVEREIGN_BLADE_STATE
        );
        assert_eq!(CARD_FLAGS_KNOWN & CARD_FLAG_RINGING, CARD_FLAG_RINGING);
        assert_eq!(CARD_FLAGS_KNOWN & CARD_FLAG_BOUND, CARD_FLAG_BOUND);
        assert_eq!(CARD_FLAGS_KNOWN & CARD_FLAG_HEXED, CARD_FLAG_HEXED);
        assert_eq!(CARD_FLAGS_KNOWN & CARD_FLAG_DUPE, CARD_FLAG_DUPE);
        assert_eq!(
            CARD_FLAGS_KNOWN & CARD_FLAG_LOCAL_EXHAUST,
            CARD_FLAG_LOCAL_EXHAUST
        );
        assert_eq!(
            CARD_FLAGS_KNOWN & CARD_FLAG_GENETIC_ALGORITHM_STATE,
            CARD_FLAG_GENETIC_ALGORITHM_STATE
        );
        assert_eq!(
            CARD_FLAGS_KNOWN & CARD_FLAG_MELANCHOLY,
            CARD_FLAG_MELANCHOLY
        );
        assert_eq!(CARD_FLAGS_KNOWN.count_ones(), 11);
    }

    #[test]
    fn packed_affliction_and_deep_relic_bits_affect_equality() {
        let baseline = HotState::at_defaults();
        let mut changed = baseline.clone();
        assert!(changed.set_bound_afflictions_this_turn(2));
        changed.set_bound_card_played(true);
        assert_ne!(changed, baseline);
        assert!(changed.card_affliction_state_is_exact());

        for relic_bit in [1 << 4, 1 << 5, 1 << 6, 1 << 7] {
            let mut owned = baseline.clone();
            owned.forge_card_affliction_state_for_test(relic_bit);
            assert!(owned.card_affliction_state_is_exact());
            assert_ne!(owned, baseline);
        }
    }

    #[test]
    fn after_energy_reset_ledger_is_known_empty_cow_and_reapply_ordered() {
        let mut fanouts = HotFanouts::new();
        fanouts.initialize_after_energy_reset_order_at_entry();
        assert_eq!(fanouts.after_energy_reset_order(), Some(&[][..]));
        assert!(fanouts.register_after_energy_reset(AfterEnergyResetPower::Genesis));
        assert!(fanouts.register_after_energy_reset(AfterEnergyResetPower::Radiance));
        assert!(fanouts.register_after_energy_reset(AfterEnergyResetPower::Genesis));
        let snapshot = fanouts.clone();
        fanouts.unregister_after_energy_reset(AfterEnergyResetPower::Genesis);
        assert!(fanouts.register_after_energy_reset(AfterEnergyResetPower::Genesis));
        assert_eq!(
            fanouts.after_energy_reset_order(),
            Some(
                &[
                    AfterEnergyResetPower::Radiance,
                    AfterEnergyResetPower::Genesis,
                ][..]
            )
        );
        assert_eq!(
            snapshot.after_energy_reset_order(),
            Some(
                &[
                    AfterEnergyResetPower::Genesis,
                    AfterEnergyResetPower::Radiance,
                ][..]
            )
        );

        // Inline padding is not provenance: a remove/reapply sequence must
        // compare equal to a cold-restored live slice with the same order.
        let mut expected = HotState::at_defaults();
        expected.fanouts.set_after_energy_reset_order(&[
            AfterEnergyResetPower::Radiance,
            AfterEnergyResetPower::Genesis,
        ]);
        let mut actual = HotState::at_defaults();
        actual.fanouts.set_after_energy_reset_order(&[
            AfterEnergyResetPower::Genesis,
            AfterEnergyResetPower::Radiance,
        ]);
        actual
            .fanouts
            .unregister_after_energy_reset(AfterEnergyResetPower::Genesis);
        actual
            .fanouts
            .register_after_energy_reset(AfterEnergyResetPower::Genesis);
        assert_eq!(actual, expected);

        fanouts.mark_after_energy_reset_order_legacy_unknown();
        assert!(fanouts.register_after_energy_reset(AfterEnergyResetPower::Spinner));
        assert_eq!(fanouts.after_energy_reset_order(), None);
        assert!(!fanouts.set_after_energy_reset_order(&[
            AfterEnergyResetPower::Genesis,
            AfterEnergyResetPower::Genesis,
        ]));
        assert_eq!(fanouts.after_energy_reset_order(), None);

        // Removing the last legacy-inferred listener restores legacy absence;
        // an explicit entry/supplied list retains its known-empty witness.
        assert!(fanouts.set_after_energy_reset_order(&[AfterEnergyResetPower::Genesis]));
        fanouts.mark_after_energy_reset_order_legacy_inferred();
        fanouts.unregister_after_energy_reset(AfterEnergyResetPower::Genesis);
        assert_eq!(fanouts.after_energy_reset_order(), None);

        assert!(fanouts.set_after_energy_reset_order(&[AfterEnergyResetPower::Genesis]));
        fanouts.unregister_after_energy_reset(AfterEnergyResetPower::Genesis);
        assert_eq!(fanouts.after_energy_reset_order(), Some(&[][..]));
        assert!(fanouts.after_energy_reset_order_is_explicit());
    }
}

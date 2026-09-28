//! The run-RNG opening: everything `combat_sim.start_combat` does after the
//! entry facts are set (#2528 E4a, spec §A3).
//!
//! # Where the cut is
//!
//! `combat_sim.start_combat` (frozen Python, deleted #2827) splits at the
//! line where the run RNG is first constructed. The part above it is pure entry
//! construction — [`crate::entry`] (#2511 E1–E3) — and the part below is the
//! run-RNG-driven opening this module ports.
//!
//! # The order, and the one reordering that is allowed
//!
//! The game creates the roster **before** the deck exists:
//! `CombatRoom::StartCombat` (`0x58d4c`, body
//! `CombatRoom/<StartCombat>d__46::MoveNext` `0x310bf0`) runs
//! `EncounterModel::GenerateMonstersWithSlots` (`0x7f88c`) and then
//! `CombatManager::SetUpCombat` (`0x135900`), whose
//! `Player::PopulateCombatState` performs the opening shuffle. Python does the
//! opposite — `rng.shuffle(pile)` (frozen Python `start_combat`, deleted #2827), `make_monsters` in `start_combat` —
//! and this module keeps Python's order.
//!
//! **That is exact only because the two consume disjoint streams**: the
//! shuffle spends `Shuffle`, creation spends `Niche` and the per-fight
//! `Encounter`. The moment an opening step consumes two streams the freedom
//! disappears, so a new step must be placed against the *game's* order, not
//! this one.
//!
//! # The two combat-start fire points
//!
//! `AfterRoomEntered` (one pass, run-level listeners only) fires after the
//! deck is instantiated and shuffled and **before** the deal;
//! `BeforeCombatStart` (two passes, the second `…Late`) fires after it. Both
//! go through [`crate::engine`], which owns their dispatch semantics and their
//! IL citations.
//!
//! They are not the last hooks in the window. `start_combat` closes with
//! `begin_player_turn(s)` (frozen Python, deleted #2827), so the player's **first**
//! `BeforeSideTurnStart` / `AfterSideTurnStart` / `AfterPlayerTurnStart` walks
//! are inside the opening too — which is why the relic gate has a turn-1 half
//! ([`TURN_ONE_RELIC_BODIES`], #2731) and not only a combat-start one.
//!
//! # I5 in this module
//!
//! Every step is either ported exactly with its citation, or refused by a Rust
//! name from [`refusal::OpeningRefusal`]. The gates are placed at Python's own
//! gate positions so the reachability argument travels with the line numbers,
//! and the roster refusal is reached **before** any un-ported step, so a fight
//! is never half-built.

#[cfg(test)]
mod fixture_tests;
pub mod mcr;
pub mod refusal;
pub mod roster;
pub mod shuffle;

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::boundary::HotBoundary;
use crate::canonical::{
    CanonicalCardV2, CanonicalEntityV2, CanonicalRngV2, CanonicalStateV2, STATE_SCHEMA_V2,
};
use crate::catalog::RewardPool;
use crate::encounters::{MonsterSpec, SpawnValue};
use crate::entry::counters::{COMBAT_STREAMS, OPTIONAL_COMBAT_STREAMS};
use crate::entry::document::EntryDocument;
use crate::ids::{CardId, EnchantmentId, MonsterKind, PowerId};
use crate::rng::{Xoshiro256StarStar, run_set_seed_v109};

use self::mcr::McrOpeningChecksum;
use self::refusal::OpeningRefusal;
use self::roster::make_monsters;

/// Options one opening is built under.
#[derive(Clone, Copy, Debug, Default)]
pub struct OpeningOptions<'a> {
    /// The capture's first checksum, when `--mcr` was requested.
    pub mcr_first_checksum: Option<&'a McrOpeningChecksum>,
    /// Record the native checkpoints the deal passes
    /// ([`Opening::native_checkpoints`], `entry --native-checkpoints`, #3392).
    pub record_native_checkpoints: bool,
}

/// One native checkpoint the opening's deal passed (#3392): the boundary, and
/// its state projected to canonical v2 or the boundary's refusal by name.
#[derive(Clone, Debug, PartialEq)]
pub struct OpeningNativeCheckpoint {
    /// The boundary ([`crate::engine::native_checkpoint::NativeCheckpointKind`]).
    pub kind: crate::engine::native_checkpoint::NativeCheckpointKind,
    /// The state at that boundary, or why it does not project.
    pub state: Result<CanonicalStateV2, String>,
}

/// One built opening.
#[derive(Clone, Debug, PartialEq)]
pub struct Opening {
    /// The post-opening state, in the wire contract both engines share.
    pub document: CanonicalStateV2,
    /// Whether the opening-checksum RNG splice was applied (§A5 obligation 2).
    pub mcr_splice_applied: bool,
    /// The nine streams as the opening computed them, **before** any splice.
    ///
    /// The sharp regression net: an opening that consumes the wrong number of
    /// draws shows up here even when the splice would have hidden it.
    pub pre_splice_rng: BTreeMap<String, CanonicalRngV2>,
    /// The native checkpoints the deal passed, in order, when
    /// [`OpeningOptions::record_native_checkpoints`] asked; `None` otherwise.
    ///
    /// Native writes the capture's opening checksum ("After player turn
    /// start") BEFORE `RunAutoPrePlayPhase`, but [`Self::document`] is the
    /// post-AutoPre state: a turn-one Imbued AutoPlay has already run in it.
    /// The certification census compares the opening checksum against the
    /// recorded [`NativeCheckpointKind::AfterPlayerTurnStart`] state instead
    /// (#3392, IL cited on the kind).
    ///
    /// [`NativeCheckpointKind::AfterPlayerTurnStart`]:
    ///     crate::engine::native_checkpoint::NativeCheckpointKind::AfterPlayerTurnStart
    pub native_checkpoints: Option<Vec<OpeningNativeCheckpoint>>,
}

// ---------------------------------------------------------------------------
// The gate tables
// ---------------------------------------------------------------------------

/// Relics with an **opening-window** body this crate has no subscriber for.
///
/// **Derived, not curated.** The oracle's own relic-hook manifest
/// (`combat_sim._current_relic_hooks`) names, at v0.111.0, exactly 46 relics
/// that declare a body on one of the five hooks the opening and the first deal
/// run — `AfterRoomEntered`, `BeforeCombatStart`, `BeforeCombatStartLate`,
/// `BeforeHandDraw`, `ModifyHandDraw` — and are **not** in
/// `combat_sim.TEMPLATE_RELICS`:
///
/// ```text
/// python3.12 -c "import combat_sim as s
/// W = {'AfterRoomEntered', 'BeforeCombatStart', 'BeforeCombatStartLate',
///      'BeforeHandDraw', 'ModifyHandDraw'}
/// print([r for r in sorted(s.KNOWN_RELICS)
///        if {h.split('[')[0] for h in s._current_relic_hooks(r)} & W
///        and r not in s.TEMPLATE_RELICS])"
/// ```
///
/// Those five are **not** the whole opening window — see
/// [`TURN_ONE_RELIC_BODIES`] for the three turn-start hooks `start_combat`'s
/// closing `begin_player_turn` also fires.
///
/// This crate reaches a subscriber for those hooks **only** through
/// `content_tables::TEMPLATE_RELIC_STEPS`, so a non-template relic with a body
/// there has no Rust subscriber at all: firing the hook would do nothing, and
/// the opening would emit a state the oracle disagrees with, **silently**.
/// That is the failure mode I5 exists to prevent, and it is not hypothetical —
/// before this gate, 14 of 14 corpus fights holding `RELIC.CRACKED_CORE`, 5 of
/// 5 holding `RELIC.VAJRA` and every Silent starter run (`RING_OF_THE_SNAKE`,
/// two extra opening draws) produced a digest-divergent document instead of a
/// refusal.
///
/// The list that one-liner prints has **46** entries at the `main` this was
/// last re-derived against (re-derived again 2026-09-22: still 46), and
/// **thirty-eight** of them are deliberately not here (eleven when this
/// sentence was first written; Vajra, Bound Phylactery, Byrdpip, Pael's Legion,
/// Ring of the Snake, Girya, Petrified Toad, Letter Opener, Pael's Flesh,
/// Eternal Feather, Ghost Seed, Kusarigama, Phylactery Unbound, Pollinous
/// Core, Vambrace, Stone Cracker, Sling of Courage, Toolbox, Meat on the Bone,
/// Pocketwatch, Unsettling Lamp, Blessed Antler, Jeweled Mask, Funerary Mask,
/// Radiant Pearl, Big Mushroom and Tea of Discourtesy left the table since) —
/// exactly
/// `OPENING_WINDOW_GATE_EXCLUSIONS`, which a test pins against the oracle's own
/// manifest (`fixtures/opening_relic_hooks_v1.json`) so neither the count in
/// this sentence nor the list below can drift from the oracle.
///
/// Five because the opening runs their body itself rather than reaching for a
/// subscriber — `ANCHOR`, `BRONZE_SCALES`, `GORGET`, `ODDLY_SMOOTH_STONE` and
/// `EMBER_TEA`. The first four seed owner state and are written in
/// [`pre_hook_document`] with their own `CanonicalVars` citation; Ember Tea
/// seeds a counter there **and** runs its room-entry Strength grant and
/// decrement in [`apply_room_entry_strength`], because unlike the other four
/// it mutates state rather than only initialising it.
///
/// `PLANISPHERE` and `PANTOGRAPH` because their bodies are node-type
/// conditional: outside the point or room each one heals on, neither does
/// anything, so refusing them unconditionally here would over-refuse. Since
/// #3162 Pantograph's boss-room heal runs in [`build`]
/// ([`pantograph_before_combat_start`]), and Planisphere keeps its own, more
/// specific refusal ([`OpeningRefusal::RoomEntryHealUnmodeled`], spec Part C6)
/// only on the `?` point its body reads ([`planisphere_heals_here`]).
/// `BOOMING_CONCH` is not here either — its own gate refuses the only case it
/// can be wrong in.
///
/// # The three room-entry bodies this slice modeled (#2693)
///
/// These left the table on 2026-09-22, each with a parity witness in
/// `fixture_tests` against Python's document for the same save. IL
/// re-read on `solver/dll-archive/v0.111.0/data_sts2_macos_arm64/sts2.dll`,
/// sha256 `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.
///
/// * `MEAL_TICKET` — **combat-inert, read from the DLL rather than asserted.**
///   `MealTicket/<AfterRoomEntered>d__5::MoveNext` (RVA `0x32a63c`) tests
///   `room isinst MerchantRoom` at `IL_0034`-`IL_003f` and `leave`s the body at
///   `IL_0041` when the room is anything else, so in a combat room its only
///   effect is to return. The heal it would otherwise apply is
///   `CreatureCmd::Heal` at `IL_0068` with `get_CanonicalVars` (`0x96a09`)
///   `ldc.i4.s 15`. The oracle agrees by having no `start_combat` body for it
///   at all (`RELIC.MEAL_TICKET` appears nowhere in `combat_sim.py`), and
///   `ENCOUNTER_MECHANICS.md` already listed it as verified combat-inert. This
///   is the same category as `TURN_ONE_GATE_EXCLUSIONS`' `AKABEKO`: nothing to
///   run, so nothing to refuse.
/// * `BAG_OF_PREPARATION` — +2 cards on the first player turn, now folded into
///   `engine::relics::modifier_total`'s `ModifyHandDraw` arm, where the IL and
///   oracle citations live. The opening already wrote the `bag_draws` mirror
///   (`boundary.rs`, `2 * (owns(BagOfPreparation) + owns(RingOfTheSnake))`);
///   what was missing was a reader, so before this slice the relic reached the
///   projected document and its effect did not. `RING_OF_THE_SNAKE` shares that
///   body byte for byte; it stayed gated until #2827 witnessed it (below).
/// * `PENDULUM` — the gate entry was **stale**: both halves of the body were
///   already in `engine::relics` (`before_hand_draw`'s
///   `(pendulum + 1) % PENDULUM_TURNS` advance and the `pendulum() == 0`
///   `ModifyHandDraw` row), and the opening already seeded the persistent
///   counter through [`SEEDED_COUNTER_RELICS`]. The manifest says so too —
///   `rust_body_site` is `true` for it — and nothing was checking, because the
///   five-hook half of the pin test asserts membership without consulting
///   `rust_body_site` the way the turn-1 half does. That asymmetry is the
///   reason this entry survived and is recorded in the pin test itself.
///
/// # The five that left in #3162
///
/// IL re-read on the installed v0.111.0 `sts2.dll`, sha256 `9cb4f1ad…fbf12b4`
/// (hash-verified 2026-09-26). The opening checkpoint of each corpus capture
/// is the witness (the frozen Python oracle that printed the parity digests
/// above is deleted, #2827).
///
/// * `JEWELED_MASK` — its one hook, the turn-1 `BeforeHandDraw` free Power, is
///   `engine::relics::jeweled_mask_before_hand_draw`, which also gained the
///   IL's non-Innate preference here. Its move breaks the prefix-deal premise
///   the uid check rests on, so the check reorders for it
///   ([`jeweled_mask_moved_order`]).
/// * `FUNERARY_MASK` and `RADIANT_PEARL` — their turn-1 `BeforeHandDraw`
///   generators were already `engine::relics` rows; the uid check covers
///   their pre-deal allocations ([`pre_deal_allocations`]).
///   These three and `NINJA_SCROLL` move Draw or Hand cards on the same hook,
///   so any two refuse ([`refuse_unordered_hand_draw_card_peers`]).
/// * `BIG_MUSHROOM` — **stale**, like Pendulum was. `ModifyHandDraw` (RVA
///   `0x90aa8`: owner test, `TurnNumber == 1`, `draw - Cards` with
///   `CardsVar(2)` from `get_CanonicalVars` `0x90a31`) is already the
///   `-2` row of `engine::relics::modifier_total`, and its `AfterRoomEntered`
///   (`0x90a9b`) is `Grow` (`0x90ae5`), an `NCreature::ScaleTo(1.5)` on the
///   combat room's node — presentation, no model write. `AfterObtained`'s
///   `GainMaxHp(20)` is the pickup, outside combat.
/// * `TEA_OF_DISCOURTESY` — a spent Tea's `BeforeCombatStart` is inert and
///   opens with `tea_discourtesy_combats_left` 0; a charged one keeps the
///   refusal it had here ([`tea_of_discourtesy_spent`]).
///
/// # `VAJRA` (#2827)
///
/// Left the table on 2026-09-22 with parity witnesses against Python's
/// document for the same save. `Vajra/<AfterRoomEntered>d__6::MoveNext`
/// (RVA `0x333abc`) `leave`s at `IL_002a` unless `room isinst CombatRoom`
/// (`IL_0023`), then `Apply<StrengthPower>` at `IL_0062` on the owner's
/// creature for `DynamicVars.Strength.BaseValue` — `get_CanonicalVars`
/// (`0x9d6ad`) builds it from `Decimal::One` ([`VAJRA_STRENGTH`]). It is one
/// of the oracle's `room_entry_strength_sources`, so it now runs in
/// [`apply_room_entry_strength`] beside Ember Tea and counts toward the Ruined
/// Helmet distinct-amount guard.
///
/// # `BYRDPIP` and `PAELS_LEGION` (#2827)
///
/// Left the table on 2026-09-23 with parity witnesses against Python's
/// document for the same save. Each relic's `BeforeCombatStart` is
/// `SummonPet` → `PlayerCmd::AddPet<…>(Owner)`: a 9999/9999 passive pet, no
/// RNG. The IL citations are in `engine::passive_relic_pets_before_combat_start`,
/// which runs where the oracle does (frozen Python `start_combat`, deleted #2827). The pet
/// itself is a projection of ownership in `boundary.rs` (`relic_pets`), which
/// refuses a document whose roster disagrees with ownership, so
/// [`pre_hook_document`] writes the roster, and says there why that is exact.
/// Pael's Legion's `paels_legion_cooldown = 0` was already seeded there. Pael's Legion's turn-1 `AfterSideTurnStart` was
/// already modeled in `engine::relics::after_side_turn_start_late`, and it is
/// a no-op on turn 1 because the cooldown starts at 0.
///
/// # `RING_OF_THE_SNAKE` (#2827)
///
/// Left the table on 2026-09-23. It had been a **deliberate over-refusal**:
/// `engine::relics::modifier_total` already contributed its +2 from the same
/// row shape as Bag of Preparation's (the oracle's `bag_draws` is one field
/// summing both, frozen Python `start_combat`, deleted #2827, read on turn one in `_begin_player_turn_hand_draw`),
/// and the gate stayed only because no Silent fight had been measured.
///
/// IL re-read on the v0.111.0 DLL for this retirement: `RingOfTheSnake`
/// declares exactly three members besides its constructor —
/// `get_Rarity` (`0x9a739`), `get_CanonicalVars` (`0x9a73c`, `ldc.i4.2;
/// newobj CardsVar::.ctor`) and `ModifyHandDraw` (`0x9a749`: owner check
/// `IL_0001`-`IL_000b`, `TurnNumber; ldc.i4.1; ble.s` at `IL_000c`-`IL_001d`,
/// `draw + DynamicVars.Cards.BaseValue` at `IL_0021`-`IL_0037`). No
/// `AfterRoomEntered`, `BeforeCombatStart`, `BeforeHandDraw` or turn-start
/// body exists, so `ModifyHandDraw` is the only opening hook it has, and the
/// manifest agrees (`window_hooks: ["ModifyHandDraw"]`, `turn_one_hooks: []`).
///
/// Witnesses: `fixture_tests` pins the oracle's digest for the fixture save
/// with the ring appended, with ring **and** Bag of Preparation (the shared
/// field reads 4, nine cards), and for a synthetic Silent starter save. The corpus witness is the opening census:
/// the Silent captures this freed open with Python's digest, and the rest
/// reach the next gate by name (see the PR body for #2827's measurement).
///
/// # `GIRYA` and `PETRIFIED_TOAD` (#2827)
///
/// Left the table on 2026-09-23 with parity witnesses against Python's
/// document for the same save. IL read on the v0.111.0 DLL (sha256
/// `9cb4f1ad…`).
///
/// * `GIRYA` — `Girya/<AfterRoomEntered>d__14::MoveNext` (RVA `0x325a60`)
///   `leave`s at `IL_0029` unless `get_TimesLifted` is `> 0`
///   (`IL_0021`-`IL_0027`), `leave`s at `IL_003b` unless `room isinst
///   CombatRoom` (`IL_0034`), then awaits `Apply<StrengthPower>` at `IL_006e`
///   on the owner's creature for `(decimal)TimesLifted` (`IL_0057`-`IL_005c`).
///   Its only other override that reads state, `TryModifyRestSiteOptions`
///   (`0x9469b`), is a rest-site hook. The amount is the saved `TimesLifted`,
///   validated in [`girya_lifts`], and it is one of the oracle's
///   `room_entry_strength_sources` (frozen Python `start_combat`, deleted #2827), so it runs in
///   [`apply_room_entry_strength`] and counts toward the Ruined Helmet
///   distinct-amount guard.
/// * `PETRIFIED_TOAD` — its one override is `BeforeCombatStartLate` (RVA
///   `0x9939c`). `<BeforeCombatStartLate>d__4::MoveNext` (`0x32dd04`) is
///   `Flash` and then an unconditional
///   `PotionCmd::TryToProcure<PotionShapedRock>(Owner)` at `IL_0029`. The body
///   is `engine::petrified_toad_before_combat_start_late`, the second pass of
///   [`crate::engine::fire_before_combat_start`], where the oracle runs it
///   (`start_combat`). The oracle requires an exact positive belt
///   capacity for it (`start_combat`), and so does [`build_pre_hook`]
///   ([`OpeningRefusal::PotionBeltNotExact`]).
///
/// # `LETTER_OPENER` and `PAELS_FLESH` (#2827)
///
/// Left the table on 2026-09-23. Both were **over-refusals**: each one's
/// window body is `BeforeCombatStart`, and in IL neither mutates combat state
/// the document carries beyond what the opening already writes. Their
/// in-fight bodies (`AfterCardPlayed`, `ModifyMaxEnergy`) were already in
/// `engine::relics`. IL re-read on the v0.111.0 DLL
/// (sha256 `9cb4f1ad…`, `dump_il.py LetterOpener` / `PaelsFlesh`):
///
/// * `LetterOpener::BeforeCombatStart` (RVA `0x963a0`) is
///   `ldc.i4.0; set_SkillsPlayedThisTurn` at `IL_0001`-`IL_0003` and
///   `ldc.i4.0; RelicModel::set_Status` at `IL_0008`-`IL_000a`, nothing else.
///   The counter's Rust home is `history.skill_plays_finished_this_turn`
///   (`boundary.rs`'s `LetterOpenerSkills` slot projects it for an owner and
///   refuses a document where the two disagree), which a fresh combat state
///   holds at 0, so the reset is already true of every opened document.
///   `Status` is presentation. `LetterOpener::AfterSideTurnStart` (`0x963b8`),
///   the turn-1 hook, tests `TurnNumber; ldc.i4.1; bne.un.s` at
///   `IL_0025`-`IL_0036` and returns at `IL_0038` **on** turn one: only a
///   later turn reaches the reset at `IL_003e`-`IL_0040`. The oracle agrees:
///   `start_combat` writes only the `letter_opener` ownership mirror
///   (frozen Python, deleted #2827; projected by `boundary.rs` from the catalog), and
///   its turn-start reset is `s.turn > 1` (`_turn_start_after_royal`).
/// * `PaelsFlesh::BeforeCombatStart` (`0x98351`) is only
///   `InvokeDisplayAmountChanged` (`IL_0001`-`IL_0002`), and
///   `BeforeSideTurnStart` (`0x9835e`) is the participants test then the same
///   display call (`IL_001a`-`IL_001b`). `AfterSideTurnStart` (`0x98384`)
///   returns at `IL_0038` while `TurnNumber < 3` (`ldc.i4.3; bge.s` at
///   `IL_0035`-`IL_0036`), and past it sets `Status`/`Flash`, presentation
///   only. The gameplay body is `ModifyMaxEnergy` (`0x9831e`): `TurnNumber;
///   ldc.i4.3; bge.s` at `IL_0012`-`IL_0018`, then `+ DynamicVars.Energy`
///   (`get_CanonicalVars` `0x98304`, `ldc.i4.1`). It is
///   `engine::relics::modifier_total`'s `state.turn >= 3` row, the oracle's
///   `PAELS_FLESH_ENERGY if s.paels_flesh and s.turn >= 3`
///   (`_player_max_energy`), so on turn one it adds nothing on either
///   side. `start_combat` writes only the `paels_flesh` mirror.
///
/// **Hooks verified:** every hook of the eight the opening fires that either
/// relic declares — Letter Opener's `BeforeCombatStart` and
/// `AfterSideTurnStart`, Pael's Flesh's `BeforeCombatStart`,
/// `BeforeSideTurnStart` and `AfterSideTurnStart` (the manifest's
/// `window_hooks`/`turn_one_hooks` rows) — plus Pael's Flesh's
/// `ModifyMaxEnergy`, which the first turn's energy reset reads. Same-hook
/// ordering needs no new refusal: every body above is inert on turn one, and
/// the oracle's `start_combat` names neither relic outside the two ownership
/// mirrors. `engine::admission` is stricter and stays so: it refuses either
/// relic beside Crossbow (`Crossbow AfterSideTurnStart acquisition order`)
/// and Pael's Flesh beside Big Hat (`Big Hat same-hook relic acquisition
/// order`).
/// Witnesses in `fixture_tests` pin the oracle's digest for the fixture save
/// with each relic, and with both.
///
/// # `ETERNAL_FEATHER` and `GHOST_SEED` (#2827)
///
/// Left the table on 2026-09-23 with parity witnesses against Python's
/// document for the same save. IL read on the v0.111.0 DLL (sha256
/// `9cb4f1ad…`).
///
/// * `ETERNAL_FEATHER` — combat-inert in IL, as Meal Ticket is.
///   `EternalFeather/<AfterRoomEntered>d__4::MoveNext` (RVA `0x323ea8`)
///   `leave`s at `IL_002d` unless `room isinst RestSiteRoom` (`IL_0026`); the
///   heal after it (`CreatureCmd::Heal` at `IL_0093`) is rest-site only. It
///   declares no other hook (`get_Rarity`, `get_CanonicalVars` and the
///   constructor are its only other members), and the oracle runs no body for
///   it (`content/relics/passive.py`: "rest-site heal only").
/// * `GHOST_SEED` — `GhostSeed::AfterRoomEntered` (RVA `0x9453c`) marks every
///   Basic Strike/Defend in `AllCards` Ethereal in a combat room. The body is
///   `engine::cards::ghost_seed_after_room_entered`, run by
///   [`crate::engine::fire_after_room_entered`] after the compiled template
///   walk, where the oracle runs it (frozen Python `start_combat`, deleted #2827); the IL and
///   ordering citations are there. Its other opening-reachable hook,
///   `AfterCardEnteredCombat` (`0x94503`), already ran for every card
///   generated during the window (`engine::cards`'
///   `apply_physical_card_after_entered_suffix`), and it has no turn-1 hook.
///
/// # `POLLINOUS_CORE` and `VAMBRACE` (#2827)
///
/// Left the table on 2026-09-23. IL re-read on the v0.111.0 DLL (sha256
/// `9cb4f1ad…`, `dump_il.py PollinousCore` / `Vambrace`):
///
/// * `POLLINOUS_CORE` declares two of the window hooks, and both already had
///   their Rust body. `PollinousCore::BeforeHandDraw` (RVA `0x9998c`) is the
///   owner test `ldarg.1; get_Owner; beq.s` at `IL_000c`-`IL_0013` and then
///   `set_TurnsSeen(TurnsSeen + 1)` at `IL_001b`-`IL_0026`.
///   `ModifyHandDraw` (`0x999cc`) is the same owner test, then
///   `TurnsSeen >= DynamicVars["Turns"]` at `IL_0017`-`IL_0032` (`Turns` is
///   `ldc.i4.4` in `get_CanonicalVars` `0x998d9` at `IL_0017`) and
///   `+ DynamicVars.Cards.BaseValue` (`CardsVar(2)`, `IL_0009`) at
///   `IL_0036`-`IL_0047`. `AfterModifyingHandDraw` (`0x99a19`) is
///   `set_TurnsSeen(0)` at `IL_0001`-`IL_0003` plus `DoActivateVisuals`
///   (flash and a wait, display only); `AfterCombatEnd` (`0x999bd`) is only
///   `set_Status`. The oracle is `_pollinous_before_hand_draw`
///   (frozen Python, deleted #2827) and `_begin_player_turn_hand_draw`; Rust's is the `RelicPollinousCore` row of
///   `engine::relics::before_hand_draw`, which on a counter in `0..=3` advances
///   it, or at 3 resets it and returns the activation that
///   `engine::relics::pollinous_hand_draw_bonus` turns into `+2`. The
///   opening reaches both through `engine::deal_opening_hand`. What kept the
///   relic gated was the opening's seed: [`SEEDED_COUNTER_RELICS`] reduced
///   the saved `TurnsSeen` modulo 3, not the oracle's `POLLINOUS_CORE_TURNS`
///   4, so a save at 3 (the turn that grants) would have opened at 0. That is
///   fixed in the same change.
///   Its only same-hook refusal in the oracle is Blessed Antler's
///   `BeforeHandDraw` acquisition-order gate,
///   which `engine::admission` carries by name (`Blessed Antler
///   BeforeHandDraw acquisition order`); Toolbox, Ninja Scroll, Jeweled Mask
///   and Radiant Pearl, its other listed peers, are still gated here.
/// * `VAMBRACE`'s one window hook is `BeforeCombatStart` (RVA `0x9d7c2`):
///   `set_TriggeringCard(null)` at `IL_0001`-`IL_0003`,
///   `set_BlockGainedThisCombat(false)` at `IL_0008`-`IL_000a`, and a status
///   display. Nothing wrote the flag before, so an opened document would have
///   carried `vambrace_available = false` where the oracle's constructor
///   writes `"RELIC.VAMBRACE" in relics`. The body is
///   now `engine::vambrace_before_combat_start`, in pass 1 of
///   [`crate::engine::fire_before_combat_start`]. Its in-combat hooks
///   (`ModifyBlockMultiplicative`, `AfterModifyingBlockAmount`,
///   `AfterCardPlayed`) were already in `engine::damage` / `engine::relics`.
///   The oracle's `start_combat` names it nowhere else, so it owes no new
///   refusal.
///
/// **Hooks verified:** every hook of the eight the opening fires that either
/// relic declares (the manifest's `window_hooks`, `turn_one_hooks` empty for
/// both), plus Pollinous Core's `AfterModifyingHandDraw`, which the same draw
/// runs.
///
/// # `KUSARIGAMA` and `PHYLACTERY_UNBOUND` (#2827)
///
/// Left the table on 2026-09-23 with parity witnesses against Python's
/// document for the same save. IL read on the v0.111.0 DLL (sha256
/// `9cb4f1ad…`, `dump_il.py Kusarigama` / `PhylacteryUnbound`).
///
/// * `KUSARIGAMA` was an **over-refusal**. `Kusarigama::BeforeCombatStart`
///   (RVA `0x959e4`) is `ldc.i4.0; set_AttacksPlayedThisTurn` at
///   `IL_0001`-`IL_0003` and `ldc.i4.0; RelicModel::set_Status` at
///   `IL_0008`-`IL_000a`, nothing else. The counter's Rust home is the hot
///   `kusarigama` slot, which [`pre_hook_document`] already seeds to 0 for an
///   owner, as `start_combat` does (`kusarigama=0 if "RELIC.KUSARIGAMA" in
///   relics else -1`, frozen Python, deleted #2827). `Status` is presentation. It
///   declares no turn-1 hook: its other bodies are `AfterCardPlayed`
///   (`<AfterCardPlayed>d__19` `0x327fe0`, the third-attack hit) and
///   `AfterSideTurnEnd` (`0x959f9`, the reset), both in `engine::relics`.
///   What leaving the gate does reach is the oracle's two `start_combat`
///   gates that name it (Music Box, and Daughter of the Wind with
///   Juggernaut), so they are ported here as
///   [`OpeningRefusal::RelicCombinationRefused`]
///   (`refuse_unordered_kusarigama_peers`).
/// * `PHYLACTERY_UNBOUND` declares bodies on two opening hooks, and both are
///   covered. `BeforeCombatStart`
///   (`<BeforeCombatStart>d__10::MoveNext`, `0x32e234`) summons Osty 5
///   unguarded; it now runs in `engine::phylactery_before_combat_start`, where
///   the IL and the oracle position are cited.
///   The turn-1 `AfterSideTurnStart` (`<AfterSideTurnStart>d__11::MoveNext`,
///   `0x32e134`) summons 2 more with no turn test, and it was already
///   `engine::relics::after_side_turn_start_late`'s row, which the opening
///   reaches through `engine::deal_opening_hand`. The document's Osty is
///   therefore 7/7, as the oracle projects. Holding it with Bound Phylactery
///   refuses by name (`refuse_incompatible_phylacteries`), as `start_combat`
///   does.
///
/// # `MEAT_ON_THE_BONE` and `POCKETWATCH` (#2827 item B)
///
/// Left the table on 2026-09-24, found by the Coach's roots, with parity
/// witnesses against Python's document for the same save. Both were
/// **over-refusals**: each window body is inert on turn one. IL read on the
/// v0.111.0 DLL (sha256 `9cb4f1ad…`, `dump_il.py MeatOnTheBone` /
/// `Pocketwatch`):
///
/// * `MeatOnTheBone::BeforeCombatStart` (RVA `0x96ada`) calls
///   `WillHealOnCombatFinished` and, when it holds, `set_Status(1)`
///   (`IL_0001`-`IL_000b`): presentation only. Its heal is
///   `<AfterCombatVictoryEarly>d__7` (`0x32a748`, `CreatureCmd::Heal` at
///   `IL_0066`), after the fight, and `AfterCurrentHpChanged` (`0x96af0`) only
///   sets `Status`. The oracle has no `start_combat` body for it.
/// * `Pocketwatch::ModifyHandDraw` (`0x99778`) returns the draw unchanged when
///   `TurnNumber == 1` (`IL_0017`-`IL_002b`), so turn one draws no bonus.
///   `BeforeSideTurnStart` (`0x997f1`) moves `_cardsPlayedThisTurn` into
///   `_cardsPlayedLastTurn` and zeroes it (`IL_001a`-`IL_0028`): both are 0 at
///   combat start, as the fresh state already holds.
///   `AfterSideTurnStart` (`0x99824`) is `RefreshCounter` (`0x9984a`),
///   `Status` and a display call. Its in-fight bodies (`AfterCardPlayed`
///   `0x99724`, the turn>1 `ModifyHandDraw` row) were already in
///   `engine::relics`.
///
/// # `BLESSED_ANTLER` (#2992)
///
/// Left the table on 2026-09-25, with parity witnesses against Python's
/// document for the same save. It was an **over-refusal** of the Pollinous
/// Core kind: the body was already `engine::relics`', and since #2973/#2988
/// built every fight-review root with this opening, the gate refused every
/// review after the relic was picked up. IL read on the v0.111.0 DLL (sha256
/// `9cb4f1ad…`, `dump_il.py BlessedAntler`):
///
/// * Its one window hook is `BeforeHandDraw` (stub `0x90d60`, body
///   `<BeforeHandDraw>d__7::MoveNext` RVA `0x31fe64`): the owner test
///   `player == Owner` at `IL_0027`-`IL_0035`, then
///   `Owner.PlayerCombatState.TurnNumber == 1` at `IL_003a`-`IL_004d` (else
///   `leave`), `Flash` (display) at `IL_0053`, then a loop that
///   `CreateCard<Dazed>`s `DynamicVars.Cards` times into one list
///   (`IL_0063`-`IL_0092`; `CardsVar(3)` in `get_CanonicalVars` `0x90cf5`
///   `IL_0011`-`IL_0013`) and ONE `CardPileCmd::AddGeneratedCardsToCombat(
///   list, PileType 1 = Draw, Owner, CardPilePosition 3 = Random)` at
///   `IL_0094`-`IL_009d`. What follows is `PreviewCardPileAdd` and a
///   `Cmd::Wait`, presentation only. So: three fresh L0 Dazed, each at a
///   uniform random Draw index over the growing pile, on the first player
///   turn only. That is the `RelicBlessedAntler` row of
///   `engine::relics::continue_before_hand_draw_after_toolbox` (three
///   `engine::cards::inject_generated_draw_random` calls, the same generated-
///   card transaction Funerary Mask's row runs), which
///   [`crate::engine::deal_opening_hand`] reaches through `begin_player_turn`'s
///   relic `BeforeHandDraw` walk, after the power listeners and before
///   `ModifyHandDraw`. The oracle's body is `_continue_before_hand_draw_relics`
///   / `_add_one_blessed_antler_dazed` (frozen Python, deleted #2827).
/// * Its other member, `ModifyMaxEnergy` (`0x90d38`, owner test then
///   `+ DynamicVars.Energy` = `EnergyVar(1)`), is not a window hook; it is the
///   `RelicBlessedAntler` row of `engine::relics::modifier_total`, read by the
///   first turn's energy reset like any later one.
/// * Combat cannot end inside the body during the opening: the generated-card
///   listeners that can end it (Smokestack's damage) are player powers that
///   no fight holds before its first hand, so the engine's per-card
///   `history.over` early return, which records less than the oracle's
///   post-ending record, is unreachable here.
/// * The row now also promotes `exact_piles` before each insert, as the
///   oracle's `force_exact=True` commit does.
///   That is a solver pile-identity flag, not native state; it was the one
///   field the first parity witness found missing, since no root had reached
///   the row before the opening did.
///
/// Same-hook ordering: native runs relic `BeforeHandDraw` bodies in
/// acquisition order, which neither the run nor the `.mcr` records, and the
/// oracle refuses Blessed Antler beside any other body in that group
/// (frozen Python `start_combat`, deleted #2827). The opening does not run `engine::admission`
/// (which carries it as `Blessed Antler BeforeHandDraw acquisition order`), so
/// that gate is ported here by name ([`refuse_unordered_blessed_antler_peers`]).
/// `PENDULUM`, whose `BeforeHandDraw` counter advance the oracle leaves out of
/// the group, runs first on both sides and touches no pile, RNG or uid.
const OPENING_WINDOW_RELIC_BODIES: [&str; 8] = [
    "RELIC.BELT_BUCKLE",
    "RELIC.DELICATE_FROND",
    "RELIC.FAKE_SNECKO_EYE",
    "RELIC.FUR_COAT",
    "RELIC.NINJA_SCROLL",
    "RELIC.PHILOSOPHERS_STONE",
    "RELIC.RING_OF_THE_DRAKE",
    "RELIC.SNECKO_EYE",
];

/// The thirty-eight window-hook relics deliberately **not** in
/// [`OPENING_WINDOW_RELIC_BODIES`], each justified in that table's doc comment.
///
/// Spelled as data rather than prose so the pin
/// (`fixture_tests::every_opening_window_relic_body_is_gated_or_explicitly_excluded`)
/// can check both directions: a relic that leaves the gate must arrive here
/// with a reason, and an entry here that the oracle no longer names is a stale
/// exclusion. It is a `cfg(test)` const because that pin is its only reader —
/// the production gate is a membership test on the tables, not on this list.
///
/// The last three arrived on 2026-09-22 with the room-entry bodies #2693's hold
/// needs: `MEAL_TICKET` because its body is combat-inert in IL,
/// `BAG_OF_PREPARATION` and `PENDULUM` because the engine now runs theirs.
/// `VAJRA` followed the same day (#2827): the opening's room-entry Strength
/// block ([`apply_room_entry_strength`]) runs its body. `BOUND_PHYLACTERY`
/// followed (#2827): `engine::phylactery_before_combat_start` runs its
/// combat-start summon inside [`crate::engine::fire_before_combat_start`].
/// `BYRDPIP` and `PAELS_LEGION` followed (#2827): their combat-start pets are
/// a projection of ownership, and `engine::passive_relic_pets_before_combat_start`
/// marks the hook at the oracle's position. `RING_OF_THE_SNAKE` followed
/// (#2827): its only hook is `ModifyHandDraw`, already folded in
/// `engine::relics::modifier_total`; this retired a measured over-refusal.
/// `GIRYA` and `PETRIFIED_TOAD` followed on 2026-09-23 (#2827): Girya's lifts
/// join the room-entry Strength block ([`apply_room_entry_strength`]), and
/// Petrified Toad's Shaped Rock procurement is the `BeforeCombatStartLate`
/// pass of [`crate::engine::fire_before_combat_start`].
/// `LETTER_OPENER` and `PAELS_FLESH` followed (#2827): their window and turn-1
/// bodies are inert in IL on turn one (a counter reset the fresh state already
/// holds, and display calls), and their gameplay bodies were already in
/// `engine::relics`.
/// `ETERNAL_FEATHER` and `GHOST_SEED` followed the same day (#2827): the
/// Feather's heal is rest-site only in IL, and Ghost Seed's Ethereal pass now
/// runs at the end of [`crate::engine::fire_after_room_entered`].
/// `POLLINOUS_CORE` and `VAMBRACE` followed (#2827):
/// Pollinous Core's `BeforeHandDraw`/`ModifyHandDraw` body was already
/// `engine::relics::before_hand_draw`, and what kept it gated was the opening's
/// own seed modulus; Vambrace's `BeforeCombatStart` reset is now
/// `engine::vambrace_before_combat_start`. See the section above
/// [`OPENING_WINDOW_RELIC_BODIES`].
/// `KUSARIGAMA` and `PHYLACTERY_UNBOUND` followed the same day (#2827):
/// Kusarigama's `BeforeCombatStart` is a counter reset the opening already
/// seeds, and Phylactery Unbound's combat-start Osty now runs beside Bound
/// Phylactery's in `engine::phylactery_before_combat_start`.
/// `STONE_CRACKER` followed (#2827): its one window body, the
/// `AfterRoomEntered` upgrade of two draw-pile cards, runs in
/// [`build_pre_hook`] at the oracle's position (after the shuffle, before
/// `make_monsters`), where `stone_cracker_upgrades` carries the IL. It has no
/// engine subscriber, so the hook does not also fire it a second time.
/// `SLING_OF_COURAGE` and `TOOLBOX` followed on 2026-09-24 (#2827 item B, the
/// Coach's roots): Sling's elite-room Strength joins the room-entry Strength
/// block ([`apply_room_entry_strength`], [`SLING_OF_COURAGE_STRENGTH`]), and
/// Toolbox's turn-1 `BeforeHandDraw` choice was already
/// `engine::relics::before_hand_draw`, reached through
/// [`crate::engine::deal_opening_hand`]; [`toolbox_provenance_is_recorded`]
/// carries the oracle's gates for it.
/// `MEAT_ON_THE_BONE` and `POCKETWATCH` followed the same day (#2827 item B):
/// both window bodies are inert in IL on turn one (see the section above
/// [`OPENING_WINDOW_RELIC_BODIES`]), and their in-fight bodies are admission's.
/// `UNSETTLING_LAMP` followed: its window body is the armed-state reset that
/// [`PER_FIGHT_RELIC_STATE_SEEDED`] now seeds.
/// `BLESSED_ANTLER` followed on 2026-09-25 (#2992): its turn-1
/// `BeforeHandDraw` Dazed body was already `engine::relics`', reached through
/// [`crate::engine::deal_opening_hand`]; its same-hook acquisition-order gate
/// is [`refuse_unordered_blessed_antler_peers`].
/// `JEWELED_MASK`, `FUNERARY_MASK`, `RADIANT_PEARL`, `BIG_MUSHROOM` and
/// `TEA_OF_DISCOURTESY` followed on 2026-09-26 (#3162); see the section above
/// [`OPENING_WINDOW_RELIC_BODIES`].
#[cfg(test)]
const OPENING_WINDOW_GATE_EXCLUSIONS: [&str; 38] = [
    "RELIC.ANCHOR",
    "RELIC.BAG_OF_PREPARATION",
    "RELIC.BIG_MUSHROOM",
    "RELIC.BLESSED_ANTLER",
    "RELIC.BOOMING_CONCH",
    "RELIC.BOUND_PHYLACTERY",
    "RELIC.BRONZE_SCALES",
    "RELIC.BYRDPIP",
    "RELIC.EMBER_TEA",
    "RELIC.ETERNAL_FEATHER",
    "RELIC.FUNERARY_MASK",
    "RELIC.GHOST_SEED",
    "RELIC.GIRYA",
    "RELIC.GORGET",
    "RELIC.JEWELED_MASK",
    "RELIC.KUSARIGAMA",
    "RELIC.LETTER_OPENER",
    "RELIC.MEAL_TICKET",
    "RELIC.MEAT_ON_THE_BONE",
    "RELIC.ODDLY_SMOOTH_STONE",
    "RELIC.PAELS_FLESH",
    "RELIC.PAELS_LEGION",
    "RELIC.PANTOGRAPH",
    "RELIC.PENDULUM",
    "RELIC.PETRIFIED_TOAD",
    "RELIC.PHYLACTERY_UNBOUND",
    "RELIC.PLANISPHERE",
    "RELIC.POCKETWATCH",
    "RELIC.POLLINOUS_CORE",
    "RELIC.RADIANT_PEARL",
    "RELIC.RING_OF_THE_SNAKE",
    "RELIC.SLING_OF_COURAGE",
    "RELIC.STONE_CRACKER",
    "RELIC.TEA_OF_DISCOURTESY",
    "RELIC.TOOLBOX",
    "RELIC.UNSETTLING_LAMP",
    "RELIC.VAJRA",
    "RELIC.VAMBRACE",
];

/// Relics with a **turn-1** body the opening runs and this crate has no
/// subscriber for (#2731).
///
/// # The window is eight hooks, not five
///
/// `combat_sim.start_combat` does not stop at the deal: its last two statements
/// are `begin_player_turn(s)` and `_normalize_card_identities(s)`
/// (frozen Python, deleted #2827), and the document it returns *is* the
/// opening's output. So the player's **first** `BeforeSideTurnStart`,
/// `AfterSideTurnStart` and `AfterPlayerTurnStart` walks are inside the
/// opening window exactly as the five hooks in
/// [`OPENING_WINDOW_RELIC_BODIES`] are, and [`build`] enters them through
/// [`crate::engine::deal_opening_hand`]. The `.mcr` opening checksum agrees:
/// its context is `"After player turn start"`
/// ([`refusal::OpeningRefusal::McrChecksumContext`]).
///
/// **Derived, not curated**, the same way and from the same manifest:
///
/// ```text
/// python3.12 -c "import combat_sim as s
/// T = {'BeforeSideTurnStart', 'AfterSideTurnStart', 'AfterPlayerTurnStart'}
/// print([r for r in sorted(s.KNOWN_RELICS)
///        if {h.split('[')[0] for h in s._current_relic_hooks(r)} & T
///        and r not in s.TEMPLATE_RELICS])"
/// ```
///
/// That prints **53** relics at v0.111.0 (2026-09-21). **42** of them have a
/// hand-authored Rust body — `engine::relics::before_side_turn_start`,
/// `…::after_side_turn_start`, `…::after_side_turn_start_late` and
/// `…::after_player_turn_start` are where most of them live — so firing the
/// hook reaches real code and nothing is silently dropped. The other **11**
/// have no body site anywhere in `src/engine`, `src/steps`, `src/entry` or
/// `src/hooks.rs`. That split is the `rust_body_site` field of
/// `fixtures/opening_relic_hooks_v1.json`, derived mechanically by
/// `tools/gen_opening_relic_pins.py` (deleted #2999; the manifest is frozen
/// data) rather than read off this comment.
///
/// `engine::admission` is excluded from that search on purpose, and it is why
/// the gap survived review: **all eleven** are members of
/// `admission::IMPLEMENTED_RELICS`, documented as *"relics with complete modeled
/// bodies"*, and **eight of the eleven** are named again across admission's
/// same-hook structures — Crossbow's `AfterSideTurnStart` acquisition-order
/// peer list and Big Hat's same-hook list (`admission.rs` 8396-8433 and
/// 8454-8495: `AKABEKO`, `FENCING_MANUAL`, `LANTERN`, `RUNIC_CAPACITOR`,
/// `SYMBIOTIC_VIRUS`), Bread's active-turn-start-energy-peer gate (8783-8793:
/// `LANTERN`), the `turn_start_damage_relics` list with Bone Tea's and Vexing
/// Puzzlebox's same-hook lists (8547-8562, 8679-8697, 8713-8728:
/// `FESTIVE_POPPER`, `BELLOWS`), and the orb co-ownership refusals (8766-8836:
/// `CRACKED_CORE`, `FENCING_MANUAL`, `RUNIC_CAPACITOR`, `SYMBIOTIC_VIRUS`).
/// Only `BAG_OF_MARBLES`, `RED_MASK` and `TWISTED_FUNNEL` appear nowhere in
/// `admission.rs` but the bare `IMPLEMENTED_RELICS` entry. None of that runs an
/// effect — `admission.rs` validates and refuses, it is not a subscriber.
///
/// Of the **three** that remain, one is already refused by
/// [`TURN_ONE_ORB_RELICS`] (`RUNIC_CAPACITOR`), one is
/// `TURN_ONE_GATE_EXCLUSIONS`, and the remaining **one** is this table.
/// (`CRACKED_CORE` made it eight until its engine body landed, #2827,
/// `SYMBIOTIC_VIRUS` made it seven until its own did, also #2827,
/// `BELLOWS` and `FESTIVE_POPPER` made it six until theirs did, #2827, and
/// `FENCING_MANUAL` made it four until its own did, #3090.)
///
/// # The three bodies this slice modeled (#2693)
///
/// `RED_MASK`, `BAG_OF_MARBLES` and `LANTERN` left the table on 2026-09-22,
/// each with a parity witness in `fixture_tests` against Python's document for
/// the same save, which is why the unsubscribed count above became **8** and
/// not 11; `CRACKED_CORE`'s engine body (#2827) took it to **7**,
/// `SYMBIOTIC_VIRUS`' to **6**, `BELLOWS`' and `FESTIVE_POPPER`'s (#2827,
/// below) to **4**, and `FENCING_MANUAL`'s (#3090, above) to **3**. Their bodies are
/// `engine::relics::turn_one_all_enemy_debuffs` and the
/// `RELIC.LANTERN` row of `engine::relics::after_side_turn_start`, where the IL
/// and oracle citations live; they are in the **engine** and not in
/// [`pre_hook_document`] precisely so that a Rust-opened document and a
/// post-opening one run the same code. The all-enemy debuffs additionally
/// brought the oracle's `partial_artifact` gate across as
/// [`OpeningRefusal::TurnOneDebuffPartialArtifact`] — see
/// [`TURN_ONE_ALL_ENEMY_DEBUFF_RELICS`].
///
/// `TWISTED_FUNNEL` **stays gated** although it shares the debuff loop: its
/// amount picks up `RELIC.SNECKO_SKULL` and a fresh Poison application
/// allocates an instance identity, which the oracle's opening call — made with
/// `state=None`, so `m.poison_uid = -1` (frozen Python `_record_misery_debuff_application`, deleted #2827) — does
/// not. That is a different shape, so it owes its own derivation and witness
/// rather than a ride on this one.
///
/// # Every entry's body is turn-1-only, which is why the opening is the whole
/// # exposure
///
/// This matters for scope, not just for tidiness: a relic whose body fires on
/// *every* side turn would also be missing from a post-opening root, and
/// walling off the opening would hide half the defect. Each entry is
/// gated to the first player turn in the oracle **and in IL**, re-read
/// 2026-09-21 and again 2026-09-22 on
/// `solver/dll-archive/v0.111.0/data_sts2_macos_arm64/sts2.dll`
/// (sha256 `9cb4f1ad…`). The all-enemy debuffs share one shape —
/// `Contains<Creature>(participants, Owner.Creature)`, then
/// `PlayerCombatState::get_TurnNumber; ldc.i4.1; ble` which `leave`s the body
/// when the turn is past one:
///
/// * `RELIC.TWISTED_FUNNEL` — Poison 4 (`TwistedFunnel::get_CanonicalVars`
///   `0x9d2b2` is `ldc.i4.4`).
///   `TwistedFunnel/<BeforeSideTurnStart>d__6::MoveNext` (`0x3336e8`), guard at
///   `IL_004f-IL_0055`, `Apply<PoisonPower>` at `IL_0192`. Oracle:
///   frozen Python `start_combat`, deleted #2827, where the amount also picks up
///   `RELIC.SNECKO_SKULL`.
///
/// # `RELIC.FENCING_MANUAL` (#3090)
///
/// A separate mechanic that shares the guard: a turn-1 `ForgeCmd::Forge(10)`.
/// It left the table on 2026-09-25. Its body is
/// `engine::relics::fencing_manual_after_side_turn_start`, in the late
/// `AfterSideTurnStart` group at the oracle's position, where the IL citations
/// live; the catalog interns the Blade it generates (`boundary.rs`, beside
/// Bellows). Retiring it brought across the same-hook co-ownerships whose
/// order around the Forge is observable, as
/// [`fencing_manual_turn_start_peers_are_ordered`].
///
/// # The two `AfterPlayerTurnStart` bodies #2827 modeled
///
/// `RELIC.BELLOWS` (upgrade the turn-one Hand once) and `RELIC.FESTIVE_POPPER`
/// (9 unpowered damage to every hittable enemy on turn one) left the table on
/// 2026-09-23. Their bodies are `engine::relics::bellows_after_player_turn_start`
/// and `…::festive_popper_after_player_turn_start`, placed in
/// `continue_after_toasty_mittens` where the oracle's
/// `_continue_after_toasty_mittens` runs them (frozen Python, deleted #2827);
/// the IL citations live there. Retiring the entries also brought across the
/// oracle's same-hook co-ownership refusals for the two, as
/// [`OpeningRefusal::TurnStartRelicOrderUnrecorded`] (see
/// [`after_player_turn_start_peers_are_ordered`]). Bellows also makes the
/// catalog intern the card-upgrade closure (`boundary.rs`, beside Razor Tooth),
/// because the opening deals the Hand it upgrades.
///
/// Because every one is turn-1-only, a document rooted from a `.mcr` capture
/// mid-fight is unaffected: the body cannot fire again, so the state it
/// produced is already in the capture. That is why the *gate* was an opening
/// change — while retiring an entry is an engine change, because the body has
/// to run at the native fire point for a Rust-opened and a post-opening
/// document to agree.
const TURN_ONE_RELIC_BODIES: [&str; 1] = ["RELIC.TWISTED_FUNNEL"];

/// The oracle's `_TURN_ONE_ALL_ENEMY_DEBUFF_RELICS` (frozen Python, deleted #2827),
/// carried whole because the `partial_artifact` comparison is against its
/// **size**.
///
/// frozen Python `start_combat` (deleted #2827) computes `relics & _TURN_ONE_ALL_ENEMY_DEBUFF_RELICS`
/// and refuses when any created monster has
/// `0 < m.artifact < len(that intersection)`. Two of the three now have Rust
/// bodies (`engine::relics::turn_one_all_enemy_debuffs`) and
/// `RELIC.TWISTED_FUNNEL` is still in [`TURN_ONE_RELIC_BODIES`], so a fight
/// holding it refuses above this check and the intersection this port ever
/// measures is exactly the oracle's — the set is spelled in full anyway, so
/// retiring Funnel cannot silently shrink the count the refusal compares.
const TURN_ONE_ALL_ENEMY_DEBUFF_RELICS: [&str; 3] = [
    "RELIC.BAG_OF_MARBLES",
    "RELIC.RED_MASK",
    "RELIC.TWISTED_FUNNEL",
];

/// The one turn-1-hook relic with neither a Rust body nor a gate entry, because
/// it needs neither.
///
/// `RELIC.AKABEKO`'s manifest hook is `AfterSideTurnStart`, but the oracle does
/// not run a body for it: `start_combat` writes
/// `vigor=AKABEKO_VIGOR if "RELIC.AKABEKO" in relics else 0` straight into the
/// constructed `State` (frozen Python, deleted #2827), above `begin_player_turn`. The
/// opening writes the same field from the same constant
/// ([`pre_hook_document`]'s `AKABEKO_VIGOR` row), so the document already
/// agrees and there is nothing to refuse. Same category as the four
/// state-seeding exclusions in `OPENING_WINDOW_GATE_EXCLUSIONS`, and
/// `cfg(test)` for the same reason: the pin is its only reader.
#[cfg(test)]
const TURN_ONE_GATE_EXCLUSIONS: [&str; 1] = ["RELIC.AKABEKO"];

/// The per-fight relic flags `start_combat` seeds from bare ownership, and
/// that [`pre_hook_document`] writes (#2756).
///
/// # Not a hook gap — a seeding gap, and it was invisible by construction
///
/// None of these is in [`OPENING_WINDOW_RELIC_BODIES`] or
/// [`TURN_ONE_RELIC_BODIES`] and none could have been: their declared hooks are
/// `ModifyCardPlayCount`, `AfterCardPlayed`, `AfterHandEmptied` and
/// `AfterDamageReceived`, so no derivation over the eight window hooks can see
/// them. What they share is that `start_combat` writes a **per-fight** `State`
/// field from bare relic ownership, and that field lives in Rust's hot state
/// rather than in the catalog's immutable relic mirrors
/// (`boundary::HAND_RELIC_SCALAR_FIELDS`), so the opening has to write it. Each
/// has a working Rust body that reads the flag, so leaving it at its `State`
/// default made the relic **inert for the whole fight** while the projected
/// inventory said it was owned — a well-formed, admissible, wrong document,
/// which is the one outcome I5 exists to forbid.
///
/// # Seeded fresh, not copied from the save
///
/// Every one is a **constant** at combat start, established from the DLL rather
/// than assumed, which is why this table carries no save reader. `start_combat`
/// writes each as `"RELIC.X" in relics` and the native per-combat field is
/// always `false` when a combat begins:
///
/// * `RELIC.THROWING_AXE` → `throwing_axe_available` (`start_combat`, frozen Python, deleted #2827).
///   Native carries `ThrowingAxe::_usedThisCombat`, and
///   `ThrowingAxe::AfterRoomEntered` (RVA `0x9c79c`) is
///   `isinst CombatRoom; brtrue.s` at `IL_0002`-`IL_0007` and then
///   `ldc.i4.0; set_UsedThisCombat` at `IL_0010`-`IL_0011` — the flag is
///   **reset on entering every combat room**, and `AfterCombatEnd` (`0x9c7f9`)
///   clears it again. `set_UsedThisCombat(1)` happens only in
///   `AfterModifyingCardPlayCount` (`0x9c7de` `IL_0002`-`IL_0003`), inside the
///   fight. So the oracle's *available* is native's `!UsedThisCombat`, which is
///   `true` for an owner at every combat start. `engine::play` (`:6753`,
///   `:7347`) and `engine::draw` (`:2202`, `:2408`) read and spend it.
///   **This is the one the corpus census measured diverging.**
/// * `RELIC.PERMAFROST` → `permafrost`. Same shape one accessor over:
///   `Permafrost::AfterRoomEntered` (`0x9931c`) resets
///   `_activatedThisCombat` at `IL_0010`-`IL_0011` behind the same
///   `isinst CombatRoom` test, and the only writer of `true` is
///   `Permafrost/<AfterCardPlayed>d__9::MoveNext` (`0x32dbc4`) at
///   `IL_00ea`-`IL_00eb`, *after* its awaited `CreatureCmd::GainBlock`.
///   `engine::relics` (`:2536`) spends it on the first Power card played.
/// * `RELIC.CENTENNIAL_PUZZLE` → `puzzle`. `_usedThisCombat` again,
///   but with **no** `AfterRoomEntered` arm: the only reset is
///   `CentennialPuzzle::AfterCombatEnd` (`0x92057` `IL_0002`-`IL_0003`), and
///   the only writer of `true` is inside
///   `<AfterDamageReceived>d__10::MoveNext` (`0x321758`). A relic constructed
///   mid-run starts at the field's default, so the value a combat begins with
///   is `false` on both paths. `engine::damage` (`:4448`, `:5214`) spends it.
/// * `RELIC.UNCEASING_TOP` → `unceasing_top`. This one carries **no
///   native per-combat field at all** — `UnceasingTop` declares no instance
///   field, and `<AfterHandEmptied>d__4::MoveNext` (`0x333994`) is the owner
///   test at `IL_001e`-`IL_002b`, `IsValidPhase` (`0x9d3a0`) at
///   `IL_003b`-`IL_0047` and `CardPileCmd::Draw` at `IL_005e`. So the oracle's
///   field is a pure ownership mirror that happens to live in Rust's hot state
///   instead of the catalog's; `engine::admission` (`:8851`) already refuses a
///   document whose flag disagrees with the inventory, which is why this one
///   was an over-refusal rather than a silent divergence.
/// * `RELIC.UNSETTLING_LAMP` → `lamp` (#2827 item B). The oracle's
///   `lamp` is native's armed state, `TriggeringCard == null &&
///   !IsFinishedTriggering`, and `UnsettlingLamp::BeforeCombatStart` (RVA
///   `0x9d4d3`) establishes exactly that at every combat start:
///   `set_TriggeringCard(null)` at `IL_0001`-`IL_0003`, `DoubledPowers.Clear()`
///   at `IL_0008`-`IL_000e`, `set_IsFinishedTriggering(false)` at
///   `IL_0013`-`IL_0015`, then `Status` (presentation). Nothing in the
///   opening can consume it: `BeforePowerAmountChanged` (`0x9d4fc`) arms the
///   doubling only for a power applied **by a card** (`cardSource` non-null at
///   `IL_0028`-`IL_002a`), so the turn-1 relic debuffs pass it by. The
///   relic left [`OPENING_WINDOW_RELIC_BODIES`] for this row: the gate held a
///   relic whose only window effect is this seed.
///
/// # How they were found, so the sweep can be re-run
///
/// By measurement, not by reading. Retiring the three room-entry bodies of
/// #2755 let two corpus fights past the gate for the first time, and both came
/// back digest-divergent from Python on exactly
/// `player.throwing_axe_available`. The sweep that generalised it adds **one
/// relic at a time** to the fixture save, roots it through
/// `mcr_replay.start_from_save`, and diffs Python's projected `player` map
/// against `sts-sim entry --opening`'s: every relic that changes a field in
/// Python and not in Rust is either a gate entry (most of them) or a gap.
/// Re-run it before widening any gate table here.
///
/// # Where the other two of the sweep's six went
///
/// The sweep found six and this table holds four, so a reader of these two
/// tables should not have to guess at the rest.
///
/// * `RELIC.BLOOD_VIAL` is **not** here because its flag was never the gap: the
///   `start_combat` write (frozen Python, deleted #2827) is a bare ownership mirror, which
///   `boundary.rs` already projects from the catalog, so the opening always
///   emitted it. What was missing is the `hp` its heal writes, and a heal is a
///   body rather than a seed — it lives in
///   [`crate::engine::relics::after_player_turn_start_late`], which the
///   opening reaches through `engine::deal_opening_hand`, so the opening and a
///   post-opening document run the same code.
/// * `RELIC.DRAGON_FRUIT` is not here either (#3320): the relic declares no
///   instance field, so the oracle's `dragon_fruit` is an ownership mirror,
///   which `boundary.rs` now projects from the catalog like `blood_vial`.
///   See [`PER_FIGHT_RELIC_STATE_UNSEEDED`] for what retired it.
const PER_FIGHT_RELIC_STATE_SEEDED: [(&str, &str); 5] = [
    ("RELIC.CENTENNIAL_PUZZLE", "puzzle"),
    ("RELIC.PERMAFROST", "permafrost"),
    ("RELIC.THROWING_AXE", "throwing_axe_available"),
    ("RELIC.UNCEASING_TOP", "unceasing_top"),
    ("RELIC.UNSETTLING_LAMP", "lamp"),
];

/// Per-fight relic fields `start_combat` seeds that this crate cannot
/// represent (#2756). **Empty since #3320**; kept, with its gate and its
/// census class, so a relic the #2779 field derivation finds can be refused by
/// name the day it is found.
///
/// Its last member was `RELIC.DRAGON_FRUIT` → `dragon_fruit`, gated because
/// the relic had no body. (The gate was the only thing stopping it: the relic
/// is in the generated `content_tables::CENSUS_INERT_RELICS`, which admission
/// accepts, so a Dragon Fruit root past this gate would have admitted and a
/// lethal Hand of Greed would have skipped its +1 max HP silently.) #3320
/// read the IL (v0.111.0, sha256
/// `9cb4f1ad…`): `DragonFruit` declares `get_Rarity` (`0x92b13`), `IsAllowed`
/// (`0x92b16`), `get_CanonicalVars` (`0x92b1e`), `AfterGoldGained`
/// (`0x92b30`) and `.ctor` (`0x92b7b`, `RelicModel::.ctor` only) — **no
/// instance field**, so there is nothing per-fight to seed, fresh or from the
/// save. The oracle's `dragon_fruit` is a pure ownership mirror, now in
/// `boundary::HAND_RELIC_SCALAR_FIELDS`, which [`pre_hook_document`] re-emits
/// through `boundary::relic_derived_player_scalars`. The body is
/// `engine::relics::dragon_fruit_after_gold_gained`, where the IL citation
/// lives. It stays census-inert rather than joining
/// `admission::IMPLEMENTED_RELICS`, as Bowler Hat does: the two registries
/// must stay disjoint (`build_coverage_badge.py`), and the inert one is
/// generated. Its one opening window reach, Maw Bank's room-entry gold, runs
/// in [`maw_bank_after_room_entered`] since #3328.
const PER_FIGHT_RELIC_STATE_UNSEEDED: [(&str, &str); 0] = [];

/// `MawBank`'s room-entry gold: `get_CanonicalVars` (RVA `0x9692e`) is
/// `ldc.i4.s 12; newobj GoldVar::.ctor` at `IL_0001`-`IL_0003`.
const MAW_BANK_ROOM_ENTRY_GOLD: i32 = 12;

/// Whether `MawBank`'s `AfterRoomEntered` gains its gold on entering this
/// combat room (#3328).
///
/// v0.111.0 `sts2.dll` (sha256 `9cb4f1ad…fbf12b4`). The body is
/// `MawBank/<AfterRoomEntered>d__12::MoveNext` (RVA `0x32a534`):
///
/// * `Owner.RunState.BaseRoom == room`, else `leave` (`IL_001d`-`IL_0035`).
///   `RunState::get_BaseRoom` (`0x4e2a8`) is `_currentRooms.FirstOrDefault()`,
///   the bottom of the room stack. A map combat is the only room on that
///   stack, so the test passes for every root this opening builds. The one
///   nested case, an event that starts a fight, is the event path's: the
///   `EventRoom` is the base there, and `entry::event_combat` refuses any
///   Maw Bank owner before an opening is built (`ROOM_ENTRY_LISTENERS`).
/// * `!HasItemBeenBought`, else `leave` (`IL_003b`-`IL_0042`). The flag is
///   the relic's one instance field, saved as the `bools` property
///   `HasItemBeenBought` (see `entry::relics`), and its only writer of `true`
///   is `AfterItemPurchased` (`0x969bb`), which is a shop hook. The save the
///   opening reads is written on reaching the point, before the room's
///   `AfterRoomEntered` (the Planisphere witness,
///   [`planisphere_after_room_entered`]), and no purchase can happen between
///   the two. So the saved value is the value the body reads.
/// * `Flash` (presentation), then `PlayerCmd::GainGold(DynamicVars.Gold
///   .BaseValue, Owner, false)` at `IL_0047`-`IL_0064`, awaited.
///
/// A missing or inexact saved flag refuses as
/// [`OpeningRefusal::RelicCounterNotExact`] rather than guessing (a `.run`
/// facts root never carries it).
///
/// `GainGold`'s one reach into combat state is Dragon Fruit's
/// `AfterGoldGained` (+1 max HP, which heals 1), and the only room-entry peer
/// that reads the player's HP against max HP is Red Skull: its
/// threshold is taken in unrecorded acquisition order against Maw Bank's heal,
/// and its latch would see a heal it has not run yet. An armed Maw Bank with
/// both Dragon Fruit and Red Skull therefore refuses by Maw Bank's name
/// ([`OpeningRefusal::RoomEntryRelicNotModeled`]), the refusal #3320 gave the
/// whole Dragon Fruit pair.
fn maw_bank_room_entry_armed(
    entry: &EntryDocument,
    relics: &BTreeSet<&str>,
) -> Result<bool, OpeningRefusal> {
    const RELIC: &str = "RELIC.MAW_BANK";
    if !relics.contains(RELIC) {
        return Ok(false);
    }
    let raw = entry.relic_entry.relic_counters.get(RELIC);
    let bought =
        raw.and_then(Value::as_bool)
            .ok_or_else(|| OpeningRefusal::RelicCounterNotExact {
                relic: RELIC,
                value: raw.map_or_else(|| "absent".to_string(), ToString::to_string),
            })?;
    if !bought && relics.contains("RELIC.DRAGON_FRUIT") && relics.contains("RELIC.RED_SKULL") {
        return Err(OpeningRefusal::RoomEntryRelicNotModeled {
            relic: RELIC.to_string(),
        });
    }
    Ok(!bought)
}

/// `MawBank`'s room-entry `GainGold` ([`maw_bank_room_entry_armed`], #3328).
///
/// `PlayerCmd::GainGold` is `engine::relics::gain_fatal_reward_gold`, whose
/// doc carries the IL: the `ModifyGoldGained` walk (Ectoplasm zeroes it,
/// Bowler Hat scales it by 1.25 — the only two overrides in the assembly),
/// the `amount > 0` test, the add, and `Hook::AfterGoldGained`, whose one
/// listener is Dragon Fruit (+1 max HP, heal 1).
///
/// Position: after the room-entry Strength block, Planisphere and Red Skull,
/// and before [`crate::engine::fire_after_room_entered`]. Native runs the
/// `AfterRoomEntered` listeners in acquisition order, but no represented
/// peer reads what this writes. The Strength block, the compiled templates
/// (Data Disk's Focus, Divine Right's stars, Sword of Jade's Strength), Stone
/// Cracker and Ghost Seed never read gold or HP. Planisphere's heal commutes
/// with Dragon Fruit's `+1/+1`: `min(M + 1, min(M, h + 5) + 1)` and
/// `min(M + 1, min(M + 1, h + 1) + 5)` are both `min(M + 1, h + 6)`. Red
/// Skull beside Dragon Fruit is refused.
fn maw_bank_after_room_entered(
    catalog: &crate::catalog::Catalog,
    state: &mut crate::hot::HotState,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), OpeningRefusal> {
    crate::engine::relics::gain_fatal_reward_gold(
        state,
        catalog,
        MAW_BANK_ROOM_ENTRY_GOLD,
        events,
    )?;
    crate::coverage::record_relic(crate::ids::RelicId::RelicMawBank);
    Ok(())
}

/// Orb relics whose turn-1 channel the opening does not reproduce.
///
/// `CRACKED_CORE` and `INFUSED_CORE` channel a Lightning orb during the first
/// turn-start walk, and `RUNIC_CAPACITOR` changes the queue's capacity around
/// that channel (the table first held three more, below). The engine's bodies for these are
/// wired to a turn walk entered from an already-opened document, and an
/// opening-built state reaches them differently: measured 2026-09-16, every
/// corpus fight holding `CRACKED_CORE` produced a document missing the
/// `[['LIGHTNING', null]]` queue the oracle projects.
///
/// `CRACKED_CORE` left this table on 2026-09-22 (#2827): its body now lives in
/// `engine::relics::cracked_core_before_side_turn_start`, where the IL and
/// oracle citations are, and the opening reaches it through
/// [`crate::engine::deal_opening_hand`] exactly as it reaches every other
/// turn-1 relic body. The parity witness is in `fixture_tests`.
///
/// `SYMBIOTIC_VIRUS` left on 2026-09-23 (#2827) the same way: its turn-1
/// `Channel<DarkOrb>` is `engine::relics::symbiotic_virus_after_side_turn_start`,
/// in the late `AfterSideTurnStart` group at the oracle's position, with the IL
/// and oracle citations there. Its order-observable co-ownerships (the oracle's
/// `start_combat` refusals) refuse by name here, in
/// [`symbiotic_virus_turn_start_order_is_recorded`], because the opening does
/// not run `engine::admission`, which carries the same four for a loaded
/// document. An undetermined character still refuses in
/// [`ORB_SLOT_SENSITIVE_RELICS`].
///
/// Kept separate from [`OPENING_WINDOW_RELIC_BODIES`] because the reason
/// differs: these relics *are* modeled, and what is unproven is that the
/// opening reaches their turn-1 bodies the way the oracle does. Distinct from
/// [`TURN_ONE_RELIC_BODIES`] for the same kind of reason — there the body does
/// not exist at all.
///
/// `GOLD_PLATED_CABLES` left on 2026-09-27 (#3381). It has no body on any hook
/// the opening fires: its type (v0.111.0 `sts2.dll` SHA-256 `9cb4f1ad…12b4`)
/// overrides only `ModifyOrbPassiveTriggerCounts` (RVA `0x94976`: `+1` when
/// the orb's owner is the relic's owner, IL_0001-IL_000d, and the orb is
/// `OrbQueue.Orbs[0]`, IL_0011-IL_002d) and
/// `AfterModifyingOrbPassiveTriggerCount` (`0x949ab`, `Flash` only). The one
/// reader of that modifier is `OrbModel/<TriggerPassive>d__74::MoveNext`
/// (`0x31da6c` IL_0045), and `TriggerPassive`'s callers are the orbs'
/// `BeforeTurnEndOrbTrigger` bodies (turn end, never inside the opening),
/// `OrbCmd/<Passive>d__7` (Emotion Chip and other in-turn commands, reached
/// through `engine::orbs::passive_at_affected_by_hooks`), and
/// `PlasmaOrb::AfterTurnStartOrbTrigger` (`0xae60c`), the only
/// `AfterTurnStartOrbTrigger` override, which `engine::orbs::turn_start_passives`
/// runs with the Cables' front-Plasma bonus. The opening reaches that
/// function through [`crate::engine::deal_opening_hand`] exactly as every
/// later turn does, so the relic's every reachable effect is the engine's
/// existing body.
const TURN_ONE_ORB_RELICS: [&str; 2] = ["RELIC.INFUSED_CORE", "RELIC.RUNIC_CAPACITOR"];

/// Relics carrying a per-combat persistent counter the opening seeds from the
/// run, with the modulus `start_combat` applies (frozen Python, deleted #2827).
const SEEDED_COUNTER_RELICS: [(&str, i64); 7] = [
    // FLOWER_TURNS / FAKE_FLOWER_TURNS / PENDULUM_TURNS / POLLINOUS_CORE_TURNS
    // / NUNCHAKU_ATTACKS / PEN_NIB_ATTACKS, and Galactic Dust's literal 10.
    // Every modulus below was re-audited against the v0.111.0 IL in #3228
    // (the Iron Club seed below the table read 3 against a native 4).
    //
    // `HappyFlower::get_CanonicalVars` (RVA `0x94b97`) builds
    // `DynamicVar("Turns", 3)` at `IL_0012`-`IL_0018`;
    // `HappyFlower/<AfterSideTurnStart>d__20::MoveNext` (RVA `0x326434`)
    // stores `(TurnsSeen + 1) % Turns` at `IL_003d`-`IL_005c`, so the saved
    // value is already in `0..3`.
    ("RELIC.HAPPY_FLOWER", 3),
    // `FAKE_FLOWER_TURNS` is 5 (frozen Python, deleted #2827; seeded in `start_combat`):
    // `FakeHappyFlower::get_CanonicalVars` (RVA `0x932f3`) builds
    // `DynamicVar("Turns", 5)` at `IL_0012`-`IL_001d`, the same period the
    // engine's turn-start row uses (`engine/relics.rs`, `fake_flower() <= 4`
    // in `hot.rs`). This read 2 until #2906, which opened a saved `TurnsSeen`
    // of 2, 3 or 4 as 0, 1 or 0 and granted the energy on the wrong turn.
    ("RELIC.FAKE_HAPPY_FLOWER", 5),
    // `Pendulum::get_CanonicalVars` (RVA `0x98f3a`) builds
    // `DynamicVar("Turns", 3)` at `IL_0012`-`IL_0018`, and
    // `Pendulum::BeforeHandDraw` (RVA `0x98fa0`) stores
    // `(TurnsSeen + 1) % Turns` at `IL_001b`-`IL_003a`.
    ("RELIC.PENDULUM", 3),
    // `POLLINOUS_CORE_TURNS` is 4 (frozen Python, deleted #2827; and the oracle's
    // `seeded("RELIC.POLLINOUS_CORE", POLLINOUS_CORE_TURNS)` in `start_combat`):
    // `PollinousCore::get_CanonicalVars` (RVA `0x998d9`) builds
    // `DynamicVar("Turns", 4)` at `IL_0012`-`IL_001d`, and `ModifyHandDraw`
    // (`0x999cc`) grants once `TurnsSeen >= Turns` (`IL_0018`-`IL_0032`). A saved
    // `TurnsSeen` of 3 is the turn that grants, so it must survive the modulus.
    // This read 3 until #2827, which made a saved 3 open as 0 — unreachable
    // while the relic was gated, and the reason it had to be fixed to retire it.
    ("RELIC.POLLINOUS_CORE", 4),
    // `Nunchaku::get_CanonicalVars` (RVA `0x97807`) builds its `Cards` var
    // from `ldc.i4.s 10` at `IL_0009`; `Nunchaku/<AfterCardPlayed>d__19
    // ::MoveNext` (RVA `0x32b888`) increments the never-reset
    // `AttacksPlayed` (`IL_0055`-`IL_0060`) and grants when
    // `AttacksPlayed % Cards == 0` (`IL_0085`-`IL_008d`) — a lifetime count
    // like Iron Club's, so the residue modulo 10 is the combat phase.
    ("RELIC.NUNCHAKU", 10),
    // `PenNib` has no `Cards` var: `set_AttacksPlayed` (RVA `0x990ea`) stores
    // `value % 10` (`ldc.i4.s 10; rem` at `IL_0009`-`IL_000b`), and
    // `ModifyDamageMultiplicative` (`0x9917c`) doubles at `AttacksPlayed == 9`
    // (`IL_006b`-`IL_0073`).
    ("RELIC.PEN_NIB", 10),
    // `GalacticDust::get_CanonicalVars` (RVA `0x94357`) builds its `Stars`
    // var from `ldc.i4.s 10` at `IL_0009`; `GalacticDust/<AfterStarsSpent>
    // d__19::MoveNext` (RVA `0x325528`) adds the spend (`IL_0033`-`IL_0041`),
    // and after the block stores `StarsSpent % Stars` (`IL_0107`-`IL_011f`).
    ("RELIC.GALACTIC_DUST", 10),
];

/// `EmberTea`'s starting charge, and therefore the inclusive maximum of its
/// saved `CombatsLeft`.
///
/// `EmberTea::.ctor` (v0.111.0 RVA `0x92fdf`) is `ldc.i4.5; stfld
/// _combatsLeft`, and the only writer is `set_CombatsLeft` (`0x92f44`), which
/// the relic's own body only ever *decrements*. The oracle's
/// `exact_finite_counter("RELIC.EMBER_TEA", 5)` (frozen Python `_mad_science_entry_variant`, deleted #2827) is the
/// same domain.
const EMBER_TEA_MAX_COMBATS: i64 = 5;

/// The Strength `EmberTea` applies on entering a combat room.
///
/// `EmberTea::get_CanonicalVars` (`0x92ee9`) builds two `DynamicVar`s: the
/// `'Combats'` mirror of `CombatsLeft`, and a second one whose base value is
/// the literal `ldc.i4.2` at `IL_0021` — the `Strength` var the body reads.
/// That is `combat_sim.EMBER_TEA_STRENGTH` (frozen Python `_ally_interpose`, deleted #2827).
const EMBER_TEA_STRENGTH: i32 = 2;

/// The Strength `Vajra` applies on entering a combat room.
///
/// `Vajra::get_CanonicalVars` (`0x9d6ad`) wraps `System.Decimal::One` at
/// `IL_0001`, and the body applies it as `StrengthPower` at
/// `Vajra/<AfterRoomEntered>d__6::MoveNext` `IL_0062` (RVA `0x333abc`). That is
/// `combat_sim.VAJRA_STRENGTH`.
const VAJRA_STRENGTH: i32 = 1;

/// The Strength `SlingOfCourage` applies on entering an **elite** room.
///
/// `SlingOfCourage::get_CanonicalVars` (v0.111.0 RVA `0x9b8fb`) wraps the
/// literal `ldc.i4.2` at `IL_0001` in its one Strength var, and
/// `SlingOfCourage/<AfterRoomEntered>d__6::MoveNext` (RVA `0x331284`) leaves at
/// `IL_001d`-`IL_002b` unless `room.RoomType == 2` (`RoomType.Elite`: the enum
/// declares `Unassigned, Monster, Elite, Boss, ...`), then applies
/// `StrengthPower` of `DynamicVars.Strength.BaseValue` to the owner at
/// `IL_0030`-`IL_0063`. That is `combat_sim.SLING_OF_COURAGE_STRENGTH`
/// (frozen Python, deleted #2827), gated on `entry["node_type"] == "elite"` (`start_combat`).
const SLING_OF_COURAGE_STRENGTH: i32 = 2;

/// The inclusive maximum of `Girya`'s saved `TimesLifted`.
///
/// `Girya::TryModifyRestSiteOptions` (v0.111.0 RVA `0x9469b`) offers the lift
/// only while `get_TimesLifted` is `ldc.i4.3; blt.s` (`IL_000d`-`IL_0013`), and
/// the lift is the counter's only writer, so a saved value is in `0..=3`. That
/// is `combat_sim.GIRYA_MAX_LIFTS` and the domain `start_combat` validates
/// (frozen Python, deleted #2827).
const GIRYA_MAX_LIFTS: i64 = 3;

/// The HP and max HP of a passive relic pet.
///
/// `Byrdpip::get_MinInitialHp`/`get_MaxInitialHp` (`0xb08b8`/`0xb08bf`) and
/// `PaelsLegion::get_MinInitialHp`/`get_MaxInitialHp` (`0xbb33f`/`0xbb346`)
/// are each `ldc.i4 9999` at `IL_0001`. That is
/// `combat_sim.PASSIVE_RELIC_PET_HP`, and the value `boundary.rs` projects.
const PASSIVE_RELIC_PET_HP: i64 = 9999;

/// `SwordOfJade`'s room-entry Strength, carried only for the Ruined Helmet
/// distinct-amount guard.
///
/// The oracle lists it in `room_entry_strength_sources` and then explicitly
/// **skips** applying it in the room-entry loop (frozen Python `start_combat`, deleted #2827) because the
/// relic's own `AfterRoomEntered` template does it; its amount still decides
/// whether Ruined Helmet's doubling is observable.
/// `SWORD_OF_JADE_STRENGTH`, the relic's
/// `AfterRoomEntered` `StrengthPower` `CanonicalVar`.
const SWORD_OF_JADE_STRENGTH: i32 = 3;

/// The canonical player field each seeded counter projects to.
const SEEDED_COUNTER_FIELDS: [(&str, &str); 7] = [
    ("RELIC.HAPPY_FLOWER", "flower"),
    ("RELIC.FAKE_HAPPY_FLOWER", "fake_flower"),
    ("RELIC.PENDULUM", "pendulum"),
    ("RELIC.POLLINOUS_CORE", "pollinous_core"),
    ("RELIC.NUNCHAKU", "nunchaku"),
    ("RELIC.PEN_NIB", "pen_nib"),
    ("RELIC.GALACTIC_DUST", "galactic_dust"),
];

/// Deck cards whose generation draws from the **owner's own card pool**.
///
/// Exactly `combat_sim.generation_pool_sources` (frozen Python `start_combat`, deleted #2827): the seven
/// ids it intersects the deck with, plus `MAD_SCIENCE`. The oracle admits a
/// Mad Science copy only at variant `(2, 6)`; this port has no per-instance
/// variant reader, so it treats every copy as a source. That is a **narrowing**
/// — it can only refuse a fight the oracle roots, never root one the oracle
/// refuses — and it is the single deliberate difference in this table.
///
/// The gate these feed is the owner gate, and it is scoped exactly as the
/// oracle scopes it. The other generation source sets
/// (`owner_power_generation_sources`, `distraction_…`, `metamorphosis_…`,
/// `regent_colorless_…`, `before_hand_draw_pool_sources`, the generation
/// potions, the six `_GENERATION_RELIC_POOLS` relics) each have their **own**
/// gates in the oracle with their own conditions; folding them in here would
/// refuse fights the oracle roots for reasons this port has not derived. They
/// are unreached today because the roster refusal precedes them, and they are
/// named in the walk as the remaining owner-analysis work — except the
/// Regent's `spectrum_shift_generation_pool`, which is not a gate at all in
/// the oracle but a field it writes unconditionally for a Regent with a
/// recorded profile, and which [`pre_hook_document`] writes since #2736.
const OWNER_POOL_GENERATION_CARDS: [&str; 8] = [
    "CALAMITY",
    "DISCOVERY",
    "INFERNAL_BLADE",
    "JACKPOT",
    "JACK_OF_ALL_TRADES",
    "MAD_SCIENCE",
    "SPLASH",
    "STOKE",
];

/// The one of those whose body the engine models for an Ironclad owner only,
/// over a frozen fully-unlocked pool (#3336).
///
/// `InfernalBlade/<OnPlay>d__3::MoveNext` (v0.111.0 RVA `0x3a7164`) reads
/// `Owner.Character.CardPool` (IL_0027-IL_0031) through
/// `GetUnlockedCards(Owner.UnlockState, CardMultiplayerConstraint)`
/// (IL_0037-IL_0051), then shuffles on `CombatCardGeneration` (IL_008b): the
/// owner's pool, like every source here. The engine
/// (`steps::ironclad_uncommon::infernal_blade_exact`) shuffles the frozen
/// `INFERNAL_BLADE_ATTACK_POOL_V109`, which is that read for an Ironclad owner
/// whose profile reveals every Ironclad gating epoch. So both the owner and
/// the owner's epochs are gated here, and nothing else is.
///
/// The oracle's `ironclad_only` (frozen Python `start_combat`, deleted #2827)
/// also named `STOKE`. #3336 drops it: `Stoke/<OnPlay>d__1::MoveNext` (RVA
/// `0x3bef44`) reads `Owner.Character.CardPool` (IL_0194-IL_0199) and
/// `Owner.UnlockState` (IL_01a4) into `GetUnlockedCards` (IL_01b9) and
/// `GetForCombat` (IL_01d9), and the engine body
/// (`steps::ironclad_rare::stoke_exact`) draws
/// `Catalog::owner_generation_pool(owner)` for any owner under the recorded
/// profile, so a foreign Stoke is modeled, not unmodeled.
const IRONCLAD_ONLY_GENERATION_CARDS: [&str; 1] = ["INFERNAL_BLADE"];

/// The owner whose fully revealed card pool Infernal Blade's frozen pool is.
const INFERNAL_BLADE_FULL_POOL_OWNER: &str = "CHARACTER.IRONCLAD";

/// The one source the owner gate requires no recorded profile for
/// (`legacy_full_pool_sources = … - {"SPLASH"}`, frozen Python `start_combat`,
/// deleted #2827). The engine's own Splash gate refuses an unrecorded profile
/// by name, which is where the oracle left it.
const PARTIAL_UNLOCK_TOLERANT_GENERATION_CARDS: [&str; 1] = ["SPLASH"];

/// The characters whose owner-pool generation sources this opening admits.
///
/// Part C1 of the spec walk re-derived what this gate is actually about: the
/// character is recorded on 3,092/3,092 saves, and the missing thing was the
/// Necrobinder / Defect / Silent pools, which are DLL facts.
///
/// **The oracle's gate is wider.** #2542 extracted all five
/// `CharacterCardPool`s from the DLL and replaced the old Ironclad-or-Regent
/// gate with a bare recorded-owner requirement (`character not in
/// _BATCH172_CHARACTER_CARD_POOLS`, frozen Python `start_combat`, deleted #2827). This list is
/// narrower than that on purpose, and a narrowing can only refuse a fight the
/// oracle roots, never root one it refuses.
///
/// **Necrobinder (#2827).** Jackpot (`Jackpot/<OnPlay>d__3::MoveNext` RVA
/// `0x3a80d4`: `Owner.Character.CardPool` at IL_00eb,
/// `GetUnlockedCards(Owner.UnlockState, …)` at IL_010b, then
/// `CardFactory::GetForCombat` on `CombatCardGeneration` at IL_0159) and
/// Discovery (`Discovery/<OnPlay>d__4::MoveNext` RVA `0x399254`: the same
/// reads at IL_0043 / IL_0063, `GetDistinctForCombat` at IL_007e) read the
/// owner's unlocked pool at *play* time, from the card's `Owner`. Nothing about it is written into the combat-entry
/// document: the oracle's entry block (frozen Python `start_combat`, deleted #2827) only
/// decides whether the fight roots, and `entropy_card_pool` stays `None` for
/// every owner but Ironclad and Regent (`start_combat`). The pool rows the
/// engine derives at play time are the generated
/// `CHARACTER_CARD_POOL_ROWS_V1101` (`steps::neutral::derive_character_generation_pool`),
/// never a hand list. So admitting Necrobinder here changes no document
/// field, and the 24 corpus fights it opens match the oracle digest for
/// digest, except where the #2809 Plating round-one Block is the
/// IL-documented Python error. Solving those roots is a separate question:
/// the engine's Jackpot/Discovery provenance still requires an
/// `entropy_card_pool` the oracle never writes for a Necrobinder, so
/// `engine::admit` refuses them by name
/// (`an_opened_necrobinder_generator_deck_is_refused_by_solver_admission_by_name`; the remainder is #2946).
///
/// **Silent and Defect (#2827 item B).** They stayed out while no corpus fight
/// reached either. The Coach's synthetic roots do (a Defect deck holding
/// Splash), and the oracle roots both under the same bare recorded-owner
/// requirement (frozen Python `start_combat`, deleted #2827), so they join on the Necrobinder
/// terms above: the owner pool is read at play time from
/// `CHARACTER_CARD_POOL_ROWS_V1101`, which carries all five pools, and admitting
/// them changes no document field. The parity witness is
/// `fixture_tests::silent_and_defect_generators_open_and_match_the_oracle`.
const MODELED_GENERATION_CHARACTERS: [&str; 5] = [
    "CHARACTER.DEFECT",
    "CHARACTER.IRONCLAD",
    "CHARACTER.NECROBINDER",
    "CHARACTER.REGENT",
    "CHARACTER.SILENT",
];

/// Content whose orb slots make `BaseOrbSlotCount` observable
/// (`_ORB_SLOT_SENSITIVE_*`, frozen Python `start_combat`, deleted #2827). Kept narrow and
/// cited: a wrong entry here would refuse fights the oracle admits.
const ORB_SLOT_SENSITIVE_RELICS: [&str; 5] = [
    "RELIC.CRACKED_CORE",
    "RELIC.INFUSED_CORE",
    "RELIC.GOLD_PLATED_CABLES",
    "RELIC.RUNIC_CAPACITOR",
    "RELIC.SYMBIOTIC_VIRUS",
];

/// Creature kinds whose turn-1 intent is rolled during combat setup.
///
/// `combat_sim._INITIAL_RAND` at v0.111.0 (frozen Python, deleted #2827), plus the
/// one per-instance entry `_INITIAL_RAND_INSTANCES` carries
/// (`(EXOSKELETON, slot 3)`), read from the oracle rather than
/// guessed from the presence of a random move table:
/// `_is_initial_rand_instance`. Each member's
/// machine is re-read from the DLL in [`roll_initial_random_ai`].
///
/// Each entry carries the two generated random-move table indexes its
/// initial `RandomBranchState` adds, in `AddBranch` order (the per-kind IL is
/// tabulated on [`roll_initial_random_ai`]). They are the same indexes the
/// engine's steady roller names (`engine::admission`, `engine::monsters`).
const INITIAL_RANDOM_AI_KINDS: [(MonsterKind, [u8; 2]); 3] = [
    (
        MonsterKind::Fabricator,
        [
            crate::engine::monsters::FABRICATOR_FABRICATE_INDEX,
            crate::engine::monsters::FABRICATOR_STRIKE_INDEX,
        ],
    ),
    (
        MonsterKind::Flyconid,
        [
            crate::engine::admission::FLYCONID_FRAIL_INDEX,
            crate::engine::admission::FLYCONID_SMASH_INDEX,
        ],
    ),
    (
        MonsterKind::LeafSlimeS,
        [
            crate::engine::admission::SLIME_ATTACK_INDEX,
            crate::engine::admission::SLIME_STATUS_INDEX,
        ],
    ),
];

/// The one `(kind, slot)` instance outside [`INITIAL_RANDOM_AI_KINDS`], with
/// its branches in the same shape.
const INITIAL_RANDOM_AI_INSTANCES: [(MonsterKind, i32, [u8; 2]); 1] = [(
    MonsterKind::Exoskeleton,
    3,
    [
        crate::engine::admission::EXOSKELETON_SKITTER_INDEX,
        crate::engine::admission::EXOSKELETON_MANDIBLES_INDEX,
    ],
)];

/// The initial-RAND branches of `monster`, when `_is_initial_rand_instance`
/// (frozen Python, deleted #2827) enrolls it.
fn initial_random_ai_branches(monster: &MonsterSpec) -> Option<[u8; 2]> {
    INITIAL_RANDOM_AI_KINDS
        .iter()
        .find_map(|(kind, branches)| (*kind == monster.kind).then_some(*branches))
        .or_else(|| {
            INITIAL_RANDOM_AI_INSTANCES
                .iter()
                .find_map(|(kind, slot, branches)| {
                    (*kind == monster.kind && *slot == monster.slot).then_some(*branches)
                })
        })
}

/// `Fabricator::get_CanFabricate` (`0xb3d2c`): `GetTeammatesOf(Creature)`
/// filtered by `IsAlive`, `Count() < 4` (IL_0036 `ldc.i4.4`, IL_0037 `clt`).
const FABRICATOR_CAN_FABRICATE_BELOW: usize = 4;

/// Card ids whose instantiation attaches the slot-7 physical payload.
///
/// `combat_sim._PHYSICAL_COST_CARD_IDS` (frozen Python, deleted #2827), consumed by
/// `_with_default_physical_card_state` as the deck
/// is built (`start_combat`), so a document that omits
/// the payload for one of them is a different document even before anything
/// modifies it. Ids, not numbers — the content values those cards carry all
/// come from the generated tables.
///
/// [`piles`] writes the **default** payload `(tag, (), 0, False, ())` for
/// every id here except [`CardId::TheScythe`], whose payload carries its saved
/// damage growth and `DeckVersion` row ([`the_scythe_entry_growth`]).
///
/// # Why the default payload is exact for the other twenty (v0.111.0 IL)
///
/// A deck card round-trips the save only through `CardModel::ToSerializable`
/// (RVA `0x7e2bc`): the id (IL_0019), `CurrentUpgradeLevel` (IL_0025),
/// `SavedProperties::From` (IL_0031), the enchantment (IL_003d..IL_0049) and
/// `FloorAddedToDeck` (IL_0055) — nothing else. `CardModel::FromSerializable`
/// (RVA `0x7e31c`) rebuilds the copy from the canonical model's `ToMutable`
/// (IL_0017), `SavedProperties::Fill`s the props (IL_002a), then re-applies the
/// enchantment (IL_0068..IL_007d) and replays the upgrades (IL_0097..IL_00ae).
/// `SavedProperties` reflects over `[SavedProperty]` properties only, and a
/// metadata walk of the `CustomAttribute` table (the same reflection
/// `build_mcr_tables.Dll.saved_properties` models) finds **none** on any of the
/// twenty types or on `CardModel` itself; the positive controls on the same
/// walk are `TheScythe` (`CurrentDamage`, `IncreasedDamage`),
/// `GeneticAlgorithm` (`CurrentBlock`, `IncreasedBlock`) and `MadScience`
/// (`TinkerTimeType`, `TinkerTimeRider`). None of the twenty overrides
/// `AfterDeserialized`. So every field the slot-7 payload models — local cost
/// rows, dynamic-var damage growth (`Rampage::set_ExtraDamageFromPlays`
/// `0xe8e48`, `Claw::set_ExtraDamageFromClawPlays` `0xdb158`,
/// `KinglyPunch::set_ExtraDamage` `0xe3aee`, `Thrash::set_ExtraDamage`
/// `0xeeb51`, …), `UpMySleeve::set_TimesPlayedThisCombat` (`0xefaed`) — is at
/// its canonical value on the deck copy, and the combat copy is its memberwise
/// clone (`Player::PopulateCombatState` `0x117a90` IL_002e →
/// `CombatState::CloneCard` `0x137004`). Combat-entry hooks such as
/// `Stomp::AfterCardEnteredCombat` (`0xeced8`) act later, in the engine, on
/// that default payload.
const PHYSICAL_STATE_CARDS: [CardId; 21] = [
    CardId::BansheesCry,
    CardId::BulletTime,
    CardId::Claw,
    CardId::Enlightenment,
    CardId::Flatten,
    CardId::KinglyKick,
    CardId::KinglyPunch,
    CardId::Maul,
    CardId::Melancholy,
    CardId::Midnight,
    CardId::Modded,
    CardId::MomentumStrike,
    CardId::Pinpoint,
    CardId::Rampage,
    CardId::RocketPunch,
    CardId::Stomp,
    CardId::TheBall,
    CardId::TheScythe,
    CardId::Thrash,
    CardId::UpMySleeve,
    CardId::Wither,
];

/// Base orb slots by character: Defect 3 (`0x280b6f`), everyone else 0
/// (`CharacterModel` default `0x22960e`).
fn orb_base_slots(character: Option<&str>) -> i64 {
    i64::from(character == Some("CHARACTER.DEFECT")) * 3
}

/// Voltaic's Lightning-channel count at the pre-hook document: `Some(0)` when
/// a Voltaic is anywhere in the fight's card closure (`Catalog::specs`), else
/// `None` (the `-1` "untracked" sentinel, which `engine::admission` pairs
/// with a closure that holds no Voltaic).
///
/// The closure, not only the entering deck (#3389). The multiplier below
/// reads `CombatManager.Instance.History` (`<>c::<get_CanonicalVars>b__5_0`,
/// `IL_0019`–`IL_0023`): the combat's history, not state the card owns. So a
/// Voltaic generated mid-combat (a Skill Potion offer, say) counts every
/// owner Lightning channeled since combat start, including the ones channeled
/// before it existed. Seeding only when one was physically in the deck left
/// that read untracked, and `steps::defect_orb::channel_voltaic` refused it
/// by name. The catalog is immutable per fight and interns every identity
/// the fight can generate, so a closure without a Voltaic can never play one.
///
/// IL (v0.111.0 `sts2.dll`, sha `9cb4f1ad…`): Voltaic keeps no counter of its
/// own. `Voltaic/<OnPlay>d__8::MoveNext` (RVA `0x3c6d14`) stores
/// `CalculatedChannels`' `Calculate(target)` into `<lightningChanneledCount>5__2`
/// at `IL_00a0`–`IL_00c9`, and `Voltaic::get_CanonicalVars` (RVA `0xf0048`)
/// builds that var from `CalculationBaseVar(0)` (`IL_0014`–`IL_0019`) and
/// `CalculationExtraVar(1)` (`IL_0021`–`IL_0026`), and `CalculatedVar::Calculate`
/// (RVA `0x101484`, `IL_0054`–`IL_0070`) returns `base + extra × multiplier`
/// (`Voltaic::OnUpgrade`, RVA `0xf0107`, only removes a keyword). The
/// multiplier `<>c::<get_CanonicalVars>b__5_0` (RVA `0x3c6c9c`) is
/// `OfType<OrbChanneledEntry>().Count(..)` over `CombatManager.History.Entries`
/// (`IL_0019`–`IL_0039`), counting entries whose
/// `Actor.Player` is the card's owner and whose `Orb` is a `LightningOrb`
/// (`<>c__DisplayClass5_0::<get_CanonicalVars>b__1`, RVA `0x3c6ce8`,
/// `IL_0001`–`IL_0027`). The history is combat-scoped: `CombatHistory::Clear`
/// (RVA `0x1386b7`) runs from `CombatManager/<EndCombatInternal>d__122`
/// (RVA `0x3f3e60`, `IL_0209`) and `CombatManager::Reset` (RVA `0x136278`,
/// `IL_008c`), so every combat starts with an empty history and the count at
/// the pre-hook document is `0`. The only writer is `OrbCmd`'s channel,
/// counted by [`crate::engine::orbs::channel`] as the opening's turn-one walk
/// runs it (Cracked Core's `BeforeSideTurnStart` Lightning included), so the
/// seed is not advanced here.
fn voltaic_lightning_channeled_seed(catalog: &crate::catalog::Catalog) -> Option<i64> {
    crate::engine::admission::closure_holds_voltaic(catalog).then_some(0)
}

// ---------------------------------------------------------------------------
// The opening
// ---------------------------------------------------------------------------

/// The opening's run-RNG products, before the engine is entered.
///
/// Split out from [`build`] deliberately, because the two answer different
/// questions and one of them can fail for reasons that are not about the
/// opening at all. Everything here is computed by this module — the shuffled
/// pile, the roster and its HP rolls, all nine stream states — and can be
/// compared with the oracle's document field for field. What comes **after**
/// it (admission, the two fire points, the deal) runs through the pre-existing
/// canonical ⇄ hot boundary, whose own narrowings refuse states the oracle
/// projects perfectly well.
#[derive(Clone, Debug, PartialEq)]
pub struct PreHook {
    /// The state the two combat-start fire points act on.
    pub document: CanonicalStateV2,
    /// The shuffled draw pile as indices into `deck_entering`, in save-array
    /// order before the shuffle and in pile order after it.
    pub pile: Vec<usize>,
    /// The pile positions (indices into [`Self::pile`]) Stone Cracker
    /// upgraded, in the order it took them; empty without the relic.
    pub stone_cracker_upgraded: Vec<usize>,
    /// The created roster, in creation order.
    pub monsters: Vec<MonsterSpec>,
    /// The nine streams as the opening left them.
    pub rng: BTreeMap<String, CanonicalRngV2>,
    /// The Strength `Girya` applies on entering the room: the saved
    /// `TimesLifted` when the relic is owned, else 0 ([`girya_lifts`]).
    pub girya_lifts: i32,
    /// Whether `SlingOfCourage` applies its Strength: owned, and the node is
    /// an elite ([`sling_of_courage_elite`]).
    pub sling_of_courage_elite: bool,
    /// Whether `Pantograph` heals at combat start: owned, and the room is a
    /// boss room ([`pantograph_before_combat_start`]).
    pub pantograph_boss: bool,
    /// Whether `Planisphere` heals on entering this room: owned, an
    /// `unknown` point, and the point's first room
    /// ([`planisphere_after_room_entered`]).
    pub planisphere_heal: bool,
    /// What `Red Skull` does on entering this room
    /// ([`red_skull_room_entry_lifts`]).
    pub red_skull_room_entry: RedSkullRoomEntry,
    /// Whether `MawBank` gains its gold on entering this room: owned, and the
    /// save says no item has been bought ([`maw_bank_room_entry_armed`]).
    pub maw_bank_gold: bool,
}

/// Red Skull's room-entry outcome ([`red_skull_room_entry_lifts`], #3044).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RedSkullRoomEntry {
    /// Not owned, or above half HP when it runs: nothing.
    Inert,
    /// At or below half HP when it runs, and it stays there: `+3` Strength.
    Lifts,
    /// It ran first at or below half HP, and Planisphere's heal then lifted
    /// the owner above it: `+3` Strength that native holds, stale, until the
    /// turn-one Blood Vial heal re-runs Red Skull.
    BeforeCrossingHeal,
}

/// Build one fight's opening from its entry facts.
///
/// Runs [`build_pre_hook`], then admits the state it built, applies the
/// room-entry Strength block, fires `AfterRoomEntered` and
/// `BeforeCombatStart`, deals the first hand, and emits the post-opening
/// canonical v2 document.
pub fn build(
    entry: &EntryDocument,
    options: &OpeningOptions<'_>,
) -> Result<Opening, OpeningRefusal> {
    let pre_hook = build_pre_hook(entry)?;
    let catalog = HotBoundary::catalog_from_canonical(&pre_hook.document)?;
    let mut state = HotBoundary::from_canonical(&pre_hook.document, &catalog)?;
    // #3364: the pre-hook roster is exactly what `AfterAddedToRoom` left, so
    // a spawn Strength the IL proves self-applied gets its owner as applier
    // here, before any hook below can move it.
    crate::engine::damage::attribute_spawn_strength_to_its_owner(&mut state);
    let mut events = Vec::new();
    // frozen Python `start_combat`, deleted #2827, and it is **before** the template pass, not
    // part of it: the oracle runs the room-entry Strength loop and Ember Tea's
    // decrement, then `_fire_relic_templates(s, "AfterRoomEntered")`.
    apply_room_entry_strength(
        &catalog,
        pre_hook.sling_of_courage_elite,
        pre_hook.girya_lifts,
        &mut state,
        &mut events,
    )?;
    // Red Skull and Planisphere in the order `red_skull_room_entry_lifts`
    // established: a Red Skull that ran first read the pre-heal HP.
    if pre_hook.red_skull_room_entry == RedSkullRoomEntry::BeforeCrossingHeal {
        red_skull_after_room_entered(&mut state, &mut events)?;
    }
    if pre_hook.planisphere_heal {
        planisphere_after_room_entered(&catalog, &mut state)?;
    }
    match pre_hook.red_skull_room_entry {
        RedSkullRoomEntry::Lifts => red_skull_after_room_entered(&mut state, &mut events)?,
        // Native now holds Strength above half HP; the turn-one Blood Vial
        // heal re-runs Red Skull and removes it.
        RedSkullRoomEntry::BeforeCrossingHeal => state.fanouts.set_red_skull_latch_stale(true),
        RedSkullRoomEntry::Inert => {}
    }
    if pre_hook.maw_bank_gold {
        maw_bank_after_room_entered(&catalog, &mut state, &mut events)?;
    }
    crate::engine::fire_after_room_entered(&catalog, &mut state, &mut events)?;
    if pre_hook.pantograph_boss {
        pantograph_before_combat_start(&catalog, &mut state, &mut events)?;
    }
    crate::engine::fire_before_combat_start(&catalog, &mut state, &mut events)?;
    let deferred_fixup = {
        let relics: BTreeSet<&str> = entry
            .relic_entry
            .relics_entering
            .iter()
            .map(String::as_str)
            .collect();
        let draw = pre_hook
            .document
            .piles
            .get("draw")
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if defers_turn_one_fixup(&relics, is_thieving_hopper_entry(entry)) {
            let (order, _) = turn_one_fixup_order(draw)?;
            (!is_identity_order(&order)).then_some(order)
        } else {
            None
        }
    };
    let deal = |state: &mut crate::hot::HotState, events: &mut Vec<_>| {
        if deferred_fixup.is_some() {
            crate::engine::deal_opening_hand_deferring_turn_one_fixup(state, &catalog, events)
        } else {
            crate::engine::deal_opening_hand(state, &catalog, events)
        }
    };
    let native_checkpoints = if options.record_native_checkpoints {
        let (dealt, recorded) =
            crate::engine::native_checkpoint::record(|| deal(&mut state, &mut events));
        dealt?;
        Some(
            recorded
                .into_iter()
                .map(|(kind, checkpoint)| OpeningNativeCheckpoint {
                    kind,
                    state: HotBoundary::try_to_canonical(&checkpoint, &catalog)
                        .map_err(|refusal| refusal.to_string()),
                })
                .collect(),
        )
    } else {
        deal(&mut state, &mut events)?;
        None
    };
    let mut opened = HotBoundary::try_to_canonical(&state, &catalog)?;

    // The oracle allocates physical-card uids at its first
    // `_normalize_card_identities` (frozen Python, deleted #2827), in
    // `_ALL_CARD_PILES` order; `stamp_card_identities` numbered the pre-hook
    // pile in the order that allocation sees. The two agree exactly while
    // nothing between the stamp and the deal moves a card other than a prefix
    // draw — checked here rather than assumed.
    let expected = jeweled_mask_moved_order(
        &opened,
        expected_identity_order(
            &pre_hook.document,
            &entry.relic_entry.relics_entering,
            deferred_fixup.as_deref(),
        ),
        &entry.relic_entry.relics_entering,
    );
    check_card_identity_allocation(
        &opened,
        &expected,
        pre_deal_allocations(&entry.relic_entry.relics_entering),
    )?;
    let pre_splice_rng = opened.rng.clone();
    let mcr_splice_applied = match options.mcr_first_checksum {
        Some(checksum) => {
            checksum.splice_into(&mut opened.rng);
            true
        }
        None => false,
    };
    Ok(Opening {
        document: opened,
        mcr_splice_applied,
        pre_splice_rng,
        native_checkpoints,
    })
}

/// The opening up to the state the fire points act on.
///
/// Takes no options: the `--mcr` splice acts on the post-deal document, and
/// nothing above the fire points depends on it.
pub fn build_pre_hook(entry: &EntryDocument) -> Result<PreHook, OpeningRefusal> {
    // --- the fight's ascension (#2539) -------------------------------------
    // `start_combat` validates `entry["ascension"]` first, in its pure
    // construction half: an exact `AscensionLevel` in `0..=MAX_ASCENSION`,
    // or a refusal. Every roster builder selects its tier by it.
    let ascension = entry
        .ascension
        .and_then(|level| u8::try_from(level).ok())
        .filter(|level| *level <= crate::encounters::MAX_ASCENSION)
        .ok_or(OpeningRefusal::AscensionNotExact {
            recorded: entry.ascension,
        })?;
    let relics: BTreeSet<&str> = entry
        .relic_entry
        .relics_entering
        .iter()
        .map(String::as_str)
        .collect();
    let deck_ids: BTreeSet<&str> = entry
        .deck_entering
        .iter()
        .map(|card| card.id.trim_start_matches("CARD."))
        .collect();

    // --- Ember Tea's persistent charge (frozen Python `_mad_science_entry_variant`, deleted #2827) -------------
    // `exact_finite_counter("RELIC.EMBER_TEA", 5)`, read in `start_combat`'s
    // pure-construction half before any stream exists, so its refusal is
    // reached before the generator analysis just as the oracle's is.
    let ember_tea = ember_tea_combats_left(entry, &relics)?;
    // The two phylacteries together (frozen Python `start_combat`, deleted #2827), a few
    // lines before Girya's read, in the same pure-construction half.
    refuse_incompatible_phylacteries(&relics)?;
    // `Girya`'s `TimesLifted` (frozen Python `start_combat`, deleted #2827), read in the same
    // pure-construction half, a few lines after Ember Tea's.
    let girya = girya_lifts(entry, &relics)?;
    // `LizardTail.WasUsed` (frozen Python `start_combat`, deleted #2827), read a few lines
    // before Girya's in the same pure-construction half; see
    // [`lizard_tail_used`].
    let lizard_tail_spent = lizard_tail_used(entry, &relics)?;
    // `entry_potion_relics` (frozen Python `start_combat`, deleted #2827): Petrified Toad's
    // fixed procurement observes trailing empty slots, so the oracle demands
    // the exact positive belt capacity even when the belt is empty.
    // `DELICATE_FROND` shares the oracle's gate but is still refused by the
    // body table below, so only the Toad half is reachable here.
    if relics.contains("RELIC.PETRIFIED_TOAD") {
        petrified_toad_belt_is_exact(entry)?;
    }

    // --- the generator / owner analysis (Part C1, C3) ---------------------
    //
    // Placed here, before any stream is constructed, because the oracle places
    // it there: it runs in the frozen Python (deleted #2827) `start_combat`'s pure
    // construction half, above the line where the run RNG first exists.
    let character = entry.character.as_deref();
    // #2942: a Mad Science copy generates only as the Chaos variant
    // (`MadScience/<ExecuteRider>d__57::MoveNext` `0x3aa9c4` IL_031e-IL_0395
    // is the one `GetDistinctForCombat` call in the card), so the variant,
    // not the id, decides whether it is an owner-pool source.
    let mad_science = entering_mad_science_variant(entry)?;
    let sources: Vec<&str> = owner_pool_generation_sources(&deck_ids)
        .into_iter()
        .filter(|id| {
            *id != CardId::MadScience.as_str()
                || mad_science.is_some_and(|variant| variant.rider_name() == "Chaos")
        })
        .collect();
    if !sources.is_empty() {
        let owner = character.unwrap_or("<undetermined>");
        let ironclad_only: Vec<&str> = sources
            .iter()
            .copied()
            .filter(|id| IRONCLAD_ONLY_GENERATION_CARDS.contains(id))
            .collect();
        if !ironclad_only.is_empty() && owner != "CHARACTER.IRONCLAD" {
            return Err(OpeningRefusal::GenerationPoolUnmodeled {
                source: ironclad_only.join("/"),
                character: owner.to_string(),
            });
        }
        if !MODELED_GENERATION_CHARACTERS.contains(&owner) {
            return Err(OpeningRefusal::GenerationPoolUnmodeled {
                source: sources.join("/"),
                character: owner.to_string(),
            });
        }
        owner_pool_generation_profile_is_recorded(entry, &sources)?;
    }

    // --- the owner-pool listener provenance (frozen Python `start_combat`, deleted #2827) ---------------
    // Hello World, Call of the Void and Creative AI each need an owner with a
    // character card pool (any character since #3375; the oracle demanded
    // the card's own) and a recorded unlock profile, and the oracle
    // checks both in its pure-construction half, before any stream exists.
    // [`pre_hook_document`] writes the pool these gates guard, so without
    // them a fight the oracle refuses would open with the field silently
    // absent. The oracle's third check per card, an exact
    // `CombatCardGeneration` counter, is [`listener_pool_streams_are_recorded`].
    listener_pool_provenance_is_recorded(entry, &deck_ids)?;

    // --- orb base slots (frozen Python `start_combat`, deleted #2827; I8) -------------------------------
    // Placed here because the oracle places it here: before any monster
    // exists, right after the belt partition.
    if character.is_none()
        && relics
            .iter()
            .any(|id| ORB_SLOT_SENSITIVE_RELICS.contains(id))
    {
        return Err(OpeningRefusal::OrbBaseSlotsUndetermined);
    }
    // Kusarigama's two same-hook order gates (frozen Python `start_combat`, deleted #2827), after the orb-slot read and before Stone Cracker, as
    // in the oracle. Blessed Antler's `BeforeHandDraw` order gate
    // (`start_combat`) sits between the two in the oracle, so it runs between
    // them here too (#2992).
    refuse_unordered_kusarigama_peers(&relics, &deck_ids, KusarigamaGate::MusicBox)?;
    refuse_unordered_blessed_antler_peers(&relics)?;
    refuse_unordered_kusarigama_peers(&relics, &deck_ids, KusarigamaGate::DaughterOfTheWind)?;

    // --- the nine streams, read from the save at their recorded counters ---
    let mut rng = stream_states(entry)?;
    listener_pool_streams_are_recorded(entry, &deck_ids)?;

    // --- the opening shuffle (frozen Python `start_combat`, deleted #2827) ------------------------------
    // `pile = list(deck)` in save-array order, then one `UnstableShuffle`.
    let mut pile: Vec<usize> = (0..entry.deck_entering.len()).collect();
    let shuffle_key = canonical_stream_key("shuffle");
    let mut shuffle_stream = xoshiro(&rng[shuffle_key]);
    let draws = shuffle::opening_shuffle(&mut shuffle_stream, &mut pile);
    debug_assert_eq!(draws, pile.len().saturating_sub(1) as u64);
    rng.insert(shuffle_key.to_string(), canonical(&shuffle_stream));

    // --- Stone Cracker's pre-draw upgrade (frozen Python `start_combat`, deleted #2827) -----------------
    // After the shuffle and before `make_monsters`, as in the oracle. The two
    // consume disjoint streams (Sel vs Niche), so only the refusal order
    // depends on this position.
    let stone_cracker_upgraded = if relics.contains("RELIC.STONE_CRACKER") {
        let sel_key = canonical_stream_key("combat_card_selection");
        let mut sel = xoshiro(&rng[sel_key]);
        let upgraded = stone_cracker_upgrades(entry, &pile, &mut sel)?;
        rng.insert(sel_key.to_string(), canonical(&sel));
        Some(upgraded)
    } else {
        None
    };

    // --- monster creation (frozen Python `start_combat`, deleted #2827) ---------------------------------
    // `total_floor = node_index + 1`.
    let total_floor = entry.node_index.checked_add(1);
    let niche_key = canonical_stream_key("niche");
    let niche = xoshiro(&rng[niche_key]);
    let mut roster = make_monsters(
        &entry.encounter_id,
        niche,
        run_set_seed_v109(&entry.seed),
        total_floor,
        ascension,
    )??;
    rng.insert(niche_key.to_string(), canonical(&roster.niche));

    // --- the room-entry relic block (frozen Python `start_combat`, deleted #2827) ------------------------
    // Ordered against the oracle, which runs every one of these AFTER
    // `make_monsters` — several of them write creature state that only exists
    // once the roster does. Keeping the order means the census reports the
    // *first* thing a fight actually lacks, which while the roster registry is
    // empty is the E4b frontier and not a relic body no fight ever reached.
    for relic in OPENING_WINDOW_RELIC_BODIES
        .into_iter()
        .chain(TURN_ONE_ORB_RELICS)
    {
        if relics.contains(relic) {
            return Err(OpeningRefusal::RoomEntryRelicNotModeled {
                relic: relic.to_string(),
            });
        }
    }
    // The turn-1 half of the window (#2731), placed here rather than after the
    // deal for the same reason `TURN_ONE_ORB_RELICS` is: the refusal depends
    // only on ownership, so the earliest position that can decide it is the
    // right one. The oracle decides `TWISTED_FUNNEL` here, since the all-enemy
    // debuffs are applied inline right after `make_monsters`
    // (frozen Python `start_combat`, deleted #2827; the `partial_artifact` gate in `start_combat`).
    for relic in TURN_ONE_RELIC_BODIES {
        if relics.contains(relic) {
            return Err(OpeningRefusal::TurnOneRelicBodyNotModeled {
                relic: relic.to_string(),
            });
        }
    }
    // The same-hook co-ownerships the oracle refuses around the two
    // `AfterPlayerTurnStart` bodies this crate gained in #2827. Ownership
    // only, so it sits with the other ownership decisions.
    after_player_turn_start_peers_are_ordered(
        &relics,
        &entry.relic_entry.relics_entering,
        entry.relic_entry.dispatch_ordered,
    )?;
    // The turn-1 `BeforeHandDraw` card movers this crate runs in a fixed
    // order (#3162). Ownership only.
    refuse_unordered_hand_draw_card_peers(&relics)?;
    // Tea of Discourtesy's `BeforeCombatStart` Dazed (#3162): inert once spent,
    // unmodeled while charged. Where the body table refused it before.
    let tea_of_discourtesy_spent = tea_of_discourtesy_spent(entry, &relics)?;
    // Fencing Manual's `AfterSideTurnStart` Forge (#3090): ownership and
    // the vouched inventory order.
    fencing_manual_turn_start_peers_are_ordered(
        &relics,
        &entry.relic_entry.relics_entering,
        entry.relic_entry.dispatch_ordered,
    )?;
    // Toolbox's owner, profile and same-hook gates (#2827 item B), before
    // its stream gate as in the oracle.
    toolbox_provenance_is_recorded(entry, &relics)?;
    // Ownership plus the save's stream table: placed after the same-hook
    // order gate so a generation relic beside an unordered peer still
    // reports that.
    generation_relic_streams_are_recorded(entry, &relics)?;
    // The oracle's `partial_artifact` gate (frozen Python `start_combat`, deleted #2827), at
    // the oracle's own position: immediately after `make_monsters` and before
    // any all-enemy debuff is applied. It is placed AFTER the body table above
    // so the intersection it measures is the oracle's — see
    // [`TURN_ONE_ALL_ENEMY_DEBUFF_RELICS`].
    turn_one_debuff_artifact_is_unambiguous(&relics, &roster.monsters)?;
    // The per-fight state this builder does not seed
    // ([`PER_FIGHT_RELIC_STATE_UNSEEDED`]). Placed with the other
    // ownership-only decisions for the same reason: the earliest position that
    // can decide it is the right one, and it can only change WHICH refusal a
    // multi-relic fight reports. Deliberately AFTER the two body tables, so a
    // fight holding both keeps reporting the body it lacks — the seeding gap is
    // the narrower, newer claim.
    for (relic, field) in PER_FIGHT_RELIC_STATE_UNSEEDED {
        if relics.contains(relic) {
            return Err(OpeningRefusal::PerFightRelicStateNotSeeded {
                relic: relic.to_string(),
                field,
            });
        }
    }
    // Maw Bank's room-entry `GainGold` (#3328), which [`build`] runs when the
    // saved flag says no item was bought, and which reaches Dragon Fruit
    // (#3320). Kept at #3320's gate position so the Dragon Fruit + Red Skull
    // refusal is reported where the pair's was.
    let maw_bank_gold = maw_bank_room_entry_armed(entry, &relics)?;
    // `Planisphere` heals 5 on entering an UNKNOWN (`?`) map point
    // ([`planisphere_heals_here`], #3162), which [`build`] runs when the point
    // resolved straight into its scheduled monster room
    // ([`planisphere_after_room_entered`]) and which refuses otherwise;
    // `Pantograph` heals 25 before a BOSS combat, which [`build`] runs
    // ([`pantograph_before_combat_start`]).
    let planisphere_heal = relics.contains("RELIC.PLANISPHERE")
        && planisphere_heals_here(entry.map_point_type.as_deref(), entry.node_type.as_deref());
    if planisphere_heal
        && !(entry.map_point_type.as_deref() == Some("unknown")
            && entry.next_normal_encounter.as_deref() == Some(entry.encounter_id.as_str()))
    {
        return Err(OpeningRefusal::RoomEntryHealUnmodeled {
            relic: "RELIC.PLANISPHERE",
        });
    }
    let pantograph_boss =
        relics.contains("RELIC.PANTOGRAPH") && entry.node_type.as_deref() == Some("boss");
    // The two Tea Sets' saved `GainEnergyInNextCombat`, which
    // [`pre_hook_document`] seeds. The oracle reads the
    // Fake one first, as an exact bool, in its pure-construction half
    // (frozen Python `start_combat`, deleted #2827), and the real one here, beside Pantograph
    // (`start_combat`); a fight holding both undated reports the real one,
    // which is the narrower position this gate can reproduce.
    if relics.contains("RELIC.VENERABLE_TEA_SET") && entry.relic_entry.tea_set_charged.is_none() {
        return Err(OpeningRefusal::TeaSetChargeUndated {
            relic: "RELIC.VENERABLE_TEA_SET",
        });
    }
    if relics.contains("RELIC.FAKE_VENERABLE_TEA_SET")
        && entry.relic_entry.fake_tea_set_charged.is_none()
    {
        return Err(OpeningRefusal::TeaSetChargeUndated {
            relic: "RELIC.FAKE_VENERABLE_TEA_SET",
        });
    }
    if relics.contains("RELIC.BOOMING_CONCH") && entry.node_type.is_none() {
        return Err(OpeningRefusal::BoomingConchRoomTypeUnknown);
    }
    let sling_elite = sling_of_courage_elite(entry, &relics)?;
    let red_skull_lifts = red_skull_room_entry_lifts(
        entry,
        &relics,
        planisphere_heal,
        &entry.relic_entry.relics_entering,
        entry.relic_entry.dispatch_ordered,
    )?;
    ruined_helmet_room_entry_strength_is_ordered(
        &relics,
        sling_elite,
        girya,
        ember_tea,
        red_skull_lifts,
    )?;

    // --- turn-1 initial-RAND intents (frozen Python `start_combat`, deleted #2827) ---------------
    // The oracle rolls these after constructing the `State` and before
    // `_fire_relic_templates(s, "AfterRoomEntered")` (`start_combat`), so the roll
    // lands on the pre-hook roster and stream, ahead of both fire points.
    roll_initial_random_ai(&mut roster.monsters, &mut rng)?;

    // --- the seeded per-relic counters (frozen Python `start_combat`, deleted #2827) --------------------
    let mut seeded = seeded_counters(entry, &relics)?;
    // A spent Lizard Tail's `lizard_tail_used` (frozen Python `start_combat`, deleted #2827). The
    // unspent value `0` is what the boundary already projects for an owner.
    if lizard_tail_spent {
        seeded.insert("lizard_tail_used", 1);
    }
    // A spent Tea of Discourtesy's `CombatsLeft` 0 (#3162); the boundary's
    // `-1` default is the not-owned value, so an owner's 0 is written.
    if tea_of_discourtesy_spent {
        seeded.insert("tea_discourtesy_combats_left", 0);
    }
    counter_relic_dispatch_is_representable(entry, &relics)?;
    // --- Symbiotic Virus's same-hook order (frozen Python `start_combat`, deleted #2827) ----------------
    // The oracle refuses these right after constructing the `State`, which is
    // after the seeded counters above.
    symbiotic_virus_turn_start_order_is_recorded(&relics, orb_base_slots(character))?;

    // --- the pre-hook document ---------------------------------------------
    let document = pre_hook_document(
        entry,
        &pile,
        &roster.monsters,
        &rng,
        &seeded,
        &relics,
        ember_tea,
        stone_cracker_upgraded.as_deref(),
    )?;
    Ok(PreHook {
        document,
        pile,
        stone_cracker_upgraded: stone_cracker_upgraded.unwrap_or_default(),
        monsters: roster.monsters,
        rng,
        girya_lifts: girya,
        pantograph_boss,
        planisphere_heal,
        red_skull_room_entry: red_skull_lifts,
        sling_of_courage_elite: sling_elite,
        maw_bank_gold,
    })
}

// ---------------------------------------------------------------------------
// Steps
// ---------------------------------------------------------------------------

/// Roll every INITIAL-RAND instance's turn-1 intent off `MonsterAi`.
///
/// The oracle is `_initialize_random_ai` (frozen Python, deleted #2827),
/// called from `start_combat`: the living roster members for
/// which `_is_initial_rand_instance` holds, in roster order,
/// Exoskeleton through `_roll_uniform_cr_intent`, Flyconid
/// through `_roll_flyconid_intent(initial=True)` and
/// Fabricator through `_roll_fabricator_intent(initial=True)`; Leaf Slime S takes the uniform-CR path too.
///
/// # Where native rolls it (v0.111.0 IL)
///
/// `CombatManager/<AfterCreatureAdded>d__113::MoveNext` (`0x3f2dfc`) calls
/// `MonsterModel::RollMove` at IL_00cb for each enemy added on the enemy
/// side, and `RollMove` (`0x825c8`) hands `RunRngSet::get_MonsterAi`
/// (IL_0015) to `MonsterMoveStateMachine::RollMove`. A machine whose initial
/// state is a `RandomBranchState` therefore spends one `MonsterAi` draw at
/// combat setup; one whose initial state is a `MoveState` spends none.
///
/// `RandomBranchState::GetNextState` (`0x792c4`) draws
/// `NextFloat(Sum(weights))` once (IL_002b-IL_0033), then walks the branches
/// in `AddBranch` order subtracting each weight and takes the first at
/// `<= 0` (IL_0050-IL_0066). `GetStateWeight` (`0x79380`) with an **empty**
/// `StateLog` gives every branch its full weight: `UseOnlyOnce` (3) tests
/// `StateLog.Contains` (IL_0036-IL_006c), `CanRepeatForever` (0) exits early
/// (IL_0087-IL_0098), `CannotRepeat` (2) compares against the log's last
/// entries (IL_00a8-IL_0136), and the cooldown filter takes
/// `Reverse().Take(cooldown)` of the log (IL_0164-IL_01b2) — every one a
/// no-op on an empty log. `RandomBranchState::get_ShouldAppearInLogs`
/// (`0x79110`) is `false`, so only the selected `MoveState` is logged.
///
/// Per kind, each machine's initial state and branch list:
///
/// | kind | `GenerateMoveStateMachine` | initial state | branches, in order |
/// |---|---|---|---|
/// | Leaf Slime S | `0xb8784` | `RAND` (ctor IL_00a2) | TACKLE, GOOP, `AddBranch(state, CannotRepeat)` `0x792b5` at IL_007f/IL_0087, weight 1 |
/// | Exoskeleton, slot `fourth` | `0xb3658` | `INIT_MOVE` conditional (ctor IL_0155); `b__22_3` (`0xb39c5`, `SlotName == "fourth"`) routes to `RAND` (IL_0103-IL_0113) | SKITTER, MANDIBLES, `AddBranch(state, CannotRepeat, 1.0f)` `0x7922c` at IL_00a8/IL_00b6 |
/// | Flyconid | `0xb4a7c` | `INITIAL` (ctor IL_011d), distinct from the steady `RAND` | FRAIL_SPORES (cooldown 2, CannotRepeat, `0x79296` IL_00e9), SMASH (CannotRepeat, `0x792b5` IL_00f2), weight 1 |
/// | Fabricator | `0xb3bc0` | `fabricateBranch` conditional (ctor IL_0164) | `RAND` while `get_CanFabricate` (`b__17_2` `0xb4051`), else DISINTEGRATE (`b__17_3` `0xb4059`); `RAND` is FABRICATE, FABRICATING_STRIKE, `AddBranch(state, 0, weight)` `0x7925a` at IL_00c4/IL_00ec with lambdas `b__17_0`/`b__17_1` (`0x35a31a`/`0x35a321`) returning `1.0f` |
///
/// Every row is two branches of weight 1 on an empty log: one
/// `NextFloat(2.0f)` selects the first when it is `<= 1`. The rolled name
/// is the generated random-move table's row for the branch
/// (`content_tables::random_moves`), at the same indexes the engine's steady
/// roller uses (`engine::admission`, `engine::monsters`).
///
/// `ConditionalBranchState::GetNextState` (`0x78d10`) takes the first branch
/// whose condition evaluates `> 0`, and `get_CanFabricate` (`0xb3d2c`) is
/// the living-teammate count `< 4` ([`FABRICATOR_CAN_FABRICATE_BELOW`]). No
/// corpus roster reaches setup with four living enemies beside a Fabricator
/// (`FABRICATOR_NORMAL` creates the Fabricator alone), so that
/// DISINTEGRATE follow refuses by name rather than open unwitnessed.
///
/// # What refuses
///
/// Any enrolled instance whose `next_move`/`move_log` is not empty: the
/// oracle requires it for Flyconid and Fabricator, and for Exoskeleton slot
/// 3 (`_validate_exoskeleton_initial_rand`, frozen Python, deleted #2827), and the
/// all-weights-1 reading above holds only for an empty log. The Exoskeleton
/// `HardToKillPower` check in that validator is the roster builder's, which
/// writes the IL value.
fn roll_initial_random_ai(
    monsters: &mut [MonsterSpec],
    rng: &mut BTreeMap<String, CanonicalRngV2>,
) -> Result<(), OpeningRefusal> {
    let enrolled: Vec<(usize, [u8; 2])> = monsters
        .iter()
        .enumerate()
        .filter(|(_, monster)| monster.hp > 0)
        .filter_map(|(index, monster)| {
            initial_random_ai_branches(monster).map(|branches| (index, branches))
        })
        .collect();
    if enrolled.is_empty() {
        return Ok(());
    }
    // Preflight every instance before the shared stream moves, as the
    // oracle's Exoskeleton pass does (frozen Python `_initialize_random_ai`, deleted #2827).
    let living = monsters.iter().filter(|monster| monster.hp > 0).count();
    for &(index, _) in &enrolled {
        let monster = &monsters[index];
        if !monster.next_move.is_empty()
            || !monster.move_log.is_empty()
            || (monster.kind == MonsterKind::Fabricator && living >= FABRICATOR_CAN_FABRICATE_BELOW)
        {
            return Err(OpeningRefusal::InitialRandomAiNotModeled {
                kind: monster.kind.as_str(),
            });
        }
    }
    let key = canonical_stream_key("monster_ai");
    let mut ai = xoshiro(&rng[key]);
    for (index, [first, second]) in enrolled {
        let kind = monsters[index].kind;
        // Two branches of weight 1: `NextFloat(2.0f)`, and the first branch
        // holds when the roll minus its weight is `<= 0`.
        let roll = ai.next_float(2.0);
        let selected = if roll - 1.0 <= 0.0 { first } else { second };
        let rolled = crate::content_tables::random_moves(kind)
            .and_then(|moves| moves.get(usize::from(selected)))
            .ok_or(OpeningRefusal::InitialRandomAiNotModeled {
                kind: kind.as_str(),
            })?;
        monsters[index].next_move = rolled.name;
        monsters[index].move_log = std::slice::from_ref(&rolled.name);
    }
    rng.insert(key.to_string(), canonical(&ai));
    Ok(())
}

/// `exact_finite_counter("RELIC.EMBER_TEA", 5)` (frozen Python `_mad_science_entry_variant`, deleted #2827).
///
/// `None` when the relic is not owned, which is the oracle's `-1` and the
/// canonical field's default, so nothing is emitted for a fight without it.
/// An owned relic with an absent, non-integer, negative or out-of-domain
/// counter refuses: the charge decides whether this fight gets Strength at
/// all, and it is run state a guess cannot recover.
///
/// The domain's upper end is the relic's own starting charge,
/// [`EMBER_TEA_MAX_COMBATS`]. Rust's compact relic-counter encoding
/// (`hot::set_ember_tea_combats_left`) holds the same `0..=5` since #3381;
/// before that it stopped at 4 and refused a save recording the untouched
/// charge of 5 at the boundary.
fn ember_tea_combats_left(
    entry: &EntryDocument,
    relics: &BTreeSet<&str>,
) -> Result<Option<i64>, OpeningRefusal> {
    if !relics.contains("RELIC.EMBER_TEA") {
        return Ok(None);
    }
    let raw = entry.relic_entry.relic_counters.get("RELIC.EMBER_TEA");
    let value = raw
        .and_then(Value::as_i64)
        .filter(|value| (0..=EMBER_TEA_MAX_COMBATS).contains(value))
        .ok_or_else(|| OpeningRefusal::RelicCounterNotExact {
            relic: "RELIC.EMBER_TEA",
            value: raw.map_or_else(|| "absent".to_string(), ToString::to_string),
        })?;
    Ok(Some(value))
}

/// Whether an owned `RELIC.TEA_OF_DISCOURTESY` is spent, so its combat-start
/// body is inert (#3162).
///
/// v0.111.0 `sts2.dll` (sha256 `9cb4f1ad…fbf12b4`, hash-verified 2026-09-26).
/// The type's one hook is `BeforeCombatStart` (RVA `0x9c56c`, the async stub).
/// Its saved state is `_combatsLeft` (`CombatsLeft`): the constructor
/// (`0x9c5af`) writes `ldc.i4.1` at `IL_0002`, and the only other writer is the
/// body's decrement, so the domain is `0..=1`. The body is
/// `TeaOfDiscourtesy/<BeforeCombatStart>d__16::MoveNext` (RVA `0x3324dc`):
///
/// * `CombatsLeft; ldc.i4.0; bgt.s`, else `leave` (`IL_001d`-`IL_0026`) —
///   a spent Tea returns before it writes anything;
/// * otherwise `CardPileCmd::AddToCombatAndPreview<Dazed>(Owner.Creature,
///   Draw, DazedCount = 2, Owner, Random)` (`IL_002b`-`IL_0053`), then
///   `set_CombatsLeft(CombatsLeft - 1)` and `Flash` (`IL_00aa`-`IL_00bb`).
///
/// So a saved `0` is the whole combat-start effect: nothing, and the canonical
/// `tea_discourtesy_combats_left` is `0`. A saved `1` would insert two Dazed
/// before the deal and allocate their uids ahead of it, which this crate does
/// not run: that is [`OpeningRefusal::RoomEntryRelicNotModeled`], the refusal
/// the relic carried from [`OPENING_WINDOW_RELIC_BODIES`] until #3162. Any
/// other value refuses as [`OpeningRefusal::RelicCounterNotExact`].
fn tea_of_discourtesy_spent(
    entry: &EntryDocument,
    relics: &BTreeSet<&str>,
) -> Result<bool, OpeningRefusal> {
    const RELIC: &str = "RELIC.TEA_OF_DISCOURTESY";
    if !relics.contains(RELIC) {
        return Ok(false);
    }
    let raw = entry.relic_entry.relic_counters.get(RELIC);
    match raw.and_then(Value::as_i64) {
        Some(0) => Ok(true),
        Some(1) => Err(OpeningRefusal::RoomEntryRelicNotModeled {
            relic: RELIC.to_string(),
        }),
        _ => Err(OpeningRefusal::RelicCounterNotExact {
            relic: RELIC,
            value: raw.map_or_else(|| "absent".to_string(), ToString::to_string),
        }),
    }
}

/// Whether an owned `RELIC.LIZARD_TAIL` is already spent (#2847).
///
/// `LizardTail::_wasUsed` is run state: its only writer of `true` is
/// `<AfterPreventingDeath>d__11::MoveNext` (RVA `0x3290fc`,
/// `ldc.i4.1; set_WasUsed` at `IL_0027`-`IL_0028`), the relic declares no
/// combat-start hook that resets it, and `ShouldDieLate` (`0x9650c`) reads it
/// at `IL_0012` to decide whether the tail still prevents a death. So the
/// value a combat starts with is exactly the saved `WasUsed`, which
/// `start_combat` requires as an exact bool (frozen Python, deleted #2827).
/// Before this seed a spent tail opened unspent: a document that would revive
/// the player a second time, silently.
fn lizard_tail_used(
    entry: &EntryDocument,
    relics: &BTreeSet<&str>,
) -> Result<bool, OpeningRefusal> {
    const RELIC: &str = "RELIC.LIZARD_TAIL";
    if !relics.contains(RELIC) {
        return Ok(false);
    }
    let raw = entry.relic_entry.relic_counters.get(RELIC);
    raw.and_then(Value::as_bool)
        .ok_or_else(|| OpeningRefusal::RelicCounterNotExact {
            relic: RELIC,
            value: raw.map_or_else(|| "absent".to_string(), ToString::to_string),
        })
}

/// `Girya`'s saved `TimesLifted`, validated as `start_combat` validates it
/// (frozen Python, deleted #2827).
///
/// 0 when the relic is not owned, which is also the oracle's `girya_lifts`
/// default. An owned Girya with an absent, non-integer or out-of-domain
/// counter refuses: the lift count *is* the room-entry Strength, and it is run
/// state a guess cannot recover. Every corpus save holding the relic records
/// the property, `0` included (38 of 38 saves, 2026-09-23).
fn girya_lifts(entry: &EntryDocument, relics: &BTreeSet<&str>) -> Result<i32, OpeningRefusal> {
    if !relics.contains("RELIC.GIRYA") {
        return Ok(0);
    }
    let raw = entry.relic_entry.relic_counters.get("RELIC.GIRYA");
    raw.and_then(Value::as_i64)
        .filter(|value| (0..=GIRYA_MAX_LIFTS).contains(value))
        .and_then(|value| i32::try_from(value).ok())
        .ok_or_else(|| OpeningRefusal::RelicCounterNotExact {
            relic: "RELIC.GIRYA",
            value: raw.map_or_else(|| "absent".to_string(), ToString::to_string),
        })
}

/// The oracle's `_potion_slots_from_entry(entry)` with `require_capacity`
/// (frozen Python, deleted #2827), for Petrified Toad.
///
/// `TryToProcure` fills the first empty slot, so the procurement depends on
/// the belt's trailing empties, which only the recorded capacity says. The
/// oracle refuses a Toad fight without an exact positive
/// `max_potion_slot_count`. A malformed slot row refuses first, by its own
/// name, through [`potion_slots`] — the same predicate every fight's belt
/// goes through, rather than a second copy of it (#2791). (A colliding pair
/// is refused one stage later by the boundary's dense-mirror check, as that
/// function's doc comment says.)
fn petrified_toad_belt_is_exact(entry: &EntryDocument) -> Result<(), OpeningRefusal> {
    potion_slots(entry)?;
    let capacity_positive = entry
        .belt
        .max_potion_slot_count
        .is_some_and(|capacity| capacity > 0);
    if capacity_positive {
        return Ok(());
    }
    Err(OpeningRefusal::PotionBeltNotExact {
        relic: "RELIC.PETRIFIED_TOAD",
        capacity: entry.belt.max_potion_slot_count,
    })
}

/// The oracle's `partial_artifact` gate (frozen Python `start_combat`, deleted #2827).
///
/// An innate enemy `ArtifactPower` is applied at `AfterAddedToRoom`, which
/// precedes the player's first `BeforeSideTurnStart`, so it **eats** a turn-1
/// relic debuff instead of being applied after it (#148). Artifact consumes one
/// stack per blocked *application*, so a monster entering with Artifact `a`
/// against `n` owned debuff relics swallows the first `min(a, n)` of them — and
/// `0 < a < n` is the case where *which* ones are swallowed depends on the
/// relics' position in `Player.Relics`, which the run payload does not vouch
/// for on this hook. The oracle refuses exactly that, and names the relics and
/// the `(slot, kind, artifact)` of every offending monster; this reproduces
/// both the condition and the report.
///
/// `a == 0` (nothing blocked) and `a >= n` (everything blocked) are both
/// order-independent and pass; `engine::relics::turn_one_all_enemy_debuffs`
/// then reaches the shared `ArtifactPower` gate in
/// `damage::apply_relic_monster_debuff` for the second case, so the decrement
/// is the engine's one implementation rather than a second copy here.
///
/// The Artifact amount is read from the roster's own spawn state
/// (`MonsterSpec::initial_state`, which mirrors the `combat_sim.Monster`
/// keyword the content builder sets) rather than from the built document, for
/// the same reason the rest of this function's neighbours are ownership tests:
/// this is above the boundary and the document does not exist yet.
fn turn_one_debuff_artifact_is_unambiguous(
    relics: &BTreeSet<&str>,
    monsters: &[MonsterSpec],
) -> Result<(), OpeningRefusal> {
    let owned: Vec<&str> = TURN_ONE_ALL_ENEMY_DEBUFF_RELICS
        .into_iter()
        .filter(|relic| relics.contains(relic))
        .collect();
    if owned.is_empty() {
        return Ok(());
    }
    let count = i64::try_from(owned.len()).expect("three relics fit in an i64");
    let offending: Vec<String> = monsters
        .iter()
        .filter_map(|monster| {
            let artifact = monster
                .initial_state
                .iter()
                .find_map(|(field, value)| (*field == "artifact").then(|| value.as_i64()).flatten())
                .unwrap_or(0);
            // The oracle's tuple verbatim: `(m.slot, m.kind, m.artifact)`.
            (0 < artifact && artifact < count)
                .then(|| format!("({}, {}, {artifact})", monster.slot, monster.kind.as_str()))
        })
        .collect();
    if offending.is_empty() {
        return Ok(());
    }
    Err(OpeningRefusal::TurnOneDebuffPartialArtifact {
        relics: owned.into_iter().map(ToString::to_string).collect(),
        monsters: offending,
    })
}

/// Whether Red Skull applies its Strength on entering this room (#3044).
///
/// `RedSkull/<AfterRoomEntered>d__11::MoveNext` (v0.111.0 RVA `0x32f6bc`)
/// returns unless the room is a `CombatRoom` (IL_001d-IL_002a) and then awaits
/// `ModifyStrengthIfNecessary` (IL_002c) with no `IsInProgress` test, so an
/// owner at or below half HP applies `+3` Strength and sets its latch
/// (`<ModifyStrengthIfNecessary>d__14::MoveNext` `0x32f790` IL_011f-IL_0198).
///
/// The HP it reads is the entry HP, unless Planisphere's room-entry heal ran
/// first: the two are peers in the acquisition-ordered `AfterRoomEntered`
/// relic walk, and Planisphere is the one peer that writes HP. That heal does
/// not re-run Red Skull, because `CombatManager.IsInProgress` is still false
/// (`<AfterCurrentHpChanged>d__13::MoveNext` `0x32f5ec` IL_001d-IL_0029):
/// `CombatManager::SetUpCombat` (`0x135900`) installs a fresh
/// `CombatTurnState` at IL_0020-IL_0028 before `<StartCombat>d__46`
/// (`0x310bf0`) fires `AfterRoomEntered` at IL_01aa, and only
/// `<StartCombatInternal>d__98` (`0x3f71b0` IL_020e-IL_020f) sets it, later.
///
/// So a heal that does not cross half HP leaves either order the same. One
/// that crosses needs the order, which is read from the inventory when it is
/// vouched for as dispatch order (`relics_entering_dispatch_ordered`):
///
/// * every Planisphere before every Red Skull: Red Skull reads the healed HP
///   and does nothing;
/// * every Red Skull first: it applies `+3` at the entry HP and the heal
///   leaves native holding that Strength above half HP, with the latch set.
///   Native keeps it until the next `AfterCurrentHpChanged` of any creature.
///   The port derives the latch from the HP quotient rather than storing it,
///   so this is admitted only when that next change is certainly a player
///   heal before anything reads Strength: Blood Vial's or Fake Blood Vial's
///   turn-one heal (`relics::after_player_turn_start_late`, and the
///   `AfterPlayerTurnStartLate` template), a `CreatureCmd.Heal` of a
///   positive amount, which raises the hook even at full HP (`<Heal>d__20`
///   `0x3eb4b0` IL_04c9-IL_04ee). Nothing between room entry and that heal
///   reads the player's Strength, and a Strength grant in between adds the
///   same amount in either state. The corpus has one such fight
///   (MFKRHBXBVZ7V node 21, Red Skull at inventory index 6 before Planisphere
///   at 7, 42/87 healed to 47/87, Blood Vial owned): its `.mcr` opening
///   checkpoint holds no Strength, the removal having run.
///
/// Every other crossing refuses.
fn red_skull_room_entry_lifts(
    entry: &EntryDocument,
    relics: &BTreeSet<&str>,
    planisphere_heal: bool,
    relics_entering: &[String],
    dispatch_ordered: bool,
) -> Result<RedSkullRoomEntry, OpeningRefusal> {
    const RED_SKULL: &str = "RELIC.RED_SKULL";
    const PLANISPHERE: &str = "RELIC.PLANISPHERE";
    let outcome = |lifts: bool| {
        if lifts {
            RedSkullRoomEntry::Lifts
        } else {
            RedSkullRoomEntry::Inert
        }
    };
    if !relics.contains(RED_SKULL) {
        return Ok(RedSkullRoomEntry::Inert);
    }
    let at_or_below = |hp: i64| hp.saturating_mul(2) <= entry.max_hp_entering;
    let entry_hp = entry.hp_entering;
    if !planisphere_heal {
        return Ok(outcome(at_or_below(entry_hp)));
    }
    let healed = entry_hp
        .saturating_add(i64::from(PLANISPHERE_HEAL))
        .min(entry.max_hp_entering);
    if at_or_below(entry_hp) == at_or_below(healed) {
        return Ok(outcome(at_or_below(entry_hp)));
    }
    let positions = |relic: &'static str| {
        relics_entering
            .iter()
            .enumerate()
            .filter(move |(_, owned)| owned.as_str() == relic)
            .map(|(index, _)| index)
    };
    let precedes = |first: &'static str, second: &'static str| {
        dispatch_ordered
            && positions(first)
                .max()
                .zip(positions(second).min())
                .is_some_and(|(last_first, first_second)| last_first < first_second)
    };
    if precedes(PLANISPHERE, RED_SKULL) {
        return Ok(outcome(at_or_below(healed)));
    }
    // A heal only lifts across the threshold, so Red Skull ran at or below it.
    if precedes(RED_SKULL, PLANISPHERE)
        && (relics.contains("RELIC.BLOOD_VIAL") || relics.contains("RELIC.FAKE_BLOOD_VIAL"))
    {
        return Ok(RedSkullRoomEntry::BeforeCrossingHeal);
    }
    Err(OpeningRefusal::RelicCombinationRefused {
        relics: vec![PLANISPHERE.to_string(), RED_SKULL.to_string()],
        reason: "Planisphere's heal crosses Red Skull's threshold in an order no turn-one heal resolves",
    })
}

/// Red Skull's room-entry `+3` Strength ([`red_skull_room_entry_lifts`]
/// decides it and carries the IL), through
/// [`crate::engine::damage::apply_owner_strength`] like every other room-entry
/// grant, so Ruined Helmet's first-positive doubling applies. [`build`] places
/// it after Planisphere's heal, or before it for
/// [`RedSkullRoomEntry::BeforeCrossingHeal`]: the order
/// [`red_skull_room_entry_lifts`] read or proved order-free.
fn red_skull_after_room_entered(
    state: &mut crate::hot::HotState,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), OpeningRefusal> {
    crate::engine::damage::apply_owner_strength(
        state,
        crate::engine::damage::RED_SKULL_STRENGTH,
        events,
    )?;
    crate::coverage::record_relic(crate::ids::RelicId::RelicRedSkull);
    Ok(())
}

/// The oracle's `room_entry_strength_sources` Ruined Helmet gate
/// (frozen Python `start_combat`, deleted #2827).
///
/// Five relics can put a positive Strength application in the
/// acquisition-ordered `AfterRoomEntered` group: `VAJRA` at
/// [`VAJRA_STRENGTH`], `SLING_OF_COURAGE` at [`SLING_OF_COURAGE_STRENGTH`] on
/// an elite node (#2827 item B; before it left [`OPENING_WINDOW_RELIC_BODIES`]
/// it refused above), `SWORD_OF_JADE` at [`SWORD_OF_JADE_STRENGTH`] —
/// whose grant the oracle deliberately leaves to the relic's own template —
/// `GIRYA` at its saved lift count while that is positive, and Ember Tea at
/// [`EMBER_TEA_STRENGTH`] while it is still charged, in the oracle's list
/// order (frozen Python `start_combat`, deleted #2827). A Girya with no lifts applies nothing
/// (`Girya/<AfterRoomEntered>d__14` `IL_0027`), and the oracle leaves it out
/// of the list for the same reason (`if girya_lifts else []`). `RED_SKULL`
/// joins at [`crate::engine::damage::RED_SKULL_STRENGTH`] when it applies on
/// entry ([`red_skull_room_entry_lifts`], #3044).
fn ruined_helmet_room_entry_strength_is_ordered(
    relics: &BTreeSet<&str>,
    sling_elite: bool,
    girya_lifts: i32,
    ember_tea: Option<i64>,
    red_skull_lifts: RedSkullRoomEntry,
) -> Result<(), OpeningRefusal> {
    if !relics.contains("RELIC.RUINED_HELMET") {
        return Ok(());
    }
    let sources: Vec<(&str, i32)> = [
        relics
            .contains("RELIC.VAJRA")
            .then_some(("RELIC.VAJRA", VAJRA_STRENGTH)),
        sling_elite.then_some(("RELIC.SLING_OF_COURAGE", SLING_OF_COURAGE_STRENGTH)),
        relics
            .contains("RELIC.SWORD_OF_JADE")
            .then_some(("RELIC.SWORD_OF_JADE", SWORD_OF_JADE_STRENGTH)),
        (girya_lifts > 0).then_some(("RELIC.GIRYA", girya_lifts)),
        (ember_tea.unwrap_or(0) > 0).then_some(("RELIC.EMBER_TEA", EMBER_TEA_STRENGTH)),
        (red_skull_lifts != RedSkullRoomEntry::Inert)
            .then_some(("RELIC.RED_SKULL", crate::engine::damage::RED_SKULL_STRENGTH)),
    ]
    .into_iter()
    .flatten()
    .collect();
    let distinct: BTreeSet<i32> = sources.iter().map(|(_, amount)| *amount).collect();
    if distinct.len() > 1 {
        return Err(OpeningRefusal::RuinedHelmetStrengthOrder {
            sources: sources
                .into_iter()
                .map(|(relic, amount)| format!("{relic}={amount}"))
                .collect(),
        });
    }
    Ok(())
}

/// The oracle's phylactery co-ownership gate (frozen Python `start_combat`, deleted #2827; #2827).
///
/// Bound Phylactery (the Necrobinder starter) and Phylactery Unbound both
/// summon Osty at `BeforeCombatStart`. `start_combat` refuses the pair as an
/// ownership the game does not produce, before any stream exists. Without this
/// the opening would summon both (`engine::phylactery_before_combat_start`)
/// and emit a document for a fight Python never roots. `engine::admission`
/// refuses the same pair for every other root, but the opening does not admit.
fn refuse_incompatible_phylacteries(relics: &BTreeSet<&str>) -> Result<(), OpeningRefusal> {
    const PAIR: [&str; 2] = ["RELIC.BOUND_PHYLACTERY", "RELIC.PHYLACTERY_UNBOUND"];
    if PAIR.iter().all(|relic| relics.contains(relic)) {
        return Err(OpeningRefusal::RelicCombinationRefused {
            relics: PAIR.iter().map(ToString::to_string).collect(),
            reason: "incompatible phylactery bootstrap relic ownership",
        });
    }
    Ok(())
}

/// The oracle's two `start_combat` gates that name Kusarigama (#2827).
///
/// Both are about `AfterCardPlayed`, inside the fight rather than the
/// opening, but `start_combat` decides them, so they refuse here in the
/// oracle's order. They only became reachable when Kusarigama left
/// [`OPENING_WINDOW_RELIC_BODIES`]. Relic acquisition order is not in the
/// run payload, and in each case it decides something observable:
///
/// * `MUSIC_BOX` + `KUSARIGAMA` (frozen Python, deleted #2827): whether Music
///   Box's Ethereal clone is inserted before Kusarigama's lethal damage ends
///   combat.
/// * `DAUGHTER_OF_THE_WIND` + `KUSARIGAMA` with `JUGGERNAUT` in the deck: Daughter of the Wind's block feeds Juggernaut's
///   `CombatTargets` roll, and so does Kusarigama's hit, so their order moves
///   the nested target rolls. `engine::admission` carries a wider version
///   (Juggernaut reachable by generation too) for other roots. The deck test
///   here is the oracle's literal `deck_ids` test, the same set `start_combat`
///   reads.
///
/// `gate` picks one of the two, because Blessed Antler's order gate
/// ([`refuse_unordered_blessed_antler_peers`]) sits between them in the oracle
/// (#2992) and a save holding several refused combinations reports the
/// oracle's first.
fn refuse_unordered_kusarigama_peers(
    relics: &BTreeSet<&str>,
    deck_ids: &BTreeSet<&str>,
    gate: KusarigamaGate,
) -> Result<(), OpeningRefusal> {
    if !relics.contains("RELIC.KUSARIGAMA") {
        return Ok(());
    }
    if gate == KusarigamaGate::MusicBox && relics.contains("RELIC.MUSIC_BOX") {
        return Err(OpeningRefusal::RelicCombinationRefused {
            relics: vec![
                "RELIC.MUSIC_BOX".to_string(),
                "RELIC.KUSARIGAMA".to_string(),
            ],
            reason: "AfterCardPlayed relic acquisition order can decide whether \
                     Music Box's clone is inserted before Kusarigama's lethal \
                     damage ends combat",
        });
    }
    if gate == KusarigamaGate::DaughterOfTheWind
        && relics.contains("RELIC.DAUGHTER_OF_THE_WIND")
        && deck_ids.contains("JUGGERNAUT")
    {
        return Err(OpeningRefusal::RelicCombinationRefused {
            relics: vec![
                "RELIC.DAUGHTER_OF_THE_WIND".to_string(),
                "RELIC.KUSARIGAMA".to_string(),
            ],
            reason: "with Juggernaut in the deck, same-hook block/damage order \
                     changes nested target rolls",
        });
    }
    Ok(())
}

/// Whether `SlingOfCourage` applies its Strength in this fight.
///
/// The oracle's gate (frozen Python `start_combat`, deleted #2827): an owned Sling needs the
/// node type, and applies only on an elite node. The body's own test is
/// `room.RoomType == Elite` ([`SLING_OF_COURAGE_STRENGTH`]), and the entry's
/// node type is that room's kind; without one, whether it fires is unknown.
fn sling_of_courage_elite(
    entry: &EntryDocument,
    relics: &BTreeSet<&str>,
) -> Result<bool, OpeningRefusal> {
    if !relics.contains("RELIC.SLING_OF_COURAGE") {
        return Ok(false);
    }
    match entry.node_type.as_deref() {
        None => Err(OpeningRefusal::SlingOfCourageRoomTypeUnknown),
        Some(kind) => Ok(kind == "elite"),
    }
}

/// The relics whose v0.111.0 relic body subscribes `BeforeHandDraw`, the hook
/// Toolbox's turn-1 choice runs on: `combat_sim._current_relic_hooks` over
/// `RELICS_CENSUS` (`Pendulum` migrated onto it, frozen Python, deleted #2827).
const BEFORE_HAND_DRAW_RELICS: [&str; 8] = [
    "RELIC.BLESSED_ANTLER",
    "RELIC.FUNERARY_MASK",
    "RELIC.JEWELED_MASK",
    "RELIC.NINJA_SCROLL",
    "RELIC.PENDULUM",
    "RELIC.POLLINOUS_CORE",
    "RELIC.RADIANT_PEARL",
    "RELIC.TOOLBOX",
];

/// The oracle's gates on `RELIC.TOOLBOX` as a generation source (#2827 item
/// B), in its order.
///
/// `Toolbox` is in `_GENERATION_RELIC_POOLS` with the Colorless transform pool
/// (frozen Python, deleted #2827), so `start_combat` requires, for it:
///
/// * a recorded owner `CharacterCardPool` (frozen Python `start_combat`, deleted #2827) —
///   [`OpeningRefusal::GenerationPoolUnmodeled`];
/// * a recorded card-pool profile — [`OpeningRefusal::GenerationPoolPartialUnlock`].
///   The oracle also demanded the explicitly fully-unlocked (Ironclad) owner
///   pool; #3325 drops that conjunct. `Toolbox/<BeforeHandDraw>d__4::MoveNext`
///   (v0.111.0 RVA `0x332adc`) reads the static `CardPool<ColorlessCardPool>`
///   (IL_005e) filtered by the owner's `UnlockState` (IL_0063-IL_007e) and no
///   character pool, and the body now draws exactly that under the recorded
///   profile (`engine::relics::colorless_generation_relic_pool`), so an
///   Ironclad epoch cannot move its options;
/// * no same-`BeforeHandDraw` relic peer (`start_combat`): relics within a
///   hook dispatch in acquisition order, and the choice's modal pause changes
///   what a later peer observes. [`OpeningRefusal::RelicCombinationRefused`].
///
/// Its exact `CombatCardGeneration` counter is
/// [`generation_relic_streams_are_recorded`]'s. The body itself is
/// `engine::relics::before_hand_draw`'s turn-1 Toolbox pause, which the
/// opening reaches through [`crate::engine::deal_opening_hand`].
fn toolbox_provenance_is_recorded(
    entry: &EntryDocument,
    relics: &BTreeSet<&str>,
) -> Result<(), OpeningRefusal> {
    if !relics.contains("RELIC.TOOLBOX") {
        return Ok(());
    }
    let owner = entry.character.as_deref().unwrap_or("<undetermined>");
    if reward_card_pool(entry.character.as_deref()).is_none() {
        return Err(OpeningRefusal::GenerationPoolUnmodeled {
            source: "TOOLBOX".to_string(),
            character: owner.to_string(),
        });
    }
    let epochs = normalized_card_pool_epochs(entry).unwrap_or_default();
    if epochs.is_empty() {
        return Err(OpeningRefusal::GenerationPoolPartialUnlock {
            source: "TOOLBOX".to_string(),
            missing: epochs,
        });
    }
    let peers: Vec<String> = BEFORE_HAND_DRAW_RELICS
        .into_iter()
        .filter(|relic| *relic != "RELIC.TOOLBOX" && relics.contains(relic))
        .map(ToString::to_string)
        .collect();
    if !peers.is_empty() {
        return Err(OpeningRefusal::RelicCombinationRefused {
            relics: std::iter::once("RELIC.TOOLBOX".to_string())
                .chain(peers)
                .collect(),
            reason: "Toolbox's same-BeforeHandDraw relic order is unrecorded",
        });
    }
    Ok(())
}

/// Which of [`refuse_unordered_kusarigama_peers`]' two oracle gates to run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KusarigamaGate {
    /// `MUSIC_BOX` + `KUSARIGAMA` (frozen Python `start_combat`, deleted #2827).
    MusicBox,
    /// `DAUGHTER_OF_THE_WIND` + `KUSARIGAMA` with Juggernaut (frozen Python `start_combat`, deleted #2827).
    DaughterOfTheWind,
}

/// The relics the oracle groups with Blessed Antler on `BeforeHandDraw`
/// (frozen Python `start_combat`, deleted #2827), Antler first.
const BEFORE_HAND_DRAW_ORDER_RELICS: [&str; 7] = [
    "RELIC.BLESSED_ANTLER",
    "RELIC.TOOLBOX",
    "RELIC.NINJA_SCROLL",
    "RELIC.FUNERARY_MASK",
    "RELIC.POLLINOUS_CORE",
    "RELIC.JEWELED_MASK",
    "RELIC.RADIANT_PEARL",
];

/// The oracle's `BeforeHandDraw` acquisition-order gate for Blessed Antler
/// (frozen Python `start_combat`, deleted #2827; #2992).
///
/// Native runs relic `BeforeHandDraw` listeners in acquisition order, which
/// neither the run nor the `.mcr` records. Antler's three random-position Draw
/// inserts (`<BeforeHandDraw>d__7::MoveNext` `0x31fe64`, `IL_0094`-`IL_009d`)
/// move the insertion bounds, RNG draws, generated-card listener chains and
/// uid allocation of every other body in this group, so the oracle refuses
/// Antler beside any of them. `start_combat` decides it before any stream
/// exists; `engine::admission` carries the same gate (`Blessed Antler
/// BeforeHandDraw acquisition order`) for every other root, but the opening
/// does not admit. Pollinous Core, Toolbox, Funerary Mask, Jeweled Mask and
/// Radiant Pearl reach it (the last three since #3162); Ninja Scroll is still
/// in [`OPENING_WINDOW_RELIC_BODIES`], which refuses later in [`build`].
/// The port keeps the oracle's whole set by name rather than leaning on that
/// table. The Toolbox pair is also refused by Toolbox's own peer gate
/// ([`toolbox_provenance_is_recorded`], the frozen Python oracle's own, deleted #2827), which the
/// oracle reaches first but which runs after this one here; both refuse the
/// same fight as `relic_combination_refused` naming both relics, so only the
/// reason string differs.
///
/// The named relics are the owned members in the oracle's `sorted(...)` order.
fn refuse_unordered_blessed_antler_peers(relics: &BTreeSet<&str>) -> Result<(), OpeningRefusal> {
    if !relics.contains("RELIC.BLESSED_ANTLER") {
        return Ok(());
    }
    let owned: BTreeSet<&str> = BEFORE_HAND_DRAW_ORDER_RELICS
        .into_iter()
        .filter(|relic| relics.contains(relic))
        .collect();
    if owned.len() > 1 {
        return Err(OpeningRefusal::RelicCombinationRefused {
            relics: owned.into_iter().map(ToString::to_string).collect(),
            reason: "BeforeHandDraw relic acquisition order is unrecorded; random \
                     insertion, generated hooks and terminal gates do not commute",
        });
    }
    Ok(())
}

/// The turn-1 `BeforeHandDraw` relic bodies that move a Draw or Hand card,
/// which the engine runs in a fixed order (#3162).
///
/// Native runs relic `BeforeHandDraw` listeners in `Player.Relics` order;
/// `engine::relics::continue_before_hand_draw_after_toolbox` runs Funerary
/// Mask's three random-position Soul inserts into Draw
/// (`FuneraryMask/<BeforeHandDraw>d__6::MoveNext` `0x325000`,
/// `AddGeneratedCardToCombat(card, Draw, Owner, Random)` at
/// `IL_0074`-`IL_007d`), then Jeweled Mask's Draw-to-Hand move
/// (`engine::relics::jeweled_mask_before_hand_draw`, `0x3272f4`), and
/// `engine::relics::radiant_pearl_before_hand_draw` inserts Luminesce at the
/// Hand's bottom last (`RadiantPearl/<BeforeHandDraw>d__6::MoveNext`
/// `0x32f10c`, `AddGeneratedCardsToCombat(list, Hand, Owner, …)` at
/// `IL_008d`-`IL_0096`). Any two of them observe each other: a Draw insert
/// moves the pile Jeweled Mask selects from and the bound of the next random
/// insert, two Hand writers reverse the Hand's order, and each generator
/// allocates the next card uid. `NINJA_SCROLL`'s turn-1 Shivs into Hand are
/// spelled too; it is still refused earlier by
/// [`OPENING_WINDOW_RELIC_BODIES`], and naming it here keeps retiring that
/// entry from silently opening the pair.
///
/// Blessed Antler and Toolbox beside any of these are refused by their own
/// gates ([`refuse_unordered_blessed_antler_peers`],
/// [`toolbox_provenance_is_recorded`]). Pendulum's and Pollinous Core's bodies
/// write only their counters and commute with every row here. The corpus holds
/// no fight with two of these, so no recorded order is vouched for; a pair
/// refuses at any order. The named relics are the owned members, sorted.
fn refuse_unordered_hand_draw_card_peers(relics: &BTreeSet<&str>) -> Result<(), OpeningRefusal> {
    const HAND_DRAW_CARD_MOVERS: [&str; 4] = [
        "RELIC.FUNERARY_MASK",
        "RELIC.JEWELED_MASK",
        "RELIC.NINJA_SCROLL",
        "RELIC.RADIANT_PEARL",
    ];
    let owned: Vec<String> = HAND_DRAW_CARD_MOVERS
        .into_iter()
        .filter(|relic| relics.contains(relic))
        .map(ToString::to_string)
        .collect();
    if owned.len() > 1 {
        return Err(OpeningRefusal::RelicCombinationRefused {
            relics: owned,
            reason: "the turn-1 BeforeHandDraw order of two Draw/Hand card movers is \
                     observable, and relic acquisition order is not vouched for it",
        });
    }
    Ok(())
}

/// `Pantograph`'s heal amount: `get_CanonicalVars` (RVA `0x98c9e`) is
/// `ldc.i4.s 25; newobj HealVar::.ctor` at `IL_0001`-`IL_0008`.
const PANTOGRAPH_HEAL: i32 = 25;

/// `Pantograph`'s boss-room heal at combat start (#3162).
///
/// v0.111.0 `sts2.dll` (sha256 `9cb4f1ad…fbf12b4`, hash-verified 2026-09-26).
/// The type declares two hooks and no saved property:
///
/// * `AfterRoomEntered` (RVA `0x98cb4`, synchronous): a dead owner returns
///   (`IL_000c`-`IL_0023`); otherwise it writes
///   `Status = Map.BossMapPoint.parents.Contains(CurrentMapPoint)`
///   (`IL_0024`-`IL_005c`) and returns. `RelicModel::set_Status` is the relic's
///   display state, so the hook writes nothing a combat reads.
/// * `BeforeCombatStart` (RVA `0x98d1c`, the async stub), whose body is
///   `Pantograph/<BeforeCombatStart>d__5::MoveNext` (RVA `0x32d694`): a dead
///   owner `leave`s (`IL_0020`-`IL_0032`); `CurrentRoom.RoomType; ldc.i4.3;
///   beq.s` (`IL_0037`-`IL_004f`, `RoomType` declares `Unassigned, Monster,
///   Elite, Boss = 3, …`) leaves in every other room; then `Flash` and
///   `CreatureCmd::Heal(Owner.Creature, DynamicVars.Heal.BaseValue, true)` at
///   `IL_0054`-`IL_0076`, awaited.
///
/// The room kind is `entry.node_type`, which is the room (the review and the
/// census name a boss fight `"boss"`), so [`build_pre_hook`] decides the gate
/// and this applies the heal with the clamp every owner heal in this crate
/// uses (`engine::relics::after_player_turn_start_late`,
/// `TemplateEffect::Heal`): `min(max_hp, hp + 25)`.
///
/// Position: the first listener of the `BeforeCombatStart` walk, ahead of
/// [`crate::engine::fire_before_combat_start`]. Native walks the walk's relic
/// listeners in `Player.Relics` order; every represented peer on the hook
/// (the two phylacteries' summons, the passive pets, Vambrace's reset, Tea of
/// Discourtesy, Petrified Toad's `…Late` potion) writes pets, relic state,
/// piles or the belt and none reads the player's HP, so an owner heal commutes
/// with them.
///
/// Red Skull is the one HP reader, and it is not a peer on this hook: the heal
/// raises its own `AfterCurrentHpChanged`, and `BeforeCombatStart` runs after
/// `CombatManager/<StartCombatInternal>d__98::MoveNext` (`0x3f71b0`) sets
/// `IsInProgress` at IL_020e-IL_020f (the hook is IL_023b), so Red Skull's
/// in-progress gate is open and a heal that lifts the owner above half HP
/// removes the room-entry Strength here (#3044,
/// `engine::damage::red_skull_after_player_hp_changed`). The removal is a
/// negative application, which Ruined Helmet never doubles.
fn pantograph_before_combat_start(
    catalog: &crate::catalog::Catalog,
    state: &mut crate::hot::HotState,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), OpeningRefusal> {
    if state.hp <= 0 || !catalog.hooks().owns(crate::ids::RelicId::RelicPantograph) {
        return Ok(());
    }
    let hp_before = state.hp;
    state.hp = state
        .hp
        .checked_add(PANTOGRAPH_HEAL)
        .ok_or(crate::engine::EngineRefusal::CounterOverflow(
            "Pantograph heal",
        ))?
        .min(state.max_hp);
    crate::coverage::record_relic(crate::ids::RelicId::RelicPantograph);
    crate::engine::damage::red_skull_after_player_hp_changed(
        state,
        hp_before,
        state.max_hp,
        events,
    )?;
    Ok(())
}

/// `Planisphere`'s heal amount: `get_CanonicalVars` (RVA `0x99686`) is
/// `ldc.i4.5; newobj HealVar::.ctor` at `IL_0001`-`IL_0007`.
const PLANISPHERE_HEAL: i32 = 5;

/// `Planisphere`'s `?`-point heal on entering the combat room (#3162).
///
/// The gate is [`planisphere_heals_here`] plus the first-room shape
/// [`build_pre_hook`] checks; this applies `CreatureCmd::Heal(Owner.Creature,
/// 5, true)` (`Planisphere/<AfterRoomEntered>d__5::MoveNext` `0x32e318`,
/// `IL_0078`-`IL_009a`) with the owner-heal clamp `min(max_hp, hp + 5)`.
///
/// The entry save's HP does NOT carry it: the save is written on reaching the
/// point, before the room's `AfterRoomEntered`. The one corpus fight of this
/// shape (MFKRHBXBVZ7V node 21, A10, `?` point into `CHOMPERS_NORMAL`, saved at
/// 42/87) matches its `.mcr` opening checkpoint and replays `lockstep_ok` with
/// the heal and mismatches the checkpoint without it.
///
/// Position: before [`crate::engine::fire_after_room_entered`], after the
/// room-entry Strength block. The represented `AfterRoomEntered` peers
/// (Vajra, Sling, Girya, Ember Tea, Stone Cracker, Ghost Seed, the compiled
/// templates) write Strength, card upgrades or keywords; none reads or writes
/// the player's HP except template heals, and owner heals commute
/// (`min(max, min(max, h + a) + b) = min(max, h + a + b)` for `a, b >= 0`).
fn planisphere_after_room_entered(
    catalog: &crate::catalog::Catalog,
    state: &mut crate::hot::HotState,
) -> Result<(), OpeningRefusal> {
    if state.hp <= 0 || !catalog.hooks().owns(crate::ids::RelicId::RelicPlanisphere) {
        return Ok(());
    }
    state.hp = state
        .hp
        .checked_add(PLANISPHERE_HEAL)
        .ok_or(crate::engine::EngineRefusal::CounterOverflow(
            "Planisphere heal",
        ))?
        .min(state.max_hp);
    crate::coverage::record_relic(crate::ids::RelicId::RelicPlanisphere);
    Ok(())
}

/// Whether `Planisphere`'s room-entry heal can fire at this node (#3162).
///
/// v0.111.0 `sts2.dll` (sha256 `9cb4f1ad…fbf12b4`, hash-verified 2026-09-26).
/// The type declares one hook, `AfterRoomEntered` (RVA `0x99698`, the async
/// stub), beside `get_Rarity`, `IsAllowed` and `get_CanonicalVars` (`0x99686`:
/// `ldc.i4.5; newobj HealVar::.ctor`, `IL_0001`-`IL_0007`). It has no fields
/// and no saved property. The body is
/// `Planisphere/<AfterRoomEntered>d__5::MoveNext` (RVA `0x32e318`):
///
/// * `Owner.Creature.IsDead`, else `leave` (`IL_0020`-`IL_0032`);
/// * `RunState.CurrentMapPoint?.PointType == 1`, else `leave`
///   (`IL_0037`-`IL_005b`; a null point also leaves, via `ldc.i4.1` at
///   `IL_004b`). `MapPointType` declares `Unassigned = 0, Unknown = 1, Shop,
///   Treasure, RestSite, Monster = 5, Elite, Boss, Ancient` (read from the
///   `Constant` table), so this is the **`?` point** — not the room the point
///   resolved into;
/// * `RunState.CurrentRoomCount <= 1`, else `leave` (`IL_0060`-`IL_0073`);
/// * `Flash`, then `CreatureCmd::Heal(Owner.Creature,
///   DynamicVars.Heal.BaseValue, true)` at `IL_0078`-`IL_009a`.
///
/// So on every point but `unknown` the body returns before it writes
/// anything: the heal is inert there and the relic needs no subscriber. On an
/// `unknown` point the heal is live when the combat is the point's first room
/// (`CurrentRoomCount <= 1`). [`build_pre_hook`] admits exactly one shape of
/// that: the fight is the act's next scheduled normal encounter
/// (`entry.next_normal_encounter`), so the `?` point resolved straight into its
/// `MonsterRoom` rather than into an event that later started a fight, and
/// [`planisphere_after_room_entered`] applies the heal. Every other `unknown`
/// point, and a point the entry does not name (`None`), stays the named
/// [`OpeningRefusal::RoomEntryHealUnmodeled`] — with one exception: a `boss`
/// room with no named point is inert, because a boss room is entered only
/// from the act's `BossMapPoint` (`MapPointType.Boss`), whereas a `?` point
/// can resolve into a monster or an elite room (`SerializableRunOddsSet`'s
/// `UnknownMapPointMonsterOddsValue` / `…EliteOddsValue`), so neither of
/// those room kinds names the point. A facts input written from a `.run`
/// (the Coach's and the legacy review's roots) carries no point.
///
/// The gate read `entry.node_type == "monster"` until #3162, which is the room
/// kind: it refused the one point type the body is certainly inert on (every
/// one of the corpus's 27 refused `monster`-point fights) and admitted a `?`
/// combat, which the eval census and the review both name `"monster"` too.
fn planisphere_heals_here(map_point_type: Option<&str>, node_type: Option<&str>) -> bool {
    if map_point_type.is_none() && node_type == Some("boss") {
        return false;
    }
    !matches!(
        map_point_type,
        Some(
            "unassigned"
                | "shop"
                | "treasure"
                | "rest_site"
                | "monster"
                | "elite"
                | "boss"
                | "ancient"
        )
    )
}

/// The room-entry Strength block, between admission and the
/// `AfterRoomEntered` template pass (frozen Python `start_combat`, deleted #2827).
///
/// Vajra, Sling of Courage, Girya and Ember Tea act here — Sword of Jade's
/// grant is template-owned ([`ruined_helmet_room_entry_strength_is_ordered`]).
/// They run in the order of the oracle's `room_entry_strength_sources`
/// (frozen Python `start_combat`, deleted #2827; applied in `start_combat`): Vajra, then Sling
/// (elite nodes only, [`SLING_OF_COURAGE_STRENGTH`]), then Girya, then Ember
/// Tea. The game
/// runs the group in unrecorded acquisition order, which is observable only
/// through Ruined Helmet's first-positive doubling, and that gate refuses
/// distinct amounts. Vajra's body is [`VAJRA_STRENGTH`] and Girya's is its
/// saved lift count, `girya_lifts` (see [`OPENING_WINDOW_RELIC_BODIES`] for
/// both IL reads); a Girya with no lifts `leave`s before applying anything
/// (`Girya/<AfterRoomEntered>d__14::MoveNext` `IL_0027`-`IL_0029`).
/// Ember Tea's order is the game's:
/// `EmberTea/<AfterRoomEntered>d__17::MoveNext` (v0.111.0 RVA `0x3239cc`)
/// returns immediately when `IsUsedUp` (`0x92eca`: `CombatsLeft > 0` is false)
/// at `IL_001e`, `Apply<StrengthPower>` at `IL_0065` with
/// `DynamicVars.Strength.BaseValue` ([`EMBER_TEA_STRENGTH`]), and only after
/// that awaited command completes does `IL_00bd-IL_00c8` call
/// `set_CombatsLeft(CombatsLeft - 1)`. Strength cannot end combat, so the
/// public state always observes the completed consumption.
///
/// Red Skull's room-entry Strength is the same kind of grant, applied by
/// [`red_skull_after_room_entered`] after Planisphere's heal (#3044).
///
/// The grant goes through [`crate::engine::damage::apply_owner_strength`] and
/// not through a direct write, so Ruined Helmet's first-positive doubling and
/// its latch are the engine's one implementation rather than a second copy.
fn apply_room_entry_strength(
    catalog: &crate::catalog::Catalog,
    sling_elite: bool,
    girya_lifts: i32,
    state: &mut crate::hot::HotState,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), OpeningRefusal> {
    if catalog.hooks().owns(crate::ids::RelicId::RelicVajra) {
        crate::engine::damage::apply_owner_strength(state, VAJRA_STRENGTH, events)?;
    }
    if sling_elite
        && catalog
            .hooks()
            .owns(crate::ids::RelicId::RelicSlingOfCourage)
    {
        crate::engine::damage::apply_owner_strength(state, SLING_OF_COURAGE_STRENGTH, events)?;
    }
    if girya_lifts > 0 && catalog.hooks().owns(crate::ids::RelicId::RelicGirya) {
        crate::engine::damage::apply_owner_strength(state, girya_lifts, events)?;
    }
    let combats_left = state.ember_tea_combats_left();
    if combats_left > 0 {
        crate::engine::damage::apply_owner_strength(state, EMBER_TEA_STRENGTH, events)?;
        assert!(
            state.set_ember_tea_combats_left(combats_left - 1),
            "a positive charge always decrements into the representable domain"
        );
    }
    Ok(())
}

/// Whether the recorded profile reveals every epoch that gates a row of
/// `character`'s own `CharacterCardPool` (#3336).
///
/// `CardPoolModel::GetUnlockedCards` (RVA `0x7e54c`) calls the pool's virtual
/// `FilterThroughEpochs` at IL_0014, and each character override (Ironclad
/// `0xf1f98`, Silent `0xf2cf8`, Regent `0xf28e0`, Necrobinder `0xf2468`,
/// Defect `0xf1944`) tests exactly three `IsEpochRevealed` epochs of its OWN
/// character (IL_0014, IL_0042, IL_0070). Those are the per-row `unlock_epoch`
/// column of the generated `CHARACTER_CARD_POOL_ROWS_V1101`, which this reads
/// rather than lists. No other character's epoch, and no Colorless epoch, can
/// move a row of this pool. `false` without a profile or for a character with
/// no pool.
///
/// For Ironclad the three are `IRONCLAD2/5/7_EPOCH`
/// (`owner_card_pool_gating_epochs_are_each_owners_own_three` pins it), which
/// is exactly what the entry's `fully_unlocked_card_pool` answers: a save
/// derives it from those three epochs
/// (`entry::unlocks::infernal_blade_pool_fully_unlocked`) and a facts document
/// states it. So the Ironclad answer IS that field, unchanged, and only the
/// other owners, whose pools that field never described, derive theirs here.
fn owner_card_pool_epochs_revealed(entry: &EntryDocument, character: Option<&str>) -> bool {
    if character == Some(INFERNAL_BLADE_FULL_POOL_OWNER) {
        return entry.fully_unlocked_card_pool == Some(true);
    }
    let Some(name) = character.and_then(|c| c.strip_prefix("CHARACTER.")) else {
        return false;
    };
    let Some(epochs) = normalized_card_pool_epochs(entry).filter(|e| !e.is_empty()) else {
        return false;
    };
    let Some((_, rows)) = crate::content_tables::CHARACTER_CARD_POOL_ROWS_V1101
        .iter()
        .find(|(pool, _)| *pool == name)
    else {
        return false;
    };
    rows.iter()
        .filter_map(|(_, gate)| *gate)
        .all(|gate| epochs.iter().any(|have| have == gate))
}

/// The owner-pool generators' profile gate, per source's pool (#3336).
///
/// The oracle (`legacy_full_pool_sources`, frozen Python `start_combat`,
/// deleted #2827) demanded the save-level `fully_unlocked_card_pool` answer,
/// the Ironclad 2/5/7 epochs, for every source but Splash, whatever the owner
/// and whatever pool the source reads. That tested the wrong epochs for every
/// source below except Infernal Blade: a Defect Discovery refused under a
/// profile hiding `IRONCLAD7_EPOCH` (which cannot move a Defect row) and
/// opened under one hiding `DEFECT7_EPOCH` (which does). Each source now gets
/// the check its native pool and its engine body need:
///
/// * **Infernal Blade**: the Ironclad owner's pool fully revealed
///   ([`owner_card_pool_epochs_revealed`]), because its engine body shuffles a
///   frozen fully-unlocked Ironclad pool (see
///   [`IRONCLAD_ONLY_GENERATION_CARDS`] for the IL).
/// * **The owner's `CharacterCardPool` under `Owner.UnlockState`**: Discovery
///   (`Discovery/<OnPlay>d__4::MoveNext` `0x399254` IL_003e-IL_0063),
///   Jackpot (`Jackpot/<OnPlay>d__3::MoveNext` `0x3a80d4` IL_00e6-IL_010b),
///   Calamity (`CalamityPower/<AfterCardPlayed>d__7::MoveNext` `0x3368bc`
///   IL_005a-IL_0089), Chaos Mad Science
///   (`MadScience/<ExecuteRider>d__57::MoveNext` `0x3aa9c4`
///   IL_033b-IL_0360) and Stoke (`0x3bef44` IL_0194-IL_01b9). Their engine
///   bodies draw `Catalog::owner_generation_pool` (or its zero-cost / Attack
///   projections) filtered by the recorded profile, under
///   `engine::cards::owner_pool_generation_provenance_is_exact`, so what
///   they need is a RECORDED profile, the one the opening then writes as
///   `splash_unlock_epochs`.
/// * **The static Colorless pool under `Owner.UnlockState`**: Jack of All
///   Trades (`JackOfAllTrades/<OnPlay>d__6::MoveNext` `0x3a7ecc`:
///   `CardPool<ColorlessCardPool>` MethodSpec `0x2b0007c2` at IL_0026,
///   `Owner.UnlockState` IL_0031, `GetUnlockedCards` IL_0046,
///   `GetDistinctForCombat` IL_0094). No character epoch can move its pool;
///   its body shuffles `Catalog::jack_of_all_trades_pool` under
///   `engine::cards::jack_of_all_trades_provenance_is_exact`, so it too needs
///   a recorded profile and nothing more.
/// * **Splash**: unchanged ([`PARTIAL_UNLOCK_TOLERANT_GENERATION_CARDS`]).
///
/// A recorded profile is the same non-empty normalized epoch set that
/// [`listener_pool_provenance_is_recorded`] and
/// [`toolbox_provenance_is_recorded`] require. The refusal names the sources
/// that failed, Infernal Blade first when both fail.
fn owner_pool_generation_profile_is_recorded(
    entry: &EntryDocument,
    sources: &[&str],
) -> Result<(), OpeningRefusal> {
    let profile_recorded = normalized_card_pool_epochs(entry).is_some_and(|e| !e.is_empty());
    let infernal_blade_pool_revealed =
        owner_card_pool_epochs_revealed(entry, Some(INFERNAL_BLADE_FULL_POOL_OWNER));
    let failing: Vec<&str> = sources
        .iter()
        .copied()
        .filter(|id| !PARTIAL_UNLOCK_TOLERANT_GENERATION_CARDS.contains(id))
        .filter(|id| {
            if IRONCLAD_ONLY_GENERATION_CARDS.contains(id) {
                !infernal_blade_pool_revealed
            } else {
                !profile_recorded
            }
        })
        .collect();
    if failing.is_empty() {
        return Ok(());
    }
    Err(OpeningRefusal::GenerationPoolPartialUnlock {
        source: failing.join("/"),
        missing: entry.unlocked_card_pool_epochs.clone().unwrap_or_default(),
    })
}

/// The owner and profile half of the three listener-card gates, in the
/// oracle's order (Hello World, Call of the Void, Creative AI; owner before
/// profile for each; frozen Python `start_combat`, deleted #2827).
///
/// **Owner (#3375).** The oracle modeled each card only in its own
/// character's deck and demanded that owner by name (`CHARACTER.DEFECT` for
/// Hello World and Creative AI, `CHARACTER.NECROBINDER` for Call of the
/// Void). Native never names a character. Current v0.111.0 IL (`sts2.dll`
/// SHA-256 `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`):
///
/// * `CreativeAiPower/<BeforeHandDraw>d__4::MoveNext` RVA `0x338180` tests
///   only that the hook's `player` is `Owner.Player` (IL_0020-IL_0031), then
///   reads `player.Character.CardPool` (IL_0044-IL_0055), `player.UnlockState`
///   (IL_005a-IL_0060) and `RunState.CardMultiplayerConstraint`
///   (IL_0065-IL_0070) into `CardPoolModel::GetUnlockedCards` (IL_0075),
///   keeps `Type == Power` (`Where` at IL_0099 over the predicate
///   `<>c::<BeforeHandDraw>b__4_0` RVA `0x338172`: `get_Type; ldc.i4.3;
///   ceq`) and calls `CardFactory::GetDistinctForCombat(.., 1,
///   CombatCardGeneration)` (IL_009e-IL_00b4);
/// * `HelloWorldPower/<BeforeHandDraw>d__4::MoveNext` RVA `0x33bfd0`: the
///   same owner-pool read at IL_0062-IL_0091 (`Rarity == Common`, predicate
///   RVA `0x33bfc2`), `GetDistinctForCombat` over `CombatCardGeneration` at
///   IL_00d5-IL_00da;
/// * `CallOfTheVoidPower/<BeforeHandDraw>d__6::MoveNext` RVA `0x336b08`: the
///   same owner-pool read at IL_0043-IL_0068 (predicate RVA `0x336ae0`
///   excludes Basic and Ancient), `GetDistinctForCombat` over
///   `CombatCardGeneration` at IL_00c6-IL_00dc;
/// * the source card's own body (`CreativeAi/<OnPlay>d__4::MoveNext` RVA
///   `0x395684`) reads `Player.Character` only for `PowerUpAnimDelay`
///   (IL_003a-IL_003f), a presentation delay.
///
/// So each pool is the OWNER's character pool whatever the character is, and
/// [`crate::steps::neutral::owner_listener_pool`] derives it for every
/// [`RewardPool`] under the recorded profile — the derivation `HotBoundary`
/// admits the written field against and the engine listener
/// (`engine::cards::exact_owner_listener_pool`) draws from. The owner
/// requirement is therefore any character the opening writes a
/// `reward_card_pool` for ([`listener_pool_owner`]); a fight without one
/// still refuses by name.
fn listener_pool_provenance_is_recorded(
    entry: &EntryDocument,
    deck_ids: &BTreeSet<&str>,
) -> Result<(), OpeningRefusal> {
    let profile_recorded = normalized_card_pool_epochs(entry).is_some_and(|e| !e.is_empty());
    for (card, ..) in OWNER_LISTENER_POOL_FIELDS {
        if !deck_ids.contains(card) {
            continue;
        }
        if listener_pool_owner(entry).is_none() {
            return Err(OpeningRefusal::ListenerPoolProvenance {
                source: card,
                requires: "an explicit character owner with a card pool",
            });
        }
        if !profile_recorded {
            return Err(OpeningRefusal::ListenerPoolProvenance {
                source: card,
                requires: "a recorded card-pool unlock profile",
            });
        }
    }
    Ok(())
}

/// The owner pool the three listeners draw from: the fight's character's own
/// `RewardPool`, the value [`reward_card_pool`] writes and `HotBoundary`
/// derives the admitted pool from (#3375).
fn listener_pool_owner(entry: &EntryDocument) -> Option<RewardPool> {
    reward_card_pool(entry.character.as_deref()).and_then(RewardPool::from_str)
}

/// The stream half of the same gates. Each listener card needs an exact
/// `CombatCardGeneration` counter (frozen Python `start_combat`, deleted #2827); Creative AI and Call of the Void also reach random-orb
/// consumers through their generated pools and need `CombatOrbGeneration`
/// (`orb_generation_consumers`, `start_combat`). `stream_states` carries an
/// absent optional stream at counter 0, which is right for a fight that never
/// consumes it and wrong for these, so the absence is refused here by name.
fn listener_pool_streams_are_recorded(
    entry: &EntryDocument,
    deck_ids: &BTreeSet<&str>,
) -> Result<(), OpeningRefusal> {
    for (card, ..) in OWNER_LISTENER_POOL_FIELDS {
        if deck_ids.contains(card) && !entry.streams.contains_key("combat_card_generation") {
            return Err(OpeningRefusal::CombatStreamAbsent {
                stream: "combat_card_generation",
            });
        }
    }
    if (deck_ids.contains("CREATIVE_AI") || deck_ids.contains("CALL_OF_THE_VOID"))
        && !entry.streams.contains_key("combat_orbs")
    {
        return Err(OpeningRefusal::CombatStreamAbsent {
            stream: "combat_orbs",
        });
    }
    Ok(())
}

/// The generation relics' two stream gates (#2827), the relic half of #2931.
///
/// The oracle's `generation_relic_sources` are the owned members of
/// `_GENERATION_RELIC_POOLS` whose pool for this owner is non-empty
/// (frozen Python `start_combat`, deleted #2827). It requires an exact `CombatCardGeneration`
/// counter for every one of them but Big Hat
/// (`_ACTIVE_IRONCLAD_GENERATION_RELIC_NAMES`, `start_combat`), and, because a
/// generated owner pool can reach a random-orb consumer, a recorded
/// `CombatOrbGeneration` counter for all of them (`orb_generation_consumers`'s
/// *"generation-relic card pools"*, `start_combat`), in that order.
/// [`stream_states`] carries an absent optional stream at counter 0, which
/// Crossbow's and the Puzzlebox's turn-one draws would then consume silently
/// (a full-profile Ironclad Crossbow did exactly that before this gate), so
/// each absence refuses by name, as the oracle does.
///
/// Big Hat's pool is its owner's Ethereal cards: empty for an Ironclad, so an
/// Ironclad Big Hat is no source and needs neither stream (the oracle roots
/// it without `combat_orbs`). For any other owner it counts here; that is
/// stricter than the oracle only for a Silent, whose Ethereal pool is also
/// empty, and a non-Ironclad Big Hat refuses in the engine anyway (#3264).
fn generation_relic_streams_are_recorded(
    entry: &EntryDocument,
    relics: &BTreeSet<&str>,
) -> Result<(), OpeningRefusal> {
    // (relic, reads `CombatCardGeneration`)
    const SOURCES: [(&str, bool); 6] = [
        ("RELIC.BIG_HAT", false),
        ("RELIC.CHOICES_PARADOX", true),
        ("RELIC.CROSSBOW", true),
        ("RELIC.ORANGE_DOUGH", true),
        ("RELIC.TOOLBOX", true),
        ("RELIC.VEXING_PUZZLEBOX", true),
    ];
    let ironclad = entry.character.as_deref() == Some("CHARACTER.IRONCLAD");
    let owned: Vec<bool> = SOURCES
        .into_iter()
        .filter(|(relic, _)| relics.contains(relic))
        .filter(|(relic, _)| !(ironclad && *relic == "RELIC.BIG_HAT"))
        .map(|(_, reads_generation)| reads_generation)
        .collect();
    if owned.is_empty() {
        return Ok(());
    }
    if owned.contains(&true) && !entry.streams.contains_key("combat_card_generation") {
        return Err(OpeningRefusal::CombatStreamAbsent {
            stream: "combat_card_generation",
        });
    }
    if !entry.streams.contains_key("combat_orbs") {
        return Err(OpeningRefusal::CombatStreamAbsent {
            stream: "combat_orbs",
        });
    }
    Ok(())
}

/// The owner-pool generation sources in this deck, in the oracle's order
/// (`sorted(deck_ids & {...})`).
fn owner_pool_generation_sources<'a>(deck_ids: &BTreeSet<&'a str>) -> Vec<&'a str> {
    deck_ids
        .iter()
        .copied()
        .filter(|id| OWNER_POOL_GENERATION_CARDS.contains(id))
        .collect()
}

/// `seeded(rid, modulus)` (frozen Python `start_combat`, deleted #2827) plus the two exact
/// counters Iron Club and Tuning Fork require.
fn seeded_counters(
    entry: &EntryDocument,
    relics: &BTreeSet<&str>,
) -> Result<BTreeMap<&'static str, i64>, OpeningRefusal> {
    let counters = &entry.relic_entry.relic_counters;
    let mut out = BTreeMap::new();
    for (relic, modulus) in SEEDED_COUNTER_RELICS {
        if !relics.contains(relic) {
            continue;
        }
        let value = counters
            .get(relic)
            .and_then(Value::as_i64)
            .ok_or(OpeningRefusal::RelicCounterUnseeded { relic })?;
        let field = SEEDED_COUNTER_FIELDS
            .iter()
            .find_map(|(id, field)| (*id == relic).then_some(*field))
            .expect("every seeded counter relic names its canonical field");
        out.insert(field, value.rem_euclid(modulus));
    }
    // `iron_club_cards = saved % IRON_CLUB_CARDS` after an exactness check
    // (see [`IRON_CLUB_CARDS`] for the IL: the saved `CardsPlayed` is a
    // lifetime count that native never resets, so only its residue modulo
    // the canonical `Cards` 4 is combat state);
    // `tuning_fork_skills` keeps the exact saved value, because the native
    // subtraction happens only after the awaited block command and subtracts
    // exactly one threshold. (#3228 audit: `TuningFork::get_CanonicalVars`,
    // RVA `0x9d149`, builds `Cards` from `ldc.i4.s 10` at `IL_0009`, which
    // `get_SkillsThreshold` (`0x9d1b2`) returns;
    // `TuningFork/<AfterCardPlayed>d__22::MoveNext` (RVA `0x3334b8`) blocks
    // once `SkillsPlayed >= SkillsThreshold` (`IL_0065`-`IL_0071`) and stores
    // `SkillsPlayed - SkillsThreshold` at `IL_00f2`-`IL_0100`; the engine's
    // `next >= 10` / `next - 10` is the same.)
    //
    // `pumpkin_candle_kindle_count` is `exact_finite_counter("RELIC.PUMPKIN_CANDLE")`
    // (frozen Python `start_combat`, deleted #2827; written in `start_combat`), the saved value unchanged
    // (#2847). `PumpkinCandle` declares no combat-start hook: `KindleCount` is
    // written only by `Rekindle` (`0x99ef8`, from `AfterObtained` and the rest
    // site) and `AfterCombatEnd` (`0x99ec3`), and `ModifyMaxEnergy`
    // (`0x99e90`) reads it at `IL_000d`, which `engine::relics::modifier_total`
    // ports. Before this seed an owner reached `HotBoundary` with the default
    // `-1` and refused as an ownership/state disagreement.
    //
    // `bone_tea_combats_left` is `exact_finite_counter("RELIC.BONE_TEA", 1)`
    // (frozen Python `start_combat`, deleted #2827; written in `start_combat`), the saved `CombatsLeft`
    // unchanged (#2884, reached by #2827's freed fights). v0.111.0 IL:
    // `CombatsLeft` is the relic's one own `[SavedProperty]`;
    // `BoneTea::.ctor` (RVA `0x910a8`) starts it at 1 (`IL_0002`), and its
    // only writer is `BoneTea::AfterPlayerTurnStart` (RVA `0x90ffc`), which
    // returns early while `IsUsedUp` (`CombatsLeft <= 0`, `get_IsUsedUp`
    // `0x90f68`) at `IL_000d`-`IL_0012`, and otherwise, on the owner's turn
    // one (`IL_0029`-`IL_003a`), upgrades each Hand card (`IL_0042`-`IL_0064`)
    // and stores `CombatsLeft - 1` (`IL_007d`-`IL_0088`). So the domain is
    // exactly `0..=1`, and the charge a combat begins with is the saved one.
    // The body that spends it is `engine::relics::continue_after_toasty_mittens`,
    // reached through the opening's turn-1 walk. Before this seed the fanout
    // stayed 0, so a charged Bone Tea's Hand upgrade was silently dropped.
    for (relic, field, modulus, maximum) in [
        (
            "RELIC.IRON_CLUB",
            "iron_club_cards",
            Some(IRON_CLUB_CARDS),
            None,
        ),
        ("RELIC.TUNING_FORK", "tuning_fork_skills", None, None),
        (
            "RELIC.PUMPKIN_CANDLE",
            "pumpkin_candle_kindle_count",
            None,
            None,
        ),
        ("RELIC.BONE_TEA", "bone_tea_combats_left", None, Some(1_i64)),
    ] {
        if !relics.contains(relic) {
            continue;
        }
        let raw = counters.get(relic);
        let value = raw
            .and_then(Value::as_i64)
            .filter(|value| *value >= 0 && maximum.is_none_or(|maximum| *value <= maximum))
            .ok_or(OpeningRefusal::RelicCounterNotExact {
                relic,
                value: raw.map_or_else(|| "absent".to_string(), ToString::to_string),
            })?;
        out.insert(field, modulus.map_or(value, |m| value.rem_euclid(m)));
    }
    // `joss_paper_cards_exhausted` (default `-1`, the "not owned" sentinel the
    // boundary's Batch 9 check pairs with ownership) is the saved
    // `CardsExhausted` unchanged, and `joss_paper_ethereal_count` is the saved
    // `EtherealCount`, which must be `0` — the oracle's gate (frozen Python `start_combat`, deleted #2827), written in `start_combat` (#2827).
    //
    // v0.111.0 IL: `JossPaper` declares no combat-start hook — its hooks are
    // `AfterCardExhausted` (RVA `0x95434`), `AfterSideTurnEnd` (`0x95490`)
    // and `AfterCombatEnd` (`0x95573`), which clears `EtherealCount` — so the
    // combat begins with the saved counter. `DrawIfThresholdMet`
    // (`<DrawIfThresholdMet>d__27::MoveNext`, RVA `0x327888`) folds the
    // counter below `ExhaustAmount` 5 (`get_CanonicalVars`, RVA `0x9540b`,
    // `IL_000e`) whenever it is reached, which is the oracle's `0..=4`
    // domain; anything else refuses rather than being folded here. The save
    // adapter (`entry::relics::joss_paper_properties`) has already restored
    // the `SaveIfNotTypeDefault` zeros. Before this seed an owner reached
    // `HotBoundary` with the `-1` default and refused as "Batch 9 relic
    // ownership and mutable state disagree".
    if relics.contains("RELIC.JOSS_PAPER") {
        let raw = counters.get("RELIC.JOSS_PAPER");
        let exact = raw.and_then(Value::as_object).and_then(|bag| {
            let exhausted = bag.get("CardsExhausted").and_then(Value::as_i64)?;
            let ethereal = bag.get("EtherealCount").and_then(Value::as_i64)?;
            (bag.len() == 2 && (0..JOSS_PAPER_EXHAUSTS).contains(&exhausted) && ethereal == 0)
                .then_some(exhausted)
        });
        let exhausted = exact.ok_or(OpeningRefusal::RelicCounterNotExact {
            relic: "RELIC.JOSS_PAPER",
            value: raw.map_or_else(|| "absent".to_string(), ToString::to_string),
        })?;
        out.insert("joss_paper_cards_exhausted", exhausted);
    }
    Ok(out)
}

/// `IronClub`'s `Cards` canonical var, the period of its draw.
///
/// v0.111.0 IL (DLL sha256 `9cb4f1ad…`): `IronClub::get_CanonicalVars` (RVA
/// `0x950c2`) builds its one `CardsVar` from `ldc.i4.4` at `IL_0001`. The
/// body, `IronClub/<AfterCardPlayed>d__19::MoveNext` (RVA `0x326f78`),
/// increments the saved `CardsPlayed` on every owner card play
/// (`IL_003d`-`IL_0048`) and draws one when `CardsPlayed % Cards == 0`
/// (`IL_006d`-`IL_0075`, draw at `IL_0095`). Nothing resets it — the type
/// declares no `AfterCombatEnd`/`BeforeCombatStart`, and `get_DisplayAmount`
/// (`0x95090`) and `UpdateDisplay` (`0x9510c`) also read `CardsPlayed % Cards`
/// — so the saved value is a lifetime count (the corpus carries 35, 87 and
/// 147) and the combat's phase is its residue modulo 4, which is what
/// `engine::relics::iron_club_after_card_played` counts. This read 3 until
/// #3228: a saved 35/87/147 opened as 2/0/0 instead of 3/3/3, and the draw
/// on the first card play was silently missed.
const IRON_CLUB_CARDS: i64 = 4;

/// `JossPaper`'s `ExhaustAmount` canonical var: `get_CanonicalVars` (RVA
/// `0x9540b`) constructs it from `ldc.i4.5` at `IL_000e`
/// (`combat_sim.JOSS_PAPER_EXHAUSTS`, frozen Python, deleted #2827).
const JOSS_PAPER_EXHAUSTS: i64 = 5;

/// The oracle's four `start_combat` refusals for Symbiotic Virus's turn-1
/// Dark channel (frozen Python, deleted #2827), by the same conditions (#2827).
///
/// * Infused Core, at any capacity: the two relics' channels interleave by
///   acquisition order, which decides which orb overflows.
/// * Runic Capacitor at `BaseOrbSlotCount` 0: whether the Dark channel
///   bootstraps a one-slot queue before or after `AddSlots(3)` makes the final
///   capacity 4 or 3.
/// * Cracked Core at `BaseOrbSlotCount` 0 with Fencing Manual or Brimstone:
///   Cracked Core's Lightning fills the bootstrapped slot, so the Dark channel
///   auto-evokes it, and that hit can land before or after Forge or
///   Brimstone's live-opponent snapshot.
///
/// `engine::admission` refuses the same four for a loaded document
/// (`Symbiotic Virus + Infused Core turn-start order`, `… + Runic Capacitor
/// zero-slot order`, `… + Cracked Core zero-slot turn-start order`), but the
/// opening builds its document without running admission, so the refusal has
/// to be here too. Infused Core and Runic Capacitor are still gated above
/// this call, so today only the Cracked Core arm is reachable (with either
/// Brimstone or, since #3090 retired its gate, Fencing Manual); the other two
/// are spelled so that retiring those gates cannot silently open a
/// combination the oracle refuses.
fn symbiotic_virus_turn_start_order_is_recorded(
    relics: &BTreeSet<&str>,
    base_slots: i64,
) -> Result<(), OpeningRefusal> {
    if !relics.contains("RELIC.SYMBIOTIC_VIRUS") {
        return Ok(());
    }
    let owns = |relic: &str| relics.contains(relic);
    let peers: Vec<&str> = if owns("RELIC.INFUSED_CORE") {
        vec!["RELIC.INFUSED_CORE"]
    } else if base_slots == 0 && owns("RELIC.RUNIC_CAPACITOR") {
        vec!["RELIC.RUNIC_CAPACITOR"]
    } else if base_slots == 0 && owns("RELIC.CRACKED_CORE") {
        match ["RELIC.FENCING_MANUAL", "RELIC.BRIMSTONE"]
            .into_iter()
            .find(|peer| owns(peer))
        {
            Some(peer) => vec!["RELIC.CRACKED_CORE", peer],
            None => Vec::new(),
        }
    } else {
        Vec::new()
    };
    if peers.is_empty() {
        return Ok(());
    }
    Err(OpeningRefusal::SymbioticVirusTurnStartOrder {
        peers: peers.into_iter().map(ToString::to_string).collect(),
    })
}

/// Part C4's obligation, discharged rather than inherited.
///
/// Python refuses a counter relic (`IRON_CLUB` / `TUNING_FORK`) whose recorded
/// order places a same-`AfterCardPlayed` peer *after* it, because its own
/// dispatch hardcodes the two counters into a fixed suffix
/// (`_after_card_played_counter_relics`, gate (frozen Python `start_combat`, deleted #2827)). That is a fact about the **oracle's**
/// implementation, not about the game, and Part C4 forbids transcribing it.
///
/// This port has no such suffix: [`crate::engine::fire_hook`] walks
/// relic-template subscribers in authenticated inventory order, which
/// `CombatState::IterateHookListeners` (`0x137409`, body `0x3f9720`) enumerates
/// as `Player.Relics` by ascending index. So where the order **is** recorded,
/// dispatch is exact by construction, and any relic whose body this engine does
/// not model refuses at the fire point by its own name
/// (`EngineRefusal::HookNotModeled`) rather than silently firing in the wrong
/// place.
///
/// What remains genuinely unprovable is a counter relic on an entry whose
/// inventory order is **not** vouched for: `relics_entering` is then just a
/// list, and the counters' position among their peers is a guess. That is the
/// one case this gate refuses, and it names the peers it could not place.
fn counter_relic_dispatch_is_representable(
    entry: &EntryDocument,
    relics: &BTreeSet<&str>,
) -> Result<(), OpeningRefusal> {
    const COUNTERS: [&str; 2] = ["RELIC.IRON_CLUB", "RELIC.TUNING_FORK"];
    if entry.relic_entry.dispatch_ordered {
        return Ok(());
    }
    if !COUNTERS.into_iter().any(|relic| relics.contains(relic)) {
        return Ok(());
    }
    let peers: Vec<String> = relics
        .iter()
        .copied()
        .filter(|relic| !COUNTERS.contains(relic))
        .map(ToString::to_string)
        .collect();
    if peers.is_empty() {
        return Ok(());
    }
    Err(OpeningRefusal::RelicDispatchOrderUnmodeled {
        hook: "AfterCardPlayed",
        relics: peers,
    })
}

/// `start_combat`'s same-`AfterPlayerTurnStart` refusals, for the two relics
/// whose bodies #2827 modeled (`RELIC.BELLOWS`, `RELIC.FESTIVE_POPPER`).
///
/// Both engines run the hook's relic bodies as one **fixed** suffix —
/// Choices Paradox, Gambling Chip, Vexing Puzzlebox, Toasty Mittens, then
/// Bellows, Bone Tea, Festive Popper, Mr Struggles, Royal Poison
/// (`combat_sim._continue_after_toasty_mittens` (frozen Python, deleted #2827)), then Emotion
/// Chip (`_turn_start_after_royal`), mirrored by
/// `engine::relics::continue_after_toasty_mittens` — while native awaits
/// the listeners in `Player.Relics` order, which the run payload does not
/// vouch for. The oracle refuses every co-ownership where that order is
/// observable, and this gate refuses the ones that touch the two new bodies
/// by the same rules:
///
/// * `RELIC.GAMBLING_CHIP` with **any** same-hook peer (frozen Python `start_combat`, deleted #2827);
/// * `RELIC.TOASTY_MITTENS` with any peer of
///   `_V1101_TOASTY_AFTER_PLAYER_TURN_START_PEERS` (`start_combat`), whose
///   only exception is an exact recorded Vexing Puzzlebox → Toasty Mittens
///   pair — a pair Bellows or Popper would break;
/// * the generation sources `RELIC.CHOICES_PARADOX` and
///   `RELIC.VEXING_PUZZLEBOX` with any same-hook peer (`start_combat`). The
///   one exception is #1102's recorded Queen trio (Choices Paradox with
///   exactly Festive Popper and Mercury Hourglass), which the rule below
///   refuses anyway: stricter than the oracle, never wrong;
/// * `RELIC.BONE_TEA` with any peer except Bellows (`start_combat`). The
///   oracle gates that on a live charge. This gate refuses Popper beside
///   Bone Tea at any charge, which again only over-refuses;
/// * Festive Popper with a second enemy-damage body, Mr Struggles or the
///   template Mercury Hourglass (`start_combat`), or with Royal Poison
///   (`start_combat`).
///
/// Bellows beside Bone Tea, Mr Struggles, Mercury Hourglass, Royal Poison or
/// Emotion Chip, and Popper beside Emotion Chip, are **admitted** by the
/// oracle, and the fixed suffix is shared, so this gate admits them too. Each
/// was compared digest for digest against the oracle on the fixture save
/// (`fixture_tests`).
///
/// # The two orders this gate reads from the save (#2884)
///
/// Where the inventory is vouched for as dispatch order
/// (`relics_entering_dispatch_ordered`), two of the pairs above no longer need
/// a guess, and every corpus fight that met this refusal holds one of them:
///
/// * **Choices Paradox, then Bellows.**
///   `ChoicesParadox/<AfterPlayerTurnStart>d__6::MoveNext` (v0.111.0 RVA
///   `0x321b0c`) acts on `TurnNumber == 1` (`IL_0045`-`IL_004b`), offers a
///   `CardFactory::GetDistinctForCombat` grid (`IL_00b3`,
///   `CardSelectCmd::FromSimpleGrid` at `IL_0144`) and adds the pick to the
///   Hand (`CardPileCmd::AddGeneratedCardToCombat`, `IL_01d0`).
///   `Bellows::AfterPlayerTurnStart` (RVA `0x906dc`, synchronous) gates
///   `player == Owner` (`IL_000c`-`IL_0013`) and `TurnNumber <= 1`
///   (`IL_001c`-`IL_002c`), then calls `CardCmd::Upgrade` on the whole Hand
///   (`IL_003a`-`IL_004c`). So Bellows upgrades the Paradox card exactly when
///   it runs second, and the two do not commute. The fixed suffix runs Choices
///   Paradox first, so an inventory recording every Choices Paradox before
///   every Bellows is exact and admitted. The reverse, or an unvouched
///   inventory, still refuses by this name. (At the Paradox pause,
///   `engine::relics::after_player_turn_start_pause_order_is_native` would
///   refuse the reverse as well; refusing it here names it at the opening.)
/// * **Festive Popper and Toasty Mittens, in either order.** These do not
///   commute either (`engine::relics::festive_popper_precedes_toasty_mittens`
///   carries the IL), but the engine now runs Popper on its recorded side of
///   Toasty, so a vouched inventory is exact both ways. An unvouched one
///   refuses.
///
/// Both exemptions are pair-local. A third peer still refuses by its own rule,
/// so relaxing one pair never admits a trio the rules above refuse.
///
/// The co-ownerships among *other* same-hook relics that #2884 lists
/// (Gambling Chip or Toasty Mittens beside Mr Struggles) do not run as a
/// silent fixed order. Both relics pause the walk on a Hand choice, and at that
/// pause `engine::relics::after_player_turn_start_pause_order_is_native`
/// requires a vouched inventory that agrees with this engine's order for every
/// relic that acts, or it refuses by name (witnessed in `fixture_tests`, in
/// both orders). Mr Struggles beside Mercury Hourglass is refused by
/// `engine::admission` (`multiple AfterPlayerTurnStart enemy-damage relics`).
/// Bone Tea's charge, the other half of #2884, is seeded from the save in
/// [`seeded_counters`], so its pass runs in a Rust-built opening exactly as it
/// does in a loaded document.
fn after_player_turn_start_peers_are_ordered(
    relics: &BTreeSet<&str>,
    relics_entering: &[String],
    dispatch_ordered: bool,
) -> Result<(), OpeningRefusal> {
    const BELLOWS: &str = "RELIC.BELLOWS";
    const POPPER: &str = "RELIC.FESTIVE_POPPER";
    const CHOICES_PARADOX: &str = "RELIC.CHOICES_PARADOX";
    const TOASTY_MITTENS: &str = "RELIC.TOASTY_MITTENS";
    // Refused beside either body.
    const ORDER_OBSERVABLE_PEERS: [&str; 4] = [
        CHOICES_PARADOX,
        "RELIC.GAMBLING_CHIP",
        TOASTY_MITTENS,
        "RELIC.VEXING_PUZZLEBOX",
    ];
    // Refused beside Popper only.
    const POPPER_ONLY_PEERS: [&str; 4] = [
        "RELIC.BONE_TEA",
        "RELIC.MERCURY_HOURGLASS",
        "RELIC.MR_STRUGGLES",
        "RELIC.ROYAL_POISON",
    ];
    let precedes = |first: &str, second: &str| {
        let positions = |relic: &str| {
            relics_entering
                .iter()
                .enumerate()
                .filter(move |(_, owned)| owned.as_str() == relic)
                .map(|(index, _)| index)
                .collect::<Vec<_>>()
        };
        dispatch_ordered
            && positions(first)
                .into_iter()
                .max()
                .zip(positions(second).into_iter().min())
                .is_some_and(|(last_first, first_second)| last_first < first_second)
    };
    // The pairs whose order the save records and this engine then follows.
    let recorded = |owner: &str, peer: &str| match (owner, peer) {
        (BELLOWS, CHOICES_PARADOX) => precedes(CHOICES_PARADOX, BELLOWS),
        (POPPER, TOASTY_MITTENS) => dispatch_ordered,
        _ => false,
    };
    for owner in [POPPER, BELLOWS] {
        if !relics.contains(owner) {
            continue;
        }
        let peers: Vec<String> = ORDER_OBSERVABLE_PEERS
            .into_iter()
            .chain(
                if owner == POPPER {
                    POPPER_ONLY_PEERS.as_slice()
                } else {
                    &[]
                }
                .iter()
                .copied(),
            )
            .filter(|peer| relics.contains(peer) && !recorded(owner, peer))
            .map(ToString::to_string)
            .collect();
        if !peers.is_empty() {
            return Err(OpeningRefusal::TurnStartRelicOrderUnrecorded { owner, peers });
        }
    }
    Ok(())
}

/// The same-`AfterSideTurnStart` co-ownerships whose order around Fencing
/// Manual's turn-one Forge is observable (#3090).
///
/// Native runs the relic listeners of this hook in `Player.Relics` order
/// (`CombatState::IterateHookListeners` `0x137409`); this port runs
/// `engine::relics::fencing_manual_after_side_turn_start` at a fixed position
/// in the late relic group, after the whole early group
/// (`engine::relics::after_side_turn_start`) and the compiled template pass.
/// The Forge writes the Hand (a generated L0 Sovereign Blade appended at its
/// bottom, with the next card uid and a generated-card history tick) and
/// nothing else a peer reads, so the peers that make the order observable are
/// the other turn-one Hand generators of the same hook:
///
/// * `RELIC.ORANGE_DOUGH` — two generated cards into Hand, in the **early**
///   group. Dough-then-Blade and Blade-then-Dough give different Hand orders
///   and uids. The fixed order is Dough first, which is native exactly when
///   the inventory is vouched for as dispatch order
///   (`relics_entering_dispatch_ordered`) and every Orange Dough precedes
///   every Fencing Manual in it. That is admitted — it is every Fencing
///   Manual fight in the corpus (UE7YGG9XC3ZB holds Orange Dough at index 3
///   and Fencing Manual at 9) — and every other case refuses.
/// * `RELIC.CROSSBOW` and `RELIC.BIG_HAT` — likewise early-group Hand
///   generators, refused at any order. `engine::admission` already refuses
///   Fencing Manual beside either (`Crossbow AfterSideTurnStart acquisition
///   order`, Big Hat's same-hook list), and the engine refuses Big Hat for a
///   non-Ironclad owner, while Fencing Manual is in `RegentRelicPool` only
///   (`Relic<FencingManual>` at its `IL_001c`, and in no other pool). Since
///   #2970 Crossbow draws any owner's pool, a Regent's included, so this
///   refusal and admission's are what keep a Crossbow + Fencing Manual pair
///   closed; both are spelled so that no widening can silently open it.
///
/// and `RELIC.INFUSED_CORE`, whose turn-one Lightning channel the oracle
/// refused beside Fencing Manual (`engine::admission`'s `Fencing Manual +
/// Infused Core turn-start order`). It is gated above by
/// [`TURN_ONE_ORB_RELICS`] today and is spelled for the same reason.
///
/// Every other represented same-hook listener writes energy, gold, Strength,
/// an orb or a pet, none of which the Forge reads or writes, so those orders
/// commute. Placed with the other ownership-only decisions.
fn fencing_manual_turn_start_peers_are_ordered(
    relics: &BTreeSet<&str>,
    relics_entering: &[String],
    dispatch_ordered: bool,
) -> Result<(), OpeningRefusal> {
    const FENCING_MANUAL: &str = "RELIC.FENCING_MANUAL";
    const ORANGE_DOUGH: &str = "RELIC.ORANGE_DOUGH";
    const ORDER_OBSERVABLE_PEERS: [&str; 4] = [
        "RELIC.BIG_HAT",
        "RELIC.CROSSBOW",
        "RELIC.INFUSED_CORE",
        ORANGE_DOUGH,
    ];
    if !relics.contains(FENCING_MANUAL) {
        return Ok(());
    }
    let positions = |relic: &'static str| {
        relics_entering
            .iter()
            .enumerate()
            .filter(move |(_, owned)| owned.as_str() == relic)
            .map(|(index, _)| index)
    };
    let dough_runs_first = dispatch_ordered
        && positions(ORANGE_DOUGH)
            .max()
            .zip(positions(FENCING_MANUAL).min())
            .is_some_and(|(last_dough, first_manual)| last_dough < first_manual);
    let peers: Vec<&str> = ORDER_OBSERVABLE_PEERS
        .into_iter()
        .filter(|peer| relics.contains(peer))
        .filter(|peer| !(*peer == ORANGE_DOUGH && dough_runs_first))
        .collect();
    if peers.is_empty() {
        return Ok(());
    }
    Err(OpeningRefusal::RelicCombinationRefused {
        relics: std::iter::once(FENCING_MANUAL)
            .chain(peers)
            .map(ToString::to_string)
            .collect(),
        reason: "the turn-1 AfterSideTurnStart order of Fencing Manual's Forge \
                 against a same-hook Hand generator (or Infused Core) is \
                 observable, and the recorded relic order does not place it \
                 where this port's fixed listener order runs it",
    })
}

/// The canonical `rng` key one save stream projects to.
fn canonical_stream_key(save_name: &str) -> &'static str {
    COMBAT_STREAMS
        .iter()
        .find_map(|(name, canonical)| (*name == save_name).then_some(*canonical))
        .expect("the nine combat streams are a closed set")
}

/// The nine streams, **read** from the save rather than derived.
///
/// A schema >= 19 save records each stream's counter and its four xoshiro
/// words, and `live_coach.verify_stream_seeding` falsifiably checks that the
/// derivation agrees: 3,382 save files scanned, 3,092 at schema 20, zero
/// disagreeing streams (`entry/counters.rs`). Reading them keeps the whole
/// nine-stream path free of a seeding scheme, which is why `rng::stream_seed`
/// still has no `v0.111.0` arm.
///
/// The three optional streams (`combat_card_generation`,
/// `combat_potion_generation`, `combat_orbs`) are absent from the synthetic
/// fixture and present on every corpus save; an absent one is carried as a
/// stream at counter 0 with the oracle's own zero words, exactly as
/// `start_combat` constructs `Rng(..., counter=None or 0)`.
fn stream_states(
    entry: &EntryDocument,
) -> Result<BTreeMap<String, CanonicalRngV2>, OpeningRefusal> {
    let mut out = BTreeMap::new();
    for (save_name, canonical_name) in COMBAT_STREAMS {
        let state = match entry.streams.get(save_name) {
            Some(state) => CanonicalRngV2 {
                counter: state.counter,
                words: [state.s0, state.s1, state.s2, state.s3],
            },
            // A **required** stream's absence is already a refusal one stage
            // earlier (`entry::counters::run_streams`), so only an optional one
            // can reach here. `start_combat` still constructs it at counter 0
            // from the derived seed, so its four words are in the projected
            // document and cannot be defaulted away.
            None if OPTIONAL_COMBAT_STREAMS.contains(&save_name) => {
                let derived = crate::rng::optional_combat_stream_at_zero(&entry.seed, save_name);
                canonical(&derived)
            }
            None => {
                return Err(OpeningRefusal::CombatStreamAbsent {
                    stream: canonical_name,
                });
            }
        };
        out.insert(canonical_name.to_string(), state);
    }
    Ok(out)
}

fn xoshiro(state: &CanonicalRngV2) -> Xoshiro256StarStar {
    Xoshiro256StarStar {
        words: state.words,
        counter: state.counter,
    }
}

fn canonical(state: &Xoshiro256StarStar) -> CanonicalRngV2 {
    CanonicalRngV2 {
        counter: state.counter,
        words: state.words,
    }
}

/// Build the canonical document of the state the two fire points act on.
///
/// Zero-default elision is the contract both engines implement, so a field
/// equal to its `State` default is **omitted**; only what the opening actually
/// computed is written. That is why this function is short and the projection
/// is total: the relic-derived scalars come from the catalog
/// (`boundary::relic_derived_player_scalars`, which since #2736 also carries
/// `template_relics` — the oracle's
/// `tuple(r for r in entry["relics_entering"] if r in TEMPLATE_RELICS)`, frozen Python `start_combat`, deleted #2827) and everything else defaults.
#[allow(clippy::too_many_arguments)]
fn pre_hook_document(
    entry: &EntryDocument,
    pile: &[usize],
    monsters: &[MonsterSpec],
    rng: &BTreeMap<String, CanonicalRngV2>,
    seeded: &BTreeMap<&'static str, i64>,
    relics: &BTreeSet<&str>,
    ember_tea: Option<i64>,
    stone_cracker_upgraded: Option<&[usize]>,
) -> Result<CanonicalStateV2, OpeningRefusal> {
    let mut player: CanonicalEntityV2 = BTreeMap::new();
    player.insert("hp".to_string(), Value::from(entry.hp_entering));
    player.insert("max_hp".to_string(), Value::from(entry.max_hp_entering));
    // `State.gold` is declared `FieldDefault::Int(0)` in `boundary.rs`, so a
    // broke player's gold is elided like every other default; writing the `0`
    // is a non-canonical document the boundary refuses (`NonCanonicalDefault`,
    // ten corpus fights before #2827).
    if entry.gold_entering != 0 {
        player.insert("gold".to_string(), Value::from(entry.gold_entering));
    }
    // `State.ascension`, elided at its default like every other field.
    // `build_pre_hook` has already refused anything but an exact level.
    if let Some(level) = entry
        .ascension
        .filter(|level| *level != i64::from(crate::encounters::MODELED_ASCENSION))
    {
        player.insert("ascension".to_string(), Value::from(level));
    }
    player.insert(
        "relics_entering".to_string(),
        Value::Array(
            entry
                .relic_entry
                .relics_entering
                .iter()
                .map(|id| Value::String(id.clone()))
                .collect(),
        ),
    );
    // `State.relics_entering_dispatch_ordered` defaults to `False`
    // (frozen Python, deleted #2827) and the boundary refuses an explicit `false` as
    // `NonCanonicalDefault` (`boundary.rs`), so only a vouched order is
    // written, like `gold` above. Every save vouches for its order; a `.run`
    // entry-facts document (the legacy review path, #2827 C2) cannot, and
    // before this elision its every opening was refused by the boundary.
    if entry.relic_entry.dispatch_ordered {
        player.insert(
            "relics_entering_dispatch_ordered".to_string(),
            Value::Bool(true),
        );
    }
    player.insert(
        "next_card_uid".to_string(),
        Value::from(entry.deck_entering.len()),
    );
    if let Some(slots) = potion_slots(entry)? {
        player.insert("potion_slots".to_string(), Value::Array(slots));
    }
    if let Some(pool) = reward_card_pool(entry.character.as_deref()) {
        player.insert("reward_card_pool".to_string(), Value::from(pool));
    }
    if let Some(odds) = reward_card_rarity_odds(entry.node_type.as_deref()) {
        player.insert("reward_card_rarity_odds".to_string(), Value::from(odds));
    }
    // `splash_unlock_epochs`: the recorded epoch set, `EPOCH.`-stripped,
    // deduplicated and sorted (frozen Python, deleted #2827). An absent profile
    // is the empty tuple, which is the field's default and therefore elided.
    if let Some(epochs) = normalized_card_pool_epochs(entry).filter(|e| !e.is_empty()) {
        player.insert(
            "splash_unlock_epochs".to_string(),
            Value::Array(epochs.into_iter().map(Value::String).collect()),
        );
    }
    // `entropy_card_pool`: the owner's own pool name, and only for the two
    // characters whose pool the oracle carries (frozen Python `start_combat`,
    // deleted #2827), and only when the profile reveals every epoch of THAT
    // owner's pool (#3336; see [`entropy_card_pool`]).
    if let Some(pool) = entropy_card_pool(entry.character.as_deref())
        .filter(|_| owner_card_pool_epochs_revealed(entry, entry.character.as_deref()))
    {
        player.insert("entropy_card_pool".to_string(), Value::from(pool));
    }
    // Two v0.111.0 build constants whose values differ from the field default,
    // so they are always emitted: `REGALITE_BLOCK` 4 (default 6) and
    // `INKY_ATTACK_DAMAGE` 0 (default 1) — `_regalite_block_amount_for_build`
    // / `_inky_attack_damage_for_build`, frozen Python `_relic_cond_eval`, deleted #2827.
    player.insert("regalite_block_amount".to_string(), Value::from(4));
    player.insert("inky_attack_damage".to_string(), Value::from(0));
    for (field, value) in seeded {
        player.insert((*field).to_string(), Value::from(*value));
    }
    // `State.ember_tea_combats_left` (frozen Python `start_combat`, deleted #2827), whose default is
    // `-1` and whose owned value is the exact saved charge. It is the one
    // persistent counter the opening both seeds AND spends
    // ([`apply_room_entry_strength`]), so it is written here rather than in
    // `seeded_counters`, whose members all take a modulus this one has not.
    if let Some(combats_left) = ember_tea {
        player.insert(
            "ember_tea_combats_left".to_string(),
            Value::from(combats_left),
        );
    }
    let base_slots = orb_base_slots(entry.character.as_deref());
    if base_slots != 0 {
        player.insert("orb_base_slots".to_string(), Value::from(base_slots));
        player.insert("orb_slots".to_string(), Value::from(base_slots));
    }
    if entry.fully_unlocked_potion_pool == Some(true) {
        player.insert("fully_unlocked_potion_pool".to_string(), Value::Bool(true));
    }
    // The dense compatibility mirror (`combat_sim._sync_potion_belt_mirrors`).
    // It goes through the same [`potion_identity`] strip as the authoritative
    // sparse slots above; before #2770 only this one stripped, which is what
    // made the boundary's cross-field mirror check unreachable behind an
    // identity refusal.
    if !entry.belt.slots.is_empty() {
        player.insert(
            "potions".to_string(),
            Value::Array(
                entry
                    .belt
                    .slots
                    .iter()
                    .map(|slot| Value::String(potion_identity(&slot.id).to_string()))
                    .collect(),
            ),
        );
    }
    // `State.kaiser_facing` (`start_combat`, frozen Python, deleted #2827): `0`
    // exactly when the roster is `(CRUSHER, ROCKET)`, else the `-1` default.
    //
    // v0.111.0 IL: `Rocket/<AfterAddedToRoom>d__29::MoveNext` (RVA
    // `0x367c94`) applies `SurroundedPower` to the Rocket's opponents at
    // `IL_008b`-`IL_00ae`. `SurroundedPower::.ctor` (RVA `0xa8e88`) only
    // chains `PowerModel::.ctor`, and the type has no apply-time body, so its
    // `_facing` field is its zero default when combat starts; `get_Facing`
    // (RVA `0xa8bac`) reads that field. `<UpdateDirection>d__14::MoveNext`
    // (RVA `0x347a5c`) is the reader that fixes the wire meaning: facing `0`
    // turns to `1` on a `BackAttackLeftPower` target (the Crusher,
    // `IL_002b`-`IL_0049`), and `1` back to `0` on a `BackAttackRightPower`
    // one (`IL_00a5`-`IL_00b4`) — `combat_sim._update_kaiser_facing`'s
    // encoding. Without this seed the opening emitted the owned roster with
    // the absent `-1`, a state `engine::monsters::kaiser_roster_is_valid`
    // calls malformed for a Crusher/Rocket roster (#2827).
    if matches!(
        monsters,
        [first, second]
            if (first.kind, second.kind) == (MonsterKind::Crusher, MonsterKind::Rocket)
    ) {
        player.insert("kaiser_facing".to_string(), Value::from(0));
    }
    // Aeonglass's Withering Presence is player state (#2957).
    // `Aeonglass/<AfterAddedToRoom>d__31::MoveNext` (v0.111.0 RVA `0x3524e8`)
    // applies `WitheringPresencePower` with Amount 6 to every opponent
    // (`IL_00d3` `Power<WitheringPresencePower>`, `IL_00fa` `ldc.i4.6`,
    // `IL_0108` `PowerCmd::Apply`), the same countdown
    // `engine::cards::withering_after_card_played` resets to. The oracle seeds
    // it in `start_combat` whenever the roster holds an Aeonglass
    // (`withering_cards_left=`, frozen Python, deleted #2827) and then
    // registers the power as the fight's first AfterCardPlayed object
    // (`_register_after_card_played_power(s, "withering")`),
    // which takes uid 0 from the fresh allocator and leaves it at 1. Nothing in `start_combat` allocates from that
    // counter earlier, so the three fields below are exactly what the fire
    // points start from. Without them the pre-hook document carried an
    // Aeonglass with no countdown, which `aeonglass_state_is_exact` refuses.
    if monsters
        .iter()
        .any(|monster| monster.kind == MonsterKind::Aeonglass)
    {
        player.insert("withering_cards_left".to_string(), Value::from(6));
        player.insert(
            "after_card_played_power_order".to_string(),
            serde_json::json!([["withering", 0]]),
        );
        player.insert(
            "next_after_side_turn_end_power_uid".to_string(),
            Value::from(1),
        );
    }
    // Owner state a relic seeds before the first turn
    // (frozen Python `start_combat`, deleted #2827). Each is that relic's own
    // `CanonicalVars` constant, cited at its declaration in the oracle:
    // Anchor's block survives turn 1 because `Creature.AfterTurnStart`'s block
    // clear is skipped when `TurnNumber == 1` (`Creature` IL `0x4204f8`).
    for (relic, field, amount) in [
        // `ANCHOR_BLOCK`, `Anchor::get_CanonicalVars` `0x23a97c`.
        ("RELIC.ANCHOR", "block", 10),
        // `BRONZE_SCALES` seeds ThornsPower 3.
        ("RELIC.BRONZE_SCALES", "thorns", 3),
        // `STONE_DEXTERITY`, OddlySmoothStone `CanonicalVars One`.
        ("RELIC.ODDLY_SMOOTH_STONE", "dexterity", 1),
        // `AKABEKO_VIGOR`, Akabeko `CanonicalVars` VigorPower 8.
        ("RELIC.AKABEKO", "vigor", 8),
        // `GORGET_PLATING`, Gorget PlatingPower amount 4.
        ("RELIC.GORGET", "plating", 4),
    ] {
        if relics.contains(relic) {
            player.insert(field.to_string(), Value::from(amount));
        }
    }
    // Per-fight constants that are zero when the relic is owned and -1
    // otherwise (frozen Python `start_combat`, deleted #2827). The negative
    // default means the owned case is what gets written.
    for (relic, field) in [
        ("RELIC.KUNAI", "kunai"),
        ("RELIC.SHURIKEN", "shuriken"),
        ("RELIC.ORNAMENTAL_FAN", "orn_fan"),
        ("RELIC.KUSARIGAMA", "kusarigama"),
        ("RELIC.METRONOME", "metronome"),
        ("RELIC.MINI_REGENT", "mini_regent_used"),
        ("RELIC.PAELS_LEGION", "paels_legion_cooldown"),
    ] {
        if relics.contains(relic) {
            player.insert(field.to_string(), Value::from(0));
        }
    }
    // The per-fight flags whose owned value is `true` and whose field default
    // is `false` ([`PER_FIGHT_RELIC_STATE_SEEDED`]). Each is seeded fresh from
    // bare ownership, exactly as `start_combat` writes it, and never read from
    // the save — the table's doc comment carries the native reset site that
    // makes "fresh" exact for each one. Spelled as a table rather than four
    // literal rows so the pin can assert it is disjoint from
    // [`PER_FIGHT_RELIC_STATE_UNSEEDED`] — a relic that was written here and
    // then refused would be a contradiction neither table could see alone.
    for (relic, field) in PER_FIGHT_RELIC_STATE_SEEDED {
        if relics.contains(relic) {
            player.insert(field.to_string(), Value::Bool(true));
        }
    }
    // The two Tea Sets' charge, copied from the save rather than seeded fresh
    // (#2847). Both relics carry a persisted `_gainEnergyInNextCombat`: the
    // only writer of `true` is `AfterRoomEntered` behind
    // `isinst RestSiteRoom` (`VenerableTeaSet` RVA `0x9da89` `IL_0002`-
    // `IL_0011`, `FakeVenerableTeaSet` `0x93823` the same), so a combat room
    // never charges it and the value a combat begins with is exactly the
    // saved one — which `build_pre_hook` has already refused when absent. The
    // body that spends it is `engine::relics::after_energy_reset`, which the
    // opening reaches through `engine::deal_opening_hand`'s turn-1 walk, so
    // the opening and a loaded document run the same code. Without this seed
    // a charged Tea Set's +2 (Fake: +1) turn-1 energy was silently dropped.
    //
    // The wire names are the oracle's: `tea_set` (frozen Python `start_combat`, deleted #2827) and
    // `fake_tea_set_charged` (`start_combat`). Only the charged case is written;
    // `false` is both fields' default and is elided.
    for (relic, field, charged) in [
        (
            "RELIC.VENERABLE_TEA_SET",
            "tea_set",
            entry.relic_entry.tea_set_charged,
        ),
        (
            "RELIC.FAKE_VENERABLE_TEA_SET",
            "fake_tea_set_charged",
            entry.relic_entry.fake_tea_set_charged,
        ),
    ] {
        if relics.contains(relic) && charged == Some(true) {
            player.insert(field.to_string(), Value::Bool(true));
        }
    }
    // The passive relic pets (#2827): one `[kind, player_key, 9999, 9999]` row
    // per owned relic, sorted by kind (frozen Python `start_combat`, deleted #2827). Native
    // adds them at `BeforeCombatStart`, but this crate has no hot state for
    // them. `HotBoundary::from_canonical` derives the roster from ownership
    // and refuses a document whose `relic_pets` disagrees, so an owned pet has
    // to be in the pre-hook document already. That is exact because nothing
    // on the walk before the oracle's assignment reads the roster: the
    // oracle's readers are `_has_living_local_pet` and `_has_passive_relic_pet`
    // (AnyAlly targeting, summon disclosure, Pael's Legion's turn-start tick
    // and card trigger), and none of them runs at `AfterRoomEntered` or in the
    // `BeforeCombatStart` listeners ahead of it. The hook itself is marked in
    // `engine::passive_relic_pets_before_combat_start`, which carries the IL.
    let relic_pets: Vec<Value> = [
        ("RELIC.BYRDPIP", "BYRDPIP"),
        ("RELIC.PAELS_LEGION", "PAELS_LEGION"),
    ]
    .into_iter()
    .filter(|(relic, _)| relics.contains(relic))
    .map(|(_, kind)| serde_json::json!([kind, 0, PASSIVE_RELIC_PET_HP, PASSIVE_RELIC_PET_HP]))
    .collect();
    if !relic_pets.is_empty() {
        player.insert("relic_pets".to_string(), Value::Array(relic_pets));
    }

    let hopper = is_thieving_hopper_entry(entry);
    let mut piles = piles(entry, pile, hopper)?;
    // `hopper_master_deck` (frozen Python `start_combat`, deleted #2827), taken here, BEFORE
    // Stone Cracker's upgrade below: the oracle appends each row in the deck
    // loop, ahead of the shuffle and of the `start_combat` pile rewrite. See
    // [`hopper_master_deck`].
    if hopper {
        player.insert(
            "hopper_master_deck".to_string(),
            hopper_master_deck(&piles["draw"]),
        );
    }
    // Stone Cracker's two upgrades (frozen Python `start_combat`, deleted #2827), written onto
    // the pile before anything below reads it, as the oracle rewrites `pile`
    // before constructing the `State`. See [`stone_cracker_upgrades`].
    if let Some(upgraded) = stone_cracker_upgraded {
        let draw = piles
            .get_mut("draw")
            .expect("the draw pile is always emitted");
        for &position in upgraded {
            draw[position].upgrade += 1;
        }
    }
    // `State.genetic_algorithm_deck_growth` (frozen Python `start_combat`, deleted #2827): one
    // `(deck_row, growth)` row per entering Genetic Algorithm copy, in the
    // deck's own save-array order, which is already ascending by row. The
    // canonical boundary re-derives this from the cards' `extra` rows and
    // **cross-checks it against the document's own field**
    // (`boundary.rs`, "master rows must be unique, sorted, and equal every
    // linked combat copy"), so omitting it is a refusal rather than an
    // elision.
    let genetic_growth: Vec<Value> = entry
        .deck_entering
        .iter()
        .enumerate()
        .filter(|(_, card)| card.id.trim_start_matches("CARD.") == "GENETIC_ALGORITHM")
        .map(|(deck_row, card)| {
            genetic_algorithm_entry_growth(card)
                .map(|growth| Value::Array(vec![Value::from(deck_row), Value::from(growth)]))
        })
        .collect::<Result<_, _>>()?;
    if !genetic_growth.is_empty() {
        player.insert(
            "genetic_algorithm_deck_growth".to_string(),
            Value::Array(genetic_growth),
        );
    }
    // `State.scythe_deck_growth` (frozen Python, deleted #2827; passed):
    // `start_combat` appends `(deck_row, growth)` for every entering The
    // Scythe copy in save-array order, which is already ascending
    // by row. It is the master copy's `IncreasedDamage`, which the play body
    // grows through `DeckVersion` (`TheScythe/<OnPlay>d__15::MoveNext`
    // `0x3c2ed0` IL_00f0-IL_0102). Elided when empty, like Genetic
    // Algorithm's.
    let scythe_growth: Vec<Value> = entry
        .deck_entering
        .iter()
        .enumerate()
        .filter(|(_, card)| card.id.trim_start_matches("CARD.") == "THE_SCYTHE")
        .map(|(deck_row, card)| {
            the_scythe_entry_growth(card)
                .map(|growth| Value::Array(vec![Value::from(deck_row), Value::from(growth)]))
        })
        .collect::<Result<_, _>>()?;
    if !scythe_growth.is_empty() {
        player.insert(
            "scythe_deck_growth".to_string(),
            Value::Array(scythe_growth),
        );
    }
    // `ps_strikes` is the Strike-tag count over ALL combat cards, which at
    // combat start is the whole pile — taken BEFORE the fixup, because the
    // fixup only permutes.
    // A deck with no Strike-tagged card has `0`, the field's declared
    // `FieldDefault::Int(0)`, which is elided rather than written (six corpus
    // fights refused `NonCanonicalDefault` here before #2827).
    let ps_strikes = strike_tag_count(&piles["draw"]);
    if ps_strikes != 0 {
        player.insert("ps_strikes".to_string(), Value::from(ps_strikes));
    }
    let draw = piles
        .get_mut("draw")
        .expect("the draw pile is always emitted");
    let (fixup_order, innates) = turn_one_fixup_order(draw)?;
    *draw = fixup_order
        .iter()
        .map(|&index| draw[index].clone())
        .collect();
    if !hopper {
        stamp_card_identities(draw, relics);
    }
    // #3404: a turn-one BeforeHandDraw relic that inserts into, or selects
    // from, Draw runs BEFORE the fixup natively, so the pre-hook pile keeps
    // the shuffled order (each card carrying the uid it was just stamped with)
    // and the engine applies the fixup after those relics. See
    // [`defers_turn_one_fixup`].
    if defers_turn_one_fixup(relics, hopper) && !is_identity_order(&fixup_order) {
        let mut shuffled: Vec<Option<CanonicalCardV2>> = vec![None; draw.len()];
        for (card, &index) in draw.drain(..).zip(&fixup_order) {
            shuffled[index] = Some(card);
        }
        *draw = shuffled
            .into_iter()
            .map(|card| card.expect("the fixup order is a permutation"))
            .collect();
    }
    if innates != 0 {
        player.insert("innate_min_draw".to_string(), Value::from(innates));
    }

    // `State.exact_piles`, elided at its `false` default like every other
    // field. Derived from the deck, not the shuffled pile: grouping is
    // order-free, and the oracle groups in `start_combat`'s deck loop.
    //
    // `exact = exact or _has_distinguishable_sort_ties(pile)`
    // (frozen Python `start_combat`, deleted #2827): an owner's post-upgrade pile is re-read, since
    // an upgrade can merge an `(id, upgrade)` group whose members differ.
    // Taken over the pre-fixup pile, which only permutes.
    if hopper
        || entering_deck_needs_exact_piles(entry)
        || (stone_cracker_upgraded.is_some() && pile_has_distinguishable_sort_ties(&piles["draw"]))
    {
        player.insert("exact_piles".to_string(), Value::Bool(true));
    }

    let mut document = CanonicalStateV2 {
        schema: STATE_SCHEMA_V2.to_string(),
        game_build: Some(entry.game_build.as_str().to_string()),
        player,
        monsters: monsters.iter().enumerate().map(monster_entity).collect(),
        piles,
        rng: rng.clone(),
        continuations: Vec::new(),
        refusal: None,
    };
    // The 112 immutable relic mirrors and the `template_relics` inventory are
    // derived from the catalog rather than enumerated here, so a relic added
    // to either list cannot be silently omitted by this builder. The catalog
    // needs a document, and the document needs the scalars, so the catalog is
    // built once from the partial document — it reads only `relics_entering`,
    // the build and the card identities, none of which the scalars touch.
    let catalog = HotBoundary::catalog_from_canonical(&document)?;
    for (name, value) in crate::boundary::relic_derived_player_scalars(&catalog) {
        document.player.insert(name, value);
    }
    // `spectrum_shift_generation_pool` (frozen Python `start_combat`, deleted #2827): the
    // Regent's Colorless generation pool, written whenever the owner is the
    // Regent and the recorded profile is non-empty; the empty tuple otherwise,
    // which is the default and is elided. The derivation is
    // `steps::neutral::derive_colorless_generation_pool`, the certified port
    // of the oracle's `_derive_colorless_generation_pool` (#2512). Reached
    // only since #2736: every Regent save holds Divine Right, a template
    // relic, so no Regent fight opened before `template_relics` was written
    // above.
    //
    // The pool is the one the recorded profile derives, partial or not. Since
    // #2739 the boundary carries it exactly in both directions:
    // `HotBoundary::from_canonical` admits only this derivation under the
    // document's partial profile and `to_canonical` re-emits the same
    // derivation from `Catalog::splash_unlock_epochs`, so a partial profile
    // (42 rows under `COLORLESS1_EPOCH` + `COLORLESS2_EPOCH`) round-trips as
    // 42 rather than coming back as the 50-row constant. The #2736 interim
    // refusal that stood here until then is retired.
    if entry.character.as_deref() == Some("CHARACTER.REGENT")
        && document.player.contains_key("splash_unlock_epochs")
    {
        let exact = crate::steps::neutral::derive_colorless_generation_pool(
            catalog.splash_unlock_epochs(),
            false,
            None,
        );
        document.player.insert(
            "spectrum_shift_generation_pool".to_string(),
            Value::Array(exact.iter().map(|id| Value::from(id.as_str())).collect()),
        );
    }
    // The three owner-pool listener provenances (#2847):
    // `hello_world_generation_pool`, `call_of_the_void_generation_pool` and
    // `creative_ai_generation_pool` (frozen Python `start_combat`, deleted #2827). The oracle
    // writes each one when the deck holds its source card, the owner is that
    // card's character, and the recorded profile is non-empty; since #3375
    // any owner with a character card pool qualifies, because native draws
    // from the OWNER's pool whatever the character
    // (`listener_pool_provenance_is_recorded` cites the IL) — under the
    // fight's OWN profile (#2512), not the fully-unlocked constant. Unlike
    // Spectrum Shift's, this projection is profile-exact in both directions:
    // `HotBoundary` admits exactly `steps::neutral::owner_listener_pool(owner,
    // profile, power)` and re-emits the same derivation, so no partial-profile
    // refusal is owed. The derivation's IL (the three `BeforeHandDraw` bodies
    // and `GetDistinctForCombat`'s filter) is cited at `owner_listener_pool`.
    //
    // Nothing native is written at combat start: the listener re-derives the
    // pool from the owner's `CharacterCardPool` when it fires. The field is
    // the oracle's frozen provenance of that derivation, and a document that
    // omits it is a different document (and one on which a later Hello World
    // / Creative AI / Call of the Void listener has no admitted pool), so the
    // opening writes it where the oracle does.
    let deck_ids: BTreeSet<&str> = entry
        .deck_entering
        .iter()
        .map(|card| card.id.trim_start_matches("CARD."))
        .collect();
    if document.player.contains_key("splash_unlock_epochs") {
        for (card, field, power) in OWNER_LISTENER_POOL_FIELDS {
            if let Some(owner) = listener_pool_owner(entry).filter(|_| deck_ids.contains(card)) {
                let pool = catalog.owner_listener_pool(owner, power);
                document.player.insert(
                    field.to_string(),
                    Value::Array(pool.iter().map(|id| Value::from(id.as_str())).collect()),
                );
            }
        }
    }
    // Last, because it reads the fight's final card closure: the catalog
    // above was built before the listener and Spectrum Shift pools were
    // written, and the boundary interns both into the closure (#3389).
    let closure = HotBoundary::catalog_from_canonical(&document)?;
    if let Some(count) = voltaic_lightning_channeled_seed(&closure) {
        document
            .player
            .insert("lightning_channeled".to_string(), Value::from(count));
    }
    Ok(document)
}

/// The deck card, wire field and listener power of each owner-pool listener
/// provenance `start_combat` writes (frozen Python, deleted #2827).
///
/// The owner is no longer a column (#3375): every listener draws from its
/// OWNER's character pool whatever the character
/// ([`listener_pool_provenance_is_recorded`] cites the IL), so the pool is
/// written for [`listener_pool_owner`], the character's own `RewardPool` —
/// the same value [`reward_card_pool`] writes and `HotBoundary` derives the
/// admitted pool from.
const OWNER_LISTENER_POOL_FIELDS: [(&str, &str, PowerId); 3] = [
    (
        "HELLO_WORLD",
        "hello_world_generation_pool",
        PowerId::HelloWorld,
    ),
    (
        "CALL_OF_THE_VOID",
        "call_of_the_void_generation_pool",
        PowerId::CallOfTheVoid,
    ),
    (
        "CREATIVE_AI",
        "creative_ai_generation_pool",
        PowerId::CreativeAi,
    ),
];

/// frozen Python (deleted #2827): the recorded epoch set, `EPOCH.`-stripped,
/// deduplicated and sorted. `None` when the save carries no profile at all,
/// which the oracle turns into the empty tuple.
fn normalized_card_pool_epochs(entry: &EntryDocument) -> Option<Vec<String>> {
    let epochs = entry.unlocked_card_pool_epochs.as_ref()?;
    let mut normalized: Vec<String> = epochs
        .iter()
        .map(|epoch| epoch.trim_start_matches("EPOCH.").to_string())
        .collect();
    normalized.sort();
    normalized.dedup();
    Some(normalized)
}

/// frozen Python `start_combat` (deleted #2827): `character.removeprefix("CHARACTER.").lower()`,
/// for the two characters whose Entropy pool the oracle carries.
///
/// The field has no native counterpart. What it certifies is read off its
/// consumers. Two require it to EQUAL `ironclad`: Infernal Blade
/// (`steps::ironclad_uncommon::infernal_blade_exact` and its admission twin),
/// which shuffles the frozen fully-unlocked Ironclad attack pool, and Big Hat's
/// admission gate. Every other reader only refuses a value that CONTRADICTS
/// `reward_card_pool` (`engine::cards::owner_pool_generation_provenance_is_exact`
/// and the #3122 Colorless twins), so for a Regent the field adds no input.
/// The certificate is therefore "this owner's own pool is fully revealed".
///
/// The oracle wrote it under the save-level `fully_unlocked_card_pool`, which
/// is the Ironclad 2/5/7 epochs for EVERY owner (#3336). For Ironclad that is
/// exactly the certificate. For a Regent it tested the wrong epochs:
/// `RegentCardPool::FilterThroughEpochs` (RVA `0xf28e0`) tests only Regent
/// epochs, so the emission now follows
/// [`owner_card_pool_epochs_revealed`]: `regent` is written when every
/// `REGENT*_EPOCH` gating a Regent row is revealed, whatever the Ironclad
/// epochs.
fn entropy_card_pool(character: Option<&str>) -> Option<&'static str> {
    match character? {
        "CHARACTER.IRONCLAD" => Some("ironclad"),
        "CHARACTER.REGENT" => Some("regent"),
        _ => None,
    }
}

/// `StoneCracker::CardsVar` (`StoneCracker::get_CanonicalVars`, RVA
/// `0x9bf6e`, IL_0001 `ldc.i4.2` into `CardsVar::.ctor`), read by the hook as
/// `DynamicVars.Cards.IntValue` (`<AfterRoomEntered>d__4::MoveNext` IL_008d-
/// IL_0097). The oracle's `STONE_CRACKER_CARDS` (frozen Python, deleted #2827).
const STONE_CRACKER_CARDS: usize = 2;

/// Stone Cracker's pre-draw upgrade: which pile positions it upgrades, in the
/// order it takes them, advancing `sel` (the run's `CombatCardSelection`) by
/// exactly what the native body consumes.
///
/// # The native body (v0.111.0 IL)
///
/// `StoneCracker/<AfterRoomEntered>d__4::MoveNext` (RVA `0x331b0c`):
///
/// * IL_0021-IL_002d: `room isinst CombatRoom`, else leave — every opening is
///   a combat room.
/// * IL_0038-IL_0044: `PileTypeExtensions::GetPile(PileType 1, Owner)` — the
///   Draw pile, already holding the combat-start shuffle
///   (`CardPile::RandomizeOrderInternal`, see [`shuffle`]). The hook runs
///   after `SetUpCombat`'s shuffle and before the opening deal
///   (`engine::fire_after_room_entered` carries that ordering).
/// * IL_0044-IL_006d: `.Cards.Where(<>c::<AfterRoomEntered>b__4_0).ToList()`,
///   whose lambda (`0x331b02`) is `CardModel::get_IsUpgradable` (RVA
///   `0x7cee5`: `CurrentUpgradeLevel < MaxUpgradeLevel`). So the pool is every
///   upgradable card **in Draw-pile list order**, the same order the opening
///   shuffle wrote.
/// * IL_0072-IL_0087: `ListExtensions::StableShuffle(pool,
///   RunRngSet.CombatCardSelection)`. `StableShuffle` (RVA `0x1131a4`) copies
///   the list, `List.Sort()`s the copy (IL_0014), writes it back, then runs
///   `UnstableShuffle` (IL_003a) — `pool.len() - 1` draws, none for a pool of
///   0 or 1. The comparer is `CardModel::CompareTo` (RVA `0x7e3d0`): the model
///   id through `AbstractModel::CompareTo` (`0x79eaa`) and `ModelId::CompareTo`
///   (`0x8190c`, category then entry, both `String.Compare(..., 4)` =
///   `StringComparison.Ordinal`), then `CurrentUpgradeLevel`; anything else
///   ties. .NET's introsort is not stable past 16 elements, so the sort is
///   done over the **positions** with
///   [`crate::dotnet_sort::dotnet_list_sort_by_key`] (the swap-for-swap port)
///   to keep each physical card's identity through the tie permutation.
/// * IL_008c-IL_00a6: `.Take(CardsVar).ToList()` — at most
///   [`STONE_CRACKER_CARDS`], fewer when the pool is smaller.
/// * IL_00a9: `CardCmd::Upgrade(list, 1)` (RVA `0x12f660`): returns at once
///   when the combat is ending (never, before the first turn), rechecks
///   `IsUpgradable` per card (IL_002d, true for every taken card, since each
///   is a distinct object upgraded at most once), and runs
///   `UpgradeInternal`/`FinalizeUpgradeInternal` (IL_0081, IL_0087): one
///   level. The pile-type-6 branch (IL_0047) is the Deck pile's history
///   bookkeeping, not taken for a Draw card. A fresh entry copy carries no
///   local cost rows, so `CardEnergyCost.UpgradeBy`'s clamp has nothing to
///   act on and the upgrade is the level alone (`card_upgrade_to`, frozen Python, deleted #2827, returns early without a physical payload and
///   otherwise re-writes the same empty modifiers).
///
/// The upgrade ladder is the generated `CARD_ROWS` (`next level has a row`,
/// [`crate::engine::cards::native_card_is_upgradable`]), which the crate pins
/// equal to the native `MaxUpgradeLevel` for every card.
///
/// # Where this departs from the oracle
///
/// The oracle (frozen Python `start_combat`, deleted #2827) sorts and shuffles the card
/// **values**, then upgrades `pile[pile.index(chosen)]` — the *first* pile
/// copy equal to each chosen value — on the argument that equal copies are
/// interchangeable. They are not in the Draw pile: which copy is upgraded is
/// its position, and the opening deal reads positions. Native upgrades the
/// object `Take` returned (IL_00a6-IL_00a9), so this follows the object.
/// Whenever the chosen object is not the first equal copy in pile order the
/// two documents differ in where the upgraded copy sits; everything else
/// (the Sel stream's advance, the set of upgraded values) agrees. The game
/// sides with the object: on every corpus capture Rust opens, the `.mcr`'s
/// first checksum (`After player turn start`, the game's ordered Hand and
/// Draw with upgrade levels) equals this opening, and the oracle's differs on
/// five of eleven (#2954, a frozen-Python error recorded, not fixed).
fn stone_cracker_upgrades(
    entry: &EntryDocument,
    pile: &[usize],
    sel: &mut Xoshiro256StarStar,
) -> Result<Vec<usize>, OpeningRefusal> {
    let mut pool = Vec::new();
    for (position, &row) in pile.iter().enumerate() {
        let card = &entry.deck_entering[row];
        let id = card.id.trim_start_matches("CARD.");
        let upgradable = CardId::from_str(id)
            .zip(u8::try_from(card.upgrade_level).ok())
            .map(|(id, upgrade)| {
                crate::engine::cards::native_card_is_upgradable(crate::catalog::CardIdentity {
                    id,
                    upgrade,
                    enchantment: None,
                })
            })
            .ok_or_else(|| OpeningRefusal::StoneCrackerUpgradeLadderUnknown {
                card: format!("{id}+{}", card.upgrade_level),
            })?;
        if upgradable {
            pool.push(position);
        }
    }
    let mut pool = crate::dotnet_sort::dotnet_list_sort_by_key(&pool, |&position| {
        let card = &entry.deck_entering[pile[position]];
        (card.id.trim_start_matches("CARD."), card.upgrade_level)
    });
    // `Xoshiro256StarStar::shuffle` is the `UnstableShuffle` port; it only
    // errors on a bound that does not fit `i32`, which a pile cannot reach.
    sel.shuffle(&mut pool)
        .expect("a pile length always fits the shuffle bound");
    pool.truncate(STONE_CRACKER_CARDS);
    Ok(pool)
}

/// `_has_distinguishable_sort_ties(pile)` (frozen Python, deleted #2827), over the
/// pre-hook pile: some `(id, upgrade)` group holds two cards that differ in
/// anything but their uid, or holds two members of a class
/// ([`crate::engine::cards::divergent_physical_card`],
/// [`crate::engine::cards::divergent_identity_enchantment`]) whose equal fresh
/// copies diverge after one plays. The uid is set aside because the oracle's
/// pile has none yet.
fn pile_has_distinguishable_sort_ties(cards: &[CanonicalCardV2]) -> bool {
    let divergent = |card: &CanonicalCardV2| {
        CardId::from_str(&card.id).is_some_and(crate::engine::cards::divergent_physical_card)
            || card
                .enchantment
                .as_ref()
                .and_then(|value| value.get(0))
                .and_then(Value::as_str)
                .and_then(EnchantmentId::from_str)
                .is_some_and(crate::engine::cards::divergent_identity_enchantment)
    };
    let mut first: BTreeMap<(&str, i64), CanonicalCardV2> = BTreeMap::new();
    for card in cards {
        let mut bare = card.clone();
        bare.uid = None;
        match first.get(&(card.id.as_str(), card.upgrade)) {
            Some(prior) => {
                if *prior != bare || divergent(card) || divergent(prior) {
                    return true;
                }
            }
            None => {
                first.insert((card.id.as_str(), card.upgrade), bare);
            }
        }
    }
    false
}

/// `State.exact_piles` at combat start (frozen Python `start_combat`, deleted #2827; passed
/// as `exact_piles=exact` in `start_combat`): whether any `(id, upgrade)` group of
/// the entering deck holds members a reshuffle could tell apart.
///
/// **Why the flag exists (IL, v0.111.0).** Every reshuffle is
/// `CardPileCmd/<Shuffle>d__22::MoveNext` (RVA `0x3e4b74`) calling
/// `ListExtensions::StableShuffle` at IL_00c4, and `StableShuffle` (RVA
/// `0x1131a4`) first `List.Sort()`s the pile (IL_0014) and only then runs
/// `UnstableShuffle` (IL_003a). The sort's comparer is `CardModel::CompareTo`
/// (RVA `0x7e3d0`): `AbstractModel::CompareTo` on the model id (IL_0019), then
/// `CurrentUpgradeLevel` (IL_002c..IL_003a), and `ldc.i4.0; ret` for anything
/// else (IL_0045). The enchantment, keywords and per-instance state are never
/// compared, and .NET's introsort is not stable, so which physical copy of a
/// tie lands where is a function of the live pile order. When a group's
/// members are interchangeable that order is unobservable; when they are not,
/// the engine has to carry the exact physical order, and this flag is how the
/// document says so.
///
/// The oracle's rule, group by `(id, upgrade)` over the deck in save order:
///
/// * two or more distinct enchantment payloads `(id, amount)` — a plain copy
///   beside an enchanted one, or two amounts — mark the group; or
/// * a group of two or more whose card is in
///   [`crate::engine::cards::divergent_physical_card`] or any of whose
///   enchantments is in
///   [`crate::engine::cards::divergent_identity_enchantment`] marks it, since
///   equal fresh copies of those diverge after one of them plays.
///
/// The oracle's other arm, `exact or hopper_entry` (frozen Python `start_combat`, deleted #2827),
/// is taken by the caller beside [`hopper_master_deck`]: a Hopper pile is
/// always exact, because equal payloads still carry distinct `DeckVersion`
/// row uids.
///
/// The two id lists are the engine's own, the same ones the live promotion
/// (`card_slice_has_distinguishable_sort_ties`) reads, so the opening's seed
/// and the mid-fight promotion cannot disagree about which classes diverge.
/// An id the crate does not know is not divergent here; the catalog built
/// from this document refuses it by name.
fn entering_deck_needs_exact_piles(entry: &EntryDocument) -> bool {
    type Payload<'a> = Option<(&'a str, i64)>;
    let mut groups: BTreeMap<(&str, i64), Vec<Payload<'_>>> = BTreeMap::new();
    let retain_clone = retains_clone_enchantment(entry);
    for card in &entry.deck_entering {
        let payload = entry_card_enchantment(card, retain_clone)
            .map(|id| (id, card.enchant_amount.unwrap_or(1)));
        groups
            .entry((card.id.trim_start_matches("CARD."), card.upgrade_level))
            .or_default()
            .push(payload);
    }
    groups.iter().any(|((id, _), payloads)| {
        let distinct: BTreeSet<&Payload<'_>> = payloads.iter().collect();
        distinct.len() > 1
            || (payloads.len() > 1
                && (CardId::from_str(id)
                    .is_some_and(crate::engine::cards::divergent_physical_card)
                    || payloads.iter().flatten().any(|(enchantment, _)| {
                        EnchantmentId::from_str(enchantment)
                            .is_some_and(crate::engine::cards::divergent_identity_enchantment)
                    })))
    })
}

/// The Thieving Hopper encounter, `hopper_entry` in `start_combat`
/// (frozen Python, deleted #2827): the one encounter whose opening publishes
/// the deck's `DeckVersion` rows.
fn is_thieving_hopper_entry(entry: &EntryDocument) -> bool {
    entry.encounter_id == "ENCOUNTER.THIEVING_HOPPER_WEAK"
}

/// `State.hopper_master_deck` at combat start (frozen Python `start_combat`, deleted #2827; passed in `start_combat`): one `(deck_row, payload)` row per entering card, in
/// save-array order, whose uid is the combat copy's own physical uid.
///
/// **Native (v0.111.0 IL).** Thievery removes a stolen combat card's
/// persistent twin through `CardModel.DeckVersion`, an object reference each
/// combat copy is given as the combat deck is populated:
/// `Player::PopulateCombatState` (RVA `0x117a90`) walks `Player.Deck.Cards`
/// in order (IL_000d-IL_001c), clones each master card with
/// `CombatState::CloneCard` (IL_002e), stores the master in the clone's
/// `DeckVersion` (`CardModel::set_DeckVersion`, IL_0036), appends the clone
/// to the draw pile (`CardPile::AddInternal`, IL_0049), and only after the
/// loop randomizes the pile (`CardPile::RandomizeOrderInternal`, IL_0075).
/// So the link is per object, fixed before the shuffle, and the master
/// carries the deck card's full payload. The oracle models the object
/// reference as the entering row ordinal, which is why a Hopper copy's uid is
/// its deck row (see [`piles`]).
///
/// Built from `draw` before anything rewrites it (Stone Cracker, the turn-one
/// fixup), by sorting the freshly built copies back into row order and
/// encoding each with the boundary's own payload encoder
/// ([`crate::boundary::canonical_card_payload_value`]), so the rows are
/// the tuples the oracle appends: `tuple(card)` after every entry transform
/// in the deck loop (enchantment, the default physical state, the Scythe and
/// Genetic Algorithm rows). Clone is retained on a Hopper copy
/// (frozen Python `start_combat`, deleted #2827), and [`piles`] never drops it.
///
/// No belt refusal accompanies it: the oracle's
/// `_validate_hopper_potion_identity_names` (frozen Python, deleted #2827; called in `start_combat`) intersects the belt with `_SELECTION_TARGET_POTIONS`,
/// which is the empty set on this build, so it never refuses.
fn hopper_master_deck(draw: &[CanonicalCardV2]) -> Value {
    let mut rows: Vec<&CanonicalCardV2> = draw.iter().collect();
    rows.sort_by_key(|card| card.uid);
    Value::Array(
        rows.into_iter()
            .map(|card| {
                Value::Array(vec![
                    Value::from(card.uid.expect("a Hopper copy carries its deck row")),
                    crate::boundary::canonical_card_payload_value(card.clone()),
                ])
            })
            .collect(),
    )
}

/// `_apply_turn_one_pile_fixups` (frozen Python `_commit_live_card_piles`, deleted #2827), the rewrite
/// `SetupPlayerTurn` performs on the draw pile after the combat-start shuffle
/// and before the deal.
///
/// Two passes, in this order: every `IMBUED`-enchanted copy moves to the
/// **bottom**, then every Innate copy that is not `IMBUED` moves to the
/// **front, reversed**. Returns the fixed pile as indices into `draw`, and
/// `innate_min_draw`, the count of the second group. The engine's twin, for
/// a fixup deferred past the turn-one BeforeHandDraw relics, is
/// `engine::turn::apply_deferred_turn_one_pile_fixup`.
///
/// The oracle reads each card's *effective* keywords — spec keywords plus the
/// instance's local and transient ones. A freshly instantiated deck card has
/// neither, so the spec's `innate` flag is the whole answer here; an instance
/// whose local keywords could differ refuses rather than being read from its
/// spec ([`OpeningRefusal::InnateKeywordProvenance`]).
///
/// The spec's `innate` flag is the row's keyword or a `ROYALLY_APPROVED`
/// enchantment (#3178): `RoyallyApproved::OnEnchant` RVA `0xd62c9`
/// IL_0007-0008 is `Card.AddKeyword(3)`, Innate, which the native fixup
/// lambda `<>c::<SetupPlayerTurn>b__102_1` RVA `0x3f256f` IL_0008 reads
/// through `Keywords.Contains(3)` like any canonical Innate
/// ([`crate::catalog::royally_approved_adds_innate_and_retain`]). The deck
/// copy was enchanted before its upgrades were replayed, and no upgrade removes
/// Innate, so the enchantment alone decides.
fn turn_one_fixup_order(draw: &[CanonicalCardV2]) -> Result<(Vec<usize>, i64), OpeningRefusal> {
    for card in draw.iter() {
        if !card.local_keywords.is_empty() || !card.transient_keywords.is_empty() {
            return Err(OpeningRefusal::InnateKeywordProvenance {
                card: card.id.clone(),
            });
        }
    }
    let imbued = |card: &CanonicalCardV2| {
        card.enchantment
            .as_ref()
            .is_some_and(|value| value.get(0).and_then(Value::as_str) == Some("IMBUED"))
    };
    let royally_approved = |card: &CanonicalCardV2| {
        card.enchantment.as_ref().is_some_and(|value| {
            value.get(0).and_then(Value::as_str) == Some(EnchantmentId::RoyallyApproved.as_str())
        })
    };
    let innate = |card: &CanonicalCardV2| {
        !imbued(card)
            && (royally_approved(card) || canonical_card_row(card).is_some_and(|row| row.innate))
    };
    let mut order: Vec<usize> = (0..draw.len()).collect();
    if draw.iter().any(imbued) {
        let (bottom, rest): (Vec<_>, Vec<_>) = order.into_iter().partition(|&i| imbued(&draw[i]));
        order = rest;
        order.extend(bottom);
    }
    let (mut front, rest): (Vec<_>, Vec<_>) = order.into_iter().partition(|&i| innate(&draw[i]));
    let innate_count = front.len();
    front.reverse();
    front.extend(rest);
    Ok((front, i64::try_from(innate_count).unwrap_or(i64::MAX)))
}

fn is_identity_order(order: &[usize]) -> bool {
    order
        .iter()
        .enumerate()
        .all(|(position, &index)| position == index)
}

/// Whether the opening leaves the turn-one fixup to the engine (#3404).
///
/// `CombatManager/<SetupPlayerTurn>d__102::MoveNext` (v0.111.0 RVA
/// `0x3f6c6c`) awaits `Hook::BeforeHandDraw` at IL_0177 and `ModifyHandDraw`
/// at IL_01fd, and only then, on turn one, moves the
/// `ShouldStartAtBottomOfDrawPile` cards to the bottom (IL_02b8-IL_02ed) and
/// the remaining Innate cards to the top (IL_0324-IL_0360), before the
/// hand's `CardPileCmd::Draw` (IL_03cd). A turn-one BeforeHandDraw relic that
/// writes or reads Draw order therefore sees the shuffled pile, not the fixed
/// one:
///
/// * Funerary Mask's three `AddGeneratedCardToCombat(card, Draw, Owner,
///   Random)` (`FuneraryMask/<BeforeHandDraw>d__6::MoveNext` `0x325000`
///   IL_005d-IL_007d) and Blessed Antler's Dazed inserts draw each index
///   against the shuffled pile, and the fixup then carries the Innate cards
///   past them — RAB8SE1H26ZH n47 dealt Defend before its two Souls natively
///   and after them here, the corpus's only opening card-pile mismatch;
/// * Jeweled Mask's `NextItem` (`JeweledMask/<BeforeHandDraw>d__2::MoveNext`
///   `0x3272f4` IL_00ce-IL_00e4) indexes the Draw Powers in shuffled order.
///
/// Thieving Hopper, Ghost Seed and Tea of Discourtesy owners keep the
/// build-time fixup: their uid stamping has its own contract
/// ([`stamp_card_identities`]), and the corpus has none beside these relics.
fn defers_turn_one_fixup(relics: &BTreeSet<&str>, hopper: bool) -> bool {
    !hopper
        && !relics.contains(GHOST_SEED)
        && !relics.contains(TEA_OF_DISCOURTESY)
        && [
            "RELIC.FUNERARY_MASK",
            "RELIC.BLESSED_ANTLER",
            "RELIC.JEWELED_MASK",
        ]
        .into_iter()
        .any(|relic| relics.contains(relic))
}

/// `RELIC.GHOST_SEED`: its oracle body allocates physical-card uids
/// **before** `_apply_turn_one_pile_fixups`, while the draw pile is still in
/// shuffled order.
///
/// `start_combat` builds the pile as plain payload tuples with
/// `next_card_uid = 0` (frozen Python, deleted #2827), and the first
/// `_normalize_card_identities` call that meets them allocates every uid in
/// `_ALL_CARD_PILES` order. Between the pile's
/// construction and the fixup exactly two bodies reach that
/// allocator:
///
/// * `_ghost_seed_after_room_entered` (frozen Python, deleted #2827) maps every card through
///   `_map_combat_cards`, whose `_commit_live_card_piles` →
///   `_prepare_card_pile_updates` normalizes **unconditionally**, whether or
///   not any card changed — so a Ghost Seed
///   owner's uids are the shuffled positions, and the fixup then carries
///   those identified cards to their new places;
/// * Tea of Discourtesy's Dazed insertion, which normalizes around each
///   `_insert_draw_random` (`start_combat`) — see [`TEA_OF_DISCOURTESY`].
///
/// Every other call `start_combat` makes on that stretch was walked for a
/// path to the allocator (`_fire_relic_templates`' energy/block/heal/stars/
/// power/damage ops, Delicate Frond, Fur Coat, Belt Buckle, Petrified Toad,
/// `summon_ally`, the roster validators, `_initialize_random_ai`): the only
/// ones found are `_validated_dampen_state`, which returns before normalizing
/// when no Dampen row is live (frozen Python, deleted #2827), and the continuation
/// write-through under `_apply_owner_strength`, which iterates
/// `s.continuations` and is empty at combat start (`_write_through_after_side_turn_end_tombstone`). The corpus
/// agrees: instrumenting the allocator over every rooted capture found
/// `_map_combat_cards` as the only pre-fixup caller.
const GHOST_SEED: &str = "RELIC.GHOST_SEED";

/// `RELIC.TEA_OF_DISCOURTESY`, the other pre-fixup allocator
/// ([`GHOST_SEED`] lists both).
///
/// A live Tea allocates in the middle of its own two Dazed insertions, which
/// this opening does not order the oracle's way (the fixup runs here before
/// the hooks, the oracle's after them (frozen Python `start_combat`, deleted #2827)); every such fight has two
/// cards beyond the deck after the deal, and the check refuses on the count.
/// A spent Tea allocates nothing (`if s.tea_discourtesy_combats_left > 0`, `start_combat`), so the oracle numbers the post-fixup pile. Rather than read the
/// counter, an owner keeps the shuffled stamping and is held to the strict
/// `0..n-1` order, which it meets exactly when the fixup moved nothing: the
/// conservative side of that choice is a refusal, never a wrong uid.
const TEA_OF_DISCOURTESY: &str = "RELIC.TEA_OF_DISCOURTESY";

/// The uids the opening allocates before the deal, and where they land
/// (#2992, #3162).
///
/// `draw` fresh cards of id `draw_id` are inserted at random Draw positions,
/// then `hand_head` fresh cards are added at the Hand's bottom, all before the
/// `fromHandDraw` Draw, so the deal appends after them. The turn-1
/// `BeforeHandDraw` generators that allocate are mutually refused
/// ([`refuse_unordered_blessed_antler_peers`],
/// [`refuse_unordered_hand_draw_card_peers`]), so at most one row is non-zero.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PreDeal {
    draw: u64,
    draw_id: &'static str,
    hand_head: u64,
}

/// A bare count is Blessed Antler's form: that many Dazed into Draw.
impl From<u64> for PreDeal {
    fn from(draw: u64) -> Self {
        Self {
            draw,
            draw_id: "DAZED",
            hand_head: 0,
        }
    }
}

/// Which uids the opening allocates before the deal (#2992, #3162).
///
/// * Blessed Antler: `CardsVar(3)` (`BlessedAntler::get_CanonicalVars`
///   `0x90cf5` `IL_0011`-`IL_0013`), all generated into one
///   `AddGeneratedCardsToCombat(Draw, Random)` call
///   (`<BeforeHandDraw>d__7::MoveNext` `0x31fe64` `IL_0094`-`IL_009d`).
/// * Funerary Mask: `CardsVar(3)` (`FuneraryMask::get_CanonicalVars`
///   `0x93ddc`, `ldc.i4.3` at `IL_0001`), one `Soul` per iteration through
///   `ICombatState::CreateCard` then `AddGeneratedCardToCombat(card, Draw,
///   Owner, Random)` (`<BeforeHandDraw>d__6::MoveNext` `0x325000`,
///   `IL_005d`-`IL_007d`), looped while `i < Cards.BaseValue`
///   (`IL_00fd`-`IL_011d`), so three fresh uids in creation order.
/// * Radiant Pearl: `CardsVar(1)` (`RadiantPearl::get_CanonicalVars`
///   `0x99f9e`, `ldc.i4.1` at `IL_0001`), one `Luminesce` created at
///   `IL_0055`-`IL_006c` and added by `AddGeneratedCardsToCombat(list, Hand,
///   Owner, …)` at `IL_008d`-`IL_0096`, before the deal, so it heads the Hand.
///
/// Every other pre-deal generator is still refused by a gate table.
fn pre_deal_allocations(relics: &[String]) -> PreDeal {
    let owns = |id: &str| relics.iter().any(|relic| relic == id);
    if owns("RELIC.BLESSED_ANTLER") {
        PreDeal::from(BLESSED_ANTLER_DAZED)
    } else if owns("RELIC.FUNERARY_MASK") {
        PreDeal {
            draw: FUNERARY_MASK_SOULS,
            draw_id: "SOUL",
            hand_head: 0,
        }
    } else if owns("RELIC.RADIANT_PEARL") {
        PreDeal {
            hand_head: RADIANT_PEARL_LUMINESCE,
            ..PreDeal::default()
        }
    } else {
        PreDeal::default()
    }
}

/// Blessed Antler's `CardsVar` (`0x90cf5` `IL_0012` `ldc.i4.3`), the oracle's
/// `BLESSED_ANTLER_CARDS` (frozen Python, deleted #2827).
const BLESSED_ANTLER_DAZED: u64 = 3;

/// Funerary Mask's `CardsVar` (`0x93ddc` `IL_0001` `ldc.i4.3`).
const FUNERARY_MASK_SOULS: u64 = 3;

/// Radiant Pearl's `CardsVar` (`0x99f9e` `IL_0001` `ldc.i4.1`).
const RADIANT_PEARL_LUMINESCE: u64 = 1;

/// The `hand ++ draw` uid order after Jeweled Mask's pre-deal move (#3162).
///
/// `engine::relics::jeweled_mask_before_hand_draw` moves one deck Power from
/// Draw to the Hand's bottom before the `fromHandDraw` Draw
/// (`JeweledMask/<BeforeHandDraw>d__2::MoveNext` `0x3272f4`,
/// `CardPileCmd::Add(card, Hand, Bottom, …)` at `IL_00f8`-`IL_00fe`), so the
/// deal is a prefix draw of the pile with that card removed, appended after
/// it. The check's premise then holds for `[moved] ++ (expected - moved)`.
///
/// The moved card is read off the document: the Hand's first card, when it
/// is a deck uid and a Power. When nothing moved (no Power in Draw), the Hand
/// head is the prefix draw's first card, `expected[0]`, and the reorder is the
/// identity, so this never widens what the check admits past one Power
/// lifted from anywhere in the pile to the Hand's head.
fn jeweled_mask_moved_order(
    document: &CanonicalStateV2,
    expected: Vec<Option<u64>>,
    relics: &[String],
) -> Vec<Option<u64>> {
    if !relics.iter().any(|relic| relic == "RELIC.JEWELED_MASK") {
        return expected;
    }
    let Some(head) = document.piles.get("hand").and_then(|hand| hand.first()) else {
        return expected;
    };
    let is_power = canonical_card_row(head).is_some_and(|row| row.is_power);
    let Some(position) = expected.iter().position(|uid| *uid == head.uid) else {
        return expected;
    };
    if !is_power {
        return expected;
    }
    let mut reordered = expected;
    let moved = reordered.remove(position);
    reordered.insert(0, moved);
    reordered
}

/// Stamp physical-card uids in the order the oracle allocates them.
///
/// With neither [`GHOST_SEED`] nor [`TEA_OF_DISCOURTESY`] owned, nothing
/// allocates before the fixup, so the oracle's first allocation sees the
/// **post-fixup** pile — the Innate copies reversed onto the front and the
/// `IMBUED` ones at the bottom — and, the deal being a prefix draw of it,
/// numbers `hand ++ draw` in that order. [`piles`] stamped shuffled order,
/// which is the same answer only when the fixup moved nothing; this restamps
/// the post-fixup order.
///
/// With either owned, the shuffled stamping is kept: it is the oracle's
/// allocation for Ghost Seed, and the strict fallback for Tea.
///
/// Uids are the simulator's own identity labels, not a native value: the
/// `.mcr` replay binds them to the recorded `combat_card_index` through
/// `mcr_replay._deal_uid_map`, which undoes exactly this Innate/`IMBUED`
/// rewrite (#2552). No native-semantics claim moves here, so no IL is read
/// for it; the fixup itself is [`turn_one_fixup_order`].
fn stamp_card_identities(draw: &mut [CanonicalCardV2], relics: &BTreeSet<&str>) {
    if relics.contains(GHOST_SEED) || relics.contains(TEA_OF_DISCOURTESY) {
        return;
    }
    for (uid, card) in draw.iter_mut().enumerate() {
        card.uid = Some(uid as u64);
    }
}

/// The uids `hand ++ draw` must carry after the deal, in order.
///
/// A prefix draw keeps the pre-hook pile's order, so this is the uid sequence
/// [`stamp_card_identities`] left on it: `0..n-1` after a post-fixup restamp,
/// and the shuffled positions carried through the fixup for a
/// [`GHOST_SEED`] owner. A [`TEA_OF_DISCOURTESY`] owner is held to `0..n-1`
/// whatever the fixup did.
fn expected_identity_order(
    pre_hook: &CanonicalStateV2,
    relics: &[String],
    deferred_fixup: Option<&[usize]>,
) -> Vec<Option<u64>> {
    let draw = pre_hook.piles.get("draw").map(Vec::as_slice).unwrap_or(&[]);
    // A deferred fixup (#3404) leaves the pre-hook pile shuffled; the deal
    // is still a prefix draw of the FIXED pile, whose stamps run in order.
    if let Some(order) = deferred_fixup {
        return order.iter().map(|&index| draw[index].uid).collect();
    }
    // A Thieving Hopper pile is already identified (its deck rows, see
    // [`piles`]), so `_normalize_card_identities` allocates nothing for it
    // and its prefix-drawn uids are the ones it carried. Checked first: a
    // Tea of Discourtesy owner's two Dazed then make the count disagree,
    // which refuses rather than holding the deck rows to `0..n-1`.
    if pre_hook.player.contains_key("hopper_master_deck") {
        return draw.iter().map(|card| card.uid).collect();
    }
    if relics.iter().any(|relic| relic == TEA_OF_DISCOURTESY) {
        return (0..draw.len() as u64).map(Some).collect();
    }
    draw.iter().map(|card| card.uid).collect()
}

/// `expected` with the uid of an `IMBUED` card that turn one's AutoPre phase
/// played out of `hand ++ draw` removed (#3381).
///
/// The fixup put the card at the draw pile's bottom, so the deal (a prefix
/// draw) left it last in `hand ++ draw`. Its AutoPlay
/// (`engine::play::autoplay_imbued_turn_one`) then moves exactly that card to
/// its result pile (Discard, Exhaust, ...), which leaves the rest of the
/// sequence in order. Only an Imbued uid found outside `hand ++ draw` is
/// dropped; anything else the play moved still fails the check.
fn without_auto_played_imbued(
    document: &CanonicalStateV2,
    expected: Vec<Option<u64>>,
) -> Vec<Option<u64>> {
    let imbued = |card: &CanonicalCardV2| {
        card.enchantment
            .as_ref()
            .is_some_and(|value| value.get(0).and_then(Value::as_str) == Some("IMBUED"))
    };
    let played: Vec<u64> = document
        .piles
        .iter()
        .filter(|(pile, _)| pile.as_str() != "hand" && pile.as_str() != "draw")
        .flat_map(|(_, cards)| cards)
        .filter(|card| imbued(card))
        .filter_map(|card| card.uid)
        .collect();
    if played.is_empty() {
        return expected;
    }
    expected
        .into_iter()
        .filter(|uid| !uid.is_some_and(|uid| played.contains(&uid)))
        .collect()
}

/// The belt's canonical *potion identity* — the `POTION.`-stripped name.
///
/// The save and the entry document speak full model ids
/// (`entry::canonical_model_id`, which admits exactly one `.` and so makes
/// this strip exact rather than repeated); `State.potion_slots`,
/// `State.potions` and `State.inert_potions` all speak bare names, because
/// `KNOWN_POTIONS` does (frozen Python `_route_card_play_result`, deleted #2827; `set(_REGISTRY.potions)` —
/// `'WEAK_POTION'`, not `'POTION.WEAK_POTION'`). The oracle performs the same
/// strip in `unmodeled_potion_caveat` (`row["id"].removeprefix("POTION.")`) and
/// again on the dense mirror in `unmodeled_potion_caveat`.
///
/// This is the single function both belt mirrors go through, because the two
/// disagreeing was the whole of #2770: the dense `potions` list stripped and
/// the authoritative sparse `potion_slots` did not, so `PotionId::from_str`
/// saw `"POTION.WEAK_POTION"`, found no such variant, and the boundary
/// refused the belt as an *"unknown/inert potion identity"* — a name that
/// described the symptom and not the cause.
fn potion_identity(model_id: &str) -> &str {
    model_id.strip_prefix("POTION.").unwrap_or(model_id)
}

/// `_potion_slots_from_entry(entry, require_capacity=False)`
/// (frozen Python, deleted #2827): the belt materialised to its recorded
/// capacity, with `null` in every empty slot and the bare potion identity
/// ([`potion_identity`]) in every occupied one.
///
/// `Ok(None)` is *"no belt materialises"* — an entry with neither a positive
/// capacity nor an occupied row, whose belt is `()`, the field default, and is
/// therefore elided. A recorded capacity of 0 (or below) with no rows is that
/// case natively too: `SetMaxPotionCountInternal` (`0x117584`, `IL_0018`)
/// grows `_potionSlots` only when the target exceeds its count, so the loaded
/// belt has no slot.
///
/// Every row must name a slot the native loader places it in
/// ([`OpeningRefusal::PotionBeltRowMalformed`], which carries the IL): a
/// `slot_index`, non-negative, and below the recorded capacity when one is
/// recorded. Until #2791 those three shapes returned `None` through `?` and
/// so *elided* a belt the save carries — a well-formed document silently
/// missing its potions. They refuse by name now, checked before any
/// allocation, so no arm of this function can drop a row.
///
/// Without a recorded capacity (the save always records one; an entry-facts
/// document may not) the belt is kept through the highest occupied index,
/// which is the oracle's rule and is unchanged here.
///
/// A fifth malformed shape, two rows colliding on one index, is not refused
/// here: the write overwrites, but the dense mirror [`pre_hook_document`]
/// builds from every row then has more entries than the sparse non-nulls, so
/// `boundary::hydrate_potion_belt`'s cross-field check refuses it. (Natively
/// `AddPotionInternal` warns and drops the second potion, `IL_0044`–`IL_00d6`;
/// the refusal is the safe side of that.) All of these are measured
/// unreachable across the corpus (0 of 2,342 occupied belts in 3,387 saves).
fn potion_slots(entry: &EntryDocument) -> Result<Option<Vec<Value>>, OpeningRefusal> {
    let recorded = entry.belt.max_potion_slot_count;
    let mut placed = Vec::with_capacity(entry.belt.slots.len());
    for slot in &entry.belt.slots {
        let index = slot
            .slot_index
            .filter(|index| *index >= 0 && recorded.is_none_or(|capacity| *index < capacity))
            .and_then(|index| usize::try_from(index).ok())
            .ok_or_else(|| OpeningRefusal::PotionBeltRowMalformed {
                potion: slot.id.clone(),
                slot_index: slot.slot_index,
                capacity: recorded,
            })?;
        placed.push((index, potion_identity(&slot.id)));
    }
    let len = match recorded {
        // A positive `i64` fits a `usize` on every 64-bit target this crate
        // builds for; the saturation is unreachable there.
        Some(capacity) if capacity > 0 => usize::try_from(capacity).unwrap_or(usize::MAX),
        // A non-positive recorded capacity admitted no row above, so this is
        // the empty belt; without a recorded capacity the oracle keeps rows
        // through the highest occupied index.
        _ => match placed.iter().map(|(index, _)| *index).max() {
            Some(highest) => highest + 1,
            None => return Ok(None),
        },
    };
    let mut slots = vec![Value::Null; len];
    for (index, potion) in placed {
        slots[index] = Value::String(potion.to_string());
    }
    Ok(Some(slots))
}

/// `_BATCH172_CHARACTER_CARD_POOLS` (frozen Python, deleted #2827).
fn reward_card_pool(character: Option<&str>) -> Option<&'static str> {
    match character? {
        "CHARACTER.IRONCLAD" => Some("Ironclad"),
        "CHARACTER.SILENT" => Some("Silent"),
        "CHARACTER.DEFECT" => Some("Defect"),
        "CHARACTER.NECROBINDER" => Some("Necrobinder"),
        "CHARACTER.REGENT" => Some("Regent"),
        _ => None,
    }
}

/// `_BATCH172_ROOM_RARITY_ODDS` (frozen Python, deleted #2827).
fn reward_card_rarity_odds(node_type: Option<&str>) -> Option<&'static str> {
    match node_type? {
        "monster" => Some("RegularEncounter"),
        "elite" => Some("EliteEncounter"),
        "boss" => Some("BossEncounter"),
        _ => None,
    }
}

/// The five piles. `draw`/`hand`/`discard` have no `State` default and are
/// therefore always emitted, even when empty (`boundary::PILE_FIELDS`).
///
/// # The saved per-instance `props`
///
/// Three deck rows carry native saved properties the entry passes through
/// verbatim (`entry::deck::PER_INSTANCE_PROP_CARDS`), and `start_combat`
/// materialises each into the instantiated copy — a step this function must
/// perform too, because dropping it produces a **different card** with nothing
/// downstream to refuse it:
///
/// * `THE_SCYTHE` — damage growth plus a deck-row link
///   (frozen Python `start_combat`, deleted #2827), ported exactly by
///   [`the_scythe_entry_growth`] below. It is in [`PHYSICAL_STATE_CARDS`] but
///   is the one id there whose payload is not the default: slot 7 is the
///   seven-field `(tag, (), growth, False, (), 0, deck_row)` row. The
///   canonical boundary admits that row only as a Thieving Hopper link, so an
///   ordinary fight holding one refuses there, exactly as the oracle's own
///   document does (#2941).
/// * `GENETIC_ALGORITHM` — accumulated Block and a `DeckVersion` row, ported
///   exactly by [`genetic_algorithm_entry_growth`] below. It is **not** in
///   `_PHYSICAL_COST_CARD_IDS`, so nothing else in
///   this function would have caught it.
/// * `MAD_SCIENCE` — two immutable Tinker integers that pick which of the
///   eighteen generated variant rows the copy is (#2942), validated by
///   [`mad_science_entry_variant`] and written as the copy's one
///   `["MAD_SCIENCE_TINKER", type, rider]` `extra` row; the catalog built from
///   the document interns every Mad Science spec under that fight variant
///   ([`crate::catalog::MadScienceVariant`]). A variant whose body is not
///   ported, or two copies that disagree, refuse earlier
///   ([`entering_mad_science_variant`]).
///
/// Every other [`PHYSICAL_STATE_CARDS`] id carries the default slot-7 payload
/// `_with_default_physical_card_state` attaches (frozen Python, deleted #2827; called in `start_combat`); why that is exact for a fresh deck copy is on the
/// constant.
///
/// `SOVEREIGN_BLADE`'s slot-6 payload is deliberately not written here: the
/// boundary already fails closed on a Sovereign Blade card without one
/// (`boundary::card_instance_state`, *"Sovereign Blade requires its native
/// slot-6 payload"*), so that case is a refusal rather than a silent
/// divergence.
fn piles(
    entry: &EntryDocument,
    pile: &[usize],
    hopper: bool,
) -> Result<BTreeMap<String, Vec<CanonicalCardV2>>, OpeningRefusal> {
    let retain_clone = retains_clone_enchantment(entry);
    let mut draw = Vec::with_capacity(pile.len());
    for (position, source) in pile.iter().enumerate() {
        // A Thieving Hopper copy is built as `PhysicalCard(card, deck_row)`
        // in the deck loop, before the shuffle (frozen Python `start_combat`, deleted #2827),
        // so its uid is its save-array row and travels with it through the
        // shuffle; every other copy is numbered by pile position here and
        // restamped by [`stamp_card_identities`].
        let uid = if hopper { *source } else { position };
        let card = &entry.deck_entering[*source];
        let id = card.id.trim_start_matches("CARD.");
        let parsed = crate::ids::CardId::from_str(id);
        let default_physical_state =
            parsed.is_some_and(|parsed| PHYSICAL_STATE_CARDS.contains(&parsed));
        // Native auto-plays an `IMBUED` copy on turn one; the engine plays a
        // single one at turn one's AutoPre phase
        // (`engine::play::autoplay_imbued_turn_one`, #3381). Two or more
        // refuse here by name
        // ([`OpeningRefusal::ImbuedAutoPlayNotModeled`] carries the IL).
        let imbued = |card: &crate::entry::deck::DeckEntry| {
            card.enchantment
                .as_deref()
                .map(|id| id.trim_start_matches("ENCHANTMENT."))
                == Some("IMBUED")
        };
        if imbued(card) && entry.deck_entering.iter().filter(|row| imbued(row)).count() > 1 {
            return Err(OpeningRefusal::ImbuedAutoPlayNotModeled {
                card: id.to_string(),
            });
        }
        // `_set_genetic_algorithm_state(card, growth, deck_row=deck_row)`
        // (frozen Python `_potion_slots_from_entry`, deleted #2827) pads the copy through slot 7 with
        // `_with_default_physical_card_state(card, force=True)` and puts the
        // state row FIRST at slot 8, ahead of any existing tail
        // (`_set_card_physical_state`). A fresh deck card has no tail, so the projection is
        // exactly one `extra` row over the default slot-7 payload.
        let (physical_state, extra) = if id == "GENETIC_ALGORITHM" {
            let growth = genetic_algorithm_entry_growth(card)?;
            (
                Some(serde_json::json!(["PHYSICAL_CARD_STATE", [], 0, false, []])),
                vec![serde_json::json!([
                    "GENETIC_ALGORITHM_STATE",
                    growth,
                    // `deck_row` is the entering deck's **save-array** index,
                    // not the shuffled pile position: `start_combat` walks
                    // `enumerate(entry["deck_entering"])`.
                    *source
                ])],
            )
        } else if parsed == Some(CardId::TheScythe) {
            // `start_combat` (frozen Python, deleted #2827): the default
            // payload, then `_card_add_damage_growth(card, growth)`
            // when the growth is nonzero (adding to slot 7's
            // growth of 0), then `_card_set_scythe_deck_row(card, deck_row)`, whose `_set_card_physical_state` write appends BaseReplayCount (`0`, the fresh value) and then the row. A zero growth
            // skips the add and lands on the same payload. Like Genetic
            // Algorithm's, the row is the save-array index `start_combat`
            // enumerates, not the shuffled position.
            let growth = the_scythe_entry_growth(card)?;
            (
                Some(serde_json::json!([
                    "PHYSICAL_CARD_STATE",
                    [],
                    growth,
                    false,
                    [],
                    0,
                    *source
                ])),
                Vec::new(),
            )
        } else if default_physical_state {
            // `_with_default_physical_card_state(card)` (frozen Python, deleted #2827): the fresh `(tag, (), 0, False, ())` payload.
            (
                Some(serde_json::json!(["PHYSICAL_CARD_STATE", [], 0, false, []])),
                Vec::new(),
            )
        } else if parsed == Some(CardId::MadScience) {
            // #2942: the copy's saved variant, as the one `extra` row the
            // boundary reads it back from (`boundary::card_tail_state`).
            // `entering_mad_science_variant` validated every copy's props
            // and that they agree; Mad Science carries no slot-7 payload
            // (it is not in `PHYSICAL_STATE_CARDS`), so none is written.
            let variant = mad_science_entry_variant(card)?;
            (
                None,
                vec![serde_json::json!([
                    crate::boundary::MAD_SCIENCE_TINKER_TAG,
                    variant.tinker_type,
                    variant.rider
                ])],
            )
        } else {
            (None, Vec::new())
        };
        let enchantment_id = entry_card_enchantment(card, retain_clone);
        draw.push(CanonicalCardV2 {
            id: id.to_string(),
            upgrade: card.upgrade_level,
            uid: Some(uid as u64),
            pick: false,
            enchantment: enchantment_id.map(|id| {
                Value::Array(vec![
                    Value::String(id.to_string()),
                    Value::from(card.enchant_amount.unwrap_or(1)),
                ])
            }),
            local_keywords: Vec::new(),
            transient_keywords: Vec::new(),
            enchantment_state: enchantment_id.and_then(entry_enchantment_state),
            sovereign_blade: None,
            physical_state,
            extra,
        });
    }
    let mut piles = BTreeMap::new();
    piles.insert("draw".to_string(), draw);
    piles.insert("hand".to_string(), Vec::new());
    piles.insert("discard".to_string(), Vec::new());
    Ok(piles)
}

/// Whether a combat copy's `CLONE` enchantment is observable, and so kept in
/// its payload (frozen Python `start_combat`, deleted #2827; #2827 item B).
///
/// `Clone` (v0.111.0) declares no member but its constructor (`Clone::.ctor`
/// RVA `0xd5ea4`, `dump_il.py Clone`): it overrides no hook, modifier or
/// keyword, so in combat it only makes `CardModel.Enchantment` non-null. The
/// oracle therefore omits it from the pile payload, and keeps it only where
/// that non-null is read: Mystic Lighter (`mystic_lighter_entering`, frozen Python `start_combat`, deleted #2827; `MysticLighter::ModifyDamageAdditive`, RVA `0x973c4`, adds
/// its damage only when `CardModel::get_Enchantment` is non-null at
/// `IL_0021`-`IL_0028`) and the Thieving Hopper encounter, whose `DeckVersion` master payload is
/// observable through theft ([`is_thieving_hopper_entry`]). Keeping it
/// everywhere made a Clone deck's document differ from the oracle's (the
/// payload, and `exact_piles` through the enchantment groups) and refuse at
/// load on an enchantment the engine never needs.
fn retains_clone_enchantment(entry: &EntryDocument) -> bool {
    is_thieving_hopper_entry(entry)
        || entry
            .relic_entry
            .relics_entering
            .iter()
            .any(|relic| relic == "RELIC.MYSTIC_LIGHTER")
}

/// The enchantment one entering card's combat copy carries in its payload:
/// the deck row's, `ENCHANTMENT.`-stripped, except an unobservable `CLONE`
/// ([`retains_clone_enchantment`]).
fn entry_card_enchantment(
    card: &crate::entry::deck::DeckEntry,
    retain_clone: bool,
) -> Option<&str> {
    card.enchantment
        .as_deref()
        .map(|id| id.trim_start_matches("ENCHANTMENT."))
        .filter(|id| retain_clone || *id != "CLONE")
}

/// The slot-5 `enchantment_state` one freshly instantiated combat copy carries
/// for its enchantment (#2891): `[NAME, 0]` for `MOMENTUM` and `VIGOROUS`,
/// absent for every other enchantment. Oracle: `start_combat` deck
/// construction, `card += ((), (), (ench_id, 0))` when
/// `ench_id in _MUTABLE_ENCHANTMENTS` (frozen Python, deleted #2827; `_MUTABLE_ENCHANTMENTS`).
///
/// # Why zero is exact (v0.111.0 IL)
///
/// The combat copy is a memberwise clone of the deck card, so it enters with
/// exactly the deck enchantment's mutable fields:
///
/// * `Player::PopulateCombatState` (`0x117a90`) IL_002e calls
///   `CombatState::CloneCard` (`0x137004`), whose IL_000d is
///   `AbstractModel::ClonePreservingMutability` (`0x79ef7`) →
///   `AbstractModel::MutableClone` (`0x79f0c`, IL_000d `MemberwiseClone`).
///   `CardModel::DeepCloneFields` (`0x7d21c`) IL_0095 clones the enchantment
///   the same way and re-attaches it (IL_00b4 `EnchantInternal`);
///   `EnchantmentModel::DeepCloneFields` (`0x7f5c3`) resets only `_card`,
///   `StatusChanged` and `_dynamicVars` — never `_status` or a subclass field.
/// * The deck enchantment is `EnchantmentModel::FromSerializable` (`0x7f6f8`):
///   IL_0017 `ToMutable` of the canonical (whose setters `AssertMutable`, so
///   it is never written), then `SavedProperties::Fill` (IL_002a) and
///   `set_Amount` (IL_0036). No field or property of `EnchantmentModel` or of
///   any `Models.Enchantments` type carries `[SavedProperty]`, so `Fill`
///   restores no mutable state and the save's `{id, amount}` is the whole
///   payload.
/// * Every write of per-combat enchantment state is a combat hook on the
///   combat copy (full xref of the v0.111.0 DLL):
///   `Momentum::set_ExtraDamage` (`0xd61df`) only from `Momentum::OnPlay`
///   (`0xd61ee` IL_000f); `EnchantmentModel::set_Status` (`0x7f4dd`) only from
///   `Vigorous::AfterCardPlayed` (`0xd66cd` IL_0017), `Glam::AfterCardPlayed`
///   (`0xd5fb7` IL_002c), `Sown/<OnPlay>d__2::MoveNext` (`0x3884d4` IL_002c)
///   and `Swift/<OnPlay>d__4::MoveNext` (`0x3885c0` IL_002c);
///   `Glam::set_UsedThisCombat` (`0xd5f85`) only from `Glam::AfterCardPlayed`
///   IL_0025.
///
/// So every combat copy enters with `Momentum._extraDamage == 0` and
/// `_status == EnchantmentStatus.Normal` (0), `Glam._usedThisCombat == false`.
///
/// # The derived set, not just the two named ones
///
/// The mutable-per-combat enchantments on v0.111.0 are exactly Momentum,
/// Vigorous, Glam, Sown and Swift (the xref above). This crate carries them in
/// two representations: Momentum/Vigorous as this slot-5 integer (Momentum's
/// `ExtraDamage`, Vigorous's `Status` as 0/1), and Glam/Sown/Swift as a
/// spent-marker id (`GLAM_USED`/`SOWN_USED`/`SWIFT_USED`,
/// `boundary::spent_marker_enchantment`) whose unspent form is the plain id
/// the enchantment payload already writes. So the unspent entry state of all
/// five is emitted exactly, and nothing is left to refuse. `Slither`'s
/// `_testEnergyCostOverride` is set only by its `.ctor` (`0xd6409` IL_0003,
/// `-1`) — `set_TestEnergyCostOverride` (`0xd631f`) has no caller — and
/// `Goopy`'s `set_Amount` writes the saved `amount` itself, which the payload
/// carries.
fn entry_enchantment_state(enchantment: &str) -> Option<Value> {
    matches!(
        EnchantmentId::from_str(enchantment),
        Some(EnchantmentId::Momentum | EnchantmentId::Vigorous)
    )
    .then(|| serde_json::json!([enchantment, 0]))
}

/// `_genetic_algorithm_entry_growth` (frozen Python, deleted #2827): the
/// accumulated Block one saved Genetic Algorithm copy brings into this fight.
///
/// The native serialization is two integer fields, and the oracle validates
/// them completely rather than reading one: exactly `{"ints": [...]}` with two
/// `{"name", "value"}` rows whose names are `CurrentBlock` / `IncreasedBlock`,
/// a non-negative `IncreasedBlock`, and the native invariant
/// `CurrentBlock == 1 + IncreasedBlock` (base Block one). `IncreasedBlock` is
/// the growth; `CurrentBlock` is a derived mirror and is not carried.
///
/// The oracle's one further gate, `props_ambiguous`, is structurally
/// unreachable from a save: it marks a row the historical `.run` endpoint
/// could not date, and this crate roots from the save's own `deck` array
/// (`entry::deck::deck_entering`), which carries the live props verbatim.
fn genetic_algorithm_entry_growth(
    card: &crate::entry::deck::DeckEntry,
) -> Result<i64, OpeningRefusal> {
    entry_props_growth(
        card,
        "GENETIC_ALGORITHM",
        ("CurrentBlock", "IncreasedBlock"),
        1,
    )
}

/// `_the_scythe_entry_growth` (frozen Python, deleted #2827): the accumulated
/// damage one saved The Scythe copy brings into this fight.
///
/// The same shape as Genetic Algorithm's: exactly `{"ints": [...]}` with the
/// two rows `CurrentDamage` / `IncreasedDamage`, a non-negative
/// `IncreasedDamage`, and `CurrentDamage == 13 + IncreasedDamage`. The
/// growth is `IncreasedDamage`.
///
/// # Why that is the native domain (v0.111.0 IL)
///
/// * `TheScythe` declares exactly two `[SavedProperty]` properties,
///   `CurrentDamage` and `IncreasedDamage` (order 0 each, filled in that
///   Ordinal order); `CardModel` declares none. Measured with the
///   `build_mcr_tables.Dll.saved_properties` reflection model on the archived
///   DLL, the same walk whose positive controls the
///   [`PHYSICAL_STATE_CARDS`] citation names.
/// * `CardModel::FromSerializable` (`0x7e31c`) rebuilds the copy from the
///   canonical `ToMutable` (IL_0017) and `SavedProperties::Fill`s both
///   (IL_002a). `set_CurrentDamage` (`0xee898`) stores the field and writes it
///   as the `Damage` var's base value (IL_000e-IL_0024); `set_IncreasedDamage`
///   (`0xee8ca`) only stores. `TheScythe` overrides no `AfterDeserialized`.
/// * Every in-game writer keeps `CurrentDamage == 13 + IncreasedDamage`: the
///   `.ctor` (`0xee87b`) starts `_currentDamage` at 13 (IL_0002-IL_0004) with
///   `_increasedDamage` 0; `BuffFromPlay` (`0xee990`) adds to
///   `IncreasedDamage` (IL_0003-IL_000a) and calls `UpdateDamage`, which sets
///   `CurrentDamage = 13 + IncreasedDamage` (`0xee9a6` IL_0002-IL_000b);
///   `AfterDowngraded` (`0xee988`) calls `UpdateDamage` too. So a save
///   outside the relation is not one the game wrote, and the copy's damage
///   would then be `CurrentDamage` rather than `13 + growth`; the oracle
///   refuses it and so does this.
/// * The combat copy is a clone of the deck card whose `DeckVersion` is that
///   deck card (`Player::PopulateCombatState` `0x117a90`: `CloneCard`
///   IL_002e, `set_DeckVersion` IL_0036), which is the deck-row link the
///   payload's seventh field records.
///
/// `props_ambiguous` is unreachable from a save, exactly as for Genetic
/// Algorithm (see [`genetic_algorithm_entry_growth`]).
fn the_scythe_entry_growth(card: &crate::entry::deck::DeckEntry) -> Result<i64, OpeningRefusal> {
    entry_props_growth(card, "THE_SCYTHE", ("CurrentDamage", "IncreasedDamage"), 13)
}

/// The shared validator behind [`genetic_algorithm_entry_growth`] and
/// [`the_scythe_entry_growth`]: exactly `{"ints": [two rows]}`, each row
/// exactly `{"name", "value"}` with an integer value, the two names being
/// `(current, increased)` once each, `increased >= 0` and
/// `current == base + increased`. Returns `increased`.
///
/// One deliberate narrowing against both oracle validators: they read
/// `props.get("ints")` and ignore any other key of `props`, and this refuses a
/// `props` object with a second key. That can only refuse a fight the oracle
/// roots, never root one it refuses.
fn entry_props_growth(
    card: &crate::entry::deck::DeckEntry,
    name: &'static str,
    (current_name, increased_name): (&str, &str),
    base: i64,
) -> Result<i64, OpeningRefusal> {
    let (current, increased) = entry_props_int_pair(card, name, (current_name, increased_name))?;
    // Checked: a malformed save can carry `Increased = i64::MAX`, and the
    // validator's contract is to refuse, never to overflow (second high-tier
    // review on #2544).
    if increased < 0 || increased.checked_add(base) != Some(current) {
        return Err(entry_props_malformed(card, name));
    }
    Ok(increased)
}

/// The shape half of [`entry_props_growth`], shared with
/// [`mad_science_entry_variant`]: `props` is exactly `{"ints": [two rows]}`,
/// each row exactly `{"name", "value"}` with an integer value, and the two
/// names are `(first, second)` once each. Returns the two values in that
/// order, whatever order the rows arrived in.
fn entry_props_int_pair(
    card: &crate::entry::deck::DeckEntry,
    name: &'static str,
    (first_name, second_name): (&str, &str),
) -> Result<(i64, i64), OpeningRefusal> {
    let malformed = || entry_props_malformed(card, name);
    let ints = card
        .props
        .as_ref()
        .filter(|props| props.len() == 1)
        .and_then(|props| props.get("ints"))
        .and_then(Value::as_array)
        .filter(|ints| ints.len() == 2)
        .ok_or_else(malformed)?;
    let mut first: Option<i64> = None;
    let mut second: Option<i64> = None;
    for row in ints {
        let row = row
            .as_object()
            .filter(|row| row.len() == 2)
            .ok_or_else(malformed)?;
        let field = row
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(malformed)?;
        let value = row
            .get("value")
            .and_then(Value::as_i64)
            .ok_or_else(malformed)?;
        let slot = if field == first_name {
            &mut first
        } else if field == second_name {
            &mut second
        } else {
            return Err(malformed());
        };
        if slot.replace(value).is_some() {
            return Err(malformed());
        }
    }
    let (Some(first), Some(second)) = (first, second) else {
        return Err(malformed());
    };
    Ok((first, second))
}

/// The generated row one opening card is: its `CARD_ROWS` entry, except a Mad
/// Science copy, which is its saved variant's row (#2942) read back from the
/// `MAD_SCIENCE_TINKER` row [`piles`] wrote. `None` for an id or level the
/// tables do not carry, or a variant whose body is not ported.
///
/// The opening's own document readers (the innate fixup, Jeweled Mask's
/// Power test, the Strike-tag count) go through this, never through the
/// placeholder `CARD_ROWS` row, whose type and body are none of the
/// eighteen Mad Science cards'.
fn canonical_card_row(card: &CanonicalCardV2) -> Option<&'static crate::content_tables::CardRow> {
    let id = CardId::from_str(&card.id)?;
    let upgrade = u8::try_from(card.upgrade).ok()?;
    if id != CardId::MadScience {
        return crate::content_tables::card_row(id, upgrade);
    }
    let [_, tinker_type, rider] = card.extra.first()?.as_array()?.as_slice() else {
        return None;
    };
    crate::catalog::MadScienceVariant::from_saved(tinker_type.as_i64()?, rider.as_i64()?)?
        .row(upgrade)
        .ok()
        .flatten()
}

/// The `CardEntryPropsNotExact` refusal for one entering card's `props`.
fn entry_props_malformed(
    card: &crate::entry::deck::DeckEntry,
    name: &'static str,
) -> OpeningRefusal {
    OpeningRefusal::CardEntryPropsNotExact {
        card: name,
        detail: card.props.as_ref().map_or_else(
            || "absent".to_string(),
            |props| Value::Object(props.clone()).to_string(),
        ),
    }
}

/// `_mad_science_entry_variant` (frozen Python, deleted #2827): the saved
/// Tinker Time variant one entering Mad Science copy is (#2942).
///
/// Exactly `{"ints": [two rows]}` naming `TinkerTimeType` and
/// `TinkerTimeRider` once each (the [`entry_props_growth`] shape, with its
/// one second-key narrowing), and the pair must be one
/// `TinkerTime::ChooseRiderEffect` can write
/// ([`crate::catalog::MadScienceVariant::from_saved`]).
///
/// # Why that is the native domain (v0.111.0 IL)
///
/// `MadScience` declares exactly two `[SavedProperty]` members,
/// `TinkerTimeType` (order 0) and `TinkerTimeRider` (order 2), measured with
/// `build_mcr_tables.Dll.saved_properties` and re-read by
/// `dll_content.DllFacts.mad_science_variants`. `CardModel::FromSerializable`
/// (`0x7e31c`) `Fill`s them at IL_002a and `MadScience` overrides no
/// `AfterDeserialized`, so the combat copy is that pair verbatim. The only
/// pile-bound writer is `TinkerTime/<RiderChosen>d__15::MoveNext` (`0x384308`),
/// which stores the event's `ChosenCardType` and one rider of the matching
/// `ChooseRiderEffect` (`0xd08b4`) blob — Attack {Sapping, Violence, Choking},
/// Skill {Energized, Wisdom, Chaos}, Power {Expertise, Curious, Improvement}.
/// Any other pair is not a card the game wrote, and native would throw on it
/// (`<OnPlay>d__51` IL_01be for a type outside 1..3); refusing it is exact.
fn mad_science_entry_variant(
    card: &crate::entry::deck::DeckEntry,
) -> Result<crate::catalog::MadScienceVariant, OpeningRefusal> {
    let (tinker_type, rider) =
        entry_props_int_pair(card, "MAD_SCIENCE", ("TinkerTimeType", "TinkerTimeRider"))?;
    crate::catalog::MadScienceVariant::from_saved(tinker_type, rider)
        .ok_or_else(|| entry_props_malformed(card, "MAD_SCIENCE"))
}

/// The one variant the entering deck's Mad Science copies carry, when it
/// holds any (#2942); refused when a copy's props are malformed, when two
/// copies disagree, or when the variant's body is not ported.
fn entering_mad_science_variant(
    entry: &EntryDocument,
) -> Result<Option<crate::catalog::MadScienceVariant>, OpeningRefusal> {
    let mut variant = None;
    for card in &entry.deck_entering {
        if card.id.trim_start_matches("CARD.") != CardId::MadScience.as_str() {
            continue;
        }
        let saved = mad_science_entry_variant(card)?;
        if variant.is_some_and(|recorded| recorded != saved) {
            return Err(OpeningRefusal::CardEntryVariantNotModeled {
                card: "MAD_SCIENCE",
                detail: "two copies carry different Tinker Time variants, and the \
                         catalog keys Mad Science by one fight variant"
                    .to_string(),
            });
        }
        variant = Some(saved);
    }
    if let Some(variant) = variant {
        for upgrade in [0, 1] {
            if let Err(row) = variant.row(upgrade) {
                return Err(OpeningRefusal::CardEntryVariantNotModeled {
                    card: "MAD_SCIENCE",
                    detail: format!(
                        "the {} rider: {}",
                        row.rider_name,
                        row.unmodeled.unwrap_or("its body is not ported")
                    ),
                });
            }
        }
    }
    Ok(variant)
}

/// `ps_strikes`: the Strike-tag count over **all** combat cards, which at
/// combat start is the whole pile. Constant per fight — `AllCards` spans every
/// pile including exhaust, and nothing added mid-fight carries the tag.
fn strike_tag_count(cards: &[CanonicalCardV2]) -> i64 {
    cards
        .iter()
        .filter(|card| canonical_card_row(card).is_some_and(|row| row.strike_tag))
        .count() as i64
}

/// One monster's canonical entity — the roster → state admission edge
/// (§A8.4).
///
/// `MonsterSpec` is data and this is the only place that turns it into state,
/// which is what keeps a family module free of engine code. Zero-default
/// elision is the wire contract, so a field equal to its `combat_sim.Monster`
/// default is omitted: `uid`, `slot` and `loop_pos` all default to `0`, and
/// `next_move` / `move_log` to `""` / `()`.
///
/// The uid is the **position**, not a field: `_EncounterCtx.done` stamps
/// `m.uid = i` over the returned list (frozen Python, deleted #2827). A builder
/// that pinned a different `slot` keeps it; `slot` and `uid` are separate
/// fields in `Monster` and only `uid` is creation order by construction.
fn monster_entity((index, spec): (usize, &MonsterSpec)) -> CanonicalEntityV2 {
    let mut entity: CanonicalEntityV2 = BTreeMap::new();
    entity.insert("kind".to_string(), Value::from(spec.kind.as_str()));
    entity.insert("hp".to_string(), Value::from(spec.hp));
    entity.insert("max_hp".to_string(), Value::from(spec.max_hp));
    if index != 0 {
        entity.insert("uid".to_string(), Value::from(index));
    }
    if spec.slot != 0 {
        entity.insert("slot".to_string(), Value::from(spec.slot));
    }
    if spec.loop_pos != 0 {
        entity.insert("loop_pos".to_string(), Value::from(spec.loop_pos));
    }
    if !spec.next_move.is_empty() {
        entity.insert("next_move".to_string(), Value::from(spec.next_move));
    }
    if !spec.move_log.is_empty() {
        entity.insert(
            "move_log".to_string(),
            Value::Array(
                spec.move_log
                    .iter()
                    .map(|name| Value::from(*name))
                    .collect(),
            ),
        );
    }
    // Each spawn-time field carries its own Python type (`SpawnValue`), and
    // the projection keeps `1` and `true` apart, so the tag the BUILDER chose
    // is what lands on the wire. Re-typing here from a table was the previous
    // shape and it could not see `shriek`: the table it consulted was
    // `boundary::MONSTER_FIELDS`, and every field any roster seeds today is a
    // power name rather than an explicit `FieldSpec` row, so the re-typing
    // never fired on a single call site (#2790).
    for (field, value) in &spec.initial_state {
        let projected = match value {
            SpawnValue::Amount(amount) => Value::from(*amount),
            SpawnValue::Flag(flag) => Value::Bool(*flag),
            // `Monster.override` (#2848), whose boundary row is
            // `FieldDefault::Text("")`; the hot state reads it through
            // `MonsterOverride::from_str`, so a string the engine has no
            // variant for refuses at the boundary rather than here.
            SpawnValue::Text(text) => Value::from(*text),
        };
        entity.insert((*field).to_string(), projected);
    }
    entity
}

/// Check the uid-allocation premise the pre-deal stamping rests on.
///
/// After the deal the concatenation `hand ++ draw` must carry exactly
/// `expected` ([`expected_identity_order`]), which is what
/// `_normalize_card_identities` allocates over `_ALL_CARD_PILES` when the
/// deal is a prefix draw of the pile [`stamp_card_identities`] numbered.
/// Anything else means the opening moved or inserted cards this premise does
/// not cover, so the uids — and therefore every `.mcr` target — would be
/// silently different.
///
/// One insertion is covered (#2827): a card a turn-one relic generates into
/// Hand AFTER the deal — Vexing Puzzlebox's `AfterPlayerTurnStart` card
/// (`engine::relics::vexing_puzzlebox_generated_card`). The deck's allocation
/// has already happened by then, so the oracle numbers it from
/// `next_card_uid` onward (`_apply_generation_relic_batch` →
/// `_add_fresh_generated_cards_to_hand`, frozen Python, deleted #2827):
/// on the corpus Necrobinder fight `faba5a5e39e4b9b9` the deck is uids
/// `0..27` and the generated card is `28`, at the bottom of the hand. So every
/// uid the document's `next_card_uid` says was allocated past the deck must
/// sit, in allocation order, at the hand's tail; the rest of `hand ++ draw` is
/// then held to `expected` exactly as before. A fresh card anywhere else — a
/// full Hand's redirect to Discard, say — still refuses.
///
/// A second insertion is covered (#2992): Blessed Antler's three Dazed,
/// inserted at random Draw indices at `BeforeHandDraw`, **before** the deal
/// ([`pre_deal_draw_allocations`]). The oracle normalizes the deck immediately
/// before them (`_normalize_card_identities(s)` (frozen Python `_continue_before_hand_draw_relics`, deleted #2827)) and
/// then reserves the next `BLESSED_ANTLER_CARDS` uids in insertion order
/// (`_continue_before_hand_draw_relics`), so they are the first `pre_deal` uids past the deck, and
/// the deal can carry any of them into Hand. Each must be a `DAZED` somewhere
/// in `hand ++ draw`; with them removed, the check above applies unchanged to
/// what is left, with the later allocations (a Vexing Puzzlebox card) held to
/// the hand's tail as before.
///
/// One removal is covered (#3381): an `IMBUED` card turn one's AutoPre phase
/// played out of `hand ++ draw` ([`without_auto_played_imbued`]).
fn check_card_identity_allocation(
    document: &CanonicalStateV2,
    expected: &[Option<u64>],
    pre_deal: impl Into<PreDeal>,
) -> Result<(), OpeningRefusal> {
    let PreDeal {
        draw: pre_deal,
        draw_id,
        hand_head,
    } = pre_deal.into();
    let empty = Vec::new();
    let deck = expected.len() as u64;
    let next = document
        .player
        .get("next_card_uid")
        .and_then(Value::as_u64)
        .unwrap_or(deck);
    let inserted = deck..deck.saturating_add(pre_deal);
    if pre_deal > 0 {
        let found: Vec<u64> = document
            .piles
            .get("hand")
            .unwrap_or(&empty)
            .iter()
            .chain(document.piles.get("draw").unwrap_or(&empty))
            .filter(|card| card.id == draw_id)
            .filter_map(|card| card.uid)
            .filter(|uid| inserted.contains(uid))
            .collect();
        let mut sorted = found.clone();
        sorted.sort_unstable();
        if next < inserted.end || sorted != inserted.clone().collect::<Vec<_>>() {
            return Err(OpeningRefusal::CardIdentityAllocationDiverged {
                detail: format!(
                    "the {pre_deal} {draw_id} inserted into Draw before the deal should carry \
                     uids {inserted:?} in hand + draw, found {found:?} (next_card_uid {next})"
                ),
            });
        }
    }
    let keep = |card: &&CanonicalCardV2| !card.uid.is_some_and(|uid| inserted.contains(&uid));
    let mut hand: Vec<&CanonicalCardV2> = document
        .piles
        .get("hand")
        .unwrap_or(&empty)
        .iter()
        .filter(keep)
        .collect();
    // The pre-deal Hand-head allocations (Radiant Pearl's Luminesce): the next
    // `hand_head` uids, in allocation order, ahead of every dealt card.
    let head_range = inserted.end..inserted.end.saturating_add(hand_head);
    if hand_head > 0 {
        let head: Vec<Option<u64>> = hand
            .iter()
            .take(usize::try_from(hand_head).unwrap_or(usize::MAX))
            .map(|card| card.uid)
            .collect();
        let allocated: Vec<Option<u64>> = head_range.clone().map(Some).collect();
        if next < head_range.end || head != allocated {
            return Err(OpeningRefusal::CardIdentityAllocationDiverged {
                detail: format!(
                    "the {hand_head} card(s) generated into Hand before the deal should head \
                     it with uids {allocated:?}, found {head:?} (next_card_uid {next})"
                ),
            });
        }
        hand.drain(..allocated.len());
    }
    let draw: Vec<&CanonicalCardV2> = document
        .piles
        .get("draw")
        .unwrap_or(&empty)
        .iter()
        .filter(keep)
        .collect();
    let after_deal = head_range.end;
    let fresh = usize::try_from(next.saturating_sub(after_deal)).unwrap_or(usize::MAX);
    let dealt = hand.len().saturating_sub(fresh);
    let tail: Vec<Option<u64>> = hand[dealt..].iter().map(|card| card.uid).collect();
    let allocated: Vec<Option<u64>> = (after_deal..next).map(Some).collect();
    if tail != allocated {
        return Err(OpeningRefusal::CardIdentityAllocationDiverged {
            detail: format!(
                "the {fresh} uid(s) allocated past the entering deck are not the \
                 hand's tail in allocation order: tail {tail:?}, allocated {allocated:?}"
            ),
        });
    }
    let observed: Vec<Option<u64>> = hand[..dealt]
        .iter()
        .chain(&draw)
        .map(|card| card.uid)
        .collect();
    let expected = without_auto_played_imbued(document, expected.to_vec());
    if observed.len() != expected.len() {
        return Err(OpeningRefusal::CardIdentityAllocationDiverged {
            detail: format!(
                "hand + draw holds {} cards, the entering deck {}",
                observed.len(),
                expected.len()
            ),
        });
    }
    for (position, (uid, want)) in observed.iter().zip(&expected).enumerate() {
        if uid != want {
            return Err(OpeningRefusal::CardIdentityAllocationDiverged {
                detail: format!(
                    "hand + draw position {position} carries uid {uid:?}, the oracle's \
                     allocation {want:?}"
                ),
            });
        }
    }
    Ok(())
}

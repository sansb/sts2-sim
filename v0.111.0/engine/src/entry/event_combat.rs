//! Event-started combats: the pre-combat half of the event (#3248).
//!
//! # Why the entry needs this
//!
//! Every root is built from the save the game writes when the player moves
//! onto a map node — before the room is entered. For a map combat that is the
//! state entering the fight. For a combat an **event** starts it is not: the
//! game first enters an `EventRoom`, runs the event's opening, and runs the
//! option the player picked, and only that option's body calls
//! `EventModel::EnterCombatWithoutExitingEvent` (`0x8068c`). Whatever the room
//! entry and the option did to the player happened after the save and before
//! the fight. The capture's decoded run (`--capture-run`) is the same pre-node
//! snapshot, so it does not help either.
//!
//! Before this module the entry built every event root from that snapshot and
//! admitted it. `ZU7KKADQBNCR` node 10 (Dense Vegetation) is the witness: the
//! Rest option heals before the fight, so native opens at 29 HP where the save
//! says 8, and the root was admitted anyway. That is an I5 hole — the root must
//! be exact or refused — and the census could only notice it after the fact,
//! at the capture's opening checkpoint.
//!
//! # The rule
//!
//! An event root is admitted only when [`EVENT_COMBATS`] names its encounter
//! with an IL-read option chain whose every pre-combat effect is modeled here.
//! Anything else refuses by name: an encounter the table does not know, an
//! option chain nobody read, a listener of a hook the chain fires, or a run
//! modifier. It is an explicit allow-list, not a blanket rule.
//!
//! # Which encounters an event starts
//!
//! Every generic instantiation of `EnterCombatWithoutExitingEvent<T>` inside
//! `MegaCrit.Sts2.Core.Models.Events` (a MethodSpec scan of the v0.111.0
//! assembly, sha256 `9cb4f1ad…`) names one of exactly the seven
//! `*_EVENT_ENCOUNTER` keys this crate registers:
//!
//! | event | option chain | call site |
//! |---|---|---|
//! | `BattlewornDummy` | `Setting1` / `Setting2` / `Setting3` | `0xc70c6` / `0xc70de` / `0xc70f6` IL_000d |
//! | `DenseVegetation` | `Rest` → `Fight` | `0xc8537` IL_0008 |
//! | `FakeMerchant` | `FoulPotionThrown` | `<FoulPotionThrown>d__23` `0x37c738` IL_014b |
//! | `TheLanternKey` | `KeepTheKey` → `Fight` | `0xd0320` IL_0052 |
//! | `PunchOff` | `TakeThem` → `Fight` | `0xcc278` IL_0054 |
//!
//! (`TheArchitect::get_CanonicalEncounter` names a `TheArchitectEventEncounter`
//! but never enters combat with it; the crate registers no such key, so it
//! would refuse here as an unknown event encounter anyway.)
//!
//! # What every event path shares: the room entry
//!
//! The `EventRoom` is entered through `Hook::BeforeRoomEntered` (`0x10484c`)
//! and `Hook::AfterRoomEntered` (`0x104898`) before any option runs, and the
//! opening models neither for an `EventRoom`. Rather than read each of their
//! bodies against an `EventRoom`, an event root refuses when the player holds
//! any model that overrides either hook ([`ROOM_ENTRY_LISTENERS`], the complete
//! override set in the assembly's MethodDef table), except the listeners whose
//! body is IL-read to return at a room-type guard the `EventRoom` fails
//! ([`EVENT_ROOM_INERT_LISTENERS`], #3351). `EventModel::BeforeEventStarted`
//! (`0x805d2`) and `AfterEventStarted` (`0x805d9`) are both a bare
//! `Task.CompletedTask`, so an event that does not override them adds nothing
//! there; only `FakeMerchant` and `PunchOff` of the five do.
//!
//! Run modifiers are hook listeners too (`Modifiers.Murderous` and `Terminal`
//! override `AfterRoomEntered`; `Modifiers.NightTerrors` overrides the rest-site
//! hooks), and the entry does not otherwise model them, so any modifier on an
//! event root refuses.

use crate::entry::deck::DeckEntry;
use crate::entry::refusal::EntryRefusal;

/// What an admitted event option does to the player before the fight.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreCombat {
    /// The option's body enters combat directly; nothing to apply.
    Nothing,
    /// `PlayerCmd::MimicRestSiteHeal` — see [`mimic_rest_site_heal`].
    MimicRestSiteHeal,
}

/// One event encounter's verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Verdict {
    Admitted(PreCombat),
    /// The option chain has a pre-combat effect this module does not model.
    Refused(&'static str),
}

/// One row of the allow-list.
#[derive(Clone, Copy, Debug)]
pub struct EventCombat {
    pub encounter: &'static str,
    /// The `EventModel` whose option starts the fight.
    pub event: &'static str,
    pub verdict: Verdict,
}

/// The allow-list: every registered event encounter, with the IL-read verdict
/// on the option chain that starts it.
///
/// * **Battleworn Dummy (V1-V3).** `BattlewornDummy::Setting1`/`2`/`3`
///   (`0xc70c6`/`0xc70de`/`0xc70f6`) are each `Encounter<T>`,
///   `Array.Empty<Reward>`, `EnterCombatWithoutExitingEvent` and return — no
///   effect on the player. `GenerateInitialOptions` (`0xc6f8c`) only writes the
///   three display HP `DynamicVars`. The event overrides neither start hook.
/// * **The Lantern Key (Mysterious Knight).** `KeepTheKey` (`0xd02e7`) only
///   sets the next page. `Fight` (`0xd0320`) builds the post-combat reward list
///   around `RunState::CreateCard<LanternKey>` (IL_003d; `RunState::CreateCard`
///   `0x4e904` is `ToMutable`, `AddCard`, `AfterCreated`, and `LanternKey` does
///   not override the no-op `CardModel::AfterCreated` `0x7d3aa`) and enters
///   combat at IL_0052. The card goes into a reward, never a combat pile.
/// * **Dense Vegetation.** `<Rest>d__8::MoveNext` (`0x37acb0`) calls
///   `PlayerCmd::MimicRestSiteHeal(owner, false)` at IL_002b, then only plays
///   audio and sets the page whose one option is `Fight` (`0xc8537`, enters
///   combat at IL_0008). `TrudgeOn` ends the event without a fight. Modeled by
///   [`mimic_rest_site_heal`].
/// * **Fake Merchant.** The fight starts from `<FoulPotionThrown>d__23`
///   (`0x37c738`, IL_014b): the player threw a Foul Potion at a merchant whose
///   stock `BeforeEventStarted` (`0xc94f8`) rolled on the event's RNG and which
///   they may have bought from first. Neither the potion spent nor any purchase
///   is in the pre-event save.
/// * **Punch Off.** `AfterEventStarted` (`0xcc13b`) starts `PunchEachOther`
///   running beside the event, and `Fight` (`0xcc278`) builds two rewards
///   before entering combat. Neither body has been read to the end, and the
///   corpus has no Punch Off fight, so it refuses.
pub const EVENT_COMBATS: [EventCombat; 7] = [
    EventCombat {
        encounter: "ENCOUNTER.BATTLEWORN_DUMMY_EVENT_V1_ENCOUNTER",
        event: "BattlewornDummy",
        verdict: Verdict::Admitted(PreCombat::Nothing),
    },
    EventCombat {
        encounter: "ENCOUNTER.BATTLEWORN_DUMMY_EVENT_V2_ENCOUNTER",
        event: "BattlewornDummy",
        verdict: Verdict::Admitted(PreCombat::Nothing),
    },
    EventCombat {
        encounter: "ENCOUNTER.BATTLEWORN_DUMMY_EVENT_V3_ENCOUNTER",
        event: "BattlewornDummy",
        verdict: Verdict::Admitted(PreCombat::Nothing),
    },
    EventCombat {
        encounter: "ENCOUNTER.MYSTERIOUS_KNIGHT_EVENT_ENCOUNTER",
        event: "TheLanternKey",
        verdict: Verdict::Admitted(PreCombat::Nothing),
    },
    EventCombat {
        encounter: "ENCOUNTER.DENSE_VEGETATION_EVENT_ENCOUNTER",
        event: "DenseVegetation",
        verdict: Verdict::Admitted(PreCombat::MimicRestSiteHeal),
    },
    EventCombat {
        encounter: "ENCOUNTER.FAKE_MERCHANT_EVENT_ENCOUNTER",
        event: "FakeMerchant",
        verdict: Verdict::Refused(
            "the fight starts when a Foul Potion is thrown (<FoulPotionThrown>d__23 \
             0x37c738 IL_014b) at a merchant the player may have bought from first; \
             the spent potion and any purchase are not in the pre-event save",
        ),
    },
    EventCombat {
        encounter: "ENCOUNTER.PUNCH_OFF_EVENT_ENCOUNTER",
        event: "PunchOff",
        verdict: Verdict::Refused(
            "AfterEventStarted (0xcc13b) runs PunchEachOther beside the event and \
             Fight (0xcc278) builds two rewards before combat; neither body is read",
        ),
    },
];

/// The marker every event encounter key carries (`..._EVENT_ENCOUNTER`, or
/// `..._EVENT_V<n>_ENCOUNTER` for the Battleworn Dummy). A key with it that
/// [`EVENT_COMBATS`] does not name refuses rather than being taken for a map
/// combat; no map encounter key contains it.
const EVENT_ENCOUNTER_MARKER: &str = "_EVENT_";

/// Every model the v0.111.0 assembly declares an override of
/// `AbstractModel::BeforeRoomEntered` (`0x7a197`) or `AfterRoomEntered`
/// (`0x7a19e`) on, that a player can hold: `(IL type name, model id)`.
///
/// The two `Achievements` overrides only reset their own tracking fields
/// (`SkillIronclad1Achievement` `0xf4f65` stores `_cardsExhaustedThisCombat = 0`;
/// `SkillSilent1Achievement` `0xf5202` clears `_firstCardOnStack` and
/// `_slyCardsPlayed`), which no combat state reads, and
/// the power/monster overriders cannot exist outside combat, so they are not
/// listed. `Modifiers.Murderous`/`Terminal` are covered by the modifier rule.
pub const ROOM_ENTRY_LISTENERS: [(&str, &str); 34] = [
    ("BigMushroom", "RELIC.BIG_MUSHROOM"),
    ("BronzeScales", "RELIC.BRONZE_SCALES"),
    ("BurningSticks", "RELIC.BURNING_STICKS"),
    ("DataDisk", "RELIC.DATA_DISK"),
    ("DivineRight", "RELIC.DIVINE_RIGHT"),
    ("EmberTea", "RELIC.EMBER_TEA"),
    ("EternalFeather", "RELIC.ETERNAL_FEATHER"),
    ("FakeVenerableTeaSet", "RELIC.FAKE_VENERABLE_TEA_SET"),
    ("GhostSeed", "RELIC.GHOST_SEED"),
    ("Girya", "RELIC.GIRYA"),
    ("Gorget", "RELIC.GORGET"),
    ("LavaLamp", "RELIC.LAVA_LAMP"),
    ("LordsParasol", "RELIC.LORDS_PARASOL"),
    ("MawBank", "RELIC.MAW_BANK"),
    ("MealTicket", "RELIC.MEAL_TICKET"),
    ("Metronome", "RELIC.METRONOME"),
    ("OddlySmoothStone", "RELIC.ODDLY_SMOOTH_STONE"),
    ("Pantograph", "RELIC.PANTOGRAPH"),
    ("Permafrost", "RELIC.PERMAFROST"),
    ("PhilosophersStone", "RELIC.PHILOSOPHERS_STONE"),
    ("Planisphere", "RELIC.PLANISPHERE"),
    ("RedSkull", "RELIC.RED_SKULL"),
    ("RegalPillow", "RELIC.REGAL_PILLOW"),
    ("SilverCrucible", "RELIC.SILVER_CRUCIBLE"),
    ("SlingOfCourage", "RELIC.SLING_OF_COURAGE"),
    ("StoneCalendar", "RELIC.STONE_CALENDAR"),
    ("StoneCracker", "RELIC.STONE_CRACKER"),
    ("SwordOfJade", "RELIC.SWORD_OF_JADE"),
    ("ThrowingAxe", "RELIC.THROWING_AXE"),
    ("Vajra", "RELIC.VAJRA"),
    ("VelvetChoker", "RELIC.VELVET_CHOKER"),
    ("VenerableTeaSet", "RELIC.VENERABLE_TEA_SET"),
    ("WingedBoots", "RELIC.WINGED_BOOTS"),
    // `Cards.Dowsing::BeforeRoomEntered` (`0xde13c`): a deck card.
    ("Dowsing", "CARD.DOWSING"),
];

/// The [`ROOM_ENTRY_LISTENERS`] whose room-entry body is IL-read to be an
/// exact no-op on the `EventRoom` (#3351, #3353): `(IL type name, model id, the
/// room test its body fails on the `EventRoom`, or what an unguarded body
/// touches)`. The gate lets these through; every other listener still refuses.
///
/// IL read on the v0.111.0 DLL (sha256 `9cb4f1ad…`). Each relic declares only
/// `AfterRoomEntered` among the room-entry and combat-start hooks (its other
/// members are `get_Rarity`, `get_CanonicalVars`, `get_ExtraHoverTips` and the
/// constructor), and each `MoveNext` opens with the async resume test
/// (`IL_001b`, taken only on re-entry after an `await`) and then a room-type
/// guard whose failure `leave`s to `SetResult` with nothing done:
///
/// * `BronzeScales/<AfterRoomEntered>d__6::MoveNext` (`0x320b74`):
///   `room isinst CombatRoom` at `IL_0023`, `leave` at `IL_002a`; the
///   `ThornsPower` apply is after it (`IL_0067`).
/// * `OddlySmoothStone/<AfterRoomEntered>d__6::MoveNext` (`0x32bb9c`):
///   `isinst CombatRoom` at `IL_0023`, `leave` at `IL_002a`; the Dexterity
///   apply is at `IL_0062`.
/// * `Gorget/<AfterRoomEntered>d__6::MoveNext` (`0x326064`): `isinst
///   CombatRoom` at `IL_0023`, `leave` at `IL_002a`; the `PlatingPower` apply
///   is at `IL_0067`.
/// * `EternalFeather/<AfterRoomEntered>d__4::MoveNext` (`0x323ea8`): `isinst
///   RestSiteRoom` at `IL_0026`, `leave` at `IL_002d`; the heal is at
///   `IL_0093`. Dense Vegetation's rest (`PlayerCmd::MimicRestSiteHeal`)
///   enters no room, so it does not reach this body either.
///
/// `EventRoom` extends `AbstractRoom` directly (TypeDef `Extends`), so it is
/// neither a `CombatRoom` nor a `RestSiteRoom`, and all four bodies return at
/// the guard for the event's room entry.
///
/// The fight itself is then entered as an ordinary room.
/// `EventCombatSynchronizer::EnterCombat` (`0x6d3d8`) builds a `CombatRoom`
/// (`IL_021b`) and passes it to `RunManager::EnterRoomWithoutExitingCurrentRoom`
/// (`IL_02d7`), whose `MoveNext` (`0x30ca70`) calls `EnterRoomInternal`
/// (`IL_017c`). That is the same `<EnterRoomInternal>d__203::MoveNext`
/// (`0x30c7f4`) a map combat enters through `EnterRoom` (`0x30c2d8`,
/// `IL_0087`): `Hook::BeforeRoomEntered` at `IL_00c5`, then `room.Enter`.
/// `CombatRoom`'s `<EnterInternal>d__40::MoveNext` (`0x31078c`) calls
/// `StartCombat` at `IL_0109` on every path but the pre-finished one, and
/// `<StartCombat>d__46` fires `Hook::AfterRoomEntered` with the `CombatRoom`.
/// So the Thorns, Dexterity and Plating these relics apply at an event-started
/// fight are the ones the opening already seeds for every root
/// (`entry::opening`'s owner-state table), and Eternal Feather stays inert
/// there as it is for a map combat.
///
/// # The rest of the sweep (#3353)
///
/// Every other relic in [`ROOM_ENTRY_LISTENERS`] was read the same way. None of
/// them overrides `BeforeRoomEntered` (only `Cards.Dowsing` does), so the
/// `EventRoom` entry reaches only their `AfterRoomEntered`. The third column
/// names the room test the body fails on an `EventRoom`, or, for the four
/// unguarded bodies, what they touch.
///
/// **A `CombatRoom` guard** (`room isinst CombatRoom`, returning `Task.CompletedTask`
/// or `leave`-ing to `SetResult` on failure, before any write):
///
/// * `BurningSticks::AfterRoomEntered` (`0x91a9a`): `isinst` `IL_0002`, `ret`
///   `IL_000e`; the `WasUsedThisCombat`/`Status` writes follow.
/// * `DataDisk/<AfterRoomEntered>d__6::MoveNext` (`0x322798`): `IL_0023`,
///   `leave` `IL_002a`.
/// * `DivineRight/<AfterRoomEntered>d__4::MoveNext` (`0x3232c0`): `IL_0023`,
///   `leave` `IL_002a`; `GainStars` is at `IL_0045`.
/// * `EmberTea/<AfterRoomEntered>d__17::MoveNext` (`0x3239cc`): an
///   `IsUsedUp` exit (`IL_001e`, `leave` `IL_0025`), then `isinst` `IL_0030`,
///   `leave` `IL_0037`; `CombatsLeft` is only decremented after the apply
///   (`IL_00c8`).
/// * `GhostSeed::AfterRoomEntered` (`0x9453c`): `isinst` `IL_000d`, `ret`
///   `IL_0019`, before the `AllCards` walk.
/// * `Girya/<AfterRoomEntered>d__14::MoveNext` (`0x325a60`): a `TimesLifted > 0`
///   exit (`IL_0021`, `leave` `IL_0029`), then `isinst` `IL_0034`, `leave`
///   `IL_003b`.
/// * `Metronome::AfterRoomEntered` (`0x96d7f`): `isinst` `IL_0002`, `ret`
///   `IL_000e`, before `OrbsChanneled = 0`.
/// * `Permafrost::AfterRoomEntered` (`0x9931c`): `isinst` `IL_0002`, `ret`
///   `IL_000e`, before `ActivatedThisCombat = false`.
/// * `PhilosophersStone/<AfterRoomEntered>d__8::MoveNext` (`0x32dff0`):
///   `isinst` `IL_0026`, `leave` `IL_002d`.
/// * `RedSkull/<AfterRoomEntered>d__11::MoveNext` (`0x32f6bc`): `isinst`
///   `IL_0023`, `leave` `IL_002a`, before `ModifyStrengthIfNecessary`
///   (`IL_002d`). (Its `AfterCurrentHpChanged` still refuses Dense Vegetation's
///   rest heal through [`REST_HEAL_LISTENERS`].)
/// * `StoneCalendar::AfterRoomEntered` (`0x9befb`): `isinst` `IL_0002`, `ret`
///   `IL_000e`.
/// * `StoneCracker/<AfterRoomEntered>d__4::MoveNext` (`0x331b0c`): `isinst`
///   `IL_0026`, `leave` `IL_002d`, before the draw-pile upgrade.
/// * `SwordOfJade/<AfterRoomEntered>d__6::MoveNext` (`0x33207c`): `isinst`
///   `IL_0023`, `leave` `IL_002a`.
/// * `ThrowingAxe::AfterRoomEntered` (`0x9c79c`): `isinst` `IL_0002`, `ret`
///   `IL_000e`, before `UsedThisCombat = false`.
/// * `Vajra/<AfterRoomEntered>d__6::MoveNext` (`0x333abc`): `isinst`
///   `IL_0023`, `leave` `IL_002a`.
/// * `VelvetChoker::AfterRoomEntered` (`0x9d9ce`): `isinst` `IL_0002`, `ret`
///   `IL_000e`, before `_cardsPlayedThisTurn = 0`.
///
/// **Another room-type guard** the `EventRoom` fails:
///
/// * `FakeVenerableTeaSet::AfterRoomEntered` (`0x93823`) and
///   `VenerableTeaSet::AfterRoomEntered` (`0x9da89`): `isinst RestSiteRoom`
///   `IL_0002`, `ret` `IL_000e`, before `GainEnergyInNextCombat = true`.
/// * `LordsParasol::AfterRoomEntered` (`0x96644`): `isinst MerchantRoom`
///   `IL_000d`, `ret` `IL_001b`, before `PurchaseEverything`.
/// * `MealTicket/<AfterRoomEntered>d__5::MoveNext` (`0x32a63c`): an `IsDead`
///   exit (`IL_0028`, `leave` `IL_002f`), then `isinst MerchantRoom` `IL_003a`,
///   `leave` `IL_0041`, before the heal (`IL_0068`).
/// * `SilverCrucible::AfterRoomEntered` (`0x9b8b0`): `isinst TreasureRoom`
///   `IL_000d`, `brfalse` `IL_0012` to the `ret` at `IL_0024`, before
///   `TreasureRoomsEntered += 1`.
/// * `SlingOfCourage/<AfterRoomEntered>d__6::MoveNext` (`0x331284`):
///   `room.RoomType` `IL_0023` compared with `2` (`IL_0028`, `beq` `IL_0029`),
///   `leave` `IL_002b`. `EventRoom::get_RoomType` (`0x58e37`) is `ldc.i4.6`.
///
/// **No guard, and nothing the fight can see:**
///
/// * `Pantograph::AfterRoomEntered` (`0x98cb4`) sets `Status` (`IL_005c`) from
///   the owner's `IsDead` (`IL_0017`) and whether the boss point's parents
///   contain `CurrentMapPoint` (`IL_004e`). `RegalPillow::AfterRoomEntered`
///   (`0x9a5c0`) sets `Status` (`IL_000e`) to `room isinst RestSiteRoom`
///   (`IL_0003`). `RelicModel::get_Status` is read only by
///   `RelicModel::UpdateTexture`, `NRelicInventoryHolder::RefreshStatus` and
///   `PaelsFlesh` on its own instance (a whole-DLL call scan), so both writes
///   are display state. The fight's own `CombatRoom` entry reruns both bodies
///   anyway. (Regal Pillow's rest-heal hooks still refuse Dense Vegetation
///   through [`REST_HEAL_LISTENERS`].)
/// * `LavaLamp::AfterRoomEntered` (`0x95f87`) sets `TookDamageThisCombat =
///   false` (`IL_0003`) unconditionally. The fight's `CombatRoom` entry writes
///   the same `false` again before any combat hook runs, so the event's write
///   leaves no trace.
/// * `BigMushroom::AfterRoomEntered` (`0x90a9b`) calls `Grow` (`0x90ae5`):
///   `NCombatRoom::get_Instance` (`IL_0001`, returning at `IL_000a` when null)
///   and `NCreature::ScaleTo` (`IL_002e`). That is a scene-node scale and no
///   model state.
///
/// These stay refused, because their `EventRoom` body does something:
///
/// * `MawBank/<AfterRoomEntered>d__12::MoveNext` (`0x32a534`) gains gold
///   (`IL_0064`) when `room` is the run's `BaseRoom` (`IL_0028`/`IL_0033`),
///   which the `EventRoom` is.
/// * `Planisphere/<AfterRoomEntered>d__5::MoveNext` (`0x32e318`) heals
///   (`IL_009a`) when `CurrentMapPoint.PointType == 1` (`IL_004e`), the `?`
///   node an event is entered from.
/// * `WingedBoots::AfterRoomEntered` (`0x9e230`) counts a free-travel move
///   (`TimesUsed += 1`, `IL_00c6`) on any room.
/// * `Cards.Dowsing::BeforeRoomEntered` (`0xde13c`) counts rooms entered.
pub const EVENT_ROOM_INERT_LISTENERS: [(&str, &str, &str); 30] = [
    (
        "BigMushroom",
        "RELIC.BIG_MUSHROOM",
        "NCombatRoom visual only",
    ),
    ("BronzeScales", "RELIC.BRONZE_SCALES", "CombatRoom"),
    ("BurningSticks", "RELIC.BURNING_STICKS", "CombatRoom"),
    ("DataDisk", "RELIC.DATA_DISK", "CombatRoom"),
    ("DivineRight", "RELIC.DIVINE_RIGHT", "CombatRoom"),
    ("EmberTea", "RELIC.EMBER_TEA", "CombatRoom"),
    ("EternalFeather", "RELIC.ETERNAL_FEATHER", "RestSiteRoom"),
    (
        "FakeVenerableTeaSet",
        "RELIC.FAKE_VENERABLE_TEA_SET",
        "RestSiteRoom",
    ),
    ("GhostSeed", "RELIC.GHOST_SEED", "CombatRoom"),
    ("Girya", "RELIC.GIRYA", "CombatRoom"),
    ("Gorget", "RELIC.GORGET", "CombatRoom"),
    (
        "LavaLamp",
        "RELIC.LAVA_LAMP",
        "TookDamageThisCombat, rewritten at the CombatRoom entry",
    ),
    ("LordsParasol", "RELIC.LORDS_PARASOL", "MerchantRoom"),
    ("MealTicket", "RELIC.MEAL_TICKET", "MerchantRoom"),
    ("Metronome", "RELIC.METRONOME", "CombatRoom"),
    ("OddlySmoothStone", "RELIC.ODDLY_SMOOTH_STONE", "CombatRoom"),
    ("Pantograph", "RELIC.PANTOGRAPH", "Status (display only)"),
    ("Permafrost", "RELIC.PERMAFROST", "CombatRoom"),
    (
        "PhilosophersStone",
        "RELIC.PHILOSOPHERS_STONE",
        "CombatRoom",
    ),
    ("RedSkull", "RELIC.RED_SKULL", "CombatRoom"),
    ("RegalPillow", "RELIC.REGAL_PILLOW", "Status (display only)"),
    ("SilverCrucible", "RELIC.SILVER_CRUCIBLE", "TreasureRoom"),
    ("SlingOfCourage", "RELIC.SLING_OF_COURAGE", "RoomType 2"),
    ("StoneCalendar", "RELIC.STONE_CALENDAR", "CombatRoom"),
    ("StoneCracker", "RELIC.STONE_CRACKER", "CombatRoom"),
    ("SwordOfJade", "RELIC.SWORD_OF_JADE", "CombatRoom"),
    ("ThrowingAxe", "RELIC.THROWING_AXE", "CombatRoom"),
    ("Vajra", "RELIC.VAJRA", "CombatRoom"),
    ("VelvetChoker", "RELIC.VELVET_CHOKER", "CombatRoom"),
    ("VenerableTeaSet", "RELIC.VENERABLE_TEA_SET", "RestSiteRoom"),
];

/// The [`ROOM_ENTRY_LISTENERS`] whose `EventRoom` body is IL-read to act
/// (#3353), with what it does; see [`EVENT_ROOM_INERT_LISTENERS`] for the
/// reads. Together the two tables partition the listener set.
pub const EVENT_ROOM_ACTIVE_LISTENERS: [(&str, &str, &str); 4] = [
    ("MawBank", "RELIC.MAW_BANK", "gains gold on the BaseRoom"),
    ("Planisphere", "RELIC.PLANISPHERE", "heals on a ? map point"),
    (
        "WingedBoots",
        "RELIC.WINGED_BOOTS",
        "counts a free-travel move",
    ),
    ("Dowsing", "CARD.DOWSING", "counts rooms entered"),
];

/// Whether `id` is a room-entry listener the `EventRoom` entry leaves
/// untouched ([`EVENT_ROOM_INERT_LISTENERS`]).
fn inert_on_event_room(id: &str) -> bool {
    EVENT_ROOM_INERT_LISTENERS
        .iter()
        .any(|(_, inert, _)| *inert == id)
}

/// Every model the assembly declares an override of a hook that
/// `PlayerCmd::MimicRestSiteHeal` reaches: `(IL type name, model id, hook)`.
///
/// The chain, all read for this module:
///
/// * `<MimicRestSiteHeal>d__16::MoveNext` (`0x3eedc4`) calls
///   `HealRestSiteOption::ExecuteRestSiteHeal(player, true)` at IL_002a.
/// * `<ExecuteRestSiteHeal>d__13::MoveNext` (`0x3d841c`): `CreatureCmd::Heal`
///   with `HealRestSiteOption::GetHealAmount` (IL_0036/IL_003c), then
///   `Hook::AfterRestSiteHeal` (IL_00ad), then `Hook::ModifyRestSiteHealRewards`
///   into a fresh list (IL_0125) and `RewardsCmd::OfferCustom` on it (IL_0132).
/// * `GetHealAmount` (`0x11533a`) is `Hook::ModifyRestSiteHealAmount` over
///   `GetBaseHealAmount` (`0x1154ea`, `MaxHp * 0.3m`).
/// * `<Heal>d__20::MoveNext` (`0x3eb4b0`) heals through
///   `Creature::HealInternal` (IL_00f5) and ends in `Hook::AfterCurrentHpChanged`
///   (IL_0549).
/// * `OfferCustom` (`<OfferCustom>d__1` `0x3f1694`) offers a `RewardsSet` whose
///   `GenerateWithoutOffering` (`<GenerateWithoutOffering>d__32`) runs
///   `Hook::ModifyRewards` (IL_0092) and `Hook::AfterModifyingRewards`
///   (IL_00ed) even over a custom list.
///
/// Every listener of those six hooks refuses: its effect is either a player
/// choice the pre-event save cannot see (Tiny Mailbox's two `PotionReward`s,
/// `TryModifyRestSiteHealRewards` `0x9c88a` IL_000e/IL_001a, taken or not on a
/// rewards screen) or a body nobody ported here.
pub const REST_HEAL_LISTENERS: [(&str, &str, &str); 13] = [
    (
        "RegalPillow",
        "RELIC.REGAL_PILLOW",
        "ModifyRestSiteHealAmount",
    ),
    ("RegalPillow", "RELIC.REGAL_PILLOW", "AfterRestSiteHeal"),
    (
        "StoneHumidifier",
        "RELIC.STONE_HUMIDIFIER",
        "AfterRestSiteHeal",
    ),
    (
        "DreamCatcher",
        "RELIC.DREAM_CATCHER",
        "TryModifyRestSiteHealRewards",
    ),
    (
        "TinyMailbox",
        "RELIC.TINY_MAILBOX",
        "TryModifyRestSiteHealRewards",
    ),
    (
        "MeatOnTheBone",
        "RELIC.MEAT_ON_THE_BONE",
        "AfterCurrentHpChanged",
    ),
    ("RedSkull", "RELIC.RED_SKULL", "AfterCurrentHpChanged"),
    (
        "AmethystAubergine",
        "RELIC.AMETHYST_AUBERGINE",
        "TryModifyRewards",
    ),
    ("BlackStar", "RELIC.BLACK_STAR", "TryModifyRewards"),
    ("LavaRock", "RELIC.LAVA_ROCK", "TryModifyRewards"),
    ("PrayerWheel", "RELIC.PRAYER_WHEEL", "TryModifyRewards"),
    ("WhiteStar", "RELIC.WHITE_STAR", "TryModifyRewards"),
    (
        "WongosMysteryTicket",
        "RELIC.WONGOS_MYSTERY_TICKET",
        "TryModifyRewards",
    ),
];

/// The event-combat facts the gate reads.
#[derive(Clone, Copy, Debug)]
pub struct EventRootFacts<'a> {
    pub relics: &'a [String],
    pub deck: &'a [DeckEntry],
    pub modifiers: &'a [String],
    pub hp: i64,
    pub max_hp: i64,
}

/// The row [`EVENT_COMBATS`] holds for an encounter, `Ok(None)` for a map
/// combat, and a refusal for an event encounter the table does not name.
pub fn event_combat(encounter_id: &str) -> Result<Option<&'static EventCombat>, EntryRefusal> {
    if let Some(row) = EVENT_COMBATS
        .iter()
        .find(|row| row.encounter == encounter_id)
    {
        return Ok(Some(row));
    }
    if encounter_id.contains(EVENT_ENCOUNTER_MARKER) {
        return Err(EntryRefusal::EventCombatPathNotModeled {
            encounter: encounter_id.to_string(),
            event: "<unregistered>",
            reason: "no event option chain for this encounter has been read",
        });
    }
    Ok(None)
}

/// The HP entering the fight: the save's HP for a map combat, or the HP after
/// the admitted event option's pre-combat effect, or a named refusal.
pub fn hp_entering(encounter_id: &str, facts: &EventRootFacts<'_>) -> Result<i64, EntryRefusal> {
    let Some(row) = event_combat(encounter_id)? else {
        return Ok(facts.hp);
    };
    let pre_combat = match row.verdict {
        Verdict::Refused(reason) => {
            return Err(EntryRefusal::EventCombatPathNotModeled {
                encounter: encounter_id.to_string(),
                event: row.event,
                reason,
            });
        }
        Verdict::Admitted(pre_combat) => pre_combat,
    };
    if !facts.modifiers.is_empty() {
        return Err(EntryRefusal::EventCombatRunModifiers {
            encounter: encounter_id.to_string(),
            modifiers: facts.modifiers.to_vec(),
        });
    }
    let held = |id: &str| {
        facts.relics.iter().any(|relic| relic == id) || facts.deck.iter().any(|card| card.id == id)
    };
    if let Some((_, id)) = ROOM_ENTRY_LISTENERS
        .iter()
        .find(|(_, id)| held(id) && !inert_on_event_room(id))
    {
        return Err(EntryRefusal::EventCombatListenerNotModeled {
            encounter: encounter_id.to_string(),
            listener: (*id).to_string(),
            hook: "BeforeRoomEntered/AfterRoomEntered on the EventRoom",
        });
    }
    match pre_combat {
        PreCombat::Nothing => Ok(facts.hp),
        PreCombat::MimicRestSiteHeal => {
            if let Some((_, id, hook)) = REST_HEAL_LISTENERS.iter().find(|(_, id, _)| held(id)) {
                return Err(EntryRefusal::EventCombatListenerNotModeled {
                    encounter: encounter_id.to_string(),
                    listener: (*id).to_string(),
                    hook,
                });
            }
            Ok(mimic_rest_site_heal(facts.hp, facts.max_hp))
        }
    }
}

/// `PlayerCmd::MimicRestSiteHeal` with no listener of any hook it reaches
/// (see [`REST_HEAL_LISTENERS`] for the chain).
///
/// The amount is `HealRestSiteOption::GetBaseHealAmount` (`0x1154ea`):
/// `(decimal)MaxHp * new decimal(3, 0, 0, false, 1)` — `0.3m`, exact in
/// decimal — passed through an empty `ModifyRestSiteHealAmount` walk.
/// `CreatureCmd::Heal` (`<Heal>d__20` `0x3eb4b0`) hands it to
/// `Creature::HealInternal` (`0x11d6dc`), which calls
/// `SetCurrentHpInternal((decimal)CurrentHp + amount)`, and
/// `SetCurrentHpInternal` (`0x11d734`) stores
/// `(int)Math.Min(value, (decimal)MaxHp)`. `decimal -> int` truncates toward
/// zero, and every term is non-negative, so the new HP is
/// `min(hp + floor(3 * max_hp / 10), max_hp)` in integers.
///
/// The `IsEnding`/`!IsPlayer` early return (IL_0041-IL_005f) cannot fire for
/// the player outside combat.
pub fn mimic_rest_site_heal(hp: i64, max_hp: i64) -> i64 {
    (hp + (3 * max_hp) / 10).min(max_hp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{CardId, EncounterId, RelicId};

    fn facts<'a>(
        relics: &'a [String],
        deck: &'a [DeckEntry],
        modifiers: &'a [String],
    ) -> EventRootFacts<'a> {
        EventRootFacts {
            relics,
            deck,
            modifiers,
            hp: 8,
            max_hp: 70,
        }
    }

    fn card(id: &str) -> DeckEntry {
        DeckEntry {
            id: id.to_string(),
            upgrade_level: 0,
            props: None,
            enchantment: None,
            enchant_amount: None,
        }
    }

    const DV: &str = "ENCOUNTER.DENSE_VEGETATION_EVENT_ENCOUNTER";
    const BW2: &str = "ENCOUNTER.BATTLEWORN_DUMMY_EVENT_V2_ENCOUNTER";
    const KNIGHT: &str = "ENCOUNTER.MYSTERIOUS_KNIGHT_EVENT_ENCOUNTER";

    fn class(result: Result<i64, EntryRefusal>) -> &'static str {
        result.expect_err("the event root refuses").class()
    }

    #[test]
    fn every_registered_event_encounter_is_on_the_allow_list() {
        let registered: Vec<String> = EncounterId::NAMES
            .iter()
            .filter(|name| name.contains(EVENT_ENCOUNTER_MARKER))
            .map(|name| format!("ENCOUNTER.{name}"))
            .collect();
        let mut listed: Vec<String> = EVENT_COMBATS
            .iter()
            .map(|row| row.encounter.to_string())
            .collect();
        listed.sort();
        let mut registered = registered;
        registered.sort();
        assert_eq!(listed, registered);
    }

    fn snake(il: &str) -> String {
        let mut out = String::new();
        for (index, ch) in il.chars().enumerate() {
            if ch.is_ascii_uppercase() && index > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_uppercase());
        }
        out
    }

    #[test]
    fn every_listener_id_is_its_il_type_name_and_a_known_model() {
        for (il, id) in ROOM_ENTRY_LISTENERS {
            let (category, name) = id.split_once('.').unwrap();
            assert_eq!(name, snake(il), "{il}");
            match category {
                "RELIC" => assert!(RelicId::from_str(id).is_some(), "{id}"),
                "CARD" => assert!(CardId::from_str(name).is_some(), "{id}"),
                other => panic!("unexpected category {other}"),
            }
        }
        for (il, id, _) in REST_HEAL_LISTENERS {
            assert_eq!(id.strip_prefix("RELIC.").unwrap(), snake(il), "{il}");
            assert!(RelicId::from_str(id).is_some(), "{id}");
        }
    }

    #[test]
    fn a_map_combat_keeps_the_saved_hp() {
        let relics = ["RELIC.TINY_MAILBOX".to_string()];
        let modifiers = ["MODIFIER.X".to_string()];
        assert_eq!(
            hp_entering("ENCOUNTER.TOADPOLES_WEAK", &facts(&relics, &[], &modifiers)),
            Ok(8)
        );
    }

    #[test]
    fn an_unregistered_event_encounter_refuses_by_name() {
        let refusal = hp_entering(
            "ENCOUNTER.THE_ARCHITECT_EVENT_ENCOUNTER",
            &facts(&[], &[], &[]),
        )
        .expect_err("refuses");
        assert_eq!(refusal.class(), "event_combat_path_not_modeled");
        assert!(refusal.to_string().contains("<unregistered>"));
    }

    #[test]
    fn a_direct_option_admits_the_saved_hp() {
        for encounter in [
            "ENCOUNTER.BATTLEWORN_DUMMY_EVENT_V1_ENCOUNTER",
            BW2,
            "ENCOUNTER.BATTLEWORN_DUMMY_EVENT_V3_ENCOUNTER",
            KNIGHT,
        ] {
            assert_eq!(
                hp_entering(encounter, &facts(&[], &[], &[])),
                Ok(8),
                "{encounter}"
            );
        }
    }

    #[test]
    fn an_unread_option_chain_refuses_by_name() {
        for (encounter, event) in [
            ("ENCOUNTER.FAKE_MERCHANT_EVENT_ENCOUNTER", "FakeMerchant"),
            ("ENCOUNTER.PUNCH_OFF_EVENT_ENCOUNTER", "PunchOff"),
        ] {
            let refusal = hp_entering(encounter, &facts(&[], &[], &[])).expect_err("refuses");
            assert_eq!(refusal.class(), "event_combat_path_not_modeled");
            assert!(refusal.to_string().contains(event));
        }
    }

    #[test]
    fn a_run_modifier_refuses_an_event_root() {
        let modifiers = ["MODIFIER.NIGHT_TERRORS".to_string()];
        assert_eq!(
            class(hp_entering(DV, &facts(&[], &[], &modifiers))),
            "event_combat_run_modifiers"
        );
        assert_eq!(
            class(hp_entering(BW2, &facts(&[], &[], &modifiers))),
            "event_combat_run_modifiers"
        );
    }

    /// The relics and deck that hold one listener (a card goes in the deck).
    fn holding(id: &str) -> (Vec<String>, Vec<DeckEntry>) {
        if id.starts_with("CARD.") {
            (vec!["RELIC.BURNING_BLOOD".to_string()], vec![card(id)])
        } else {
            (
                vec!["RELIC.BURNING_BLOOD".to_string(), id.to_string()],
                vec![],
            )
        }
    }

    /// Each listener whose `EventRoom` body acts (#3353) refuses every
    /// admitted event, naming itself.
    #[test]
    fn a_room_entry_listener_refuses_every_admitted_event() {
        for (_, id, _) in EVENT_ROOM_ACTIVE_LISTENERS {
            let (relics, deck) = holding(id);
            for encounter in [BW2, KNIGHT, DV] {
                let refusal =
                    hp_entering(encounter, &facts(&relics, &deck, &[])).expect_err("refuses");
                assert_eq!(refusal.class(), "event_combat_listener_not_modeled");
                assert!(refusal.to_string().contains(id), "{id} on {encounter}");
            }
        }
    }

    /// #3353: every room-entry listener was read, and sits in exactly one of
    /// the inert and active tables.
    #[test]
    fn the_inert_and_active_tables_partition_the_room_entry_listeners() {
        for (il, id) in ROOM_ENTRY_LISTENERS {
            let inert = EVENT_ROOM_INERT_LISTENERS
                .iter()
                .filter(|(i, m, _)| (*i, *m) == (il, id))
                .count();
            let active = EVENT_ROOM_ACTIVE_LISTENERS
                .iter()
                .filter(|(i, m, _)| (*i, *m) == (il, id))
                .count();
            assert_eq!(inert + active, 1, "{id}");
        }
        assert_eq!(
            EVENT_ROOM_INERT_LISTENERS.len() + EVENT_ROOM_ACTIVE_LISTENERS.len(),
            ROOM_ENTRY_LISTENERS.len()
        );
    }

    #[test]
    fn every_event_room_inert_listener_is_a_room_entry_listener() {
        for (il, id, _) in EVENT_ROOM_INERT_LISTENERS {
            assert!(
                ROOM_ENTRY_LISTENERS.contains(&(il, id)),
                "{id} is not a room-entry listener"
            );
        }
    }

    /// One witness per relic (#3351, #3353): each is admitted on every
    /// allow-listed event, with the saved HP (or the rest heal) unchanged,
    /// alone and beside the others. A relic that also listens on the rest
    /// heal (Regal Pillow, Red Skull) still refuses Dense Vegetation, naming
    /// its rest-heal hook rather than the room entry.
    #[test]
    fn an_event_room_inert_listener_admits_every_admitted_event() {
        for (_, id, _) in EVENT_ROOM_INERT_LISTENERS {
            let relics = ["RELIC.BURNING_BLOOD".to_string(), id.to_string()];
            for encounter in [BW2, KNIGHT] {
                assert_eq!(
                    hp_entering(encounter, &facts(&relics, &[], &[])),
                    Ok(8),
                    "{id} on {encounter}"
                );
            }
            match REST_HEAL_LISTENERS.iter().find(|(_, rest, _)| *rest == id) {
                // Eternal Feather's rest-site heal is not reached by the rest.
                None => assert_eq!(hp_entering(DV, &facts(&relics, &[], &[])), Ok(29), "{id}"),
                Some((_, _, hook)) => {
                    let refusal = hp_entering(DV, &facts(&relics, &[], &[])).expect_err("refuses");
                    assert_eq!(refusal.class(), "event_combat_listener_not_modeled");
                    assert!(refusal.to_string().contains(hook), "{id}");
                    assert!(!refusal.to_string().contains("EventRoom"), "{id}");
                }
            }
        }
        // All thirty together, minus the two rest-heal listeners on DV.
        let all: Vec<String> = EVENT_ROOM_INERT_LISTENERS
            .iter()
            .map(|(_, id, _)| id.to_string())
            .collect();
        assert_eq!(hp_entering(KNIGHT, &facts(&all, &[], &[])), Ok(8));
        assert_eq!(hp_entering(BW2, &facts(&all, &[], &[])), Ok(8));
        let off_rest: Vec<String> = all
            .iter()
            .filter(|id| !REST_HEAL_LISTENERS.iter().any(|(_, rest, _)| rest == id))
            .cloned()
            .collect();
        assert_eq!(off_rest.len(), all.len() - 2);
        assert_eq!(hp_entering(DV, &facts(&off_rest, &[], &[])), Ok(29));
    }

    /// The exemption is per relic: an inert listener beside any listener
    /// whose `EventRoom` body acts still refuses, naming the other one, and
    /// so does the whole inert table held together beside it.
    #[test]
    fn an_inert_listener_does_not_shield_another_listener() {
        for (_, active, _) in EVENT_ROOM_ACTIVE_LISTENERS {
            let (mut extra, deck) = holding(active);
            extra.retain(|id| id != "RELIC.BURNING_BLOOD");
            for (_, id, _) in EVENT_ROOM_INERT_LISTENERS {
                let mut relics = vec![id.to_string()];
                relics.extend(extra.iter().cloned());
                let refusal = hp_entering(BW2, &facts(&relics, &deck, &[])).expect_err("refuses");
                assert_eq!(refusal.class(), "event_combat_listener_not_modeled");
                assert!(refusal.to_string().contains(active), "{id} beside {active}");
            }
            let mut all: Vec<String> = EVENT_ROOM_INERT_LISTENERS
                .iter()
                .map(|(_, id, _)| id.to_string())
                .collect();
            all.extend(extra.iter().cloned());
            let refusal = hp_entering(KNIGHT, &facts(&all, &deck, &[])).expect_err("refuses");
            assert!(refusal.to_string().contains(active), "all beside {active}");
        }
    }

    /// Tiny Mailbox stays refused on Dense Vegetation (its two potion rewards
    /// are picked on a rewards screen), and never mattered off the rest path.
    #[test]
    fn tiny_mailbox_still_refuses_dense_vegetation() {
        let relics = ["RELIC.TINY_MAILBOX".to_string()];
        let refusal = hp_entering(DV, &facts(&relics, &[], &[])).expect_err("refuses");
        assert_eq!(refusal.class(), "event_combat_listener_not_modeled");
        assert!(refusal.to_string().contains("TryModifyRestSiteHealRewards"));
        assert_eq!(hp_entering(KNIGHT, &facts(&relics, &[], &[])), Ok(8));
    }

    #[test]
    fn dense_vegetation_rest_heals_thirty_percent_truncated_and_capped() {
        let relics = ["RELIC.BURNING_BLOOD".to_string()];
        // ZU7KKADQBNCR node 10: 8 of 70 opens at 29 natively.
        assert_eq!(hp_entering(DV, &facts(&relics, &[], &[])), Ok(29));
        // 75 * 0.3 = 22.5; `(int)` truncates.
        assert_eq!(mimic_rest_site_heal(10, 75), 32);
        // `Math.Min` against MaxHp.
        assert_eq!(mimic_rest_site_heal(60, 70), 70);
        assert_eq!(mimic_rest_site_heal(70, 70), 70);
    }

    #[test]
    fn every_rest_heal_listener_refuses_dense_vegetation_but_not_a_direct_option() {
        for (_, id, hook) in REST_HEAL_LISTENERS {
            if ROOM_ENTRY_LISTENERS.iter().any(|(_, room)| *room == id) {
                continue; // refused by the room-entry rule first
            }
            let relics = [id.to_string()];
            let refusal = hp_entering(DV, &facts(&relics, &[], &[])).expect_err("refuses");
            assert_eq!(refusal.class(), "event_combat_listener_not_modeled", "{id}");
            assert!(refusal.to_string().contains(hook), "{id}");
            assert_eq!(hp_entering(BW2, &facts(&relics, &[], &[])), Ok(8), "{id}");
        }
    }
}

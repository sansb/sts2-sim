//! The non-`Shuffle` counters, from the parser's per-fight projection.
//!
//! A port of `solve_fight`'s pre-fight accounting: `shuffle_counter_caveats`,
//! `monsterai_caveats`, `niche_counter_entering`, `card_sel_counter_entering`,
//! `combat_targets_counter_entering`, `energy_costs_counter_entering`,
//! `combat_card_generation_counter_entering` and
//! `combat_orb_generation_counter_entering`. Every registry is transcribed
//! element for element and pinned to the frozen copy of its Python original
//! (`fixtures/frozen_python_run_counter_registries_v1.json`). The consumer
//! censuses behind them are the IL reads recorded in `STREAM_CONSUMERS.md`.
//!
//! The `Niche` registries have grown past that original. It kept a hand-made
//! spawner list, which missed an Ovicopter's eggs among others (#2997); they
//! are now held to a whole-DLL census, `fixtures/niche_consumer_census_v1.json`,
//! written by `tools/niche_consumer_census.py`, and the tests name every row
//! added since the freeze.
//!
//! The rule every stream follows: a counter is proved only by a prefix of
//! fights that held no consumer (or, for Stone Cracker, a consumer whose draw
//! count is a pure function of the recorded deck). Anything the `.run` does
//! not record (plays, triggers, generated cards) makes the answer unknown or a
//! labelled baseline, never a guess.
//!
//! Caveat text is reproduced character for character, including where Python
//! interpolated a `list` repr (`['RELIC.X', 'RELIC.Y']`), because the review
//! document stores it. The one exception, the `Niche` on-obtain `set` repr, is
//! rendered sorted (see [`super`]).

use std::collections::{BTreeSet, HashMap};
use std::sync::OnceLock;

use serde_json::Value;

use super::history::Fight;
use crate::content_tables::card_row;
use crate::ids::CardId;

/// `NICHE_ONOBTAIN_RELICS`: relics whose own hooks draw `Niche`.
///
/// The whole-DLL census (`fixtures/niche_consumer_census_v1.json`, #2997)
/// finds fifteen relic methods that call `RunRngSet::get_Niche` (`0x4ddeb`)
/// directly: fourteen of the frozen `solve_fight` registry's rows, plus
/// Distinguished Cape, which that registry missed
/// (`DistinguishedCape/<AfterObtained>d__9::MoveNext`, `0x322e20`, `IL_00cb`:
/// one `NextItem` per curse it adds). `RELIC.SERE_TALON` is the frozen
/// registry's fifteenth row and has no `get_Niche` site in this build; it
/// stays, because an extra row can only turn an exact claim into a baseline.
const NICHE_ONOBTAIN_RELICS: &[&str] = &[
    "RELIC.ASTROLABE",
    "RELIC.BEAUTIFUL_BRACELET",
    "RELIC.DISTINGUISHED_CAPE",
    "RELIC.FISHING_ROD",
    "RELIC.FRAGRANT_MUSHROOM",
    "RELIC.KALEIDOSCOPE",
    "RELIC.NEOWS_BONES",
    "RELIC.NEW_LEAF",
    "RELIC.PANDORAS_BOX",
    "RELIC.ROYAL_STAMP",
    "RELIC.SAND_CASTLE",
    "RELIC.SERE_TALON",
    "RELIC.WAR_HAMMER",
    "RELIC.WAR_PAINT",
    "RELIC.WHETSTONE",
    "RELIC.WING_CHARM",
];

/// `MONSTERAI_CONSUMERS`: Monsters whose intent rolls consume `MonsterAi`: the random-AI monsters of
/// `STREAM_CONSUMERS.md` (a `RandomBranchState` in the move graph) minus Phrog
/// Parasite, whose random node is unreachable, plus Decimillipede's revive
/// rolls. Matched as substrings of the recorded monster ids.
const MONSTERAI_CONSUMERS: &[&str] = &[
    "TWO_TAILED_RAT",
    "FLAIL_KNIGHT",
    "SPECTRAL_KNIGHT",
    "SOUL_NEXUS",
    "DECIMILLIPEDE",
    "EXOSKELETON",
    "FABRICATOR",
    "FAKE_MERCHANT",
    "FLYCONID",
    "FOGMOG",
    "FOSSIL_STALKER",
    "HUNTER_KILLER",
    "INKLET",
    "LEAF_SLIME_S",
    "MAWLER",
    "SCROLL_OF_BITING",
    "SLITHERING_STRANGLER",
    "SLUDGE_SPINNER",
    "THE_OBSCURA",
    "TWIG_SLIME_M",
];

/// `CARD_AFFLICTING_MONSTERS`: Monsters that add cards to the player's piles mid-combat (IL census
/// 2026-07-10: every `AddToCombatAndPreview` / `AddGeneratedCard*` caller, plus
/// Entomancer's Personal Hive Dazes and Wriggler). Each afflicted card in a
/// pre-reshuffle discard adds one unrecorded `Shuffle` draw.
const CARD_AFFLICTING_MONSTERS: &[&str] = &[
    "AEONGLASS",
    "CHOMPER",
    "ENTOMANCER",
    "EYE_WITH_TEETH",
    "HAUNTED_SHIP",
    "LEAF_SLIME_M",
    "LEAF_SLIME_S",
    "MECHA_KNIGHT",
    "MYTE",
    "NOISEBOT",
    "PHROG_PARASITE",
    "SLIMED_BERSERKER",
    "SOUL_FYSH",
    "TEST_SUBJECT",
    "THE_INSATIABLE",
    "TWIG_SLIME_M",
    "VANTOM",
    "WRIGGLER",
];

/// `PRIOR_FIGHT_DRAW_RELICS`: Modeled relics that draw, retain, auto-play, generate or exhaust at
/// prior-fight times the `.run` does not record, moving reshuffle boundaries
/// (Batches 68, 176-182, 188, 203; W173, W209, W215, W216 in the Python
/// registry). Biiig Hug is separate: it also inserts Soot at random positions.
const PRIOR_FIGHT_DRAW_RELICS: &[&str] = &[
    "RELIC.BAG_OF_PREPARATION",
    "RELIC.BIG_MUSHROOM",
    "RELIC.BLESSED_ANTLER",
    "RELIC.BOOMING_CONCH",
    "RELIC.BURNING_STICKS",
    "RELIC.CENTENNIAL_PUZZLE",
    "RELIC.CHOICES_PARADOX",
    "RELIC.CROSSBOW",
    "RELIC.FIDDLE",
    "RELIC.FUNERARY_MASK",
    "RELIC.GAMBLING_CHIP",
    "RELIC.GAME_PIECE",
    "RELIC.GHOST_SEED",
    "RELIC.GREMLIN_HORN",
    "RELIC.HISTORY_COURSE",
    "RELIC.IRON_CLUB",
    "RELIC.JOSS_PAPER",
    "RELIC.NINJA_SCROLL",
    "RELIC.ORANGE_DOUGH",
    "RELIC.PAELS_BLOOD",
    "RELIC.PENDULUM",
    "RELIC.POCKETWATCH",
    "RELIC.POLLINOUS_CORE",
    "RELIC.RINGING_TRIANGLE",
    "RELIC.RING_OF_THE_DRAKE",
    "RELIC.RING_OF_THE_SNAKE",
    "RELIC.RUNIC_PYRAMID",
    "RELIC.SNECKO_EYE",
    "RELIC.TEA_OF_DISCOURTESY",
    "RELIC.TOASTY_MITTENS",
    "RELIC.TOOLBOX",
    "RELIC.UNCEASING_TOP",
    "RELIC.VEXING_PUZZLEBOX",
    "RELIC.WHISPERING_EARRING",
];

/// `PRIOR_FIGHT_DRAW_POTIONS`: Potions whose recorded use drew, reshuffled, removed or replayed cards at an
/// unrecorded time or selection (Batches 65, 145).
const PRIOR_FIGHT_DRAW_POTIONS: &[&str] = &[
    "POTION.BOTTLED_POTENTIAL",
    "POTION.CLARITY",
    "POTION.CURE_ALL",
    "POTION.DISTILLED_CHAOS",
    "POTION.DROPLET_OF_PRECOGNITION",
    "POTION.GAMBLERS_BREW",
    "POTION.GLOWWATER_POTION",
    "POTION.LIQUID_MEMORIES",
    "POTION.SNECKO_OIL",
    "POTION.SOLDIERS_STEW",
    "POTION.SWIFT_POTION",
];

/// `PRIOR_FIGHT_SHUFFLE_CARDS`: Cards whose unrecorded play draws, auto-plays, inserts at random, generates,
/// removes or retains, and so can shift the run-lifetime `Shuffle` stream (the
/// per-card reasons are the Python registry's batch comments).
const PRIOR_FIGHT_SHUFFLE_CARDS: &[&str] = &[
    "ADAPTIVE_STRIKE",
    "ALL_FOR_ONE",
    "ANGER",
    "BEAT_DOWN",
    "BLADE_SYMPHONY",
    "BOLAS",
    "BOOST_AWAY",
    "BUNDLE_OF_JOY",
    "CALL_OF_THE_VOID",
    "CASCADE",
    "CATASTROPHE",
    "CLEANSE",
    "COLLISION_COURSE",
    "COSMIC_INDIFFERENCE",
    "CREATIVE_AI",
    "DIRGE",
    "DRUM_OF_BATTLE",
    "ENTROPY",
    "EXPERTISE",
    "FAN_OF_KNIVES",
    "FERAL",
    "FETCH",
    "FIEND_FIRE",
    "FIGHT_THROUGH",
    "FLAK_CANNON",
    "GLIMMER",
    "GLIMPSE_BEYOND",
    "GUIDING_STAR",
    "GUNK_UP",
    "HAVOC",
    "HELLO_WORLD",
    "HELLRAISER",
    "HIDDEN_DAGGERS",
    "HIDDEN_GEM",
    "HOWL_FROM_BEYOND",
    "HUDDLE_UP",
    "INFERNAL_BLADE",
    "INFINITE_BLADES",
    "I_AM_INVINCIBLE",
    "JACK_OF_ALL_TRADES",
    "JUGGLING",
    "LARGESSE",
    "MAKE_IT_SO",
    "MASTER_PLANNER",
    "MAYHEM",
    "MODDED",
    "NIGHTMARE",
    "OVERCLOCK",
    "PARTICLE_WALL",
    "PHOTON_CUT",
    "PLOT",
    "PRIMAL_FORCE",
    "PURITY",
    "REBOOT",
    "RIGHT_HAND_HAND",
    "ROCKET_PUNCH",
    "SCRAPE",
    "SECRET_TECHNIQUE",
    "SECRET_WEAPON",
    "SEEKER_STRIKE",
    "SENTRY_MODE",
    "SHADOW_STEP",
    "SNAP",
    "SOULBOUND",
    "SPLASH",
    "STAMPEDE",
    "STOKE",
    "STORM_OF_STEEL",
    "THE_BALL",
    "THRUMMING_HATCHET",
    "TURBO",
    "UNDEATH",
    "UPROAR",
    "UP_MY_SLEEVE",
    "WELL_LAID_PLANS",
];

/// `PRIOR_FIGHT_SHUFFLE_ENCHANTMENTS`: Enchantments with the same effect (Imbued auto-plays its card).
const PRIOR_FIGHT_SHUFFLE_ENCHANTMENTS: &[&str] = &["IMBUED"];

/// `PRIOR_FIGHT_FORGE_CARDS`: The complete v0.109 `ForgeCmd::Forge` source census, refused sources
/// included: a Forge can create a Sovereign Blade and change reshuffle sizes.
const PRIOR_FIGHT_FORGE_CARDS: &[&str] = &[
    "BEAT_INTO_SHAPE",
    "BIG_BANG",
    "BULWARK",
    "CONQUEROR",
    "FURNACE",
    "HAMMER_TIME",
    "REFINE_BLADE",
    "SEEKING_EDGE",
    "SPOILS_OF_BATTLE",
    "SUMMON_FORTH",
    "THE_SMITH",
    "WROUGHT_IN_WAR",
];

/// `PRIOR_FIGHT_FORGE_POTIONS`: Forge sources among potions.
const PRIOR_FIGHT_FORGE_POTIONS: &[&str] = &["POTION.KINGS_COURAGE"];

/// `PRIOR_FIGHT_FORGE_RELICS`: Forge sources among relics.
const PRIOR_FIGHT_FORGE_RELICS: &[&str] = &["RELIC.FENCING_MANUAL"];

/// `CARDSEL_CONSUMER_CARDS`: Cards whose play (or granted power) reads `CombatCardSelection` directly.
/// The auto-players are absent: every `AutoPlayFromDrawPile` caller passes
/// `CardPilePosition` 2 = Top (IL-verified 2026-07-11), so auto-play itself
/// consumes nothing on this stream.
const CARDSEL_CONSUMER_CARDS: &[&str] = &[
    "AGGRESSION",
    "ANOINTED",
    "CINDER",
    "DRAIN_POWER",
    "ENTROPY",
    "HIDDEN_GEM",
    "MAD_SCIENCE",
    "SEEKER_STRIKE",
    "THRASH",
];

/// `CARDSEL_GENERATION_CARDS`: Card-generation sources (the `CombatCardGeneration` census): a generated
/// card can itself be a consumer of any stream, and its play is unrecorded.
const CARDSEL_GENERATION_CARDS: &[&str] = &[
    "ABUNDANCE",
    "BUNDLE_OF_JOY",
    "CALAMITY",
    "CALL_OF_THE_VOID",
    "CREATIVE_AI",
    "DISCOVERY",
    "DISTRACTION",
    "HELLO_WORLD",
    "INFERNAL_BLADE",
    "JACKPOT",
    "JACK_OF_ALL_TRADES",
    "LARGESSE",
    "MAD_SCIENCE",
    "MANIFEST_AUTHORITY",
    "METAMORPHOSIS",
    "QUASAR",
    "SPECTRUM_SHIFT",
    "SPLASH",
    "STOKE",
    "WHITE_NOISE",
];

/// `CARDSEL_GENERATION_RELICS`: Card-generation relics.
const CARDSEL_GENERATION_RELICS: &[&str] = &[
    "RELIC.BIG_HAT",
    "RELIC.CHOICES_PARADOX",
    "RELIC.CROSSBOW",
    "RELIC.ORANGE_DOUGH",
    "RELIC.TOOLBOX",
    "RELIC.VEXING_PUZZLEBOX",
];

/// `CARDSEL_GENERATION_POTIONS`: Card-generation potions.
const CARDSEL_GENERATION_POTIONS: &[&str] = &[
    "POTION.ATTACK_POTION",
    "POTION.COLORLESS_POTION",
    "POTION.COSMIC_CONCOCTION",
    "POTION.OROBIC_ACID",
    "POTION.POWER_POTION",
    "POTION.SKILL_POTION",
];

/// `CARDSEL_EVENT_RELICS`: Relics that consume `CombatCardSelection` per unrecorded in-fight event
/// (Bookmark per flush, Mummified Hand per Power play, Power Cell and Jeweled
/// Mask on turn 1).
const CARDSEL_EVENT_RELICS: &[&str] = &[
    "RELIC.BOOKMARK",
    "RELIC.JEWELED_MASK",
    "RELIC.MUMMIFIED_HAND",
    "RELIC.POWER_CELL",
];

/// `TARGETS_CONSUMER_CARDS`: Cards whose play consumes `CombatTargets`: random-target attacks (one
/// `NextItem` per hit in `AttackCommand::Execute`), their own `OnPlay` rolls,
/// rolling powers, the auto-players (`CardCmd::AutoPlay` rolls a target for a
/// null-targeted enemy card), and Lightning channelers/evokers.
const TARGETS_CONSUMER_CARDS: &[&str] = &[
    "BALL_LIGHTNING",
    "BEAT_DOWN",
    "BOMBARDMENT",
    "BOUNCING_FLASK",
    "CACOPHONY",
    "CALL_OF_THE_VOID",
    "CASCADE",
    "CATASTROPHE",
    "CHAOS",
    "COUNTDOWN",
    "CREATIVE_AI",
    "DUALCAST",
    "FLAK_CANNON",
    "HAND_TRICK",
    "HAUNT",
    "HAVOC",
    "HELLO_WORLD",
    "JUGGERNAUT",
    "LIGHTNING_ROD",
    "MASTER_PLANNER",
    "MAYHEM",
    "MULTI_CAST",
    "QUADCAST",
    "RAINBOW",
    "RICOCHET",
    "RIP_AND_TEAR",
    "SENTRY_MODE",
    "SERPENT_FORM",
    "SHATTER",
    "STAMPEDE",
    "STARDUST",
    "STORM",
    "STORM_OF_STEEL",
    "SWEEPING_GAZE",
    "SWORD_BOOMERANG",
    "TEMPEST",
    "TESLA_COIL",
    "THE_BALL",
    "THUNDER",
    "TRASH_TO_TREASURE",
    "UPROAR",
    "VOLLEY",
    "VOLTAIC",
    "ZAP",
];

/// `TARGETS_CONSUMER_RELICS`: Relics whose per-fight `CombatTargets` draws follow unrecorded events.
const TARGETS_CONSUMER_RELICS: &[&str] = &[
    "RELIC.CRACKED_CORE",
    "RELIC.EMOTION_CHIP",
    "RELIC.FORGOTTEN_SOUL",
    "RELIC.HISTORY_COURSE",
    "RELIC.INFUSED_CORE",
    "RELIC.KUSARIGAMA",
    "RELIC.PARRYING_SHIELD",
    "RELIC.TINGSHA",
    "RELIC.WHISPERING_EARRING",
];

/// `TARGETS_CONSUMER_POTIONS`: Distilled Chaos auto-plays three cards null-targeted.
const TARGETS_CONSUMER_POTIONS: &[&str] = &["POTION.DISTILLED_CHAOS"];

/// `ENERGY_COSTS_CONSUMER_RELICS`: `CombatEnergyCosts` consumers besides Slither: ConfusedPower's relics.
const ENERGY_COSTS_CONSUMER_RELICS: &[&str] = &["RELIC.FAKE_SNECKO_EYE", "RELIC.SNECKO_EYE"];

/// `ENERGY_COSTS_CONSUMER_POTIONS`: and Snecko Oil.
const ENERGY_COSTS_CONSUMER_POTIONS: &[&str] = &["POTION.SNECKO_OIL"];

/// `NICHE_MIDFIGHT_SPAWNERS`: monsters whose fights create creatures after
/// the encounter is set up, keyed by a substring of the recorded monster id.
/// The number is the fewest turns before a creation is possible.
///
/// # Which call draws `Niche`
///
/// `CombatState::CreateCreature` (`0x137074`) draws once per **enemy-side**
/// creature: `IL_003f`-`IL_0041` skips to `IL_007c` unless `side == 2`, and
/// `IL_0050`-`IL_0055` passes `RunRngSet::get_Niche` to
/// `Creature::SetUniqueMonsterHpValue` (`0x11d3ec`), which makes exactly one
/// draw (`NextItem` at `IL_0089`, or `NextInt` at `IL_0094` when every HP
/// value is taken). One more site draws inside a fight without creating
/// anything: `ToughEgg/<Hatch>d__36::MoveNext` (`0x3726d4`, `IL_0094`-`IL_00a7`)
/// rolls the hatchling's HP.
///
/// # What `monster_ids` records
///
/// `CombatRoom/<StartCombat>d__46::MoveNext` (`0x310bf0`, `IL_0130`) appends
/// every set-up monster's id. A creature added later goes through
/// `CreatureCmd/<Add>d__2::MoveNext` (`0x3e9234`), which appends its id only
/// when the list does not hold it yet (`IL_01a7`-`IL_01da`). So the roster is
/// the set-up monsters plus one row per *new* id, and a fight that created
/// creatures mid-combat can be short by any amount.
///
/// # The census (#2997)
///
/// `fixtures/niche_consumer_census_v1.json` lists every caller, in the
/// archived DLL, of `get_Niche`, of `CreateCreature`, of the two creating
/// `CreatureCmd::Add` overloads (`0x132154`, `0x1321a0`) and of
/// `ToughEgg::Hatch`. The mid-fight ones, all gated here:
///
/// * `LivingFog/<BloatMove>d__26` (`0x3620c0`, `IL_00ed`): `Add<GasBomb>`;
/// * `TwoTailedRat/<CallForBackup>d__41` (`0x373d98`, `IL_0183`):
///   `Add<TwoTailedRat>`;
/// * `InfestedPower/<AfterDeath>d__4` (`0x33d3ec`, `IL_008b`): the Wrigglers,
///   the power applied only by `PhrogParasite/<AfterAddedToRoom>d__10`
///   (`0x3667dc`, `IL_0098`);
/// * `StockPower/<AfterDeath>d__4` (`0x346280`, `IL_0090`), applied only by
///   `Axebot/<AfterAddedToRoom>d__34` (`0x352dcc`, `IL_003f`);
/// * `Ovicopter/<LayEggsMove>d__26` (`0x3651e8`, `IL_00e7`): `Add<ToughEgg>`
///   once per free slot, and each egg's `Hatch` (above) draws again;
/// * `Fogmog/<IllusionMove>d__15` (`0x35bef4`, `IL_00b5`): `Add<EyeWithTeeth>`;
/// * `TheObscura/<IllusionMove>d__25` (`0x370830`, `IL_00b5`):
///   `Add<Parafright>`;
/// * `Fabricator/<SpawnBot>d__25` (`0x35a80c`, `IL_0092`).
///
/// The first four rows are the frozen `solve_fight` registry, unchanged. The
/// rest were missing from it; their counts are not a function of anything the
/// `.run` records, so each takes the conservative threshold 0 and makes the
/// counter a baseline for every later fight.
///
/// One mid-fight creator is deliberately **not** a row, because the roster
/// already counts it exactly. `SurprisePower/<AfterDeath>d__4` (`0x347044`)
/// runs its body once, for its own owner's unprevented death
/// (`IL_002c`-`IL_0047`), and creates exactly one Fat Gremlin (`IL_0063`) and
/// one Sneaky Gremlin (`IL_0198`). Only
/// `GremlinMerc/<AfterAddedToRoom>d__21` (`0x35d898`, `IL_009f`) applies the
/// power, and `GremlinMercNormal::GenerateMonsters` (`0xd3e94`) sets up the
/// Merc alone, so both ids are new to the roster and each is recorded once:
/// two draws, two rows. The fifteen exact claims the captured saves hold
/// after a Gremlin Merc fight all match.
const NICHE_MIDFIGHT_SPAWNERS: &[(&str, i64, &str)] = &[
    (
        "LIVING_FOG",
        2,
        "BLOAT spawned GasBombs (one Niche draw each, 0-5 per fight)",
    ),
    (
        "TWO_TAILED_RAT",
        3,
        "rats may have summoned (one Niche draw each, 2 summons max)",
    ),
    (
        "PHROG_PARASITE",
        0,
        "4 Wrigglers spawned on the phrog's death (one Niche draw each)",
    ),
    (
        "AXEBOT",
        1,
        "Stock may have replaced Axebot one or two times (one unrecorded Niche draw per \
         replacement)",
    ),
    (
        "OVICOPTER",
        0,
        "Ovicopter may have laid Tough Eggs (one Niche draw per egg and one more per hatch)",
    ),
    (
        "FOGMOG",
        0,
        "Fogmog may have summoned Eyes With Teeth (one Niche draw each)",
    ),
    (
        "THE_OBSCURA",
        0,
        "The Obscura may have summoned Parafrights (one Niche draw each)",
    ),
    (
        "FABRICATOR",
        0,
        "Fabricator may have built bots (one Niche draw each)",
    ),
];

/// Events with the combat layout, whose encounter is created when the event
/// is *entered*, whether or not it is ever fought.
///
/// `EventRoom/<EnterInternal>d__18::MoveNext` (`0x3110f8`, `IL_0164`) calls
/// `EventSynchronizer::GenerateInternalCombatStateIfNecessary` (`0x6e70c`),
/// and `EventCombatSynchronizer::InitializeForEvent` (`0x6d0ec`) creates every
/// monster of `CanonicalEncounter` on the enemy side (`IL_010b`-`IL_010e`) when
/// `LayoutType == 1` (`IL_0019`-`IL_001f`). Exactly three events override
/// `get_LayoutType` to 1: `PunchOff` (`0xcc0bb`), `TheArchitect` (`0xcf2ef`)
/// and `TheLanternKey` (`0xd0231`). When the fight happens,
/// `EventCombatSynchronizer::EnterCombat` (`0x6d3d8`, `IL_021b`-`IL_0232`)
/// hands that combat state to the room and sets `ShouldCreateCombat` from the
/// layout, `CombatRoom/<StartCombat>d__46` skips the creation
/// (`IL_00ea`-`IL_00ef`) and still records the roster (`IL_0130`), so the
/// count is right: run `1787331639` fought The Lantern Key's knight and the
/// saved counter rose by its one monster. When the fight does not happen the
/// draws are in no roster: run `1786850545` left Punch Off unfought and the
/// saved counter rose by 2 across the node.
const NICHE_COMBAT_LAYOUT_EVENTS: &[&str] = &[
    "EVENT.PUNCH_OFF",
    "EVENT.THE_ARCHITECT",
    "EVENT.THE_LANTERN_KEY",
];

/// Run modifiers that draw `Niche`: `CursedRun/<AfterActEntered>d__0::MoveNext`
/// (`0x377420`, `IL_005b`-`IL_00a9`) draws one `NextItem` per player on every
/// act entry.
const NICHE_MODIFIERS: &[&str] = &["MODIFIER.CURSED_RUN"];

/// Python's `repr` of a list of plain identifiers: `['A', 'B']`.
fn list_repr(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|item| format!("'{item}'")).collect();
    format!("[{}]", quoted.join(", "))
}

/// `sorted(set(values) & registry)`.
fn owned_in(values: &[String], registry: &[&str]) -> Vec<String> {
    values
        .iter()
        .filter(|v| registry.contains(&v.as_str()))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// `sorted({bare card id for each deck row that satisfies keep})`.
fn deck_ids_where(fight: &Fight, keep: impl Fn(&str) -> bool) -> Vec<String> {
    fight
        .deck_entering
        .iter()
        .map(|row| row.bare_id())
        .filter(|id| keep(id))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// `solve_fight.shuffle_counter_caveats`: every represented prior-fight
/// `Shuffle` uncertainty.
pub fn shuffle_counter_caveats(fights: &[Fight], fight_index: usize) -> Vec<String> {
    let mut out = Vec::new();
    for (k, f) in fights.iter().enumerate().take(fight_index) {
        let enc = &f.encounter_id;
        let bad: BTreeSet<&str> = CARD_AFFLICTING_MONSTERS
            .iter()
            .copied()
            .filter(|c| f.monster_ids.iter().any(|m| m.contains(c)))
            .collect();
        if !bad.is_empty() {
            let bad: Vec<&str> = bad.into_iter().collect();
            out.push(format!(
                "fight {k} ({enc}): {} add cards mid-combat — the predicted Shuffle counter is \
                 a baseline; each afflicted card in a pre-reshuffle discard adds +1 (enumerate \
                 with --counter)",
                bad.join(", ")
            ));
        }
        let draw_relics = owned_in(&f.relics_entering, PRIOR_FIGHT_DRAW_RELICS);
        if !draw_relics.is_empty() {
            out.push(format!(
                "fight {k} ({enc}): modeled draw/cycle relics {} had unrecorded exact trigger/\
                 play timing — their effects can shift reshuffle boundaries; the predicted \
                 Shuffle counter is a hypothesis; enumerate with --counter",
                draw_relics.join(", ")
            ));
        }
        let draw_potions = owned_in(&f.potions_used, PRIOR_FIGHT_DRAW_POTIONS);
        if !draw_potions.is_empty() {
            out.push(format!(
                "fight {k} ({enc}): modeled pile-changing potions used: {}; the .run records \
                 use but not exact timing/selection, so reshuffle boundaries are unaccounted — \
                 the predicted Shuffle counter is a hypothesis; enumerate with --counter",
                draw_potions.join(", ")
            ));
        }
        let shuffle_cards = deck_ids_where(f, |id| PRIOR_FIGHT_SHUFFLE_CARDS.contains(&id));
        let shuffle_enchantments: BTreeSet<String> = f
            .deck_entering
            .iter()
            .map(|row| row.bare_enchantment())
            .filter(|e| PRIOR_FIGHT_SHUFFLE_ENCHANTMENTS.contains(&e.as_str()))
            .collect();
        if !shuffle_cards.is_empty() || !shuffle_enchantments.is_empty() {
            let mut sources = shuffle_cards;
            sources.extend(
                shuffle_enchantments
                    .iter()
                    .map(|e| format!("ENCHANTMENT.{e}")),
            );
            out.push(format!(
                "fight {k} ({enc}): modeled draw-cycle sources {} had unrecorded exact play \
                 timing; their draw, auto-play, insertion, or retention effects can shift the \
                 run-lifetime Shuffle stream — the predicted counter is a hypothesis; enumerate \
                 with --counter",
                sources.join(", ")
            ));
        }
        let mut forge = deck_ids_where(f, |id| PRIOR_FIGHT_FORGE_CARDS.contains(&id));
        forge.extend(owned_in(&f.potions_used, PRIOR_FIGHT_FORGE_POTIONS));
        forge.extend(owned_in(&f.relics_entering, PRIOR_FIGHT_FORGE_RELICS));
        if !forge.is_empty() {
            out.push(format!(
                "fight {k} ({enc}): Forge sources {} had unrecorded exact trigger/play timing — \
                 Forge may create a Sovereign Blade and change reshuffle size/timing; the \
                 predicted Shuffle counter is a hypothesis; enumerate with --counter",
                forge.join(", ")
            ));
        }
        if f.relics_entering.iter().any(|r| r == "RELIC.BIIIG_HUG") {
            // `BiiigHug.AfterShuffle` inserts one generated Soot at a random
            // Draw position; the run records neither the count nor the timing
            // of prior reshuffles.
            out.push(format!(
                "fight {k} ({enc}): BIIIG_HUG generated one Soot at a random draw position \
                 after each reshuffle (timing/count unrecorded) — the predicted Shuffle counter \
                 is a hypothesis; enumerate with --counter"
            ));
        }
    }
    out
}

/// `solve_fight.monsterai_caveats`: only a Decimillipede fight reads the
/// entering `MonsterAi` counter (its revive rolls), so only it is caveated.
pub fn monsterai_caveats(fights: &[Fight], fight_index: usize) -> Vec<String> {
    if !fights[fight_index].encounter_id.contains("DECIMILLIPEDE") {
        return Vec::new();
    }
    fights
        .iter()
        .enumerate()
        .take(fight_index)
        .filter(|(_, f)| {
            MONSTERAI_CONSUMERS
                .iter()
                .any(|c| f.monster_ids.iter().any(|m| m.contains(c)))
        })
        .map(|(k, f)| {
            format!(
                "fight {k} ({}) consumed MonsterAi draws (unrecorded count) — segment revive \
                 rolls need an exact --ai-counter",
                f.encounter_id
            )
        })
        .collect()
}

/// Every node before `before_node` that entered a combat-layout event
/// ([`NICHE_COMBAT_LAYOUT_EVENTS`]) and recorded no fight, as
/// `(node index, event id)`. `load_combats` has already refused a run whose
/// `map_point_history` or `rooms` has the wrong shape.
fn unfought_layout_events(run: &Value, fights: &[Fight], before_node: i64) -> Vec<(i64, String)> {
    let nodes = run
        .get("map_point_history")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_array)
        .flatten();
    let mut out = Vec::new();
    for (node, point) in (0_i64..).zip(nodes) {
        if node >= before_node {
            break;
        }
        if fights.iter().any(|f| f.node_index == node) {
            continue;
        }
        let rooms = point
            .get("rooms")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for room in rooms {
            let id = room.get("model_id").and_then(Value::as_str);
            if let Some(id) = id.filter(|id| NICHE_COMBAT_LAYOUT_EVENTS.contains(id)) {
                out.push((node, id.to_owned()));
            }
        }
    }
    out
}

/// The run's modifiers that are in [`NICHE_MODIFIERS`], sorted.
fn niche_modifiers(run: &Value) -> Vec<String> {
    run.get("modifiers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("id").and_then(Value::as_str))
        .filter(|id| NICHE_MODIFIERS.contains(id))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// `solve_fight.niche_counter_entering`: one `Niche` draw per creature a prior
/// combat created (`CombatState::CreateCreature` ->
/// `SetUniqueMonsterHpValue`), counted from the recorded roster.
///
/// Every other `Niche` consumer the census finds makes the count a baseline
/// (see [`NICHE_MIDFIGHT_SPAWNERS`]). Two of them are not in the Python
/// original: an unfought combat-layout event and the Cursed Run modifier.
/// Their caveats come after the per-fight ones, so the frozen oracle
/// documents keep their order.
pub fn niche_counter_entering(
    run: &Value,
    fights: &[Fight],
    fight_index: usize,
) -> (u64, Vec<String>) {
    let mut counter = 0_u64;
    let mut caveats = Vec::new();
    for (k, f) in fights.iter().enumerate().take(fight_index) {
        counter += f.monster_ids.len() as u64;
        for (mid, min_turns, what) in NICHE_MIDFIGHT_SPAWNERS {
            if f.monster_ids.iter().any(|m| m.contains(mid)) && f.turns_taken >= *min_turns {
                caveats.push(format!(
                    "fight {k} ({}): {what} — monster_ids is the end-of-combat roster and \
                     under-counts mid-fight creature creations; the predicted Niche counter is \
                     a baseline, not exact",
                    f.encounter_id
                ));
            }
        }
    }
    for (node, event) in unfought_layout_events(run, fights, fights[fight_index].node_index) {
        caveats.push(format!(
            "node {node} ({event}): a combat-layout event creates its encounter's monsters on \
             entry (one Niche draw each) and no fight was recorded there; the predicted Niche \
             counter is a baseline, not exact"
        ));
    }
    for modifier in niche_modifiers(run) {
        caveats.push(format!(
            "{modifier} draws Niche on every act entry; the predicted Niche counter is a \
             baseline, not exact"
        ));
    }
    let bad = owned_in(&fights[fight_index].relics_entering, NICHE_ONOBTAIN_RELICS);
    if !bad.is_empty() {
        // Python interpolated the `set`; its order follows the per-process
        // string hash, so the same members are rendered sorted here.
        let quoted: Vec<String> = bad.iter().map(|r| format!("'{r}'")).collect();
        caveats.push(format!(
            "Niche-consuming on-obtain relics owned: {{{}}}",
            quoted.join(", ")
        ));
    }
    if bad.iter().any(|r| r == "RELIC.BEAUTIFUL_BRACELET") {
        caveats.push(
            "Beautiful Bracelet shuffles its complete eligible acquisition-time deck pool \
             before taking up to 4 cards, consuming max(0, eligible_count - 1) Niche draws. The \
             .run record does not preserve that acquisition-time pool/order, so the predicted \
             Niche counter is a baseline, not exact"
                .to_owned(),
        );
    }
    (counter, caveats)
}

/// `combat_sim.MAX_UPGRADE`: every card class's `MaxUpgradeLevel`, from
/// `data/card_max_upgrade.json` (the solver's `card_templates.json`
/// `max_upgrade` table, pinned in the tests). It is the whole-DLL table, not
/// the modeled rows: `CALCULATED_GAMBLE` has one modeled row but maximum 1.
fn max_upgrade(bare_id: &str) -> Option<i64> {
    static TABLE: OnceLock<HashMap<String, i64>> = OnceLock::new();
    TABLE
        .get_or_init(|| {
            let raw: HashMap<String, i64> =
                serde_json::from_str(include_str!("../../data/card_max_upgrade.json"))
                    .expect("the checked-in max-upgrade table is a JSON object of integers");
            raw.into_iter()
                .map(|(id, level)| (id.trim_start_matches("CARD.").to_owned(), level))
                .collect()
        })
        .get(bare_id)
        .copied()
}

/// The card-generation sources a prior fight held, in Python's
/// `gen_cards + gen_relics + gen_potions` order.
fn generation_sources(f: &Fight) -> Vec<String> {
    let mut sources = deck_ids_where(f, |id| CARDSEL_GENERATION_CARDS.contains(&id));
    sources.extend(owned_in(&f.relics_entering, CARDSEL_GENERATION_RELICS));
    sources.extend(owned_in(&f.potions_used, CARDSEL_GENERATION_POTIONS));
    sources
}

/// `solve_fight.card_sel_counter_entering`.
///
/// Stone Cracker is the one exactly accountable consumer: its
/// `AfterRoomEntered` (see [`crate::entry::opening`]'s Stone Cracker doc,
/// `StableShuffle` over the upgradable Draw cards) spends
/// `max(0, upgradable - 1)` draws per combat room, a pure function of the
/// recorded deck (`IsUpgradable` = level < `MaxUpgradeLevel`, `0x7cee5`).
pub fn card_sel_counter_entering(
    fights: &[Fight],
    fight_index: usize,
) -> (Option<u64>, Vec<String>) {
    let mut counter = 0_u64;
    let mut caveats = Vec::new();
    let mut exact = true;
    for (k, f) in fights.iter().enumerate().take(fight_index) {
        if f.relics_entering.iter().any(|r| r == "RELIC.STONE_CRACKER") {
            let mut upgradable = 0_u64;
            let mut resolved = true;
            for row in &f.deck_entering {
                match max_upgrade(&row.bare_id()) {
                    Some(maximum) if !row.upgrade_ambiguous => {
                        upgradable += u64::from(row.upgrade_level < maximum);
                    }
                    _ => {
                        caveats.push(format!(
                            "fight {k}: STONE_CRACKER owned but {} upgradability is \
                             unresolvable — its draw count is unknown",
                            row.id
                        ));
                        exact = false;
                        resolved = false;
                        break;
                    }
                }
            }
            if resolved {
                counter += upgradable.saturating_sub(1);
            }
        }
        let bad_cards = f
            .deck_entering
            .iter()
            .filter(|row| {
                CARDSEL_CONSUMER_CARDS.contains(&row.bare_id().as_str())
                    || (row.id == "CARD.TRUE_GRIT" && row.upgrade_level == 0)
            })
            .map(|row| row.bare_id())
            .collect::<BTreeSet<_>>();
        if !bad_cards.is_empty() {
            let bad: Vec<String> = bad_cards.into_iter().collect();
            caveats.push(format!(
                "fight {k}: deck held CombatCardSelection consumers ({}) — plays unrecorded",
                bad.join(", ")
            ));
            exact = false;
        }
        let generation = generation_sources(f);
        if !generation.is_empty() {
            caveats.push(format!(
                "fight {k}: card-generation sources present ({}) — a generated card could be a \
                 CombatCardSelection consumer, plays unrecorded",
                generation.join(", ")
            ));
            exact = false;
        }
        let bad_relics = owned_in(&f.relics_entering, CARDSEL_EVENT_RELICS);
        if !bad_relics.is_empty() {
            caveats.push(format!(
                "fight {k}: relics {} consume CombatCardSelection draws per unrecorded in-fight \
                 events",
                list_repr(&bad_relics)
            ));
            exact = false;
        }
    }
    (exact.then_some(counter), caveats)
}

/// `(bare, level) in CARDS and CARDS[...].targeted`: the modeled row's
/// `targeted` flag (current-build `TargetType` `AnyEnemy`), read from the
/// generated [`crate::content_tables::CARD_ROWS`], which mirror `CARDS`.
fn modeled_row_targeted(bare_id: &str, level: i64) -> bool {
    let (Some(id), Ok(level)) = (CardId::from_str(bare_id), u8::try_from(level)) else {
        return false;
    };
    card_row(id, level).is_some_and(|row| row.targeted)
}

/// `solve_fight.combat_targets_counter_entering`: exactly zero when no prior
/// fight held a consumer, otherwise unknown.
pub fn combat_targets_counter_entering(
    fights: &[Fight],
    fight_index: usize,
) -> (Option<u64>, Vec<String>) {
    let mut caveats = Vec::new();
    let mut exact = true;
    for (k, f) in fights.iter().enumerate().take(fight_index) {
        let bad_cards = deck_ids_where(f, |id| TARGETS_CONSUMER_CARDS.contains(&id));
        if !bad_cards.is_empty() {
            caveats.push(format!(
                "fight {k}: deck held CombatTargets consumers ({}) — plays unrecorded",
                bad_cards.join(", ")
            ));
            exact = false;
        }
        let targeted_imbued: Vec<String> = f
            .deck_entering
            .iter()
            .filter(|row| {
                row.bare_enchantment() == "IMBUED"
                    && modeled_row_targeted(&row.bare_id(), row.upgrade_level)
            })
            .map(|row| row.bare_id())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        if !targeted_imbued.is_empty() {
            caveats.push(format!(
                "fight {k}: targeted IMBUED cards ({}) auto-played with null targets — plays \
                 unrecorded",
                targeted_imbued.join(", ")
            ));
            exact = false;
        }
        let bad_relics = owned_in(&f.relics_entering, TARGETS_CONSUMER_RELICS);
        if !bad_relics.is_empty() {
            caveats.push(format!(
                "fight {k}: relics {} consume CombatTargets draws per unrecorded in-fight events",
                list_repr(&bad_relics)
            ));
            exact = false;
        }
        let generation = generation_sources(f);
        if !generation.is_empty() {
            caveats.push(format!(
                "fight {k}: card-generation sources present ({}) — a generated card could be a \
                 CombatTargets consumer, plays unrecorded",
                generation.join(", ")
            ));
            exact = false;
        }
        let bad_potions = owned_in(&f.potions_used, TARGETS_CONSUMER_POTIONS);
        if !bad_potions.is_empty() {
            caveats.push(format!(
                "fight {k}: {} auto-played null-targeted cards — enemy-targeted flips rolled \
                 CombatTargets, count unrecorded",
                bad_potions.join(", ")
            ));
            exact = false;
        }
    }
    (exact.then_some(0), caveats)
}

/// `solve_fight.energy_costs_counter_entering`: Slither, ConfusedPower and
/// Snecko Oil are the three current-build `CombatEnergyCosts` consumers.
pub fn energy_costs_counter_entering(
    fights: &[Fight],
    fight_index: usize,
) -> (Option<u64>, Vec<String>) {
    let mut caveats = Vec::new();
    for (k, f) in fights.iter().enumerate().take(fight_index) {
        let slither: Vec<String> = f
            .deck_entering
            .iter()
            .filter(|row| row.bare_enchantment() == "SLITHER")
            .map(|row| row.bare_id())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let relics = owned_in(&f.relics_entering, ENERGY_COSTS_CONSUMER_RELICS);
        let potions = owned_in(&f.potions_used, ENERGY_COSTS_CONSUMER_POTIONS);
        if !slither.is_empty() {
            caveats.push(format!(
                "fight {k}: Slither cards ({}) consumed CombatEnergyCosts on unrecorded draws",
                slither.join(", ")
            ));
        }
        if !relics.is_empty() {
            caveats.push(format!(
                "fight {k}: relics {} applied ConfusedPower and consumed CombatEnergyCosts on \
                 unrecorded draws",
                list_repr(&relics)
            ));
        }
        if !potions.is_empty() {
            caveats.push(format!(
                "fight {k}: used potions {} consumed CombatEnergyCosts for unrecorded hand cards",
                list_repr(&potions)
            ));
        }
    }
    ((caveats.is_empty()).then_some(0), caveats)
}

/// `solve_fight.combat_card_generation_counter_entering`: only fight 0's
/// source-free prefix proves the counter (zero).
pub fn combat_card_generation_counter_entering(fight_index: usize) -> (Option<u64>, Vec<String>) {
    if fight_index == 0 {
        return (Some(0), Vec::new());
    }
    (
        None,
        vec![
            "prior fights may have consumed CombatCardGeneration; the .run payload does not \
             expose the stream counter or all affliction consumers, so only fight 0 is \
             inferred exactly"
                .to_owned(),
        ],
    )
}

/// `solve_fight.combat_orb_generation_counter_entering`: Chaos and an active
/// Trash to Treasure power consume `CombatOrbGeneration`; a consumer-free
/// prefix proves zero.
pub fn combat_orb_generation_counter_entering(
    fights: &[Fight],
    fight_index: usize,
) -> (Option<u64>, Vec<String>) {
    let mut caveats = Vec::new();
    for (k, f) in fights.iter().enumerate().take(fight_index) {
        let direct = deck_ids_where(f, |id| matches!(id, "CHAOS" | "TRASH_TO_TREASURE"));
        let generation = generation_sources(f);
        if !direct.is_empty() {
            caveats.push(format!(
                "fight {k}: deck held CombatOrbGeneration consumers ({}) — plays/triggers \
                 unrecorded",
                direct.join(", ")
            ));
        }
        if !generation.is_empty() {
            caveats.push(format!(
                "fight {k}: card-generation sources present ({}) — a generated \
                 Chaos/TrashToTreasure could consume CombatOrbGeneration",
                generation.join(", ")
            ));
        }
    }
    ((caveats.is_empty()).then_some(0), caveats)
}

#[cfg(test)]
pub(super) mod registries {
    //! The transcribed registries, for the pin against their Python originals.
    pub const ALL: [(&str, &[&str]); 20] = [
        ("NICHE_ONOBTAIN_RELICS", super::NICHE_ONOBTAIN_RELICS),
        ("MONSTERAI_CONSUMERS", super::MONSTERAI_CONSUMERS),
        ("CARD_AFFLICTING_MONSTERS", super::CARD_AFFLICTING_MONSTERS),
        ("PRIOR_FIGHT_DRAW_RELICS", super::PRIOR_FIGHT_DRAW_RELICS),
        ("PRIOR_FIGHT_DRAW_POTIONS", super::PRIOR_FIGHT_DRAW_POTIONS),
        (
            "PRIOR_FIGHT_SHUFFLE_CARDS",
            super::PRIOR_FIGHT_SHUFFLE_CARDS,
        ),
        (
            "PRIOR_FIGHT_SHUFFLE_ENCHANTMENTS",
            super::PRIOR_FIGHT_SHUFFLE_ENCHANTMENTS,
        ),
        ("PRIOR_FIGHT_FORGE_CARDS", super::PRIOR_FIGHT_FORGE_CARDS),
        (
            "PRIOR_FIGHT_FORGE_POTIONS",
            super::PRIOR_FIGHT_FORGE_POTIONS,
        ),
        ("PRIOR_FIGHT_FORGE_RELICS", super::PRIOR_FIGHT_FORGE_RELICS),
        ("CARDSEL_CONSUMER_CARDS", super::CARDSEL_CONSUMER_CARDS),
        ("CARDSEL_GENERATION_CARDS", super::CARDSEL_GENERATION_CARDS),
        (
            "CARDSEL_GENERATION_RELICS",
            super::CARDSEL_GENERATION_RELICS,
        ),
        (
            "CARDSEL_GENERATION_POTIONS",
            super::CARDSEL_GENERATION_POTIONS,
        ),
        ("CARDSEL_EVENT_RELICS", super::CARDSEL_EVENT_RELICS),
        ("TARGETS_CONSUMER_CARDS", super::TARGETS_CONSUMER_CARDS),
        ("TARGETS_CONSUMER_RELICS", super::TARGETS_CONSUMER_RELICS),
        ("TARGETS_CONSUMER_POTIONS", super::TARGETS_CONSUMER_POTIONS),
        (
            "ENERGY_COSTS_CONSUMER_RELICS",
            super::ENERGY_COSTS_CONSUMER_RELICS,
        ),
        (
            "ENERGY_COSTS_CONSUMER_POTIONS",
            super::ENERGY_COSTS_CONSUMER_POTIONS,
        ),
    ];

    pub const NICHE_SPAWNERS: &[(&str, i64, &str)] = super::NICHE_MIDFIGHT_SPAWNERS;
    pub const NICHE_ONOBTAIN_RELICS: &[&str] = super::NICHE_ONOBTAIN_RELICS;
    pub const NICHE_COMBAT_LAYOUT_EVENTS: &[&str] = super::NICHE_COMBAT_LAYOUT_EVENTS;
    pub const NICHE_MODIFIERS: &[&str] = super::NICHE_MODIFIERS;

    pub fn max_upgrade(bare_id: &str) -> Option<i64> {
        super::max_upgrade(bare_id)
    }
}

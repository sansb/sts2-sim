//! The non-`Shuffle` counters, from the parser's per-fight projection.
//!
//! A port of `solve_fight`'s pre-fight accounting: `shuffle_counter_caveats`,
//! `monsterai_caveats`, `niche_counter_entering`, `card_sel_counter_entering`,
//! `combat_targets_counter_entering`, `energy_costs_counter_entering`,
//! `combat_card_generation_counter_entering` and
//! `combat_orb_generation_counter_entering`. Every registry is transcribed
//! element for element; `test_rust_run_counters.py` pins each one equal to its
//! Python original while that exists. The consumer censuses behind them are
//! the IL reads recorded in `STREAM_CONSUMERS.md`.
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

use super::history::Fight;
use crate::content_tables::card_row;
use crate::ids::CardId;

/// `NICHE_ONOBTAIN_RELICS`: Relics whose on-obtain effect consumes `Niche` (`STREAM_CONSUMERS.md`).
const NICHE_ONOBTAIN_RELICS: &[&str] = &[
    "RELIC.ASTROLABE",
    "RELIC.BEAUTIFUL_BRACELET",
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

/// `NICHE_MIDFIGHT_SPAWNERS`, in the dict's insertion order: monsters whose
/// moves or death triggers create creatures mid-fight (one
/// `SetUniqueMonsterHpValue` `Niche` draw each) that the end-of-combat
/// `monster_ids` roster can under-count. The number is the fewest turns
/// needed before a creation is possible.
const NICHE_MIDFIGHT_SPAWNERS: [(&str, i64, &str); 4] = [
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
];

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

/// `solve_fight.niche_counter_entering`: one `Niche` draw per creature a prior
/// combat created (`CombatState::CreateCreature` ->
/// `SetUniqueMonsterHpValue`), counted from the end-of-combat roster.
pub fn niche_counter_entering(fights: &[Fight], fight_index: usize) -> (u64, Vec<String>) {
    let mut counter = 0_u64;
    let mut caveats = Vec::new();
    for (k, f) in fights.iter().enumerate().take(fight_index) {
        counter += f.monster_ids.len() as u64;
        for (mid, min_turns, what) in NICHE_MIDFIGHT_SPAWNERS {
            if f.monster_ids.iter().any(|m| m.contains(mid)) && f.turns_taken >= min_turns {
                caveats.push(format!(
                    "fight {k} ({}): {what} — monster_ids is the end-of-combat roster and \
                     under-counts mid-fight creature creations; the predicted Niche counter is \
                     a baseline, not exact",
                    f.encounter_id
                ));
            }
        }
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

    pub const NICHE_SPAWNERS: [(&str, i64, &str); 4] = super::NICHE_MIDFIGHT_SPAWNERS;

    pub fn max_upgrade(bare_id: &str) -> Option<i64> {
        super::max_upgrade(bare_id)
    }
}

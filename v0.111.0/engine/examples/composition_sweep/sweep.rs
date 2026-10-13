//! The composition sweep's roots and its one-root pipeline (#3877).
//!
//! Not an example target itself: `main.rs` beside it is the driver, and
//! `tests/composition_sweep.rs` includes this file for the self-test.
//!
//! A root is a synthetic schema-20 save: the checked-in
//! `fixtures/entry_builder_v1.json` Toadpoles save, re-pointed at one
//! character with four Strikes, four Defends, the starter relic, every unlock
//! epoch and ascension 10, plus the one or two things under test. It goes
//! through the crate's public path only (`entry::build_root`, the canonical
//! boundary, `engine::admit`) and then through seeded random legal rollouts.
//! Every way that path can stop is an outcome with a stage:
//!
//! | stage | meaning |
//! |---|---|
//! | `ok` | nothing below was met (never "certified": the lines are shallow) |
//! | `entry` | `entry::build` refused the save |
//! | `opening` | `entry::opening::build` refused |
//! | `boundary` | the opened document did not load |
//! | `admission` | `engine::admit` refused; the detail is the missing set |
//! | `line` | an admitted root refused during legal play |
//! | `stuck` | a live state enumerated no legal action |
//! | `roundtrip` | a state projected, reloaded and projected differently, or would not |
//! | `panic` | the engine panicked (written by the driver's panic hook) |

use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde_json::{Value, json};
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::catalog::{Catalog, GameBuild};
use sts_sim::content_tables::{CARD_ROWS, UNLOCK_EPOCH_UNIVERSE_V1101};
use sts_sim::engine::{self, Action, EngineRefusal, LegalActionBuffer};
use sts_sim::entry::{self, EntryInput, EntryRequest, RootOutcome};
use sts_sim::hot::{HotState, PileId};
use sts_sim::ids::{CardId, EnchantmentId, EncounterId, PotionId, RelicId};

/// The five characters, with the starter facts a root needs.
pub const CHARACTERS: [Character; 5] = [
    Character::new("IRONCLAD", "RELIC.BURNING_BLOOD", 80, 0),
    Character::new("SILENT", "RELIC.RING_OF_THE_SNAKE", 70, 0),
    Character::new("DEFECT", "RELIC.CRACKED_CORE", 75, 3),
    Character::new("NECROBINDER", "RELIC.BOUND_PHYLACTERY", 66, 0),
    Character::new("REGENT", "RELIC.DIVINE_RIGHT", 75, 0),
];

#[derive(Clone, Copy, Debug)]
pub struct Character {
    pub name: &'static str,
    pub starter_relic: &'static str,
    pub max_hp: i64,
    pub orb_slots: i64,
}

impl Character {
    const fn new(
        name: &'static str,
        starter_relic: &'static str,
        max_hp: i64,
        orb_slots: i64,
    ) -> Self {
        Self {
            name,
            starter_relic,
            max_hp,
            orb_slots,
        }
    }

    pub fn named(name: &str) -> Option<Self> {
        CHARACTERS
            .into_iter()
            .find(|character| character.name.eq_ignore_ascii_case(name))
    }
}

/// One deck card under test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeckCard {
    pub id: String,
    pub upgrade: u8,
    pub enchantment: Option<String>,
}

impl DeckCard {
    pub fn plain(id: &str, upgrade: u8) -> Self {
        Self {
            id: id.to_string(),
            upgrade,
            enchantment: None,
        }
    }
}

/// One synthetic root: the starter kit plus what is under test.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    /// The dimension, e.g. `card`, `pair_cr`, `ench`.
    pub dim: &'static str,
    /// The human name of what is under test, unique within `(character, dim)`.
    pub item: String,
    pub cards: Vec<DeckCard>,
    /// Relics after the starter relic, in acquisition order.
    pub relics: Vec<String>,
    pub potions: Vec<String>,
    /// `None` is the fixture's Toadpoles.
    pub encounter: Option<String>,
}

impl Root {
    fn new(dim: &'static str, item: String) -> Self {
        Self {
            dim,
            item,
            cards: Vec::new(),
            relics: Vec::new(),
            potions: Vec::new(),
            encounter: None,
        }
    }
}

/// Which family of roots to enumerate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The control, every card at every level, two copies of every card,
    /// every relic, potion and encounter.
    Single,
    /// Every pair of card, relic, potion and encounter (cards at level 0).
    Pairs,
    /// Every enchantment on every card (level 0, amount 1).
    Enchant,
    /// The relic Draw chains of #3822 and #3903: Game Piece and Iron Club
    /// over a Hellraiser or Stratagem chain the root holds, or only reaches
    /// through a generator, beside each owner of another choice.
    Chains,
}

impl Mode {
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "single" => Some(Self::Single),
            "pairs" => Some(Self::Pairs),
            "enchant" => Some(Self::Enchant),
            "chains" => Some(Self::Chains),
            _ => None,
        }
    }
}

/// Saved properties for the relics whose opening needs one: a counter relic
/// with no saved counter refuses at the opening (`relic_counter_unseeded`,
/// `relic_counter_not_exact`, `tea_set_charge_undated`), which is a fact about
/// the input and not about the engine. Counters enter at zero, charges
/// charged, one-shot flags unspent.
const RELIC_PROPS: [(&str, &str, &str, i64); 18] = [
    ("RELIC.BONE_TEA", "ints", "CombatsLeft", 1),
    ("RELIC.EMBER_TEA", "ints", "CombatsLeft", 1),
    ("RELIC.TEA_OF_DISCOURTESY", "ints", "CombatsLeft", 1),
    ("RELIC.PUMPKIN_CANDLE", "ints", "KindleCount", 1),
    (
        "RELIC.VENERABLE_TEA_SET",
        "bools",
        "GainEnergyInNextCombat",
        1,
    ),
    (
        "RELIC.FAKE_VENERABLE_TEA_SET",
        "bools",
        "GainEnergyInNextCombat",
        1,
    ),
    ("RELIC.LIZARD_TAIL", "bools", "WasUsed", 0),
    ("RELIC.MAW_BANK", "bools", "HasItemBeenBought", 0),
    ("RELIC.GIRYA", "ints", "TimesLifted", 0),
    ("RELIC.IRON_CLUB", "ints", "CardsPlayed", 0),
    ("RELIC.TUNING_FORK", "ints", "SkillsPlayed", 0),
    ("RELIC.HAPPY_FLOWER", "ints", "TurnsSeen", 0),
    ("RELIC.FAKE_HAPPY_FLOWER", "ints", "TurnsSeen", 0),
    ("RELIC.PENDULUM", "ints", "TurnsSeen", 0),
    ("RELIC.POLLINOUS_CORE", "ints", "TurnsSeen", 0),
    ("RELIC.NUNCHAKU", "ints", "AttacksPlayed", 0),
    ("RELIC.PEN_NIB", "ints", "AttacksPlayed", 0),
    ("RELIC.GALACTIC_DUST", "ints", "StarsSpent", 0),
];

fn relic_row(relic: &str) -> Value {
    let mut row = json!({"floor_added_to_deck": 1, "id": relic});
    if let Some((_, group, name, value)) = RELIC_PROPS.iter().find(|(id, ..)| *id == relic) {
        let value = if *group == "bools" {
            json!(*value != 0)
        } else {
            json!(value)
        };
        row["props"] = json!({*group: [{"name": name, "value": value}]});
    }
    row
}

fn relics_for(character: Character) -> impl Iterator<Item = &'static str> {
    RelicId::NAMES
        .into_iter()
        .filter(move |relic| *relic != character.starter_relic)
}

fn bare(relic: &str) -> &str {
    relic.strip_prefix("RELIC.").unwrap_or(relic)
}

/// The relic inventories of [`Mode::Chains`], in acquisition order. The last
/// is the control: the same chains with neither relic.
const CHAIN_OWNERS: [(&str, &[&str]); 5] = [
    ("piece", &["GAME_PIECE"]),
    ("club", &["IRON_CLUB"]),
    ("piece+club", &["GAME_PIECE", "IRON_CLUB"]),
    ("club+piece", &["IRON_CLUB", "GAME_PIECE"]),
    ("neither", &[]),
];

/// Where the chain comes from: the name, the deck cards and the belt. The
/// `generated` rows hold no card that installs the listening power (or no
/// selecting Strike); a potion's offer is the only way to it, which is the
/// closure an admission wall keyed on held cards cannot see (#3903).
const CHAIN_SOURCES: [(&str, &[&str], &[&str]); 6] = [
    (
        "hellraiser held",
        &[
            "HELLRAISER",
            "HELLRAISER",
            "INFLAME",
            "INFLAME",
            "SEEKER_STRIKE",
            "SEEKER_STRIKE",
            "SCULPTING_STRIKE",
            "SCULPTING_STRIKE",
        ],
        &[],
    ),
    (
        "stratagem held",
        &[
            "STRATAGEM",
            "STRATAGEM",
            "INFLAME",
            "INFLAME",
            "SHRUG_IT_OFF",
        ],
        &[],
    ),
    (
        "both held",
        &[
            "HELLRAISER",
            "STRATAGEM",
            "INFLAME",
            "INFLAME",
            "SEEKER_STRIKE",
            "SCULPTING_STRIKE",
        ],
        &[],
    ),
    (
        "power generated",
        &[
            "INFLAME",
            "INFLAME",
            "SEEKER_STRIKE",
            "SEEKER_STRIKE",
            "SCULPTING_STRIKE",
            "SCULPTING_STRIKE",
        ],
        &["POWER_POTION", "POWER_POTION"],
    ),
    (
        "strikes generated",
        &["HELLRAISER", "HELLRAISER", "INFLAME", "INFLAME"],
        &["COLORLESS_POTION", "COLORLESS_POTION"],
    ),
    (
        "all generated",
        &["INFLAME", "INFLAME", "INFLAME"],
        &["POWER_POTION", "COLORLESS_POTION"],
    ),
];

/// The other choice owners a chain is swept beside: the name, the relics
/// after the chain's own, and extra deck cards.
const CHAIN_COMPANIONS: [(&str, &[&str], &[&str]); 10] = [
    ("alone", &[], &[]),
    ("gremlin horn", &["GREMLIN_HORN"], &[]),
    ("centennial puzzle", &["CENTENNIAL_PUZZLE"], &[]),
    ("joss paper", &["JOSS_PAPER"], &["TRUE_GRIT", "TRUE_GRIT"]),
    ("whispering earring", &["WHISPERING_EARRING"], &[]),
    ("unceasing top", &["UNCEASING_TOP"], &[]),
    ("dark embrace", &[], &["DARK_EMBRACE", "DARK_EMBRACE"]),
    (
        "horn and puzzle",
        &["GREMLIN_HORN", "CENTENNIAL_PUZZLE"],
        &[],
    ),
    ("horn and top", &["GREMLIN_HORN", "UNCEASING_TOP"], &[]),
    // Joss Paper's own side-end wall reads a live Hellraiser (#3914).
    (
        "joss paper and art of war",
        &["JOSS_PAPER", "ART_OF_WAR"],
        &["TRUE_GRIT", "TRUE_GRIT"],
    ),
];

/// Visit every root of `mode` for `character`, in a fixed order. `visit`
/// returns `false` to stop.
///
/// The order is part of the tool's contract only within one invocation (the
/// driver restarts a crashed worker by ordinal); outcomes are keyed by
/// `(character, dim, item)`, and each root's rollouts are seeded from that
/// key, so a slice and the full sweep agree on every root they share.
pub fn for_each_root(mode: Mode, character: Character, visit: &mut dyn FnMut(Root) -> bool) {
    let cards = CardId::NAMES;
    let potions = PotionId::NAMES;
    let encounters = EncounterId::NAMES;
    macro_rules! emit {
        ($root:expr) => {
            if !visit($root) {
                return;
            }
        };
    }
    emit!(Root::new("control", "base".to_string()));
    if mode == Mode::Chains {
        if character.name != "IRONCLAD" {
            return;
        }
        for (owner, owned) in CHAIN_OWNERS {
            for (chain, cards, potions) in CHAIN_SOURCES {
                for (beside, relics, extra) in CHAIN_COMPANIONS {
                    let mut root = Root::new("chain", format!("{owner} / {chain} / {beside}"));
                    root.cards = cards
                        .iter()
                        .chain(extra.iter())
                        .map(|id| DeckCard::plain(id, 0))
                        .collect();
                    root.relics = owned
                        .iter()
                        .chain(relics.iter())
                        .map(|relic| format!("RELIC.{relic}"))
                        .collect();
                    root.potions = potions.iter().map(|potion| potion.to_string()).collect();
                    emit!(root);
                }
            }
        }
        return;
    }
    match mode {
        Mode::Single => {
            for row in CARD_ROWS.iter() {
                let id = row.id.as_str();
                let mut root = Root::new("card", format!("{id}+{}", row.upgrade));
                root.cards.push(DeckCard::plain(id, row.upgrade));
                emit!(root);
            }
            for id in cards {
                let mut root = Root::new("card_x2", format!("{id}+0"));
                root.cards = vec![DeckCard::plain(id, 0), DeckCard::plain(id, 0)];
                emit!(root);
            }
            for relic in relics_for(character) {
                let mut root = Root::new("relic", relic.to_string());
                root.relics.push(relic.to_string());
                emit!(root);
            }
            for potion in potions {
                let mut root = Root::new("potion", potion.to_string());
                root.potions.push(potion.to_string());
                emit!(root);
            }
            for encounter in encounters {
                let mut root = Root::new("encounter", encounter.to_string());
                root.encounter = Some(encounter.to_string());
                emit!(root);
            }
        }
        Mode::Pairs => {
            let relics: Vec<&str> = relics_for(character).collect();
            for (index, first) in relics.iter().enumerate() {
                for second in &relics[index + 1..] {
                    let item = format!("{first} & {second}");
                    let mut root = Root::new("pair_rr", item.clone());
                    root.relics = vec![first.to_string(), second.to_string()];
                    emit!(root);
                    let mut root = Root::new("pair_rr_rev", item);
                    root.relics = vec![second.to_string(), first.to_string()];
                    emit!(root);
                }
            }
            for (index, first) in cards.iter().enumerate() {
                for second in &cards[index + 1..] {
                    let mut root = Root::new("pair_cc", format!("{first} & {second}"));
                    root.cards = vec![DeckCard::plain(first, 0), DeckCard::plain(second, 0)];
                    emit!(root);
                }
            }
            for card in cards {
                for relic in &relics {
                    let mut root = Root::new("pair_cr", format!("{card} & {relic}"));
                    root.cards.push(DeckCard::plain(card, 0));
                    root.relics.push(relic.to_string());
                    emit!(root);
                }
            }
            for potion in potions {
                for card in cards {
                    let mut root = Root::new("pair_pc", format!("{potion} & {card}"));
                    root.potions.push(potion.to_string());
                    root.cards.push(DeckCard::plain(card, 0));
                    emit!(root);
                }
            }
            for potion in potions {
                for relic in &relics {
                    let mut root = Root::new("pair_pr", format!("{potion} & {relic}"));
                    root.potions.push(potion.to_string());
                    root.relics.push(relic.to_string());
                    emit!(root);
                }
            }
            for first in potions {
                for second in potions {
                    let mut root = Root::new("pair_pp", format!("{first} & {second}"));
                    root.potions = vec![first.to_string(), second.to_string()];
                    emit!(root);
                }
            }
            for encounter in encounters {
                for card in cards {
                    let mut root = Root::new("pair_ec", format!("{encounter} & {card}"));
                    root.encounter = Some(encounter.to_string());
                    root.cards.push(DeckCard::plain(card, 0));
                    emit!(root);
                }
            }
            for encounter in encounters {
                for relic in &relics {
                    let mut root = Root::new("pair_er", format!("{encounter} & {relic}"));
                    root.encounter = Some(encounter.to_string());
                    root.relics.push(relic.to_string());
                    emit!(root);
                }
            }
            for encounter in encounters {
                for potion in potions {
                    let mut root = Root::new("pair_ep", format!("{encounter} & {potion}"));
                    root.encounter = Some(encounter.to_string());
                    root.potions.push(potion.to_string());
                    emit!(root);
                }
            }
        }
        Mode::Chains => unreachable!("enumerated above"),
        Mode::Enchant => {
            for enchantment in EnchantmentId::NAMES {
                for card in cards {
                    let mut root = Root::new("ench", format!("{enchantment}@{card}"));
                    root.cards.push(DeckCard {
                        id: card.to_string(),
                        upgrade: 0,
                        enchantment: Some(enchantment.to_string()),
                    });
                    emit!(root);
                }
            }
        }
    }
}

/// A root named on the command line: `ID+LEVEL` or `ENCHANTMENT@ID+LEVEL`
/// cards, bare or `RELIC.`-prefixed relics.
pub fn custom_root(
    cards: &[String],
    relics: &[String],
    potions: &[String],
    encounter: Option<&str>,
) -> Result<Root, String> {
    let mut root = Root::new("custom", String::new());
    for spec in cards {
        let (enchantment, rest) = match spec.split_once('@') {
            Some((enchantment, rest)) => (Some(enchantment.to_string()), rest),
            None => (None, spec.as_str()),
        };
        let (id, upgrade) = match rest.split_once('+') {
            Some((id, level)) => (
                id,
                level
                    .parse::<u8>()
                    .map_err(|_| format!("card level in {spec:?} is not a number"))?,
            ),
            None => (rest, 0),
        };
        if CardId::from_str(id).is_none() {
            return Err(format!("unknown card {id:?}"));
        }
        root.cards.push(DeckCard {
            id: id.to_string(),
            upgrade,
            enchantment,
        });
    }
    for relic in relics {
        let full = format!("RELIC.{}", bare(relic));
        if RelicId::from_str(&full).is_none() {
            return Err(format!("unknown relic {relic:?}"));
        }
        root.relics.push(full);
    }
    for potion in potions {
        if PotionId::from_str(potion).is_none() {
            return Err(format!("unknown potion {potion:?}"));
        }
        root.potions.push(potion.clone());
    }
    if let Some(encounter) = encounter {
        if EncounterId::from_str(encounter).is_none() {
            return Err(format!("unknown encounter {encounter:?}"));
        }
        root.encounter = Some(encounter.to_string());
    }
    let mut parts: Vec<String> = cards.to_vec();
    parts.extend(root.relics.iter().cloned());
    parts.extend(potions.iter().cloned());
    parts.extend(root.encounter.iter().cloned());
    root.item = parts.join(" & ");
    Ok(root)
}

fn template() -> &'static Value {
    static TEMPLATE: OnceLock<Value> = OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let fixture: Value =
            serde_json::from_str(include_str!("../../fixtures/entry_builder_v1.json"))
                .expect("the entry-builder fixture is JSON");
        fixture["cases"][0]["save"].clone()
    })
}

/// The node kind an encounter id is entered as. An event combat reaches the
/// opening as a monster room (`entry/node.rs`).
fn node_type(encounter: &str) -> &'static str {
    if encounter.ends_with("_BOSS") {
        "boss"
    } else if encounter.ends_with("_ELITE") {
        "elite"
    } else {
        "monster"
    }
}

/// The synthetic save for one root.
pub fn save_json(character: Character, root: &Root) -> String {
    let mut save = template().clone();
    let name = character.name;
    let mut deck: Vec<Value> = Vec::new();
    for basic in ["STRIKE", "DEFEND"] {
        for _ in 0..4 {
            deck.push(json!({"floor_added_to_deck": 1, "id": format!("CARD.{basic}_{name}")}));
        }
    }
    for card in &root.cards {
        let mut row = json!({"floor_added_to_deck": 1, "id": format!("CARD.{}", card.id)});
        if card.upgrade > 0 {
            row["current_upgrade_level"] = json!(card.upgrade);
        }
        if let Some(enchantment) = &card.enchantment {
            row["enchantment"] = json!({"id": format!("ENCHANTMENT.{enchantment}"), "amount": 1});
        }
        deck.push(row);
    }
    let relics: Vec<Value> = std::iter::once(character.starter_relic)
        .chain(root.relics.iter().map(String::as_str))
        .map(relic_row)
        .collect();
    let potions: Vec<Value> = root
        .potions
        .iter()
        .enumerate()
        .map(|(slot, potion)| json!({"id": format!("POTION.{potion}"), "slot_index": slot}))
        .collect();
    let epochs: Vec<String> = UNLOCK_EPOCH_UNIVERSE_V1101
        .iter()
        .map(|epoch| format!("EPOCH.{epoch}"))
        .collect();
    // The fixture save predates the orb stream. A fight that channels reads
    // it, and native seeds it from the run seed like every other run stream.
    let seed = save["rng"]["seed"]
        .as_str()
        .expect("the fixture save names its seed")
        .to_string();
    let orbs = sts_sim::rng::optional_combat_stream_at_zero(&seed, "combat_orbs");
    save["rng"]["rngs"]["combat_orbs"] = json!({
        "counter": orbs.counter,
        "s0": orbs.words[0], "s1": orbs.words[1], "s2": orbs.words[2], "s3": orbs.words[3],
    });
    let player = &mut save["players"][0];
    player["character_id"] = json!(format!("CHARACTER.{name}"));
    player["max_hp"] = json!(character.max_hp);
    player["current_hp"] = json!(character.max_hp);
    player["base_orb_slot_count"] = json!(character.orb_slots);
    player["deck"] = Value::Array(deck);
    player["relics"] = Value::Array(relics);
    player["potions"] = Value::Array(potions);
    player["unlock_state"]["unlocked_epochs"] = json!(epochs);
    serde_json::to_string(&save).expect("a save serializes")
}

/// One thing a root met. `stage` is one of the names in the module doc.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub stage: &'static str,
    pub detail: String,
    /// The action that met it, for `line`, `stuck` and `roundtrip`.
    pub action: Option<String>,
    pub rollout: Option<u32>,
    pub step: Option<u32>,
    /// The actions that led there, oldest first (the last few unless tracing).
    pub trail: Vec<String>,
    /// When tracing: the state the finding was met in, as its canonical
    /// document, or why it does not project.
    pub state: Option<Value>,
}

impl Finding {
    fn root(stage: &'static str, detail: String) -> Self {
        Self {
            stage,
            detail,
            action: None,
            rollout: None,
            step: None,
            trail: Vec::new(),
            state: None,
        }
    }

    pub fn to_json(&self) -> Value {
        let mut value = json!({"stage": self.stage, "detail": self.detail});
        if let Some(action) = &self.action {
            value["action"] = json!(action);
        }
        if let Some(rollout) = self.rollout {
            value["rollout"] = json!(rollout);
        }
        if let Some(step) = self.step {
            value["step"] = json!(step);
        }
        if !self.trail.is_empty() {
            value["trail"] = json!(self.trail);
        }
        if let Some(state) = &self.state {
            value["state"] = state.clone();
        }
        value
    }
}

/// How one root is played.
#[derive(Clone, Copy, Debug)]
pub struct Play {
    pub rollouts: u32,
    pub depth: u32,
    pub seed: u64,
    /// How many of a root's rollouts round-trip one sampled state.
    pub round_trips: u32,
    /// Also round-trip every state that has a choice pending.
    pub park_round_trips: bool,
    /// Keep the whole action trail instead of the last few.
    pub trace: bool,
}

impl Default for Play {
    fn default() -> Self {
        Self {
            rollouts: 3,
            depth: 400,
            seed: 0x5354_5332_5357_4545,
            round_trips: 1,
            park_round_trips: false,
            trace: false,
        }
    }
}

const TRAIL: usize = 8;

/// SplitMix64, as `eval_throughput` and the Coach examples use it.
struct PolicyRng(u64);

impl PolicyRng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut word = self.0;
        word = (word ^ (word >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        word = (word ^ (word >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        word ^ (word >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

/// FNV-1a over the root's key, so a root's lines do not depend on which
/// slice or shard it ran in.
fn root_seed(seed: u64, character: Character, root: &Root) -> u64 {
    let mut hash = 0xCBF2_9CE4_8422_2325u64 ^ seed;
    for part in [character.name, root.dim, &root.item] {
        for byte in part.bytes().chain(std::iter::once(0)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
    hash
}

fn describe(action: &Action, state: &HotState, catalog: &Catalog) -> String {
    match action {
        Action::Play {
            uid,
            target,
            selection,
        } => {
            let name = state
                .piles
                .get(PileId::Hand)
                .as_slice()
                .iter()
                .find(|card| card.uid == *uid)
                .and_then(|card| catalog.spec(card.atom))
                .map_or_else(
                    || format!("uid {uid}"),
                    |spec| {
                        let identity = spec.identity;
                        let enchantment = identity
                            .enchantment
                            .map_or_else(String::new, |enchantment| format!("{enchantment:?}@"));
                        format!("{enchantment}{}+{}", identity.id.as_str(), identity.upgrade)
                    },
                );
            let mut text = format!("Play {name}");
            if let Some(target) = target {
                text.push_str(&format!(" target {target}"));
            }
            if let Some(selected) = selection.get() {
                text.push_str(&format!(" selecting uid {selected}"));
            }
            text
        }
        Action::Select { answer } => format!("Select {answer:?}"),
        Action::UsePotion { slot, target } => match target {
            Some(target) => format!("UsePotion slot {slot} target {target}"),
            None => format!("UsePotion slot {slot}"),
        },
        Action::EndTurn => "EndTurn".to_string(),
    }
}

/// The first few JSON paths at which two documents differ.
fn differing_paths(left: &Value, right: &Value, path: &str, found: &mut Vec<String>) {
    if found.len() >= 6 || left == right {
        return;
    }
    match (left, right) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for key in keys {
                let child = format!("{path}/{key}");
                match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => differing_paths(x, y, &child, found),
                    _ => {
                        if found.len() < 6 {
                            found.push(child);
                        }
                    }
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (index, (x, y)) in a.iter().zip(b).enumerate() {
                differing_paths(x, y, &format!("{path}/{index}"), found);
            }
        }
        _ => found.push(path.to_string()),
    }
}

/// Project, reload from the projection alone, project again.
fn round_trip(state: &HotState, catalog: &Catalog) -> Result<(), String> {
    let document = HotBoundary::try_to_canonical(state, catalog)
        .map_err(|refusal| format!("project: {refusal}"))?;
    let reloaded_catalog = HotBoundary::catalog_from_canonical(&document)
        .map_err(|refusal| format!("reload: {refusal}"))?;
    let reloaded = HotBoundary::from_canonical(&document, &reloaded_catalog)
        .map_err(|refusal| format!("reload: {refusal}"))?;
    let again = HotBoundary::try_to_canonical(&reloaded, &reloaded_catalog)
        .map_err(|refusal| format!("reproject: {refusal}"))?;
    if again.differential_digest() == document.differential_digest() {
        return Ok(());
    }
    let mut paths = Vec::new();
    differing_paths(
        &serde_json::to_value(&document).expect("a canonical state serializes"),
        &serde_json::to_value(&again).expect("a canonical state serializes"),
        "",
        &mut paths,
    );
    Err(format!("mismatch at {}", paths.join(", ")))
}

fn admit(document: &CanonicalStateV2) -> Result<(HotState, Catalog), (&'static str, String)> {
    document
        .validate_schema()
        .map_err(|refusal| ("boundary", refusal.to_string()))?;
    let catalog = HotBoundary::catalog_from_canonical(document)
        .map_err(|refusal| ("boundary", refusal.to_string()))?;
    let state = HotBoundary::from_canonical(document, &catalog)
        .map_err(|refusal| ("boundary", refusal.to_string()))?;
    engine::admit(document, &state, &catalog).map_err(|refusal| {
        let mut missing: Vec<String> = refusal
            .missing()
            .map(|capability| capability.to_string())
            .collect();
        missing.sort();
        ("admission", missing.join(" ;; "))
    })?;
    Ok((state, catalog))
}

/// Build one root and play it. An empty answer is `ok`.
///
/// A root that stops before play returns that one finding. An admitted root
/// returns every distinct `(stage, detail)` its rollouts met, first met
/// first: one line can refuse where another gets stuck.
pub fn run_root(character: Character, root: &Root, play: &Play) -> Vec<Finding> {
    let save = save_json(character, root);
    let encounter = root
        .encounter
        .as_ref()
        .map(|encounter| format!("ENCOUNTER.{encounter}"));
    let request = EntryRequest {
        input: EntryInput::Save(&save),
        encounter_id: encounter.as_deref(),
        node_type: root.encounter.as_deref().map(node_type),
        game_build: GameBuild::ALL[GameBuild::COUNT - 1],
        mcr_splice: false,
    };
    let opening = match entry::build_root(&request, None) {
        RootOutcome::Refused(refusal) => {
            return vec![Finding::root(
                "entry",
                format!("{}: {refusal}", refusal.class()),
            )];
        }
        RootOutcome::EntryOnly(_, refusal) => {
            return vec![Finding::root(
                "opening",
                format!("{}: {refusal}", refusal.class()),
            )];
        }
        RootOutcome::Opened(opening) => opening,
    };
    let (entry_state, catalog) = match admit(&opening.document) {
        Ok(admitted) => admitted,
        Err((stage, detail)) => return vec![Finding::root(stage, detail)],
    };

    let mut findings: Vec<Finding> = Vec::new();
    let mut note = |finding: Finding| {
        if !findings
            .iter()
            .any(|known| known.stage == finding.stage && known.detail == finding.detail)
        {
            findings.push(finding);
        }
    };
    let mut legal = LegalActionBuffer::new();
    let mut events = Vec::new();
    let base = root_seed(play.seed, character, root);
    for rollout in 0..play.rollouts {
        let mut rng = PolicyRng(base ^ u64::from(rollout).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        // The step whose state is round-tripped: early, where every root has
        // a state, with a tail so later states are sampled too.
        let sampled = (rollout < play.round_trips).then(|| {
            let wide = rng.below(4) == 0;
            rng.below(if wide { 48 } else { 8 }) as u32
        });
        let mut state = entry_state.clone();
        let mut trail: Vec<String> = Vec::new();
        let finding =
            |stage, detail, action: Option<String>, step, trail: &[String], state: &HotState| {
                Finding {
                    stage,
                    detail,
                    action,
                    rollout: Some(rollout),
                    step: Some(step),
                    trail: if play.trace {
                        trail.to_vec()
                    } else {
                        trail[trail.len().saturating_sub(TRAIL)..].to_vec()
                    },
                    state: play.trace.then(|| {
                        match HotBoundary::try_to_canonical(state, &catalog) {
                            Ok(document) => serde_json::to_value(&document)
                                .expect("a canonical state serializes"),
                            Err(refusal) => json!({"does_not_project": refusal.to_string()}),
                        }
                    }),
                }
            };
        for step in 0..play.depth {
            if (sampled == Some(step) || play.park_round_trips && state.pending.is_some())
                && let Err(detail) = round_trip(&state, &catalog)
            {
                note(finding(
                    "roundtrip",
                    detail,
                    trail.last().cloned(),
                    step,
                    &trail,
                    &state,
                ));
            }
            if state.history.over {
                break;
            }
            let actions = match engine::legal_actions_checked(&state, &catalog, &mut legal) {
                Ok(actions) => actions,
                Err(EngineRefusal::NoLegalActions) => {
                    note(finding(
                        "stuck",
                        EngineRefusal::NoLegalActions.to_string(),
                        trail.last().cloned(),
                        step,
                        &trail,
                        &state,
                    ));
                    break;
                }
                Err(refusal) => {
                    note(finding(
                        "line",
                        refusal.to_string(),
                        Some("(enumerating legal actions)".to_string()),
                        step,
                        &trail,
                        &state,
                    ));
                    break;
                }
            };
            // Three picks in four avoid the last enumerated action, which is
            // EndTurn whenever the turn can end, so a line plays its hand.
            let pool = if actions.len() > 1 && rng.below(4) != 0 {
                actions.len() - 1
            } else {
                actions.len()
            };
            let action = actions[rng.below(pool)];
            let described = describe(&action, &state, &catalog);
            match engine::apply_action_into(&state, &catalog, &action, &mut events) {
                Ok(next) => {
                    state = next;
                    trail.push(described);
                }
                Err(refusal) => {
                    note(finding(
                        "line",
                        refusal.to_string(),
                        Some(described),
                        step,
                        &trail,
                        &state,
                    ));
                    break;
                }
            }
        }
    }
    findings
}

/// One root's JSON line: the first finding at the top level (the shape the
/// first sweep wrote), any further distinct findings under `more`.
pub fn outcome_json(
    ordinal: u64,
    character: Character,
    root: &Root,
    findings: &[Finding],
) -> Value {
    let mut line = json!({
        "i": ordinal,
        "ch": character.name,
        "dim": root.dim,
        "item": root.item,
        "stage": "ok",
        "detail": "",
    });
    if let Some((first, rest)) = findings.split_first() {
        let top = first.to_json();
        for (key, value) in top.as_object().expect("a finding is an object") {
            line[key] = value.clone();
        }
        if !rest.is_empty() {
            line["more"] = Value::Array(rest.iter().map(Finding::to_json).collect());
        }
    }
    line
}

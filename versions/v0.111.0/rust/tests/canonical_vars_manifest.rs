//! #3027: the generated card tables agree with the game's own canonical vars.
//!
//! `data/canonical_vars.v0.111.0.json` is written by
//! `tools/content_vars_census.py --write` from the game itself: every card is
//! instantiated through `ModelDb`, cloned, and upgraded level by level with the
//! game's own `UpgradeInternal`, so each level's vars are the
//! `get_CanonicalVars` constructor values with every `OnUpgrade` delta applied
//! (the manifest's `il` block names the RVA of each model's
//! `get_CanonicalVars`, `OnUpgrade` and `get_CanonicalKeywords`).
//!
//! Two checks run against it:
//!
//! * per card and upgrade level, the energy cost, star cost, X flags and the
//!   keyword columns (the Misery+ class, #3036);
//! * card step integer arguments **by role** (the Fetch class, #2519): a
//!   per-`StepKind` map names which var each argument position carries, and
//!   the argument must equal that var at the row's upgrade level. A value that
//!   merely equals *some* var of the card is not enough, which is what let
//!   Fetch's literal draw count 3 hide behind `OstyDamage` 3.
//!
//! Anything the role map cannot place is either an allowlisted literal with an
//! IL citation (`LITERALS`) or a whole kind (or kind mode) on the pinned
//! `NOT_YET_MAPPED` list, so coverage can only grow.

use rust_decimal::Decimal;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::str::FromStr;
use sts_sim::content_tables::{Arg, CARD_ROWS, CardRow, NATIVE_UNPLAYABLE_CARD_ROWS};
use sts_sim::ids::StepKind as K;

fn manifest() -> Value {
    serde_json::from_str(include_str!("../data/canonical_vars.v0.111.0.json")).unwrap()
}

fn level<'a>(manifest: &'a Value, row: &CardRow) -> &'a Value {
    let key = format!("CARD.{}", row.id.as_str());
    let card = manifest["cards"]
        .get(&key)
        .unwrap_or_else(|| panic!("{key} is not in the manifest"));
    card["levels"]
        .get(usize::from(row.upgrade))
        .unwrap_or_else(|| panic!("{key} has no level {}", row.upgrade))
}

fn decimal(var: &Value) -> Decimal {
    Decimal::from_str(var["value"].as_str().unwrap()).unwrap()
}

// ---------------------------------------------------------------------------
// The manifest itself
// ---------------------------------------------------------------------------

/// sha256 over the manifest's canonical payload with `self_sha256` removed:
/// sorted keys, no whitespace — `content_vars_census.py`'s `self_hash`.
fn payload_sha256(manifest: &Value) -> String {
    use sha2::{Digest, Sha256};
    let mut payload = manifest.clone();
    payload.as_object_mut().unwrap().remove("self_sha256");
    let digest = Sha256::digest(serde_json::to_string(&payload).unwrap().as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The committed manifest is the one the census tool wrote from the
/// certified assembly and has not been edited since: its schema, build and
/// DLL sha256, and its `self_sha256` over the payload. (The tool's `--check`
/// re-extracts from the DLL where one is present; CI has none, so this is the
/// hermetic half, #2515's precedent.)
#[test]
fn manifest_is_the_certified_builds_and_unedited() {
    let manifest = manifest();
    assert_eq!(manifest["schema"], "sts-sim-canonical-vars-manifest/v1");
    assert_eq!(manifest["build"], "v0.111.0");
    assert_eq!(
        manifest["provenance"]["dll_sha256"],
        "9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4"
    );
    assert_eq!(
        manifest["self_sha256"].as_str().unwrap(),
        payload_sha256(&manifest),
        "canonical_vars.v0.111.0.json was edited by hand; regenerate it with \
         tools/content_vars_census.py --write"
    );
    // Mutation control: moving one amount moves the hash.
    let mut edited = manifest.clone();
    edited["relics"]["RELIC.BONE_FLUTE"]["vars"]["Block"]["value"] = Value::from("4");
    assert_ne!(payload_sha256(&edited), payload_sha256(&manifest));
    for family in ["cards", "relics", "potions"] {
        assert!(
            !manifest[family].as_object().unwrap().is_empty(),
            "{family}"
        );
    }
}

// ---------------------------------------------------------------------------
// Per-level columns
// ---------------------------------------------------------------------------

/// Known table/native disagreements in the per-level columns. Each is a
/// finding, not a tolerance; the test fails if one stops disagreeing.
const COLUMN_DISAGREEMENTS: &[(&str, u8, &str, &str)] = &[
    // Cascade's -1 cost row was corrected at the generator (#3147).
];

#[test]
fn card_columns_match_manifest_per_upgrade_level() {
    let manifest = manifest();
    let mut problems = Vec::new();
    let mut used = BTreeSet::new();
    for row in &CARD_ROWS {
        let snap = level(&manifest, row);
        let keywords: BTreeSet<&str> = snap["keywords"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();
        let columns: [(&str, i64, i64); 4] = [
            ("cost", row.cost, snap["cost"].as_i64().unwrap()),
            (
                "star_cost",
                row.star_cost,
                snap["star_cost"].as_i64().unwrap(),
            ),
            (
                "x_cost",
                i64::from(row.x_cost),
                i64::from(snap["x_cost"].as_bool().unwrap()),
            ),
            (
                "star_x",
                i64::from(row.star_x),
                i64::from(snap["star_x"].as_bool().unwrap()),
            ),
        ];
        let keyword_columns = [
            ("Exhaust", row.exhausts),
            ("Ethereal", row.ethereal),
            ("Innate", row.innate),
            ("Retain", row.retain),
            ("Sly", row.sly),
            (
                "Unplayable",
                NATIVE_UNPLAYABLE_CARD_ROWS.contains(&(row.id, row.upgrade)),
            ),
        ];
        let differing = columns
            .iter()
            .filter(|(_, ours, native)| ours != native)
            .map(|(name, ours, native)| (*name, format!("{ours} vs native {native}")))
            .chain(
                keyword_columns
                    .iter()
                    .filter(|(kw, ours)| *ours != keywords.contains(kw))
                    .map(|(kw, ours)| (*kw, format!("{ours} vs native {}", !ours))),
            );
        for (column, detail) in differing {
            let known = COLUMN_DISAGREEMENTS.iter().position(|(c, l, col, _)| {
                *c == row.id.as_str() && *l == row.upgrade && *col == column
            });
            match known {
                Some(index) => {
                    used.insert(index);
                }
                None => problems.push(format!(
                    "{}@{} {column}: {detail}",
                    row.id.as_str(),
                    row.upgrade
                )),
            }
        }
        // An Unplayable keyword always makes the row unplayable. (The
        // converse does not hold: `playable` is also false for cost -1 status
        // cards with no keyword, which the solo-unplayable census owns.)
        if keywords.contains("Unplayable") && row.playable {
            problems.push(format!(
                "{}@{} is natively Unplayable but playable",
                row.id.as_str(),
                row.upgrade
            ));
        }
    }
    for (index, entry) in COLUMN_DISAGREEMENTS.iter().enumerate() {
        if !used.contains(&index) {
            problems.push(format!("stale COLUMN_DISAGREEMENTS entry {entry:?}"));
        }
    }
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Step arguments by role
// ---------------------------------------------------------------------------

/// One argument position and the var names that may carry it, in preference
/// order: the first one the card declares at that level is the role. Two
/// pseudo-names close the list:
///
/// * `"=1 hits"` — no hit-count var declared means one hit: `AttackCommand`'s
///   constructors initialise `_hitCount` to 1 (v0.111.0 `AttackCommand::.ctor`
///   RVA 0x1349a0 IL_000d `ldc.i4.1` / IL_000e `stfld _hitCount`), and only
///   `WithHitCount` changes it. A card that calls `WithHitCount` with a
///   literal is a `LITERALS` row.
/// * `"=upgrade"` — the argument is the row's upgrade level: the
///   upgraded-generated-card flag or the `X+1` bonus that the card's `OnPlay`
///   reads from `IsUpgraded` rather than from a var.
type Role = (usize, &'static [&'static str]);

/// `(kind, mode, roles)`. `mode` is `(index, word)` for kinds whose argument
/// layout depends on a mode word (or a power id) at `index`.
type Spec = (K, Option<(usize, &'static str)>, &'static [Role]);

const HITS: &[&str] = &["Repeat", "=1 hits"];

#[rustfmt::skip]
const ROLES: &[Spec] = &[
    // The high-volume kinds, placed by hand.
    (K::Attack, None, &[(0, &["Damage"]), (1, HITS)]),
    (K::AttackAll, None, &[(0, &["Damage"]), (1, HITS)]),
    (K::AttackRandom, None, &[(0, &["Damage"]), (1, HITS)]),
    (K::ShivAttack, None, &[(0, &["Damage"]), (1, HITS)]),
    (K::PhysicalDamageAttack, None, &[(0, &["Damage"]), (1, HITS)]),
    (K::TagTeamExact, None, &[(0, &["Damage"]), (1, HITS)]),
    (K::UproarExact, None, &[(0, &["Damage"]), (1, HITS)]),
    (K::ThrashExact, None, &[(0, &["Damage"]), (1, HITS)]),
    (K::Block, None, &[(0, &["Block"])]),
    // Mad Science names each rider's amount after the rider.
    (K::Draw, None, &[(0, &["Cards", "WisdomCards"])]),
    (K::DrawNextTurn, None, &[(0, &["Cards"])]),
    (K::Vulnerable, None, &[(0, &["VulnerablePower", "Power", "SappingVulnerable"])]),
    (K::Weak, None, &[(0, &["WeakPower", "Power", "SappingWeak"])]),
    (K::GenerateFixedShivs, None, &[(0, &["Cards", "Shivs"])]),
    (K::StarNextTurn, None, &[(0, &["StarNextTurnPower", "Stars"])]),
    (K::RetainHand, None, &[(0, &["Equilibrium"])]),
    (K::EvokeFrontExact, None, &[(0, &["Repeat"])]),
    (K::AttackResult, Some((1, "stars_on_kill")), &[(0, &["Damage"]), (2, &["Stars"])]),
    (K::AttackResult, Some((1, "energy_on_kill")), &[(0, &["Damage"]), (2, &["Energy"])]),
    (K::FatalAttackRewardExact, Some((1, "max_hp")), &[(0, &["Damage"]), (2, &["MaxHp"])]),
    (K::FatalAttackRewardExact, Some((1, "gold")), &[(0, &["Damage"]), (2, &["Gold"])]),
    (K::PowerAllSerial, Some((0, "doom")), &[(1, &["DoomPower"])]),
    (K::PowerAllSerial, Some((0, "weak")), &[(1, &["WeakPower"])]),
    (K::PowerAllSerial, Some((0, "poison")), &[(1, &["PoisonPower"])]),
    (K::PowerAllSerial, Some((0, "temp_strength_enemy")), &[(1, &["StrengthLoss"])]),
    (K::PowerAllSerial, Some((0, "strength_enemy")), &[(1, &[])]),
    (K::ForgeFamilyExact, Some((0, "conqueror")), &[(1, &["Forge"]), (2, &[])]),
    (K::ForgeFamilyExact, Some((0, "seeking_edge")), &[(1, &[]), (2, &["Forge"])]),
    (K::CompactExact, None, &[(0, &["Block"]), (1, &["=upgrade"])]),
    (K::DirgeXExact, None, &[(0, &["Summon"]), (1, &["=upgrade"])]),
    (K::GenerateShivsThenUpgradeExact, None, &[(0, &["Cards"]), (1, &["=upgrade"])]),
    (K::AutoplayDrawX, None, &[(0, &["=upgrade"])]),
    (K::EvokeFrontX, None, &[(0, &["=upgrade"])]),
    (K::MalaiseX, None, &[(0, &["=upgrade"])]),
    (K::PrimalForceExact, None, &[(0, &["=upgrade"])]),
    (K::StormOfSteelExact, None, &[(0, &["=upgrade"])]),
    // Kinds where one var carried every row's value, each reviewed by name
    // (ambiguous ones resolved from the card's `OnPlay` IL).
    (K::Accelerant, None, &[(0, &["Accelerant"])]),
    (K::Accuracy, None, &[(0, &["AccuracyPower"])]),
    (K::ActiveCardCostAddExact, None, &[(0, &["Repeat"])]),
    (K::AdaptiveStrikeExact, None, &[(0, &["Damage"])]),
    (K::AddOrbSlotsExact, None, &[(0, &["Repeat"])]),
    (K::Afterimage, None, &[(0, &["AfterimagePower"])]),
    (K::AllForOneExact, None, &[(0, &["Damage"])]),
    (K::AngerExact, None, &[(0, &["Damage"])]),
    (K::Arsenal, None, &[(0, &["ArsenalPower"])]),
    (K::AttackAllTempStrengthSnapshot, None, &[(0, &["Damage"]), (1, &["StrengthLoss"])]),
    (K::AttackAllX, None, &[(0, &["Damage"])]),
    (K::AttackContextResultExact, Some((0, "echoing_slash")), &[(1, &["Damage"])]),
    (K::AttackContextResultExact, Some((0, "omnislice")), &[(1, &["Damage"])]),
    (K::AttackPerStrike, None, &[(0, &["CalculationBase"]), (1, &["ExtraDamage"])]),
    (K::AttackPerVuln, None, &[(0, &["CalculationBase"]), (1, &["ExtraDamage"])]),
    (K::AttackRandomX, None, &[(0, &["Damage"])]),
    (K::AttackX, None, &[(0, &["Damage"])]),
    (K::Automation, None, &[(0, &["Energy"])]),
    (K::BeatDownExact, None, &[(0, &["Cards"])]),
    (K::BelieveInYouExact, None, &[(0, &["Energy"])]),
    (K::BiasedCognition, None, &[(0, &["BiasedCognitionPower"])]),
    (K::BlackHole, None, &[(0, &["BlackHolePower"])]),
    (K::BladeSymphonyExact, None, &[(0, &["Cards"])]),
    (K::BlazeExact, None, &[(0, &["StrengthPower"])]),
    (K::BlockNextBody, Some((0, "dodge_roll")), &[(1, &["Block"])]),
    (K::BlockNextBody, Some((0, "glitterstream")), &[(1, &["Block"]), (2, &["BlockNextTurn"])]),
    (K::Blur, None, &[(0, &["Blur"])]),
    (K::BorrowedTime, None, &[(0, &["ExtraCost"])]),
    (K::BrightestFlameExact, None, &[(0, &["Energy"]), (1, &["Cards"]), (2, &["MaxHp"])]),
    (K::Buffer, None, &[(0, &["BufferPower"])]),
    (K::BundleOfJoyExact, None, &[(0, &["Cards"])]),
    (K::Burst, None, &[(0, &["Skills"])]),
    (K::Cacophony, None, &[(0, &["Damage"])]),
    (K::Calcify, None, &[(0, &["CalcifyPower"])]),
    (K::CalculatedAttack, Some((0, "card_plays_finished_combat")), &[(1, &["CalculationBase"]), (2, &["ExtraDamage"])]),
    (K::CalculatedAttack, Some((0, "cards_drawn_combat")), &[(1, &["CalculationBase"]), (2, &["ExtraDamage"])]),
    (K::CalculatedAttack, Some((0, "discarded_cards_this_turn")), &[(1, &["CalculationBase"]), (2, &["ExtraDamage"])]),
    (K::CalculatedAttack, Some((0, "draw_count")), &[(1, &["CalculationBase"]), (2, &["ExtraDamage"])]),
    (K::CalculatedAttack, Some((0, "exhaust_count")), &[(1, &["CalculationBase"]), (2, &["ExtraDamage"])]),
    (K::CalculatedAttack, Some((0, "exhaust_exact_soul")), &[(1, &["CalculationBase"]), (2, &["ExtraDamage"])]),
    (K::CalculatedAttack, Some((0, "negative_hand_count")), &[(1, &["CalculationBase"]), (2, &["ExtraDamage"])]),
    (K::CalculatedAttack, Some((0, "non_hand_draws_this_turn")), &[(1, &["CalculationBase"]), (2, &["ExtraDamage"])]),
    (K::CalculatedAttack, Some((0, "owner_generated_cards_combat")), &[(1, &["CalculationBase"]), (2, &["ExtraDamage"])]),
    (K::CalculatedAttack, Some((0, "player_block")), &[(1, &["CalculationBase"]), (2, &["ExtraDamage"])]),
    (K::CalculatedAttack, Some((0, "target_doom")), &[(1, &["CalculationBase"]), (2, &["ExtraDamage"])]),
    (K::CalculatedBlock, Some((0, "discard_count")), &[(1, &["CalculationBase"]), (2, &["CalculationExtra"])]),
    (K::CalculatedBlock, Some((0, "strength_power_amount")), &[(1, &["CalculationBase"]), (2, &["CalculationExtra"])]),
    (K::CalculatedDoomExact, Some((0, "target_doom_tens")), &[(1, &["CalculationBase"]), (2, &["CalculationExtra"])]),
    (K::CalculatedHits, None, &[(0, &["Damage"])]),
    (K::CallOfTheVoid, None, &[(0, &["Cards"])]),
    (K::CatastropheExact, None, &[(0, &["Cards"])]),
    (K::ChildOfTheStars, None, &[(0, &["BlockForStars"])]),
    (K::ClawExact, None, &[(0, &["Damage"]), (1, &["Increase"])]),
    (K::Colossus, None, &[(0, &["Colossus"])]),
    (K::CompileDriverUniqueOrbExact, None, &[(0, &["Damage"])]),
    (K::ConcoctExact, None, &[(0, &["ConcoctPower"])]),
    (K::ConstellationExact, None, &[(0, &["Cards"]), (1, &["Energy"]), (2, &["Block"])]),
    (K::Coolant, None, &[(0, &["CoolantPower"])]),
    (K::CoordinateExact, None, &[(0, &["StrengthPower"])]),
    (K::CorrosiveWave, None, &[(0, &["CorrosiveWave"])]),
    (K::Corruption, None, &[(0, &["Power"])]),
    (K::Countdown, None, &[(0, &["CountdownPower"])]),
    (K::CreativeAi, None, &[(0, &["CreativeAi"])]),
    (K::CrescentSpearExact, None, &[(0, &["CalculationBase"]), (1, &["ExtraDamage"])]),
    (K::CrimsonMantle, None, &[(0, &["CrimsonMantlePower"])]),
    (K::Cruelty, None, &[(0, &["CrueltyPower"])]),
    (K::DanseMacabre, None, &[(0, &["DanseMacabrePower"])]),
    (K::DeathsDoorExact, None, &[(0, &["Block"]), (1, &["Repeat"])]),
    (K::Debilitate, None, &[(0, &["DebilitatePower"])]),
    (K::Demesne, None, &[(0, &["Cards"])]),
    (K::DemonForm, None, &[(0, &["StrengthPower"])]),
    (K::DevourLife, None, &[(0, &["DevourLifePower"])]),
    (K::Dexterity, None, &[(0, &["DexterityPower"])]),
    (K::Doom, None, &[(0, &["DoomPower"])]),
    (K::DrainPowerExact, None, &[(0, &["Damage"]), (1, &["Cards"])]),
    (K::DrawIfNoHandAttacks, None, &[(0, &["Cards"])]),
    (K::EchoForm, None, &[(0, &["EchoForm"])]),
    (K::EndOfDaysExact, None, &[(0, &["DoomPower"])]),
    (K::Energy, None, &[(0, &["Energy"])]),
    (K::EnergyNextTurn, None, &[(0, &["Energy"])]),
    (K::EnergySurgeExact, None, &[(0, &["Energy"])]),
    (K::Entropy, None, &[(0, &["Cards"])]),
    (K::Envenom, None, &[(0, &["EnvenomPower"])]),
    (K::EscapePlanExact, None, &[(0, &["Block"])]),
    (K::ExhaustDraw, None, &[(0, &["Cards"])]),
    (K::ExhaustNonattacksBlock, None, &[(0, &["Block"])]),
    (K::ExpertiseExact, None, &[(0, &["Cards"])]),
    (K::ExposeExact, None, &[(0, &["Power"])]),
    (K::FadeExact, None, &[(0, &["DexterityPower"])]),
    (K::Fasten, None, &[(0, &["ExtraBlock"])]),
    (K::FeelNoPain, None, &[(0, &["Power"])]),
    (K::Feral, None, &[(0, &["FeralPower"])]),
    (K::FiendFireExact, None, &[(0, &["Damage"])]),
    (K::FlakCannonExact, None, &[(0, &["Damage"])]),
    (K::FlameBarrier, None, &[(0, &["DamageBack"])]),
    (K::Focus, None, &[(0, &["FocusPower"])]),
    (K::Foregone, None, &[(0, &["Cards"])]),
    (K::ForgeFamilyExact, Some((0, "beat_into_shape")), &[(1, &["Damage"])]),
    (K::ForgeFamilyExact, Some((0, "big_bang")), &[(1, &["Forge"])]),
    (K::ForgeFamilyExact, Some((0, "bulwark")), &[(1, &["Block"]), (2, &["Forge"])]),
    (K::ForgeFamilyExact, Some((0, "refine_blade")), &[(1, &["Forge"]), (2, &["Energy"])]),
    (K::ForgeFamilyExact, Some((0, "spoils_of_battle")), &[(1, &["Forge"]), (2, &["Cards"])]),
    (K::ForgeFamilyExact, Some((0, "summon_forth")), &[(1, &["Forge"])]),
    (K::ForgeFamilyExact, Some((0, "the_smith")), &[(1, &["Forge"])]),
    (K::ForgeFamilyExact, Some((0, "wrought_in_war")), &[(1, &["Damage"]), (2, &["Forge"])]),
    (K::ForgottenRitualEnergyExact, None, &[(0, &["Energy"])]),
    (K::Furnace, None, &[(0, &["Forge"])]),
    (K::GangUpExact, None, &[(0, &["ExtraDamage"])]),
    (K::GenerateShivsThenInkyExact, None, &[(0, &["Cards"])]),
    (K::Genesis, None, &[(0, &["StarsPerTurn"])]),
    (K::GeneticAlgorithmGrowthExact, None, &[(0, &["Increase"])]),
    (K::GlimmerExact, None, &[(0, &["Cards"]), (1, &["PutBack"])]),
    (K::GlimpseBeyondExact, None, &[(0, &["Cards"])]),
    (K::GoForTheEyesExact, None, &[(0, &["Damage"]), (1, &["WeakPower"])]),
    (K::GuidingStarDrawNextTurnExact, None, &[(0, &["Cards"])]),
    (K::GunkUpExact, None, &[(0, &["Damage"]), (1, &["Repeat"])]),
    (K::Hailstorm, None, &[(0, &["HailstormPower"])]),
    (K::HandCapBody, Some((0, "aoe_debris")), &[(1, &["Damage"])]),
    (K::HandCapBody, Some((0, "attack_discard_optional")), &[(1, &["Damage"]), (2, &["Cards"])]),
    (K::HandCapBody, Some((0, "discard_fixed")), &[(1, &["Cards"])]),
    (K::HangExact, None, &[(0, &["Damage"])]),
    (K::HauntPower, None, &[(0, &["HpLoss"])]),
    (K::Heal, None, &[(0, &["Heal"])]),
    (K::HeavenlyDrillX, None, &[(0, &["Damage"]), (1, &["Energy"])]),
    (K::HelixDrillExact, None, &[(0, &["Damage"])]),
    (K::HiddenGemExact, None, &[(0, &["Replay"])]),
    (K::HpLoss, None, &[(0, &["HpLoss"])]),
    (K::HuddleUpExact, None, &[(0, &["Cards"])]),
    (K::ImitationLearningExact, None, &[(0, &["ImitationLearningPower"])]),
    (K::Inferno, None, &[(0, &["InfernoPower"])]),
    (K::Intangible, None, &[(0, &["IntangiblePower"])]),
    (K::Iteration, None, &[(0, &["IterationPower"])]),
    (K::JackpotExact, None, &[(0, &["Damage"]), (1, &["Cards"])]),
    (K::Juggernaut, None, &[(0, &["JuggernautPower"])]),
    (K::Knockdown, None, &[(0, &["KnockdownPower"])]),
    (K::LegionOfBoneExact, None, &[(0, &["Summon"])]),
    (K::Lethality, None, &[(0, &["LethalityPower"])]),
    (K::LiftExact, None, &[(0, &["Block"])]),
    (K::LightningRod, None, &[(0, &["LightningRodPower"])]),
    (K::Loop, None, &[(0, &["Loop"])]),
    (K::MachineLearning, None, &[(0, &["Cards"])]),
    (K::ManifestAuthorityExact, None, &[(0, &["Block"])]),
    (K::MaulExact, None, &[(0, &["Damage"]), (1, &["Increase"])]),
    (K::MetamorphosisExact, None, &[(0, &["Cards"])]),
    (K::MeteorShowerExact, None, &[(0, &["Damage"]), (1, &["WeakPower"])]),
    (K::MiseryExact, None, &[(0, &["Damage"])]),
    (K::MomentumStrikeExact, None, &[(0, &["Damage"])]),
    (K::MonarchsGaze, None, &[(0, &["StrengthLoss"])]),
    (K::Monologue, None, &[(0, &["Power"])]),
    (K::Neurosurge, None, &[(0, &["NeurosurgePower"])]),
    (K::NoxiousFumes, None, &[(0, &["PoisonPerTurn"])]),
    (K::Oblivion, None, &[(0, &["DoomPower"])]),
    (K::OneForAllExact, None, &[(0, &["OneForAllPower"])]),
    (K::OneTwoPunch, None, &[(0, &["Attacks"])]),
    (K::Orbit, None, &[(0, &["Energy"])]),
    (K::OstyBody, Some((0, "aoe_block_kill")), &[(1, &["OstyDamage"]), (2, &["Block"])]),
    (K::OstyBody, Some((0, "aoe_vulnerable")), &[(1, &["OstyDamage"]), (2, &["VulnerablePower"])]),
    (K::OstyBody, Some((0, "fetch")), &[(1, &["OstyDamage"]), (2, &["Cards"])]),
    (K::OstyBody, Some((0, "random")), &[(1, &["OstyDamage"])]),
    (K::OstyBody, Some((0, "single")), &[(1, &["OstyDamage"])]),
    (K::OstyBody, Some((0, "single_current_hp")), &[(1, &["CalculationBase"]), (2, &["ExtraDamage"])]),
    (K::OstyBody, Some((0, "single_max_hp")), &[(1, &["CalculationBase"]), (2, &["ExtraDamage"])]),
    (K::OstyBody, Some((0, "single_osty_cards")), &[(1, &["CalculationBase"]), (2, &["ExtraDamage"])]),
    (K::OstySummonHeal, None, &[(0, &["Summon"]), (1, &["Heal"])]),
    (K::OutbreakExact, None, &[(0, &["PoisonPower"])]),
    (K::OutrageExact, None, &[(0, &["Damage"])]),
    (K::PactsEndExact, None, &[(0, &["Damage"]), (1, &["Cards"])]),
    (K::Pagestorm, None, &[(0, &["Cards"])]),
    (K::PaleBlueDot, None, &[(0, &["Cards"])]),
    (K::Panache, None, &[(0, &["PanacheDamage"])]),
    (K::PanicButtonExact, None, &[(0, &["Block"]), (1, &["Turns"])]),
    (K::Parry, None, &[(0, &["ParryPower"])]),
    (K::PhantomBlades, None, &[(0, &["PhantomBladesPower"])]),
    (K::PillageExact, None, &[(0, &["Damage"])]),
    (K::PillarOfCreation, None, &[(0, &["Block"])]),
    (K::Plating, None, &[(0, &["PlatingPower"])]),
    (K::PlotExact, None, &[(0, &["Cards"])]),
    (K::Poison, None, &[(0, &["PoisonPower"])]),
    (K::PoisonIfPoisoned, None, &[(0, &["PoisonPower"])]),
    (K::PoisonRandomSerial, None, &[(0, &["PoisonPower"]), (1, &["Repeat"])]),
    (K::PrepTime, None, &[(0, &["PrepTimePower"])]),
    (K::PullFromBelowExact, None, &[(0, &["Damage"])]),
    (K::PurityExact, None, &[(0, &["Cards"])]),
    (K::Pyre, None, &[(0, &["Energy"])]),
    (K::RadiateExact, None, &[(0, &["Damage"])]),
    (K::Rage, None, &[(0, &["Power"])]),
    (K::RallyExact, None, &[(0, &["Block"])]),
    (K::RampageExact, None, &[(0, &["Damage"]), (1, &["Increase"])]),
    (K::RattleExact, None, &[(0, &["OstyDamage"])]),
    (K::RemoveOrbSlotsExact, None, &[(0, &["OrbSlots"])]),
    (K::RendExact, None, &[(0, &["CalculationBase"]), (1, &["ExtraDamage"])]),
    (K::RestlessnessExact, None, &[(0, &["Cards"])]),
    (K::RollingBoulder, None, &[(0, &["RollingBoulderPower"])]),
    (K::Rupture, None, &[(0, &["StrengthPower"])]),
    (K::SameTurnHistoryBody, Some((0, "evil_eye")), &[(1, &["Block"])]),
    (K::SameTurnHistoryBody, Some((0, "ftl")), &[(1, &["Damage"]), (2, &["PlayMax"])]),
    (K::SameTurnHistoryBody, Some((0, "spite")), &[(1, &["Damage"]), (2, &["Repeat"])]),
    (K::ScrapeExact, None, &[(0, &["Damage"]), (1, &["Cards"])]),
    (K::SeekerStrikeExact, None, &[(0, &["Damage"]), (1, &["Cards"])]),
    (K::SentryMode, None, &[(0, &["SentryModePower"])]),
    (K::SerpentForm, None, &[(0, &["SerpentFormPower"])]),
    (K::Shadowmeld, None, &[(0, &["Power"])]),
    (K::ShatterOrbsExact, None, &[(0, &["Damage"])]),
    (K::ShockwaveExact, None, &[(0, &["Power"])]),
    (K::Shroud, None, &[(0, &["Block"])]),
    (K::SicEmExact, None, &[(0, &["OstyDamage"]), (1, &["SicEmPower"])]),
    (K::SignalBoost, None, &[(0, &["SignalBoostPower"])]),
    (K::SleightOfFlesh, None, &[(0, &["SleightOfFleshPower"])]),
    (K::Smokestack, None, &[(0, &["SmokestackPower"])]),
    (K::Sneaky, None, &[(0, &["SneakyPower"])]),
    (K::SoulBody, Some((0, "attack")), &[(1, &["Damage"])]),
    (K::SoulBody, Some((0, "block")), &[(1, &["Block"])]),
    (K::SoulBody, Some((0, "unpowered_unblockable_damage")), &[(1, &["Damage"])]),
    (K::SpectrumShift, None, &[(0, &["Cards"])]),
    (K::Speedster, None, &[(0, &["SpeedsterPower"])]),
    (K::Spinner, None, &[(0, &["SpinnerPower"])]),
    (K::SpiritOfAsh, None, &[(0, &["BlockOnExhaust"])]),
    (K::Stampede, None, &[(0, &["Power"])]),
    (K::Stars, None, &[(0, &["Stars"])]),
    (K::StompExact, None, &[(0, &["Damage"])]),
    (K::Storm, None, &[(0, &["StormPower"])]),
    (K::Strangle, None, &[(0, &["StranglePower"])]),
    (K::Strength, None, &[(0, &["StrengthPower"])]),
    (K::StrengthEnemy, None, &[(0, &["EnemyStrength"])]),
    (K::Summon, Some((0, "OSTY")), &[(1, &["Summon"])]),
    (K::SummonNextTurn, None, &[(0, &["Summon"])]),
    (K::SwordSage, None, &[(0, &["SwordSagePower"])]),
    (K::SynchronizeUniqueOrbFocusExact, None, &[(0, &["CalculationExtra"])]),
    (K::TearAsunderExact, None, &[(0, &["Damage"])]),
    (K::TempDexterity, None, &[(0, &["DexterityPower"])]),
    (K::TempFocus, None, &[(0, &["FocusPower"])]),
    (K::TempStrength, None, &[(0, &["StrengthPower"])]),
    (K::TempStrengthEnemy, None, &[(0, &["StrengthLoss"])]),
    (K::TheBallExact, None, &[(0, &["Damage"]), (1, &["Increase"])]),
    (K::TheBombExact, None, &[(0, &["Turns"]), (1, &["BombDamage"])]),
    (K::TheHuntExact, None, &[(0, &["Damage"])]),
    (K::TheScytheExact, None, &[(0, &["Damage"]), (1, &["Increase"])]),
    (K::Thorns, None, &[(0, &["ThornsPower"])]),
    (K::Thunder, None, &[(0, &["ThunderPower"])]),
    (K::ThunderclapExact, None, &[(0, &["Damage"]), (1, &["VulnerablePower"])]),
    (K::ToricToughnessExact, None, &[(0, &["Block"]), (1, &["Turns"])]),
    (K::TurnEndGoldLossExact, None, &[(0, &["Gold"])]),
    (K::UndeathExact, None, &[(0, &["Block"])]),
    (K::Vicious, None, &[(0, &["Cards"])]),
    (K::Vigor, None, &[(0, &["VigorPower"])]),
    (K::VoidForm, None, &[(0, &["VoidFormPower"])]),
    (K::VulnerableThenStrengthCurrent, None, &[(0, &["VulnerablePower"])]),
    (K::WraithForm, None, &[(0, &["WraithFormPower"])]),
];

/// Integer arguments with no var role: literals in the card's own IL. Checked
/// before `ROLES`, so a row here also overrides a role (Ice Lance's `Repeat`
/// counts Frost channels, not hits). `(card, kind, index, value, citation)`.
#[rustfmt::skip]
const LITERALS: &[(&str, K, usize, i64, &str)] = &[
    ("AGGRESSION", K::Aggression, 0, 1, "Aggression/<OnPlay>d__1::MoveNext RVA 0x389654 IL_00af -> spec:Apply<AggressionPower>"),
    ("ASTRAL_PULSE", K::AttackAll, 1, 2, "AstralPulse/<OnPlay>d__5::MoveNext RVA 0x38a994 IL_0049 -> AttackCommand::WithHitCount"),
    ("BARRICADE", K::Barricade, 0, 1, "Barricade/<OnPlay>d__3::MoveNext RVA 0x38b388 IL_00af -> spec:Apply<BarricadePower>"),
    ("BATTLE_TRANCE", K::NoDraw, 0, 1, "BattleTrance/<OnPlay>d__3::MoveNext RVA 0x38b6bc IL_00ad -> spec:Apply<NoDrawPower>"),
    ("BEACON_OF_HOPE", K::BeaconOfHopeExact, 0, 1, "BeaconOfHope/<OnPlay>d__5::MoveNext RVA 0x38b824 IL_00af -> spec:Apply<BeaconOfHopePower>"),
    ("CONQUEROR", K::ForgeFamilyExact, 2, 1, "Conqueror/<OnPlay>d__5::MoveNext RVA 0x393f3c IL_0143 -> spec:Apply<ConquerorPower>"),
    ("CONVERGENCE", K::RetainHand, 0, 1, "Convergence/<OnPlay>d__5::MoveNext RVA 0x39469c IL_00bb -> spec:Apply<RetainHandPower>"),
    ("DAGGER_SPRAY", K::AttackAll, 1, 2, "DaggerSpray/<OnPlay>d__4::MoveNext RVA 0x395f60 IL_0044 -> AttackCommand::WithHitCount"),
    ("DAGGER_THROW", K::Draw, 0, 1, "DaggerThrow/<OnPlay>d__3::MoveNext RVA 0x3960b8 IL_0115 -> CardPileCmd::Draw"),
    ("DARK_EMBRACE", K::DarkEmbrace, 0, 1, "DarkEmbrace/<OnPlay>d__3::MoveNext RVA 0x3964fc IL_00af -> spec:Apply<DarkEmbracePower>"),
    ("DEPRECATED_CARD", K::Draw, 0, 1, "DeprecatedCard/<OnPlay>d__5::MoveNext RVA 0x398bb8 IL_002a -> CardPileCmd::Draw"),
    ("DUALCAST", K::EvokeFrontExact, 0, 2, "Dualcast/<OnPlay>d__5::MoveNext RVA 0x39a7b4 IL_00d9 and IL_01b2: two OrbCmd::EvokeNext calls"),
    ("FAN_OF_KNIVES", K::FanOfKnivesExact, 0, 1, "FanOfKnives/<OnPlay>d__8::MoveNext RVA 0x39d294 IL_003d -> spec:Apply<FanOfKnivesPower>"),
    ("FLANKING", K::FlankingExact, 0, 2, "Flanking/<OnPlay>d__3::MoveNext RVA 0x39f168 IL_00c4 -> Decimal::.ctor -> IL_00d7 spec:Apply<FlankingPower>"),
    ("HAMMER_TIME", K::HammerTimeExact, 0, 1, "HammerTime/<OnPlay>d__5::MoveNext RVA 0x3a3458 IL_00af -> spec:Apply<HammerTimePower>"),
    ("HELLO_WORLD", K::HelloWorld, 0, 1, "HelloWorld/<OnPlay>d__3::MoveNext RVA 0x3a4bc8 IL_00af -> spec:Apply<HelloWorldPower>"),
    ("HELLRAISER", K::Hellraiser, 0, 1, "Hellraiser/<OnPlay>d__3::MoveNext RVA 0x3a4e44 IL_002e -> spec:Apply<HellraiserPower>"),
    ("HIBERNATE", K::HibernatePowerExact, 0, 1, "Hibernate/<OnPlay>d__7::MoveNext RVA 0x3a50e0 IL_0035 -> spec:Apply<HibernatePower>"),
    ("ICE_LANCE", K::Attack, 1, 1, "IceLance/<OnPlay>d__5::MoveNext RVA 0x3a67e8 IL_004c DamageCmd::Attack with no WithHitCount (AttackCommand::.ctor RVA 0x1349a0 IL_000d _hitCount 1); its Repeat is read at IL_0164 as the Channel<FrostOrb> count"),
    ("INFINITE_BLADES", K::InfiniteBlades, 0, 1, "InfiniteBlades/<OnPlay>d__3::MoveNext RVA 0x3a744c IL_00af -> spec:Apply<InfiniteBladesPower>"),
    ("JUGGLING", K::JugglingExact, 0, 1, "Juggling/<OnPlay>d__1::MoveNext RVA 0x3a84f8 IL_00af -> spec:Apply<JugglingPower>"),
    ("MASTER_PLANNER", K::MasterPlanner, 0, 1, "MasterPlanner/<OnPlay>d__3::MoveNext RVA 0x3abafc IL_00af -> spec:Apply<MasterPlannerPower>"),
    ("MAYHEM", K::Mayhem, 0, 1, "Mayhem/<OnPlay>d__1::MoveNext RVA 0x3abe58 IL_002e -> spec:Apply<MayhemPower>"),
    ("NECRO_MASTERY", K::NecroMastery, 0, 1, "NecroMastery/<OnPlay>d__5::MoveNext RVA 0x3ae420 IL_012f -> spec:Apply<NecroMasteryPower>"),
    ("NOSTALGIA", K::Nostalgia, 0, 1, "Nostalgia/<OnPlay>d__1::MoveNext RVA 0x3af5f0 IL_002e -> spec:Apply<NostalgiaPower>"),
    ("POUNCE", K::FreeSkill, 0, 1, "Pounce/<OnPlay>d__3::MoveNext RVA 0x3b319c IL_00eb -> spec:Apply<FreeSkillPower>"),
    ("PREDATOR", K::DrawNextTurn, 0, 2, "Predator/<OnPlay>d__3::MoveNext RVA 0x3b34c4 IL_00eb -> Decimal::.ctor -> IL_00fe spec:Apply<DrawCardsNextTurnPower>"),
    ("REAPER_FORM", K::ReaperForm, 0, 1, "ReaperForm/<OnPlay>d__3::MoveNext RVA 0x3b5d9c IL_00af -> spec:Apply<ReaperFormPower>"),
    ("REBOUND", K::Rebound, 0, 1, "Rebound/<OnPlay>d__5::MoveNext RVA 0x3b63fc IL_00eb -> spec:Apply<ReboundPower>"),
    ("REFLECT", K::Reflect, 0, 1, "Reflect/<OnPlay>d__7::MoveNext RVA 0x3b67b0 IL_012f -> spec:Apply<ReflectPower>"),
    ("RESONANCE", K::PowerAllSerial, 1, -1, "Resonance/<OnPlay>d__7::MoveNext RVA 0x3b71cc IL_0173 -> spec:Apply<StrengthPower>"),
    ("RIP_AND_TEAR", K::AttackRandom, 1, 2, "RipAndTear/<OnPlay>d__5::MoveNext RVA 0x3b7998 IL_0035 -> AttackCommand::WithHitCount"),
    ("SALVO", K::RetainHand, 0, 1, "Salvo/<OnPlay>d__5::MoveNext RVA 0x3b8390 IL_00eb -> spec:Apply<RetainHandPower>"),
    ("SEEKING_EDGE", K::ForgeFamilyExact, 1, 1, "SeekingEdge/<OnPlay>d__5::MoveNext RVA 0x3b99fc IL_00b7 -> spec:Apply<SeekingEdgePower>"),
    ("SHADOW_STEP", K::ShadowStepPowerExact, 0, 1, "ShadowStep/<OnPlay>d__3::MoveNext RVA 0x3ba870 IL_00a6 -> spec:Apply<ShadowStepPower>"),
    ("SOULBOUND", K::SoulboundExact, 0, 1, "Soulbound/<OnPlay>d__5::MoveNext RVA 0x3bd0bc IL_0150 -> spec:Apply<SoulboundPower>"),
    ("STRATAGEM", K::Stratagem, 0, 1, "Stratagem/<OnPlay>d__1::MoveNext RVA 0x3bfac4 IL_002e -> spec:Apply<StratagemPower>"),
    ("SUBROUTINE", K::Subroutine, 0, 1, "Subroutine/<OnPlay>d__1::MoveNext RVA 0x3c014c IL_00af -> spec:Apply<SubroutinePower>"),
    ("SYNTHESIS", K::FreePower, 0, 1, "Synthesis/<OnPlay>d__3::MoveNext RVA 0x3c18cc IL_00eb -> spec:Apply<FreePowerPower>"),
    ("TANK", K::TankExact, 0, 1, "Tank/<OnPlay>d__7::MoveNext RVA 0x3c1d08 IL_00af -> spec:Apply<TankPower>"),
    ("THE_GAMBIT", K::TheGambit, 0, 1, "TheGambit/<OnPlay>d__5::MoveNext RVA 0x3c2a80 IL_00ad -> spec:Apply<TheGambitPower>"),
    ("THE_SEALED_THRONE", K::SealedThrone, 0, 1, "TheSealedThrone/<OnPlay>d__3::MoveNext RVA 0x3c3028 IL_00af -> spec:Apply<TheSealedThronePower>"),
    ("THRASH", K::ThrashExact, 1, 2, "Thrash/<OnPlay>d__9::MoveNext RVA 0x3c3510 IL_0051 -> AttackCommand::WithHitCount"),
    ("TOOLS_OF_THE_TRADE", K::ToolsOfTheTrade, 0, 1, "ToolsOfTheTrade/<OnPlay>d__1::MoveNext RVA 0x3c3ec8 IL_00af -> spec:Apply<ToolsOfTheTradePower>"),
    ("TRACKING", K::Tracking, 0, 50, "Tracking/<OnPlay>d__3::MoveNext RVA 0x3c42c0 IL_00af -> Decimal::.ctor -> IL_00c3 spec:Apply<TrackingPower>"),
    ("TRASH_TO_TREASURE", K::TrashToTreasure, 0, 1, "TrashToTreasure/<OnPlay>d__3::MoveNext RVA 0x3c4648 IL_00af -> spec:Apply<TrashToTreasurePower>"),
    ("TWIN_STRIKE", K::Attack, 1, 2, "TwinStrike/<OnPlay>d__5::MoveNext RVA 0x3c4f70 IL_004a -> AttackCommand::WithHitCount"),
    ("TYRANNY", K::Tyranny, 0, 1, "Tyranny/<OnPlay>d__3::MoveNext RVA 0x3c5094 IL_00af -> spec:Apply<TyrannyPower>"),
    ("UNDERWORLD", K::Underworld, 0, 1, "Underworld/<OnPlay>d__7::MoveNext RVA 0x3c5580 IL_00af -> spec:Apply<UnderworldPower>"),
    ("UNMOVABLE", K::Unmovable, 0, 1, "Unmovable/<OnPlay>d__3::MoveNext RVA 0x3c5878 IL_00c0 -> spec:Apply<UnmovablePower>"),
    ("UNRELENTING", K::FreeAttack, 0, 1, "Unrelenting/<OnPlay>d__3::MoveNext RVA 0x3c5a00 IL_011a -> spec:Apply<FreeAttackPower>"),
    ("UPROAR", K::UproarExact, 1, 2, "Uproar/<OnPlay>d__3::MoveNext RVA 0x3c61ac IL_005d -> AttackCommand::WithHitCount"),
    ("VEILPIERCER", K::FreeEthereal, 0, 1, "Veilpiercer/<OnPlay>d__5::MoveNext RVA 0x3c6408 IL_00eb -> spec:Apply<VeilpiercerPower>"),
];

/// Kinds (or `kind:mode`) with an integer argument and no role map yet.
/// Pinned both ways: mapping one removes it here; a new one must be added.
const NOT_YET_MAPPED: &[&str] = &[
    "attack_result:block_total_overkill",
    "attack_result:doom_total",
    "channel:DARK",
    "channel:FROST",
    "channel:GLASS",
    "channel:LIGHTNING",
    "channel:PLASMA",
    "darkness",
    "enlightenment_exact",
    "friendship_exact",
    "hyperbeam_focus_down_exact",
    "sacrifice_osty_exact",
    "select:discard",
    "select:draw",
    "select:hand",
    "shared_fate_exact",
    "tesla_coil",
    "up_my_sleeve_cost_exact",
];

fn mode_word(arg: &Arg) -> Option<&'static str> {
    match arg {
        Arg::S(word) => Some(word),
        Arg::Power(power) => Some(power.as_str()),
        _ => None,
    }
}

fn spec_for(kind: K, args: &[Arg]) -> Option<&'static Spec> {
    let moded = ROLES.iter().find(|(k, mode, _)| {
        *k == kind && mode.is_some_and(|(at, word)| args.get(at).and_then(mode_word) == Some(word))
    });
    moded.or_else(|| {
        ROLES
            .iter()
            .find(|(k, mode, _)| *k == kind && mode.is_none())
    })
}

fn unmapped_key(kind: K, args: &[Arg]) -> String {
    let has_moded = ROLES
        .iter()
        .any(|(k, mode, _)| *k == kind && mode.is_some());
    let moded = ROLES
        .iter()
        .find(|(k, mode, _)| *k == kind && mode.is_some());
    match (has_moded, moded) {
        (true, Some((_, Some((at, _)), _))) => format!(
            "{}:{}",
            kind.as_str(),
            args.get(*at).and_then(mode_word).unwrap_or("?")
        ),
        _ => match args.first().and_then(mode_word) {
            Some(word) if kind == K::Channel || kind == K::Select => {
                format!("{}:{word}", kind.as_str())
            }
            _ => kind.as_str().to_string(),
        },
    }
}

fn check_role(
    row: &CardRow,
    vars: &Map<String, Value>,
    names: &[&str],
    value: i64,
) -> Result<(), String> {
    for name in names {
        match *name {
            "=1 hits" => {
                return (value == 1)
                    .then_some(())
                    .ok_or_else(|| "no hit-count var, so native hits are 1".to_string());
            }
            "=upgrade" => {
                return (value == i64::from(row.upgrade))
                    .then_some(())
                    .ok_or_else(|| format!("expected the upgrade level {}", row.upgrade));
            }
            _ => {
                if let Some(var) = vars.get(*name) {
                    return (decimal(var) == Decimal::from(value))
                        .then_some(())
                        .ok_or_else(|| format!("native {name} = {}", decimal(var)));
                }
            }
        }
    }
    Err(format!(
        "declares none of {names:?} (vars {:?})",
        vars.keys().collect::<Vec<_>>()
    ))
}

#[test]
fn card_step_arguments_match_their_native_var_roles() {
    let manifest = manifest();
    let mut problems = Vec::new();
    let mut unmapped: BTreeSet<String> = BTreeSet::new();
    let mut used_literals = BTreeSet::new();
    let (mut by_role, mut by_literal, mut total) = (0_usize, 0_usize, 0_usize);
    for row in &CARD_ROWS {
        let snap = level(&manifest, row);
        let vars = snap["vars"].as_object().unwrap();
        for step in row.steps {
            let ints: Vec<(usize, i64)> = step
                .args
                .iter()
                .enumerate()
                .filter_map(|(i, a)| match a {
                    Arg::I(v) => Some((i, *v)),
                    _ => None,
                })
                .collect();
            if ints.is_empty() {
                continue;
            }
            total += ints.len();
            let spec = spec_for(step.kind, step.args);
            let all_literal = ints.iter().all(|(index, _)| {
                LITERALS
                    .iter()
                    .any(|(c, k, i, _, _)| *c == row.id.as_str() && *k == step.kind && i == index)
            });
            if spec.is_none() && !all_literal {
                unmapped.insert(unmapped_key(step.kind, step.args));
            }
            for (index, value) in ints {
                let at = format!(
                    "{}@{} {}[{index}]={value}",
                    row.id.as_str(),
                    row.upgrade,
                    step.kind.as_str()
                );
                let literal = LITERALS.iter().position(|(c, k, i, _, _)| {
                    *c == row.id.as_str() && *k == step.kind && *i == index
                });
                if let Some(position) = literal {
                    used_literals.insert(position);
                    by_literal += 1;
                    if LITERALS[position].3 != value {
                        problems.push(format!("{at}: LITERALS says {}", LITERALS[position].3));
                    }
                    continue;
                }
                let Some((_, _, roles)) = spec else {
                    continue;
                };
                let Some((_, names)) = roles.iter().find(|(i, _)| *i == index) else {
                    problems.push(format!("{at}: position has no role"));
                    continue;
                };
                match check_role(row, vars, names, value) {
                    Ok(()) => by_role += 1,
                    Err(why) => problems.push(format!("{at}: {why}")),
                }
            }
        }
    }
    for (position, entry) in LITERALS.iter().enumerate() {
        if !used_literals.contains(&position) {
            problems.push(format!("stale LITERALS entry {entry:?}"));
        }
        if !entry.4.contains("RVA 0x") || !entry.4.contains("IL_") {
            problems.push(format!(
                "LITERALS entry without an RVA/IL citation {entry:?}"
            ));
        }
    }
    let pinned: BTreeSet<String> = NOT_YET_MAPPED.iter().map(|s| s.to_string()).collect();
    for key in unmapped.difference(&pinned) {
        problems.push(format!(
            "{key} has integer arguments and no role map; map it or pin it"
        ));
    }
    for key in pinned.difference(&unmapped) {
        problems.push(format!(
            "{key} is pinned as unmapped but is mapped or gone; unpin it"
        ));
    }
    eprintln!(
        "{by_role} of {total} integer step arguments checked by role, {by_literal} allowlisted \
         literals, {} kinds/modes not yet mapped",
        unmapped.len()
    );
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// #2942: every ported Mad Science variant row reads each of its integer
/// arguments from the var the native body reads, at its own level, and carries
/// the level's cost and keywords. The role map is per rider, from
/// `MadScience`'s nested `MoveNext` bodies (v0.111.0 `sts2.dll` sha256
/// `9cb4f1ad…`): `<ExecuteAttack>d__52` RVA 0x3aa530 attacks with `Damage`
/// (IL_0042-IL_0052) times `ViolenceHits` for Violence and 1 otherwise
/// (IL_0020-IL_0041); `<ExecuteSkill>d__53` RVA 0x3aae28 blocks with `Block`
/// (IL_001d-IL_003a); `<ExecuteRider>d__57` RVA 0x3aa9c4 applies
/// `SappingWeak`/`SappingVulnerable` (IL_0065-IL_011e), `ChokingDamage`
/// (IL_0182-IL_01af), gains `EnergizedEnergy` (IL_0212-IL_0232) and draws
/// `WisdomCards` (IL_0295-IL_02bb); `<ExecutePower>d__54` RVA 0x3aa660 applies
/// `ExpertiseStrength` then `ExpertiseDexterity` (IL_00ce-IL_0192).
#[test]
fn mad_science_variant_rows_read_their_native_vars_by_role() {
    use sts_sim::content_tables::MAD_SCIENCE_VARIANT_ROWS;
    let manifest = manifest();
    let mut checked = 0;
    for variant in &MAD_SCIENCE_VARIANT_ROWS {
        let Some(row) = variant.row.as_ref() else {
            continue;
        };
        let snap = level(&manifest, row);
        let var = |name: &str| decimal(&snap["vars"][name]);
        assert_eq!(row.cost, snap["cost"].as_i64().unwrap());
        let innate = snap["keywords"]
            .as_array()
            .unwrap()
            .iter()
            .any(|k| k == "Innate");
        assert_eq!(
            row.innate, innate,
            "{} L{}",
            variant.rider_name, row.upgrade
        );
        for step in row.steps {
            let roles: Vec<Option<&str>> = match step.kind {
                K::Attack if variant.rider_name == "Violence" => {
                    vec![Some("Damage"), Some("ViolenceHits")]
                }
                K::Attack => vec![Some("Damage"), None],
                K::Weak => vec![Some("SappingWeak")],
                K::Vulnerable => vec![Some("SappingVulnerable")],
                K::Strangle => vec![Some("ChokingDamage")],
                K::Block => vec![Some("Block")],
                K::Energy => vec![Some("EnergizedEnergy")],
                K::Draw => vec![Some("WisdomCards")],
                K::Strength => vec![Some("ExpertiseStrength")],
                K::Dexterity => vec![Some("ExpertiseDexterity")],
                // #3322: Chaos reads no var (`<ExecuteRider>d__57`
                // IL_031e-IL_0395 passes the literal count 1).
                K::MadScienceChaosExact => vec![],
                kind => panic!("{kind:?} has no Mad Science role"),
            };
            assert_eq!(roles.len(), step.args.len(), "{:?}", step.kind);
            for (role, arg) in roles.iter().zip(step.args) {
                let Arg::I(value) = arg else {
                    panic!("{:?} carries a non-integer argument", step.kind);
                };
                let expected = role.map_or(Decimal::ONE, var);
                assert_eq!(
                    Decimal::from(*value),
                    expected,
                    "{} L{} {:?} {role:?}",
                    variant.rider_name,
                    row.upgrade,
                    step.kind
                );
                checked += 1;
            }
        }
    }
    // Per level: Sapping 4, Violence 2, Choking 3, Energized 2, Wisdom 2,
    // Chaos 1 (its Block; the Chaos step has no argument), Expertise 2.
    assert_eq!(checked, 32, "every ported argument is checked");
}

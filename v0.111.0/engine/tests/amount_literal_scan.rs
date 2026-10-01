//! #3027: no new bare amount literal in the hand-written relic, potion and
//! card-step code without a cited source.
//!
//! Bone Flute (#2830) carried `gain_flat_block(.., 4, ..)` for two builds: the
//! IL read had taken `BlockVar(2m, ValueProp.Unpowered)`'s props operand (4)
//! as the amount. Fetch (#2519) drew a literal 3 cards where the card's
//! `CardsVar` is 1. Both were bare integers in hand-written code, and neither
//! was ever compared against the game.
//!
//! This test lexes the scanned files (comments, strings and `#[cfg(test)]`
//! items removed) and records every nonzero integer literal passed as a whole
//! argument to a call or tuple, plus every integer `const`. Each occurrence is
//! keyed by file, enclosing `fn`, callee, value and the nearest preceding
//! `RelicId::`/`PotionId::`/`CardId::` in that `fn` (the owner), and the
//! multiset of keys must equal `ALLOWED` exactly: a new literal, or a changed
//! one, fails until it is added with a citation. A citation is either
//!
//! * `var <Name>` — the owner's canonical var in
//!   `data/canonical_vars.v0.111.0.json` (#3027's DLL-derived manifest) has
//!   exactly this value (for a card, at some upgrade level). The test checks
//!   it, so a wrong amount cannot be allowlisted by naming a var; or
//! * an IL citation naming an RVA and an IL offset, for a literal that is not
//!   a var (a counter period, a fixed channel count, a structural constant).
//!
//! Callees in `STRUCTURAL` take positions, sizes and indices rather than game
//! amounts and are skipped, as is the `I(..)` compiled-argument pattern, which
//! the role test in `canonical_vars_manifest.rs` covers from the table side.

use rust_decimal::Decimal;
use serde_json::Value;
use std::collections::BTreeMap;
use std::str::FromStr;
use sts_sim::ids::{CardId, PotionId, RelicId};

/// Hand-written relic and potion bodies: every literal.
const SCANNED: &[(&str, &str)] = &[
    ("engine/relics.rs", include_str!("../src/engine/relics.rs")),
    (
        "engine/potions.rs",
        include_str!("../src/engine/potions.rs"),
    ),
];

/// Card step handlers: draw-count literals only (callees named `draw*`), the
/// Fetch class. The handlers' other amounts arrive as compiled step arguments
/// that `canonical_vars_manifest.rs` checks by role; widening this scan to
/// every literal in `steps/` is future work (about 330 unseeded sites).
const DRAW_SCANNED: &[(&str, &str)] = &[
    (
        "steps/block_next.rs",
        include_str!("../src/steps/block_next.rs"),
    ),
    (
        "steps/calculated_history.rs",
        include_str!("../src/steps/calculated_history.rs"),
    ),
    ("steps/curses.rs", include_str!("../src/steps/curses.rs")),
    (
        "steps/defect_common.rs",
        include_str!("../src/steps/defect_common.rs"),
    ),
    (
        "steps/defect_orb.rs",
        include_str!("../src/steps/defect_orb.rs"),
    ),
    (
        "steps/defect_rare.rs",
        include_str!("../src/steps/defect_rare.rs"),
    ),
    (
        "steps/defect_uncommon.rs",
        include_str!("../src/steps/defect_uncommon.rs"),
    ),
    (
        "steps/hand_cap.rs",
        include_str!("../src/steps/hand_cap.rs"),
    ),
    (
        "steps/ironclad_common.rs",
        include_str!("../src/steps/ironclad_common.rs"),
    ),
    (
        "steps/ironclad_rare.rs",
        include_str!("../src/steps/ironclad_rare.rs"),
    ),
    (
        "steps/ironclad_uncommon.rs",
        include_str!("../src/steps/ironclad_uncommon.rs"),
    ),
    ("steps/mod.rs", include_str!("../src/steps/mod.rs")),
    (
        "steps/necrobinder_common.rs",
        include_str!("../src/steps/necrobinder_common.rs"),
    ),
    (
        "steps/necrobinder_osty.rs",
        include_str!("../src/steps/necrobinder_osty.rs"),
    ),
    (
        "steps/necrobinder_rare.rs",
        include_str!("../src/steps/necrobinder_rare.rs"),
    ),
    (
        "steps/necrobinder_uncommon.rs",
        include_str!("../src/steps/necrobinder_uncommon.rs"),
    ),
    ("steps/neutral.rs", include_str!("../src/steps/neutral.rs")),
    (
        "steps/physical_cost.rs",
        include_str!("../src/steps/physical_cost.rs"),
    ),
    (
        "steps/physical_lifecycle.rs",
        include_str!("../src/steps/physical_lifecycle.rs"),
    ),
    (
        "steps/regent_ancient.rs",
        include_str!("../src/steps/regent_ancient.rs"),
    ),
    (
        "steps/regent_forge.rs",
        include_str!("../src/steps/regent_forge.rs"),
    ),
    (
        "steps/regent_uncommon.rs",
        include_str!("../src/steps/regent_uncommon.rs"),
    ),
    (
        "steps/resource_powers.rs",
        include_str!("../src/steps/resource_powers.rs"),
    ),
    (
        "steps/same_turn_history.rs",
        include_str!("../src/steps/same_turn_history.rs"),
    ),
    ("steps/shared.rs", include_str!("../src/steps/shared.rs")),
    (
        "steps/silent_rare.rs",
        include_str!("../src/steps/silent_rare.rs"),
    ),
    (
        "steps/silent_special.rs",
        include_str!("../src/steps/silent_special.rs"),
    ),
    (
        "steps/silent_uncommon.rs",
        include_str!("../src/steps/silent_uncommon.rs"),
    ),
    (
        "steps/single_pile_selection.rs",
        include_str!("../src/steps/single_pile_selection.rs"),
    ),
    ("steps/status.rs", include_str!("../src/steps/status.rs")),
    (
        "steps/templates.rs",
        include_str!("../src/steps/templates.rs"),
    ),
];

/// Callees whose integer arguments are positions, sizes or indices.
const STRUCTURAL: &[&str] = &[
    "I",
    "take",
    "skip",
    "nth",
    "get",
    "get_mut",
    "with_capacity",
    "pow",
    "windows",
    "split_off",
    "truncate",
    "swap",
    "resize",
    "insert",
    "remove",
    "try_fold",
    "fold",
    "step_by",
    "rotate_left",
    "rotate_right",
    "is_multiple_of",
    "rem_euclid",
    "div_euclid",
    "shl",
    "shr",
];

/// `(file, fn, callee, value, owner, count, citation)`.
type Allowed = (
    &'static str,
    &'static str,
    &'static str,
    i64,
    &'static str,
    usize,
    &'static str,
);

#[rustfmt::skip]
const ALLOWED: &[Allowed] = &[
    ("engine/potions.rs", "alchemize_one_attempt", "checked_add", 2, "", 1, "structural: Rust-side RNG draw-count arithmetic"),
    ("engine/potions.rs", "apply", "inject_generated_exact_bottom", 1, "PotionId::ColorlessPotion", 1, "CosmicConcoction/<OnUse>d__8::MoveNext RVA 0x34c934 IL_00a7 CardPileCmd::AddGeneratedCardToCombat (one card per add)"),
    ("engine/potions.rs", "apply_belt_buckle_after_use", "apply_signed_player_stat", 2, "", 1, "var RELIC.BELT_BUCKLE:DexterityPower"),
    ("engine/potions.rs", "apply_snecko_oil_after_draw", "next_bounded", 4, "", 1, "SneckoOil::NextEnergyCost RVA 0xad6cf IL_0026 ldc.i4.4 -> Rng::NextInt (CombatEnergyCosts)"),
    ("engine/potions.rs", "begin_fairy_wrapper_after_lethal", "max", 1, "PotionId::FairyInABottle", 1, "FairyInABottle/<OnUse>d__8::MoveNext RVA 0x34d8bc IL_0046 Decimal::One -> IL_004b Math::Max (heal floor)"),
    ("engine/potions.rs", "begin_generation_choice", "saturating_sub", 1, "", 1, "structural: index/size arithmetic over a pool or slot list"),
    ("engine/potions.rs", "belt_buckle_after_procured", "apply_signed_player_stat", -2, "", 1, "var -RELIC.BELT_BUCKLE:DexterityPower"),
    ("engine/potions.rs", "bone_brew", "summon_osty", 15, "", 1, "var POTION.BONE_BREW:Summon"),
    ("engine/potions.rs", "consume_first_fairy_after_lethal", "max", 1, "PotionId::FairyInABottle", 1, "FairyInABottle/<OnUse>d__8::MoveNext RVA 0x34d8bc IL_0046 Decimal::One -> IL_004b Math::Max (heal floor)"),
    ("engine/potions.rs", "delicate_frond_before_combat_start", "checked_mul", 2, "", 1, "structural: Rust-side RNG draw-count arithmetic"),
    ("engine/potions.rs", "enter", "checked_add", 1, "PotionId::DistilledChaos", 1, "structural: Rust-side RNG draw-count arithmetic"),
    ("engine/potions.rs", "entropic_brew", "checked_mul", 2, "", 1, "structural: Rust-side RNG draw-count arithmetic"),
    ("engine/potions.rs", "finish", "draw_cards_for_potion", 1, "RelicId::RelicReptileTrinket", 1, "UnceasingTop/<AfterHandEmptied>d__4::MoveNext RVA 0x333994 IL_005e CardPileCmd::Draw of one card (Unceasing Top declares no vars)"),
    ("engine/potions.rs", "finish", "gain_temp_strength", 3, "RelicId::RelicReptileTrinket", 1, "var StrengthPower"),
    ("engine/potions.rs", "generation_selection_action_count", "then_some", 4, "", 1, "structural: the generation choice's legal-action count (three options plus skip)"),
    ("engine/potions.rs", "glowwater_draw_tail", "draw_cards_for_potion", 10, "PotionId::GlowwaterPotion", 1, "var Cards"),
    ("engine/potions.rs", "held_target_applications_are_exact", "checked_mul", 4, "PotionId::BeetleJuice", 1, "var Repeat"),
    ("engine/potions.rs", "held_target_applications_are_exact", "checked_mul", 9, "PotionId::PowderedDemise", 1, "var Demise"),
    ("engine/potions.rs", "manual_part_d_prerequisites_are_exact", "checked_add", 2, "PotionId::FairyInABottle", 1, "var RELIC.BELT_BUCKLE:DexterityPower"),
    ("engine/potions.rs", "manual_part_d_prerequisites_are_exact", "checked_mul", 5, "PotionId::RegenPotion", 1, "var RegenPower"),
    ("engine/potions.rs", "ordered_pick_count", "tuple", 1, "", 2, "structural: permutation-count accumulator seeds"),
    ("engine/potions.rs", "orobic_rng_draws", "saturating_sub", 1, "", 1, "structural: Rust-side RNG draw-count arithmetic"),
    ("engine/potions.rs", "preflight_distilled_target_rng", "checked_add", 1, "", 1, "structural: Rust-side RNG draw-count arithmetic"),
    ("engine/potions.rs", "preflight_distilled_target_rng", "min", 3, "", 1, "var POTION.DISTILLED_CHAOS:Repeat"),
    ("engine/potions.rs", "preflight_manual_potion_rng", "checked_add", 1, "PotionId::EntropicBrew", 1, "structural: Rust-side RNG draw-count arithmetic"),
    ("engine/potions.rs", "preflight_manual_potion_rng", "checked_mul", 2, "PotionId::EntropicBrew", 1, "structural: Rust-side RNG draw-count arithmetic"),
    ("engine/potions.rs", "preflight_manual_potion_rng", "saturating_sub", 1, "PotionId::OrobicAcid", 1, "structural: Rust-side RNG draw-count arithmetic"),
    ("engine/potions.rs", "resume_after_draw", "checked_cold_add", 3, "PotionId::Clarity", 1, "var ClarityPower"),
    ("engine/potions.rs", "resume_bottled_after_shuffle", "draw_cards_for_potion", 5, "PotionId::BottledPotential", 1, "var Cards"),
    ("engine/potions.rs", "resume_distilled_gather_inner", "checked_add", 1, "", 1, "structural: history/progress counter increment, not an amount"),
    ("engine/potions.rs", "run_ashwater_exhaust_sequence", "checked_add", 1, "PotionId::Ashwater", 1, "structural: history/progress counter increment, not an amount"),
    ("engine/potions.rs", "soldiers_stew", "checked_add", 1, "", 1, "SoldiersStew::OnUse RVA 0xad728 IL_0081 ldc.i4.1 -> CardModel::set_BaseReplayCount (+1 replay)"),
    ("engine/potions.rs", "target_application_is_exact", "tuple", 4, "PotionId::BeetleJuice", 1, "var Repeat"),
    ("engine/potions.rs", "target_application_is_exact", "tuple", 9, "PotionId::PowderedDemise", 1, "var Demise"),
    ("engine/potions.rs", "use_potion", "add_slots", 2, "PotionId::PotionOfCapacity", 1, "var Repeat"),
    ("engine/potions.rs", "use_potion", "apply_block_next_turn", 10, "PotionId::ShipInABottle", 1, "var Block"),
    ("engine/potions.rs", "use_potion", "apply_owner_strength", 1, "PotionId::FyshOil", 1, "var StrengthPower"),
    ("engine/potions.rs", "use_potion", "apply_owner_strength", 2, "PotionId::StrengthPotion", 1, "var StrengthPower"),
    ("engine/potions.rs", "use_potion", "apply_potion_temporary_strength", 7, "PotionId::ShacklingPotion", 1, "var StrengthPower"),
    ("engine/potions.rs", "use_potion", "inject_generated_storm_shivs", 1, "PotionId::CunningPotion", 1, "CunningPotion/<OnUse>d__10::MoveNext RVA 0x34cae0 IL_00c4-IL_00da CardCmd::Upgrade over each returned Shiv (upgrade level 1)"),
    ("engine/potions.rs", "use_potion", "inject_generated_storm_shivs", 3, "PotionId::CunningPotion", 1, "var Cards"),
    ("engine/potions.rs", "use_potion", "checked_add", 1, "PotionId::CureAll", 1, "var Energy"),
    ("engine/potions.rs", "use_potion", "checked_add", 1, "PotionId::RadiantTincture", 1, "var Energy"),
    ("engine/potions.rs", "use_potion", "checked_add", 2, "PotionId::EnergyPotion", 1, "var Energy"),
    ("engine/potions.rs", "use_potion", "checked_cold_add", 1, "PotionId::Duplicator", 1, "Duplicator/<OnUse>d__6::MoveNext RVA 0x34d154 IL_002d Decimal::One -> IL_003a Apply<DuplicationPower>"),
    ("engine/potions.rs", "use_potion", "checked_cold_add", 1, "PotionId::GigantificationPotion", 1, "var GigantificationPower"),
    ("engine/potions.rs", "use_potion", "checked_cold_add", 1, "PotionId::MazalethsGift", 1, "var RitualPower"),
    ("engine/potions.rs", "use_potion", "checked_cold_add", 3, "PotionId::Clarity", 1, "var ClarityPower"),
    ("engine/potions.rs", "use_potion", "checked_cold_add", 3, "PotionId::RadiantTincture", 1, "var RadiancePower"),
    ("engine/potions.rs", "use_potion", "checked_cold_add", 5, "PotionId::RegenPotion", 1, "var RegenPower"),
    ("engine/potions.rs", "use_potion", "checked_mul", 2, "PotionId::Fortifier", 1, "Fortifier/<OnUse>d__8::MoveNext RVA 0x34dccc IL_0032 ldc.i4.2 -> IL_003c CreatureCmd::GainBlock (block x2)"),
    ("engine/potions.rs", "use_potion", "checked_power_add", 1, "PotionId::FyshOil", 1, "var DexterityPower"),
    ("engine/potions.rs", "use_potion", "checked_power_add", 1, "PotionId::GhostInAJar", 1, "var IntangiblePower"),
    ("engine/potions.rs", "use_potion", "checked_power_add", 1, "PotionId::LuckyTonic", 1, "var BufferPower"),
    ("engine/potions.rs", "use_potion", "checked_power_add", 2, "PotionId::DexterityPotion", 1, "var DexterityPower"),
    ("engine/potions.rs", "use_potion", "checked_power_add", 2, "PotionId::FocusPotion", 1, "var FocusPower"),
    ("engine/potions.rs", "use_potion", "checked_power_add", 2, "PotionId::StableSerum", 1, "var Repeat"),
    ("engine/potions.rs", "use_potion", "checked_power_add", 3, "PotionId::LiquidBronze", 1, "var ThornsPower"),
    ("engine/potions.rs", "use_potion", "checked_power_add", 7, "PotionId::HeartOfIron", 1, "var PlatingPower"),
    ("engine/potions.rs", "use_potion", "damage_player_from_power_with_catalog", 12, "PotionId::FoulPotion", 1, "var Damage"),
    ("engine/potions.rs", "use_potion", "debuff", 1, "PotionId::PotionOfBinding", 2, "var VulnerablePower"),
    ("engine/potions.rs", "use_potion", "debuff", 3, "PotionId::VulnerablePotion", 1, "var VulnerablePower"),
    ("engine/potions.rs", "use_potion", "debuff", 3, "PotionId::WeakPotion", 1, "var WeakPower"),
    ("engine/potions.rs", "use_potion", "debuff", 4, "PotionId::BeetleJuice", 1, "var Repeat"),
    ("engine/potions.rs", "use_potion", "debuff", 6, "PotionId::PoisonPotion", 1, "var PoisonPower"),
    ("engine/potions.rs", "use_potion", "debuff", 9, "PotionId::PowderedDemise", 1, "var Demise"),
    ("engine/potions.rs", "use_potion", "debuff", 33, "PotionId::PotionOfDoom", 1, "var DoomPower"),
    ("engine/potions.rs", "use_potion", "draw_cards_for_potion", 1, "PotionId::Clarity", 1, "var Cards"),
    ("engine/potions.rs", "use_potion", "draw_cards_for_potion", 2, "PotionId::CureAll", 1, "var Cards"),
    ("engine/potions.rs", "use_potion", "draw_cards_for_potion", 3, "PotionId::SwiftPotion", 1, "var Cards"),
    ("engine/potions.rs", "use_potion", "draw_cards_for_potion", 5, "PotionId::BottledPotential", 1, "var Cards"),
    ("engine/potions.rs", "use_potion", "draw_cards_for_potion", 7, "PotionId::SneckoOil", 1, "var Cards"),
    ("engine/potions.rs", "use_potion", "from_i64", 10, "PotionId::ExplosiveAmpoule", 1, "var Damage"),
    ("engine/potions.rs", "use_potion", "from_i64", 10, "PotionId::ShipInABottle", 1, "var Block"),
    ("engine/potions.rs", "use_potion", "from_i64", 12, "PotionId::BlockPotion", 1, "var Block"),
    ("engine/potions.rs", "use_potion", "from_i64", 12, "PotionId::FoulPotion", 1, "var Damage"),
    ("engine/potions.rs", "use_potion", "from_i64", 15, "PotionId::PotionShapedRock", 1, "var Damage"),
    ("engine/potions.rs", "use_potion", "from_i64", 20, "PotionId::FirePotion", 1, "var Damage"),
    ("engine/potions.rs", "use_potion", "gain_stars", 3, "PotionId::StarPotion", 1, "var Stars"),
    ("engine/potions.rs", "use_potion", "gain_temp_dexterity", 5, "PotionId::SpeedPotion", 1, "var DexterityPower"),
    ("engine/potions.rs", "use_potion", "gain_temp_strength", 5, "PotionId::FlexPotion", 1, "var StrengthPower"),
    ("engine/relics.rs", "<const>", "BLOOD_VIAL_HEAL", 2, "", 1, "var RELIC.BLOOD_VIAL:Heal"),
    ("engine/relics.rs", "<const>", "CRACKED_CORE_CHANNELS", 1, "", 1, "var RELIC.CRACKED_CORE:Lightning"),
    ("engine/relics.rs", "<const>", "FENCING_MANUAL_FORGE", 10, "", 1, "var RELIC.FENCING_MANUAL:Forge"),
    ("engine/relics.rs", "<const>", "FESTIVE_POPPER_DAMAGE", 9, "", 1, "var RELIC.FESTIVE_POPPER:Damage"),
    ("engine/relics.rs", "<const>", "SYMBIOTIC_VIRUS_CHANNELS", 1, "", 1, "var RELIC.SYMBIOTIC_VIRUS:Dark"),
    ("engine/relics.rs", "after_block_cleared", "gain_flat_block", 14, "RelicId::RelicHornCleat", 1, "var Block"),
    ("engine/relics.rs", "after_card_discarded", "from_i64", 3, "RelicId::RelicTingsha", 1, "var Damage"),
    ("engine/relics.rs", "after_card_discarded", "gain_flat_block", 3, "RelicId::RelicToughBandages", 1, "var Block"),
    ("engine/relics.rs", "after_card_exhausted", "checked_add", 1, "RelicId::RelicJossPaper", 2, "var Cards"),
    ("engine/relics.rs", "after_card_exhausted", "from_i64", 1, "RelicId::RelicForgottenSoul", 1, "var Damage"),
    ("engine/relics.rs", "after_card_exhausted", "from_i64", 3, "RelicId::RelicCharonsAshes", 1, "var Damage"),
    ("engine/relics.rs", "after_card_exhausted", "inject_generated_clones_bottom", 1, "RelicId::RelicBurningSticks", 1, "BurningSticks/<AfterCardExhausted>d__9::MoveNext RVA 0x320d7c IL_0072 ldc.i4.1 -> IL_0073 AddGeneratedCardToCombat (one clone)"),
    ("engine/relics.rs", "after_card_played_peer_segment", "apply_owner_strength", 1, "RelicId::RelicRainbowRing", 1, "var StrengthPower"),
    ("engine/relics.rs", "after_card_played_peer_segment", "apply_owner_strength", 1, "RelicId::RelicShuriken", 1, "var StrengthPower"),
    ("engine/relics.rs", "after_card_played_peer_segment", "apply_signed_player_stat", 1, "RelicId::RelicKunai", 1, "var DexterityPower"),
    ("engine/relics.rs", "after_card_played_peer_segment", "apply_signed_player_stat", 1, "RelicId::RelicRainbowRing", 1, "var DexterityPower"),
    ("engine/relics.rs", "after_card_played_peer_segment", "checked_add", 1, "RelicId::RelicHelicalDart", 1, "var DexterityPower"),
    ("engine/relics.rs", "after_card_played_peer_segment", "checked_add", 1, "RelicId::RelicIvoryTile", 1, "var Energy"),
    ("engine/relics.rs", "after_card_played_peer_segment", "checked_add", 1, "RelicId::RelicNunchaku", 1, "var Energy"),
    ("engine/relics.rs", "after_card_played_hand", "checked_add", 1, "RelicId::RelicTuningFork", 1, "structural: history/progress counter increment, not an amount"),
    ("engine/relics.rs", "after_card_played_peer_segment", "draw_cards", 1, "RelicId::RelicGamePiece", 1, "var Cards"),
    ("engine/relics.rs", "iron_club_after_card_played", "draw_cards", 1, "RelicId::RelicIronClub", 1, "IronClub/<AfterCardPlayed>d__19::MoveNext RVA 0x326f78 IL_0089 Decimal::One -> IL_0095 CardPileCmd::Draw"),
    ("engine/relics.rs", "after_card_played_peer_segment", "from_i64", 5, "RelicId::RelicLetterOpener", 1, "var Damage"),
    ("engine/relics.rs", "after_card_played_peer_segment", "from_i64", 6, "RelicId::RelicKusarigama", 1, "var Damage"),
    ("engine/relics.rs", "after_card_played_peer_segment", "from_i64", 8, "RelicId::RelicLostWisp", 1, "var Damage"),
    ("engine/relics.rs", "after_card_played_peer_segment", "gain_flat_block", 1, "RelicId::RelicDaughterOfTheWind", 1, "var Block"),
    ("engine/relics.rs", "after_card_played_peer_segment", "gain_flat_block", 4, "RelicId::RelicOrnamentalFan", 1, "var Block"),
    ("engine/relics.rs", "permafrost_after_card_played", "gain_flat_block", 7, "RelicId::RelicPermafrost", 1, "var Block"),
    ("engine/relics.rs", "after_card_played_peer_segment", "inject_generated_clones_bottom", 1, "RelicId::RelicMusicBox", 1, "MusicBox/<AfterCardPlayed>d__13::MoveNext RVA 0x32ae04 IL_0064 ldc.i4.1 -> IL_0065 AddGeneratedCardToCombat (one clone)"),
    ("engine/relics.rs", "after_card_played_peer_segment", "set_paels_legion_cooldown", 2, "RelicId::RelicPaelsLegion", 1, "var Turns"),
    ("engine/relics.rs", "after_energy_reset", "checked_add", 1, "RelicId::RelicArtOfWar", 1, "var Energy"),
    ("engine/relics.rs", "after_energy_reset", "checked_add", 1, "RelicId::RelicFakeVenerableTeaSet", 1, "var Energy"),
    ("engine/relics.rs", "after_energy_reset", "checked_add", 2, "RelicId::RelicVenerableTeaSet", 1, "var Energy"),
    ("engine/relics.rs", "after_energy_reset", "summon", 1, "RelicId::RelicBoundPhylactery", 1, "var Summon"),
    ("engine/relics.rs", "after_pet_attack", "gain_flat_block", 2, "RelicId::RelicBoneFlute", 1, "var Block"),
    ("engine/relics.rs", "after_player_turn_start", "begin_generation_relic_selection", 5, "RelicId::RelicChoicesParadox", 1, "var Cards"),
    ("engine/relics.rs", "after_shuffle", "gain_flat_block", 6, "RelicId::RelicTheAbacus", 1, "var Block"),
    ("engine/relics.rs", "after_side_turn_end_hand_relic", "from_i64", 6, "RelicId::RelicParryingShield", 1, "var Damage"),
    ("engine/relics.rs", "after_side_turn_start", "checked_add", 1, "RelicId::RelicBoomingConch", 1, "var Energy"),
    ("engine/relics.rs", "after_side_turn_start", "checked_add", 1, "RelicId::RelicLantern", 1, "var Energy"),
    ("engine/relics.rs", "after_side_turn_start", "inject_generated_bottom", 1, "RelicId::RelicOrangeDough", 1, "OrangeDough/<AfterSideTurnStart>d__4::MoveNext RVA 0x32bd7c IL_00c4 AddGeneratedCardsToCombat, one card per generated id"),
    ("engine/relics.rs", "after_side_turn_start", "tuple", 1, "RelicId::RelicFakeHappyFlower", 1, "var Energy"),
    ("engine/relics.rs", "after_side_turn_start", "tuple", 1, "RelicId::RelicHappyFlower", 1, "var Energy"),
    ("engine/relics.rs", "after_side_turn_start", "tuple", 2, "RelicId::RelicCandelabra", 1, "var Energy"),
    ("engine/relics.rs", "after_side_turn_start", "tuple", 3, "RelicId::RelicChandelier", 1, "var Energy"),
    ("engine/relics.rs", "after_side_turn_start", "tuple", 3, "RelicId::RelicHappyFlower", 1, "var Turns"),
    ("engine/relics.rs", "after_side_turn_start", "tuple", 5, "RelicId::RelicFakeHappyFlower", 1, "var Turns"),
    ("engine/relics.rs", "brimstone_after_side_turn_start", "apply_monster_strength_delta_after_type_two_gate", 1, "RelicId::RelicBrimstone", 1, "var EnemyStrength"),
    ("engine/relics.rs", "brimstone_after_side_turn_start", "apply_owner_strength", 2, "RelicId::RelicBrimstone", 1, "var SelfStrength"),
    ("engine/relics.rs", "after_side_turn_start_late", "checked_add", 1, "RelicId::RelicSealOfGold", 1, "var Energy"),
    ("engine/relics.rs", "after_side_turn_start_late", "checked_add", 2, "RelicId::RelicPaelsTears", 1, "var Energy"),
    ("engine/relics.rs", "after_side_turn_start_late", "checked_sub", 3, "RelicId::RelicSealOfGold", 1, "var Gold"),
    ("engine/relics.rs", "after_side_turn_start_late", "saturating_sub", 2, "RelicId::RelicBread", 1, "var LoseEnergy"),
    ("engine/relics.rs", "after_side_turn_start_late", "summon_osty", 2, "RelicId::RelicPhylacteryUnbound", 1, "var StartOfTurn"),
    ("engine/relics.rs", "after_stars_spent", "apply_owner_strength", 1, "RelicId::RelicMiniRegent", 1, "var StrengthPower"),
    ("engine/relics.rs", "before_hand_draw", "begin_generation_relic_selection", 3, "RelicId::RelicToolbox", 1, "var Cards"),
    ("engine/relics.rs", "before_side_turn_end_damage_relics", "from_i64", 20, "RelicId::RelicScreamingFlagon", 1, "var Damage"),
    ("engine/relics.rs", "before_side_turn_end_damage_relics", "from_i64", 52, "RelicId::RelicStoneCalendar", 1, "var Damage"),
    ("engine/relics.rs", "before_side_turn_end_block_relics", "gain_flat_block", 4, "RelicId::RelicRippleBasin", 1, "var Block"),
    ("engine/relics.rs", "before_side_turn_end_block_relics", "tuple", 3, "RelicId::RelicFakeOrichalcum", 1, "var Block"),
    ("engine/relics.rs", "before_side_turn_end_block_relics", "tuple", 6, "RelicId::RelicOrichalcum", 1, "var Block"),
    ("engine/relics.rs", "toasty_mittens_select", "apply_owner_strength", 1, "RelicId::RelicToastyMittens", 1, "var StrengthPower"),
    ("engine/relics.rs", "continue_after_toasty_mittens", "damage_player_from_power_with_catalog", 4, "RelicId::RelicRoyalPoison", 1, "var Damage"),
    ("engine/relics.rs", "dragon_fruit_after_gold_gained", "gain_player_max_hp", 1, "RelicId::RelicDragonFruit", 1, "var MaxHp"),
    ("engine/relics.rs", "intimidating_helmet_before_card_played", "gain_flat_block", 4, "RelicId::RelicIntimidatingHelmet", 1, "var Block"),
    ("engine/relics.rs", "modifier_total", "checked_add", 1, "RelicId::RelicBread", 1, "var GainEnergy"),
    ("engine/relics.rs", "modifier_total", "checked_add", 1, "RelicId::RelicPaelsFlesh", 1, "var Energy"),
    ("engine/relics.rs", "modifier_total", "checked_add", 1, "RelicId::RelicPhilosophersStone", 1, "var Energy"),
    ("engine/relics.rs", "modifier_total", "checked_add", 1, "RelicId::RelicPumpkinCandle", 1, "var Energy"),
    ("engine/relics.rs", "modifier_total", "checked_add", 1, "RelicId::RelicSpikedGauntlets", 1, "var Energy"),
    ("engine/relics.rs", "modifier_total", "checked_add", 1, "RelicId::RelicVelvetChoker", 1, "var Energy"),
    ("engine/relics.rs", "modifier_total", "checked_add", 1, "RelicId::RelicWhisperingEarring", 1, "var Energy"),
    ("engine/relics.rs", "modifier_total", "tuple", -2, "RelicId::RelicBigMushroom", 1, "var -Cards"),
    ("engine/relics.rs", "modifier_total", "tuple", 1, "RelicId::RelicBagOfMarbles", 1, "var VulnerablePower"),
    ("engine/relics.rs", "modifier_total", "tuple", 1, "RelicId::RelicPendulum", 1, "var Cards"),
    ("engine/relics.rs", "modifier_total", "tuple", 1, "RelicId::RelicRedMask", 1, "var WeakPower"),
    ("engine/relics.rs", "modifier_total", "tuple", 2, "RelicId::RelicBagOfPreparation", 1, "var Cards"),
    ("engine/relics.rs", "modifier_total", "tuple", 2, "RelicId::RelicFiddle", 1, "var Cards"),
    ("engine/relics.rs", "modifier_total", "tuple", 2, "RelicId::RelicRingOfTheDrake", 2, "var Cards"),
    ("engine/relics.rs", "modifier_total", "tuple", 2, "RelicId::RelicRingOfTheSnake", 1, "var Cards"),
    ("engine/relics.rs", "modifier_total", "tuple", 2, "RelicId::RelicSneckoEye", 1, "var Cards"),
    ("engine/relics.rs", "modifier_total", "tuple", 3, "RelicId::RelicPocketwatch", 1, "var CardThreshold"),
    ("engine/relics.rs", "radiant_pearl_before_hand_draw", "inject_generated_bottom", 1, "RelicId::RelicRadiantPearl", 1, "var Cards"),
    ("engine/relics.rs", "relic_pending_is_exact", "tuple", 3, "RelicId::RelicToolbox", 1, "var Cards"),
    ("engine/relics.rs", "relic_pending_is_exact", "tuple", 5, "RelicId::RelicChoicesParadox", 1, "var Cards"),
    ("engine/relics.rs", "resume_relic_selection", "apply_owner_strength", 1, "RelicId::RelicToastyMittens", 1, "var StrengthPower"),
    ("engine/relics.rs", "resume_relic_selection", "inject_generated_bottom", 1, "RelicId::RelicToastyMittens", 1, "var StrengthPower"),
    ("engine/relics.rs", "tuning_fork_after_card_played", "checked_add", 1, "RelicId::RelicTuningFork", 1, "structural: history/progress counter increment, not an amount"),
    ("engine/relics.rs", "tuning_fork_after_card_played", "gain_flat_block", 7, "RelicId::RelicTuningFork", 1, "var Block"),
    ("steps/neutral.rs", "continue_restlessness", "draw_cards_for_card_result", 1, "", 1, "Restlessness/<OnPlay>d__9::MoveNext RVA 0x3b7454 IL_0046 CardPileCmd::Draw single-card overload, looped Cards times"),
    ("steps/regent_forge.rs", "forge_writer_body", "draw_cardplay_no_result", 2, "CardId::SovereignBlade", 1, "var CARD.SPOILS_OF_BATTLE:Cards"),
    ("steps/regent_forge.rs", "forge_writer_body", "draw_cardplay_owned_tail", 1, "CardId::SovereignBlade", 1, "var CARD.BIG_BANG:Cards"),
    ("steps/regent_forge.rs", "forge_writer_body", "draw_cards", 1, "CardId::SovereignBlade", 1, "var CARD.BIG_BANG:Cards"),
    ("steps/regent_uncommon.rs", "huddle_up_remote_suffix", "draw_for_player", 1, "", 1, "structural: the multiplayer ally key (player slot 1), not a draw count"),
    ("steps/same_turn_history.rs", "same_turn_history_body", "draw_cardplay_no_result", 1, "CardId::Ftl", 1, "var Cards"),
    ("steps/silent_uncommon.rs", "continue_pillage_draws", "draw_cards_for_card_result", 1, "", 1, "Pillage/<OnPlay>d__3::MoveNext RVA 0x3b2734 IL_00e6 CardPileCmd::Draw single-card overload, looped"),
    ("steps/silent_uncommon.rs", "escape_plan_exact", "draw_cardplay_owned_result", 1, "", 1, "EscapePlan/<OnPlay>d__5::MoveNext RVA 0x39c314 IL_002a ldc.i4.1 -> CardPileCmd::Draw"),
    ("steps/silent_uncommon.rs", "escape_plan_exact", "draw_cards_into", 1, "", 1, "EscapePlan/<OnPlay>d__5::MoveNext RVA 0x39c314 IL_002a ldc.i4.1 -> CardPileCmd::Draw"),
];

/// Blank comments, string and char literals, keeping byte offsets.
fn strip_comments_and_strings(src: &str) -> Vec<u8> {
    let b = src.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in &mut out[from..to] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    while i < b.len() {
        if b[i..].starts_with(b"//") {
            let end = b[i..]
                .iter()
                .position(|&c| c == b'\n')
                .map_or(b.len(), |p| i + p);
            blank(&mut out, i, end);
            i = end;
        } else if b[i..].starts_with(b"/*") {
            let end = src[i + 2..].find("*/").map_or(b.len(), |p| i + 2 + p + 2);
            blank(&mut out, i, end);
            i = end;
        } else if b[i] == b'r'
            && (b[i + 1..].starts_with(b"\"") || b[i + 1..].starts_with(b"#\""))
            && (i == 0 || !(b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_'))
        {
            let hashes = b[i + 1..].iter().take_while(|&&c| c == b'#').count();
            let close = format!("\"{}", "#".repeat(hashes));
            let start = i + 1 + hashes + 1;
            let end = src[start..]
                .find(&close)
                .map_or(b.len(), |p| start + p + close.len());
            blank(&mut out, i, end);
            i = end;
        } else if b[i] == b'"' {
            let mut j = i + 1;
            while j < b.len() && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            blank(&mut out, i, (j + 1).min(b.len()));
            i = j + 1;
        } else if b[i] == b'\'' {
            // A char literal closes within a few bytes; a lifetime does not.
            let close = if b.get(i + 1) == Some(&b'\\') {
                b[i + 2..]
                    .iter()
                    .take(10)
                    .position(|&c| c == b'\'')
                    .map(|p| i + 2 + p)
            } else {
                (b.get(i + 2) == Some(&b'\'')).then_some(i + 2)
            };
            match close {
                Some(end) => {
                    blank(&mut out, i, end + 1);
                    i = end + 1;
                }
                None => i += 1,
            }
        } else {
            i += 1;
        }
    }
    out
}

/// Blank every `#[cfg(test)]` item (a `{..}` body or a `;`-terminated item).
fn strip_test_items(code: &mut [u8]) {
    let marker = b"#[cfg(test)]";
    let mut i = 0;
    while let Some(p) = code[i..].windows(marker.len()).position(|w| w == marker) {
        let start = i + p;
        let mut j = start + marker.len();
        let mut depth = 0_i32;
        let mut opened = false;
        while j < code.len() {
            match code[j] {
                b'(' | b'[' => depth += 1,
                b'{' => {
                    depth += 1;
                    if depth == 1 {
                        opened = true;
                    }
                }
                b')' | b']' => depth -= 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 && opened {
                        j += 1;
                        break;
                    }
                }
                b';' if depth == 0 => {
                    j += 1;
                    break;
                }
                _ => {}
            }
            j += 1;
        }
        for c in &mut code[start..j] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
        i = j;
    }
}

fn ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The identifier ending just before `end` (skipping whitespace), if any.
fn ident_before(code: &[u8], end: usize) -> Option<(usize, &str)> {
    let mut e = end;
    while e > 0 && code[e - 1].is_ascii_whitespace() {
        e -= 1;
    }
    let mut s = e;
    while s > 0 && ident_byte(code[s - 1]) {
        s -= 1;
    }
    (s < e).then(|| (s, std::str::from_utf8(&code[s..e]).unwrap()))
}

fn integer_literal(arg: &str) -> Option<i64> {
    let arg = arg.trim();
    let (negative, digits) = match arg.strip_prefix('-') {
        Some(rest) => (true, rest.trim_start()),
        None => (false, arg),
    };
    let number: String = digits
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '_')
        .collect();
    if number.is_empty() || !number.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let suffix = digits[number.len()..].trim_start_matches('_');
    if ![
        "", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "usize", "isize",
    ]
    .contains(&suffix)
    {
        return None;
    }
    let value: i64 = number.replace('_', "").parse().ok()?;
    Some(if negative { -value } else { value })
}

const KEYWORDS: &[&str] = &[
    "if", "while", "match", "return", "for", "in", "as", "let", "mut", "else", "move", "ref",
    "Some", "Ok", "Err",
];

/// `(fn, callee, value, owner)` for every scanned literal in one file.
fn scan(src: &str) -> Vec<(String, String, i64, String)> {
    let mut code = strip_comments_and_strings(src);
    strip_test_items(&mut code);
    let text = std::str::from_utf8(&code).unwrap();
    let mut fns = Vec::new();
    let mut owners = Vec::new();
    for (pos, _) in text.match_indices("fn ") {
        if pos == 0 || !ident_byte(code[pos - 1]) {
            let name: String = text[pos + 3..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                fns.push((pos, name));
            }
        }
    }
    for prefix in ["RelicId::", "PotionId::", "CardId::"] {
        for (pos, _) in text.match_indices(prefix) {
            if pos > 0 && ident_byte(code[pos - 1]) {
                continue;
            }
            let variant: String = text[pos + prefix.len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            owners.push((pos, format!("{prefix}{variant}")));
        }
    }
    owners.sort();
    // The enclosing `fn`, and the id the literal belongs to inside it: the
    // next id when it is the `coverage::record_relic(..)` closing the
    // literal's block (hand-written relic hooks end each block that way),
    // else the nearest preceding id (`owns(RelicId::X)`, a match arm), else
    // the next one.
    let context = |pos: usize| -> (String, String) {
        let index = fns.iter().rposition(|(p, _)| *p < pos);
        let (fn_start, name) =
            index.map_or((0, "<file>".to_string()), |i| (fns[i].0, fns[i].1.clone()));
        let fn_end = index
            .and_then(|i| fns.get(i + 1))
            .map_or(text.len(), |(p, _)| *p);
        let in_fn = |p: usize| p >= fn_start && p < fn_end;
        let next = owners.iter().find(|(p, _)| *p > pos && in_fn(*p));
        let closing =
            next.filter(|(p, _)| text[p.saturating_sub(30)..*p].contains("record_relic("));
        let previous = owners.iter().rev().find(|(p, _)| *p < pos && in_fn(*p));
        let owner = closing
            .or(previous)
            .or(next)
            .map_or(String::new(), |(_, o)| o.clone());
        (name, owner)
    };
    let mut hits = Vec::new();
    for (open, _) in text.match_indices('(') {
        let callee = match ident_before(&code, open) {
            Some((_, name)) if !KEYWORDS.contains(&name) => name.to_string(),
            _ => "tuple".to_string(),
        };
        // `name!(` is a macro, not a call.
        let mut e = open;
        while e > 0 && code[e - 1].is_ascii_whitespace() {
            e -= 1;
        }
        if e > 0 && code[e - 1] == b'!' {
            continue;
        }
        if STRUCTURAL.contains(&callee.as_str()) {
            continue;
        }
        let mut depth = 0_i32;
        let mut arg_start = open + 1;
        let mut j = open + 1;
        while j < code.len() {
            let c = code[j];
            let at_end = c == b')' && depth == 0;
            if (c == b',' && depth == 0) || at_end {
                if let Some(value) = integer_literal(&text[arg_start..j]).filter(|v| *v != 0) {
                    let (name, owner) = context(arg_start);
                    hits.push((name, callee.clone(), value, owner));
                }
                if at_end {
                    break;
                }
                arg_start = j + 1;
            } else if matches!(c, b'(' | b'[' | b'{') {
                depth += 1;
            } else if matches!(c, b')' | b']' | b'}') {
                depth -= 1;
            }
            j += 1;
        }
    }
    for (pos, _) in text.match_indices("const ") {
        if pos > 0 && ident_byte(code[pos - 1]) {
            continue;
        }
        let rest = &text[pos + 6..];
        let Some(eq) = rest.find('=') else { continue };
        let Some(semi) = rest.find(';') else { continue };
        if semi < eq || rest[..eq].contains('(') {
            continue;
        }
        let name = rest[..rest.find(':').unwrap_or(eq)].trim().to_string();
        if let Some(value) = integer_literal(&rest[eq + 1..semi]).filter(|v| *v != 0) {
            // A module-level const has no enclosing owner; its citation names
            // the model explicitly (`var RELIC.X:Name`).
            hits.push(("<const>".to_string(), name, value, String::new()));
        }
    }
    hits
}

fn manifest() -> Value {
    serde_json::from_str(include_str!("../data/canonical_vars.v0.111.0.json")).unwrap()
}

/// The owner's manifest vars, one map per level (relics and potions: one).
fn owner_vars<'a>(manifest: &'a Value, owner: &str) -> Vec<&'a serde_json::Map<String, Value>> {
    let (prefix, variant) = owner.split_once("::").unwrap();
    let key = match prefix {
        "RelicId" => RelicId::ALL
            .iter()
            .find(|id| format!("{id:?}") == variant)
            .map(|id| id.as_str().to_string()),
        "PotionId" => PotionId::ALL
            .iter()
            .find(|id| format!("{id:?}") == variant)
            .map(|id| format!("POTION.{}", id.as_str())),
        "CardId" => CardId::ALL
            .iter()
            .find(|id| format!("{id:?}") == variant)
            .map(|id| format!("CARD.{}", id.as_str())),
        _ => None,
    };
    key.map_or_else(Vec::new, |key| model_vars(manifest, &key))
}

/// A manifest model's vars by model id (`RELIC.X`, `POTION.X`, `CARD.X`).
fn model_vars<'a>(manifest: &'a Value, key: &str) -> Vec<&'a serde_json::Map<String, Value>> {
    let family = match key.split_once('.').map(|(f, _)| f) {
        Some("RELIC") => "relics",
        Some("POTION") => "potions",
        Some("CARD") => "cards",
        _ => return Vec::new(),
    };
    let Some(model) = manifest[family].get(key) else {
        return Vec::new();
    };
    match model.get("levels") {
        Some(levels) => levels
            .as_array()
            .unwrap()
            .iter()
            .map(|level| level["vars"].as_object().unwrap())
            .collect(),
        None => vec![model["vars"].as_object().unwrap()],
    }
}

fn var_named(levels: &[&serde_json::Map<String, Value>], name: &str, value: i64) -> bool {
    levels.iter().any(|vars| {
        vars.get(name).is_some_and(|var| {
            Decimal::from_str(var["value"].as_str().unwrap()).unwrap() == Decimal::from(value)
        })
    })
}

#[test]
fn every_amount_literal_is_allowlisted_with_a_checked_citation() {
    let manifest = manifest();
    let mut found: BTreeMap<(String, String, String, i64, String), usize> = BTreeMap::new();
    let scoped = SCANNED
        .iter()
        .map(|entry| (entry, false))
        .chain(DRAW_SCANNED.iter().map(|entry| (entry, true)));
    for ((file, src), draw_only) in scoped {
        for (name, callee, value, owner) in scan(src) {
            if draw_only && !callee.starts_with("draw") {
                continue;
            }
            *found
                .entry((file.to_string(), name, callee, value, owner))
                .or_default() += 1;
        }
    }
    let mut problems = Vec::new();
    let mut allowed: BTreeMap<(String, String, String, i64, String), usize> = BTreeMap::new();
    for &(file, name, callee, value, owner, count, cite) in ALLOWED {
        let key = (
            file.to_string(),
            name.to_string(),
            callee.to_string(),
            value,
            owner.to_string(),
        );
        if allowed.insert(key, count).is_some() {
            problems.push(format!(
                "duplicate ALLOWED row {file} {name} {callee} {value} {owner}"
            ));
        }
        if let Some(var) = cite.strip_prefix("var ") {
            let (negated, var) = match var.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, var),
            };
            let (levels, var) = match var.split_once(':') {
                Some((model, var)) => (model_vars(&manifest, model), var),
                None if owner.is_empty() => (Vec::new(), var),
                None => (owner_vars(&manifest, owner), var),
            };
            let expected = if negated { -value } else { value };
            if !var_named(&levels, var, expected) {
                problems.push(format!(
                    "{file} {name} {callee}({value}) {owner}: cites `{cite}` but that manifest var \
                     is not {expected}"
                ));
            }
        } else if cite.contains("RVA 0x") && cite.contains("IL_") {
            // An IL literal that is not a var.
        } else if cite
            .strip_prefix("structural: ")
            .is_some_and(|why| why.len() > 10)
        {
            // Rust-side bookkeeping with no game amount.
        } else {
            problems.push(format!(
                "{file} {name} {callee}({value}) {owner}: citation {cite:?} is not `var <Name>`, \
                 an RVA + IL offset, or `structural: <reason>`"
            ));
        }
    }
    for (key, count) in &found {
        if allowed.get(key) != Some(count) {
            let (file, name, callee, value, owner) = key;
            let hint = if owner.is_empty() {
                None
            } else {
                let levels = owner_vars(&manifest, owner);
                levels
                    .iter()
                    .flat_map(|vars| vars.iter())
                    .find(|(_, var)| {
                        Decimal::from_str(var["value"].as_str().unwrap()).unwrap()
                            == Decimal::from(*value)
                    })
                    .map(|(n, _)| format!("var {n}"))
            };
            problems.push(format!(
                "NEW    (\"{file}\", \"{name}\", \"{callee}\", {value}, \"{owner}\", {count}, \"{}\"),",
                hint.unwrap_or_else(|| "TODO".to_string())
            ));
        }
    }
    for (key, count) in &allowed {
        if found.get(key) != Some(count) {
            problems.push(format!(
                "STALE  {key:?} x{count} (found {:?})",
                found.get(key)
            ));
        }
    }
    eprintln!(
        "{} literal sites across {} files",
        found.values().sum::<usize>(),
        SCANNED.len() + DRAW_SCANNED.len()
    );
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
}

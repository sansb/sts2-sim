//! Generated monster-move dispatch — DO NOT EDIT BY HAND.
//!
//! Regenerate with:
//!
//! ```text
//! python3 sim/v0.111.0/engine/tools/generate_content.py
//! ```
//!
//! PORT_PLAN.md D4. This file is the **complete** dispatch surface: one arm
//! per MoveKind variant (93 of them), each delegating to a body in the
//! per-family module that owns it. It is generated once and never edited by a
//! wave PR, so two wave PRs cannot conflict here; a wave PR replaces a stub
//! body in its own `moves/<family>.rs` and flips nothing shared.
//!
//! The family split is derived, not assigned:
//! a kind belongs to the `content/encounters/*.py` pool whose monsters use
//! it, `spawned` when its only monster is one no pool builder constructs,
//! and `shared` when more than one pool does.
//!
//! Freshness (this file, and the existence of every dispatch target) is
//! CI-enforced by the `rust port` workflow's generator `--check` step.

pub mod boss;
pub mod elite;
pub mod normal;
pub mod shared;
pub mod spawned;

use crate::engine::{EngineRefusal, MoveCtx};
use crate::ids::MoveKind;

/// Every MoveKind, with the family module that owns its body.
///
/// Published so the completeness/uniqueness contract test can be
/// derived rather than hand-listed.
#[rustfmt::skip]
pub static FAMILY_OF: [(MoveKind, &str); 93] = [
    (MoveKind::AddStatus, "elite"), (MoveKind::AddStatusDiscard, "shared"),
    (MoveKind::AddStatusHand, "normal"), (MoveKind::AeonglassIntensity, "boss"),
    (MoveKind::Attack, "shared"), (MoveKind::AttackBeckon, "boss"), (MoveKind::AttackBlock,
    "shared"), (MoveKind::AttackBlockVital, "elite"), (MoveKind::AttackCharge, "normal"),
    (MoveKind::AttackDexterity, "normal"), (MoveKind::AttackFrail, "shared"),
    (MoveKind::AttackPileInject, "elite"), (MoveKind::AttackScrollBranch, "normal"),
    (MoveKind::AttackSmoggy, "normal"), (MoveKind::AttackSpawnAggro, "normal"),
    (MoveKind::AttackSteal, "normal"), (MoveKind::AttackStrength, "shared"),
    (MoveKind::AttackStrengthBranch, "normal"), (MoveKind::AttackTangle, "normal"),
    (MoveKind::AttackVigor, "elite"), (MoveKind::AttackVulnWeakPlayer, "spawned"),
    (MoveKind::AttackVulnerableDeferredOvicopter, "normal"), (MoveKind::AttackWeakFrail,
    "shared"), (MoveKind::AttackWeakPlayer, "shared"), (MoveKind::AttackWounds, "boss"),
    (MoveKind::AxebotBootup, "normal"), (MoveKind::BeastCry, "boss"), (MoveKind::BeastStamp,
    "boss"), (MoveKind::Beckon, "boss"), (MoveKind::Bloat, "normal"), (MoveKind::Block,
    "shared"), (MoveKind::BlockStrength, "shared"), (MoveKind::BuffStrength, "shared"),
    (MoveKind::BuffTeamBlock, "normal"), (MoveKind::BuffTeamStrength, "spawned"),
    (MoveKind::BuffThorns, "shared"), (MoveKind::Burrow, "shared"), (MoveKind::ConstrictPlayer,
    "spawned"), (MoveKind::DampenPlayer, "elite"), (MoveKind::EntoSpit, "elite"),
    (MoveKind::Escape, "normal"), (MoveKind::Explode, "normal"), (MoveKind::Fabricate,
    "normal"), (MoveKind::Fade, "boss"), (MoveKind::FrailPlayer, "shared"),
    (MoveKind::HatchToughEgg, "normal"), (MoveKind::Haunt, "normal"), (MoveKind::Headbutt,
    "shared"), (MoveKind::HexPlayer, "elite"), (MoveKind::HopperEscape, "normal"),
    (MoveKind::HopperFlutter, "normal"), (MoveKind::HopperThievery, "normal"),
    (MoveKind::InsatiableLiquify, "boss"), (MoveKind::KdCurse, "boss"), (MoveKind::LayToughEggs,
    "normal"), (MoveKind::LouseCurlGrow, "normal"), (MoveKind::LousePounce, "normal"),
    (MoveKind::LouseWeb, "normal"), (MoveKind::NoiseStatus, "normal"), (MoveKind::None,
    "shared"), (MoveKind::PileInject, "normal"), (MoveKind::Ponder, "boss"),
    (MoveKind::PossessSpeed, "normal"), (MoveKind::PossessStrength, "normal"),
    (MoveKind::QueenBurnBright, "boss"), (MoveKind::QueenPuppetStrings, "boss"),
    (MoveKind::QueenYoureMine, "boss"), (MoveKind::Ritual, "shared"), (MoveKind::Scream,
    "boss"), (MoveKind::Shrink, "shared"), (MoveKind::Soar, "normal"), (MoveKind::SoulSiphon,
    "boss"), (MoveKind::SpitAttack, "shared"), (MoveKind::SummonIllusion, "normal"),
    (MoveKind::SummonRat, "normal"), (MoveKind::TenderGoop, "spawned"),
    (MoveKind::TestSubjectBurningGrowl, "boss"), (MoveKind::TestSubjectMultiClaw, "boss"),
    (MoveKind::TestSubjectRespawn, "boss"), (MoveKind::TestSubjectSkullBash, "boss"),
    (MoveKind::Verdict, "normal"), (MoveKind::VulnPlayer, "normal"), (MoveKind::WaterfallAbout,
    "boss"), (MoveKind::WaterfallExplode, "boss"), (MoveKind::WaterfallPressureGun, "boss"),
    (MoveKind::WaterfallPressureUp, "boss"), (MoveKind::WaterfallPressurize, "boss"),
    (MoveKind::WaterfallRam, "boss"), (MoveKind::WaterfallSiphon, "boss"),
    (MoveKind::WaterfallStomp, "boss"), (MoveKind::WeakPlayer, "normal"),
    (MoveKind::WeakPlayerStrength, "normal"), (MoveKind::Wriggle, "shared"),
];

/// Each family's own implemented-kind registry.
///
/// D6: the capability manifest is DERIVED from these, never
/// transcribed. A family file owns its entry; this table only
/// concatenates them.
#[rustfmt::skip]
pub static FAMILIES: [(&str, &[MoveKind]); 5] = [
    ("boss", boss::IMPLEMENTED), ("elite", elite::IMPLEMENTED), ("normal", normal::IMPLEMENTED),
    ("shared", shared::IMPLEMENTED), ("spawned", spawned::IMPLEMENTED),
];

/// Whether some family implements `kind`.
///
/// Boundary-time work (the admission gate and the manifest), never a
/// per-node path: it walks the family registries.
pub fn is_implemented(kind: MoveKind) -> bool {
    FAMILIES.iter().any(|(_, kinds)| kinds.contains(&kind))
}

/// Dispatch one monster-move to its family body.
pub fn apply_move(kind: MoveKind, ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {
    match kind {
        MoveKind::AddStatus => elite::add_status(ctx),
        MoveKind::AddStatusDiscard => shared::add_status_discard(ctx),
        MoveKind::AddStatusHand => normal::add_status_hand(ctx),
        MoveKind::AeonglassIntensity => boss::aeonglass_intensity(ctx),
        MoveKind::Attack => shared::attack(ctx),
        MoveKind::AttackBeckon => boss::attack_beckon(ctx),
        MoveKind::AttackBlock => shared::attack_block(ctx),
        MoveKind::AttackBlockVital => elite::attack_block_vital(ctx),
        MoveKind::AttackCharge => normal::attack_charge(ctx),
        MoveKind::AttackDexterity => normal::attack_dexterity(ctx),
        MoveKind::AttackFrail => shared::attack_frail(ctx),
        MoveKind::AttackPileInject => elite::attack_pile_inject(ctx),
        MoveKind::AttackScrollBranch => normal::attack_scroll_branch(ctx),
        MoveKind::AttackSmoggy => normal::attack_smoggy(ctx),
        MoveKind::AttackSpawnAggro => normal::attack_spawn_aggro(ctx),
        MoveKind::AttackSteal => normal::attack_steal(ctx),
        MoveKind::AttackStrength => shared::attack_strength(ctx),
        MoveKind::AttackStrengthBranch => normal::attack_strength_branch(ctx),
        MoveKind::AttackTangle => normal::attack_tangle(ctx),
        MoveKind::AttackVigor => elite::attack_vigor(ctx),
        MoveKind::AttackVulnWeakPlayer => spawned::attack_vuln_weak_player(ctx),
        MoveKind::AttackVulnerableDeferredOvicopter => {
            normal::attack_vulnerable_deferred_ovicopter(ctx)
        }
        MoveKind::AttackWeakFrail => shared::attack_weak_frail(ctx),
        MoveKind::AttackWeakPlayer => shared::attack_weak_player(ctx),
        MoveKind::AttackWounds => boss::attack_wounds(ctx),
        MoveKind::AxebotBootup => normal::axebot_bootup(ctx),
        MoveKind::BeastCry => boss::beast_cry(ctx),
        MoveKind::BeastStamp => boss::beast_stamp(ctx),
        MoveKind::Beckon => boss::beckon(ctx),
        MoveKind::Bloat => normal::bloat(ctx),
        MoveKind::Block => shared::block(ctx),
        MoveKind::BlockStrength => shared::block_strength(ctx),
        MoveKind::BuffStrength => shared::buff_strength(ctx),
        MoveKind::BuffTeamBlock => normal::buff_team_block(ctx),
        MoveKind::BuffTeamStrength => spawned::buff_team_strength(ctx),
        MoveKind::BuffThorns => shared::buff_thorns(ctx),
        MoveKind::Burrow => shared::burrow(ctx),
        MoveKind::ConstrictPlayer => spawned::constrict_player(ctx),
        MoveKind::DampenPlayer => elite::dampen_player(ctx),
        MoveKind::EntoSpit => elite::ento_spit(ctx),
        MoveKind::Escape => normal::escape(ctx),
        MoveKind::Explode => normal::explode(ctx),
        MoveKind::Fabricate => normal::fabricate(ctx),
        MoveKind::Fade => boss::fade(ctx),
        MoveKind::FrailPlayer => shared::frail_player(ctx),
        MoveKind::HatchToughEgg => normal::hatch_tough_egg(ctx),
        MoveKind::Haunt => normal::haunt(ctx),
        MoveKind::Headbutt => shared::headbutt(ctx),
        MoveKind::HexPlayer => elite::hex_player(ctx),
        MoveKind::HopperEscape => normal::hopper_escape(ctx),
        MoveKind::HopperFlutter => normal::hopper_flutter(ctx),
        MoveKind::HopperThievery => normal::hopper_thievery(ctx),
        MoveKind::InsatiableLiquify => boss::insatiable_liquify(ctx),
        MoveKind::KdCurse => boss::kd_curse(ctx),
        MoveKind::LayToughEggs => normal::lay_tough_eggs(ctx),
        MoveKind::LouseCurlGrow => normal::louse_curl_grow(ctx),
        MoveKind::LousePounce => normal::louse_pounce(ctx),
        MoveKind::LouseWeb => normal::louse_web(ctx),
        MoveKind::NoiseStatus => normal::noise_status(ctx),
        MoveKind::None => shared::none(ctx),
        MoveKind::PileInject => normal::pile_inject(ctx),
        MoveKind::Ponder => boss::ponder(ctx),
        MoveKind::PossessSpeed => normal::possess_speed(ctx),
        MoveKind::PossessStrength => normal::possess_strength(ctx),
        MoveKind::QueenBurnBright => boss::queen_burn_bright(ctx),
        MoveKind::QueenPuppetStrings => boss::queen_puppet_strings(ctx),
        MoveKind::QueenYoureMine => boss::queen_youre_mine(ctx),
        MoveKind::Ritual => shared::ritual(ctx),
        MoveKind::Scream => boss::scream(ctx),
        MoveKind::Shrink => shared::shrink(ctx),
        MoveKind::Soar => normal::soar(ctx),
        MoveKind::SoulSiphon => boss::soul_siphon(ctx),
        MoveKind::SpitAttack => shared::spit_attack(ctx),
        MoveKind::SummonIllusion => normal::summon_illusion(ctx),
        MoveKind::SummonRat => normal::summon_rat(ctx),
        MoveKind::TenderGoop => spawned::tender_goop(ctx),
        MoveKind::TestSubjectBurningGrowl => boss::test_subject_burning_growl(ctx),
        MoveKind::TestSubjectMultiClaw => boss::test_subject_multi_claw(ctx),
        MoveKind::TestSubjectRespawn => boss::test_subject_respawn(ctx),
        MoveKind::TestSubjectSkullBash => boss::test_subject_skull_bash(ctx),
        MoveKind::Verdict => normal::verdict(ctx),
        MoveKind::VulnPlayer => normal::vuln_player(ctx),
        MoveKind::WaterfallAbout => boss::waterfall_about(ctx),
        MoveKind::WaterfallExplode => boss::waterfall_explode(ctx),
        MoveKind::WaterfallPressureGun => boss::waterfall_pressure_gun(ctx),
        MoveKind::WaterfallPressureUp => boss::waterfall_pressure_up(ctx),
        MoveKind::WaterfallPressurize => boss::waterfall_pressurize(ctx),
        MoveKind::WaterfallRam => boss::waterfall_ram(ctx),
        MoveKind::WaterfallSiphon => boss::waterfall_siphon(ctx),
        MoveKind::WaterfallStomp => boss::waterfall_stomp(ctx),
        MoveKind::WeakPlayer => normal::weak_player(ctx),
        MoveKind::WeakPlayerStrength => normal::weak_player_strength(ctx),
        MoveKind::Wriggle => shared::wriggle(ctx),
    }
}

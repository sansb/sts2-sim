//! The pre-cutover authority boundary for Rust exact solving (#2445).
//!
//! This module is intentionally **not** wired into `diff-serve`, the review
//! worker, or a product command.  It answers one narrower question: may a
//! canonical v0.111.0 state enter the future Rust exact-solve authority?  The
//! answer is yes only for a structurally solo state.  Parsing, provenance,
//! and review rendering remain Python-owned until a later, explicit cutover.
//!
//! Presence is the rule rather than value interpretation.  Canonical v2
//! elides every default-valued field, so a multiplayer/teammate field present
//! in the player bag is evidence of non-solo state.  Treating a forged
//! `false`, empty, or malformed value as harmless would turn a schema drift
//! into a guessed solve; it therefore refuses too.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::canonical::{CanonicalStateV2, STATE_SCHEMA_V2};

/// Machine-readable policy name recorded by the checked-in solve corpus.
pub const SOLO_V1_POLICY: &str = "sts-sim-solo-v1";

/// The explicit result of the solo-v1 gate.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoloV1Admission {
    /// The state is structurally solo and may enter a future Rust exact solve.
    Admitted,
}

/// Why a canonical state cannot enter the solo-v1 exact-solve authority.
///
/// These variants deliberately name the projected field rather than sharing
/// a vague "multiplayer" fallback.  They are an API boundary: callers can
/// display the stable code while retaining the full canonical document for
/// investigation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoloV1Refusal {
    /// The state is not the canonical v2 document this policy understands.
    UnsupportedSchema,
    /// The state does not name the exact game build this policy covers.
    UnsupportedGameBuild,
    /// A canonical document holding a prior refusal is not a state to solve.
    DocumentCarriesRefusal,
    /// A remote teammate roster is present.
    MultiplayerAllies,
    /// The local/remote player-order relation is present.
    MultiplayerPlayerOrder,
    /// The explicit teammate-presence flag is present.
    TeammatePresent,
    /// A teammate Power-card callback is pending.
    TeammatePowerCardPending,
    /// A teammate damage callback is pending.
    TeammateDamagePending,
    /// A teammate damage callback names an owner.
    TeammateDamageOwnerKey,
}

impl SoloV1Refusal {
    /// Stable, wire-safe refusal identity.
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnsupportedSchema => "unsupported_schema",
            Self::UnsupportedGameBuild => "unsupported_game_build",
            Self::DocumentCarriesRefusal => "document_carries_refusal",
            Self::MultiplayerAllies => "multiplayer_allies",
            Self::MultiplayerPlayerOrder => "multiplayer_player_order",
            Self::TeammatePresent => "teammate_present",
            Self::TeammatePowerCardPending => "teammate_power_card_pending",
            Self::TeammateDamagePending => "teammate_damage_pending",
            Self::TeammateDamageOwnerKey => "teammate_damage_owner_key",
        }
    }
}

impl fmt::Display for SoloV1Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for SoloV1Refusal {}

/// Admit only a canonical, structurally solo state to the future exact solver.
///
/// This performs no boundary conversion and no engine admission walk.  Those
/// remain separate proofs; the point here is that a state which *does* pass
/// them cannot silently acquire teammate semantics when exact search becomes
/// callable.
pub fn admit_exact_solve(state: &CanonicalStateV2) -> Result<SoloV1Admission, SoloV1Refusal> {
    if state.schema != STATE_SCHEMA_V2 {
        return Err(SoloV1Refusal::UnsupportedSchema);
    }
    if state.game_build.as_deref() != Some("v0.111.0") {
        return Err(SoloV1Refusal::UnsupportedGameBuild);
    }
    if state.refusal.is_some() {
        return Err(SoloV1Refusal::DocumentCarriesRefusal);
    }

    // Canonical zero-default elision makes any presence non-default evidence.
    // Refuse on presence, not truthiness, to fail closed on malformed or
    // future projection shapes.
    for (field, refusal) in [
        ("multiplayer_allies", SoloV1Refusal::MultiplayerAllies),
        (
            "multiplayer_player_order",
            SoloV1Refusal::MultiplayerPlayerOrder,
        ),
        ("teammate_present", SoloV1Refusal::TeammatePresent),
        (
            "teammate_power_card_pending",
            SoloV1Refusal::TeammatePowerCardPending,
        ),
        (
            "teammate_damage_pending",
            SoloV1Refusal::TeammateDamagePending,
        ),
        (
            "teammate_damage_owner_key",
            SoloV1Refusal::TeammateDamageOwnerKey,
        ),
    ] {
        if state.player.contains_key(field) {
            return Err(refusal);
        }
    }

    Ok(SoloV1Admission::Admitted)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn solo() -> CanonicalStateV2 {
        CanonicalStateV2 {
            schema: STATE_SCHEMA_V2.to_owned(),
            game_build: Some("v0.111.0".to_owned()),
            player: Default::default(),
            monsters: vec![],
            piles: Default::default(),
            rng: Default::default(),
            continuations: vec![],
            refusal: None,
        }
    }

    #[test]
    fn default_canonical_state_is_a_solo_v1_candidate() {
        assert_eq!(admit_exact_solve(&solo()), Ok(SoloV1Admission::Admitted));
    }

    #[test]
    fn every_multiplayer_projection_field_refuses_on_presence() {
        for (field, value, expected) in [
            (
                "multiplayer_allies",
                json!([]),
                SoloV1Refusal::MultiplayerAllies,
            ),
            (
                "multiplayer_player_order",
                json!([]),
                SoloV1Refusal::MultiplayerPlayerOrder,
            ),
            (
                "teammate_present",
                json!(false),
                SoloV1Refusal::TeammatePresent,
            ),
            (
                "teammate_power_card_pending",
                json!(null),
                SoloV1Refusal::TeammatePowerCardPending,
            ),
            (
                "teammate_damage_pending",
                json!(null),
                SoloV1Refusal::TeammateDamagePending,
            ),
            (
                "teammate_damage_owner_key",
                json!(-1),
                SoloV1Refusal::TeammateDamageOwnerKey,
            ),
        ] {
            let mut state = solo();
            state.player.insert(field.to_owned(), value);
            assert_eq!(admit_exact_solve(&state), Err(expected), "{field}");
        }
    }

    #[test]
    fn schema_build_and_prior_refusal_are_not_solve_inputs() {
        let mut unsupported = solo();
        unsupported.schema = "other".to_owned();
        assert_eq!(
            admit_exact_solve(&unsupported),
            Err(SoloV1Refusal::UnsupportedSchema)
        );

        let mut wrong_build = solo();
        wrong_build.game_build = Some("v0.110.1".to_owned());
        assert_eq!(
            admit_exact_solve(&wrong_build),
            Err(SoloV1Refusal::UnsupportedGameBuild)
        );

        let mut missing_build = solo();
        missing_build.game_build = None;
        assert_eq!(
            admit_exact_solve(&missing_build),
            Err(SoloV1Refusal::UnsupportedGameBuild)
        );

        let mut refused = solo();
        refused.refusal = Some(crate::canonical::Refusal::new(
            "load",
            crate::canonical::RefusalKind::NotAdmitted,
            "fixture",
        ));
        assert_eq!(
            admit_exact_solve(&refused),
            Err(SoloV1Refusal::DocumentCarriesRefusal)
        );
    }
}

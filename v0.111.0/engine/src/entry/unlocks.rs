//! What the save proves about the run's unlock profile.
//!
//! Oracle: `live_coach.infernal_blade_pool_fully_unlocked`,
//! `live_coach.card_pool_unlocked_epochs` and
//! `live_coach.potion_pool_fully_unlocked`.
//!
//! All three answer a three-valued question — proven true, proven false, or
//! *unprovable from this save* (`None`) — and the third value is load bearing.
//! A generator whose pool depends on the unlock profile refuses on `None`
//! rather than assuming the full pool, which is the `epoch_provenance` refusal
//! class the corpus census counts.
//!
//! Native authority: `Unlocks.SerializableUnlockState` (`UnlockedEpochs`,
//! `EncountersSeen`, `NumberOfRuns`) in v0.111.0 `sts2.dll` sha256
//! `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`;
//! `UnlockState.ToSerializable` / `FromSerializable` is the round trip. The
//! corpus carries `unlock_state.unlocked_epochs` on 3,092 of 3,092 schema-20
//! saves.

use crate::entry::save::SerializedPlayer;

/// The three Ironclad epochs that together prove the whole Infernal
/// Blade / Stoke Ironclad generation pool.
const INFERNAL_BLADE_IRONCLAD_EPOCHS: [&str; 3] =
    ["IRONCLAD2_EPOCH", "IRONCLAD5_EPOCH", "IRONCLAD7_EPOCH"];

/// The three epochs that together prove #678's complete solo-Ironclad potion
/// pool: `IroncladPotionPool::GetUnlockedPotions` `0xadd38`'s one
/// `Ironclad4Epoch` test and `SharedPotionPool::GetUnlockedPotions`
/// `0xadfb4`'s two. They decide the Ironclad owner's generation pool and no
/// other owner's (`engine::potions::generation_potion_profile`, #3343).
pub(crate) const FULLY_UNLOCKED_POTION_POOL_FLAG_EPOCHS: [&str; 3] =
    ["IRONCLAD4_EPOCH", "POTION1_EPOCH", "POTION2_EPOCH"];

fn raw_epochs(player: &SerializedPlayer) -> Option<&[String]> {
    player.unlock_state.as_ref()?.unlocked_epochs.as_deref()
}

/// The save's normalized exact card-pool `UnlockState` proof, or `None` when
/// the save does not carry one.
///
/// Normalization is the oracle's: strip the `EPOCH.` prefix, deduplicate,
/// sort. An empty-string epoch makes the whole proof unprovable rather than
/// contributing an empty name.
///
/// One deliberate difference from the oracle, recorded rather than hidden: the
/// oracle also answers `None` when an epoch entry is not a string, whereas the
/// typed parser refuses such a save outright as an untaught value shape. No
/// save in the 3,092-save corpus carries a non-string epoch.
pub fn card_pool_unlocked_epochs(player: &SerializedPlayer) -> Option<Vec<String>> {
    let epochs = raw_epochs(player)?;
    if epochs.iter().any(String::is_empty) {
        return None;
    }
    let mut names: Vec<String> = epochs
        .iter()
        .map(|epoch| epoch.strip_prefix("EPOCH.").unwrap_or(epoch).to_string())
        .collect();
    names.sort();
    names.dedup();
    Some(names)
}

/// Whether the save proves the full Infernal Blade / Stoke Ironclad pool.
///
/// Note the oracle normalizes with `str(epoch)` here rather than requiring a
/// string, so this predicate can answer on a profile
/// [`card_pool_unlocked_epochs`] calls unprovable. That asymmetry is
/// reproduced, not smoothed over: an empty-string epoch leaves this `Some` and
/// the epoch tuple `None`.
pub fn infernal_blade_pool_fully_unlocked(player: &SerializedPlayer) -> Option<bool> {
    let epochs = raw_epochs(player)?;
    let normalized: Vec<&str> = epochs
        .iter()
        .map(|epoch| epoch.strip_prefix("EPOCH.").unwrap_or(epoch))
        .collect();
    Some(
        INFERNAL_BLADE_IRONCLAD_EPOCHS
            .iter()
            .all(|needed| normalized.contains(needed)),
    )
}

/// Whether the save proves the complete solo-Ironclad potion pool.
///
/// This is an Ironclad fact, not an owner-generic one. Since #3343 the
/// potion generators (Alchemize, Entropic Brew) decide the owner's pool from
/// the recorded profile, and use this flag alone only for an Ironclad owner.
/// Since #3347 the boundary accepts it, or a recorded profile, as a
/// materialised belt's pool proof.
pub fn potion_pool_fully_unlocked(player: &SerializedPlayer) -> Option<bool> {
    let epochs = card_pool_unlocked_epochs(player)?;
    Some(
        FULLY_UNLOCKED_POTION_POOL_FLAG_EPOCHS
            .iter()
            .all(|needed| epochs.iter().any(|have| have == needed)),
    )
}

/// The run's character, normalized to a full `CHARACTER.` model id.
///
/// Oracle: `live_coach.player_character_id`. A save writes the full id; the
/// decoded-MCR form writes the bare name, and both normalize to the same
/// value. A non-string is `None` there and unrepresentable here.
pub fn player_character_id(player: &SerializedPlayer) -> Option<String> {
    let value = player.character_id.as_deref()?;
    Some(if value.starts_with("CHARACTER.") {
        value.to_string()
    } else {
        format!("CHARACTER.{value}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::save::SerializedUnlockState;

    fn player(epochs: Option<Vec<&str>>) -> SerializedPlayer {
        SerializedPlayer {
            unlock_state: epochs.map(|names| SerializedUnlockState {
                unlocked_epochs: Some(names.into_iter().map(str::to_string).collect()),
                ..SerializedUnlockState::default()
            }),
            ..SerializedPlayer::default()
        }
    }

    #[test]
    fn absent_unlock_state_is_unprovable_not_false() {
        let bare = SerializedPlayer::default();
        assert_eq!(card_pool_unlocked_epochs(&bare), None);
        assert_eq!(infernal_blade_pool_fully_unlocked(&bare), None);
        assert_eq!(potion_pool_fully_unlocked(&bare), None);
    }

    #[test]
    fn epochs_normalize_prefix_dedupe_and_sort() {
        let save = player(Some(vec![
            "EPOCH.RELIC1_EPOCH",
            "COLORLESS1_EPOCH",
            "RELIC1_EPOCH",
        ]));
        assert_eq!(
            card_pool_unlocked_epochs(&save).unwrap(),
            vec!["COLORLESS1_EPOCH".to_string(), "RELIC1_EPOCH".to_string()]
        );
    }

    #[test]
    fn the_three_ironclad_epochs_prove_the_infernal_blade_pool() {
        assert_eq!(
            infernal_blade_pool_fully_unlocked(&player(Some(vec![
                "IRONCLAD2_EPOCH",
                "IRONCLAD5_EPOCH",
                "EPOCH.IRONCLAD7_EPOCH",
            ]))),
            Some(true)
        );
        assert_eq!(
            infernal_blade_pool_fully_unlocked(&player(Some(vec![
                "IRONCLAD2_EPOCH",
                "IRONCLAD5_EPOCH",
            ]))),
            Some(false)
        );
    }

    #[test]
    fn the_potion_pool_predicate_follows_the_epoch_tuple() {
        assert_eq!(
            potion_pool_fully_unlocked(&player(Some(vec![
                "IRONCLAD4_EPOCH",
                "POTION1_EPOCH",
                "POTION2_EPOCH",
            ]))),
            Some(true)
        );
        assert_eq!(
            potion_pool_fully_unlocked(&player(Some(vec!["POTION1_EPOCH"]))),
            Some(false)
        );
    }

    #[test]
    fn an_empty_epoch_name_splits_the_two_predicates_exactly_as_the_oracle_does() {
        let save = player(Some(vec!["", "IRONCLAD2_EPOCH"]));
        assert_eq!(card_pool_unlocked_epochs(&save), None);
        assert_eq!(potion_pool_fully_unlocked(&save), None);
        assert_eq!(infernal_blade_pool_fully_unlocked(&save), Some(false));
    }

    #[test]
    fn character_ids_normalize_to_the_full_model_id() {
        let mut save = SerializedPlayer {
            character_id: Some("CHARACTER.DEFECT".to_string()),
            ..SerializedPlayer::default()
        };
        assert_eq!(
            player_character_id(&save).as_deref(),
            Some("CHARACTER.DEFECT")
        );
        save.character_id = Some("IRONCLAD".to_string());
        assert_eq!(
            player_character_id(&save).as_deref(),
            Some("CHARACTER.IRONCLAD")
        );
        save.character_id = None;
        assert_eq!(player_character_id(&save), None);
    }
}

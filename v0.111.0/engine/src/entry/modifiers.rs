//! Run modifiers at a combat root (#3760).
//!
//! A daily or custom run carries `ModifierModel`s, and every one is a hook
//! listener inside every combat: `CombatRoom::.ctor` (v0.111.0 RVA `0x58960`,
//! IL_0046) hands `IRunState.Modifiers` to the combat state, and
//! `CombatState/<IterateHookListeners>d__69::MoveNext` (RVA `0x3f9720`)
//! appends each of them to the listener list at IL_0219-0225, after the
//! creatures' powers, relics, potions, orbs and cards.
//! `ModifierModel::get_ShouldReceiveCombatHooks` (RVA `0x81ae7`) is
//! `ldc.i4.1` and no subclass overrides it.
//!
//! An entry that ignored the save's `modifiers` rooted the fight as if they
//! were absent: Murderous's 3 Strength on every creature was a silent
//! disagreement with the game on every fight of a daily run (#3756).
//! [`RUN_MODIFIERS`] is the gate's table: every `ModifierModel` subclass in
//! the assembly, the methods it overrides, and whether any of them can run
//! between room entry and the end of a combat. One that can is either
//! modeled (Murderous, #3934: the opening and the engine run its two
//! bodies) or refuses by name; one that cannot is admitted, since what it
//! did before the fight is already in the save (the deck Sealed Deck or
//! Draft built, the map Big Game Hunter generated).
//!
//! # How the table was read (v0.111.0 DLL `9cb4f1ad`)
//!
//! * **Membership** is the TypeDef `Extends` column: the 17 types whose base
//!   is `MegaCrit.Sts2.Core.Models.ModifierModel`, all in
//!   `MegaCrit.Sts2.Core.Models.Modifiers`. None is extended further.
//! * **Overrides** are each type's own MethodDef rows (RVAs below).
//! * **Nothing reaches a modifier except through a hook.** A scan of every
//!   method body in the assembly for a token naming a `Models.Modifiers`
//!   TypeDef, method or field finds, outside the modifiers themselves, only
//!   the model registry (`AbstractModelSubtypes::.cctor`), the custom-run
//!   screen, and `ModifierModel::Pick2Good1Bad`'s `CharacterCards` setup.
//! * **Which hooks run in a combat** is read from each hook's callers:
//!   - `AfterRoomEntered`: `CombatRoom/<StartCombat>d__46::MoveNext` (RVA
//!     `0x310bf0`, IL_01aa). In combat.
//!   - `AfterCreatureAddedToCombat`: `CreatureCmd/<Add>d__2::MoveNext` (RVA
//!     `0x3e9234`, IL_01f0). In combat.
//!   - `AfterCardChangedPiles`: `CardPileCmd/<Add>d__10` (RVA `0x3e1ba4`,
//!     IL_0887), `<AddDuringManualCardPlay>d__13` (`0x3e2b44`, IL_0220),
//!     `<RemoveFromCombat>d__3` (`0x3e42a4`, IL_03b4) and
//!     `CardCmd/<Transform>d__13` (`0x3e0ae0`, IL_04a3). In combat.
//!   - `ModifyCardRewardCreationOptions`: `Hook` RVA `0x104f6c`, called only
//!     by `CardFactory::CreateForReward` (`0x112b10`, IL_001a) and
//!     `ScrollBoxes::GenerateRandomBundles` (`0x9adbc`). `CreateForReward`'s
//!     callers are `CardReward::Populate`, relic `AfterObtained` bodies,
//!     event options, Lasting Candy's reward hook and the Neow-option
//!     modifiers: card REWARDS, never a card created in a fight.
//!   - `TryModifyRewardsLate`: `Hook::ModifyRewards` (RVA `0x105e18`,
//!     IL_006a), called only by `RewardsSet/<GenerateWithoutOffering>d__32`
//!     (`0x313550`, IL_0092). It edits a reward list; the combat state holds
//!     no reward list.
//!   - `AfterActEntered`: `RunManager/<EnterAct>d__208` (`0x30b624`,
//!     IL_0332). `ModifyGeneratedMap`: `RunManager/<GenerateMap>d__188`
//!     (`0x30d3dc`, IL_0169). `ModifyMerchantCardPool`:
//!     `CardFactory::CreateForMerchant`. The rest-site, free-travel,
//!     merchant-removal, room-odds, run-created/loaded and Neow-option
//!     methods are named for the only screen that calls them.
//!
//! The event-started combats keep their own, wider rule
//! ([`crate::entry::event_combat`]): any modifier refuses there, because the
//! event's room entry and option chain fire hooks this table does not read.

use crate::entry::refusal::EntryRefusal;

/// Whether a modifier can act between room entry and the end of a combat.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CombatVerdict {
    /// No method the type overrides runs inside a combat.
    Inert,
    /// It overrides `hook`, which fires inside a combat room, and the opening
    /// and the engine run its bodies.
    Modeled { hook: &'static str },
    /// It overrides `hook`, which fires inside a combat room, and the engine
    /// does not model what its body does there (`effect`).
    Refused {
        hook: &'static str,
        effect: &'static str,
    },
}

/// One `ModifierModel` subclass.
#[derive(Clone, Copy, Debug)]
pub struct RunModifier {
    /// The model id's entry, as the save's `modifiers[].id` carries it after
    /// the `MODIFIER.` category.
    pub entry: &'static str,
    /// The native type, in `MegaCrit.Sts2.Core.Models.Modifiers`.
    pub native: &'static str,
    /// Every method the type overrides (its own MethodDef rows, constructor
    /// and private helpers left out).
    pub overrides: &'static [&'static str],
    pub verdict: CombatVerdict,
}

const fn inert(
    entry: &'static str,
    native: &'static str,
    overrides: &'static [&'static str],
) -> RunModifier {
    RunModifier {
        entry,
        native,
        overrides,
        verdict: CombatVerdict::Inert,
    }
}

/// Every run modifier of v0.111.0, in the assembly's TypeDef order.
pub const RUN_MODIFIERS: &[RunModifier] = &[
    // `GenerateNeowOption` RVA `0xc5f44`; its option's `ObtainCards` runs at
    // Neow.
    inert("ALL_STAR", "AllStar", &["GenerateNeowOption"]),
    // `ModifyGeneratedMap` `0xc5fb8`, `ModifyCardRewardCreationOptions`
    // `0xc60c0`.
    inert(
        "BIG_GAME_HUNTER",
        "BigGameHunter",
        &["ModifyGeneratedMap", "ModifyCardRewardCreationOptions"],
    ),
    // `ModifyMerchantCardPool` `0xc61d8`, `ModifyCardRewardCreationOptions`
    // `0xc623c`; `get_Title`/`get_Description`/`IsEquivalent` are display and
    // identity.
    inert(
        "CHARACTER_CARDS",
        "CharacterCards",
        &["ModifyMerchantCardPool", "ModifyCardRewardCreationOptions"],
    ),
    // `AfterActEntered` `0xc62cc`.
    inert("CURSED_RUN", "CursedRun", &["AfterActEntered"]),
    // `AfterRunCreated` `0xc6318`, `AfterRunLoaded` `0xc63a0`,
    // `ModifyOddsIncreaseForUnrolledRoomType` `0xc63b8`.
    inert(
        "DEADLY_EVENTS",
        "DeadlyEvents",
        &[
            "AfterRunCreated",
            "AfterRunLoaded",
            "ModifyOddsIncreaseForUnrolledRoomType",
        ],
    ),
    // Only a constructor (`0xc63cf`).
    inert("DEPRECATED_MODIFIER", "DeprecatedModifier", &[]),
    // `get_ClearsPlayerDeck` `0xc63d7`, `GenerateNeowOption` `0xc63dc`.
    inert(
        "DRAFT",
        "Draft",
        &["get_ClearsPlayerDeck", "GenerateNeowOption"],
    ),
    // `ShouldAllowFreeTravel` `0xc644f`.
    inert("FLIGHT", "Flight", &["ShouldAllowFreeTravel"]),
    // `Hoarder/<AfterCardChangedPiles>d__1::MoveNext` RVA `0x3777f4`: returns
    // unless the card is NEW (`oldPileType` is `PileType.None`, IL_0021-0026)
    // and now sits in the `PileType.Deck` pile (`get_Type` against
    // `ldc.i4.6`, IL_003a-0045), was not cloned (IL_004d-0052) and is not one
    // of its own copies (`_cardsToSkip.Remove`, IL_005a-006a); then it adds
    // two clones to the Deck (`ICardScope::CloneCard` IL_0093,
    // `CardPileCmd::Add` with `ldc.i4.6` IL_00a7-00ab, loop bound
    // `ldc.i4.2` IL_0131). The hook fires on every pile change in a fight,
    // and the body acts whenever a fight creates a card into the master
    // deck. `ShouldAllowMerchantCardRemoval` `0xc64b7` is the shop's.
    RunModifier {
        entry: "HOARDER",
        native: "Hoarder",
        overrides: &["AfterCardChangedPiles", "ShouldAllowMerchantCardRemoval"],
        verdict: CombatVerdict::Refused {
            hook: "AfterCardChangedPiles",
            effect: "adds two more copies of every card a fight creates into the master deck",
        },
    },
    // `get_ClearsPlayerDeck` `0xc64cd`, `GenerateNeowOption` `0xc64d0`.
    inert(
        "INSANITY",
        "Insanity",
        &["get_ClearsPlayerDeck", "GenerateNeowOption"],
    ),
    // `TryModifyRewardsLate` `0xc6544` (doubles each GoldReward),
    // `TryModifyRestSiteOptions` `0xc65c8`.
    inert(
        "MIDAS",
        "Midas",
        &["TryModifyRewardsLate", "TryModifyRestSiteOptions"],
    ),
    // `Murderous/<AfterRoomEntered>d__1::MoveNext` RVA `0x377cf8`: when the
    // room is a `CombatRoom` (`isinst` IL_001c), `PowerCmd.Apply
    // <StrengthPower>` of `ldc.i4.3` to `CombatState.Creatures`
    // (IL_0030-0043): the player, every pet and every monster.
    // `Murderous::AfterCreatureAddedToCombat` RVA `0xc666f`: a creature added
    // later gets the same 3 Strength (IL_0010-001f) unless its `Side` is
    // `CombatSide.Player` (`ldc.i4.1`, IL_0002-0008). Modeled (#3934): the
    // room-entry body is `entry::opening`'s `murderous_after_room_entered`
    // and the later one is a row of
    // `engine::monsters::fur_coat_after_opponent_added`. No pet takes the
    // Strength: `_allies` holds only the players' creatures when the hook
    // fires (see the opening's function), and a pet added later is on the
    // player's side.
    RunModifier {
        entry: MURDEROUS,
        native: "Murderous",
        overrides: &["AfterRoomEntered", "AfterCreatureAddedToCombat"],
        verdict: CombatVerdict::Modeled {
            hook: "AfterRoomEntered",
        },
    },
    // `ModifyRestSiteHealAmount` `0xc669c`, `AfterRestSiteHeal` `0xc66ac`,
    // `ModifyExtraRestSiteHealText` `0xc66f8`.
    inert(
        "NIGHT_TERRORS",
        "NightTerrors",
        &[
            "ModifyRestSiteHealAmount",
            "AfterRestSiteHeal",
            "ModifyExtraRestSiteHealText",
        ],
    ),
    // `get_ClearsPlayerDeck` `0xc67a0`, `GenerateNeowOption` `0xc67a4`.
    inert(
        "SEALED_DECK",
        "SealedDeck",
        &["get_ClearsPlayerDeck", "GenerateNeowOption"],
    ),
    // `GenerateNeowOption` `0xc6870`.
    inert("SPECIALIZED", "Specialized", &["GenerateNeowOption"]),
    // `Terminal/<AfterRoomEntered>d__0::MoveNext` RVA `0x378398`: when the
    // room is the run's base room (IL_0025-0035), every player whose
    // creature can still pay it loses 1 Max HP (`CreatureCmd::LoseMaxHp` of
    // `Decimal::One`, IL_0085-0096); then, when the room is a `CombatRoom`
    // (`isinst` IL_012a), every player creature gets `PowerCmd.Apply
    // <PlatingPower>` of `ldc.i4.5` (IL_0160-0170).
    RunModifier {
        entry: "TERMINAL",
        native: "Terminal",
        overrides: &["AfterRoomEntered"],
        verdict: CombatVerdict::Refused {
            hook: "AfterRoomEntered",
            effect: "takes 1 Max HP from the player and applies 5 Plating at room entry",
        },
    },
    // `TryModifyRewardsLate` `0xc6938` (a combat room's card reward becomes
    // a relic reward).
    inert("VINTAGE", "Vintage", &["TryModifyRewardsLate"]),
];

/// Murderous's model id entry.
const MURDEROUS: &str = "MURDEROUS";

/// Whether the run carries Murderous, in either spelling a root reads (a
/// save's `MODIFIER.MURDEROUS`, a capture run's bare entry). The opening
/// reads this to seed `player.murderous`.
pub fn murderous(modifiers: &[String]) -> bool {
    modifiers
        .iter()
        .any(|id| id.strip_prefix(CATEGORY).unwrap_or(id) == MURDEROUS)
}

/// The save's modifier category prefix; a capture's run carries the bare
/// entry and `entry::capture` adds it.
const CATEGORY: &str = "MODIFIER.";

/// The table row for a save's modifier id, with or without its category.
pub fn run_modifier(id: &str) -> Option<&'static RunModifier> {
    let entry = id.strip_prefix(CATEGORY).unwrap_or(id);
    RUN_MODIFIERS.iter().find(|row| row.entry == entry)
}

/// Refuse a combat root whose run carries a modifier that can act in the
/// fight, or one this table has never read.
pub fn combat_gate(modifiers: &[String]) -> Result<(), EntryRefusal> {
    for id in modifiers {
        match run_modifier(id).map(|row| row.verdict) {
            Some(CombatVerdict::Inert | CombatVerdict::Modeled { .. }) => {}
            Some(CombatVerdict::Refused { hook, effect }) => {
                return Err(EntryRefusal::RunModifierNotModeled {
                    modifier: id.clone(),
                    hook,
                    effect,
                });
            }
            None => {
                return Err(EntryRefusal::RunModifierUnknown {
                    modifier: id.clone(),
                });
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|entry| (*entry).to_string()).collect()
    }

    fn class(result: Result<(), EntryRefusal>) -> &'static str {
        result.expect_err("refuses").class()
    }

    #[test]
    fn the_table_is_the_seventeen_modifier_types_each_once() {
        assert_eq!(RUN_MODIFIERS.len(), 17);
        let mut entries: Vec<&str> = RUN_MODIFIERS.iter().map(|row| row.entry).collect();
        // TypeDef order is alphabetical by type name, which is the entries'.
        assert!(entries.is_sorted());
        entries.dedup();
        assert_eq!(entries.len(), 17);
        // Each entry is its type name in the model id's SCREAMING_SNAKE form.
        for row in RUN_MODIFIERS {
            let mut slug = String::new();
            for (index, letter) in row.native.chars().enumerate() {
                if letter.is_ascii_uppercase() && index != 0 {
                    slug.push('_');
                }
                slug.push(letter.to_ascii_uppercase());
            }
            assert_eq!(slug, row.entry);
        }
    }

    /// The hooks this module's header reads as firing inside a combat. A
    /// modifier overriding one is modeled or refuses, and one overriding
    /// none is inert: the verdict column cannot drift from the override
    /// column.
    #[test]
    fn a_modifier_is_inert_exactly_when_it_overrides_no_combat_hook() {
        const COMBAT_HOOKS: [&str; 3] = [
            "AfterRoomEntered",
            "AfterCreatureAddedToCombat",
            "AfterCardChangedPiles",
        ];
        for row in RUN_MODIFIERS {
            let in_combat = row.overrides.iter().any(|name| COMBAT_HOOKS.contains(name));
            match row.verdict {
                CombatVerdict::Inert => assert!(!in_combat, "{}", row.native),
                CombatVerdict::Modeled { hook } => {
                    assert!(in_combat, "{}", row.native);
                    assert!(row.overrides.contains(&hook), "{}", row.native);
                    assert!(COMBAT_HOOKS.contains(&hook), "{}", row.native);
                }
                CombatVerdict::Refused { hook, effect } => {
                    assert!(in_combat, "{}", row.native);
                    assert!(row.overrides.contains(&hook), "{}", row.native);
                    assert!(COMBAT_HOOKS.contains(&hook), "{}", row.native);
                    assert!(!effect.is_empty());
                }
            }
        }
        let refused: Vec<&str> = RUN_MODIFIERS
            .iter()
            .filter(|row| matches!(row.verdict, CombatVerdict::Refused { .. }))
            .map(|row| row.entry)
            .collect();
        assert_eq!(refused, ["HOARDER", "TERMINAL"]);
        let modeled: Vec<&str> = RUN_MODIFIERS
            .iter()
            .filter(|row| matches!(row.verdict, CombatVerdict::Modeled { .. }))
            .map(|row| row.entry)
            .collect();
        assert_eq!(modeled, ["MURDEROUS"]);
    }

    /// Murderous is admitted in both spellings, and `murderous` reads it in
    /// both and beside other modifiers (#3934).
    #[test]
    fn murderous_is_admitted_and_read_in_both_spellings() {
        for id in ["MURDEROUS", "MODIFIER.MURDEROUS"] {
            let run = ids(&["FLIGHT", "INSANITY", id]);
            assert_eq!(combat_gate(&run), Ok(()), "{id}");
            assert!(murderous(&run), "{id}");
        }
        assert!(!murderous(&[]));
        assert!(!murderous(&ids(&["FLIGHT", "INSANITY"])));
        // A different modifier whose id merely contains the word.
        assert!(!murderous(&ids(&["MODIFIER.NOT_MURDEROUS"])));
    }

    #[test]
    fn each_modifier_with_a_combat_hook_refuses_by_name() {
        for (entry, hook) in [
            ("TERMINAL", "AfterRoomEntered"),
            ("HOARDER", "AfterCardChangedPiles"),
        ] {
            // Both spellings: a game save's id and a capture run's bare entry.
            for id in [entry.to_string(), format!("MODIFIER.{entry}")] {
                let refusal = combat_gate(std::slice::from_ref(&id)).expect_err("refuses");
                assert_eq!(refusal.class(), "run_modifier_not_modeled", "{id}");
                let detail = refusal.to_string();
                assert!(detail.contains(&id), "{detail}");
                assert!(detail.contains(hook), "{detail}");
            }
        }
    }

    #[test]
    fn every_inert_modifier_is_admitted_alone_and_together() {
        let inert: Vec<String> = RUN_MODIFIERS
            .iter()
            .filter(|row| row.verdict == CombatVerdict::Inert)
            .map(|row| format!("MODIFIER.{}", row.entry))
            .collect();
        assert_eq!(inert.len(), 14);
        assert_eq!(RUN_MODIFIERS.len() - inert.len(), 3);
        for id in &inert {
            assert_eq!(combat_gate(std::slice::from_ref(id)), Ok(()), "{id}");
        }
        assert_eq!(combat_gate(&inert), Ok(()));
        assert_eq!(combat_gate(&[]), Ok(()));
        // The three daily runs of #3756, all admitted: two carry only inert
        // modifiers and the third's Murderous is modeled (#3934).
        assert_eq!(
            combat_gate(&ids(&["ALL_STAR", "SEALED_DECK", "BIG_GAME_HUNTER"])),
            Ok(())
        );
        assert_eq!(
            combat_gate(&ids(&["BIG_GAME_HUNTER", "CHARACTER_CARDS", "DRAFT"])),
            Ok(())
        );
        assert_eq!(
            combat_gate(&ids(&["FLIGHT", "INSANITY", "MURDEROUS"])),
            Ok(())
        );
        // A refused modifier still names the refusal beside a modeled one.
        let refusal = combat_gate(&ids(&["MURDEROUS", "TERMINAL"])).expect_err("refuses");
        assert!(refusal.to_string().contains("TERMINAL"));
    }

    #[test]
    fn a_modifier_the_table_has_not_read_refuses() {
        assert_eq!(
            class(combat_gate(&ids(&["MODIFIER.NOT_A_MODIFIER"]))),
            "run_modifier_unknown"
        );
        // A save row without an id, as `build_document` spells it.
        assert_eq!(
            class(combat_gate(&ids(&["<no id>"]))),
            "run_modifier_unknown"
        );
        // Refused beside inert ones, and ahead of nothing: the first
        // offending modifier in the save's order names the refusal.
        let refusal = combat_gate(&ids(&["FLIGHT", "WHO", "TERMINAL"])).expect_err("refuses");
        assert_eq!(refusal.class(), "run_modifier_unknown");
        assert!(refusal.to_string().contains("WHO"));
    }
}

//! Exact hot value for the one solo player-side Osty slot.
//!
//! Native owns one Osty creature. The live pet slot omits its corpse, but
//! Die For You and the retained creature survive death until combat ends. This value deliberately stays private until the
//! complete #1675 boundary, admission, dealer, and listener surfaces land
//! atomically. Keeping the per-turn attack history beside, rather than
//! inside, the live slot preserves history when Osty dies.

/// The one live Osty payload projected from canonical `(OSTY, hp, max_hp)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Osty {
    hp: i32,
    max_hp: i32,
}

impl Osty {
    pub(crate) fn hp(self) -> i32 {
        self.hp
    }

    pub(crate) fn max_hp(self) -> i32 {
        self.max_hp
    }
}

/// The complete solo-pet state that can live behind an existing COW handle.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct SoloPetState {
    osty: Option<Osty>,
    attacks_this_turn: i32,
}

/// Fail-closed validation and arithmetic failures for the private pet value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PetStateError {
    InvalidOsty,
    InvalidAttackCount,
    NegativeAmount,
    CounterOverflow,
    /// A summon reached a dead or absent Osty while the combat is ending
    /// (the unrepresented add/revive branch of [`SoloPetState::summon_ending`]).
    EndingSummonNotLive,
}

impl SoloPetState {
    /// Construct from the canonical slot and history counter.
    pub(crate) fn from_canonical(
        osty: Option<(i32, i32)>,
        attacks_this_turn: i32,
    ) -> Result<Self, PetStateError> {
        if attacks_this_turn < 0 {
            return Err(PetStateError::InvalidAttackCount);
        }
        let osty = match osty {
            None => None,
            Some((hp, max_hp)) if hp > 0 && max_hp > 0 && hp <= max_hp => Some(Osty { hp, max_hp }),
            Some(_) => return Err(PetStateError::InvalidOsty),
        };
        Ok(Self {
            osty,
            attacks_this_turn,
        })
    }

    pub(crate) fn osty(self) -> Option<Osty> {
        self.osty.filter(|osty| osty.hp > 0)
    }

    pub(crate) fn has_die_for_you(self) -> bool {
        self.osty.is_some()
    }

    pub(crate) fn corpse(self) -> bool {
        self.osty.is_some_and(|osty| osty.hp == 0)
    }

    pub(crate) fn set_corpse(&mut self, value: bool) -> Result<(), PetStateError> {
        if value {
            if self.osty().is_some() {
                return Err(PetStateError::InvalidOsty);
            }
            self.osty = Some(Osty { hp: 0, max_hp: 0 });
        } else if self.corpse() {
            self.osty = None;
        }
        Ok(())
    }

    pub(crate) fn attacks_this_turn(self) -> i32 {
        self.attacks_this_turn
    }

    pub(crate) fn set_osty(&mut self, osty: Option<(i32, i32)>) -> Result<(), PetStateError> {
        self.osty = Self::from_canonical(osty, self.attacks_this_turn)?.osty;
        Ok(())
    }

    pub(crate) fn set_attacks_this_turn(
        &mut self,
        attacks_this_turn: i32,
    ) -> Result<(), PetStateError> {
        if attacks_this_turn < 0 {
            return Err(PetStateError::InvalidAttackCount);
        }
        self.attacks_this_turn = attacks_this_turn;
        Ok(())
    }

    pub(crate) fn lose_hp(&mut self, amount: i32) -> Result<i32, PetStateError> {
        if amount < 0 {
            return Err(PetStateError::NegativeAmount);
        }
        let Some(current) = self.osty() else {
            return Ok(0);
        };
        let lost = current.hp.min(amount);
        self.osty = Some(if lost < current.hp {
            Osty {
                hp: current.hp - lost,
                max_hp: current.max_hp,
            }
        } else {
            Osty { hp: 0, max_hp: 0 }
        });
        Ok(lost)
    }

    /// `OstyCmd::Summon` outside the IsEnding window: zero is a pure no-op,
    /// fresh summon sets hp/max, and a living re-summon grows both by the
    /// same amount with no RNG. See [`Self::summon_ending`].
    pub(crate) fn summon(&mut self, amount: i32) -> Result<(), PetStateError> {
        self.summon_ending(amount, false)
    }

    /// `OstyCmd::Summon` with the caller's IsEnding projection.
    ///
    /// # What the DLL does (v0.111.0, sha256 `9cb4f1ad…`)
    ///
    /// `OstyCmd/<Summon>d__0::MoveNext` (RVA `0x3ee040`): a zero amount
    /// returns at `IL_00a4`-`IL_00c1`. A living Osty (`get_IsOstyAlive`,
    /// `IL_0113`) takes `CreatureCmd.GainMaxHp(osty, amount)` (`IL_0130`).
    /// `<GainMaxHp>d__22::MoveNext` (`0x3eb2f0`) raises MaxHp with no gate
    /// (`SetMaxHp`, `IL_005b`) and then calls `Heal(creature, amount)`
    /// (`IL_010b`). `<Heal>d__20::MoveNext` (`0x3eb4b0`) leaves at
    /// `IL_0041`-`IL_005f` when `CombatManager.IsEnding` and the creature is
    /// not a player (`Creature.get_IsPlayer` `0x11d050` is `Player != null`;
    /// Osty is a pet, not a player). So a living re-summon while the combat
    /// is ending grows MaxHp only (#3246, BR2R60965GJ1 n13: 16/23 -> 16/26).
    ///
    /// A dead or absent Osty takes the add/revive branch: `SetMaxHp(osty,
    /// amount)` (`IL_0387`; `Creature.SetMaxHpInternal` `0x11d754` clamps
    /// CurrentHp to the new MaxHp) then the same gated `Heal` (`IL_03f7`),
    /// followed by `AfterOstyRevived` for a revive (`IL_0468`). While ending,
    /// a revived corpse therefore stays dead with a new MaxHp and a fresh Osty
    /// keeps its initial 1 HP (`Osty::get_MinInitialHp` `0xba86b`). Neither
    /// the dead pet's re-added MaxHp nor a fresh pet's Add path while ending is
    /// represented, so that branch refuses by name.
    pub(crate) fn summon_ending(
        &mut self,
        amount: i32,
        combat_ending: bool,
    ) -> Result<(), PetStateError> {
        if amount < 0 {
            return Err(PetStateError::NegativeAmount);
        }
        if amount == 0 {
            return Ok(());
        }
        let next = match self.osty() {
            None if combat_ending => return Err(PetStateError::EndingSummonNotLive),
            None => Osty {
                hp: amount,
                max_hp: amount,
            },
            Some(current) => Osty {
                hp: if combat_ending {
                    current.hp
                } else {
                    current
                        .hp
                        .checked_add(amount)
                        .ok_or(PetStateError::CounterOverflow)?
                },
                max_hp: current
                    .max_hp
                    .checked_add(amount)
                    .ok_or(PetStateError::CounterOverflow)?,
            },
        };
        self.osty = Some(next);
        Ok(())
    }

    /// Heal the current live Osty. A missing pet is the native re-read no-op.
    pub(crate) fn heal(&mut self, amount: i32) -> Result<(), PetStateError> {
        if amount < 0 {
            return Err(PetStateError::NegativeAmount);
        }
        let Some(current) = self.osty() else {
            return Ok(());
        };
        // Native Heal clamps to the already-live MaxHp. Avoid computing the
        // unbounded intermediate: `hp == max_hp` with a large positive heal
        // is an exact no-op, not an overflow refusal.
        let room = current.max_hp - current.hp;
        let hp = current.hp + amount.min(room);
        self.osty = Some(Osty {
            hp,
            max_hp: current.max_hp,
        });
        Ok(())
    }

    /// DieForYouPower vetoes creature and power removal (0xa1861/0xa186f).
    /// Preserve its presence even though the player no longer has a live pet.
    pub(crate) fn kill(&mut self) {
        if self.osty.is_some() {
            self.osty = Some(Osty { hp: 0, max_hp: 0 });
        }
    }

    /// Record one completed pet AttackCommand, independent of hit count.
    pub(crate) fn record_attack(&mut self) -> Result<(), PetStateError> {
        self.attacks_this_turn = self
            .attacks_this_turn
            .checked_add(1)
            .ok_or(PetStateError::CounterOverflow)?;
        Ok(())
    }

    pub(crate) fn reset_attacks(&mut self) {
        self.attacks_this_turn = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_shape_is_singular_live_osty_plus_independent_history() {
        assert_eq!(
            SoloPetState::from_canonical(Some((4, 7)), 3),
            Ok(SoloPetState {
                osty: Some(Osty { hp: 4, max_hp: 7 }),
                attacks_this_turn: 3,
            })
        );
        for invalid in [Some((0, 7)), Some((-1, 7)), Some((8, 7)), Some((1, 0))] {
            assert_eq!(
                SoloPetState::from_canonical(invalid, 0),
                Err(PetStateError::InvalidOsty)
            );
        }
        assert_eq!(
            SoloPetState::from_canonical(None, -1),
            Err(PetStateError::InvalidAttackCount)
        );
    }

    #[test]
    fn summon_and_heal_are_exact_and_refuse_atomically() {
        let mut state = SoloPetState::default();
        state.summon(5).unwrap();
        assert_eq!(state.osty(), Some(Osty { hp: 5, max_hp: 5 }));
        state.summon(3).unwrap();
        assert_eq!(state.osty(), Some(Osty { hp: 8, max_hp: 8 }));
        assert_eq!(state.osty().map(Osty::hp), Some(8));
        assert_eq!(state.osty().map(Osty::max_hp), Some(8));

        state.osty = Some(Osty { hp: 4, max_hp: 8 });
        state.heal(7).unwrap();
        assert_eq!(state.osty(), Some(Osty { hp: 8, max_hp: 8 }));
        state.heal(i32::MAX).unwrap();
        assert_eq!(state.osty(), Some(Osty { hp: 8, max_hp: 8 }));

        for operation in [
            SoloPetState::summon as fn(&mut SoloPetState, i32) -> _,
            SoloPetState::heal,
        ] {
            let before = state;
            assert_eq!(
                operation(&mut state, -1),
                Err(PetStateError::NegativeAmount)
            );
            assert_eq!(state, before);
        }

        let mut overflow = SoloPetState::from_canonical(Some((i32::MAX, i32::MAX)), 0).unwrap();
        let before = overflow;
        assert_eq!(overflow.summon(1), Err(PetStateError::CounterOverflow));
        assert_eq!(overflow, before);
    }

    /// #3246: while ending, a living re-summon grows MaxHp only (the
    /// GainMaxHp Heal returns early for a non-player); a dead or absent Osty
    /// refuses atomically; zero stays a no-op and MaxHp overflow still refuses.
    #[test]
    fn ending_summon_grows_max_hp_only_and_refuses_the_add_revive_branch() {
        let mut state = SoloPetState::from_canonical(Some((16, 23)), 0).unwrap();
        state.summon_ending(3, true).unwrap();
        assert_eq!(state.osty(), Some(Osty { hp: 16, max_hp: 26 }));
        state.summon_ending(0, true).unwrap();
        assert_eq!(state.osty(), Some(Osty { hp: 16, max_hp: 26 }));

        let mut corpse = SoloPetState::from_canonical(Some((3, 9)), 1).unwrap();
        corpse.kill();
        for mut dead in [SoloPetState::default(), corpse] {
            let before = dead;
            assert_eq!(
                dead.summon_ending(5, true),
                Err(PetStateError::EndingSummonNotLive)
            );
            assert_eq!(dead, before);
            dead.summon_ending(0, true).unwrap();
            assert_eq!(dead, before);
        }

        let mut overflow = SoloPetState::from_canonical(Some((1, i32::MAX)), 0).unwrap();
        let before = overflow;
        assert_eq!(
            overflow.summon_ending(1, true),
            Err(PetStateError::CounterOverflow)
        );
        assert_eq!(overflow, before);
    }

    #[test]
    fn death_preserves_attack_history_and_turn_reset_does_not_revive() {
        let mut state = SoloPetState::from_canonical(Some((3, 9)), 2).unwrap();
        state.record_attack().unwrap();
        state.kill();
        assert_eq!(state.osty(), None);
        assert_eq!(state.attacks_this_turn(), 3);
        state.reset_attacks();
        assert!(state.corpse());
        assert!(state.has_die_for_you());
        state.heal(9).unwrap();
        assert_eq!(state.osty(), None);
        state.summon(2).unwrap();
        assert_eq!(state.osty(), Some(Osty { hp: 2, max_hp: 2 }));

        let mut overflow = SoloPetState::from_canonical(None, i32::MAX).unwrap();
        let before = overflow;
        assert_eq!(
            overflow.record_attack(),
            Err(PetStateError::CounterOverflow)
        );
        assert_eq!(overflow, before);
    }
}

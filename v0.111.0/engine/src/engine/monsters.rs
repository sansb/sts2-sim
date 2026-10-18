//! Shared monster-roster primitives for R2 unit A (#1560).
//!
//! Mid-combat additions allocate creation-order uids, consume exactly one
//! Niche draw, and publish in board-slot order. Ovicopter's retained egg
//! identities and the illusion corpse/revive state are validated here so the
//! move, death, turn, and admission paths cannot grow separate opinions.

use crate::catalog::{CardIdentity, Catalog};
use crate::content_tables::AscensionTier;
use crate::content_tables::{CardRarity, CardType};
use crate::engine::{EngineRefusal, Event, Subject};
use crate::hot::{
    CARD_FLAG_BOUND, CARD_FLAG_DEFAULT_PHYSICAL_STATE, CARD_FLAG_GENETIC_ALGORITHM_STATE,
    CARD_FLAG_HEXED, CARD_FLAG_LEGACY, CARD_FLAG_PICK, CARD_FLAG_RINGING,
    CARD_FLAG_SOVEREIGN_BLADE_STATE, CARD_FLAGS_KNOWN, HopperDeckRow, HopperLootKind, HotMonster,
    HotState, MonsterFollowUp, MonsterOverride, PileId, RngStream, RngStreamState,
};
use crate::ids::{CardId, EnchantmentId, MonsterKind, PowerId};
use crate::powers::{SlotWire, Slots};
use crate::rng::Xoshiro256StarStar;

pub(crate) const OVICOPTER_SLOTS: i32 = 6;
pub(crate) const OVICOPTER_BRANCH_POS: i32 = 4;
// A10 (`MODELED_ASCENSION`) fixture HPs. Every validator selects the fight's
// tier from `MONSTER_MODELS` instead (`native_fixed_hp`, #2539), and
// `a10_fixture_hps_are_the_modeled_tier` pins these against it.
#[cfg(test)]
pub(crate) const THE_LOST_HP: i32 = 99;
#[cfg(test)]
pub(crate) const THE_FORGOTTEN_HP: i32 = 111;
pub(crate) const FABRICATOR_SLOTS: i32 = 5;
pub(crate) const RAT_SLOTS: i32 = 5;
pub(crate) const AXEBOT_STOCK: i32 = 2;
#[cfg(test)]
pub(crate) const SOUL_FYSH_HP: i32 = 221;
#[cfg(test)]
pub(crate) const ENTOMANCER_HP: i32 = 165;
pub const TORCH_HEAD_AMALGAM_HP: i32 = 211;
pub const QUEEN_HP: i32 = 419;
pub(crate) const MONSTER_BLOCK_CAP: i32 = 999_999_999;
/// `ReattachPower`'s revival, as the segment's `NextMove`: 0 alive, 1 DEAD
/// showing, 2 REATTACH showing, 3 REATTACH performed and RAND roll pending.
///
/// `DecimillipedeSegment::GenerateMoveStateMachine` RVA `0xb2c04` is the
/// authority for the shape: `DeadState.FollowUpState = REATTACH_MOVE`
/// (IL_0114-IL_011c) and `REATTACH_MOVE.FollowUpState = RAND`
/// (IL_0121-IL_0125). Only REATTACH_MOVE carries
/// `MustPerformOnceBeforeTransitioning` (IL_00eb-IL_00ec); DEAD_MOVE
/// (IL_00a6-IL_00c1) does not. So stage 1 -> 2 happens at the next
/// player-side roll whether or not DEAD was performed
/// (`turn::advance_segment_dead_states`, #3251), stage 2 -> 3 is the
/// REATTACH action's heal, and stage 3 -> 0 is `prepare_random_ai`'s RAND.
pub(crate) const SEGMENT_REVIVE_DEAD: u8 = 1;
pub(crate) const SEGMENT_REVIVE_REATTACH: u8 = 2;
pub(crate) const SEGMENT_REVIVE_RAND: u8 = 3;
/// A segment killed while stunned (#3823): dead for good, and its machine
/// still acting.
///
/// `ReattachPower/<AfterDeath>d__11::MoveNext` `0x341d30` sets `isReviving`
/// (IL_005a-IL_0065) and then installs DEAD with `SetMoveImmediate(DeadState,
/// force: false)` (IL_0086-IL_008c). An unperformed STUNNED cannot be left
/// (`MonsterModel::SetMoveImmediate` `0x825f0` IL_000c-IL_001c), so DEAD is
/// dropped: the corpse performs STUNNED, then the parked move, then its loop,
/// every enemy turn at 0 HP, and never reaches REATTACH (`turn::dead_segment_act`).
/// `isReviving` stays set, so `ReattachPower::ShouldAllowHitting` `0xa6643`
/// (IL_000c-IL_0015) keeps vetoing powers on it. It counts as dead for
/// `AreAllOtherSegmentsDead` (`0xa683f`, `IsDead`), so the last living
/// segment's death still ends the fight. Native traces `dcm_s1k1`,
/// `dcm_s1k1k2`, `dcm_s1k1k2k3`, `dcm_k1s3k3`.
pub(crate) const SEGMENT_REVIVE_STUN_KEPT: u8 = 4;
/// An illusion's `revive_stage` while its corpse waits to spend
/// REVIVE_MOVE (`IllusionPower/<AfterDeath>d__18::MoveNext` `0x33ca54`
/// IL_00b3-IL_011b installs it; `turn::monster_act` spends it).
pub(crate) const ILLUSION_REVIVING: u8 = 1;
/// An illusion killed while stunned (#3823): dead for good, and its machine
/// still acts every enemy turn.
///
/// `IllusionPower/<AfterDeath>d__18::MoveNext` `0x33ca54` sets
/// `isReviving` (IL_00a7-IL_00ae), builds REVIVE_MOVE (IL_00b3-IL_0108) and
/// installs it with `MonsterModel::SetMoveImmediate(state, force: false)`
/// (IL_0119-IL_011b). `SetMoveImmediate` `0x825f0` IL_000c-IL_001c returns
/// without installing when the current state cannot transition away, and the
/// STUNNED state is `MustPerformOnceBeforeTransitioning` until it has
/// performed (`Creature::StunInternal` `0x11d7cc` IL_0078,
/// `MoveState::get_CanTransitionAway` `0x78ff3`). So the revive is dropped
/// and nothing installs it again: AfterDeath is its only caller.
///
/// The creature stays in combat (`IllusionPower::
/// ShouldCreatureBeRemovedFromCombatAfterDeath` `0xa3b33` vetoes its own
/// owner's removal) and `isReviving` stays set, so it cannot be hit
/// (`IllusionPower::ShouldAllowHitting` `0xa3b1b` IL_000c-IL_0015).
/// `CombatManager/<ExecuteEnemyTurn>d__126::MoveNext` `0x3f597c`
/// IL_00e4-IL_01c4 calls `TakeTurn` on every enemy the combat still contains,
/// so the dead illusion performs STUNNED, then the parked move, then its
/// loop, every enemy turn at 0 HP. How each move resolves for a dead actor is
/// at [`crate::engine::turn`]'s dead-actor arm. It does not keep the fight
/// going: `CombatManager::IsCombatEnding` `0x135854` IL_0026-IL_0058 asks
/// only whether an enemy `IsAlive && IsPrimaryEnemy`
/// (`<IsCombatEnding>b__82_0` `0x3f2532`).
///
/// Native traces `obs_sk2*`, `fog_sk2*`, `*_s2s3k3*`, `*_skall*`: the dead
/// Parafright's SLAM deals nothing, the dead Eye With Teeth's DISTRACT adds
/// three Dazed every turn, and neither ever revives.
pub(crate) const ILLUSION_REVIVE_DROPPED: u8 = 2;

/// Is this a Parafright or an Eye With Teeth (`Apply<IllusionPower>`'s two
/// owners)?
pub(crate) fn is_illusion(kind: MonsterKind) -> bool {
    matches!(kind, MonsterKind::Parafright | MonsterKind::EyeWithTeeth)
}

/// The dead-acting illusion while its STUNNED has not yet performed: a
/// corpse at [`ILLUSION_REVIVE_DROPPED`] carrying the generic stun its death
/// could not replace.
pub(crate) fn illusion_dead_stun_is_exact(monster: &HotMonster) -> bool {
    is_illusion(monster.kind)
        && monster.hp <= 0
        && monster.revive_stage == ILLUSION_REVIVE_DROPPED
        && generic_stun_shape_parks_this_row(monster)
}

/// A segment killed under a stun, still holding it
/// ([`SEGMENT_REVIVE_STUN_KEPT`]).
pub(crate) fn segment_dead_stun_is_exact(monster: &HotMonster) -> bool {
    monster.kind == MonsterKind::DecimillipedeSegment
        && monster.hp <= 0
        && monster.revive_stage == SEGMENT_REVIVE_STUN_KEPT
        && generic_stun_shape_parks_this_row(monster)
}

/// The stun a segment dies under, which its death keeps
/// ([`SEGMENT_REVIVE_STUN_KEPT`]), or `None`. Read before
/// [`clear_generic_stun_at_death`]; the death body puts it back only when a
/// peer still stands (otherwise the fight ends).
pub(crate) fn segment_stun_survives_death(
    monster: &HotMonster,
) -> Option<(MonsterOverride, MonsterFollowUp)> {
    (monster.kind == MonsterKind::DecimillipedeSegment
        && monster.revive_stage == 0
        && generic_stun_shape_parks_this_row(monster))
    .then_some((monster.override_state, monster.forced_follow_up))
}
/// `ReattachPower/<DoReattach>d__10::MoveNext` RVA `0x341e90` IL_0081-IL_0093
/// heals the owner for the power's own Amount once `AreAllOtherSegmentsDead`
/// is false. `monster_act` (frozen Python, deleted #2827) pins that Amount at its `REATTACH_HEAL`
/// module constant, 25.
pub(crate) const SEGMENT_REATTACH_HEAL: i32 = 25;
pub const THIEVING_HOPPER_HP: i32 = 84;
pub const THIEVING_HOPPER_ESCAPE_ARTIST: i32 = 5;
pub const THIEVING_HOPPER_FLUTTER: i32 = 5;
#[cfg(test)]
pub const AEONGLASS_HP: i32 = 535;

/// `kind`'s single native `MaxInitialHp` at this fight's ascension (#2539),
/// from `MONSTER_MODELS` through [`crate::encounters::fixed_hp`]: what a
/// fixed-HP roster pin compares `max_hp` with. `None` for an unread row or a
/// ranged band, which no fixed pin can equal.
/// Ceremonial Beast's Plow threshold at this fight's ascension (#3366).
///
/// `<StampMove>d__53::MoveNext` RVA `0x356060` loads
/// `CeremonialBeast::get_PlowAmount` (IL_0121) as the amount of the
/// `PowerCmd.Apply<PlowPower>` call at IL_0133, and `get_PlowAmount` RVA
/// `0xb0c33` (IL_0001-IL_000d) is `GetValueIfAscension(9, 160, 150)`. Plow's
/// own check, `PlowPower/<AfterDamageReceived>d__8::MoveNext` RVA `0x340848`
/// IL_005e-IL_006f, compares `CurrentHp <= Amount` against that installed
/// amount, so every Plow owner/state predicate reads the fight's tier rather
/// than the A9+ value: an A6 Beast carries 150, not 160.
pub(crate) fn ceremonial_beast_plow_threshold(state: &HotState) -> i32 {
    crate::encounters::tier(
        crate::content_tables::move_constants::CEREMONIAL_BEAST_PLOW_AMOUNT,
        state.fanouts.ascension(),
    ) as i32
}

pub(crate) fn native_fixed_hp(state: &HotState, kind: MonsterKind) -> Option<i32> {
    crate::encounters::fixed_hp(kind, state.fanouts.ascension())
}

/// Whether `max_hp` is inside `kind`'s native band at this fight's ascension.
pub(crate) fn native_hp_in_band(state: &HotState, kind: MonsterKind, max_hp: i32) -> bool {
    crate::encounters::hp_in_band(kind, state.fanouts.ascension(), max_hp)
}

/// One spawn-time `Apply<XPower>` amount at this fight's ascension, or `None`
/// for an unread row / a power the kind does not apply.
pub(crate) fn native_initial_power(
    state: &HotState,
    kind: MonsterKind,
    power: &'static str,
) -> Option<i32> {
    crate::encounters::initial_power(kind, power, state.fanouts.ascension())
        .ok()
        .and_then(|amount| i32::try_from(amount).ok())
}

/// `kind`'s native HP band at this fight's ascension, for a spawn's
/// `SetUniqueMonsterHpValue` draw.
fn native_hp_band(state: &HotState, kind: MonsterKind) -> Result<(i32, i32), EngineRefusal> {
    crate::encounters::hp_band(kind, state.fanouts.ascension())
        .map_err(|_| EngineRefusal::MalformedArgs("spawned creature HP band"))
}

/// `kind`'s `MONSTER_MODELS` HP tiers `(MinInitialHp, MaxInitialHp)`, resolved
/// during constant evaluation (#2852).
///
/// The constants below are built from this, so the generated table is their
/// only source and no table lookup runs in a validator or a transition: the
/// row is indexed once, by the compiler. A row the generator refuses fails
/// the build here by name instead of leaving a stale hand-read number behind.
const fn model_hp_tiers(kind: MonsterKind) -> (AscensionTier, AscensionTier) {
    let model = &crate::content_tables::MONSTER_MODELS[kind as usize];
    match (model.min_initial_hp, model.max_initial_hp) {
        (Some(low), Some(high)) => (low, high),
        _ => panic!("MONSTER_MODELS refuses a row engine::monsters reads"),
    }
}

/// `kind`'s spawn-time `Apply<XPower>` amount tier for `power`, from
/// `MONSTER_MODELS`, resolved during constant evaluation like
/// [`model_hp_tiers`] (#2852). A row without that power fails the build.
const fn model_initial_power_tier(kind: MonsterKind, power: &str) -> AscensionTier {
    let powers = crate::content_tables::MONSTER_MODELS[kind as usize].initial_powers;
    let wanted = power.as_bytes();
    let mut index = 0;
    while index < powers.len() {
        let name = powers[index].0.as_bytes();
        let mut same = name.len() == wanted.len();
        let mut byte = 0;
        while same && byte < name.len() {
            same = name[byte] == wanted[byte];
            byte += 1;
        }
        if same {
            return powers[index].1;
        }
        index += 1;
    }
    panic!("MONSTER_MODELS row lacks an initial power engine::monsters reads")
}

/// Axebot's HP band before any respawn, read from its `MONSTER_MODELS` row
/// (#2852; the row has been read since #2534). Natively
/// `Axebot::get_MinInitialHp` (RVA `0xaed95`) and `get_MaxInitialHp`
/// (`0xaeda8`), IL_0001-IL_0011, are `GetValueIfAscension(8, 76, 70)` and
/// `(8, 86, 78)`, each **plus** `get_RespawnMaxHpBonus` (`0xaedbb`,
/// `RespawnCount * 10`). The generated row carries the two tiers at
/// `RespawnCount` zero; the addend for a body created at a lower Stock is not
/// a model fact, so [`AXEBOT_RESPAWN_HP_BONUS`] stays here.
const AXEBOT_MIN_HP: AscensionTier = model_hp_tiers(MonsterKind::Axebot).0;
const AXEBOT_MAX_HP: AscensionTier = model_hp_tiers(MonsterKind::Axebot).1;
/// `Axebot::get_RespawnMaxHpBonus` (RVA `0xaedbb`): `RespawnCount * 10`, the
/// per-respawn addend to both ends of the band. `MONSTER_MODELS` has no
/// column for it, so it is read here.
const AXEBOT_RESPAWN_HP_BONUS: i32 = 10;
/// `ToughEgg::get_MinInitialHp` (`0xc303a`) / `get_MaxInitialHp` (`0xc3046`):
/// `GetValueIfAscension(8, 15, 14)` / `(8, 19, 18)`; the hatchling's
/// `get_HatchlingMinHp` (`0xc3052`) / `get_HatchlingMaxHp` (`0xc305e`):
/// `(8, 20, 19)` / `(8, 23, 22)`. `MONSTER_MODELS` refuses the row over its
/// `HatchPower` amount spelling (a side-dependent 1/2, not a tier), so the
/// HP getters are read here (#2539).
const TOUGH_EGG_MIN_HP: AscensionTier = AscensionTier {
    gate: Some(8),
    at_or_above: 15,
    below: 14,
};
const TOUGH_EGG_MAX_HP: AscensionTier = AscensionTier {
    gate: Some(8),
    at_or_above: 19,
    below: 18,
};
const TOUGH_EGG_HATCHLING_MIN_HP: AscensionTier = AscensionTier {
    gate: Some(8),
    at_or_above: 20,
    below: 19,
};
const TOUGH_EGG_HATCHLING_MAX_HP: AscensionTier = AscensionTier {
    gate: Some(8),
    at_or_above: 23,
    below: 22,
};

/// Test Subject's three form HPs and its spawn-time Enrage (#2539).
///
/// The first form and the Enrage amount are read from the `MONSTER_MODELS`
/// row (#2852; read since #2534): `get_MinInitialHp` (`0xc0136`) delegates to
/// `get_FirstFormHp` `0xc0146` `GetValueIfAscension(8, 111, 100)`, and
/// `get_EnrageAmount` `0xc0176` `(9, 3, 2)` is applied by
/// `<AfterAddedToRoom>d__68::MoveNext` `0x36d628` IL_0106-IL_0118 as
/// `Apply<EnragePower>`, the row's `EnragePower` initial power.
///
/// The second and third forms are not model facts — the row has one HP band,
/// the spawn's — so they stay hand-read here: v0.111.0 `sts2.dll`
/// `9cb4f1ad…` `get_SecondFormHp` `0xc0152` `(8, 212, 200)` and
/// `get_ThirdFormHp` `0xc0164` `(8, 313, 300)`, which the RESPAWN body
/// `<RespawnMove>d__74` `0x36de44` revives at (IL_021d / IL_02fc).
pub(crate) const TEST_SUBJECT_FORM_HP: [AscensionTier; 3] = [
    model_hp_tiers(MonsterKind::TestSubject).1,
    AscensionTier {
        gate: Some(8),
        at_or_above: 212,
        below: 200,
    },
    AscensionTier {
        gate: Some(8),
        at_or_above: 313,
        below: 300,
    },
];
pub(crate) const TEST_SUBJECT_ENRAGE_AMOUNT: AscensionTier =
    model_initial_power_tier(MonsterKind::TestSubject, "EnragePower");

/// Test Subject's max HP in `form` (0..=2) at this fight's ascension.
pub(crate) fn test_subject_form_hp(state: &HotState, form: usize) -> Option<i32> {
    TEST_SUBJECT_FORM_HP
        .get(form)
        .map(|tier| crate::encounters::tier(*tier, state.fanouts.ascension()) as i32)
}

/// Test Subject's spawn-time Enrage amount at this fight's ascension.
pub(crate) fn test_subject_enrage(state: &HotState) -> i32 {
    crate::encounters::tier(TEST_SUBJECT_ENRAGE_AMOUNT, state.fanouts.ascension()) as i32
}

/// Globe Head's intrinsic `GalvanicPower` Amount at this fight's ascension
/// (#3300).
///
/// `GlobeHead/<AfterAddedToRoom>d__20::MoveNext` RVA `0x35d338` (v0.111.0,
/// DLL 9cb4f1ad) awaits the base `AfterAddedToRoom`, then at IL_007f-IL_009d
/// `Apply<GalvanicPower>(Creature, get_GalvanicPowerAmount, Creature, null)`;
/// `get_GalvanicPowerAmount` RVA `0xb5ce6` IL_0001-IL_0005 is
/// `GetValueIfAscension(9, 8, 6)`. That is the `MONSTER_MODELS` initial-power
/// row, read here at the fight's ascension. Nothing in the lone Globe Head
/// encounter writes the power again: the monster's three moves
/// (`GenerateMoveStateMachine` RVA `0xb5d38`) apply only Frail to the player
/// and Strength to itself, and no player-side command in this build changes
/// another creature's Galvanic, so the spawn amount is the live Amount for
/// as long as the owner lives.
pub(crate) fn globe_head_galvanic_amount(state: &HotState) -> Option<i32> {
    native_initial_power(state, MonsterKind::GlobeHead, "GalvanicPower")
        .filter(|amount| *amount > 0)
}

/// The state in which "the played card is a Power" is exactly "the played
/// card carries `Galvanized`" (#3300), which is what lets the enemy
/// `AfterCardPlayed` suffix read the card's type instead of a per-card
/// affliction the hot state does not carry.
///
/// Native, v0.111.0 DLL 9cb4f1ad:
///
/// * `GalvanicPower/<BeforeCombatStart>d__8::MoveNext` RVA `0x33b35c`
///   walks every player ally's `PlayerCombatState.AllCards` filtered by
///   `<>c::<BeforeCombatStart>b__8_0` RVA `0x33b12a` (`card.Type == 3`,
///   Power) and awaits `CardCmd.Afflict<Galvanized>(card, Amount)` on each
///   (IL_00b1-IL_00be).
/// * `GalvanicPower/<AfterCardEnteredCombat>d__9::MoveNext` RVA `0x33b138`
///   returns when the entering card already has an affliction
///   (IL_001d-IL_002a) or is not a Power (IL_002f-IL_003d), and otherwise
///   afflicts it the same way (IL_0042-IL_0053). `Hook.AfterCardEnteredCombat`
///   is raised by `CardPileCmd.Add` (`<Add>d__10`) and by `CardCmd.Transform`
///   (`<Transform>d__13::MoveNext` RVA `0x3e0ae0` IL_0428) for the
///   replacement, which is a fresh model: a Power transformed away leaves no
///   Galvanized behind, and a card transformed into a Power is afflicted.
///   `CardModel::DeepCloneFields` copies the affliction to a clone.
/// * `CardCmd::Afflict` RVA `0x12fc3c` applies unless the combat is ending
///   (IL_000c-IL_0030), `Hook.ShouldAfflict` vetoes (no model in this build
///   overrides `AbstractModel::ShouldAfflict` RVA `0x7a308`, which returns
///   true), or `AfflictionModel::CanAfflict` RVA `0x7b460` refuses: the
///   base class accepts every card type and Unplayable cards, and refuses
///   only a card that already carries a *different* affliction.
/// * The only `ClearAffliction` callers are other affliction powers
///   (Chains of Binding, Hex, Ringing, Smoggy, Tangled, Vital Spark), the
///   `Hexed` affliction itself, and `NightmarePower::SetSelectedCard` RVA
///   `0xa4e2c`, which clears its private clone (IL_000d-IL_0014), whose later
///   copies re-enter combat through `CardPileCmd.Add`.
///
/// So with Galvanic live, every Power card in the combat is Galvanized and no
/// other card is, *provided no other affliction exists*. This proof is the
/// lone-Globe-Head roster (`GlobeHeadNormal`, `normal_c.rs`), which has no
/// other affliction source, with every represented affliction state clear.
pub(crate) fn globe_head_galvanic_state_is_exact(state: &HotState) -> bool {
    matches!(state.monsters.as_slice(), [only] if only.kind == MonsterKind::GlobeHead)
        && globe_head_galvanic_amount(state).is_some()
        && [
            PowerId::HexPower,
            PowerId::ChainsOfBinding,
            PowerId::Smoggy,
            PowerId::SmoggyFresh,
            PowerId::SmogLock,
            PowerId::Tangled,
        ]
        .into_iter()
        .all(|power| state.powers.value(power) == 0)
        && !state.ringing()
        && state.bound_afflictions_this_turn() == 0
        && PileId::ALL.into_iter().all(|pile| {
            state.piles.get(pile).as_slice().iter().all(|card| {
                card.flags & (CARD_FLAG_BOUND | CARD_FLAG_HEXED | CARD_FLAG_RINGING) == 0
            })
        })
}

/// Construct the exact A8+ solo Aeonglass boss and run its startup powers.
///
/// Current v0.111.0 `AfterAddedToRoom` outer/body `0xae85c`/`0x3524e8`
/// applies Withering Presence(6) before Artifact(3). The fixed 535..535 HP
/// range still consumes one ordinary Niche draw through SetHpValue.
pub fn construct_aeonglass_boss(
    state: &mut HotState,
    catalog: &Catalog,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    fn apply(
        state: &mut HotState,
        catalog: &Catalog,
        events: &mut Vec<Event>,
    ) -> Result<(), EngineRefusal> {
        let identity = CardIdentity {
            id: CardId::Wither,
            upgrade: 0,
            enchantment: None,
        };
        if state.history.over
            || state.multiplayer_ally_key != 0
            || !state.monsters.is_empty()
            || !state.exact_piles
            || state.fanouts.withering_cards_left() != 0
            || !state.rng.has_nonzero_words(RngStream::Niche)
            || !catalog.is_reachable(identity)
            || catalog
                .atom(&identity)
                .and_then(|atom| catalog.spec(atom))
                .is_none()
        {
            return Err(EngineRefusal::MalformedArgs(
                "Aeonglass encounter construction",
            ));
        }
        let hp = native_fixed_hp(state, MonsterKind::Aeonglass)
            .ok_or(EngineRefusal::MalformedArgs("Aeonglass HP band"))?;
        let hp = roll_hp(state, hp, hp)?;
        let uid = 0;
        let mut boss = HotMonster::new(MonsterKind::Aeonglass, hp);
        boss.max_hp = hp;
        boss.slot = 0;
        boss.uid = uid;
        state.monsters_mut().push(boss);
        // Native power-list order is observable: Withering is installed
        // first, then Artifact. The countdown is its sole exact token.
        let stored = state.fanouts.set_withering_cards_left(6);
        debug_assert!(stored);
        state
            .fanouts
            .register_after_card_played_power(
                crate::hot::AfterCardPlayedPowerToken::Withering,
                None,
            )
            .map_err(|_| EngineRefusal::CounterOverflow("Withering object ledger"))?;
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Artifact, SlotWire::Int, 3);
        super::damage::note_power(events, Subject::Monster(uid), PowerId::Artifact, 3);
        if !super::cards::aeonglass_state_is_exact(state, catalog) {
            return Err(EngineRefusal::MalformedArgs("Aeonglass startup result"));
        }
        Ok(())
    }

    let mut next = state.clone();
    let mut emitted = Vec::new();
    apply(&mut next, catalog, &mut emitted)?;
    *state = next;
    events.extend(emitted);
    Ok(())
}
/// The A10 (`MODELED_ASCENSION`) form HPs and Enrage, for fixtures; every
/// validator selects by the fight's ascension instead
/// ([`test_subject_form_hp`], [`test_subject_enrage`]).
#[allow(dead_code)]
pub(crate) const TEST_SUBJECT_FIRST_HP: i32 = crate::encounters::tier(
    TEST_SUBJECT_FORM_HP[0],
    crate::encounters::MODELED_ASCENSION,
) as i32;
#[allow(dead_code)]
pub(crate) const TEST_SUBJECT_SECOND_HP: i32 = crate::encounters::tier(
    TEST_SUBJECT_FORM_HP[1],
    crate::encounters::MODELED_ASCENSION,
) as i32;
#[allow(dead_code)]
pub(crate) const TEST_SUBJECT_THIRD_HP: i32 = crate::encounters::tier(
    TEST_SUBJECT_FORM_HP[2],
    crate::encounters::MODELED_ASCENSION,
) as i32;
#[allow(dead_code)]
pub(crate) const TEST_SUBJECT_ENRAGE: i32 = crate::encounters::tier(
    TEST_SUBJECT_ENRAGE_AMOUNT,
    crate::encounters::MODELED_ASCENSION,
) as i32;
pub(crate) const FABRICATOR_FABRICATE_INDEX: u8 = 0;
pub(crate) const FABRICATOR_STRIKE_INDEX: u8 = 1;
pub(crate) const FABRICATOR_DISINTEGRATE_INDEX: u8 = 2;

/// Structural half of a live copy's correspondence with its master row: one
/// live physical object per master uid. The identity half needs the catalog
/// and lives in [`hopper_live_identity_matches_master`].
fn hopper_row_live_card_is_exact(state: &HotState, row: &HopperDeckRow) -> bool {
    if row.card.uid == u32::MAX || row.card.flags & CARD_FLAG_LEGACY != 0 {
        return false;
    }
    let mut matches = PileId::ALL
        .into_iter()
        .flat_map(|pile| state.piles.get(pile).as_slice().iter().copied())
        .filter(|card| card.uid == row.card.uid);
    matches.next().is_some() && matches.next().is_none()
}

/// Whether a live copy is still its master row's card: same card id and same
/// enchantment, at any upgrade level (#2965).
///
/// A master row is the card's `DeckVersion` object, and a combat upgrade
/// never reaches it: `CardCmd::Upgrade` RVA `0x12f660` runs
/// `CardModel::UpgradeInternal` (`0x7e0c8`) and `FinalizeUpgradeInternal`
/// (`0x7e10c`) on the combat object alone (IL_0080-IL_0087), and neither
/// body reads `DeckVersion`. Stone Cracker's pre-deal upgrade and every
/// in-combat upgrade therefore leave the master at its pre-combat level, and
/// Thievery steals and Swipe returns exactly that object
/// (`SwipePower::<Steal>d__13::MoveNext` `0x347bbc` IL_003f-IL_005b removes
/// `card.DeckVersion` from the deck; `SwipePower::BeforeDeath` `0xa8ed4`
/// IL_0043-IL_0058 adds `StolenCard.DeckVersion` back). Thievery's priority
/// reads the combat object's rarity and Imbued enchantment
/// (`ThievingHopper::<>c` lambdas `b__43_0`-`b__43_3`), which an upgrade
/// does not change. A differing enchantment stays refused: only Goopy grows
/// a master enchantment natively, and its mirror keeps live and master equal.
fn hopper_live_identity_matches_master(
    catalog: &crate::catalog::Catalog,
    live: crate::hot::HotCard,
    row: &HopperDeckRow,
) -> bool {
    match (catalog.spec(live.atom), catalog.spec(row.card.atom)) {
        (Some(live), Some(master)) => {
            live.identity.id == master.identity.id
                && live.identity.enchantment == master.identity.enchantment
        }
        _ => false,
    }
}

fn hopper_deck_row_payload_is_exact(
    row: &HopperDeckRow,
    catalog: &crate::catalog::Catalog,
) -> bool {
    let card = row.card;
    let instance = &row.state;
    let Some(spec) = catalog.spec(card.atom) else {
        return false;
    };
    let forbidden_flags = CARD_FLAG_PICK
        | CARD_FLAG_LEGACY
        | CARD_FLAG_RINGING
        | CARD_FLAG_BOUND
        | CARD_FLAG_HEXED
        | CARD_FLAG_SOVEREIGN_BLADE_STATE;
    if card.uid == u32::MAX
        || card.flags & !CARD_FLAGS_KNOWN != 0
        || card.flags & forbidden_flags != 0
        || instance
            .base_replay_count()
            .is_some_and(|replay| replay != 0 || !matches!(spec.identity.id, CardId::TheScythe))
        || !instance.local_cost_modifiers.is_empty()
        || instance.local_cost_modifiers.free_star_cost_this_combat()
        || instance.local_ethereal()
        || instance.free_star_cost_this_turn_or_played_rows != 0
        || instance.local_retain
        || instance.local_sly
        || instance.transient_retain
        || instance.transient_sly()
        || instance.has_exact_damage_growth_aux()
    {
        return false;
    }
    let has_default = card.flags & CARD_FLAG_DEFAULT_PHYSICAL_STATE != 0;
    let has_genetic = card.flags & CARD_FLAG_GENETIC_ALGORITHM_STATE != 0;
    let zero_mutable_enchantment = crate::hot::CardInstanceState {
        enchantment_state: instance.enchantment_state,
        ..crate::hot::CardInstanceState::default()
    };
    let is_only_explicit_zero_mutable_enchantment =
        instance.enchantment_state.get() == Some(0) && *instance == zero_mutable_enchantment;
    if !instance.is_vacant() && !has_default && !is_only_explicit_zero_mutable_enchantment {
        return false;
    }
    let enchantment_state_is_exact = match instance.enchantment_state.get() {
        None => true,
        Some(0) => spec.identity.enchantment.is_some_and(|enchantment| {
            matches!(
                enchantment.id,
                EnchantmentId::Momentum | EnchantmentId::Vigorous
            ) && enchantment.amount > 0
        }),
        Some(_) => false,
    };
    if !enchantment_state_is_exact {
        return false;
    }
    match spec.identity.id {
        CardId::GeneticAlgorithm => {
            has_default
                && has_genetic
                && instance.damage_growth == 0
                && instance.genetic_algorithm.deck_row() == Some(card.uid)
        }
        CardId::TheScythe => {
            !has_genetic
                && instance.genetic_algorithm == Default::default()
                && instance.damage_growth >= 0
                && (instance.damage_growth == 0 || has_default)
        }
        _ => {
            !has_genetic
                && instance.genetic_algorithm == Default::default()
                && instance.damage_growth == 0
        }
    }
}

/// Cold source-derived validation for the complete persistent Hopper payload.
///
/// Combat-local flags and local/transient slot-7 rows never enter DeckVersion.
/// The two represented persistent mutable cards keep their native keyed
/// correspondence: Genetic Algorithm's row id equals the physical uid, while
/// The Scythe's live and master damage growth remain equal.  Ordinary live
/// objects may still diverge through Ringing/Bound/Hexed, local keywords,
/// costs, and replay state without mutating the master row.
///
/// A master row may have no combat object. Three things leave one behind:
/// the returned row, a played Power, and a transform (#3856).
/// `CardCmd/<Transform>d__13::MoveNext` RVA `0x3e0ae0` takes the original
/// out of its pile (`RemoveFromCurrentPile`, IL_0186-IL_018c) and out of the
/// combat (`CardModel::RemoveFromState` RVA `0x7dbb2`, IL_0a22); it reaches
/// the run deck only for an original whose pile is the Deck itself
/// (`CardPile::get_Type` IL_0316, `Hook::ModifyCardBeingAddedToDeck`
/// IL_032f). So the transformed card's `DeckVersion` stays in the run deck
/// with nothing in combat pointing at it. A transform is open to every card
/// type, so an absent row that is neither the returned one nor a Power is
/// admitted exactly when a transform recorded it
/// ([`note_hopper_transformed`], `HopperDeckState::transformed`, the
/// document's `hopper_transformed_rows`).
pub(crate) fn thieving_hopper_deck_payload_is_exact(
    state: &HotState,
    catalog: &crate::catalog::Catalog,
) -> bool {
    let Some(deck) = state.card_states.hopper() else {
        return false;
    };
    let returned_uid = match deck.history.as_slice() {
        [stolen, returned]
            if stolen.kind == HopperLootKind::Stolen
                && returned.kind == HopperLootKind::Returned
                && stolen.row == returned.row =>
        {
            Some(returned.row.card.uid)
        }
        _ => None,
    };
    let all_rows_exact = deck.master.iter().all(|row| {
        hopper_deck_row_payload_is_exact(row, catalog)
            && match PileId::ALL
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .find(|card| card.uid == row.card.uid)
            {
                Some(live) if !hopper_live_identity_matches_master(catalog, *live, row) => false,
                Some(live) => {
                    let live_state = state.card_states.get(live.uid);
                    match catalog.spec(row.card.atom).map(|spec| spec.identity.id) {
                        Some(CardId::GeneticAlgorithm) => {
                            live_state.genetic_algorithm == row.state.genetic_algorithm
                                && live_state.genetic_algorithm.deck_row() == Some(live.uid)
                        }
                        Some(CardId::TheScythe) => {
                            live_state.exact_damage_growth() == row.state.exact_damage_growth()
                        }
                        _ => true,
                    }
                }
                None => {
                    returned_uid == Some(row.card.uid)
                        || catalog
                            .spec(row.card.atom)
                            .is_some_and(|spec| spec.row.card_type == CardType::Power)
                        || deck.transformed.binary_search(&row.card.uid).is_ok()
                }
            }
    });
    // Each transformed uid names one master row that no pile holds.
    let transformed_rows_exact = deck.transformed.windows(2).all(|pair| pair[0] < pair[1])
        && deck.transformed.iter().all(|uid| {
            deck.master.iter().any(|row| row.card.uid == *uid)
                && !PileId::ALL
                    .into_iter()
                    .flat_map(|pile| state.piles.get(pile).as_slice())
                    .any(|card| card.uid == *uid)
        });
    all_rows_exact
        && transformed_rows_exact
        && deck
            .history
            .iter()
            .all(|event| hopper_deck_row_payload_is_exact(&event.row, catalog))
}

/// Record that a transform took the combat object of master row `uid` out
/// of combat (#3856). A card with no master row (a generated card, or any
/// card outside a Thieving Hopper fight) records nothing.
///
/// `CardCmd/<Transform>d__13::MoveNext` RVA `0x3e0ae0` removes the original
/// from its pile (IL_0186-018c) and from the combat (IL_0a22) and reaches
/// run-deck state only for a Deck-pile original (IL_0316, IL_032f), so the
/// `DeckVersion` object is untouched and unreferenced afterwards. The
/// engine's transform writers are closed and each calls this where it drops
/// the original: `cards::bulk_transform_fixed_same_pile` (Begone, Charge,
/// Guards, Seance, Compact), Primal Force, and Entropy.
pub(crate) fn note_hopper_transformed(state: &mut HotState, uid: u32) {
    let owns_row = state
        .card_states
        .hopper()
        .is_some_and(|deck| deck.master.iter().any(|row| row.card.uid == uid));
    if !owns_row {
        return;
    }
    if let Some(deck) = state.card_states.hopper_mut()
        && let Err(at) = deck.transformed.binary_search(&uid)
    {
        deck.transformed.insert(at, uid);
    }
}

/// Keep Genetic Algorithm's persistent DeckVersion object synchronized with
/// the live physical object. Generated/mutable copies have no Hopper master
/// row and therefore take the ordinary no-op branch.
pub(crate) fn update_hopper_genetic_algorithm_master(
    state: &mut HotState,
    uid: u32,
    before: &crate::hot::CardInstanceState,
    after: &crate::hot::CardInstanceState,
) -> Result<(), EngineRefusal> {
    if before.genetic_algorithm.deck_row() != Some(uid) {
        return Ok(());
    }
    let Some(deck) = state.card_states.hopper_mut() else {
        return Ok(());
    };
    let matching: Vec<_> = deck
        .master
        .iter()
        .enumerate()
        .filter(|(_, row)| row.card.uid == uid)
        .map(|(index, _)| index)
        .collect();
    if matching.len() != 1
        || deck.master[matching[0]].state.genetic_algorithm != before.genetic_algorithm
    {
        return Err(EngineRefusal::MalformedArgs(
            "Hopper Genetic Algorithm master row",
        ));
    }
    deck.master[matching[0]].state.genetic_algorithm = after.genetic_algorithm;
    Ok(())
}

/// Keep The Scythe's persistent DeckVersion object synchronized with the live
/// card. The caller supplies both exact physical snapshots so this cannot
/// silently manufacture a missing or stale master relation.
pub(crate) fn update_hopper_scythe_master(
    state: &mut HotState,
    uid: u32,
    before: &crate::hot::CardInstanceState,
    after: &crate::hot::CardInstanceState,
) -> Result<(), EngineRefusal> {
    let Some(deck) = state.card_states.hopper_mut() else {
        return Ok(());
    };
    let matching: Vec<_> = deck
        .master
        .iter()
        .enumerate()
        .filter(|(_, row)| row.card.uid == uid)
        .map(|(index, _)| index)
        .collect();
    if matching.is_empty() {
        return Ok(());
    }
    if matching.len() != 1 || deck.master[matching[0]].state.damage_growth != before.damage_growth {
        return Err(EngineRefusal::MalformedArgs("Hopper Scythe master row"));
    }
    deck.master[matching[0]].state.damage_growth = after.damage_growth;
    Ok(())
}

/// Whether a transform of `uid` is still refused for the run-deck object it
/// is linked to: an ordinary fight's The Scythe `DeckVersion` link (#2941),
/// whose growth ledger no port has read against a transform.
///
/// A Thieving Hopper master row is not one of these (#3856). The transform
/// leaves the run-deck object where it is
/// ([`thieving_hopper_deck_payload_is_exact`] has the IL), the replacement
/// is a fresh card with no `DeckVersion`, and the theft takes only cards
/// that have one (`ThievingHopper/<>c::<ThieveryMove>b__36_0` RVA
/// `0x370c1e`, `get_DeckVersion` IL_0002). In the engine that is a master
/// row whose uid no pile holds any more and a replacement uid no row names.
pub(crate) fn deck_version_link_refuses_transform(state: &HotState, uid: u32) -> bool {
    state.card_states.scythe_deck_row(uid).is_some()
}

fn hopper_owner_shape_is_exact(state: &HotState, owner: &HotMonster) -> bool {
    owner.kind == MonsterKind::ThievingHopper
        && (owner.slot, owner.uid, Some(owner.max_hp))
            == (0, 0, native_fixed_hp(state, MonsterKind::ThievingHopper))
        && owner.hp <= owner.max_hp
        && (0..=MONSTER_BLOCK_CAP).contains(&owner.block)
        && matches!(owner.loop_pos, 0..=4)
        && owner.random_ai.is_empty()
        && !owner.spawn_noop
        && owner.last_spawned.is_none()
        && owner.curl_up_card_uid == -1
        && !owner.louse_curled()
        && owner.possess_strength_debit == 0
        && owner.possess_speed_debit == 0
        && owner.hopper_private_state_is_exact()
        && (generic_stun_parks_this_row(owner)
            || matches!(
                (owner.override_state, owner.forced_follow_up),
                (MonsterOverride::None, MonsterFollowUp::None)
                    | (MonsterOverride::Stunned, MonsterFollowUp::HopperNab)
                    | (MonsterOverride::Stunned, MonsterFollowUp::HopperEscape)
            ) && (owner.override_state == MonsterOverride::None
                || (owner.hp > 0
                    && matches!(
                        (owner.loop_pos, owner.forced_follow_up),
                        (2, MonsterFollowUp::HopperNab) | (3 | 4, MonsterFollowUp::HopperEscape)
                    ))))
}

/// The Hopper's machine with a Whistle's generic stun read as the row it
/// parks (#3823): `(None, None)` for an undisturbed or generically stunned
/// owner, otherwise its own override pair (the Flutter stun).
///
/// The generic stun changes nothing the Hopper's validators guard but the
/// pace of the machine: `Creature::StunInternal` `0x11d7cc` writes the move
/// state alone, and `EscapeArtistPower::AfterSideTurnEnd` `0xa2238` ticks at
/// every enemy-side end whatever the owner performed. So a stunned turn
/// leaves `loop_pos` where it is while Escape Artist falls by one (it stops
/// at 1, `turn::run_side_end` only decrements above 1). With no stun the two
/// move together, `escape_artist == 5 - loop_pos`; each stunned turn opens
/// the gap by one, so the reachable pairs are `1 <= escape_artist <= 5 -
/// loop_pos` ([`hopper_escape_artist_is_reachable`]). Native traces
/// `hop_s*`, `hop_every`, `hop_late`.
fn hopper_machine_shape(owner: &HotMonster) -> (MonsterOverride, MonsterFollowUp) {
    if generic_stun_parks_this_row(owner) {
        (MonsterOverride::None, MonsterFollowUp::None)
    } else {
        (owner.override_state, owner.forced_follow_up)
    }
}

/// `1 <= escape_artist <= 5 - loop_pos` ([`hopper_machine_shape`]), with one
/// more on the enemy side before Escape Artist's own tick (`pre_tick`).
fn hopper_escape_artist_is_reachable(escape_artist: i32, loop_pos: i32, pre_tick: bool) -> bool {
    (1..=THIEVING_HOPPER_ESCAPE_ARTIST - loop_pos + i32::from(pre_tick)).contains(&escape_artist)
}

fn hopper_power_owners_are_exact(state: &HotState) -> bool {
    state.monsters.iter().all(|monster| {
        [PowerId::EscapeArtist, PowerId::Flutter]
            .into_iter()
            .all(|power| {
                monster.powers.get(power).is_none_or(|slot| {
                    monster.kind == MonsterKind::ThievingHopper
                        && slot.wire == SlotWire::Int
                        && match power {
                            PowerId::EscapeArtist => {
                                (1..=THIEVING_HOPPER_ESCAPE_ARTIST).contains(&slot.value)
                            }
                            PowerId::Flutter => (1..=THIEVING_HOPPER_FLUTTER).contains(&slot.value),
                            _ => unreachable!(),
                        }
                })
            })
    })
}

/// Complete structural solo Thieving Hopper/DeckVersion relation.
///
/// The cold sidecar owns every persistent master row in native order and the
/// exact empty/stolen/stolen+returned history grammar. A live pile object
/// bearing a master uid uses the same immutable card identity; combat-local
/// instance state remains independent. A played Power can legitimately be
/// absent from combat while its row remains in the native master deck; the
/// catalog-aware payload validator is the sole place that admits that narrow
/// absence. The stolen row is absent from both master and combat, while a
/// returned row is appended to the master but remains absent from combat
/// until a later run-level deck reconstruction.
fn thieving_hopper_structural_state_is_valid(state: &HotState) -> bool {
    if state.multiplayer_ally_key != 0
        || !state.exact_piles
        || !hopper_power_owners_are_exact(state)
    {
        return false;
    }
    let Some(deck) = state.card_states.hopper() else {
        return false;
    };
    if deck.master.iter().enumerate().any(|(position, row)| {
        row.card.uid >= state.next_card_uid
            || deck.master[..position]
                .iter()
                .any(|earlier| earlier.card.uid == row.card.uid)
    }) {
        return false;
    }
    let history_row = match deck.history.as_slice() {
        [] => None,
        [stolen] if stolen.kind == HopperLootKind::Stolen => Some((&stolen.row, false)),
        [stolen, returned]
            if stolen.kind == HopperLootKind::Stolen
                && returned.kind == HopperLootKind::Returned
                && stolen.row == returned.row =>
        {
            Some((&stolen.row, true))
        }
        _ => return false,
    };
    let ordered_prefix_len = deck.master.len().saturating_sub(usize::from(
        history_row.is_some_and(|(_, returned)| returned),
    ));
    if deck.master[..ordered_prefix_len]
        .windows(2)
        .any(|pair| pair[0].card.uid >= pair[1].card.uid)
    {
        return false;
    }
    let returned_uid = history_row
        .filter(|(_, returned)| *returned)
        .map(|(row, _)| row.card.uid);
    if deck.master.iter().any(|row| {
        if Some(row.card.uid) == returned_uid {
            !deck.master.last().is_some_and(|last| last == row)
                || PileId::ALL.into_iter().any(|pile| {
                    state
                        .piles
                        .get(pile)
                        .as_slice()
                        .iter()
                        .any(|card| card.uid == row.card.uid)
                })
        } else {
            PileId::ALL
                .into_iter()
                .flat_map(|pile| state.piles.get(pile).as_slice())
                .find(|card| card.uid == row.card.uid)
                .is_some_and(|_| !hopper_row_live_card_is_exact(state, row))
        }
    }) {
        return false;
    }
    if let Some((row, returned)) = history_row
        && (row.card.uid >= state.next_card_uid
            || (!returned
                && deck
                    .master
                    .iter()
                    .any(|master| master.card.uid == row.card.uid))
            || PileId::ALL.into_iter().any(|pile| {
                state
                    .piles
                    .get(pile)
                    .as_slice()
                    .iter()
                    .any(|card| card.uid == row.card.uid)
            }))
    {
        return false;
    }
    match state.monsters.as_slice() {
        [owner] if hopper_owner_shape_is_exact(state, owner) => {
            let escape_artist = owner.powers.value(PowerId::EscapeArtist);
            let flutter = owner.powers.value(PowerId::Flutter);
            if owner.hp > 0 {
                if escape_artist == 0
                    || owner.override_state == MonsterOverride::Stunned
                        && flutter != 0
                        && !generic_stun_parks_this_row(owner)
                {
                    return false;
                }
            } else if escape_artist != 0 || flutter != 0 {
                return false;
            }
            match history_row {
                None => owner.hopper_swipe_uid().is_none(),
                Some((row, false)) => {
                    owner.hp > 0 && owner.hopper_swipe_uid() == Some(row.card.uid)
                }
                Some((_row, true)) => owner.hopper_swipe_uid().is_none() && owner.hp <= 0,
            }
        }
        [] => {
            state.history.over && state.hp > 0 && history_row.is_none_or(|(_, returned)| !returned)
        }
        _ => false,
    }
}

/// Stable public Thieving Hopper boundary.
///
/// Escape Artist is applied at five before the first action and decrements at
/// each enemy-side end.  The fixed machine therefore authenticates the exact
/// `(loop position, Escape Artist)` pairs `(0,5)` through `(4,1)`.  Flutter is
/// absent before its loop-one body, remains live from loop two onward, and a
/// zero stack exists only after its delayed stun has installed the exact
/// native follow-up.  The sole loop-zero Swipe state is an internal transient:
/// THIEVERY has stolen the row but its enclosing `monster_act` has not yet
/// advanced the machine.
pub(crate) fn thieving_hopper_state_is_valid(state: &HotState) -> bool {
    if !thieving_hopper_structural_state_is_valid(state) {
        return false;
    }
    let [owner] = state.monsters.as_slice() else {
        // ESCAPE removes the sole owner and publishes the terminal boundary.
        return state.monsters.is_empty();
    };
    if owner.hp <= 0 {
        return state.history.over;
    }
    let escape_artist = owner.powers.value(PowerId::EscapeArtist);
    let flutter = owner.powers.value(PowerId::Flutter);
    let deck = state
        .card_states
        .hopper()
        .expect("structural Hopper sidecar");
    if state.history.over {
        // A lethal Hopper attack returns before the generic scheduler can
        // advance the machine or tick Escape Artist. Thievery has already
        // published Swipe; later HAT_TRICK/NAB attacks retain that same exact
        // stolen row and their current Flutter amount. No non-attacking row
        // can create a live-owner player-loss boundary.
        let stolen = owner.hopper_swipe_uid().is_some()
            && matches!(deck.history.as_slice(), [stolen]
                if stolen.kind == HopperLootKind::Stolen
                    && owner.hopper_swipe_uid() == Some(stolen.row.card.uid));
        let no_prior_theft = owner.hopper_swipe_uid().is_none() && deck.history.is_empty();
        let empty_theft = no_prior_theft
            && (owner.loop_pos != 0
                || [PileId::Draw, PileId::Discard].into_iter().all(|pile| {
                    state
                        .piles
                        .get(pile)
                        .as_slice()
                        .iter()
                        .all(|card| !deck.master.iter().any(|row| row.card.uid == card.uid))
                }));
        let terminal_loss = state.hp == 0 && (stolen || empty_theft);
        // #3823: a Whistle's stunned turns leave Escape Artist behind the
        // machine ([`hopper_machine_shape`]); an attack that kills the player
        // still runs undisturbed (a stunned Hopper performs nothing).
        return terminal_loss
            && hopper_escape_artist_is_reachable(escape_artist, owner.loop_pos, false)
            && owner.override_state == MonsterOverride::None
            && owner.forced_follow_up == MonsterFollowUp::None
            && match owner.loop_pos {
                0 => flutter == 0,
                2 | 3 => (0..=THIEVING_HOPPER_FLUTTER).contains(&flutter),
                _ => false,
            };
    }
    if !hopper_escape_artist_is_reachable(escape_artist, owner.loop_pos, false) {
        return false;
    }
    let swipe_is_live = owner.hopper_swipe_uid().is_some();
    let undisturbed = hopper_machine_shape(owner) == (MonsterOverride::None, MonsterFollowUp::None);
    match owner.loop_pos {
        0 => flutter == 0 && undisturbed && !swipe_is_live && deck.history.is_empty(),
        1 => flutter == 0 && undisturbed,
        // #3823: a Flutter break under a Whistle's stun drops its own stun,
        // so HAT_TRICK can show again with Flutter spent (native trace
        // `hop_brk_hat`).
        2 => match hopper_machine_shape(owner) {
            (MonsterOverride::None, MonsterFollowUp::None) => {
                (0..=THIEVING_HOPPER_FLUTTER).contains(&flutter)
            }
            (MonsterOverride::Stunned, MonsterFollowUp::HopperNab) => flutter == 0,
            _ => false,
        },
        3 | 4 => match hopper_machine_shape(owner) {
            (MonsterOverride::None, MonsterFollowUp::None) => {
                (0..=THIEVING_HOPPER_FLUTTER).contains(&flutter)
            }
            (MonsterOverride::Stunned, MonsterFollowUp::HopperEscape) => flutter == 0,
            _ => false,
        },
        _ => false,
    }
}

/// The sole non-stable Hopper state needed by the serial native action walk:
/// THIEVERY has removed exactly one master row and installed Swipe, but its
/// enclosing `monster_act` has not yet advanced loop zero to loop one.
pub(crate) fn thieving_hopper_internal_state_is_valid(state: &HotState) -> bool {
    if thieving_hopper_state_is_valid(state) {
        return true;
    }
    if !thieving_hopper_structural_state_is_valid(state) || state.history.over {
        return false;
    }
    let [owner] = state.monsters.as_slice() else {
        return false;
    };
    let Some(deck) = state.card_states.hopper() else {
        return false;
    };
    matches!(deck.history.as_slice(), [stolen]
        if stolen.kind == HopperLootKind::Stolen
            && owner.hopper_swipe_uid() == Some(stolen.row.card.uid))
        && owner.hp > 0
        && owner.loop_pos == 0
        && hopper_escape_artist_is_reachable(owner.powers.value(PowerId::EscapeArtist), 0, false)
        && owner.powers.value(PowerId::Flutter) == 0
        && owner.override_state == MonsterOverride::None
        && owner.forced_follow_up == MonsterFollowUp::None
}

/// Authenticate the native `SwipePower.BeforeDeath` entry without pretending
/// the dying owner is already a stable post-death root.
///
/// Most lethal paths start from a strict public Hopper state.  THIEVERY is the
/// one exception: it installs Swipe and removes the master row before its
/// attack can trigger retaliation, while the machine is still at loop zero.
/// This proof admits only that exact internal prefix and still requires the
/// owner to retain its death-removed Escape Artist/Flutter powers.
pub(crate) fn hopper_dying_entry_is_exact(state: &HotState, owner: usize) -> bool {
    let Some(dying) = state.monsters.get(owner) else {
        return false;
    };
    if owner != 0 || dying.kind != MonsterKind::ThievingHopper || dying.hp > 0 {
        return false;
    }
    let mut live = state.clone();
    live.monsters_mut()[owner].hp = 1;
    if !thieving_hopper_structural_state_is_valid(&live) {
        return false;
    }
    let owner = &live.monsters[owner];
    let escape_artist = owner.powers.value(PowerId::EscapeArtist);
    // An enemy-side Doom kill lands between the machine's advance and Escape
    // Artist's tick (#2965 lane, corpus `f581e517c74f6fdf`): `DoomPower`
    // kills the enemy side in `BeforeSideTurnEnd`
    // (`<BeforeSideTurnEnd>d__8::MoveNext` RVA `0x3391dc` IL_001d-IL_0047
    // skips only side 1 and then `DoomKill`s; its `AfterSideTurnEnd` twin
    // `0x3390f0` IL_001d-IL_0026 skips side 2), after every monster acted, while
    // `EscapeArtistPower::AfterSideTurnEnd` (RVA `0xa2238`) decrements only
    // afterwards. Flutter has no side-end hook (`FlutterPower` carries only
    // `ModifyDamageMultiplicative`, `AfterDamageReceived` and `StunnedMove`).
    // So on the enemy side the dying owner may carry the one un-ticked
    // Escape Artist stack, and every other field is the post-tick shape.
    // #3823: a Whistle's stunned turns open the gap between the two
    // ([`hopper_machine_shape`]), so this is a range, not a pair.
    if live.history.over
        || !hopper_escape_artist_is_reachable(
            escape_artist,
            owner.loop_pos,
            !live.player_side_active,
        )
    {
        return false;
    }
    match owner.loop_pos {
        0 => {
            owner.powers.value(PowerId::Flutter) == 0
                && hopper_machine_shape(owner) == (MonsterOverride::None, MonsterFollowUp::None)
        }
        1 => {
            owner.powers.value(PowerId::Flutter) == 0
                && hopper_machine_shape(owner) == (MonsterOverride::None, MonsterFollowUp::None)
        }
        2 => {
            (0..=THIEVING_HOPPER_FLUTTER).contains(&owner.powers.value(PowerId::Flutter))
                && matches!(
                    hopper_machine_shape(owner),
                    (MonsterOverride::None, MonsterFollowUp::None)
                        | (MonsterOverride::Stunned, MonsterFollowUp::HopperNab)
                )
        }
        3 | 4 => {
            (0..=THIEVING_HOPPER_FLUTTER).contains(&owner.powers.value(PowerId::Flutter))
                && matches!(
                    hopper_machine_shape(owner),
                    (MonsterOverride::None, MonsterFollowUp::None)
                        | (MonsterOverride::Stunned, MonsterFollowUp::HopperEscape)
                )
        }
        _ => false,
    }
}

fn hopper_theft_priority(
    catalog: &crate::catalog::Catalog,
    card: crate::hot::HotCard,
) -> Result<Option<usize>, EngineRefusal> {
    let spec = catalog
        .spec(card.atom)
        .ok_or(EngineRefusal::UnknownAtom(card.atom))?;
    let imbued = spec
        .identity
        .enchantment
        .is_some_and(|enchantment| enchantment.id == EnchantmentId::Imbued);
    Ok(match spec.row.rarity {
        CardRarity::Uncommon if !imbued => Some(0),
        CardRarity::Common | CardRarity::Rare | CardRarity::Event if !imbued => Some(1),
        CardRarity::Basic | CardRarity::Curse if !imbued => Some(2),
        CardRarity::Ancient if !imbued => Some(3),
        _ if imbued => Some(3),
        CardRarity::Status | CardRarity::Token | CardRarity::Quest => None,
        _ => None,
    })
}

/// Native Hopper Thievery over Draw then Discard, including one Generation
/// `NextItem` even for a singleton and exact DeckVersion removal.
pub(crate) fn hopper_steal_one_card(
    state: &mut HotState,
    catalog: &crate::catalog::Catalog,
    owner: usize,
) -> Result<(), EngineRefusal> {
    if owner != 0
        || !thieving_hopper_state_is_valid(state)
        || !thieving_hopper_deck_payload_is_exact(state, catalog)
        || state.monsters[owner].hp <= 0
        || state.monsters[owner].hopper_swipe_uid().is_some()
    {
        return Err(EngineRefusal::MalformedArgs("Thieving Hopper theft entry"));
    }
    let deck = state
        .card_states
        .hopper()
        .ok_or(EngineRefusal::MalformedArgs("Thieving Hopper DeckVersion"))?;
    let mut eligible = Vec::new();
    let mut priorities: [Vec<usize>; 4] = std::array::from_fn(|_| Vec::new());
    for pile in [PileId::Draw, PileId::Discard] {
        for (index, card) in state.piles.get(pile).as_slice().iter().copied().enumerate() {
            if !deck.master.iter().any(|row| row.card.uid == card.uid) {
                continue;
            }
            let eligible_index = eligible.len();
            eligible.push((pile, index, card));
            if let Some(priority) = hopper_theft_priority(catalog, card)? {
                priorities[priority].push(eligible_index);
            }
        }
    }
    let pool: &[usize] = priorities
        .iter()
        .find(|pool| !pool.is_empty())
        .map(Vec::as_slice)
        .unwrap_or_else(|| {
            // Indices are generated below solely for the fallback. Keep the
            // allocation out of the common priority path by using this empty
            // marker and handling it explicitly.
            &[]
        });
    if eligible.is_empty() {
        return Ok(());
    }
    let live = state.rng.get(RngStream::Generation);
    if live.words == [0; 4] {
        return Err(EngineRefusal::MalformedArgs(
            "Thieving Hopper Generation stream",
        ));
    }
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let bound = if pool.is_empty() {
        eligible.len()
    } else {
        pool.len()
    };
    let offset: usize = rng
        .next_bounded(
            i32::try_from(bound)
                .map_err(|_| EngineRefusal::CounterOverflow("Hopper theft pool"))?,
        )
        .map_err(|_| EngineRefusal::CounterOverflow("Hopper theft RNG"))?
        .try_into()
        .map_err(|_| EngineRefusal::CounterOverflow("Hopper theft result"))?;
    let selected_index = if pool.is_empty() {
        offset
    } else {
        pool[offset]
    };
    let (pile, pile_index, selected) = eligible[selected_index];
    let spec = catalog
        .spec(selected.atom)
        .ok_or(EngineRefusal::UnknownAtom(selected.atom))?;
    if spec.identity.id == CardId::SpoilsMap {
        return Err(EngineRefusal::MalformedArgs(
            "Hopper Spoils Map BeforeCardRemoved",
        ));
    }
    let master_index = deck
        .master
        .iter()
        .position(|row| row.card.uid == selected.uid)
        .ok_or(EngineRefusal::MalformedArgs(
            "Hopper selected DeckVersion row",
        ))?;
    let row = deck.master[master_index].clone();
    if state.piles.get(pile).as_slice().get(pile_index) != Some(&selected) {
        return Err(EngineRefusal::MalformedArgs("Hopper theft pile drift"));
    }
    let mut next_deck = deck.clone();
    next_deck.master.remove(master_index);
    next_deck.history.push(crate::hot::HopperLootEvent {
        kind: HopperLootKind::Stolen,
        row: row.clone(),
    });
    state.rng.set(
        RngStream::Generation,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    state.piles.get_mut(pile).make_mut().remove(pile_index);
    state.card_states.set(selected.uid, Default::default());
    state.card_states.set_hopper(Some(next_deck));
    if !state.monsters_mut()[owner].set_hopper_swipe_uid(Some(row.card.uid)) {
        unreachable!("validated Hopper owner and non-sentinel uid")
    }
    Ok(())
}

/// SwipePower.BeforeDeath restores the exact stolen master row before ordinary
/// Hopper power cleanup, MonsterDied publication, and every AfterDeath group.
pub(crate) fn hopper_return_stolen_card(
    state: &mut HotState,
    owner: usize,
) -> Result<(), EngineRefusal> {
    if !hopper_dying_entry_is_exact(state, owner) {
        return Err(EngineRefusal::MalformedArgs("Hopper Swipe death entry"));
    }
    let Some(uid) = state.monsters[owner].hopper_swipe_uid() else {
        return Ok(());
    };
    let deck = state
        .card_states
        .hopper()
        .ok_or(EngineRefusal::MalformedArgs("Hopper Swipe DeckVersion"))?;
    let [stolen] = deck.history.as_slice() else {
        return Err(EngineRefusal::MalformedArgs("Hopper Swipe history"));
    };
    if stolen.kind != HopperLootKind::Stolen
        || stolen.row.card.uid != uid
        || deck.master.iter().any(|row| row.card.uid == uid)
    {
        return Err(EngineRefusal::MalformedArgs("Hopper Swipe row"));
    }
    let mut next_deck = deck.clone();
    next_deck.master.push(stolen.row.clone());
    next_deck.history.push(crate::hot::HopperLootEvent {
        kind: HopperLootKind::Returned,
        row: stolen.row.clone(),
    });
    state.card_states.set_hopper(Some(next_deck));
    if !state.monsters_mut()[owner].set_hopper_swipe_uid(None) {
        unreachable!("validated Hopper owner")
    }
    Ok(())
}

fn queen_roster_shape_is_exact(state: &HotState) -> bool {
    let [amalgam, queen] = state.monsters.as_slice() else {
        return false;
    };
    amalgam.kind == MonsterKind::TorchHeadAmalgam
        && (amalgam.slot, amalgam.uid, Some(amalgam.max_hp))
            == (0, 0, native_fixed_hp(state, MonsterKind::TorchHeadAmalgam))
        && amalgam.hp <= amalgam.max_hp
        && (0..=MONSTER_BLOCK_CAP).contains(&amalgam.block)
        && matches!(amalgam.loop_pos, 0..=4)
        && amalgam.random_ai.is_empty()
        && !amalgam.spawn_noop
        && amalgam.revive_stage == 0
        && amalgam
            .powers
            .get(PowerId::Secondary)
            .is_some_and(|slot| slot.wire == SlotWire::Bool && slot.value == 1)
        && amalgam.pressure_buildup_idx() == 0
        && !amalgam.is_about_to_blow()
        && queen.kind == MonsterKind::Queen
        && (queen.slot, queen.uid, Some(queen.max_hp))
            == (1, 1, native_fixed_hp(state, MonsterKind::Queen))
        && queen.hp <= queen.max_hp
        && (0..=MONSTER_BLOCK_CAP).contains(&queen.block)
        && matches!(queen.loop_pos, 0..=5)
        && queen.random_ai.is_empty()
        && !queen.spawn_noop
        && queen.revive_stage == 0
        && queen.queen_private_state_is_exact()
        && queen.powers.value(PowerId::Secondary) == 0
}

pub(crate) fn queen_amalgam_follow_up(kind: MonsterKind, loop_pos: i32) -> Option<MonsterFollowUp> {
    match (kind, loop_pos) {
        (MonsterKind::Queen, 0) => Some(MonsterFollowUp::QueenPuppetStrings),
        (MonsterKind::Queen, 1) => Some(MonsterFollowUp::QueenYoureMine),
        (MonsterKind::Queen, 2) => Some(MonsterFollowUp::QueenBurnBright),
        (MonsterKind::Queen, 3) => Some(MonsterFollowUp::QueenOffWithYourHead),
        (MonsterKind::Queen, 4) => Some(MonsterFollowUp::QueenExecution),
        (MonsterKind::Queen, 5) => Some(MonsterFollowUp::QueenEnrage),
        (MonsterKind::TorchHeadAmalgam, 0) => Some(MonsterFollowUp::AmalgamStrongTackle),
        (MonsterKind::TorchHeadAmalgam, 1) => Some(MonsterFollowUp::AmalgamTackleTwo),
        (MonsterKind::TorchHeadAmalgam, 2) => Some(MonsterFollowUp::AmalgamBeam),
        (MonsterKind::TorchHeadAmalgam, 3) => Some(MonsterFollowUp::AmalgamTackleThree),
        (MonsterKind::TorchHeadAmalgam, 4) => Some(MonsterFollowUp::AmalgamTackleFour),
        _ => None,
    }
}

fn queen_forced_state_is_exact(monster: &HotMonster) -> bool {
    match (monster.override_state, monster.forced_follow_up) {
        (MonsterOverride::None, MonsterFollowUp::None) => true,
        (MonsterOverride::Stunned, follow_up) => {
            monster.hp > 0
                && queen_amalgam_follow_up(monster.kind, monster.loop_pos) == Some(follow_up)
        }
        _ => false,
    }
}

pub(crate) fn queen_amalgam_stun_state_is_exact(state: &HotState, owner: usize) -> bool {
    queen_roster_state_is_valid(state)
        && state.monsters.get(owner).is_some_and(|monster| {
            matches!(
                monster.kind,
                MonsterKind::Queen | MonsterKind::TorchHeadAmalgam
            ) && monster.hp > 0
                && monster.override_state == MonsterOverride::Stunned
                && queen_amalgam_follow_up(monster.kind, monster.loop_pos)
                    == Some(monster.forced_follow_up)
        })
}

/// Does this owner carry `ImbalancedPower`?
///
/// One predicate for every reader of the power, so no call site has to know
/// which of the two representations a given carrier uses.
///
/// `ImbalancedPower::get_Type` `0xa3bd3` and `get_StackType` `0xa3bd6` both
/// return `2`, so the power is a concrete Type-2 listener outside the
/// represented Misery scalar vocabulary and has no scalar amount to project.
///
/// **Two representations, deliberately.**
///
/// * `BowlbugRock` carries it **by kind**. A whole-assembly IL census finds
///   exactly one native writer of the power,
///   `BowlbugRock/<AfterAddedToRoom>d__22::MoveNext` `0x353e30`
///   IL_007f-IL_0097, which calls `PowerCmd.Apply<ImbalancedPower>` on the
///   Rock's own `Creature` with `System.Decimal::One`. Every Rock therefore
///   has it, always, with amount 1 — so representing it by kind is exact and
///   costs no wire field, and #2693 B1 deliberately does **not** migrate it.
///   Legacy byte-identity for every existing Bowlbug document is the hard
///   bar, and a Rock that also claimed a ledger row would double-count at
///   `rend_target_power_count`, so admission refuses that combination.
/// * Any other creature carries it as a **physical ledger attachment**
///   ([`crate::hot::AttachedPowerModel::Imbalanced`]). The only native route
///   onto a non-Bowlbug owner is a `Misery` clone, which is PR B2; B1 makes
///   the state representable so the listener's generic arm is reachable.
///
/// The native instance is a singleton either way — neither `ImbalancedPower`
/// nor its base overrides `PowerModel::get_InstanceType` `0x83751` = 0 — so a
/// creature carries at most one, which admission enforces and this predicate
/// is free to answer as a bare boolean. That is what keeps it matching frozen
/// `count_target_powers_for_rend` (frozen Python, deleted #2827) ("unique ImbalancedPower"), which
/// contributes cardinality 1 per carrier and never one per carrier test.
pub(crate) fn owner_carries_imbalanced(monster: &HotMonster) -> bool {
    monster.kind == MonsterKind::BowlbugRock
        || monster
            .misery_debuff_order
            .attachments()
            .any(|record| record.power == crate::hot::AttachedPowerModel::Imbalanced)
}

/// The attachment position of this owner's `ImbalancedPower` row, if it has
/// one.
///
/// A Bowlbug Rock carrying the intrinsic amount-1 instance has **no** row —
/// that is the by-kind representation [`owner_carries_imbalanced`] documents.
/// It grows one exactly when a `Misery` clone stacks the instance past 1
/// (#2647 B2), because the amount then stops being derivable from the kind.
pub(crate) fn imbalanced_attachment(monster: &HotMonster) -> Option<usize> {
    monster
        .misery_debuff_order
        .attachments()
        .position(|record| record.power == crate::hot::AttachedPowerModel::Imbalanced)
}

/// This owner's live `ImbalancedPower.Amount`, or `None` when it carries none.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `BowlbugRock/<AfterAddedToRoom>d__22::MoveNext` `0x353e30` IL_007f-IL_0097
/// applies `System.Decimal::One`, and a whole-assembly census finds no other
/// writer, so an un-stacked Rock is always exactly 1. The only way any
/// instance moves off its applied amount is `PowerCmd::ModifyAmount`
/// `0x3f032c` reached from `Misery/<OnPlay>d__3::MoveNext` `0x3ad358`
/// IL_0274-IL_02bd, which is what a row records.
pub(crate) fn imbalanced_amount(monster: &HotMonster) -> Option<i32> {
    if let Some(index) = imbalanced_attachment(monster) {
        return monster
            .misery_debuff_order
            .attachments()
            .get(index)
            .map(|record| record.amount);
    }
    (monster.kind == MonsterKind::BowlbugRock).then_some(1)
}

/// The ledger position of this monster's physically attached
/// `StrengthPower`, or `None` when its provenance was never recorded.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`:
/// `StrengthPower` declares exactly four members (`get_Type` `0xa8943` = 1,
/// `get_StackType` `0xa8946` = 1, `get_AllowNegative` `0xa8949` = true,
/// `ModifyDamageAdditive` `0xa894c`) and overrides neither
/// `PowerModel::get_InstanceType` `0x83751` (= 0) nor
/// `get_IsVisibleInternal`. It is therefore a **singleton** per creature, so
/// the lookup is by model alone — never by applier — exactly as
/// `PowerCmd::FindExistingInstanceForStacking` `0x1338d8` IL_0058 resolves it.
///
/// `None` is a real state, not an error: it means the scalar's provenance was
/// not recorded, either because the fight has no `Misery` to read it
/// ([`crate::hot::HotFanouts::misery_attachment_upkeep`]) or because the
/// document is a legacy checkpoint written before #2693 S1. Every reader
/// refuses on it exactly where it refused before — see
/// [`crate::engine::damage::misery_scalar_state_is_exact`].
pub(crate) fn strength_attachment(monster: &HotMonster) -> Option<usize> {
    monster
        .misery_debuff_order
        .attachments()
        .position(|record| record.power == crate::hot::AttachedPowerModel::Strength)
}

/// This monster's recorded `StrengthPower` attachment, if it has one.
pub(crate) fn strength_attachment_record(
    monster: &HotMonster,
) -> Option<crate::hot::AttachmentRecord> {
    strength_attachment(monster).and_then(|index| {
        monster
            .misery_debuff_order
            .attachments()
            .get(index)
            .copied()
    })
}

/// Who applied this monster's live `StrengthPower`, if that was recorded.
pub(crate) fn strength_applier(monster: &HotMonster) -> Option<crate::hot::Applier> {
    strength_attachment_record(monster).map(|record| record.applier)
}

/// Does this monster's `PowerId::Strength` scalar have recorded provenance?
///
/// `true` with no row means the scalar is zero, so there is no native
/// instance to place. `true` with a row means the row mirrors the scalar, so
/// position and applier are known for the instance the scalar describes.
/// `false` is the *unrecorded* state — a nonzero scalar with no row, or a row
/// that disagrees with it.
///
/// A nonzero scalar with no row is exactly what #2693 S1 left behind and
/// #2693 S4 closes wherever the ledger makes the position uniquely
/// determined: see
/// [`crate::engine::damage::materialize_entering_strength_provenance`]. Where
/// it is NOT determined the scalar stays unrecorded, and #2693 S4b refuses
/// that root at ADMISSION — but only where a reducer is reachable, so a
/// `Misery` can actually come to select it; see
/// `"unplaceable entering Strength beside a reachable reducer"` in
/// [`crate::engine::admission`]. Refusing every such root unconditionally was
/// drafted, measured and rejected: it flipped three then-admitted census
/// roots.
pub(crate) fn strength_provenance_is_recorded(monster: &HotMonster) -> bool {
    let strength = monster.powers.value(PowerId::Strength);
    match strength_attachment_record(monster) {
        None => strength == 0,
        Some(record) => record.amount == strength,
    }
}

/// Is this monster's entering `StrengthPower` at a uniquely determined ledger
/// position (#2693 S4)?
///
/// The ledger records the relative order of everything `Misery` can read. An
/// instance whose provenance was never recorded has a *knowable* position
/// exactly when there is nothing recorded for it to be before or after:
///
/// * no acquisition token and no Knockdown amount — a scalar debuff the
///   monster already carries is a physical `Creature.Powers` entry too, and
///   which of the two attached first is not in the document;
/// * no attachment row, for the same reason;
/// * no by-kind `ImbalancedPower` — a Bowlbug Rock's intrinsic instance is
///   recorded by kind rather than by row
///   ([`imbalanced_amount`]), and
///   [`crate::engine::damage::misery_snapshot`] replays it at position 0, so
///   an entering Strength would have to be placed relative to an instance
///   the document does not order it against.
pub(crate) fn entering_strength_position_is_determined(monster: &HotMonster) -> bool {
    monster.misery_debuff_order.as_slice().is_empty()
        && monster.misery_debuff_order.knockdown().is_empty()
        && monster.misery_debuff_order.attachments().is_empty()
        && imbalanced_amount(monster).is_none()
}

/// This monster's concrete temporary-Strength wrapper rows, in
/// `Creature.Powers` order (#2693 S2).
///
/// Yields `(ledger position, record)`. The position is the index into
/// [`crate::hot::MiseryOrder::attachments`], which is what
/// [`crate::hot::MiseryOrder::remove_attachment`] and
/// [`crate::hot::MiseryOrder::set_attachment_amount`] take.
pub(crate) fn temp_strength_wrapper_rows(
    monster: &HotMonster,
) -> impl Iterator<Item = (usize, crate::hot::AttachmentRecord)> + '_ {
    monster
        .misery_debuff_order
        .attachments()
        .enumerate()
        .filter(|(_, record)| record.power.is_temporary_strength_wrapper())
        .map(|(index, record)| (index, *record))
}

/// The ledger position of this monster's instance of one concrete
/// temporary-Strength wrapper model, or `None` when it carries none.
///
/// Current-build authority is `sts2.dll` SHA-256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`: every
/// member of the family takes the base `PowerModel::get_InstanceType`
/// `0x83751` = 0, so `PowerCmd::FindExistingInstanceForStacking` `0x1338d8`
/// resolves it at IL_0058 through `Creature::GetPower(Id)` — **by model id
/// alone**. The applier is deliberately not part of this lookup; passing it
/// would be the InstanceType-2 shape
/// ([`crate::hot::MiseryOrder::find_attachment`]), which no member of this
/// vocabulary has.
pub(crate) fn temp_strength_wrapper_attachment(
    monster: &HotMonster,
    model: crate::hot::AttachedPowerModel,
) -> Option<usize> {
    debug_assert!(model.is_temporary_strength_wrapper());
    monster
        .misery_debuff_order
        .attachments()
        .position(|record| record.power == model)
}

/// The summed native `Amount` of this monster's temporary-Strength wrapper
/// rows, as an `i64` so a malformed document cannot overflow the check that
/// reads it.
///
/// The rows carry POSITIVE native amounts
/// ([`crate::hot::AttachedPowerModel::is_temporary_strength_wrapper`]), so
/// the aggregate `PowerId::TempStrength` scalar they must mirror is the
/// negation of this sum.
pub(crate) fn temp_strength_wrapper_total(monster: &HotMonster) -> i64 {
    temp_strength_wrapper_rows(monster)
        .map(|(_, record)| i64::from(record.amount))
        .sum()
}

/// Do this monster's temporary-Strength wrapper rows account for its whole
/// aggregate scalar?
///
/// `true` with no rows means the scalar is zero; `true` with rows means every
/// point of it has recorded provenance. `false` is the *unrecorded* state —
/// a legacy checkpoint, a fight that cannot reach `Misery`, or a writer that
/// gave up rather than inventing a position — and it is what
/// [`crate::engine::damage::misery_scalar_state_is_exact`] keeps refusing
/// until #2693 S3 reads the fold.
pub(crate) fn temp_strength_provenance_is_recorded(monster: &HotMonster) -> bool {
    temp_strength_wrapper_total(monster) == -i64::from(monster.powers.value(PowerId::TempStrength))
}

/// May a `Misery` clone of `ImbalancedPower` land on this owner?
///
/// Two disjoint reasons an owner is representable, and nothing else is:
///
/// * a `BowlbugRock` — `ImbalancedPower/<AfterDamageGiven>d__4::MoveNext`
///   `0x33cd60` IL_0056's `isinst BowlbugRock` succeeds, so the listener sets
///   the `_isOffBalance` latch that #2662 and slice A already model, and the
///   clone only moves the amount (see [`imbalanced_amount`]);
/// * one of [`IMBALANCED_GENERIC_CARRIER_KINDS`] whose generic stun is
///   representable at **every** position its loop can reach
///   ([`generic_stun_owner_state_is_representable`]), because
///   IL_0068-IL_006f then awaits `CreatureCmd::Stun(Owner, null)`;
/// * a `SlumberingBeetle` (#2647), whose only dealer path is its awake
///   ROLL_OUT_MOVE and whose parked telegraph there is determined
///   ([`beetle_rollout_stun_is_exact`]). Asleep it deals nothing, so the clone
///   is inert until it wakes; the root is admitted for the whole fight, and
///   every state it can then reach is one [`install_imbalanced_self_stun`]
///   writes.
///
/// Thorns is refused on top of both, for the reason
/// [`crate::engine::damage::refuse_imbalanced_thorns_owner`] carries: a
/// retaliation makes the owner a dealer inside *another* creature's attack,
/// where #2647 open question 2 leaves `StateLog.Last()` unestablished for the
/// generic arm and the collapsed latch observable for the Rock.
pub(crate) fn imbalanced_clone_recipient_is_representable(
    catalog: &Catalog,
    monster: &HotMonster,
) -> bool {
    monster.powers.value(PowerId::Thorns) == 0
        && (monster.kind == MonsterKind::BowlbugRock
            || monster.kind == MonsterKind::SlumberingBeetle
            || (IMBALANCED_GENERIC_CARRIER_KINDS.contains(&monster.kind)
                && generic_stun_owner_state_is_representable(catalog, monster)))
}

/// The generic stun owners a `Misery` clone of `ImbalancedPower` may land on.
///
/// Until #3823 this gate read the whole generic stun owner set, which was
/// these five kinds. #3823 widens that set to every plain-cycle machine for
/// Whistle's `CreatureCmd::Stun(target, null)`; the clone gate deliberately
/// does not follow. A carrier's stun is installed inside its **own** attack
/// (`ImbalancedPower/<AfterDamageGiven>d__4::MoveNext` `0x33cd60`
/// IL_0020-IL_006f), where what is left of a multi-hit move and the other
/// `AfterDamageGiven` listeners on the owner are part of the reading, and
/// those were established for these kinds only (#2647, #3258). The only
/// native source of the power is a Bowlbug Rock
/// (`BowlbugRock/<AfterAddedToRoom>d__22::MoveNext` `0x353e30`), which shares
/// an encounter with the three Bowlbug workers and the Slumbering Beetle and
/// with nothing else, so no root loses anything to the narrower list.
const IMBALANCED_GENERIC_CARRIER_KINDS: [MonsterKind; 5] = [
    MonsterKind::BowlbugEgg,
    MonsterKind::BowlbugNectar,
    MonsterKind::BowlbugSilk,
    MonsterKind::MechaKnight,
    MonsterKind::OwlMagistrate,
];

/// The telegraph a generic `CreatureCmd::Stun(creature, null)` parks.
///
/// `Creature::StunInternal` RVA `0x11d7cc` IL_0031–IL_0055: a null
/// `nextMoveId` becomes `MoveStateMachine.StateLog.Last().Id`. The dynamic
/// `STUNNED` state is installed with `ForceCurrentState`
/// (`MonsterModel::SetMoveImmediate` `0x825f0` -> `0x78e7f` -> `0x78f6b`),
/// which never appends to `StateLog`, so `StateLog.Last()` at stun time is
/// still the running or telegraphed move — i.e. exactly the row `loop_pos`
/// names.
///
/// `None` — a refusal, never a guess — when there is no row to name:
///
/// * the kind has no generated loop at all (a kind whose executable table is
///   the private native loop), so `loop_pos` is not what carries its
///   telegraph. A **random table** in
///   [`GENERIC_STUN_RANDOM_TABLE_KINDS`] answers
///   [`MonsterFollowUp::ParkedTelegraph`] for `loop_pos` 0: its telegraph is
///   the row `random_ai.next` names, which the document carries
///   (`next_move`), and the stun moves neither;
/// * `loop_pos` is out of range for that loop;
/// * for a **named** loop only, the name occurs more than once in it, so the
///   parked id would not identify one row
///   (`no_generated_loop_repeats_a_move_name` pins that none does).
///
/// **Which token names it (#3823).** A kind whose every row is in the named
/// vocabulary keeps the row's name, as before: the Queen roster, the Bowlbug
/// workers, Corpse Slug, Mecha Knight and Owl Magistrate, pinned by
/// `the_named_parked_telegraph_kinds_are_pinned`. Every other loop answers
/// [`MonsterFollowUp::ParkedTelegraph`] for an in-range `loop_pos`: the
/// follow-up is then "the row `loop_pos` names", which the document already
/// carries (the generic resume never maps a name back to an index; it leaves
/// `loop_pos` where it is).
///
/// That `StateLog.Last()` is the telegraphed `MoveState` for **every** machine
/// shape, not only a plain cycle, is IL: `FindNextMoveState` `0x78e94`
/// IL_009c-IL_00d6 logs the first state of the walk whose
/// `ShouldAppearInLogs` holds, and both branch states answer false
/// (`RandomBranchState::get_ShouldAppearInLogs` `0x79110`,
/// `ConditionalBranchState::get_ShouldAppearInLogs` `0x78d8f`), so the walk
/// logs the `MoveState` it lands on. The one way the two part company is a
/// state installed by `SetMoveImmediate`/`ForceCurrentState`, which is never
/// logged; every kind with such a writer is in
/// [`GENERIC_STUN_BESPOKE_KINDS`].
pub(crate) fn parked_telegraph(kind: MonsterKind, loop_pos: i32) -> Option<MonsterFollowUp> {
    if crate::content_tables::random_moves(kind).is_some() {
        // A random table (#3823): the telegraph is the row `random_ai.next`
        // names, and `loop_pos` is always 0 (`valid_random_ai_state`).
        return (GENERIC_STUN_RANDOM_TABLE_KINDS.contains(&kind) && loop_pos == 0)
            .then_some(MonsterFollowUp::ParkedTelegraph);
    }
    let rows = crate::content_tables::monster_loop(kind)?;
    if loop_rows_are_all_named(rows) {
        return parked_telegraph_in(rows, loop_pos);
    }
    let index = usize::try_from(loop_pos).ok()?;
    rows.get(index).map(|_| MonsterFollowUp::ParkedTelegraph)
}

/// Does every row of this loop resolve to its own named [`MonsterFollowUp`]?
fn loop_rows_are_all_named(rows: &'static [crate::content_tables::Move]) -> bool {
    (0..rows.len()).all(|index| {
        parked_telegraph_in(rows, index as i32)
            .is_some_and(|follow_up| follow_up != MonsterFollowUp::ParkedTelegraph)
    })
}

/// [`parked_telegraph`] over an explicit loop.
///
/// Split out because the uniqueness arm is not reachable from the v0.111.0
/// generated tables — `no_generated_loop_repeats_a_move_name` pins that — and a
/// forward guard with no witness is how #2700's lesson gets relearned. This
/// takes the rows, so the arm is exercised directly.
fn parked_telegraph_in(
    rows: &'static [crate::content_tables::Move],
    loop_pos: i32,
) -> Option<MonsterFollowUp> {
    let index = usize::try_from(loop_pos).ok()?;
    let name = rows.get(index)?.name;
    if rows.iter().filter(|row| row.name == name).count() != 1 {
        return None;
    }
    MonsterFollowUp::from_str(name)
}

/// Kinds whose stun state is NOT the generic parked-telegraph shape.
///
/// Every one of these already owns a bespoke arm in `monster_act` or a
/// bespoke `valid_override` disjunct, so admitting them generically would
/// give one wire state two readers.
///
/// #3823 added the kinds whose native machine has a **second writer of the
/// move** besides the roll, found by scanning the whole v0.111.0 assembly for
/// callers of `MonsterModel::SetMoveImmediate` `0x825f0`,
/// `MonsterMoveStateMachine::ForceCurrentState` `0x78e7f`,
/// `CreatureCmd::Stun` `0x132b2c` and `get_StateLog` `0x78d9a`, and for
/// `set_MustPerformOnceBeforeTransitioning` inside a
/// `GenerateMoveStateMachine`. Such a state is installed without being
/// logged, so `StateLog.Last()` no longer names the move the monster is
/// showing, or it refuses to be left before it has performed, so
/// `SetMoveImmediate(state, force: false)` drops the stun
/// (`0x825f0` IL_000c-IL_001c). Each needs its own reading, and what is left
/// here is the Queen roster (its own two-kind table,
/// [`queen_amalgam_follow_up`]).
///
/// `DecimillipedeSegment` left this list (#3823). A living segment's machine
/// is the WRITHE, CONSTRICT, BULK cycle (`GenerateMoveStateMachine`
/// `0xb2c04`), so a stun parks its row. Its second writer is
/// `ReattachPower/<AfterDeath>d__11::MoveNext` `0x341d30` IL_0086-IL_008c,
/// `SetMoveImmediate(DeadState, force: false)`, which a standing stun drops:
/// the segment killed under a stun acts dead ([`SEGMENT_REVIVE_STUN_KEPT`]).
/// A dead segment (DEAD, REATTACH) is a corpse to `StunInternal` `0x11d7cc`
/// (IL_0028), and the RAND roll after REATTACH is resolved before the player
/// acts, so neither is ever parked.
///
/// `TerrorEel` left this list (#3823). Its own stun is ShriekPower's
/// `CreatureCmd.Stun(Owner, TerrorState.StateId)`
/// (`ShriekPower/<AfterDamageReceived>d__10::MoveNext` `0x344348`
/// IL_0065-IL_008b), carried as `STUNNED` with no follow-up (the Terror
/// chain, #2647 §5.7); a Whistle's parks the loop row as
/// [`MonsterFollowUp::ParkedTelegraph`] or a named row, never with no
/// follow-up, so the two wire states stay apart. What each does to the other
/// is at [`install_generic_stun`] and the Shriek break in
/// `damage::damage_monster`.
///
/// `WaterfallGiant` and `TestSubject` left this list (#3823). Their second
/// writers are death writers, `WaterfallGiant/<TriggerAboutToBlowState>d__77::
/// MoveNext` `0x3762a8` IL_00a9-IL_00b1 and `TestSubject/<TriggerDeadState>
/// d__67::MoveNext` `0x36e658` IL_00c4-IL_00c5, and both call
/// `SetMoveImmediate(state, force: true)`, so each replaces a standing stun
/// (the engine's death cleanup clears the override before it installs
/// ABOUT_TO_BLOW or the retained corpse). ABOUT_TO_BLOW_MOVE is must-perform
/// (`WaterfallGiant::GenerateMoveStateMachine` `0xc4eb0` IL_0169), so a
/// Whistle on it is dropped ([`install_generic_stun`]); EXPLODE_MOVE follows
/// itself (IL_01e2-IL_01e6) and parks like any row. RESPAWN_MOVE is shown by
/// a creature the game counts dead, so a Whistle on it returns at
/// `StunInternal` `0x11d7cc` IL_0028 (the corpse arm). Native traces
/// `wfg_s2k2`, `wfg_k2s2`, `wfg_k2s3`, `tsj_s1k1`, `tsj_k1s1`.
///
/// `EyeWithTeeth` and `Parafright` left this list (#3823). Their second
/// writer is `IllusionPower/<AfterDeath>d__18::MoveNext` `0x33ca54`
/// IL_00b3-IL_011b, which builds REVIVE_MOVE and installs it with
/// `SetMoveImmediate(state, force: false)`. It runs only at the illusion's
/// death, when no stun can land any more (`StunInternal` `0x11d7cc` IL_0028),
/// so a stun on a living illusion parks its one-row loop like any other. What
/// the writer does to a stun already standing is the dead-acting illusion
/// ([`ILLUSION_REVIVE_DROPPED`]).
///
/// `Tunneler` left this list (#3823). Its own stun is the burrow break's
/// DIZZY (`BurrowedPower/<AfterBlockBroken>d__5::MoveNext` `0x33622c`
/// IL_00d6-IL_00db, follow-up BITE), carried as `MonsterOverride::Dizzy`, a
/// wire state apart from `STUNNED`. A Whistle parks its loop (BITE, BURROW,
/// then BELOW following to itself); a Whistle on DIZZY is dropped
/// ([`install_generic_stun`]), and a break under a Whistle's stun drops the
/// DIZZY but still removes Burrowed (`damage::after_monster_block_broken`).
///
/// `ThievingHopper` left this list (#3823). Its own stun is FlutterPower's
/// break, `Stun(owner, ...)` with a named follow-up (NAB over HAT_TRICK,
/// ESCAPE over NAB or ESCAPE), which the generic `PARKED_TELEGRAPH` never
/// names, so the two wire states stay apart. A Whistle parks its plain chain
/// (`ThievingHopper::GenerateMoveStateMachine` `0xc21e0` IL_00e7-IL_010a:
/// THIEVERY, FLUTTER, HAT_TRICK, NAB, then ESCAPE following to itself); what
/// that does to Escape Artist is [`hopper_machine_shape`], and what each stun
/// does to the other is at [`install_generic_stun`] and the Flutter break in
/// `damage::damage_monster`.
///
/// `BowlbugRock`, `CorpseSlug` and `CeremonialBeast` left this list (#3823).
/// Each one's own stun is a separate wire state (the Rock's DIZZY,
/// RAVENOUS_STUN_MOVE, STUN_MOVE), installed through `StunInternal` like a
/// Whistle's, so whichever lands second is dropped
/// ([`own_stun_is_unperformed`], and the Ravenous, Plow-break writers in
/// `damage`). A Whistle parks their generated loops as any other: HEADBUTT;
/// WHIP_SLAP, GLOMP, GOOP; STAMP, PLOW, BEAST_CRY, STOMP, CRUSH
/// (`CeremonialBeast::GenerateMoveStateMachine` `0xb0cfc` IL_0121-IL_0159:
/// PLOW follows itself, so a Plow break under a Whistle leaves the Beast on
/// PLOW for good, [`ceremonial_beast_state_is_valid`]).
///
/// `LagavulinMatriarch` and `SlumberingBeetle` left this list (#3823). Their
/// sleep move (SLEEP_MOVE, SNORE_MOVE) is not a generated loop row: a stun
/// parks it as [`MonsterFollowUp::LagavulinSleep`] /
/// [`MonsterFollowUp::BeetleSnore`] ([`lagavulin_sleep_is_parked`],
/// [`beetle_snore_is_parked`]), and their wake by damage is their own stun.
/// The Beetle has no generated loop, so it stays outside
/// [`generic_stun_owner_is_eligible`]; [`install_beetle_stun`] is its stun.
///
/// `ToughEgg` left this list (#3823). Its one second writer,
/// `ToughEgg/<AfterAddedToRoom>d__33::MoveNext` `0x3724a8` IL_01c6-IL_01cb
/// (`ForceCurrentState(AfterHatchedState)`), sits behind `get_IsHatched`
/// (IL_00c6-IL_00cb): it runs only for an egg that is already hatched when it
/// is added. `set_IsHatched` `0xc30ac` has one caller in the assembly,
/// `<HatchMove>d__35::MoveNext` `0x372874` IL_002e, and the only egg a fight
/// adds is the fresh one `Ovicopter/<LayEggsMove>` makes, so that branch is
/// not reached in combat. What is left is a two-row loop (HATCH_MOVE follows
/// to NIBBLE_MOVE, NIBBLE_MOVE to itself: `GenerateMoveStateMachine`
/// `0xc3160` IL_0061-IL_006a) that takes the generic stun.
/// ABOUT_TO_BLOW_MOVE's row in the Waterfall Giant's generated loop, which
/// its death writes (`damage::finish_monster_death`).
pub(crate) const WATERFALL_ABOUT_TO_BLOW_INDEX: i32 = 6;

const GENERIC_STUN_BESPOKE_KINDS: [MonsterKind; 2] =
    [MonsterKind::Queen, MonsterKind::TorchHeadAmalgam];

/// Random-table kinds that take the generic stun (#3823).
///
/// What a stun does to a machine with a `RandomBranchState`, from IL
/// (v0.111.0, `sts2.dll` sha256 `9cb4f1ad…`):
///
/// * It never rolls. `Creature::StunInternal` `0x11d7cc` IL_0039-IL_0055
///   parks `StateLog.Last()`, which is the `MoveState` the last walk landed
///   on, because `RandomBranchState::get_ShouldAppearInLogs` `0x79110` is
///   false ([`parked_telegraph`]).
/// * The STUNNED turn performs nothing, and the roll that follows it
///   (`MonsterMoveStateMachine::FindNextMoveState` `0x78e94`) leaves STUNNED
///   by its `FollowUpStateId` (IL_0045-IL_008f) straight into the parked
///   `MoveState`: no `RandomBranchState::GetNextState` `0x792c4`, so **no
///   `MonsterAi` draw**, where the unstunned monster's roll at that seam
///   would have drawn one.
/// * That walk appends the parked move to `StateLog` **a second time**
///   (IL_009c-IL_00d1).
/// * `RandomBranchState::GetStateWeight` `0x79380`, read whole, is the only
///   reader of the log in a roll: `UseOnlyOnce` asks `Contains`
///   (IL_0049-IL_0077), which a duplicate cannot change; CannotRepeat
///   (N = 1) and `CanRepeatXTimes` (N = `maxTimes`) weigh 0 exactly when the
///   last N entries are all this state (IL_009d-IL_014e), so a duplicate
///   moves a `maxTimes >= 2` branch only; a cooldown weighs 0 when the state
///   is among the last `cooldown` entries (IL_0150-IL_01b9), which a
///   duplicate shortens the reach of by one.
///
/// The engine's `RandomAiState` log is that `StateLog`'s tail and every
/// weight above is already computed from it, so the whole model is:
/// `monster_act` performs nothing for a stunned random-table monster and
/// leaves the override in place (native's `NextMove` is still STUNNED while
/// the rest of the enemy side acts), and `prepare_random_ai` resumes it in
/// the monster's own roster position: the override is cleared, `next` is
/// pushed onto the log again, and nothing is drawn
/// (`turn::resume_stunned_random_ai`).
///
/// A kind is listed once its log readers accept the duplicated shapes
/// (`admission::valid_random_ai_state`, the Strangler's and the Hunter
/// Killer's private validators) and its native traces match
/// (`engine::native_stun_traces`).
///
/// `TwoTailedRat` is listed with one more consequence, which is the roster's
/// and not the machine's. `TwoTailedRat::CanSummon` `0xc42c8` IL_0082-IL_009b
/// compares each teammate's `NextMove.Id` with CALL_FOR_BACKUP_MOVE, and a
/// stunned teammate's `NextMove` is STUNNED. A rat rolled while a teammate is
/// stunned over a parked CALL_FOR_BACKUP can therefore roll the call too, and
/// both perform it (`<CallForBackup>d__41::MoveNext` `0x373d98` does not ask
/// `CanSummon` again; it skips the Add only when no slot is free,
/// IL_00d5-IL_010f, and still raises the counts, IL_01e1-IL_025d). So more
/// than one living rat may hold the call, more than three calls may complete,
/// and a call may complete without a summon: [`rat_roster_is_valid`] and
/// [`spawn_rat`] hold all three.
///
/// `FakeMerchantMonster` is the one listed machine with two
/// `RandomBranchState`s (`GenerateMoveStateMachine` `0xb40c4`; the reading is
/// on `turn::roll_random_intent`'s arm). The parked move decides which of
/// them the roll after it enters, as in an unstunned fight: STUNNED follows
/// to the parked `MoveState` itself, never to a branch state, so a stun over
/// THROW_RELIC resumes THROW_RELIC and the roll after that THROW is
/// `RAND_ATTACK_MOVE`'s. The duplicate entry is read by ENRAGE's cooldown
/// like any other entry: a stun over ENRAGE leaves two ENRAGE entries, which
/// keep it out of the next three rolls instead of the next two.
///
/// `Fabricator` (`GenerateMoveStateMachine` `0xb3bc0`) is the one listed
/// machine whose moves follow to a `ConditionalBranchState`: all three go to
/// `fabricateBranch` (IL_0126-IL_0139), which enters `RAND` under `b__17_2`
/// (`0xb4051`, `CanFabricate`) and DISINTEGRATE under `b__17_3` (`0xb4059`,
/// its complement) (IL_00fd-IL_0121). STUNNED follows to the parked
/// `MoveState`, not to that branch, so the stunned turn's roll evaluates no
/// condition and draws nothing; `CanFabricate` (`0xb3d2c`, fewer than four
/// living teammates) is read by the roll after the parked move performs,
/// which is `turn::roll_fabricator_intent` in the same roster walk. Both
/// `RAND` branches are `CanRepeatForever`, so the second log entry has no
/// reader. A parked FABRICATE or FABRICATING_STRIKE makes its bots a turn
/// late, whatever the teammate count has become: neither body asks
/// `CanFabricate` again.
///
/// `TheObscura` (`GenerateMoveStateMachine` `0xc1d2c`) is listed with its
/// illusion (#3823). ILLUSION_MOVE, PIERCING_GAZE_MOVE, SAIL_MOVE and
/// HARDENING_STRIKE_MOVE all follow to `RAND` (IL_00c7-IL_00e3), and `RAND`
/// holds the last three, each `CannotRepeat` (repeat type 2,
/// IL_00e8-IL_00ff), which reads the last log entry alone: a duplicate
/// changes no weight. ILLUSION_MOVE is in no branch, so it is shown once, as
/// the opening. The `Parafright` it summons takes the generic stun too, and
/// one killed while stunned is the dead-acting illusion
/// ([`ILLUSION_REVIVE_DROPPED`]).
pub(crate) const GENERIC_STUN_RANDOM_TABLE_KINDS: [MonsterKind; 18] = [
    MonsterKind::Exoskeleton,
    MonsterKind::Fabricator,
    MonsterKind::FakeMerchantMonster,
    MonsterKind::FlailKnight,
    MonsterKind::Flyconid,
    MonsterKind::FossilStalker,
    MonsterKind::HunterKiller,
    MonsterKind::Inklet,
    MonsterKind::LeafSlimeS,
    MonsterKind::Mawler,
    MonsterKind::MysteriousKnight,
    MonsterKind::SlitheringStrangler,
    MonsterKind::SludgeSpinner,
    MonsterKind::SoulNexus,
    MonsterKind::SpectralKnight,
    MonsterKind::TheObscura,
    MonsterKind::TwigSlimeM,
    MonsterKind::TwoTailedRat,
];

/// Kinds whose loop holds a `Repeats::Conditional` row and that take the
/// generic stun (#3823).
///
/// What a stun does to a machine with a `ConditionalBranchState`, from IL:
/// the parked state is the `MoveState` the last walk landed on
/// (`ConditionalBranchState::get_ShouldAppearInLogs` `0x78d8f` is false, see
/// [`parked_telegraph`]), and the roll after the STUNNED turn re-enters it by
/// `States[FollowUpStateId]` (`FindNextMoveState` `0x78e94` IL_0045-IL_008f)
/// without evaluating any condition. The condition is evaluated when the
/// parked move has performed and the next roll leaves it, as in an
/// unstunned fight. Nothing reads the log in such a machine, so the second
/// log entry has no reader.
///
/// * `LivingShield` (`GenerateMoveStateMachine` `0xb8c70`): SHIELD_SLAM
///   follows to `SHIELD_SLAM_BRANCH` (IL_007b-IL_007d), which holds
///   SHIELD_SLAM under `b__15_0` and SMASH under `b__15_1`
///   (IL_0082-IL_00a3); SMASH follows to itself (IL_00a8-IL_00aa); the
///   machine opens on SHIELD_SLAM (IL_00c4-IL_00c6). A stun over SHIELD_SLAM
///   resumes SHIELD_SLAM whether or not the Turret Operator died meanwhile,
///   and the ally count is read by the roll after that SLAM, which is the
///   engine's completed-Slam receipt (`turn::monster_act`,
///   `turn::prepare_random_ai`): the stunned action writes no receipt and
///   leaves `loop_pos`, so the receipt is written by the SLAM that performs
///   a turn later. A stun over SMASH resumes SMASH. `RampartPower` is a
///   player-side-start listener on the Shield (`turn::
///   rampart_after_player_side_start`) and reads no move state.
/// * `FrogKnight` (`GenerateMoveStateMachine` `0xb53bc`): FOR_THE_QUEEN
///   follows to `HALF_HEALTH` (IL_00f7-IL_00fa), which holds TONGUE_LASH
///   under `b__25_0` (IL_00ce-IL_00dd; `0xb574b` is `HasBeetleCharged ||
///   !(CurrentHp < MaxHp / 2)`) and BEETLE_CHARGE under `b__25_1`
///   (IL_00e2-IL_00f2; `0xb5774`, the complement). STRIKE_DOWN_EVIL follows
///   to FOR_THE_QUEEN, TONGUE_LASH to STRIKE_DOWN_EVIL and BEETLE_CHARGE to
///   TONGUE_LASH (IL_00ff-IL_0110); the machine opens on TONGUE_LASH
///   (IL_013a-IL_013b). A stun over FOR_THE_QUEEN resumes FOR_THE_QUEEN, and
///   the HP is read by the roll after it performs: the engine's successor is
///   chosen in the tail of the move that performed (`turn::monster_act`),
///   which the stunned action never reaches. So a Frog Knight below half HP
///   only while stunned does not charge, and one that falls below half after
///   the stun does (native cases `frog_low_only_while_stunned`,
///   `frog_low_after_stun`). `PlatingPower` counts rounds, not moves
///   ([`frog_knight_state_is_valid`]).
/// * `KnowledgeDemon` (`GenerateMoveStateMachine` `0xb79bc`):
///   CURSE_OF_KNOWLEDGE follows to SLAP, SLAP to KNOWLEDGE_OVERWHELMING,
///   that to PONDER (IL_00d7-IL_00e8), and PONDER to
///   `CurseOfKnowledgeBranch` (IL_00ed-IL_00f1), which holds
///   CURSE_OF_KNOWLEDGE under `b__37_0` (IL_00f6-IL_0105; `0xb7eab` is
///   `_curseOfKnowledgeCounter < 3`) and SLAP under `b__37_1`
///   (IL_010a-IL_0119; `0xb7eb6`, the complement). The machine opens on
///   CURSE_OF_KNOWLEDGE (IL_0143-IL_0145). The counter has one writer, the
///   end of the curse move's own body
///   (`<CurseOfKnowledgeMove>d__38::MoveNext` `0x3607a8` IL_01ae-IL_01ba,
///   after the player has chosen), so a stunned turn does not move it: a
///   stun over CURSE_OF_KNOWLEDGE resumes it with the same counter, the
///   choice is offered a turn late from the same set
///   (`<ChooseCurse>d__39` `0x3605e0` IL_0069-IL_0074 indexes
///   `_curseOfKnowledgeSets` by the counter), and a stun over PONDER resumes
///   PONDER, whose successor is read when it performs
///   (`turn::monster_act`). `turn::knowledge_demon_state_is_exact` ties the
///   row to the counter only where a stun cannot separate them (counter 0
///   is the opening row; at counter 3 the curse row is never shown again).
const GENERIC_STUN_CONDITIONAL_KINDS: [MonsterKind; 3] = [
    MonsterKind::FrogKnight,
    MonsterKind::KnowledgeDemon,
    MonsterKind::LivingShield,
];

/// May `kind` carry the generic `STUNNED` + parked-telegraph state?
///
/// A random table in [`GENERIC_STUN_RANDOM_TABLE_KINDS`], or a kind with a
/// generated loop, except (#3823):
///
/// * the bespoke owners above;
/// * a kind whose native machine **re-enters a branch state after a move**.
///   The stun itself neither re-rolls nor re-evaluates the branch (the parked
///   state is the `MoveState` the walk landed on, see [`parked_telegraph`]),
///   but the resume logs that move a second time
///   (`FindNextMoveState` `0x78e94` IL_00c3-IL_00d1 on the walk out of
///   STUNNED), and `RandomBranchState::GetStateWeight` `0x79380` reads the
///   log; a conditional successor is deferred past the stunned turn. Those
///   are their own slices. In the generated tables such a machine is a
///   `random_moves` table outside [`GENERIC_STUN_RANDOM_TABLE_KINDS`] or a
///   `Repeats::Conditional` row outside [`GENERIC_STUN_CONDITIONAL_KINDS`].
///
/// `AttackStrengthBranch` (`Fogmog` `0xb4cb8`, SWIPE_MOVE -> `BRANCH`) carries
/// a `RandomBranchState` inside its row and **is** eligible (#3823). ILLUSION
/// follows to SWIPE, SWIPE to `BRANCH`, SWIPE_RANDOM to HEADBUTT and HEADBUTT
/// to SWIPE (IL_0126-IL_0144); `BRANCH` holds SWIPE_RANDOM and HEADBUTT, each
/// `CannotRepeat` (repeat type 2) under a constant weight
/// (`<GenerateMoveStateMachine>b__14_0` `0x35bde2`, `b__14_1` `0x35bde9`;
/// IL_00c9-IL_0121). `BRANCH` is entered only from SWIPE, so the last log
/// entry it reads is always SWIPE, and a stun's duplicate changes neither
/// weight. A stun over SWIPE parks the `MoveState`, so the STUNNED turn draws
/// nothing; the resumed SWIPE draws its one MonsterAi in its own tail
/// (`turn::roll_fogmog_branch`), as an unstunned one does. Native trace
/// `fog_s2`: no MonsterAi draw on the stunned turn, one on the turn after.
///
/// `AttackVulnerableDeferredOvicopter` (`Ovicopter` `0xbab10`) also carries a
/// branch and **is** eligible (#3823). TENDERIZER_MOVE follows to
/// `SUMMON_BRANCH_STATE` (IL_00de-IL_00e1), a `ConditionalBranchState` that
/// enters LAY_EGGS_MOVE under `b__25_0` `0xbae6c` and NUTRITIONAL_PASTE_MOVE
/// under `b__25_1` `0xbae74` (IL_00e6-IL_010a). The branch is read only on
/// the walk out of TENDERIZER_MOVE, which the engine runs once TENDERIZER
/// has performed (the move parks the loop on `OVICOPTER_BRANCH_POS`) and
/// resolves after the enemy side's end ([`resolve_ovicopter_after_side_end`]).
/// A stun over TENDERIZER parks the `MoveState`, so the STUNNED turn reads no
/// condition and the resumed TENDERIZER reads it after it performs, as an
/// unstunned one does. The sentinel row is resolved before the player acts,
/// so no stun lands on it, and [`parked_telegraph`] names no follow-up for
/// it: a stun there refuses by name. No branch reads the log.
///
/// `AttackScrollBranch` (`ScrollOfBiting` `0xbcde4`, CHEW -> `rand`) is the
/// third such move kind and **is** eligible. The one weight of its `rand`
/// that a duplicated log entry can move is CHEW's `maxRepeats = 2`
/// (IL_00b9-IL_00bd), whose whole dependence on the log is the
/// `scroll_chew_repeated` latch, and the generic arm of `monster_act` sets
/// that latch when it resumes a parked CHEW (the reading and the native trace
/// are cited there). CHOMP's CannotRepeat (IL_00b0-IL_00b4) reads the last
/// entry alone, which a duplicate does not change.
///
/// A branch state that is only the machine's **initial** state is not a
/// re-entry: `Toadpole` `0xc2798`, `Myte` `0xba07c`, `Nibbit` `0xba3cc` and
/// `PhantasmalGardener` `0xbb890` open on a `ConditionalBranchState`
/// `INIT_MOVE` that no `MoveState` follows into, so after the opening roll
/// (which the root already carries as `loop_pos`) each is a plain cycle.
/// `PhrogParasite` `0xbbdac` registers a `RandomBranchState` `RAND` that
/// nothing follows into at all (IL_006f-IL_0078 wire INFECT_MOVE and
/// LASH_MOVE to each other, and INFECT_MOVE is the initial state, IL_00a3).
/// `Wriggler` `0xc5630` enters its `INIT_MOVE` once, from SPAWNED_MOVE
/// (IL_00ec-IL_00ef), and the spawn has already written the outcome to
/// `loop_pos`; while SPAWNED_MOVE is showing the instance carries
/// `spawn_noop` ([`generic_stun_owner_state_is_representable`]).
///
/// The set is pinned by `the_generic_stun_owner_set_is_pinned`.
pub(crate) fn generic_stun_owner_is_eligible(kind: MonsterKind) -> bool {
    if GENERIC_STUN_BESPOKE_KINDS.contains(&kind) {
        return false;
    }
    if crate::content_tables::random_moves(kind).is_some() {
        return GENERIC_STUN_RANDOM_TABLE_KINDS.contains(&kind);
    }
    let Some(rows) = crate::content_tables::monster_loop(kind) else {
        return false;
    };
    !rows.iter().any(|row| {
        matches!(row.repeats, crate::content_tables::Repeats::Conditional(_))
            && !GENERIC_STUN_CONDITIONAL_KINDS.contains(&kind)
    })
}

/// Is this monster's move machine undisturbed, or carrying exactly the
/// generic stun ([`generic_stun_parks_this_row`])?
///
/// The condition every per-kind lifecycle validator of a generic stun owner
/// asks of its machine (#3823). Before #3823 each of them asked for
/// `override_state == None` and `forced_follow_up == None`, which was the
/// whole truth while no such kind could be stunned. They are read at runtime
/// (a move body, a card step, a death, the projection), not only at
/// admission, so a root admitted with a Whistle refused mid-line the moment
/// the stun landed: Soul Fysh, The Insatiable, Aeonglass, Gremlin Merc, The
/// Lost and Forgotten, Entomancer and The Kin did. The stun changes nothing
/// those validators guard: `Creature::StunInternal` `0x11d7cc` writes the
/// move state alone (IL_0057-IL_0087), no power, no HP, no roster slot.
///
/// A kind outside the generic set gets no new state from this: its stunned
/// state still fails here.
///
/// Liveness is not asked. A lethal hit commits the HP before the death
/// callback runs, and the validators are read in between (a Kin death starts
/// from `kin_roster_shape_is_exact`); the callback then drops the stun with
/// the creature ([`clear_generic_stun_at_death`]).
pub(crate) fn machine_is_undisturbed_or_generic_stun(monster: &HotMonster) -> bool {
    (monster.override_state == MonsterOverride::None
        && monster.forced_follow_up == MonsterFollowUp::None)
        || generic_stun_shape_parks_this_row(monster)
}

/// `STUNNED` over the follow-up [`parked_telegraph`] names for this
/// `loop_pos`, on an eligible kind, alive or not.
fn generic_stun_shape_parks_this_row(monster: &HotMonster) -> bool {
    generic_stun_owner_is_eligible(monster.kind)
        && monster.override_state == MonsterOverride::Stunned
        && parked_telegraph(monster.kind, monster.loop_pos) == Some(monster.forced_follow_up)
        && (crate::content_tables::random_moves(monster.kind).is_none()
            || monster.random_ai.next().is_some())
}

/// The generic stun's document state on one monster: an eligible kind,
/// alive, `STUNNED`, and the follow-up [`parked_telegraph`] names for its
/// `loop_pos`.
pub(crate) fn generic_stun_parks_this_row(monster: &HotMonster) -> bool {
    monster.hp > 0 && generic_stun_shape_parks_this_row(monster)
}

/// A dying monster's generic stun goes with it.
///
/// Native removes the creature from combat (`CombatState::RemoveCreature`
/// `0x1371d0` IL_0029-IL_0043), so nothing reads its move state again; the
/// retained corpse row must therefore not depend on whether it died stunned.
///
/// The one generic owner that stays in combat is an illusion, and its death
/// does not clear the stun: the stun is what keeps the revive out
/// ([`ILLUSION_REVIVE_DROPPED`]). The death body reads
/// [`illusion_stun_survives_death`] before this and puts the stun back.
pub(crate) fn clear_generic_stun_at_death(monster: &mut HotMonster) {
    if generic_stun_shape_parks_this_row(monster) {
        monster.override_state = MonsterOverride::None;
        monster.forced_follow_up = MonsterFollowUp::None;
    }
}

/// The stun an illusion dies under, which its death keeps
/// ([`ILLUSION_REVIVE_DROPPED`]), or `None` when it dies unstunned and its
/// REVIVE_MOVE is installed ([`ILLUSION_REVIVING`]).
///
/// Read before [`clear_generic_stun_at_death`] and the death body's power
/// wipe, which both clear the override.
pub(crate) fn illusion_stun_survives_death(
    monster: &HotMonster,
) -> Option<(MonsterOverride, MonsterFollowUp)> {
    (is_illusion(monster.kind) && generic_stun_shape_parks_this_row(monster))
        .then_some((monster.override_state, monster.forced_follow_up))
}

/// Is `owner`'s generic stun state the one [`parked_telegraph`] determines?
///
/// The exactness twin of [`queen_amalgam_stun_state_is_exact`] for every other
/// eligible owner. `StunInternal` `0x11d7cc` IL_0028 returns silently on a
/// dead creature, so a corpse can never carry the state.
///
/// It also covers the one bespoke owner whose `CreatureCmd::Stun(creature,
/// null)` state is fully determined: an awake Slumbering Beetle parked over
/// ROLL_OUT_MOVE ([`beetle_rollout_stun_is_exact`], #2647). Its restore is the
/// same as every eligible owner's — clear the override, leave `loop_pos` —
/// because its one executable row is ROLL_OUT itself.
pub(crate) fn generic_stun_state_is_exact(state: &HotState, owner: usize) -> bool {
    state.monsters.get(owner).is_some_and(|monster| {
        generic_stun_parks_this_row(monster)
            || beetle_rollout_stun_is_exact(monster)
            || (monster.hp > 0
                && monster.override_state == MonsterOverride::Stunned
                && (lagavulin_sleep_is_parked(monster) || beetle_snore_is_parked(monster)))
    })
}

/// A Lagavulin Matriarch whose SLEEP_MOVE is parked (#3823): under a stun
/// (`STUNNED` over [`MonsterFollowUp::LagavulinSleep`]), or, with no override,
/// the resumed SLEEP_MOVE that stun hands back.
///
/// While it sleeps the machine sits on SLEEP_MOVE (`LagavulinMatriarch::
/// GenerateMoveStateMachine` `0xb8124` IL_0012-IL_0031, the initial state at
/// IL_017f-IL_0181; SLEEP_BRANCH returns to it while `HasPower<AsleepPower>`,
/// `b__43_0` `0xb85d8`), so `StateLog.Last()` is SLEEP_MOVE and a stun parks
/// it (`Creature::StunInternal` `0x11d7cc` IL_0039-IL_0055). The walk out of
/// STUNNED follows `FollowUpStateId` to SLEEP_MOVE itself
/// (`MonsterMoveStateMachine::FindNextMoveState` `0x78e94` IL_0045-IL_008f):
/// a move state, so no branch is read and SLEEP_MOVE is shown again whatever
/// Asleep holds by then. Only the roll after that SLEEP_MOVE reads
/// SLEEP_BRANCH. Native traces `lag_s3` (a stun on the turn Asleep runs out:
/// SLEEP_MOVE again, then SLASH), `lag_s3s4` (a stun on that resumed
/// SLEEP_MOVE parks it again).
///
/// `loop_pos` is the SLASH the machine rolls to once awake, as for every
/// sleeping Matriarch (`turn::lagavulin_sleep_move`).
pub(crate) fn lagavulin_sleep_is_parked(monster: &HotMonster) -> bool {
    monster.kind == MonsterKind::LagavulinMatriarch
        && matches!(
            monster.override_state,
            MonsterOverride::Stunned | MonsterOverride::None
        )
        && monster.forced_follow_up == MonsterFollowUp::LagavulinSleep
        && monster.loop_pos == super::admission::LAGAVULIN_SLASH_INDEX
        && !monster.spawn_noop
        && (0..=3).contains(&monster.powers.value(PowerId::Asleep))
}

/// A Slumbering Beetle whose SNORE_MOVE is parked (#3823), the
/// [`lagavulin_sleep_is_parked`] reading of `SlumberingBeetle::
/// GenerateMoveStateMachine` `0xbe5f4`: SNORE_MOVE follows to the SNORE_NEXT
/// `ConditionalBranchState` (IL_006a-IL_009d), which only the roll after a
/// performed SNORE_MOVE reads. So a stun parks SNORE_MOVE, the walk out of
/// STUNNED shows it again whatever Slumber holds, and only then does the
/// Beetle roll on to ROLL_OUT. Native traces `bee_s3s4`, `bee_s2s3`.
///
/// Slumber is 0..=3, and Plating may outlive it: a wake by damage under the
/// stun is dropped (`damage::damage_monster`), so `WakeUpMove` never removes
/// Plating (native trace `bee_s3b0st3`).
pub(crate) fn beetle_snore_is_parked(monster: &HotMonster) -> bool {
    monster.kind == MonsterKind::SlumberingBeetle
        && matches!(
            monster.override_state,
            MonsterOverride::Stunned | MonsterOverride::None
        )
        && monster.forced_follow_up == MonsterFollowUp::BeetleSnore
        && monster.loop_pos == 0
        && !monster.spawn_noop
        && (0..=3).contains(&monster.powers.value(PowerId::Slumber))
}

/// Is this an awake Slumbering Beetle under a `CreatureCmd::Stun(creature,
/// null)` that parked ROLL_OUT_MOVE (#2647)?
///
/// The Beetle is a bespoke owner — its SNORE/WAKE machine is private
/// overrides, not a generated loop — so [`parked_telegraph`] cannot name its
/// row. The telegraph is nonetheless unique, re-derived on DLL
/// `9cb4f1ad…`:
///
/// * the Beetle deals damage only in ROLL_OUT_MOVE
///   (`SlumberingBeetle::GenerateMoveStateMachine` `0xbe5f4`: SNORE_MOVE's
///   body `0xbe6ba` is a completed task; ROLL_OUT_MOVE is IL_0037-IL_0069),
///   and a monster-owned Imbalanced stuns only its dealer
///   (`ImbalancedPower` `0x33cd60` IL_0020-IL_002e), so the stun lands with
///   ROLL_OUT_MOVE the current state;
/// * every way into ROLL_OUT_MOVE appends it to `StateLog`: the SNORE_NEXT
///   `ConditionalBranchState` (IL_006a-IL_009d) is not loggable
///   (`ConditionalBranchState::get_ShouldAppearInLogs` `0x78d8f` = false),
///   so `FindNextMoveState` `0x78e94` IL_009c-IL_00d6 logs the move the walk
///   lands on; SlumberPower's damage wake is `Stun(owner, WakeUpMove,
///   "ROLL_OUT_MOVE")` (`SlumberPower/<AfterDamageReceived>d__4::MoveNext`
///   `0x34500c` IL_00c1-IL_00d8), whose follow-up roll logs ROLL_OUT_MOVE the
///   same way; and ROLL_OUT_MOVE is its own follow-up (IL_00a2-IL_00a4);
/// * so `StunInternal` `0x11d7cc` IL_0031-IL_0055's `StateLog.Last().Id` is
///   ROLL_OUT_MOVE, which is the Beetle's single executable row
///   (`catalog.rs` `BEETLE_ROLLOUT`), at `loop_pos` 0.
///
/// Awake means no Slumber and no Plating — `beetle_state_is_valid`'s
/// `MonsterOverride::None` arm, which this state interrupts.
pub(crate) fn beetle_rollout_stun_is_exact(monster: &HotMonster) -> bool {
    monster.kind == MonsterKind::SlumberingBeetle
        && monster.hp > 0
        && monster.override_state == MonsterOverride::Stunned
        && monster.forced_follow_up == MonsterFollowUp::BeetleRollOut
        && monster.loop_pos == 0
        && monster.powers.value(PowerId::Slumber) == 0
}

/// Install an Imbalanced carrier's `CreatureCmd::Stun(Owner, null)` on
/// itself (`ImbalancedPower/<AfterDamageGiven>d__4::MoveNext` `0x33cd60`
/// IL_0068-IL_006f).
///
/// Every generic owner goes through [`install_generic_stun`]. The awake
/// Slumbering Beetle is the one bespoke owner whose parked telegraph is
/// determined ([`beetle_rollout_stun_is_exact`]); it takes the same
/// `StunInternal` `0x11d7cc` contract here — silent on a corpse or a finished
/// combat (IL_001f / IL_0028), a silent no-op over its own unperformed stun
/// (`SetMoveImmediate` `0x825f0` IL_000c-IL_001c with `force: false`), and a
/// refusal over anything else rather than an overwrite. It is deliberately
/// NOT reachable from Whistle's `install_generic_stun`: a sleeping Beetle's
/// stun would park SNORE_MOVE, a shape nothing here models.
pub(crate) fn install_imbalanced_self_stun(
    state: &mut HotState,
    owner: usize,
) -> Result<(), EngineRefusal> {
    let Some(monster) = state.monsters.get(owner) else {
        return Ok(());
    };
    if monster.kind != MonsterKind::SlumberingBeetle {
        return install_generic_stun(state, owner);
    }
    install_beetle_stun(state, owner)
}

/// `CreatureCmd::Stun(creature, null)` on a Slumbering Beetle, from a Whistle
/// or its own Imbalanced clone (#2647, #3823), with `StunInternal`
/// `0x11d7cc`'s contract: silent on a corpse or a finished combat (IL_001f /
/// IL_0028), a silent drop over an unperformed stun (its own WAKE or a
/// standing STUNNED: `SetMoveImmediate` `0x825f0` IL_000c-IL_001c with
/// `force: false`), otherwise STUNNED over the move the Beetle is showing:
/// ROLL_OUT_MOVE awake ([`beetle_rollout_stun_is_exact`]), SNORE_MOVE asleep
/// or resumed ([`beetle_snore_is_parked`]).
///
/// Awake with Plating is the wake a stun dropped (`bee_s3b0st3`): it still
/// shows ROLL_OUT_MOVE, so a stun parks ROLL_OUT (`bee_s3b0st3s4`).
fn install_beetle_stun(state: &mut HotState, owner: usize) -> Result<(), EngineRefusal> {
    let monster = &state.monsters[owner];
    if state.history.over || monster.hp <= 0 {
        return Ok(());
    }
    if monster.override_state == MonsterOverride::Stunned {
        return generic_stun_state_is_exact(state, owner)
            .then_some(())
            .ok_or(EngineRefusal::MalformedArgs("generic stun existing state"));
    }
    if own_stun_is_unperformed(monster) {
        return Ok(());
    }
    if !beetle_state_is_valid(monster) {
        return Err(EngineRefusal::MalformedArgs("Slumbering Beetle stun state"));
    }
    let follow_up = match (monster.override_state, monster.forced_follow_up) {
        (MonsterOverride::None, MonsterFollowUp::None) => MonsterFollowUp::BeetleRollOut,
        (MonsterOverride::BeetleSnore, MonsterFollowUp::None)
        | (MonsterOverride::None, MonsterFollowUp::BeetleSnore) => MonsterFollowUp::BeetleSnore,
        _ => return Err(EngineRefusal::MalformedArgs("Slumbering Beetle stun state")),
    };
    let monster = &mut state.monsters_mut()[owner];
    monster.override_state = MonsterOverride::Stunned;
    monster.forced_follow_up = follow_up;
    generic_stun_state_is_exact(state, owner)
        .then_some(())
        .ok_or(EngineRefusal::MalformedArgs(
            "Slumbering Beetle stun result",
        ))
}

/// Can a player-sourced `CreatureCmd::Stun(target, null)` represent stunning
/// this monster, at any point in the fight?
///
/// The single gate behind Whistle's admission wall, its preflight, and its step
/// body, so the three cannot grow separate opinions. #2647 slice A (A-opt)
/// widens it from "the Queen roster and nothing else" — which is all
/// [`queen_amalgam_follow_up`]'s two-kind table could express — to any owner
/// whose parked telegraph is uniquely determined, and #3823 to every owner
/// whose machine is a plain cycle after its opening roll
/// ([`generic_stun_owner_is_eligible`]).
///
/// **Every** position of the loop must resolve, not just the current one:
/// `loop_pos` advances during the fight and a root is admitted for the whole
/// fight.
pub(crate) fn whistle_stun_target_is_representable(
    state: &HotState,
    catalog: &Catalog,
    monster: &HotMonster,
) -> bool {
    if monster.hp <= 0 {
        // `Creature::StunInternal` `0x11d7cc` IL_0028 — a corpse is a silent
        // no-op, never a state this root has to represent.
        return true;
    }
    if matches!(
        monster.kind,
        MonsterKind::Queen | MonsterKind::TorchHeadAmalgam
    ) {
        return queen_roster_state_is_valid(state)
            && crate::moves::boss::queen_amalgam_machine_is_exact(catalog);
    }
    if monster.kind == MonsterKind::SlumberingBeetle {
        // #3823: no generated loop, but every move the Beetle can show parks
        // exactly: SNORE_MOVE asleep, ROLL_OUT_MOVE awake, and its own WAKE
        // drops the stun ([`install_beetle_stun`]).
        return beetle_state_is_valid(monster);
    }
    generic_stun_owner_state_is_representable(catalog, monster)
}

/// Why Whistle's stun cannot be represented on this monster, by name (#3209,
/// #3823), or `None` when [`whistle_stun_target_is_representable`].
///
/// Before #3823 every such root refused as `"Whistle Queen/Amalgam
/// lifecycle"`, whatever the roster held: the five site fights that carried
/// the name held a Scroll of Biting, a Living Shield with its Turret Operator,
/// an Owl Magistrate and a Mecha Knight, and no Queen.
pub(crate) fn whistle_stun_target_refusal(
    state: &HotState,
    catalog: &Catalog,
    monster: &HotMonster,
) -> Option<&'static str> {
    if whistle_stun_target_is_representable(state, catalog, monster) {
        return None;
    }
    Some(
        if matches!(
            monster.kind,
            MonsterKind::Queen | MonsterKind::TorchHeadAmalgam
        ) {
            WHISTLE_QUEEN_AMALGAM
        } else if GENERIC_STUN_BESPOKE_KINDS.contains(&monster.kind) {
            WHISTLE_STUN_BESPOKE
        } else if !generic_stun_owner_is_eligible(monster.kind) {
            WHISTLE_STUN_BRANCH
        } else {
            WHISTLE_STUN_INSTANCE
        },
    )
}

/// The Queen roster's own lifecycle, and a multiplayer roster.
pub(crate) const WHISTLE_QUEEN_AMALGAM: &str = "Whistle Queen/Amalgam lifecycle";
/// A kind in [`GENERIC_STUN_BESPOKE_KINDS`]: its stun state has its own reader.
pub(crate) const WHISTLE_STUN_BESPOKE: &str = "Whistle stun target with a bespoke move machine";
/// Random AI, a private loop, a conditional successor or a branch move kind:
/// `StateLog.Last()` is not established to name the `loop_pos` row.
pub(crate) const WHISTLE_STUN_BRANCH: &str = "Whistle stun target whose move machine branches";
/// An eligible kind whose instance is off its generated loop (a revive stage,
/// a random walk, or a catalog table of another length).
pub(crate) const WHISTLE_STUN_INSTANCE: &str = "Whistle stun target off its generated loop";

/// Is every position of `kind`'s loop a resolvable parked telegraph?
///
/// The *kind*-level half of generic-stun representability: not bespoke, no
/// random AI, no `Repeats::Conditional` successor
/// ([`generic_stun_owner_is_eligible`]), **and** every row resolving through
/// [`parked_telegraph`]. Every position must resolve, not just the current
/// one, because `loop_pos` advances during the fight and a root is admitted
/// for the whole fight.
///
/// Factored out because three readers must not grow separate opinions of it:
/// [`whistle_stun_target_is_representable`], the admission gate for an
/// Imbalanced attachment (#2693 B1 — a carrier is admissible only if the
/// stun its listener installs is representable at every position it could
/// reach), and the pin `the_generic_stun_owner_set_is_pinned`. The pin is the
/// tripwire for the new-reader-invalidates-old-shortcut class (#1432): the
/// set is *derived* from the generated tables, so a table change could widen
/// all three readers at once.
pub(crate) fn generic_stun_owner_loop_is_fully_resolvable(kind: MonsterKind) -> bool {
    if !generic_stun_owner_is_eligible(kind) {
        return false;
    }
    if crate::content_tables::random_moves(kind).is_some() {
        // Every row of a random table parks the same way: `random_ai.next`.
        return parked_telegraph(kind, 0).is_some();
    }
    crate::content_tables::monster_loop(kind).is_some_and(|rows| {
        (0..rows.len()).all(|index| parked_telegraph(kind, index as i32).is_some())
    })
}

/// The kind-level gate above, plus the per-instance conditions that decide
/// whether *this* monster's `loop_pos` still indexes the loop.
///
/// For a loop kind `random_ai` must be empty and `revive_stage` zero — each
/// of those is a machine whose current row is not the generated loop's — and
/// the catalog's executable table must agree in length with the generated
/// loop, since the catalog compiles it from that same loop in row order and
/// `loop_pos` indexes both only while the two agree. For a random table the
/// machine must be the exact one admission accepts
/// (`admission::random_ai_state_is_exact`).
///
/// `spawn_noop` is **not** a bar (#3823). It is the native `SPAWNED_MOVE`
/// `MoveState` of the three kinds admission lets carry it (`FatGremlin`
/// `0xb4504` IL_0012-IL_0031, `SneakyGremlin` `0xbea48`, `Wriggler`
/// `0xc5630`): each machine's initial state, so the constructor logs it
/// (`MonsterMoveStateMachine::.ctor` `0x78da4` IL_006b-IL_0084) and a stun
/// parks it like any other telegraph. The stunned action then clears only
/// the override; the action after it spends `spawn_noop`; the one after
/// that performs the `loop_pos` row. `monster_act` resolves the override
/// before `spawn_noop`, which is that order. Native trace: a Fat Gremlin
/// spawned and stunned in the same player turn performs STUNNED,
/// SPAWNED_MOVE, FLEE_MOVE on the next three enemy turns.
pub(crate) fn generic_stun_owner_state_is_representable(
    catalog: &Catalog,
    monster: &HotMonster,
) -> bool {
    // `revive_stage` is a revive machine's own row for the kinds that have
    // one. The Knowledge Demon keeps its curse counter in that lane
    // (`HotMonster::knowledge_demon_curse_counter`), which is no row at all:
    // its `loop_pos` indexes the generated loop at every counter (#3823).
    // The Test Subject keeps its form count and its two lifecycle latches
    // there (`HotMonster::test_subject_respawns`); each form's rows are rows
    // of the one generated loop (BITE and SKULL_BASH, MULTI_CLAW, then
    // LACERATE, BIG_POUNCE and BURNING_GROWL), so its `loop_pos` indexes the
    // loop in every form too (#3823).
    let off_loop = monster.revive_stage != 0
        && !matches!(
            monster.kind,
            MonsterKind::KnowledgeDemon | MonsterKind::TestSubject
        );
    if !generic_stun_owner_loop_is_fully_resolvable(monster.kind) || off_loop {
        return false;
    }
    if let Some(rows) = crate::content_tables::random_moves(monster.kind) {
        // A random table (#3823): the telegraph is `random_ai.next`, so the
        // instance is representable exactly when its machine is the one
        // admission accepts. A rat published without an intent
        // (`rat_spawn_fresh`, #3747) has no `StateLog` entry yet and is
        // reachable only inside an enemy side, where no Whistle lands.
        return catalog.moves(monster.kind).len() == rows.len()
            && super::admission::random_ai_state_is_exact(monster, rows.len());
    }
    monster.random_ai.is_empty()
        && crate::content_tables::monster_loop(monster.kind)
            .is_some_and(|rows| catalog.moves(monster.kind).len() == rows.len())
}

/// Is this monster showing its kind's own unperformed stun, which a
/// Whistle's `CreatureCmd::Stun(target, null)` cannot replace (#3823)?
///
/// Each of these is installed through `Creature::StunInternal` `0x11d7cc`,
/// which marks the state `MustPerformOnceBeforeTransitioning` (IL_0078), so
/// a second stun's `SetMoveImmediate(state, force: false)` is dropped
/// (`MonsterModel::SetMoveImmediate` `0x825f0` IL_000c-IL_001c):
///
/// * the Tunneler's DIZZY: `BurrowedPower/<AfterBlockBroken>d__5::MoveNext`
///   `0x33622c` IL_00d6-IL_00db `CreatureCmd.Stun(owner, ..., "BITE_MOVE")`.
///   Native trace `tun_br3s3`.
/// * the Bowlbug Rock's DIZZY: `BowlbugRock/<HeadbuttMove>d__24::MoveNext`
///   IL_00bf-IL_00c7 calls `BowlbugRock::Stun` `0xafddc` when the HEADBUTT
///   left it off balance, and `BowlbugRock/<Stun>d__25::MoveNext` IL_009e-
///   IL_00b0 is `CreatureCmd.Stun(creature, DizzyMove, null)` `0x132b2c`.
///   Native traces `rck_pb1s2`, `rck_pb1w2`.
/// * the Corpse Slug's RAVENOUS_STUN_MOVE: `RavenousPower/<AfterDeath>d__6`
///   installs it through the same `StunInternal` (the stun its sibling's
///   death lands). Native traces `slg_k1s1`, `slg_k1w2`.
/// * the Ceremonial Beast's STUN_MOVE: `PlowPower/<AfterDamageReceived>d__8::
///   MoveNext` IL_0225-IL_024c `CreatureCmd.Stun(owner, StunnedMove,
///   BeastCryState.StateId)` `0x132b2c`, after `SetStunned` `0xb0f24`.
///   Native traces `bst_hb2s`, `bst_hbw2`.
/// * the Lagavulin Matriarch's WAKE_UP_MOVE: `AsleepPower/<AfterDamageReceived>
///   d__4::MoveNext` `0x335094` IL_0162-IL_017e `CreatureCmd.Stun(Owner,
///   WakeUpMove, "SLASH_MOVE")`. Native traces `lag_b0st2s`, `lag_w`.
/// * the Slumbering Beetle's WAKE: `SlumberPower/<AfterDamageReceived>d__4::
///   MoveNext` `0x34500c` IL_00c1-IL_00d8 `Stun(owner, WakeUpMove,
///   "ROLL_OUT_MOVE")`. Native trace `bee_b0st3s3`.
/// * the Terror Eel's STUNNED, the Terror chain:
///   `ShriekPower/<AfterDamageReceived>d__10::MoveNext` `0x344348`
///   IL_0065-IL_008b `CreatureCmd.Stun(Owner, TerrorState.StateId)`
///   `0x132ae0`. The engine carries it as `STUNNED` with no follow-up, and a
///   Whistle on a showing TERROR_MOVE writes the same state
///   ([`install_generic_stun`]). Native traces `eel_hb2s`, `eel_hbw2`.
pub(crate) fn own_stun_is_unperformed(monster: &HotMonster) -> bool {
    match (
        monster.kind,
        monster.override_state,
        monster.forced_follow_up,
    ) {
        (MonsterKind::TerrorEel, MonsterOverride::Stunned, MonsterFollowUp::None) => {
            monster.powers.value(PowerId::Shriek) == 0
        }
        (MonsterKind::Tunneler, MonsterOverride::Dizzy, MonsterFollowUp::None) => {
            monster.powers.value(PowerId::Burrowed) == 0
        }
        (MonsterKind::BowlbugRock, MonsterOverride::Dizzy, MonsterFollowUp::None) => true,
        (
            MonsterKind::CorpseSlug,
            MonsterOverride::RavenousStun,
            MonsterFollowUp::SlugWhipSlap | MonsterFollowUp::SlugGlomp | MonsterFollowUp::SlugGoop,
        ) => true,
        (MonsterKind::CeremonialBeast, MonsterOverride::BeastStun, MonsterFollowUp::BeastCry) => {
            true
        }
        (
            MonsterKind::LagavulinMatriarch,
            MonsterOverride::LagavulinWakeUp,
            MonsterFollowUp::LagavulinSlash,
        ) => true,
        (MonsterKind::SlumberingBeetle, MonsterOverride::BeetleWake, MonsterFollowUp::None) => true,
        _ => false,
    }
}

/// Install `CreatureCmd::Stun(creature, null)` on a monster.
///
/// `0x132ae0` -> `d__26::MoveNext` `0x3ed060` IL_0016–IL_0041 forwards to
/// `0x132b2c` with the completed-task body `<Stun>b__26_0` `0x3e8f83`, so the
/// dynamic state's `onPerform` is a no-op and the state's whole effect is to
/// consume one enemy action. `0x132b2c` IL_0032 then calls
/// `Creature::StunInternal` `0x11d7cc`, which
///
/// * IL_001f / IL_0028 returns silently with no combat state or on a corpse;
/// * IL_0057–IL_0071 builds `MoveState("STUNNED", …)` with
///   `FollowUpStateId = StateLog.Last().Id` — see [`parked_telegraph`];
/// * IL_0078 sets `MustPerformOnceBeforeTransitioning`, so
///   `MoveState::get_CanTransitionAway` `0x78ff3` is false until the state has
///   performed, and IL_007f's `SetMoveImmediate(state, force: false)`
///   (`0x825f0` IL_000c–IL_001c) therefore makes a **second** stun landing
///   before the first performs a silent no-op rather than a reinstall.
///
/// Anything else already occupying the override refuses: native would install
/// over it, and the bespoke owners' wire states are not this one.
pub(crate) fn install_generic_stun(
    state: &mut HotState,
    owner: usize,
) -> Result<(), EngineRefusal> {
    if state.history.over {
        return Ok(());
    }
    let Some(monster) = state.monsters.get(owner) else {
        return Ok(());
    };
    if monster.hp <= 0 {
        return Ok(());
    }
    if monster.override_state == MonsterOverride::Stunned {
        // The unperformed dynamic state cannot transition away and `force` is
        // false: native drops this stun on the floor without touching state.
        // #3823: that holds for the Thieving Hopper's Flutter stun too, which
        // `FlutterPower` installs through the same `StunInternal` `0x11d7cc`
        // (must-perform, IL_0078). Native trace `hop_fls4`.
        let hopper_flutter_stun = monster.kind == MonsterKind::ThievingHopper
            && matches!(
                (monster.loop_pos, monster.forced_follow_up),
                (2, MonsterFollowUp::HopperNab) | (3 | 4, MonsterFollowUp::HopperEscape)
            )
            && thieving_hopper_state_is_valid(state);
        // #3823: so does the Terror Eel's own STUNNED (the Terror chain) and
        // the Whistle-over-TERROR state that is wire-identical to it
        // ([`own_stun_is_unperformed`]).
        return (generic_stun_state_is_exact(state, owner)
            || hopper_flutter_stun
            || own_stun_is_unperformed(monster))
        .then_some(())
        .ok_or(EngineRefusal::MalformedArgs("generic stun existing state"));
    }
    if monster.kind == MonsterKind::TerrorEel
        && (monster.override_state, monster.forced_follow_up)
            == (MonsterOverride::Terror, MonsterFollowUp::None)
    {
        // #3823: a stun over a showing TERROR_MOVE. `StateLog.Last()` is
        // TERROR_MOVE (the walk out of the Eel's own STUNNED logs the state
        // it lands on, `FindNextMoveState` `0x78e94` IL_009c-IL_00d6), and
        // TerrorState is a plain `MoveState`
        // (`TerrorEel::GenerateMoveStateMachine` `0xbfe28` IL_009b-IL_00c1,
        // no `MustPerformOnceBeforeTransitioning`), so `StunInternal`
        // `0x11d7cc` parks it: STUNNED following to TERROR_MOVE, which is
        // exactly the state ShriekPower's own stun installs
        // (`ShriekPower/<AfterDamageReceived>d__10::MoveNext` `0x344348`
        // IL_0081-IL_008b). The second `StateLog` entry has no reader (the
        // machine has no branch state). So the engine writes the Eel's own
        // STUNNED: it performs nothing, TERROR_MOVE follows, then CRASH
        // (TerrorState follows to CRASH, IL_00e0-IL_00e7). Native trace
        // `eel_hb2s3`.
        if monster.powers.value(PowerId::Shriek) != 0 {
            return Err(EngineRefusal::MalformedArgs("Terror Eel stun state"));
        }
        state.monsters_mut()[owner].override_state = MonsterOverride::Stunned;
        return Ok(());
    }
    if monster.kind == MonsterKind::WaterfallGiant
        && monster.is_about_to_blow()
        && monster.override_state == MonsterOverride::None
        && monster.forced_follow_up == MonsterFollowUp::None
        && monster.loop_pos == WATERFALL_ABOUT_TO_BLOW_INDEX
    {
        // #3823: ABOUT_TO_BLOW_MOVE is must-perform
        // (`WaterfallGiant::GenerateMoveStateMachine` `0xc4eb0` IL_0169), so
        // `MoveState::get_CanTransitionAway` `0x78ff3` is false until it has
        // performed and `StunInternal`'s `SetMoveImmediate(state, force:
        // false)` (`0x825f0` IL_000c-IL_001c) drops the stun. Native trace
        // `wfg_k2s2`: ABOUT_TO_BLOW performs, then EXPLODE.
        return Ok(());
    }
    if monster.kind == MonsterKind::SlumberingBeetle {
        return install_beetle_stun(state, owner);
    }
    if monster.kind == MonsterKind::LagavulinMatriarch
        && monster.override_state == MonsterOverride::None
        && (monster.powers.value(PowerId::Asleep) > 0
            || monster.forced_follow_up == MonsterFollowUp::LagavulinSleep)
    {
        // #3823: SLEEP_MOVE parked ([`lagavulin_sleep_is_parked`]).
        let monster = &mut state.monsters_mut()[owner];
        monster.override_state = MonsterOverride::Stunned;
        monster.forced_follow_up = MonsterFollowUp::LagavulinSleep;
        return generic_stun_state_is_exact(state, owner)
            .then_some(())
            .ok_or(EngineRefusal::MalformedArgs("Lagavulin sleep stun result"));
    }
    if own_stun_is_unperformed(monster) {
        // #3823: the kind's own stun is a `StunInternal` state too,
        // must-perform like any other (`0x11d7cc` IL_0078), so a Whistle
        // landing on it is dropped (`SetMoveImmediate` `0x825f0`
        // IL_000c-IL_001c). See [`own_stun_is_unperformed`].
        return Ok(());
    }
    if monster.override_state != MonsterOverride::None
        || monster.forced_follow_up != MonsterFollowUp::None
    {
        return Err(EngineRefusal::MalformedArgs(
            "generic stun foreign override",
        ));
    }
    if !generic_stun_owner_is_eligible(monster.kind) {
        return Err(EngineRefusal::MalformedArgs("generic stun owner"));
    }
    let follow_up = parked_telegraph(monster.kind, monster.loop_pos)
        .ok_or(EngineRefusal::MalformedArgs("generic stun target machine"))?;
    let monster = &mut state.monsters_mut()[owner];
    monster.override_state = MonsterOverride::Stunned;
    monster.forced_follow_up = follow_up;
    generic_stun_state_is_exact(state, owner)
        .then_some(())
        .ok_or(EngineRefusal::MalformedArgs("generic stun result"))
}

/// Install `CreatureCmd::Stun(creature, null)` on a living Queen or Torch
/// Head Amalgam: `STUNNED` over the follow-up their own two-kind table names
/// ([`queen_amalgam_follow_up`]).
///
/// This is the tail of `steps::neutral::stun_target`, moved here unchanged so
/// that `engine::native_stun_traces` installs the Queen roster's stun
/// through the same code a Whistle does (#3823). The caller has checked that
/// the target is alive and the roster representable
/// ([`whistle_stun_target_is_representable`]). A second stun before the first
/// performs is a no-op, as in [`install_generic_stun`].
pub(crate) fn install_queen_amalgam_stun(
    state: &mut HotState,
    target: usize,
) -> Result<(), EngineRefusal> {
    let Some(monster) = state.monsters.get(target) else {
        return Ok(());
    };
    if monster.override_state == MonsterOverride::Stunned {
        return queen_amalgam_stun_state_is_exact(state, target)
            .then_some(())
            .ok_or(EngineRefusal::MalformedArgs("Whistle existing stun"));
    }
    if monster.override_state != MonsterOverride::None
        || monster.forced_follow_up != MonsterFollowUp::None
    {
        return Err(EngineRefusal::MalformedArgs("Whistle target state"));
    }
    let follow_up = queen_amalgam_follow_up(monster.kind, monster.loop_pos)
        .ok_or(EngineRefusal::MalformedArgs("Whistle target machine"))?;
    let monster = &mut state.monsters_mut()[target];
    monster.override_state = MonsterOverride::Stunned;
    monster.forced_follow_up = follow_up;
    if !queen_amalgam_stun_state_is_exact(state, target) {
        return Err(EngineRefusal::MalformedArgs("Whistle stun result"));
    }
    Ok(())
}

/// Queen/Torch Head Amalgam state admitted at a public action boundary.
///
/// The roster is retained in creation order after death. The two private
/// latches distinguish the ordinary dead-Amalgam loop from the native
/// STUNNED route that must consume one empty Burn before retargeting.
pub(crate) fn queen_roster_state_is_valid(state: &HotState) -> bool {
    queen_stable_roster_state_is_valid(state)
        || (state
            .monsters
            .iter()
            .any(|m| state.fanouts.monster_death_is_pending(m.uid))
            && queen_damage_batch_state_is_valid(state))
        || (state
            .monsters
            .iter()
            .any(|m| state.fanouts.monster_death_cleanup_is_active(m.uid))
            && queen_death_callback_state_is_valid(state))
}

fn queen_stable_roster_state_is_valid(state: &HotState) -> bool {
    if !queen_roster_shape_is_exact(state) {
        return false;
    }
    let [amalgam, queen] = state.monsters.as_slice() else {
        unreachable!("shape checked above")
    };
    if !queen_forced_state_is_exact(amalgam) || !queen_forced_state_is_exact(queen) {
        return false;
    }
    let amalgam_alive = amalgam.hp > 0;
    let queen_alive = queen.hp > 0;
    let dead = queen.queen_amalgam_dead();
    let retained = queen.queen_burn_bright_retained();
    match (queen_alive, amalgam_alive) {
        (true, true) => !dead && !retained && matches!(queen.loop_pos, 0..=2),
        (true, false) => {
            dead && match queen.loop_pos {
                0 | 1 => !retained,
                2 => retained,
                3..=5 => !retained,
                _ => false,
            }
        }
        (false, false) => {
            queen.override_state == MonsterOverride::None
                && queen.forced_follow_up == MonsterFollowUp::None
                && matches!(
                    (dead, retained),
                    (false, false) | (true, false) | (true, true)
                )
        }
        (false, true) => false,
    }
}

fn queen_damage_batch_state_is_valid(state: &HotState) -> bool {
    if !queen_roster_shape_is_exact(state) {
        return false;
    }
    let [amalgam, queen] = state.monsters.as_slice() else {
        return false;
    };
    let pending_amalgam = state.fanouts.monster_death_is_pending(amalgam.uid);
    let pending_queen = state.fanouts.monster_death_is_pending(queen.uid);
    for monster in [amalgam, queen] {
        let mut probe = monster.clone();
        if state.fanouts.monster_death_is_pending(monster.uid) {
            probe.hp = 1;
        }
        if !queen_forced_state_is_exact(&probe) {
            return false;
        }
    }
    if pending_amalgam {
        // The Amalgam callback has not happened, so Queen cannot yet have
        // latched its death. Native creation order drains Amalgam first.
        !queen.queen_amalgam_dead()
            && !queen.queen_burn_bright_retained()
            && (queen.hp > 0 || pending_queen)
            && matches!(queen.loop_pos, 0..=2)
    } else if pending_queen {
        // A surviving secondary still awaits Queen's final Kill cascade.
        // If Amalgam died in this batch, dead Queen ignored that callback.
        if amalgam.hp > 0 {
            !queen.queen_amalgam_dead()
                && !queen.queen_burn_bright_retained()
                && matches!(queen.loop_pos, 0..=2)
        } else {
            matches!(
                (
                    queen.queen_amalgam_dead(),
                    queen.queen_burn_bright_retained()
                ),
                (false, false) | (true, false) | (true, true)
            )
        }
    } else {
        false
    }
}

/// Queen/Torch Head Amalgam state while one native death command is between
/// cleanup and its listener/cascade/loop-successor suffix.
pub(crate) fn queen_roster_internal_state_is_valid(state: &HotState) -> bool {
    queen_roster_state_is_valid(state) || queen_death_callback_state_is_valid(state)
}

fn queen_death_callback_state_is_valid(state: &HotState) -> bool {
    if !queen_roster_shape_is_exact(state) {
        return false;
    }
    let [amalgam, queen] = state.monsters.as_slice() else {
        unreachable!("shape checked above")
    };
    let amalgam_alive = amalgam.hp > 0;
    let queen_alive = queen.hp > 0;
    let dead = queen.queen_amalgam_dead();
    let retained = queen.queen_burn_bright_retained();
    queen_forced_state_is_exact(amalgam)
        && queen_forced_state_is_exact(queen)
        && ((queen_alive && !amalgam_alive && !dead && !retained)
            || (!queen_alive
                && amalgam_alive
                && !dead
                && !retained
                && queen.override_state == MonsterOverride::None
                && queen.forced_follow_up == MonsterFollowUp::None)
            || (queen_alive
                && !amalgam_alive
                && dead
                && !retained
                && queen.loop_pos == 2
                && queen.override_state == MonsterOverride::None
                && queen.forced_follow_up == MonsterFollowUp::None))
}

/// Queen's native `AfterDeath` listener, after card-pile identity
/// normalization and before the secondary cascade/terminal boundary.
/// `_validated_corpse_slug_roster` (frozen Python, deleted #2827).
///
/// The Corpse Slugs encounter is a closed two- or three-slug roster with
/// contiguous slots and uids, per-slug distinct `max_hp` inside the fight's tier band,
/// and the exact Ravenous amount on every slug that is alive — or is the one
/// currently dying, which is why `dying` is a parameter rather than an
/// `hp > 0` test. `_finish_monster_death` (frozen Python, deleted #2827) zeroes the amount, so without
/// that exception the post-death re-validation would reject the very roster it
/// just produced.
pub(crate) fn corpse_slug_roster_is_valid(state: &HotState, dying: Option<usize>) -> bool {
    if !state
        .monsters
        .iter()
        .any(|monster| monster.kind == MonsterKind::CorpseSlug)
    {
        return true;
    }
    let count = state.monsters.len();
    if !(2..=3).contains(&count)
        || state
            .monsters
            .iter()
            .any(|monster| monster.kind != MonsterKind::CorpseSlug)
    {
        return false;
    }
    let contiguous = state
        .monsters
        .iter()
        .enumerate()
        .all(|(index, monster)| monster.slot == index as i32 && monster.uid == index as u32);
    if !contiguous {
        return false;
    }
    // Band and `RavenousPower` amount at this fight's ascension (#2539):
    // HP gates at A8, Ravenous at A9 (`MONSTER_MODELS`).
    let Some(ravenous) = native_initial_power(state, MonsterKind::CorpseSlug, "RavenousPower")
    else {
        return false;
    };
    let mut seen_max_hp: Vec<i32> = Vec::with_capacity(count);
    for (index, monster) in state.monsters.iter().enumerate() {
        if !native_hp_in_band(state, MonsterKind::CorpseSlug, monster.max_hp)
            || seen_max_hp.contains(&monster.max_hp)
        {
            return false;
        }
        seen_max_hp.push(monster.max_hp);
        let expected = if monster.hp > 0
            || dying == Some(index)
            || state.fanouts.monster_death_is_pending(monster.uid)
        {
            ravenous
        } else {
            0
        };
        if monster.hp > monster.max_hp
            || monster.powers.value(PowerId::Ravenous) != expected
            || !(0..crate::engine::admission::CORPSE_SLUG_LOOP_LEN).contains(&monster.loop_pos)
        {
            return false;
        }
    }
    true
}

/// The fixed Kaiser Crab roster and its complete `SurroundedPower` /
/// `CrabRagePower` state (`_validated_kaiser_roster`, frozen Python, deleted #2827).
///
/// Current-build IL (v0.111.0, DLL `9cb4f1ad`). `Crusher/<AfterAddedToRoom>d__36::MoveNext`
/// `0x3570b8` applies `BackAttackLeftPower` (IL_009f) then `CrabRagePower`
/// (IL_0112) to its own `Creature`; `Rocket/<AfterAddedToRoom>d__29::MoveNext`
/// `0x367c94` applies `SurroundedPower` to
/// `CombatState.GetOpponentsOf(rocket)` — the PLAYER — at IL_00ae, then
/// `BackAttackRightPower` (IL_0121) and `CrabRagePower` (IL_0197) to itself.
/// Nothing else in the build installs any of the four, so the pair is the only
/// shape that can carry them, and a document carrying one without the pair is
/// forged rather than merely unrepresented.
///
/// `dying` is the index whose death is currently being finished. Like Corpse
/// Slug's roster, a corpse must not retain `crab_rage` — but the creature
/// whose own `_finish_monster_death` has not yet cleared it, and a receiver
/// inside a powered batch whose death is still queued, both legitimately do.
pub(crate) fn kaiser_roster_is_valid(state: &HotState, dying: Option<usize>) -> bool {
    use crate::engine::admission::KAISER_LOOP_LEN;
    let has_kaiser = state
        .monsters
        .iter()
        .any(|monster| matches!(monster.kind, MonsterKind::Crusher | MonsterKind::Rocket));
    if !has_kaiser {
        // `SurroundedPower` and `CrabRagePower` have no other installer.
        return state.fanouts.kaiser_facing() == -1
            && !state.monsters.iter().any(HotMonster::crab_rage);
    }
    let [crusher, rocket] = state.monsters.as_slice() else {
        return false;
    };
    if (crusher.kind, rocket.kind) != (MonsterKind::Crusher, MonsterKind::Rocket)
        || (crusher.slot, rocket.slot) != (0, 1)
        || (crusher.uid, rocket.uid) != (0, 1)
    {
        return false;
    }
    // The fixed HP pair at this fight's ascension (#2539): 219/209 at A8+,
    // 209/199 below. The move tier is not yet selected (the `KAISER_LOOP_LEN`
    // doc in `admission`). Python pins the identical tier-selected pair
    // (`_validated_kaiser_roster`, frozen Python, deleted #2827).
    if (Some(crusher.max_hp), Some(rocket.max_hp))
        != (
            native_fixed_hp(state, MonsterKind::Crusher),
            native_fixed_hp(state, MonsterKind::Rocket),
        )
        || !(0..=1).contains(&state.fanouts.kaiser_facing())
    {
        return false;
    }
    let live_count = state
        .monsters
        .iter()
        .filter(|monster| monster.hp > 0)
        .count();
    for (index, monster) in state.monsters.iter().enumerate() {
        let corpse_may_keep_rage =
            dying == Some(index) || state.fanouts.monster_death_is_pending(monster.uid);
        if monster.hp > monster.max_hp
            || !(0..KAISER_LOOP_LEN).contains(&monster.loop_pos)
            || (monster.hp <= 0 && monster.crab_rage() && !corpse_may_keep_rage)
            || (live_count == 2 && !monster.crab_rage())
        {
            return false;
        }
    }
    true
}

/// `SurroundedPower.UpdateDirection` for ONE targeted owner action
/// (`_update_kaiser_facing`, frozen Python, deleted #2827).
///
/// Current-build IL `SurroundedPower/<UpdateDirection>d__14::MoveNext`
/// `0x347a5c`: `switch (Facing)`, where facing 0 flips to 1 only when the
/// target has `BackAttackLeftPower` (IL_0037-IL_0049) and facing 1 flips to 0
/// only when it has `BackAttackRightPower` (IL_00a5-IL_00b4); the flip itself
/// is `FaceDirection`'s `set_Facing` (`0x34770c` IL_0027-IL_002e). Every other
/// combination, including a `Direction` outside the two-member enum, is a
/// no-op.
///
/// Keying on the target's KIND rather than on a `BackAttack*` power slot is
/// exact for the validated roster: the two markers have no other installer,
/// and a dead crab carries neither. `Creature.RemoveAllPowersAfterDeath`
/// `0x11dbac` keeps a power only when BOTH conjuncts of its lambda
/// (`0x3dad34`) allow it — `ShouldPowerBeRemovedAfterOwnerDeath`, whose base
/// `0x840dd` returns 1, and `Hook::ShouldPowerBeRemovedOnDeath` `0x106a34`,
/// which folds every listener's `AbstractModel::ShouldPowerBeRemovedOnDeath`
/// (base `0x7a341`, returns 1). Neither marker overrides the first, and the
/// only override of the second is `IllusionPower::ShouldPowerBeRemovedOnDeath`
/// `0xa3a80`, which vetoes only a `PowerModel.Type == 2` non-`ITemporaryPower`
/// — while both markers are Type 1 (`0x9fa83`, `0x9fa91`). So even an Illusion
/// in the room cannot retain them, and a dead target flips nothing.
///
/// Every targeted card play and potion use in EVERY fight reaches this, so the
/// non-Kaiser cost is deliberately the first line: one cold-pointer load and a
/// compare against the absent sentinel, ahead of the roster validator's two
/// O(len(monsters)) scans. The skipped half of that validator — `crab_rage` on
/// a non-crab with no facing — is unreachable rather than unchecked: the
/// boundary refuses that write by name on any kind but Crusher/Rocket, so no
/// `HotState` can carry it, and `admit`'s roster-level arm re-checks it once
/// per document.
pub(crate) fn update_kaiser_facing(
    state: &mut HotState,
    target: Option<usize>,
) -> Result<(), EngineRefusal> {
    let facing = state.fanouts.kaiser_facing();
    if facing < 0 {
        return Ok(());
    }
    if !kaiser_roster_is_valid(state, None) {
        return Err(EngineRefusal::MalformedArgs("Kaiser Crab roster"));
    }
    let Some(target) = target.and_then(|index| state.monsters.get(index)) else {
        return Ok(());
    };
    if target.hp <= 0 {
        return Ok(());
    }
    let flipped = match (facing, target.kind) {
        (0, MonsterKind::Crusher) => 1,
        (1, MonsterKind::Rocket) => 0,
        _ => return Ok(()),
    };
    if !state.fanouts.set_kaiser_facing(flipped) {
        return Err(EngineRefusal::MalformedArgs("Kaiser Crab facing"));
    }
    Ok(())
}

/// Whether `SurroundedPower.ModifyDamageMultiplicative` contributes its one
/// exact 3/2 factor to this in-flight monster hit
/// (`_kaiser_back_attack_multiplier`, frozen Python, deleted #2827).
///
/// Current-build IL `SurroundedPower::ModifyDamageMultiplicative` `0xa8bc4`:
/// the listener returns `Decimal.One` unless the damage RECEIVER is its own
/// owner — the player (IL_0016-IL_0024) — and the SOURCE is non-null
/// (IL_000c-IL_0015). It then switches on `Facing`: facing 0 needs the source
/// to hold `BackAttackLeftPower` (IL_0035-IL_0044), facing 1 needs
/// `BackAttackRightPower` (IL_0046-IL_0055), and either way the factor is the
/// `Decimal(15, 0, 0, false, 1)` built at IL_0057 — exactly 3/2.
///
/// False for every state outside the Kaiser Crab encounter, where no
/// `SurroundedPower` instance exists and the facing sentinel is -1.
pub(crate) fn kaiser_back_attacks(state: &HotState, dealer: &HotMonster) -> bool {
    let facing = state.fanouts.kaiser_facing();
    dealer.hp > 0
        && matches!(
            (facing, dealer.kind),
            (0, MonsterKind::Crusher) | (1, MonsterKind::Rocket)
        )
}

/// `SurroundedPower.AfterDeath` on one ACTUAL enemy death
/// (`_kaiser_surrounded_after_actual_death`, frozen Python, deleted #2827).
///
/// Current-build IL `SurroundedPower/<AfterDeath>d__13::MoveNext` `0x3473a8`
/// returns on a prevented removal (IL_0020-IL_0028), on a death on the
/// listener's OWN side (IL_002d-IL_0045, where `bne.un` is the continue edge),
/// and on an empty `ICombatState.HittableEnemies` (IL_004a-IL_0063). It then
/// re-faces toward `enemies[0]` when every remaining hittable enemy carries
/// `BackAttackLeftPower` (IL_0068-IL_008d) or every one carries
/// `BackAttackRightPower` (IL_008f-IL_00b4), through the same `UpdateDirection`
/// (IL_00b6-IL_00be) a targeted action takes.
///
/// `HittableEnemies` is `CombatState.Enemies.Where(IsHittable)` (`0x1373a5`),
/// so a corpse is never a member — the `hp > 0` filter below is that `Where`.
/// For the validated two-crab roster "all carry one side's marker" is
/// precisely "the survivors are all Crusher or all Rocket", which is why one
/// live crab always re-faces onto itself and a back attack can never outlive
/// its sibling.
pub(crate) fn kaiser_surrounded_after_actual_death(
    state: &mut HotState,
    dying: usize,
) -> Result<(), EngineRefusal> {
    if !matches!(
        state.monsters.get(dying).map(|monster| monster.kind),
        Some(MonsterKind::Crusher | MonsterKind::Rocket)
    ) {
        return Ok(());
    }
    if !kaiser_roster_is_valid(state, Some(dying)) {
        return Err(EngineRefusal::MalformedArgs("Kaiser Crab roster"));
    }
    let live: Vec<usize> = state
        .monsters
        .iter()
        .enumerate()
        .filter_map(|(index, monster)| (monster.hp > 0).then_some(index))
        .collect();
    let Some(first) = live.first().copied() else {
        return Ok(());
    };
    let kind = state.monsters[first].kind;
    if live.iter().any(|index| state.monsters[*index].kind != kind) {
        return Ok(());
    }
    update_kaiser_facing(state, Some(first))
}

/// `CrabRagePower.AfterDeath` for the Kaiser Crab roster
/// (`_content_monsters_after_actual_death`, frozen Python, deleted #2827).
///
/// Current-build IL `CrabRagePower/<AfterDeath>d__8::MoveNext` `0x337f54`
/// gates on the dying creature being someone else (IL_002c-IL_003a) and on the
/// same side (IL_003f-IL_0057) — and, unlike `RavenousPower` (`0x341844`
/// IL_0069-IL_0076), it does NOT test its own owner's death. It then serially
/// applies `StrengthPower` with the owner as applier (IL_0062-IL_008b),
/// `GainBlock` (IL_00e6-IL_00f9) and `PowerCmd.Remove(this)` (IL_0154). Both
/// amounts come from `CrabRagePower::get_CanonicalVars` `0xa0e0b`: Strength 6,
/// a `BlockVar` of 99.
///
/// The missing owner-death gate is nevertheless unobservable, which is why the
/// walk below skips a dead sibling exactly as Python does: `PowerCmd.Apply`
/// (`0x3efbac` IL_0061-IL_006e) returns when the target's
/// `Creature.CanReceivePowers` is false, and that property is
/// `Hook.ShouldAllowHitting` (`0x11d2b5`) — the same predicate `IsHittable`
/// takes; and `CreatureCmd.GainBlock` (`0x3eaec0` IL_0046-IL_005b) returns a
/// zero `Decimal` on a dead creature, after an identical early return for
/// `CombatManager.IsOverOrEnding` (IL_002d-IL_0041). A same-batch corpse
/// therefore takes neither half of the rage.
///
/// This is why #2655's powered batch phases are the prerequisite: a killing
/// AoE commits HP to BOTH crabs in phase 1, so by the time this hook runs in
/// phase 3 the sibling is already dead and is shielded by nothing. A per-target
/// commit/death loop would have handed the second crab Block 99 before its own
/// damage landed.
pub(crate) fn crab_rage_after_death(
    state: &mut HotState,
    dying: usize,
    events: &mut Vec<Event>,
) -> Result<(), EngineRefusal> {
    if !matches!(
        state.monsters.get(dying).map(|monster| monster.kind),
        Some(MonsterKind::Crusher | MonsterKind::Rocket)
    ) {
        return Ok(());
    }
    if !kaiser_roster_is_valid(state, Some(dying)) {
        return Err(EngineRefusal::MalformedArgs("Kaiser Crab roster"));
    }
    for index in 0..state.monsters.len() {
        let monster = &state.monsters[index];
        if index == dying || monster.hp <= 0 || !monster.crab_rage() {
            continue;
        }
        let uid = monster.uid;
        let strength = monster
            .powers
            .value(PowerId::Strength)
            .checked_add(crate::engine::admission::CRAB_RAGE_STRENGTH)
            .ok_or(EngineRefusal::CounterOverflow("CrabRage strength"))?;
        let block = monster
            .block
            .checked_add(crate::engine::admission::CRAB_RAGE_BLOCK)
            .ok_or(EngineRefusal::CounterOverflow("CrabRage block"))?;
        let upkeep = state.fanouts.misery_attachment_upkeep();
        crate::engine::damage::write_monster_strength(
            &mut state.monsters_mut()[index],
            strength,
            crate::hot::Applier::Monster(uid),
            upkeep,
        );
        crate::engine::damage::note_power(
            events,
            Subject::Monster(uid),
            PowerId::Strength,
            strength,
        );
        let monster = &mut state.monsters_mut()[index];
        monster.block = block;
        monster.set_crab_rage(false);
    }
    if !kaiser_roster_is_valid(state, Some(dying)) {
        return Err(EngineRefusal::MalformedArgs("Kaiser Crab rage result"));
    }
    Ok(())
}

pub(crate) fn queen_after_monster_death(
    state: &mut HotState,
    dying_kind: MonsterKind,
) -> Result<(), EngineRefusal> {
    if !state.monsters.iter().any(|monster| {
        matches!(
            monster.kind,
            MonsterKind::Queen | MonsterKind::TorchHeadAmalgam
        )
    }) {
        return Ok(());
    }
    if !queen_roster_internal_state_is_valid(state) {
        return Err(EngineRefusal::MalformedArgs(
            "Queen/Amalgam death lifecycle",
        ));
    }
    let queen = &mut state.monsters_mut()[1];
    match dying_kind {
        MonsterKind::TorchHeadAmalgam if queen.hp > 0 => {
            if queen.queen_amalgam_dead() {
                return Err(EngineRefusal::MalformedArgs(
                    "Queen duplicate Amalgam death latch",
                ));
            }
            queen.set_queen_amalgam_dead(true);
            if queen.loop_pos == 2 {
                if (queen.override_state, queen.forced_follow_up)
                    == (MonsterOverride::Stunned, MonsterFollowUp::QueenBurnBright)
                {
                    queen.set_queen_burn_bright_retained(true);
                } else if (queen.override_state, queen.forced_follow_up)
                    == (MonsterOverride::None, MonsterFollowUp::None)
                {
                    queen.loop_pos = 5;
                } else {
                    return Err(EngineRefusal::MalformedArgs(
                        "Queen Burn death retarget state",
                    ));
                }
            }
        }
        MonsterKind::Queen => {
            queen.override_state = MonsterOverride::None;
            queen.forced_follow_up = MonsterFollowUp::None;
        }
        MonsterKind::TorchHeadAmalgam => {}
        _ => return Ok(()),
    }
    if !queen_roster_state_is_valid(state) && !queen_roster_internal_state_is_valid(state) {
        return Err(EngineRefusal::MalformedArgs("Queen/Amalgam death result"));
    }
    Ok(())
}

/// SlumberingBeetle::GenerateMoveStateMachine 0xbe5f4 and SlumberPower
/// bodies 0x34500c/0x34519c. Only the sleep counter and pending damage wake
/// affect combat; IsAwake is an animation predicate. The native power starts
/// at three, never stacks above it, and belongs only to this creature.
pub(crate) fn beetle_state_is_valid(monster: &HotMonster) -> bool {
    if monster.kind != MonsterKind::SlumberingBeetle {
        return monster.powers.value(PowerId::Slumber) == 0
            && !matches!(
                monster.override_state,
                MonsterOverride::BeetleSnore | MonsterOverride::BeetleWake
            );
    }
    // #2647 — the awake Beetle's Imbalanced self-stun, the one state here
    // that carries a follow-up. A corpse never holds it: death cleanup clears
    // the override, as `StunInternal` `0x11d7cc` IL_0028 refuses a corpse.
    // #3823: a Whistle's stun parks SNORE_MOVE as well, and the walk out of it
    // shows SNORE_MOVE once more ([`beetle_snore_is_parked`]).
    if monster.override_state == MonsterOverride::Stunned {
        return beetle_rollout_stun_is_exact(monster) || beetle_snore_is_parked(monster);
    }
    if beetle_snore_is_parked(monster) {
        return true;
    }
    if monster.loop_pos != 0 || monster.forced_follow_up != MonsterFollowUp::None {
        return false;
    }
    let slumber = monster.powers.value(PowerId::Slumber);
    match monster.override_state {
        MonsterOverride::BeetleSnore => (1..=3).contains(&slumber),
        MonsterOverride::BeetleWake => slumber == 0,
        // #3823: Plating outlives a wake by damage that a stun dropped
        // (`damage::damage_monster`; native trace `bee_s3b0st3`: the awake
        // Beetle keeps its decaying Plating).
        MonsterOverride::None => slumber == 0,
        _ => false,
    }
}

/// SlumberingBeetle/<WakeUpMove>d__24::MoveNext 0x36ae8c removes
/// Plating after its cosmetic animation. It does not remove existing Block.
/// Damage installs this as the next (nonattacking) action; natural expiry
/// invokes it immediately at enemy side end, before the next RollMove.
pub(crate) fn wake_beetle(monster: &mut HotMonster, events: &mut Vec<Event>) {
    if monster.powers.value(PowerId::Mplating) > 0 {
        monster.powers.set(PowerId::Mplating, SlotWire::Int, 0);
        super::damage::note_power(events, Subject::Monster(monster.uid), PowerId::Mplating, 0);
    }
    monster.override_state = MonsterOverride::None;
}

/// `SewerClam/<AfterAddedToRoom>d__9::MoveNext` (RVA `0x368f9c`) IL_007f-IL_00a2
/// applies `PlatingPower(GetValueIfAscension(8, 9, 8))` inline, with no
/// getter. The generator reads that inline-local spelling (#2534), so the
/// amount is the Sewer Clam `MONSTER_MODELS` row's `PlatingPower` (#2852).
/// Resolved during constant evaluation ([`model_initial_power_tier`]), so the
/// Plating cap costs no table scan where it is checked.
const SEWER_CLAM_PLATING: AscensionTier =
    model_initial_power_tier(MonsterKind::SewerClam, "PlatingPower");

/// Each Plating owner's entry amount at this fight's ascension — the ceiling
/// its Plating decrements from (#2539). Frog Knight (8, 19, 15), Slumbering
/// Beetle (8, 18, 15), Sewer Clam (8, 9, 8) and Mysterious Knight (untiered
/// 6) are all `MONSTER_MODELS` rows, Sewer Clam's through
/// [`SEWER_CLAM_PLATING`]; Lagavulin Matriarch's 12 is a plain
/// `ldc.i4.s 12` in `<Sleep>d__40::MoveNext` `0x3613b8` IL_00ab.
pub(crate) fn monster_plating_cap(state: &HotState, kind: MonsterKind) -> Option<i32> {
    match kind {
        MonsterKind::FrogKnight | MonsterKind::SlumberingBeetle | MonsterKind::MysteriousKnight => {
            native_initial_power(state, kind, "PlatingPower")
        }
        MonsterKind::SewerClam => {
            Some(crate::encounters::tier(SEWER_CLAM_PLATING, state.fanouts.ascension()) as i32)
        }
        MonsterKind::LagavulinMatriarch => Some(12),
        _ => None,
    }
}

/// Refuse malformed monster-owned turn listeners before `end_player_turn`
/// publishes or mutates any turn state. Every represented Plating owner starts
/// at its fixed cap, then monotonically decrements to zero. Conqueror is an
/// admitted nonnegative integer duration; return whether a living positive
/// owner exists so the later side-end walk can retain its live re-read without
/// rescanning an entry that proved the card-sourced power absent.
pub(crate) fn require_monster_turn_entry_state(state: &HotState) -> Result<bool, EngineRefusal> {
    let mut conqueror_reachable = false;
    let mut conqueror_malformed = false;
    for monster in state.monsters.iter() {
        let knockdown_reachable = !monster.misery_debuff_order.knockdown().is_empty()
            || monster.powers.get(PowerId::Knockdown).is_some()
            || monster
                .misery_debuff_order
                .as_slice()
                .contains(&crate::hot::MiseryToken::Knockdown);
        if knockdown_reachable
            && (monster.hp <= 0
                || !super::damage::knockdown_state_is_exact(monster)
                || super::damage::misery_scalar_snapshot(monster).is_err())
        {
            return Err(EngineRefusal::MalformedArgs(
                "Knockdown distinct-instance state",
            ));
        }
        if let Some(slot) = monster.powers.get(PowerId::Conqueror) {
            if slot.wire != SlotWire::Int || slot.value < 0 {
                conqueror_malformed = true;
            } else {
                conqueror_reachable |= monster.hp > 0 && slot.value > 0;
            }
        }
        let plating = monster.powers.value(PowerId::Mplating);
        if plating != 0
            && (monster.hp <= 0
                || monster.powers.value(PowerId::Burrowed) != 0
                || monster_plating_cap(state, monster.kind)
                    .is_none_or(|cap| !(1..=cap).contains(&plating)))
        {
            return Err(EngineRefusal::MalformedArgs("monster Plating owner/state"));
        }
        // Lagavulin Matriarch's AsleepPower owns three enemy-side bodies
        // (#2814, `turn.rs`): its SLEEP_MOVE turn, the `…VeryEarly` Plating
        // removal and the `AfterSideTurnEnd` decrement with its natural wake.
        // They are ported for the only native carrier, a living Matriarch
        // (`LagavulinMatriarch/<Sleep>d__40` RVA `0x3613b8` `IL_0131` is the
        // sole `Apply<AsleepPower>`). A corpse or any other kind carrying it is
        // outside that proof and refuses before any mutation. The decrement is
        // a null-applier `PowerCmd::ModifyAmount`, so its
        // `AfterPowerAmountChanged` listener order is proven here too, before
        // the turn end publishes anything, as Regen's is.
        let asleep = monster.powers.value(PowerId::Asleep);
        if asleep != 0 {
            if asleep < 0 || monster.hp <= 0 || monster.kind != MonsterKind::LagavulinMatriarch {
                return Err(EngineRefusal::MalformedArgs("Asleep owner/state"));
            }
            super::damage::null_applier_power_amount_changed_is_exact(state)?;
        }
    }
    if conqueror_malformed {
        return Err(EngineRefusal::MalformedArgs("Conqueror power state"));
    }
    Ok(conqueror_reachable)
}

/// The Kin boss roster's identity quotient (#3191): what stays true of the
/// three creatures for the whole fight, dead or alive.
///
/// Current-build IL (v0.111.0, DLL
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`),
/// re-derived for #3191 with `sim/v0.111.0/python/tools/dump_il.py`:
///
/// * **Roster.** `TheKinBoss::GenerateMonsters` (`0xd56c4`) builds exactly
///   three creatures and nothing in the fight spawns another: `KinFollower`
///   into `'slot1'` (`IL_000c`-`IL_002b`, with `set_StartsWithDance(true)` at
///   `IL_001e`), a second `KinFollower` into `'slot2'` (`IL_003d`) and
///   `KinPriest` into `'leaderSlot'` (`IL_0058`). Creation order is slot
///   order, so the uids are `0, 1, 2` (`encounters::boss::build_the_kin_boss`).
/// * **HP.** `KinFollower::get_MinInitialHp`/`get_MaxInitialHp` (`0xb707b` /
///   `0xb7087`) are `GetValueIfAscension(8, 62, 58)` / `(8, 63, 59)`, a
///   two-value band drawn twice through `SetUniqueMonsterHpValue`, so the two
///   followers' `max_hp` are the two distinct band members.
///   `KinPriest::get_MinInitialHp` (`0xb7415`, `GetValueIfAscension(8, 199,
///   190)`) is its own max (`0xb7427`), so the Priest's is fixed.
/// * **Secondary.** `KinFollower/<AfterAddedToRoom>d__31::MoveNext`
///   (`0x35f954`) applies `MinionPower` with `Decimal.One` (`IL_008a`-`IL_0097`);
///   `MinionPower::get_OwnerIsSecondaryEnemy` (`0xa4a48`) returns 1 and
///   `ShouldPowerBeRemovedAfterOwnerDeath` (`0xa4a4b`) returns 0, so a
///   follower stays secondary as a corpse. `KinPriest` has no
///   `AfterAddedToRoom` override and carries no `MinionPower`: it is the one
///   primary enemy.
/// * **Machines.** `KinFollower::GenerateMoveStateMachine` (`0xb7160`) is the
///   fixed cycle QUICK_SLASH -> BOOMERANG -> POWER_DANCE -> QUICK_SLASH
///   (`IL_008e`-`IL_009e`); `KinPriest::GenerateMoveStateMachine` (`0xb7564`)
///   is ORB_OF_FRAILTY -> ORB_OF_WEAKNESS -> BEAM -> RITUAL -> ORB_OF_FRAILTY
///   (`IL_00cc`-`IL_00e5`), starting on ORB_OF_FRAILTY (`IL_0108`). Neither
///   has a conditional or random branch, so `loop_pos` is inside the
///   generated loop, and nothing in the encounter installs an override or a
///   forced follow-up on these kinds. A stun or other dynamic move would be
///   one; it is outside this port and refuses by name here.
pub(crate) fn kin_roster_shape_is_exact(state: &HotState) -> bool {
    let [first, second, priest] = state.monsters.as_slice() else {
        return false;
    };
    let follower_len =
        crate::content_tables::monster_loop(MonsterKind::KinFollower).map_or(0, <[_]>::len);
    let priest_len =
        crate::content_tables::monster_loop(MonsterKind::KinPriest).map_or(0, <[_]>::len);
    let secondary = |monster: &HotMonster| {
        monster
            .powers
            .get(PowerId::Secondary)
            .is_some_and(|slot| slot.wire == SlotWire::Bool && slot.value == 1)
    };
    let fixed_machine = |monster: &HotMonster, len: usize| {
        usize::try_from(monster.loop_pos).is_ok_and(|pos| pos < len)
            && monster.hp <= monster.max_hp
            && monster.random_ai.is_empty()
            && machine_is_undisturbed_or_generic_stun(monster)
            && !monster.spawn_noop
            && monster.last_spawned.is_none()
            && monster.revive_stage == 0
    };
    (first.kind, second.kind, priest.kind)
        == (
            MonsterKind::KinFollower,
            MonsterKind::KinFollower,
            MonsterKind::KinPriest,
        )
        && (first.slot, second.slot, priest.slot) == (0, 1, 2)
        && (first.uid, second.uid, priest.uid) == (0, 1, 2)
        && native_hp_in_band(state, MonsterKind::KinFollower, first.max_hp)
        && native_hp_in_band(state, MonsterKind::KinFollower, second.max_hp)
        && first.max_hp != second.max_hp
        && Some(priest.max_hp) == native_fixed_hp(state, MonsterKind::KinPriest)
        && secondary(first)
        && secondary(second)
        && priest.powers.get(PowerId::Secondary).is_none()
        && fixed_machine(first, follower_len)
        && fixed_machine(second, follower_len)
        && fixed_machine(priest, priest_len)
}

/// The Kin boss roster at a public action boundary (#3191): the identity
/// quotient of [`kin_roster_shape_is_exact`], plus the one liveness fact the
/// death cascade makes impossible to violate.
///
/// A Priest corpse beside a live Follower cannot be published. (A corpse's
/// HP may be negative: a lethal Thorns retaliation publishes the overkill,
/// which is why neither predicate floors HP at zero.)
/// `CreatureCmd/<KillWithoutCheckingWinCondition>d__15::MoveNext` (`0x3ebe90`)
/// snapshots `IsPrimaryEnemy` (`IL_04e1`) before the kill and, for a dying
/// enemy-side primary (`IL_05ac`-`IL_05c3`) whose living teammates
/// (`<>c::<KillWithoutCheckingWinCondition>b__15_0` `0x3e8f73`, `IsAlive`) are
/// non-empty and all secondary (`b__15_1` `0x3e8f7b`, `IsSecondaryEnemy`;
/// `IL_05c8`-`IL_0602`), kills every one of them (`IL_0607`-`IL_060e`,
/// `CreatureCmd::Kill`). The Priest is the only primary, so its death takes
/// both Followers with it: `damage::finish_secondary_death_cascade`.
///
/// The reverse direction has no gameplay. `KinPriest::AfterDeath` (`0xb7478`)
/// reacts to a dying `KinFollower` (`IL_000c`-`IL_0017`) only while the Priest
/// lives (`IL_001c`-`IL_0027`): a music parameter (`IL_003a`-`IL_0044`) and,
/// once no Follower is alive (`<>c::<AfterDeath>b__35_0` `0x35ff1a`),
/// `AllFollowerDeathResponse` (`0xb785b`), which is one `TalkCmd::Play`
/// speech bubble (`0x133fe8`: `NSpeechBubbleVfx`, no RNG, no power). On its
/// own death it sets only music (`IL_00c2`-`IL_00e0`).
/// `MinionPower::ShouldOwnerDeathTriggerFatal` (`0xa4a4e`) returns 0, which
/// the shared `Secondary` projection already carries to every Fatal reader.
/// Nothing in the build revives or respawns a Kin creature.
pub(crate) fn kin_roster_is_valid(state: &HotState) -> bool {
    let [first, second, priest] = state.monsters.as_slice() else {
        return false;
    };
    kin_roster_shape_is_exact(state) && (priest.hp > 0 || (first.hp <= 0 && second.hp <= 0))
}

/// Whether any creature of the Kin boss roster is present.
pub(crate) fn kin_roster_reachable(state: &HotState) -> bool {
    state.monsters.iter().any(|monster| {
        matches!(
            monster.kind,
            MonsterKind::KinFollower | MonsterKind::KinPriest
        )
    })
}

/// The admitted Turret Operator encounter is one fixed native pair at the
/// fight's ascension (#2539): `LivingShield::get_MinInitialHp` is
/// `GetValueIfAscension(8, 65, 55)` and `TurretOperator::get_MinInitialHp`
/// `(8, 51, 41)`; both native max getters delegate to their min getter.
///
/// Native authority: v0.111.0 DLL SHA256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`,
/// `TurretOperatorWeak::GenerateMonsters` RVA `0xd59ac` emits the Shield
/// before the Operator without spawning, and `LivingShield::AfterAddedToRoom`
/// RVA `0x362430` installs `Rampart(25)` on that first member.
///
/// `LivingShieldSlam` is intentionally absent here.  It is an internal
/// completed-move receipt, written and consumed during one enemy phase before
/// `EndTurn` can publish a stable state; accepting it from canonical input
/// would turn a forged transient into a public branch selector.
pub(crate) fn turret_operator_state_is_valid(state: &HotState) -> bool {
    turret_operator_roster_is_exact(state)
}

/// [`turret_operator_state_is_valid`] for the in-phase Living Shield
/// completed-Slam receipt (`turn::monster_act`), which also accepts an
/// Operator corpse carrying negative (overkill) HP (#3287).
///
/// A Rust corpse keeps the HP its killing hit left, so an Operator overkilled
/// earlier in the fight (Fiend Fire on turn one, 1KJJGR1GFZR6 node 37) reads
/// below zero while the Shield performs `SHIELD_SLAM_MOVE`. The boundary
/// validator rejects that HP, but nothing the receipt feeds reads it: the
/// successor roll in `turn::prepare_random_ai` asks only `hp > 0`, exactly
/// native's `LivingShield/<GetAllyCount>b__18_0` (v0.111.0 DLL SHA256
/// `9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`, RVA
/// `0xb8e0d` `IL_0001`-`IL_0017`: count teammates that are `IsAlive` and not
/// the Shield itself), whose zero count makes `<GenerateMoveStateMachine>
/// b__15_1` (RVA `0xb8e02` `IL_0002`-`IL_0008`, `GetAllyCount() == 0`) select
/// `SMASH_MOVE` (`GenerateMoveStateMachine` RVA `0xb8c70` `IL_0095`-`IL_00a3`).
/// Every other field keeps the public check.
///
/// Since #3823 the public check has no HP floor either (a corpse's HP is
/// read by nobody, see [`turret_operator_roster_is_exact`]), so the two
/// forms are the same predicate; the name stays for the receipt's call site.
pub(crate) fn turret_operator_slam_roster_is_exact(state: &HotState) -> bool {
    turret_operator_roster_is_exact(state)
}

fn turret_operator_roster_is_exact(state: &HotState) -> bool {
    let [shield, operator] = state.monsters.as_slice() else {
        return false;
    };
    let shield_live = shield.hp > 0;
    // #3823: a corpse keeps the HP its killing hit left, which is below zero
    // after an overkill, and the document carries it. Nothing reads a
    // corpse's HP but liveness (`hp > 0`): Rampart's owner check below, the
    // Shield's ally count (`turn::prepare_random_ai`), the grant
    // (`turn::rampart_after_player_side_start`). So neither member has an HP
    // floor at the public boundary. Before this the public form refused an
    // overkilled corpse, which made every state after such a kill fail to
    // reload (the Shield or the Operator killed by more than its remaining
    // HP, with or without a stun in the fight). The in-phase receipt form
    // had lifted the floor for the Operator alone (#3287).
    shield.kind == MonsterKind::LivingShield
        && operator.kind == MonsterKind::TurretOperator
        && (shield.slot, shield.uid, Some(shield.max_hp))
            == (0, 0, native_fixed_hp(state, MonsterKind::LivingShield))
        && (operator.slot, operator.uid, Some(operator.max_hp))
            == (1, 1, native_fixed_hp(state, MonsterKind::TurretOperator))
        && shield.hp <= shield.max_hp
        && operator.hp <= operator.max_hp
        && (0..=MONSTER_BLOCK_CAP).contains(&shield.block)
        && (0..=MONSTER_BLOCK_CAP).contains(&operator.block)
        && shield.random_ai.is_empty()
        && operator.random_ai.is_empty()
        // #3823: either member may carry exactly the generic stun. The
        // Shield's completed-Slam receipt is a follow-up without an override,
        // which this still refuses.
        && machine_is_undisturbed_or_generic_stun(shield)
        && machine_is_undisturbed_or_generic_stun(operator)
        && !shield.spawn_noop
        && !operator.spawn_noop
        && shield.last_spawned.is_none()
        && operator.last_spawned.is_none()
        && shield.revive_stage == 0
        && operator.revive_stage == 0
        && matches!(shield.loop_pos, 0 | 1)
        && matches!(operator.loop_pos, 0..=2)
        && shield.powers.value(PowerId::Rampart) == if shield_live { 25 } else { 0 }
        && operator.powers.value(PowerId::Rampart) == 0
}

/// The complete fixed Frog Knight encounter at a serialized player-action
/// boundary. Plating starts at 19 and decrements once per completed enemy-side
/// start from round two on — round one's side start does not decrement
/// (`PlatingPower/<AfterSideTurnStart>d__14` RVA `0x340584` `IL_0069`-`IL_0085`,
/// see `turn::gain_monster_plating_block`, #2809) — so round `r` holds
/// `19` for `r <= 2` and `21 - r` after. The one-shot charge latch may be live
/// after BEETLE_CHARGE, but never while that same move is still the current
/// intent.
pub(crate) fn frog_knight_state_is_valid(state: &HotState) -> bool {
    let [frog] = state.monsters.as_slice() else {
        return false;
    };
    // Plating's entry amount is `GetValueIfAscension(8, 19, 15)` (#2539).
    let Some(plating) = native_initial_power(state, MonsterKind::FrogKnight, "PlatingPower") else {
        return false;
    };
    let expected_plating = (plating + 2 - i32::from(state.history.round_number)).clamp(0, plating);
    frog.kind == MonsterKind::FrogKnight
        && (frog.slot, frog.uid, Some(frog.max_hp))
            == (0, 0, native_fixed_hp(state, MonsterKind::FrogKnight))
        && (1..=frog.max_hp).contains(&frog.hp)
        && matches!(frog.loop_pos, 0..=3)
        && frog.random_ai.is_empty()
        // #3823: the Frog Knight takes the generic stun, which writes the
        // move state alone.
        && machine_is_undisturbed_or_generic_stun(frog)
        && !frog.spawn_noop
        && frog.revive_stage == 0
        && frog.powers.value(PowerId::Mplating) == expected_plating
        && !(frog.loop_pos == 3 && frog.louse_curled())
}

/// The uid of the Axebot body after `respawn_count` Stock respawns (#3039).
///
/// The first body is the encounter's creature 0. Each respawn is a fresh
/// `CreatureCmd` creation ([`allocate_creature_uid`]): untracked, uid
/// `respawn_count`; tracked, the last id the counter handed out, which a
/// combat-start pet has pushed past `respawn_count`.
fn axebot_expected_uid(state: &HotState, respawn_count: i32) -> Option<u32> {
    let respawn_count = u32::try_from(respawn_count).ok()?;
    if respawn_count == 0 {
        return Some(0);
    }
    match tracked_lineage_start(state, 1) {
        None => Some(respawn_count),
        Some(last) => last.filter(|uid| *uid >= respawn_count),
    }
}

/// The roster index of the Axebot body that REPLACED the Creature a persisted
/// CardPlay froze as `(slot, uid)`, or `None` when that identity is not an
/// earlier body of this fight's Axebot (#3747).
///
/// `StockPower/<AfterDeath>d__4::MoveNext` RVA `0x346280` adds the fresh
/// Axebot in the dead owner's slot (IL_0073-IL_0090) and the corpse then
/// leaves combat (see [`crate::steps::templates::stock_replaced_target_is_current`]
/// for the removal IL), so a play that killed a Stock body still holds a
/// target that no roster row carries. The roster must be the valid singleton
/// ([`axebot_state_is_valid`]) after at least one respawn, the slot must be
/// its slot, and the uid must be the body the live one replaced: exactly 0
/// after the first respawn; after the second, exactly 1 while creation uids
/// are untracked, and otherwise (a tracked counter, where a combat-start pet
/// sits between the bodies and the first respawn's uid is no longer
/// derivable) any uid from 1 up to but excluding the live body's. A target
/// two bodies back (one play that saw two respawns) is not admitted.
pub(crate) fn axebot_replacement_of(state: &HotState, slot: i32, uid: u32) -> Option<usize> {
    if !axebot_state_is_valid(state) {
        return None;
    }
    let owner = &state.monsters[0];
    let respawn_count = AXEBOT_STOCK - owner.powers.value(PowerId::Stock);
    let replaced_body = match respawn_count {
        1 => uid == 0,
        2 => match tracked_lineage_start(state, 1) {
            None => uid == 1,
            Some(_) => (1..owner.uid).contains(&uid) && !is_osty_creature_uid(state, uid),
        },
        _ => false,
    };
    (slot == owner.slot && replaced_body).then_some(0)
}

/// The exact singleton Stock state after zero, one, or two Axebot creations.
/// UID, HP band, and remaining Stock are one mechanical tuple; accepting each
/// coordinate independently would admit histories native creation cannot
/// produce. Python `_respawn_axebot` installs BOOT_UP (`loop_pos = 2`) only
/// after decrementing Stock (frozen Python, deleted #2827), so the initial Stock-2 body may
/// occupy HAMMER/ONE_TWO but not the respawn opener.
pub(crate) fn axebot_state_is_valid(state: &HotState) -> bool {
    let [owner] = state.monsters.as_slice() else {
        return false;
    };
    let stock = owner.powers.value(PowerId::Stock);
    if !matches!(stock, 0..=AXEBOT_STOCK) {
        return false;
    }
    let respawn_count = AXEBOT_STOCK - stock;
    // `get_Min/MaxInitialHp` at the fight's tier plus the respawn bonus (#2539).
    let ascension = state.fanouts.ascension();
    let bonus = AXEBOT_RESPAWN_HP_BONUS * respawn_count;
    let low = crate::encounters::tier(AXEBOT_MIN_HP, ascension) as i32 + bonus;
    let high = crate::encounters::tier(AXEBOT_MAX_HP, ascension) as i32 + bonus;
    owner.kind == MonsterKind::Axebot
        && owner.slot == 0
        && axebot_expected_uid(state, respawn_count) == Some(owner.uid)
        && (1..=owner.max_hp).contains(&owner.hp)
        && (low..=high).contains(&owner.max_hp)
        && matches!(owner.loop_pos, 0..=2)
        && !(stock == AXEBOT_STOCK && owner.loop_pos == 2)
        && owner.random_ai.is_empty()
        && machine_is_undisturbed_or_generic_stun(owner)
        && !owner.spawn_noop
        && owner.revive_stage == 0
        && !owner.louse_curled()
}

/// Ceremonial Beast's complete singleton loop at serialized player-action
/// boundaries. The PLOW self-loop retains its exact threshold only above the
/// tier's Plow amount ([`ceremonial_beast_plow_threshold`]: 160 at A9+, 150
/// below); a crossing hit atomically replaces it with the paired dynamic
/// STUN/BEAST_CRY state. Ringing is observable only after BEAST_CRY has
/// advanced the ordinary loop to STOMP and before the next owner side end.
pub(crate) fn ceremonial_beast_state_is_valid(state: &HotState) -> bool {
    let [beast] = state.monsters.as_slice() else {
        return false;
    };
    if beast.kind != MonsterKind::CeremonialBeast
        || (beast.slot, beast.uid, Some(beast.max_hp))
            != (0, 0, native_fixed_hp(state, MonsterKind::CeremonialBeast))
        || !(1..=beast.max_hp).contains(&beast.hp)
        || !beast.random_ai.is_empty()
        || beast.spawn_noop
        || beast.revive_stage != 0
    {
        return false;
    }
    let plow = beast.powers.value(PowerId::PlowThreshold);
    let threshold = ceremonial_beast_plow_threshold(state);
    // #3823: a Whistle's stun reads as the row it parks (`StunInternal`
    // `0x11d7cc` writes the move state alone).
    let (override_state, forced_follow_up) = if generic_stun_parks_this_row(beast) {
        (MonsterOverride::None, MonsterFollowUp::None)
    } else {
        (beast.override_state, beast.forced_follow_up)
    };
    match (
        beast.loop_pos,
        plow,
        override_state,
        forced_follow_up,
        state.ringing(),
    ) {
        (0, 0, MonsterOverride::None, MonsterFollowUp::None, false)
        | (2 | 4, 0, MonsterOverride::None, MonsterFollowUp::None, false) => true,
        // Plow lives at or below its threshold when STAMP applied it to a
        // Beast already that low (`PlowPower` breaks only on the next hit,
        // `<AfterDamageReceived>d__8` IL_0051-IL_006a). Reachable unstunned
        // by damage before the first STAMP, and over several turns behind a
        // stunned STAMP (#3823; native traces `bst_h1low`, `bst_s1h1low`,
        // and `bst_h1low_st2`, `bst_s1h1low_st3` for the break after it).
        (1, plow, MonsterOverride::None, MonsterFollowUp::None, false) if plow == threshold => true,
        (1, 0, MonsterOverride::BeastStun, MonsterFollowUp::BeastCry, false) => {
            beast.hp <= threshold
        }
        // #3823: a Plow break under a Whistle's stun drops the break's own
        // STUN_MOVE (`damage::damage_monster`), so PLOW follows itself with
        // Plow gone, for the rest of the fight. Native traces `bst_s2hb`,
        // `bst_w2hb`, `bst_s3hb3`.
        (1, 0, MonsterOverride::None, MonsterFollowUp::None, false) => beast.hp <= threshold,
        (3, 0, MonsterOverride::None, MonsterFollowUp::None, true) => true,
        // #3823: a stunned STOMP's enemy-side end removes the Ringing BEAST_CRY
        // applied, so the parked STOMP stands without it. Native trace
        // `bst_hb2s4`.
        (3, 0, MonsterOverride::None, MonsterFollowUp::None, false) => true,
        _ => false,
    }
}

fn soul_fysh_owner_is_valid(state: &HotState, actor: usize) -> bool {
    let [owner] = state.monsters.as_slice() else {
        return false;
    };
    actor == 0
        && owner.kind == MonsterKind::SoulFysh
        && (owner.slot, owner.uid, Some(owner.max_hp))
            == (0, 0, native_fixed_hp(state, MonsterKind::SoulFysh))
        && (1..=owner.max_hp).contains(&owner.hp)
        && owner.random_ai.is_empty()
        && machine_is_undisturbed_or_generic_stun(owner)
        && !owner.spawn_noop
        && owner.revive_stage == 0
        && !owner.louse_curled()
        && !state.ringing()
}

/// Soul Fysh's exact singleton at serialized player-action boundaries.
///
/// FADE applies Intangible 2 during its action; the same enemy-side end ticks
/// it to one before publication. SCREAM then executes under that one stack
/// and its side end removes it as the loop returns to BECKON.
///
/// #3823: a stun over SCREAM_MOVE spends that enemy turn on STUNNED while the
/// side end still ticks the stack, so the parked SCREAM stands at Intangible
/// 0 and performs without it. Native (harness, `SOUL_FYSH_BOSS` A10,
/// `CreatureCmd.Stun` in player turn 5): Intangible 1 after FADE, none after
/// the stunned turn, SCREAM_MOVE on turn 6. Hence `(4, 0)`.
pub(crate) fn soul_fysh_state_is_valid(state: &HotState) -> bool {
    if !soul_fysh_owner_is_valid(state, 0) {
        return false;
    }
    let owner = &state.monsters[0];
    matches!(
        (owner.loop_pos, owner.powers.value(PowerId::Intangible)),
        (0..=3, 0) | (4, 0 | 1)
    )
}

/// Authenticate one exact move body before it mutates the encounter.
pub(crate) fn soul_fysh_move_state_is_valid(
    state: &HotState,
    actor: usize,
    loop_pos: i32,
    intangible: i32,
) -> bool {
    soul_fysh_owner_is_valid(state, actor)
        && state.monsters[actor].loop_pos == loop_pos
        && state.monsters[actor].powers.value(PowerId::Intangible) == intangible
}

/// Validate the post-move, pre-duration-tick transient at enemy side end.
///
/// After a move: `(1..=3, 0)`, `(4, 2)` after FADE, `(0, 1)` after a SCREAM
/// under its stack, `(0, 0)` after a SCREAM a stun delayed. After a stunned
/// turn (#3823) the row has not moved, so the transient is the entry state
/// itself: `(0..=3, 0)` or `(4, 0 | 1)`.
pub(crate) fn soul_fysh_side_end_state_is_valid(state: &HotState) -> bool {
    if !soul_fysh_owner_is_valid(state, 0) {
        return false;
    }
    let owner = &state.monsters[0];
    matches!(
        (owner.loop_pos, owner.powers.value(PowerId::Intangible)),
        (0..=3, 0) | (4, 0..=2) | (0, 1)
    )
}

/// Entomancer's complete fixed singleton at serialized action boundaries.
///
/// Personal Hive is intrinsic at amount one, grows only through the fixed
/// PHEROMONE_SPIT loop position, and caps at three. Strength may be any signed
/// live amount because ordinary buffs/debuffs share that power; Hive is the
/// owner-local coordinate that closes the encounter and its damage reader.
pub(crate) fn entomancer_state_is_valid(state: &HotState) -> bool {
    let [owner] = state.monsters.as_slice() else {
        return false;
    };
    owner.kind == MonsterKind::Entomancer
        && (owner.slot, owner.uid, Some(owner.max_hp))
            == (0, 0, native_fixed_hp(state, MonsterKind::Entomancer))
        && (1..=owner.max_hp).contains(&owner.hp)
        && matches!(owner.loop_pos, 0..=2)
        && owner.random_ai.is_empty()
        && machine_is_undisturbed_or_generic_stun(owner)
        && !owner.spawn_noop
        && owner.revive_stage == 0
        && !owner.louse_curled()
        && !state.ringing()
        && matches!(owner.powers.value(PowerId::Hive), 1..=3)
}

#[cfg(test)]
pub(crate) const THE_INSATIABLE_HP: i32 = 341;

/// The Insatiable's exact singleton and Sandpit lifecycle at serialized
/// command boundaries (`_validated_insatiable_roster`, frozen Python, deleted #2827).
pub(crate) fn insatiable_state_is_valid(state: &HotState) -> bool {
    let [owner] = state.monsters.as_slice() else {
        return false;
    };
    owner.kind == MonsterKind::TheInsatiable
        && (owner.slot, owner.uid, Some(owner.max_hp))
            == (0, 0, native_fixed_hp(state, MonsterKind::TheInsatiable))
        && (1..=owner.max_hp).contains(&owner.hp)
        && owner.random_ai.is_empty()
        && machine_is_undisturbed_or_generic_stun(owner)
        && !owner.spawn_noop
        && owner.revive_stage == 0
        && !owner.louse_curled()
        && !state.ringing()
        && matches!(
            (owner.loop_pos, owner.powers.value(PowerId::Sandpit)),
            (0, 0) | (1..=4, 1..=i32::MAX)
        )
}

/// Authenticate LIQUIFY before Sandpit, uid, RNG, or pile state mutates.
pub(crate) fn insatiable_liquify_state_is_valid(state: &HotState, actor: usize) -> bool {
    insatiable_state_is_valid(state)
        && actor == 0
        && state.monsters[0].loop_pos == 0
        && state.monsters[0].powers.value(PowerId::Sandpit) == 0
}

/// Frantic Escape is a valid standalone generated Status when no Insatiable
/// exists; any live Sandpit slot still requires the exact singleton owner.
pub(crate) fn optional_insatiable_state_is_valid(state: &HotState) -> bool {
    if state
        .monsters
        .iter()
        .any(|monster| monster.kind == MonsterKind::TheInsatiable)
    {
        insatiable_state_is_valid(state)
    } else {
        state
            .monsters
            .iter()
            .all(|monster| monster.powers.value(PowerId::Sandpit) == 0)
    }
}

/// Test Subject's exact singleton three-form state at a serialized command
/// boundary (`_validated_test_subject_roster`, frozen Python, deleted #2827).
///
/// Adaptable retains the first two corpses with loop position zero. Its next
/// enemy action performs the form transition in place; the third death is
/// ordinary terminal state and therefore never enters this live admission
/// predicate. Owner-disjoint packed accessors carry the three private fields
/// without exposing Waterfall/illusion aliases at the canonical boundary.
/// Python puts its solo-party clause at the front of this shared validator;
/// mirror it here so admission and every later reader retain the same domain.
pub(crate) fn test_subject_state_is_valid(state: &HotState) -> bool {
    let [owner] = state.monsters.as_slice() else {
        return false;
    };
    if state.multiplayer_ally_key != 0
        || state.history.over
        || owner.kind != MonsterKind::TestSubject
        || (owner.slot, owner.uid) != (0, 0)
        || owner.hp > owner.max_hp
        || owner.block < 0
        || !owner.random_ai.is_empty()
        // #3823: or a Whistle's stun parked over a form's row; the death
        // replaces it with the retained corpse (`TestSubject/
        // <TriggerDeadState>d__67::MoveNext` `0x36e658` IL_00c4-IL_00c5,
        // `SetMoveImmediate(state, force: true)`).
        || !machine_is_undisturbed_or_generic_stun(owner)
        || owner.spawn_noop
        || owner.last_spawned.is_some()
        || owner.curl_up_card_uid != -1
        || owner.louse_curled()
        || owner.pressure_gun_damage != 0
        || owner.waterfall_steam_payload_is_set()
        || owner.is_about_to_blow()
        || owner.possess_strength_debit != 0
        || owner.possess_speed_debit != 0
        || owner.ritual_fresh
        || owner.revive_stage & !0b0000_1111 != 0
        || state.ringing()
    {
        return false;
    }

    let form = owner.test_subject_respawns();
    let reviving = owner.test_subject_adaptable_reviving();
    let extra = owner.test_subject_extra_multi_claw_count();
    let adaptable = owner.powers.value(PowerId::Adaptable);
    let painful_stabs = owner.powers.value(PowerId::PainfulStabs);
    let nemesis = owner.powers.value(PowerId::Nemesis);
    let enrage = owner.powers.value(PowerId::Enrage);
    let intangible = owner.powers.value(PowerId::Intangible);
    let nemesis_applies = owner.test_subject_nemesis_apply_intangible();

    match form {
        0 => {
            Some(owner.max_hp) == test_subject_form_hp(state, 0)
                && adaptable == 1
                && painful_stabs == 0
                && nemesis == 0
                && extra == 0
                && !nemesis_applies
                && intangible == 0
                && ((owner.hp > 0
                    && matches!(owner.loop_pos, 1 | 2)
                    && !reviving
                    && enrage == test_subject_enrage(state))
                    || (owner.hp <= 0 && owner.loop_pos == 0 && reviving && enrage == 0))
        }
        1 => {
            Some(owner.max_hp) == test_subject_form_hp(state, 1)
                && adaptable == 1
                && painful_stabs == 1
                && nemesis == 0
                && enrage == 0
                && !nemesis_applies
                && intangible == 0
                && ((owner.hp > 0 && owner.loop_pos == 3 && !reviving)
                    || (owner.hp <= 0 && owner.loop_pos == 0 && reviving))
        }
        2 => {
            Some(owner.max_hp) == test_subject_form_hp(state, 2)
                && owner.hp > 0
                && matches!(owner.loop_pos, 4..=6)
                && !reviving
                && adaptable == 0
                && painful_stabs == 0
                && nemesis == 1
                && enrage == 0
                && ((nemesis_applies && intangible == 1) || (!nemesis_applies && intangible == 0))
        }
        _ => false,
    }
}

/// Authenticate one direct Test Subject move body against its exact live form
/// and loop position before any damage, pile, uid, or power mutation.
pub(crate) fn test_subject_live_move_state_is_valid(
    state: &HotState,
    actor: usize,
    form: u8,
    loop_pos: i32,
) -> bool {
    test_subject_state_is_valid(state)
        && actor == 0
        && state.monsters[0].hp > 0
        && state.monsters[0].test_subject_respawns() == form
        && state.monsters[0].loop_pos == loop_pos
}

/// Adaptable's retained-corpse transition after ordinary Test Subject death
/// cleanup. The caller has already committed the lethal HP result and removed
/// current-form ordinary powers; this function proves the exact prospective
/// corpse before publishing either the latch or the RESPawn loop position.
pub(crate) fn latch_test_subject_after_death(
    state: &mut HotState,
    target: usize,
) -> Result<(), EngineRefusal> {
    let Some(owner) = state.monsters.get(target) else {
        return Err(EngineRefusal::MalformedArgs("Test Subject Adaptable owner"));
    };
    if target != 0
        || owner.kind != MonsterKind::TestSubject
        || owner.hp > 0
        || owner.test_subject_respawns() >= 2
        || owner.test_subject_adaptable_reviving()
        || owner.powers.value(PowerId::Adaptable) != 1
    {
        return Err(EngineRefusal::MalformedArgs(
            "Test Subject Adaptable death state",
        ));
    }
    let mut probe = state.clone();
    probe.monsters_mut()[target].loop_pos = 0;
    probe.monsters_mut()[target].set_test_subject_adaptable_reviving(true);
    if !test_subject_state_is_valid(&probe) {
        return Err(EngineRefusal::MalformedArgs(
            "Test Subject Adaptable retained corpse",
        ));
    }
    state.monsters_mut()[target].loop_pos = 0;
    state.monsters_mut()[target].set_test_subject_adaptable_reviving(true);
    Ok(())
}

/// Nemesis's alternating Intangible state at the late enemy-side-end owner
/// position. The newly applied stack survives the generic duration tick in
/// the same side end because native freezes that listener snapshot first.
pub(crate) fn test_subject_nemesis_side_end(
    state: &mut HotState,
    target: usize,
) -> Result<i32, EngineRefusal> {
    if target != 0 || state.monsters.get(target).is_none() {
        return Err(EngineRefusal::MalformedArgs(
            "Test Subject Nemesis side-end state",
        ));
    }
    // The duration walk precedes Nemesis. Its one old Intangible stack has
    // therefore become zero while the private application latch still records
    // the old side. Reconstitute that one canonical pre-tick pair solely for
    // validation; the live transition below consumes the latch and leaves 0.
    let expired_old_stack = state.monsters[0].test_subject_nemesis_apply_intangible()
        && state.monsters[0].powers.value(PowerId::Intangible) == 0;
    let valid = if expired_old_stack {
        let mut probe = state.clone();
        probe.monsters_mut()[0]
            .powers
            .set(PowerId::Intangible, SlotWire::Int, 1);
        test_subject_state_is_valid(&probe)
    } else {
        test_subject_state_is_valid(state)
    };
    if !valid
        || state.monsters[0].test_subject_respawns() != 2
        || state.monsters[0].powers.value(PowerId::Nemesis) != 1
    {
        return Err(EngineRefusal::MalformedArgs(
            "Test Subject Nemesis side-end state",
        ));
    }
    let apply = !state.monsters[0].test_subject_nemesis_apply_intangible();
    let amount = i32::from(apply);
    let owner = &mut state.monsters_mut()[0];
    owner.set_test_subject_nemesis_apply_intangible(apply);
    owner.powers.set(PowerId::Intangible, SlotWire::Int, amount);
    debug_assert!(test_subject_state_is_valid(state));
    Ok(amount)
}

pub(crate) fn require_insatiable_state(state: &HotState) -> Result<(), EngineRefusal> {
    if insatiable_state_is_valid(state) {
        Ok(())
    } else {
        Err(EngineRefusal::MalformedArgs(
            "The Insatiable/Sandpit lifecycle",
        ))
    }
}

/// Pure preflight for Plow's post-damage mutation. This is intentionally
/// stricter than checking the power slot: a duplicated/missing owner or a
/// forged continuation must refuse before Block or HP changes.
pub(crate) fn beast_plow_damage_owner_is_valid(state: &HotState, target: usize) -> bool {
    let [beast] = state.monsters.as_slice() else {
        return false;
    };
    target == 0
        && beast.kind == MonsterKind::CeremonialBeast
        && beast.loop_pos == 1
        // A player AfterDamageGiven listener can synchronously deal nested
        // damage after the outer hit crossed 160 but before Plow's own
        // AfterDamageReceived slot runs. Admission rejects that shape at a
        // command boundary; the runtime preflight must accept the live
        // listener transient so the nested result remains exact.
        && beast.hp > 0
        && beast.powers.value(PowerId::PlowThreshold) == ceremonial_beast_plow_threshold(state)
        // #3823: or a Whistle's stun parked over PLOW (the break under it is
        // `damage::damage_monster`'s; native trace `bst_s2hb`).
        && machine_is_undisturbed_or_generic_stun(beast)
        && !state.ringing()
}

pub(crate) const FABRICATOR_AGGRO_BOTS: [MonsterKind; 2] =
    [MonsterKind::Zapbot, MonsterKind::Stabbot];
pub(crate) const FABRICATOR_DEFENSE_BOTS: [MonsterKind; 2] =
    [MonsterKind::Guardbot, MonsterKind::Noisebot];

fn is_fabricator_bot(kind: MonsterKind) -> bool {
    FABRICATOR_AGGRO_BOTS.contains(&kind) || FABRICATOR_DEFENSE_BOTS.contains(&kind)
}

pub(crate) fn fabricator_ai_state_is_valid(
    monster: &HotMonster,
    allow_post_move_transient: bool,
) -> bool {
    if monster.kind != MonsterKind::Fabricator
        || monster.powers.value(PowerId::Secondary) != 0
        || monster.powers.value(PowerId::HighVoltage) != 0
        || monster.loop_pos != 0
        || monster.random_ai.once_len() != 0
    {
        return false;
    }
    let Some(next) = monster.random_ai.next() else {
        return false;
    };
    let log: Vec<u8> = (0..monster.random_ai.log_len())
        .filter_map(|position| monster.random_ai.log_at(position))
        .collect();
    (1..=3).contains(&log.len())
        && log.len() == monster.random_ai.log_len()
        && log.last() == Some(&next)
        && !log
            .iter()
            .any(|index| *index > FABRICATOR_DISINTEGRATE_INDEX)
        && !(log.len() < 3 && log[0] > FABRICATOR_STRIKE_INDEX)
        // No adjacent-pair constraint: `RAND`'s FABRICATE and
        // FABRICATING_STRIKE are `CanRepeatForever` at v0.111.0, so either
        // may follow itself (#2945; IL on `engine::turn::roll_fabricator_intent`:
        // `Fabricator::GenerateMoveStateMachine` `0xb3bc0` IL_00a4/IL_00cc pass
        // repeat type 0 to `AddBranch` `0x7925a`).
        // The log against the bots made. FABRICATE and FABRICATING_STRIKE
        // both end on an aggro bot (`<FabricateMove>d__20` `0x35a434`
        // IL_00f2, `<FabricatingStrikeMove>d__21` `0x35a5dc` IL_00c0), and
        // the roll after a performed move logs a second entry. #3823: a
        // stun logs the parked move again without performing it
        // (`turn::resume_stunned_random_ai`), so a Fabricator that has made
        // nothing yet holds its opener once, twice or three times.
        && (allow_post_move_transient
            || (monster.last_spawned.is_none() && log.iter().all(|index| *index == log[0]))
            || (log.len() >= 2
                && monster
                    .last_spawned
                    .is_some_and(|kind| FABRICATOR_AGGRO_BOTS.contains(&kind))))
}

/// Python `_validate_fabricator_ai` (frozen, deleted #2827) and
/// `_validate_fabricator_bot`, at a serialized command
/// boundary.
fn fabricator_state_is_valid_inner(state: &HotState, allow_post_move_transient: bool) -> bool {
    for monster in state.monsters.iter() {
        if monster.kind == MonsterKind::Fabricator {
            if !fabricator_ai_state_is_valid(monster, allow_post_move_transient) {
                return false;
            }
        } else if is_fabricator_bot(monster.kind) {
            let high_voltage = monster.powers.value(PowerId::HighVoltage);
            let expected_high_voltage = if monster.kind == MonsterKind::Zapbot && monster.hp > 0 {
                2
            } else {
                0
            };
            if monster.powers.value(PowerId::Secondary) != 1
                || high_voltage != expected_high_voltage
                || monster.last_spawned.is_some()
                || !monster.random_ai.is_empty()
                || monster.loop_pos != 0
            {
                return false;
            }
        } else if monster.last_spawned.is_some() {
            return false;
        }
    }
    // Only a live Fabricator can allocate another monster identity. Standalone
    // bot documents execute fixed move bodies and therefore retain the full
    // canonical u32 uid range instead of inheriting an unrelated spawn limit.
    !state
        .monsters
        .iter()
        .any(|monster| monster.kind == MonsterKind::Fabricator)
        || state
            .monsters
            .iter()
            .map(|monster| monster.uid)
            .max()
            .is_none_or(|uid| uid < u32::MAX)
}

pub(crate) fn fabricator_state_is_valid(state: &HotState) -> bool {
    fabricator_state_is_valid_inner(state, false)
}

pub(crate) fn fabricator_prepare_state_is_valid(state: &HotState) -> bool {
    fabricator_state_is_valid_inner(state, true)
}

pub(crate) fn require_fabricator_state(
    state: &HotState,
    owner_uid: u32,
) -> Result<(), EngineRefusal> {
    if !fabricator_state_is_valid(state)
        || state
            .monsters
            .iter()
            .find(|monster| monster.uid == owner_uid)
            .is_none_or(|monster| monster.kind != MonsterKind::Fabricator)
    {
        return Err(EngineRefusal::MalformedArgs("Fabricator bot/AI lifecycle"));
    }
    Ok(())
}

/// Python `_is_secondary_enemy` (frozen, deleted #2827).
pub(crate) fn is_secondary_enemy(monster: &HotMonster) -> bool {
    monster.powers.value(PowerId::Secondary) > 0
        || matches!(
            monster.kind,
            MonsterKind::GasBomb | MonsterKind::Parafright | MonsterKind::EyeWithTeeth
        )
}

fn overflow(site: &'static str) -> EngineRefusal {
    EngineRefusal::CounterOverflow(site)
}

/// Whether the complete hot roster matches `_validated_possess_roster`
/// (frozen Python, deleted #2827). The Python validator is explicitly solo: with a represented
/// teammate, its player-keyed debit dictionaries need a second owner row that
/// `HotState` does not carry. The two signed scalars therefore carry the
/// complete reachable one-row dictionaries only while the party is absent.
pub(crate) fn possess_roster_is_valid(state: &HotState) -> bool {
    if state.multiplayer_ally_key != 0 || state.monsters.len() != 2 {
        return false;
    }
    let lost = &state.monsters[0];
    let forgotten = &state.monsters[1];
    if lost.kind != MonsterKind::TheLost
        || forgotten.kind != MonsterKind::TheForgotten
        || (lost.slot, lost.uid, Some(lost.max_hp))
            != (0, 0, native_fixed_hp(state, MonsterKind::TheLost))
        || (forgotten.slot, forgotten.uid, Some(forgotten.max_hp))
            != (1, 1, native_fixed_hp(state, MonsterKind::TheForgotten))
        || lost.hp > lost.max_hp
        || forgotten.hp > forgotten.max_hp
        || !(0..2).contains(&lost.loop_pos)
        || !(0..2).contains(&forgotten.loop_pos)
        || !lost.random_ai.is_empty()
        || !forgotten.random_ai.is_empty()
        || !machine_is_undisturbed_or_generic_stun(lost)
        || !machine_is_undisturbed_or_generic_stun(forgotten)
        || lost.spawn_noop
        || forgotten.spawn_noop
        || lost.revive_stage != 0
        || forgotten.revive_stage != 0
        || lost.block < 0
        || forgotten.block < 0
        || lost.powers.value(PowerId::Dexterity) != 0
        || forgotten.powers.value(PowerId::Dexterity) < 0
        || lost.possess_speed_debit != 0
        || forgotten.possess_strength_debit != 0
        || lost.possess_strength_debit > 0
        || forgotten.possess_speed_debit > 0
    {
        return false;
    }
    true
}

pub(crate) fn require_possess_roster(
    state: &HotState,
    owner: usize,
    kind: MonsterKind,
) -> Result<(), EngineRefusal> {
    if !possess_roster_is_valid(state)
        || state
            .monsters
            .get(owner)
            .is_none_or(|monster| monster.kind != kind)
    {
        return Err(EngineRefusal::MalformedArgs(
            "TheLostAndForgottenNormal roster lifecycle",
        ));
    }
    Ok(())
}

fn gremlin_member_shape_is_exact(
    state: &HotState,
    monster: &HotMonster,
    kind: MonsterKind,
    slot: i32,
    uid: u32,
) -> bool {
    monster.kind == kind
        && (monster.slot, monster.uid) == (slot, uid)
        && native_hp_in_band(state, kind, monster.max_hp)
        && monster.hp <= monster.max_hp
        && monster.block >= 0
        && monster.loop_pos == 0
        && monster.random_ai.is_empty()
        && machine_is_undisturbed_or_generic_stun(monster)
        && monster.revive_stage == 0
        && monster.last_spawned.is_none()
        && monster.curl_up_card_uid == -1
        && !monster.louse_curled()
        && monster.pressure_gun_damage == 0
        && !monster.waterfall_steam_payload_is_set()
        && monster.pressure_buildup_idx() == 0
        && monster.possess_strength_debit == 0
        && monster.possess_speed_debit == 0
}

fn gremlin_merc_owner_shape_is_exact(state: &HotState, merc: &HotMonster) -> bool {
    merc.kind == MonsterKind::GremlinMerc
        && (merc.slot, merc.uid) == (0, 0)
        && native_hp_in_band(state, MonsterKind::GremlinMerc, merc.max_hp)
        && merc.hp <= merc.max_hp
        && merc.block >= 0
        && matches!(merc.loop_pos, 0..=2)
        && merc.random_ai.is_empty()
        && machine_is_undisturbed_or_generic_stun(merc)
        && !merc.spawn_noop
        && merc.revive_stage == 0
        && merc.last_spawned.is_none()
        && merc.curl_up_card_uid == -1
        && !merc.louse_curled()
        && merc.pressure_gun_damage == 0
        && merc.gremlin_merc_private_state_is_exact()
        && merc.possess_strength_debit == 0
        && merc.possess_speed_debit == 0
}

fn gremlin_power_owners_are_exact(state: &HotState) -> bool {
    state.monsters.iter().all(|monster| {
        let stolen = monster.powers.get(PowerId::StolenGold);
        let heist = monster.powers.get(PowerId::HeistGold);
        stolen.is_none_or(|slot| {
            monster.kind == MonsterKind::GremlinMerc
                && slot.wire == SlotWire::Int
                && slot.value >= 0
        }) && heist.is_none_or(|slot| {
            monster.kind == MonsterKind::FatGremlin && slot.wire == SlotWire::Int && slot.value >= 0
        })
    })
}

/// Fat Gremlin's creation uid in a spawned Gremlin Merc lineage (#3039).
///
/// Surprise creates Fat first and Sneaky second, each taking the next native
/// creature id ([`allocate_creature_uid`]). Untracked, that is uids 1 and 2
/// after the Merc's 0. Tracked, a combat-start pet holds ids between the Merc
/// and Fat, and nothing in this encounter creates a creature after the pair,
/// so Fat is the counter minus two. `None` is no representable lineage.
pub(crate) fn gremlin_merc_fat_uid(state: &HotState) -> Option<u32> {
    match tracked_lineage_start(state, 2) {
        None => Some(1),
        Some(start) => start.filter(|uid| *uid >= 1),
    }
}

/// Exact stable Gremlin Merc lineage at a public command boundary.
///
/// Native retains the dead Merc after Surprise. Fat is created first
/// ([`gremlin_merc_fat_uid`], uid 1 without a pet) but published after Sneaky
/// (slot 1, the next uid). The absent proportion field is
/// derived by [`gremlin_merc_gold_proportion_halves`], while the two private
/// owner-disjoint hot lanes retain the stolen latch and returned Heist gold.
///
/// Both children's HP is unique against the Merc only: each is rolled while
/// `_enemies` holds just the Merc (#3040, see [`spawn_gremlin_merc_pair`]),
/// so Sneaky may share Fat's max HP.
pub(crate) fn gremlin_merc_state_is_valid(state: &HotState) -> bool {
    if state.multiplayer_ally_key != 0 || !gremlin_power_owners_are_exact(state) {
        return false;
    }
    let Some(merc) = state.monsters.first() else {
        return false;
    };
    if !gremlin_merc_owner_shape_is_exact(state, merc) {
        return false;
    }
    let stolen = merc.gremlin_merc_gold_was_stolen();
    let returned = merc.gremlin_merc_returned_gold();
    let merc_stolen = merc.powers.value(PowerId::StolenGold);
    let lineage =
        gremlin_merc_fat_uid(state).and_then(|fat_uid| Some((fat_uid, fat_uid.checked_add(1)?)));
    match state.monsters.as_slice() {
        [merc] => merc.hp > 0 && !stolen && returned == 0 && merc_stolen >= 0,
        [merc, sneaky] => {
            let Some((_, sneaky_uid)) = lineage else {
                return false;
            };
            merc.hp <= 0
                && merc_stolen == 0
                && returned == 0
                && gremlin_member_shape_is_exact(
                    state,
                    sneaky,
                    MonsterKind::SneakyGremlin,
                    1,
                    sneaky_uid,
                )
                && sneaky.max_hp != merc.max_hp
                && (sneaky.hp <= 0 || !sneaky.spawn_noop)
        }
        [merc, sneaky, fat] => {
            let Some((fat_uid, sneaky_uid)) = lineage else {
                return false;
            };
            if merc.hp > 0
                || merc_stolen != 0
                || !gremlin_member_shape_is_exact(
                    state,
                    sneaky,
                    MonsterKind::SneakyGremlin,
                    1,
                    sneaky_uid,
                )
                || !gremlin_member_shape_is_exact(state, fat, MonsterKind::FatGremlin, 2, fat_uid)
                || sneaky.max_hp == merc.max_hp
                || fat.max_hp == merc.max_hp
            {
                return false;
            }
            let heist = fat.powers.value(PowerId::HeistGold);
            if fat.hp > 0 || state.fanouts.monster_death_is_pending(fat.uid) {
                returned == 0 && stolen == (heist > 0)
            } else {
                heist == 0 && stolen == (returned > 0)
            }
        }
        _ => false,
    }
}

/// Stable external Gremlin Merc boundary. Internal card execution may carry
/// its authenticated CardPlay stack while damage traverses this encounter;
/// imported/publicly resumed Merc states may not begin with any continuation.
pub(crate) fn gremlin_merc_public_state_is_valid(state: &HotState) -> bool {
    state.pending.is_none()
        && state.frames.is_empty()
        && (state.history.over || state.monsters.iter().any(|monster| monster.hp > 0))
        && gremlin_merc_state_is_valid(state)
    // #3823: the two gremlins' SPAWNED_MOVE flags are independent. Until a
    // gremlin could be stunned, two live siblings always spent the state on
    // the same enemy turn, and this boundary asked for equal flags. A stun
    // over SPAWNED_MOVE spends that turn on STUNNED and leaves the spawn for
    // the next one (native: STUNNED, SPAWNED_MOVE, FLEE_MOVE against the
    // sibling's SPAWNED_MOVE first), so every pair of flags is reachable.
}

/// Projectable Gremlin Merc boundary: the stable public boundary above, or a
/// player card's own parked selection (#3359, e.g. Armaments' upgrade choice
/// on the turn after a Gimme steal).
///
/// Native keeps every piece of the stolen-gold state outside the action
/// queue, so parking a card's selection cannot hold any of it mid-flight:
///
/// - the steal latch is the encounter field `_goldWasStolen`
///   (`GremlinMercNormal::get_GoldWasStolen` v0.111.0 RVA `0xd3eaf`
///   `IL_0002` `ldfld`; `set_GoldWasStolen` RVA `0xd3eb7` `IL_0009`
///   `stfld`), held by the encounter model rather than by any creature or
///   pending command;
/// - the proportion is a pure read of that latch and the escaped roster
///   (`GremlinMercNormal::CalculateGoldProportion` RVA `0xd3ed0`: no
///   qualifying escaped creature returns `1.0` at `IL_0038`, otherwise
///   `get_GoldWasStolen` at `IL_003f` picks `0.5` at `IL_0046` or `0.0` at
///   `IL_004c`);
/// - the stolen amount itself is the StolenGold / HeistGold power amount on
///   the Merc / Fat, which the roster already carries.
///
/// So the parked state's proportion is exactly the one
/// [`gremlin_merc_gold_proportion_halves`] derives from the retained lineage.
/// The continuation must be a card-play selection (`pending.record` resolves
/// to a CardPlay record) over a stack of `CardPlay` frames only, rooted on
/// the bottom `ActionReplay` witness that every public park installs (which
/// the boundary authenticates by re-running the recorded action from its
/// admitted, stable-public predecessor): no damage, death or listener walk
/// is suspended, so no transient Surprise or Escape state can sit under the
/// park, and a rootless park keeps the public rule. A transient lineage (a dead lone Merc whose
/// Surprise has not run) still fails [`gremlin_merc_state_is_valid`] and
/// refuses by name. Every other continuation keeps the public rule.
///
/// #3756: a generation potion's own choose-a-card screen is the same kind of
/// park. `AttackPotion/<OnUse>d__6::MoveNext` (RVA `0x34bdb4`) draws its
/// offers (`CardFactory::GetDistinctForCombat` IL_0095), awaits
/// `CardSelectCmd::FromChooseACardScreen` (IL_00a9) and adds the pick to
/// Hand (`CardPileCmd::AddGeneratedCardToCombat` IL_0121);
/// `ColorlessPotion/<OnUse>d__6::MoveNext` (RVA `0x34c784`) is the same body
/// at IL_006b / IL_007f / IL_00f7. `PotionModel/<OnUseWrapper>d__75::MoveNext`
/// (RVA `0x31dd20`) has finished `Hook::BeforePotionUsed` (IL_00db) before it
/// calls `OnUse` (IL_0397), and `Hook::AfterPotionUsed` (IL_0515) runs only
/// after the body returns. So while the screen is open no damage, death or
/// listener walk is suspended under it, and the body itself touches no
/// creature. The accepted stack is exactly the root `ActionReplay` witness
/// and the one `PotionFinish` frame whose record is the open
/// `GenerationSelecting` choice; a potion parked on any other selection keeps
/// the public rule.
pub(crate) fn gremlin_merc_projection_state_is_valid(state: &HotState) -> bool {
    if gremlin_merc_public_state_is_valid(state) {
        return true;
    }
    let Some(pending) = state.pending.as_deref() else {
        return false;
    };
    use crate::frame::Frame;
    let [Frame::ActionReplay { .. }, frames @ ..] = state.frames.as_slice() else {
        return false;
    };
    let card_selection = pending.record(&state.frames).is_some()
        && !frames.is_empty()
        && frames
            .iter()
            .all(|frame| matches!(frame, Frame::CardPlay { .. }));
    let generation_potion_choice = matches!(frames, [Frame::PotionFinish { .. }])
        && pending.generation_potion_record(&state.frames).is_some();
    (card_selection || generation_potion_choice)
        && !state.history.over
        && state.monsters.iter().any(|monster| monster.hp > 0)
        && gremlin_merc_state_is_valid(state)
    // #3823: the two gremlins' SPAWNED_MOVE flags are independent. Until a
    // gremlin could be stunned, two live siblings always spent the state on
    // the same enemy turn, and this boundary asked for equal flags. A stun
    // over SPAWNED_MOVE spends that turn on STUNNED and leaves the spawn for
    // the next one (native: STUNNED, SPAWNED_MOVE, FLEE_MOVE against the
    // sibling's SPAWNED_MOVE first), so every pair of flags is reachable.
}

/// Stable canonical projection of `CalculateGoldProportion`, in half-units.
pub(crate) fn gremlin_merc_gold_proportion_halves(state: &HotState) -> Option<i32> {
    if !gremlin_merc_state_is_valid(state) {
        return None;
    }
    match state.monsters.as_slice() {
        [merc] if merc.hp > 0 => Some(-1),
        [merc, _sneaky] if merc.hp <= 0 => Some(if merc.gremlin_merc_gold_was_stolen() {
            0
        } else {
            1
        }),
        [merc, _sneaky, _fat] if merc.hp <= 0 => Some(2),
        _ => None,
    }
}

/// Exact transient immediately after lethal Merc HP has committed and before
/// Surprise's enemy-power listener runs.
fn gremlin_merc_surprise_entry_is_exact(state: &HotState, owner: usize) -> bool {
    let [merc] = state.monsters.as_slice() else {
        return false;
    };
    owner == 0
        && merc.hp <= 0
        && gremlin_merc_owner_shape_is_exact(state, merc)
        && !merc.gremlin_merc_gold_was_stolen()
        && merc.gremlin_merc_returned_gold() == 0
        && merc.powers.value(PowerId::StolenGold) >= 0
        && merc.powers.value(PowerId::HeistGold) == 0
        && state.multiplayer_ally_key == 0
        && gremlin_power_owners_are_exact(state)
}

/// SurprisePower.AfterDeath, excluding ordinary death cleanup of StolenGold.
///
/// The function clone-rehearses the complete two-draw/two-publication body so
/// malformed Niche state, UID overflow, or a later insertion error cannot
/// publish a partial child roster.
pub(crate) fn spawn_gremlin_merc_pair(
    state: &mut HotState,
    owner: usize,
) -> Result<(), EngineRefusal> {
    if !gremlin_merc_surprise_entry_is_exact(state, owner) {
        return Err(EngineRefusal::MalformedArgs("Gremlin Merc Surprise entry"));
    }
    let niche = state.rng.get(RngStream::Niche);
    if niche.words == [0; 4] {
        return Err(EngineRefusal::MalformedArgs("Gremlin Merc Niche stream"));
    }
    let mut next = state.clone();
    let heist = next.monsters[owner].powers.value(PowerId::StolenGold);
    // `SurprisePower/<AfterDeath>d__4::MoveNext` (v0.111.0 RVA `0x347044`)
    // creates Fat at `IL_0063` (`ICombatState::CreateCreature`: its unique
    // HP roll and its creature id) but only publishes it at `IL_01fc`
    // (`CreatureCmd::Add(fat)`), after `CreatureCmd.Add<SneakyGremlin>` at
    // `IL_0198` has created and published Sneaky. `CreateCreature`
    // (`0x137074` `IL_003f`-`IL_0055`) makes HP unique against `_enemies`
    // only (`Creature::SetUniqueMonsterHpValue` `0x11d3ec`
    // `IL_0048`-`IL_0079` `ExceptWith`), and `CreateCreature` itself never
    // adds to `_enemies`, so Sneaky's roll does not see Fat (#3040).
    //
    // The dying Merc is still in `_enemies` for both rolls (#3146): Surprise
    // runs from `Hook.AfterDeath`, which
    // `<KillWithoutCheckingWinCondition>d__15::MoveNext` (`0x3ebe90`) awaits
    // at `IL_03bc`, before its `CombatState::RemoveCreature` at `IL_04d5`.
    // The entry check above pins the roster to that one row, so the
    // all-rows roll below reserves exactly the native set: the Merc's max HP.
    let fat_uid = allocate_creature_uids(&mut next, 2)?;
    let (fat_low, fat_high) = native_hp_band(&next, MonsterKind::FatGremlin)?;
    let fat_hp = roll_unique_hp(&mut next, fat_low, fat_high)?;
    let mut fat = HotMonster::new(MonsterKind::FatGremlin, fat_hp);
    fat.max_hp = fat_hp;
    fat.slot = 2;
    fat.uid = fat_uid;
    fat.spawn_noop = true;
    fat.powers.set(PowerId::HeistGold, SlotWire::Int, heist);
    let sneaky_uid = fat_uid
        .checked_add(1)
        .ok_or(EngineRefusal::CounterOverflow("Gremlin Merc child uid"))?;
    let (sneaky_low, sneaky_high) = native_hp_band(&next, MonsterKind::SneakyGremlin)?;
    let sneaky_hp = roll_unique_hp(&mut next, sneaky_low, sneaky_high)?;

    let mut sneaky = HotMonster::new(MonsterKind::SneakyGremlin, sneaky_hp);
    sneaky.max_hp = sneaky_hp;
    sneaky.slot = 1;
    sneaky.uid = sneaky_uid;
    sneaky.spawn_noop = true;

    // Native Add order is Sneaky then the already-created Fat.
    insert_slot_ordered(&mut next, sneaky);
    fur_coat_after_opponent_added(&mut next, sneaky_uid)?;
    insert_slot_ordered(&mut next, fat);
    fur_coat_after_opponent_added(&mut next, fat_uid)?;
    if !next.monsters_mut()[owner].set_gremlin_merc_gold_was_stolen(heist > 0) {
        return Err(EngineRefusal::MalformedArgs("Gremlin Merc private owner"));
    }
    *state = next;
    Ok(())
}

/// Python `_roll_unique_hp` (frozen, deleted #2827): ascending candidates after
/// excluding every existing max HP, falling back to the full range when all
/// candidates collide, and exactly one Niche draw in either case.
///
/// Every roster row reserves its max HP, a corpse included. The two callers
/// (Gremlin Merc's Surprise and Axebot's Stock) both run inside the dying
/// owner's `Hook.AfterDeath`, where that owner is still in `_enemies`; see
/// [`respawn_axebot`] for the IL.
fn roll_unique_hp(state: &mut HotState, low: i32, high: i32) -> Result<i32, EngineRefusal> {
    let taken: Vec<i32> = state
        .monsters
        .iter()
        .map(|monster| monster.max_hp)
        .collect();
    roll_unique_hp_over(state, low, high, &taken)
}

/// Whether a retained hot-roster row is still in native
/// `CombatState.Enemies` at a mid-combat creation seam.
///
/// A dying enemy leaves `Enemies` unless a keep-on-death listener vetoes it:
/// `<KillWithoutCheckingWinCondition>d__15::MoveNext` (v0.111.0 RVA
/// `0x3ebe90`) reads `Hook::ShouldCreatureBeRemovedFromCombatAfterDeath`
/// (`0x1063a0`) at `IL_0330`, then calls `CombatState::RemoveCreature`
/// (`0x1371d0`, `_enemies.Remove` at `IL_003e`) at `IL_04d5`; when the
/// creature died during its own move the same removal is deferred to
/// `<PerformMove>d__105::MoveNext` `IL_01df`. The hook's default
/// (`AbstractModel` `0x7a34a`) is `true`; only six powers veto it, each for
/// its own owner (Adaptable `0x9f608`, DieForYou `0xa1861`, Illusion
/// `0xa3b33`, PainfulStabs `0xa5564`, Reattach `0xa665b`, SteamEruption
/// `0xa85fe`). Of those only Adaptable and PainfulStabs are represented; a
/// corpse carrying either refuses by name rather than guess its membership.
/// The callers' encounters (Fabricator, Living Fog, Two-Tailed Rats) carry
/// none of the other four.
///
/// This answers for a seam inside a monster move, after every earlier death
/// has finished. It is wrong inside a death's own `Hook.AfterDeath` fan-out,
/// which runs (`IL_03bc`) before that creature's removal (`IL_04d5`), so the
/// dying creature is still a member there (Gremlin Merc's Surprise, #3146).
pub(crate) fn in_native_enemies(monster: &HotMonster) -> Result<bool, EngineRefusal> {
    if monster.hp > 0 {
        return Ok(true);
    }
    if monster.powers.value(PowerId::Adaptable) != 0
        || monster.powers.value(PowerId::PainfulStabs) != 0
    {
        return Err(EngineRefusal::MalformedArgs(
            "spawn seam: corpse kept in Enemies by a death veto",
        ));
    }
    Ok(false)
}

/// Slots held by native `CombatState.Enemies` members (see
/// [`in_native_enemies`]); a removed corpse releases its slot.
fn native_enemy_slots(state: &HotState) -> Result<Vec<i32>, EngineRefusal> {
    let mut used = Vec::new();
    for monster in state.monsters.iter() {
        if in_native_enemies(monster)? {
            used.push(monster.slot);
        }
    }
    Ok(used)
}

/// `SetUniqueMonsterHpValue` over the live native enemy list.
///
/// `CombatState::CreateCreature` (v0.111.0 RVA `0x137074`) passes
/// `_enemies` itself (`IL_0031`, argument at `IL_0044`) to
/// `Creature::SetUniqueMonsterHpValue` (`0x11d3ec`, call at `IL_0055`), which
/// `ExceptWith`s those creatures' `MaxHp` (`IL_0079`). Corpses already
/// removed from `_enemies` (see [`in_native_enemies`]) therefore reserve no
/// max HP, although the hot roster retains their rows.
fn roll_unique_hp_native_enemies(
    state: &mut HotState,
    low: i32,
    high: i32,
) -> Result<i32, EngineRefusal> {
    let mut taken = Vec::new();
    for monster in state.monsters.iter() {
        if in_native_enemies(monster)? {
            taken.push(monster.max_hp);
        }
    }
    roll_unique_hp_over(state, low, high, &taken)
}

fn roll_unique_hp_over(
    state: &mut HotState,
    low: i32,
    high: i32,
    taken: &[i32],
) -> Result<i32, EngineRefusal> {
    if low > high {
        return Err(EngineRefusal::MalformedArgs("monster HP range"));
    }
    let mut candidates: Vec<i32> = (low..=high)
        .filter(|candidate| !taken.contains(candidate))
        .collect();
    if candidates.is_empty() {
        candidates.extend(low..=high);
    }
    let live = state.rng.get(RngStream::Niche);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let bound: i32 = candidates
        .len()
        .try_into()
        .map_err(|_| overflow("monster HP candidates"))?;
    let pick: usize = rng
        .next_bounded(bound)
        .map_err(|_| overflow("monster HP candidates"))?
        .try_into()
        .map_err(|_| overflow("monster HP result"))?;
    state.rng.set(
        RngStream::Niche,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    Ok(candidates[pick])
}

/// One ordinary inclusive Niche HP roll. Tough Egg's retained-object hatch
/// does not enter `SetUniqueMonsterHpValue`; equal hatched HP is observable.
fn roll_hp(state: &mut HotState, low: i32, high: i32) -> Result<i32, EngineRefusal> {
    let bound = high
        .checked_sub(low)
        .and_then(|width| width.checked_add(1))
        .filter(|bound| *bound > 0)
        .ok_or(EngineRefusal::MalformedArgs("monster HP range"))?;
    let live = state.rng.get(RngStream::Niche);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let offset = rng
        .next_bounded(bound)
        .map_err(|_| overflow("monster HP range"))?;
    let hp = low
        .checked_add(offset)
        .ok_or_else(|| overflow("monster HP result"))?;
    state.rng.set(
        RngStream::Niche,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    Ok(hp)
}

/// Allocate the uid of one newly created enemy creature (#3039).
///
/// Native `CombatState::CreateCreature` (v0.111.0 RVA `0x137074`) attaches
/// the new creature at `IL_007e`, and `CombatState::AttachCreature`
/// (`0x13718f`) stamps `CombatId = _nextCreatureId++` at `IL_0009`-`IL_0022`.
/// That one counter also advanced for the player (`AddPlayer` `0x137059`
/// `IL_0008`) and for every pet `PlayerCmd.AddPet` created
/// (`<AddPet>d__14`1::MoveNext` `0x3ee68c` `IL_0054`), and nothing else
/// reads or writes it (`_nextCreatureId`'s only other reader is
/// `<GetCreatureAsync>d__60` `0x3f927c` `IL_0097`, a network wait). So a
/// spawn's uid is the tracked counter
/// ([`crate::hot::FanoutState::next_creature_uid`]), which this advances.
///
/// An untracked (zero) counter is the canonical default of every root with
/// no combat-start pet; there `max(monster uid) + 1` is the counter. A pet
/// under an untracked counter (an Osty, live or retained, or a Byrdpip /
/// Pael's Legion relic pet, [`crate::hot::HotFanouts::relic_pet_creatures`])
/// holds a creature id this state cannot place relative to the enemies (a
/// root serialized before #3039), so that refuses by name (I5).
///
/// The same `CombatId` also seeds each monster's own `MonsterModel.Rng`
/// (`CreateCreature` `IL_00ed`-`IL_0102`), but its only reader is
/// `MonsterModel::get_Rng` (`0x81f38`), whose only caller is
/// `ToughEgg::SetupSkins` (`0xc30d4` `IL_0020`, a visual skin pick). No
/// combat state reads it, so this crate keeps no per-monster stream.
///
/// `count` consecutive creations are allocated at once and the first uid is
/// returned, because an untracked counter is re-derived from the roster and
/// a creature created but not yet published (Surprise's Fat) is not on it.
pub(crate) fn allocate_creature_uids(
    state: &mut HotState,
    count: u32,
) -> Result<u32, EngineRefusal> {
    let tracked = state.fanouts.next_creature_uid();
    let floor = state
        .monsters
        .iter()
        .map(|monster| monster.uid)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| overflow("monster uid"))?;
    if tracked == 0 {
        if state.fanouts.pet().has_die_for_you() || state.fanouts.relic_pet_creatures() != 0 {
            return Err(EngineRefusal::MalformedArgs(
                "creature id counter untracked with a live pet",
            ));
        }
        floor
            .checked_add(count.saturating_sub(1))
            .ok_or_else(|| overflow("monster uid"))?;
        return Ok(floor);
    }
    if tracked < floor {
        return Err(EngineRefusal::MalformedArgs(
            "creature id counter behind a monster uid",
        ));
    }
    let next = tracked
        .checked_add(count)
        .ok_or_else(|| overflow("creature id counter"))?;
    state.fanouts.set_next_creature_uid(next);
    Ok(tracked)
}

/// One creation: [`allocate_creature_uids`] with a count of one.
pub(crate) fn allocate_creature_uid(state: &mut HotState) -> Result<u32, EngineRefusal> {
    allocate_creature_uids(state, 1)
}

/// The first uid of the last `created` enemy creations when the counter is
/// tracked (`Some(None)` if that underflows), and `None` when it is not.
///
/// Validators of a spawn lineage read the counter backwards: after a
/// complete spawn no other ENEMY is created in those encounters, so the
/// lineage ends at the last id handed out.
///
/// One creature can still be created after it: the player's Osty (#3904).
/// `CombatState::AttachCreature` (`0x13718f` `IL_0009`-`IL_0022`) numbers
/// creatures in creation order whatever they are, and a first
/// `OstyCmd.Summon` (`<Summon>d__0` `0x3ee040` `IL_01e9`) creates one whenever
/// it runs. On the game itself an Afterlife played after Gremlin Merc's
/// Surprise gives Fat 2, Sneaky 3 and the Osty 4, and played before it gives
/// the Osty 2, Fat 3 and Sneaky 4; around Fabricator's bots it is the same
/// (`tests/native/osty_*.native.json`). So when the recorded Osty id
/// ([`crate::hot::FanoutState::osty_creature_uid`]) is the last id handed
/// out, the lineage ends one earlier. An Osty created before the lineage, or
/// at combat start, sits below it and changes nothing here.
pub(crate) fn tracked_lineage_start(state: &HotState, created: u32) -> Option<Option<u32>> {
    match state.fanouts.next_creature_uid() {
        0 => None,
        tracked => {
            let osty = state.fanouts.osty_creature_uid();
            let end = if osty != 0 && osty.checked_add(1) == Some(tracked) {
                osty
            } else {
                tracked
            };
            Some(end.checked_sub(created))
        }
    }
}

/// Whether `uid` is the recorded mid-fight Osty's creature id (#3904), which
/// no enemy holds.
pub(crate) fn is_osty_creature_uid(state: &HotState, uid: u32) -> bool {
    let osty = state.fanouts.osty_creature_uid();
    osty != 0 && osty == uid
}

/// Validate the complete pre-mutation Stock replacement boundary.
///
/// Python `_respawn_axebot` (frozen, deleted #2827) accepts only the unique retained
/// Axebot corpse with Stock 1 or 2. The replacement keeps the board slot but
/// receives a fresh creation uid and otherwise starts from a blank creature.
pub(crate) fn require_axebot_respawn(
    state: &HotState,
    owner_uid: u32,
) -> Result<(), EngineRefusal> {
    let [owner] = state.monsters.as_slice() else {
        return Err(EngineRefusal::MalformedArgs("Axebot Stock lifecycle"));
    };
    let stock = owner.powers.value(PowerId::Stock);
    if !matches!(stock, 1 | 2) {
        return Err(EngineRefusal::MalformedArgs("Axebot Stock lifecycle"));
    }
    let respawn_count = AXEBOT_STOCK - stock;
    // The band this body was rolled from, at the fight's tier (#2539).
    let ascension = state.fanouts.ascension();
    let bonus = AXEBOT_RESPAWN_HP_BONUS * respawn_count;
    let expected_low = crate::encounters::tier(AXEBOT_MIN_HP, ascension) as i32 + bonus;
    let expected_high = crate::encounters::tier(AXEBOT_MAX_HP, ascension) as i32 + bonus;
    if owner.kind != MonsterKind::Axebot
        || owner.uid != owner_uid
        || owner.slot != 0
        || owner.hp > 0
        || axebot_expected_uid(state, respawn_count) != Some(owner.uid)
        || !(expected_low..=expected_high).contains(&owner.max_hp)
        || !matches!(owner.loop_pos, 0..=2)
        || owner.uid == u32::MAX
    {
        return Err(EngineRefusal::MalformedArgs("Axebot Stock lifecycle"));
    }
    Ok(())
}

/// StockPower.AfterDeath's in-place Axebot replacement.
///
/// The dead native model is reused at the same board slot, but `CreatureCmd`
/// creates a fresh creature identity: Stock decrements first, one Niche draw
/// chooses unique HP from the respawn-adjusted inclusive range, and the fresh
/// owner opens on BOOT_UP with every other power/private field reset.
///
/// The dying body's max HP is reserved in that draw (#3756). v0.111.0
/// `sts2.dll` (SHA-256 `9cb4f1ad…12b4`):
/// `StockPower/<AfterDeath>d__4::MoveNext` RVA `0x346280` calls
/// `CreatureCmd.Add<Axebot>` (IL_0090), whose `ICombatState::CreateCreature`
/// (`CreatureCmd/<Add>d__1::MoveNext` RVA `0x3e9130` IL_003a) hands
/// `_enemies` to `Creature::SetUniqueMonsterHpValue` (RVA `0x11d3ec`). That
/// builds `Range(MinInitialHp, MaxInitialHp + 1 - MinInitialHp)`
/// (IL_001f-IL_0042), removes the `MaxHp` of every listed creature but the
/// new one (`Except` IL_0050, `ExceptWith` IL_0079) and draws
/// `NextItem` over what is left (IL_0089). `Hook.AfterDeath` is awaited at
/// IL_03bc of `CreatureCmd/<KillWithoutCheckingWinCondition>d__15::MoveNext`
/// (RVA `0x3ebe90`), before `ICombatState::RemoveCreature` at IL_04d5, so the
/// dying Axebot is still listed. The bands of consecutive bodies share one
/// value (86 and 96 at A8+, 78 and 88 below: `get_MaxInitialHp` of one is
/// `get_MinInitialHp` of the next), so a body rolled at its band's top
/// narrows its successor's draw from eleven candidates to ten. Until #3756
/// this excluded the dying body, which is the same thing Surprise got wrong
/// before #3146.
pub(crate) fn respawn_axebot(
    state: &mut HotState,
    owner_uid: u32,
    events: &mut Vec<crate::engine::Event>,
) -> Result<(), EngineRefusal> {
    require_axebot_respawn(state, owner_uid)?;
    let owner = &state.monsters[0];
    let stock = owner.powers.value(PowerId::Stock);
    let new_stock = stock - 1;
    let respawn_count = AXEBOT_STOCK - new_stock;
    let hp_bonus = AXEBOT_RESPAWN_HP_BONUS
        .checked_mul(respawn_count)
        .ok_or(EngineRefusal::CounterOverflow("Axebot respawn HP"))?;
    let ascension = state.fanouts.ascension();
    let low = (crate::encounters::tier(AXEBOT_MIN_HP, ascension) as i32)
        .checked_add(hp_bonus)
        .ok_or(EngineRefusal::CounterOverflow("Axebot respawn HP"))?;
    let high = (crate::encounters::tier(AXEBOT_MAX_HP, ascension) as i32)
        .checked_add(hp_bonus)
        .ok_or(EngineRefusal::CounterOverflow("Axebot respawn HP"))?;
    let next_uid = allocate_creature_uid(state)?;
    let hp = roll_unique_hp(state, low, high)?;

    let mut replacement = HotMonster::new(MonsterKind::Axebot, hp);
    replacement.max_hp = hp;
    replacement.slot = 0;
    replacement.uid = next_uid;
    replacement.loop_pos = AXEBOT_BOOT_UP_LOOP_POS;
    if new_stock > 0 {
        replacement
            .powers
            .set(PowerId::Stock, SlotWire::Int, new_stock);
    }
    state.monsters_mut()[0] = replacement;
    fur_coat_after_opponent_added(state, next_uid)?;
    super::damage::note_power(
        events,
        crate::engine::Subject::Monster(next_uid),
        PowerId::Stock,
        new_stock,
    );
    Ok(())
}

/// The loop position a Stock replacement is created at: `BOOT_UP_MOVE`, the
/// machine's initial state when `_stockOverrideAmount` has a value
/// (`Axebot::GenerateMoveStateMachine` RVA `0xaee50`, IL_00ca-IL_00d8).
pub(crate) const AXEBOT_BOOT_UP_LOOP_POS: i32 = 2;

fn insert_slot_ordered(state: &mut HotState, monster: HotMonster) {
    let roster = state.monsters_mut();
    let position = roster
        .iter()
        .position(|candidate| candidate.slot > monster.slot)
        .unwrap_or(roster.len());
    roster.insert(position, monster);
}

/// Fur Coat's `BeforeCombatStart` in a marked room: every hittable enemy of
/// the initial roster starts at 1 HP (#3634).
///
/// # The native contract
///
/// v0.111.0 `sts2.dll` (sha256 `9cb4f1ad…12b4`).
/// `FurCoat/<BeforeCombatStart>d__26::MoveNext` (RVA `0x325360`):
///
/// * leaves unless `GetMarkedCoords` is non-null and holds
///   `Owner.RunState.CurrentMapPoint.coord` (`IL_0021`-`IL_0047`). That
///   membership is the entry's (`entry::relics::fur_coat_membership`), carried
///   here as the `fur_coat_active` state flag;
/// * `CurrentMapPointHistoryEntry.GetEntry(Owner.NetId).IsAffectedByFurCoat =
///   true` (`IL_004d`-`IL_0072`), run history only, and `Flash`;
/// * `Owner.Creature.CombatState.HittableEnemies` (`IL_007e`-`IL_008d`), a VFX
///   call over them, and per enemy `CreatureCmd::SetCurrentHp(enemy,
///   Decimal::One)` (`IL_00b9`-`IL_00c7`), awaited one at a time.
///
/// `CreatureCmd/<SetCurrentHp>d__21::MoveNext` (`0x3ecbe8`) calls
/// `Creature::SetCurrentHpInternal` (`0x11d734`: `CurrentHp = Math.Min(amount,
/// MaxHp)`, so max HP is untouched and an enemy with less than one max HP
/// would keep it) and, when the value changed, `Hook::AfterCurrentHpChanged`.
///
/// # Which enemies
///
/// `Creature::get_IsHittable` (`0x11d298`) is `!IsDead &&
/// Hook.ShouldAllowHitting`. The hook's overriders in the assembly are
/// `AdaptablePower`, `IllusionPower` and `ReattachPower`, each of which
/// vetoes only its own owner while that owner is reviving after a death, and
/// `DieForYouPower`, which vetoes dead creatures (the two `Mock…` powers are
/// test fixtures). No creature of an initial roster has died yet, so at this
/// hook the hittable enemies are exactly the living ones.
///
/// # What the HP write wakes
///
/// The overriders of `AfterCurrentHpChanged` in the assembly:
///
/// * `MeatOnTheBone` (`0x96af0`) returns unless the creature is its owner's
///   (`IL_0001`-`IL_000d`), and then only sets its display status;
/// * `Crusher` (`0xb203b`) and `Rocket` (`0xbcabb`) act only on their own
///   creature (`IL_0001`-`IL_0008`) and only play a hurt animation;
/// * `NecroMasteryPower` is a combat power, and none exists at combat start;
/// * `RedSkull` (`<AfterCurrentHpChanged>d__13` `0x32f5ec`) has **no**
///   creature test: once `CombatManager.IsInProgress` (`IL_001d`-`IL_0027`,
///   already true at this hook) it awaits `ModifyStrengthIfNecessary`
///   (`IL_002c`), which re-reads its owner's HP against its latch.
///
/// So the write itself is the whole effect, with one exception. It is not
/// damage, so no damage hook, threshold power or death check runs, and every
/// enemy is still alive at 1. The exception is Red Skull's re-check. It is a
/// no-op whenever the latch already agrees with the owner's HP, which the
/// relic's own room-entry body has just made true, except where this crate
/// deliberately left the latch stale (Planisphere's heal crossing half HP
/// after Red Skull applied, `entry::opening`'s
/// `RedSkullRoomEntry::BeforeCrossingHeal`): there native would settle the
/// latch here rather than on turn one, so the opening refuses a marked Fur
/// Coat room in that case
/// (`entry::opening::fur_coat_red_skull_recheck_is_inert`).
///
/// # Position
///
/// A first-walk `BeforeCombatStart` listener, ordered among its peers by
/// relic acquisition in native. No other listener of the hook reads or
/// writes an enemy's HP (the represented ones are the combat-start pets and
/// summons, Vambrace's reset, Belt Buckle's latch, Delicate Frond's belt fill
/// and the compiled templates, none of which touches a monster), so the fixed
/// place is not observable. Enemies added later take the same write in
/// [`fur_coat_after_opponent_added`].
pub(crate) fn fur_coat_before_combat_start(state: &mut HotState) {
    if !state.fur_coat_active() {
        return;
    }
    for monster in state.monsters_mut().iter_mut() {
        if monster.hp > 0 {
            monster.hp = monster.max_hp.min(1);
        }
    }
    crate::coverage::record_relic(crate::ids::RelicId::RelicFurCoat);
}

/// Fur Coat's global enemy-add listener. Initial enemies have already passed
/// this hook before canonical projection; every represented mid-combat enemy
/// creation converges here immediately after publication.
pub(crate) fn fur_coat_after_opponent_added(
    state: &mut HotState,
    monster_uid: u32,
) -> Result<(), EngineRefusal> {
    let fur_coat = state.fur_coat_active();
    let philosophers_stone = state.philosophers_stone_owned() && !state.history.over;
    let murderous = state.fanouts.murderous() && !state.history.over;
    if !fur_coat && !philosophers_stone && !murderous {
        return Ok(());
    }
    let upkeep = state.fanouts.misery_attachment_upkeep();
    let monster = state
        .monsters_mut()
        .iter_mut()
        .find(|monster| monster.uid == monster_uid)
        .ok_or(EngineRefusal::MalformedArgs("Fur Coat added enemy uid"))?;
    if fur_coat {
        monster.hp = monster.max_hp.min(1);
        crate::coverage::record_relic(crate::ids::RelicId::RelicFurCoat);
    }
    if philosophers_stone && monster.hp > 0 {
        let strength = monster
            .powers
            .value(PowerId::Strength)
            .checked_add(1)
            .ok_or(EngineRefusal::CounterOverflow(
                "Philosopher's Stone strength",
            ))?;
        // `PhilosophersStone::AfterCreatureAddedToCombat` `0x994f0`
        // IL_004b-IL_004e and `<AfterRoomEntered>d__8::MoveNext` `0x32dff0`
        // IL_0098-IL_009b both pass a literal **null** applier, so
        // `set_Applier` `0x3efbac` IL_013d writes null and the row records
        // `Applier::None` — a state distinct from "unrecorded" and from any
        // creature.
        super::damage::write_monster_strength(monster, strength, crate::hot::Applier::None, upkeep);
        crate::coverage::record_relic(crate::ids::RelicId::RelicPhilosophersStone);
    }
    // The Murderous run modifier (#3934). `Murderous::
    // AfterCreatureAddedToCombat` (v0.111.0 RVA `0xc666f`) returns for a
    // creature whose `Side` is `CombatSide.Player` (`ldc.i4.1`,
    // IL_0002-IL_000f) and otherwise calls `PowerCmd.Apply<StrengthPower>`
    // of `ldc.i4.3` with a null applier and a null card (IL_0010-IL_001f).
    // It runs after the relic rows above: modifiers follow the player's
    // relics in `CombatState/<IterateHookListeners>d__69::MoveNext`
    // (`0x3f9720`, IL_0219-IL_0225). Both rows only add Strength, so the
    // order is not observable here. Only opponents reach this function.
    if murderous && monster.hp > 0 {
        let strength = monster
            .powers
            .value(PowerId::Strength)
            .checked_add(MURDEROUS_STRENGTH)
            .ok_or(EngineRefusal::CounterOverflow("Murderous strength"))?;
        super::damage::write_monster_strength(monster, strength, crate::hot::Applier::None, upkeep);
    }
    Ok(())
}

/// The Murderous run modifier's Strength: `ldc.i4.3` in both of its bodies
/// (`Murderous/<AfterRoomEntered>d__1::MoveNext` RVA `0x377cf8` IL_003a and
/// `Murderous::AfterCreatureAddedToCombat` RVA `0xc666f` IL_0016). The type's
/// `_strengthAmount` field is a constant with no initializer body: the type
/// has no static constructor, and neither body loads a field.
pub(crate) const MURDEROUS_STRENGTH: i32 = 3;

/// Fabricator's bot spawn: filter the selected pool by the one shared
/// `_lastSpawned`, spend one AI draw even for a singleton, take the first
/// free named slot (or the native empty slot `-1`), spend one unique-HP Niche
/// draw, then apply intrinsic bot powers.
///
/// Slot (#3123): `<SpawnBot>d__25::MoveNext` (v0.111.0 RVA `0x35a80c`)
/// passes `CombatState.Encounter.GetNextSlot(CombatState)` (`IL_0082`-
/// `IL_008d`) to `CreatureCmd.Add` (`IL_0092`). `EncounterModel::GetNextSlot`
/// (`0x7f854`) is `Slots.FirstOrDefault(s => Enemies.All(e => e.SlotName !=
/// s), string.Empty)` (closures `0x31cf7c`/`0x31cfba`) over
/// `FabricatorNormal::get_Slots` (`0xd3a53`: bot1, bot2, fabricator, bot3,
/// bot4). A dead bot has left `Enemies` ([`in_native_enemies`]), so its slot
/// is free again and the new bot takes it; `CombatManager::AddCreature`
/// (`0x1360f8`) then re-sorts `Enemies` by slot index
/// (`SortEnemiesBySlotName` `0x137419`, `IL_0046`), which
/// [`insert_slot_ordered`] reproduces. The corpse row is retained for uid
/// identity and shares the slot number; the uid stays the creation-order
/// counter ([`allocate_creature_uid`]), distinct from the slot. The HP draw
/// likewise excludes removed corpses ([`roll_unique_hp_native_enemies`]).
pub(crate) fn spawn_fabricator_bot(
    state: &mut HotState,
    owner_uid: u32,
    options: &[MonsterKind; 2],
) -> Result<(), EngineRefusal> {
    let owner_index = state
        .monsters
        .iter()
        .position(|monster| monster.uid == owner_uid)
        .ok_or(EngineRefusal::MalformedArgs("Fabricator spawn owner"))?;
    if state.monsters[owner_index].kind != MonsterKind::Fabricator {
        return Err(EngineRefusal::MalformedArgs("Fabricator spawn owner"));
    }
    if state.history.over || state.monsters[owner_index].hp <= 0 {
        return Ok(());
    }
    if options != &FABRICATOR_AGGRO_BOTS && options != &FABRICATOR_DEFENSE_BOTS {
        return Err(EngineRefusal::MalformedArgs("Fabricator spawn pool"));
    }
    let last_spawned = state.monsters[owner_index].last_spawned;
    let candidates: Vec<MonsterKind> = options
        .iter()
        .copied()
        .filter(|kind| Some(*kind) != last_spawned)
        .collect();
    let bound: i32 = candidates
        .len()
        .try_into()
        .map_err(|_| overflow("Fabricator spawn candidates"))?;
    let live = state.rng.get(RngStream::Ai);
    let mut rng = Xoshiro256StarStar {
        words: live.words,
        counter: live.counter,
    };
    let selected: usize = rng
        .next_bounded(bound)
        .map_err(|_| overflow("Fabricator spawn candidates"))?
        .try_into()
        .map_err(|_| overflow("Fabricator spawn result"))?;
    let kind = candidates[selected];
    let used = native_enemy_slots(state)?;
    let slot = (0..FABRICATOR_SLOTS)
        .find(|slot| !used.contains(slot))
        .unwrap_or(-1);
    if !matches!(
        kind,
        MonsterKind::Guardbot | MonsterKind::Noisebot | MonsterKind::Stabbot | MonsterKind::Zapbot
    ) {
        return Err(EngineRefusal::MalformedArgs("Fabricator spawn kind"));
    }
    let (low, high) = native_hp_band(state, kind)?;
    let hp = roll_unique_hp_native_enemies(state, low, high)?;
    let uid = allocate_creature_uid(state)?;
    state.rng.set(
        RngStream::Ai,
        RngStreamState {
            words: rng.words,
            counter: rng.counter,
        },
    );
    let owner_index = state
        .monsters
        .iter()
        .position(|monster| monster.uid == owner_uid)
        .expect("Fabricator owner cannot disappear during a synchronous spawn");
    state.monsters_mut()[owner_index].last_spawned = Some(kind);

    let mut bot = HotMonster::new(kind, hp);
    bot.max_hp = hp;
    bot.slot = slot;
    bot.uid = uid;
    if kind == MonsterKind::Zapbot {
        bot.powers.set(PowerId::HighVoltage, SlotWire::Int, 2);
    }
    // `CreatureCmd.Add` and its deterministic initial roll complete before
    // MinionPower is applied. No relic is admitted, so only this final
    // ordering state is externally visible in the Rust slice.
    bot.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
    insert_slot_ordered(state, bot);
    fur_coat_after_opponent_added(state, uid)?;
    Ok(())
}

/// Living Fog BLOAT's gas-bomb spawn.
///
/// Slot (#3123): `<BloatMove>d__26::MoveNext` (v0.111.0 RVA `0x3620c0`)
/// takes `Encounter.GetNextSlot(CombatState)` (`IL_00c9`) and skips the
/// spawn when it is null or empty (`IL_00d0`). `GetNextSlot` (`0x7f854`) is
/// the first `LivingFogNormal::get_Slots` (`0xd42db`) name (bomb1..bomb5,
/// livingFog) that
/// no `Enemies` member holds, so an exploded or killed bomb, which has left
/// `Enemies` ([`in_native_enemies`]), frees its slot for the next bomb. The
/// Living Fog itself holds `livingFog` (slot 5), outside the bomb range. The
/// HP draw excludes removed corpses ([`roll_unique_hp_native_enemies`]).
pub(crate) fn spawn_gas_bomb(state: &mut HotState, owner_uid: u32) -> Result<(), EngineRefusal> {
    let owner = state
        .monsters
        .iter()
        .find(|monster| monster.uid == owner_uid)
        .ok_or(EngineRefusal::MalformedArgs("bloat owner"))?;
    if owner.kind != MonsterKind::LivingFog {
        return Err(EngineRefusal::MalformedArgs("bloat owner"));
    }
    if state.history.over || owner.hp <= 0 {
        return Ok(());
    }
    let used = native_enemy_slots(state)?;
    let Some(slot) = (0..5).find(|slot| !used.contains(slot)) else {
        return Ok(());
    };
    let (low, high) = native_hp_band(state, MonsterKind::GasBomb)?;
    let hp = roll_unique_hp_native_enemies(state, low, high)?;
    let uid = allocate_creature_uid(state)?;
    let mut bomb = HotMonster::new(MonsterKind::GasBomb, hp);
    bomb.max_hp = hp;
    bomb.slot = slot;
    bomb.uid = uid;
    insert_slot_ordered(state, bomb);
    fur_coat_after_opponent_added(state, uid)?;
    Ok(())
}

/// Whether the complete hot roster matches `_validated_ovicopter_roster`
/// (frozen Python, deleted #2827). This is shared by admission and every mutating caller;
/// Python's front-of-validator solo clause is part of the same lifecycle.
pub(crate) fn ovicopter_roster_is_valid(state: &HotState) -> bool {
    if state.multiplayer_ally_key != 0
        || state
            .monsters
            .iter()
            .any(|monster| !matches!(monster.kind, MonsterKind::Ovicopter | MonsterKind::ToughEgg))
    {
        return false;
    }
    let mut parents = state
        .monsters
        .iter()
        .filter(|monster| monster.kind == MonsterKind::Ovicopter);
    let Some(parent) = parents.next() else {
        return false;
    };
    if parents.next().is_some()
        || parent.slot != OVICOPTER_SLOTS - 1
        || parent.uid != 0
        || parent.powers.value(PowerId::Secondary) != 0
        || parent.powers.value(PowerId::IsHatched) != 0
        || parent.powers.value(PowerId::HatchPower) != 0
        || !(0..=OVICOPTER_BRANCH_POS).contains(&parent.loop_pos)
        || parent.revive_stage != 0
    {
        return false;
    }
    let mut eggs: Vec<&HotMonster> = state
        .monsters
        .iter()
        .filter(|monster| monster.kind == MonsterKind::ToughEgg)
        .collect();
    if eggs.iter().filter(|egg| egg.hp > 0).count() >= OVICOPTER_SLOTS as usize {
        return false;
    }
    let mut live_slots = std::collections::BTreeSet::new();
    if eggs
        .iter()
        .any(|egg| egg.hp > 0 && !live_slots.insert(egg.slot))
    {
        return false;
    }
    eggs.sort_unstable_by_key(|egg| egg.uid);
    let mut previous_uid = 0;
    for egg in eggs {
        if egg.uid <= previous_uid {
            return false;
        }
        previous_uid = egg.uid;
        let secondary = egg.powers.get(PowerId::Secondary);
        let hatched = egg.powers.get(PowerId::IsHatched);
        let hatch = egg.powers.get(PowerId::HatchPower);
        if !(0..OVICOPTER_SLOTS - 1).contains(&egg.slot)
            || !secondary.is_some_and(|slot| slot.wire == SlotWire::Bool && slot.value == 1)
            || hatched.is_some_and(|slot| slot.wire != SlotWire::Bool || slot.value != 1)
            || hatch.is_some_and(|slot| slot.wire != SlotWire::Int || slot.value <= 0)
            || !(0..2).contains(&egg.loop_pos)
            || egg.revive_stage != 0
        {
            return false;
        }
        let is_hatched = hatched.is_some();
        let hatch_amount = hatch.map_or(0, |slot| slot.value);
        // An unhatched egg may hold no HatchPower (#3823): a stun over
        // HATCH_MOVE parks it, and `HatchPower/<AfterSideTurnEnd>d__4::
        // MoveNext` `0x33bd80` decrements at its owner's side end whatever
        // the owner's move (the participant test IL_001e-IL_002e, then
        // `PowerCmd.Decrement` `0x13394c` at IL_0033), so the side end of the
        // STUNNED turn takes 1 to nothing and the egg hatches when the parked
        // HATCH_MOVE performs (native: `ovi_rnd40_*`, `ovi_rndk60_*`).
        let valid_state = if egg.hp > 0 {
            if is_hatched {
                hatch_amount == 0 && egg.loop_pos == 1
            } else {
                matches!(hatch_amount, 0..=2) && egg.loop_pos == 0
            }
        } else {
            hatch_amount == 0
        };
        if !valid_state {
            return false;
        }
    }
    true
}

fn require_ovicopter_roster(state: &HotState, owner_uid: u32) -> Result<(), EngineRefusal> {
    if !ovicopter_roster_is_valid(state)
        || state
            .monsters
            .iter()
            .filter(|monster| monster.uid == owner_uid)
            .count()
            != 1
    {
        return Err(EngineRefusal::MalformedArgs("Ovicopter roster lifecycle"));
    }
    Ok(())
}

/// Corrects the frozen Python `_spawn_tough_egg` (deleted #2827), which
/// retains dead slots. Ovicopter::LayEggsMove, v0.111.0 RVA 0x3651e8: choose the last
/// encounter slot absent from CombatState.Enemies, then Add<ToughEgg>.
/// Dead eggs have left native Enemies and release their slot; the sim retains
/// their UID rows for action identity, so neither slot occupancy nor the
/// creation HP uniqueness census may count those retained corpses.
/// Encounter slots: OvicopterNormal::get_Slots RVA 0xd4643 (five egg slots).
/// Chosen replacement keeps a fresh monotone UID, never reuses the dead UID.
pub(crate) fn spawn_tough_egg(state: &mut HotState, parent_uid: u32) -> Result<(), EngineRefusal> {
    require_ovicopter_roster(state, parent_uid)?;
    let parent = state
        .monsters
        .iter()
        .find(|monster| monster.uid == parent_uid)
        .expect("validated Ovicopter parent uid");
    if parent.kind != MonsterKind::Ovicopter {
        return Err(EngineRefusal::MalformedArgs("lay_tough_eggs owner"));
    }
    if state.history.over || parent.hp <= 0 {
        return Ok(());
    }
    let used: Vec<i32> = state
        .monsters
        .iter()
        .filter(|monster| monster.hp > 0)
        .map(|monster| monster.slot)
        .collect();
    let Some(slot) = (0..OVICOPTER_SLOTS - 1)
        .rev()
        .find(|slot| !used.contains(slot))
    else {
        return Ok(());
    };
    // Allocate before removing corpses so creation ordinals never rewind.
    // Native CombatState::RemoveCreature (RVA 0x1371d0) removes dead eggs
    // from Enemies. Compact at this insertion seam to keep the bounded roster
    // and avoid counting their max HP in SetUniqueMonsterHpValue.
    let uid = allocate_creature_uid(state)?;
    state.monsters_mut().retain(|monster| monster.hp > 0);
    let ascension = state.fanouts.ascension();
    let hp = roll_unique_hp(
        state,
        crate::encounters::tier(TOUGH_EGG_MIN_HP, ascension) as i32,
        crate::encounters::tier(TOUGH_EGG_MAX_HP, ascension) as i32,
    )?;
    let mut egg = HotMonster::new(MonsterKind::ToughEgg, hp);
    egg.max_hp = hp;
    egg.slot = slot;
    egg.uid = uid;
    egg.powers.set(PowerId::HatchPower, SlotWire::Int, 2);
    // Native MinionPower is applied after the global add-listener walk. No
    // represented relic subscribes to that walk, so only the final ordering
    // state is projected here.
    egg.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
    insert_slot_ordered(state, egg);
    fur_coat_after_opponent_added(state, uid)?;
    require_ovicopter_roster(state, parent_uid)
}

/// Tough Egg HATCH's `_hatch_tough_egg` (frozen Python, deleted #2827).
///
/// HatchPower may be 1 or absent (#3823: a stun over HATCH_MOVE outlives it,
/// see [`ovicopter_roster_is_valid`]). `<HatchMove>d__35::MoveNext`
/// `0x372874` never reads the amount: it sets `IsHatched` (IL_002d-IL_002e)
/// and then removes powers from a filtered copy of the egg's list
/// (IL_00aa-IL_0100; MinionPower stays, as every native trace shows), with
/// no test of HatchPower's amount anywhere in the body.
pub(crate) fn hatch_tough_egg(state: &mut HotState, egg_uid: u32) -> Result<(), EngineRefusal> {
    require_ovicopter_roster(state, egg_uid)?;
    let index = state
        .monsters
        .iter()
        .position(|monster| monster.uid == egg_uid)
        .ok_or(EngineRefusal::MalformedArgs("hatch_tough_egg owner"))?;
    let egg = &state.monsters[index];
    if egg.kind != MonsterKind::ToughEgg
        || egg.hp <= 0
        || egg.powers.value(PowerId::IsHatched) != 0
        || !matches!(egg.powers.value(PowerId::HatchPower), 0 | 1)
        || egg.loop_pos != 0
    {
        return Err(EngineRefusal::MalformedArgs("hatch_tough_egg owner/state"));
    }
    let ascension = state.fanouts.ascension();
    let hp = roll_hp(
        state,
        crate::encounters::tier(TOUGH_EGG_HATCHLING_MIN_HP, ascension) as i32,
        crate::encounters::tier(TOUGH_EGG_HATCHLING_MAX_HP, ascension) as i32,
    )?;
    let egg = &mut state.monsters_mut()[index];
    egg.powers = Slots::new();
    egg.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
    egg.powers.set(PowerId::IsHatched, SlotWire::Bool, 1);
    egg.misery_debuff_order = Default::default();
    egg.owner_powered_damage_results_this_turn = 0;
    egg.nonowner_same_side_powered_damage_results_this_turn = 0;
    egg.override_state = crate::hot::MonsterOverride::None;
    egg.poison_uid = -1;
    egg.hp = hp;
    egg.max_hp = hp;
    egg.loop_pos = 1;
    require_ovicopter_roster(state, egg_uid)
}

/// Obscura/Fogmog `_spawn_illusion` (frozen Python, deleted #2827).
pub(crate) fn spawn_illusion(
    state: &mut HotState,
    owner_uid: u32,
    kind: MonsterKind,
    hp_value: i32,
) -> Result<(), EngineRefusal> {
    let owner = state
        .monsters
        .iter()
        .find(|monster| monster.uid == owner_uid)
        .ok_or(EngineRefusal::MalformedArgs("summon_illusion owner"))?;
    let valid = matches!(
        (owner.kind, kind, hp_value),
        (MonsterKind::TheObscura, MonsterKind::Parafright, 21)
            | (MonsterKind::Fogmog, MonsterKind::EyeWithTeeth, 6)
    );
    if !valid || state.monsters.iter().any(|monster| monster.kind == kind) {
        return Err(EngineRefusal::MalformedArgs("summon_illusion owner/row"));
    }
    if state.history.over || owner.hp <= 0 {
        return Ok(());
    }
    // `CombatState::CreateCreature` (`0x137074`) rolls unique HP against
    // `_enemies` only (#3146, [`roll_unique_hp_native_enemies`]). The
    // encounter holds the living owner alone (a second illusion refused
    // above), so no corpse can reach this roll, and the illusion's
    // `MinInitialHp == MaxInitialHp` (Parafright `0xbb4a3`/`0xbb4a7`,
    // EyeWithTeeth `0xb39dc`/`0xb39df`) makes the result that one value
    // whichever set is excluded.
    let hp = roll_unique_hp_native_enemies(state, hp_value, hp_value)?;
    let uid = allocate_creature_uid(state)?;
    let mut illusion = HotMonster::new(kind, hp);
    illusion.max_hp = hp;
    illusion.slot = 0;
    illusion.uid = uid;
    insert_slot_ordered(state, illusion);
    fur_coat_after_opponent_added(state, uid)?;
    Ok(())
}

/// Whether one rat carries an exact generated four-branch AI state. The empty
/// form exists only between spawn publication and the immediate starter roll.
/// Otherwise current intent is the StateLog tail; Screech is rolled at most
/// once in the trailing cooldown window, and CALL_FOR_BACKUP is coupled to
/// its UseOnlyOnce record. No roll repeats Disease Bite or Screech (Scratch,
/// the first branch, repeats on the exact-zero roll), but a stun logs any
/// parked move a second time (#3823, `turn::resume_stunned_random_ai`), so
/// adjacent entries may be equal and the Screech window is asked of the log
/// with each run of equal entries read as one.
pub(crate) fn rat_ai_state_is_valid(monster: &HotMonster, allow_spawn_empty: bool) -> bool {
    if monster.kind != MonsterKind::TwoTailedRat || monster.loop_pos != 0 {
        return false;
    }
    let machine = monster.random_ai;
    if machine.next().is_none() {
        return allow_spawn_empty
            && machine.log_len() == 0
            && machine.once_len() == 0
            && monster.rat_turns_until_summonable() == 2
            && monster.rat_spawn_fresh();
    }
    let next = machine.next().expect("checked present intent");
    let log: Vec<u8> = (0..machine.log_len())
        .filter_map(|position| machine.log_at(position))
        .collect();
    let used = machine.once_len() == 1
        && machine.once_at(0) == Some(super::turn::RAT_CALL_FOR_BACKUP_INDEX);
    let screech_runs = log
        .iter()
        .enumerate()
        .filter(|(position, entry)| {
            **entry == super::turn::RAT_SCREECH_INDEX
                && (*position == 0 || log[*position - 1] != super::turn::RAT_SCREECH_INDEX)
        })
        .count();
    if machine.once_len() > usize::from(used)
        || log.len() != machine.log_len()
        || !(1..=3).contains(&log.len())
        || log.last() != Some(&next)
        || log
            .iter()
            .any(|entry| *entry > super::turn::RAT_CALL_FOR_BACKUP_INDEX)
        || screech_runs > 1
        || (log.contains(&super::turn::RAT_CALL_FOR_BACKUP_INDEX) && !used)
        || (log.len() < 3 && used)
        || (next == super::turn::RAT_CALL_FOR_BACKUP_INDEX
            && monster.rat_turns_until_summonable() > 0)
    {
        return false;
    }
    true
}

/// The native `TwoTailedRatsNormal::get_Slots` index of a rat's hot `slot`.
///
/// `TwoTailedRatsNormal::get_Slots` (v0.111.0 RVA `0xd59f2`) is `first,
/// second, third, fourth, fifth`, and `GenerateMonsters` (`0xd5a50`
/// `IL_0066`-`IL_00b2`) seats the three initial rats at `Slots[2]`,
/// `Slots[3]` and `Slots[4]`. The hot roster (and its frozen roster pin
/// `fixtures/normal_a_rosters_v1.json`) labels those three `0`, `1`, `2`, and
/// labels the two slots left of them `3` (`first`) and `4` (`second`). The
/// label is therefore the native index rotated by two; every slot choice and
/// every `Enemies` order is decided on the native index.
pub(crate) fn rat_native_slot(slot: i32) -> Option<i32> {
    (0..RAT_SLOTS)
        .contains(&slot)
        .then(|| (slot + 2) % RAT_SLOTS)
}

/// The hot label of native rat slot index `native` (inverse of
/// [`rat_native_slot`]).
fn rat_slot_label(native: i32) -> i32 {
    (native + RAT_SLOTS - 2) % RAT_SLOTS
}

/// The most CALL_FOR_BACKUP summons a Two-Tailed Rat fight can complete.
///
/// `TwoTailedRat::CanSummon` (RVA `0xc42c8`, `IL_0017`-`IL_001e`) refuses
/// once `CallForBackupCount >= 3`, and every completed call raises every
/// living rat's count by one (see [`spawn_rat`]). With no stun at most one
/// call is pending at a time (`IL_0048`-`IL_009b`), so three calls complete
/// and each is a summon.
///
/// A stun hides a parked call from that teammate test (#3823,
/// `GENERIC_STUN_RANDOM_TABLE_KINDS`), so calls can be pending on several
/// living rats at once, and more than three can complete. The bound is five:
///
/// * a call is only *rolled* while the roller's count is below 3
///   (`CanSummon` IL_0017-IL_001e), and the count is the number of calls
///   performed so far (every performed call raises every `Enemies` rat's
///   count to the maximum plus one, `<CallForBackup>d__41::MoveNext`
///   `0x373d98` IL_01e1-IL_025d);
/// * so every rat that ever rolls the call stood in the fight while fewer
///   than three calls had been performed: the three of the opening and the
///   at most two that the first two calls summoned. The rat the third call
///   summons arrives with the count at 3 and never rolls it;
/// * CALL_FOR_BACKUP is `UseOnlyOnce`: `RandomBranchState::GetStateWeight`
///   `0x79380` IL_0036-IL_0077 weighs it 0 once that rat's `StateLog` holds
///   it, and a rat's own stun only logs it again. Each of those five rats
///   rolls it at most once.
///
/// At most five calls are rolled, so at most five are performed, and a
/// summon is a performed call that found a free slot. (Five is reached
/// natively: the #3927 review recorded five summons with the count at 5.)
const RAT_MAX_SUMMONS: usize = 5;

/// The value `CallForBackupCount` is held at.
///
/// The count's one reader is `CanSummon`'s `CallForBackupCount < 3`
/// (`0xc42c8` `IL_0017`-`IL_001e`); its one writer is the synchronization in
/// `<CallForBackup>d__41::MoveNext` (`0x373d98` `IL_01e1`-`IL_025d`), `max +
/// 1` over the living rats. Native's count can pass 3 once a stun has let two
/// calls be pending together (#3823); every value from 3 up reads the same,
/// so the hot count saturates here and the document's `cfb` is
/// `min(CallForBackupCount, 3)`.
pub(crate) const RAT_CALL_COUNT_CAP: u8 = 3;

/// Whether this is a reachable Two-Tailed Rat roster at a serialized command
/// boundary (#3146).
///
/// Three initial rats hold labels 0/1/2 (native `third`..`fifth`, see
/// [`rat_native_slot`]) with uids 0/1/2 and pairwise distinct max HP. Each
/// completed CALL_FOR_BACKUP ([`spawn_rat`]) appends one row, retained after
/// death for uid identity, so a roster has three to ten rows
/// ([`RAT_MAX_SUMMONS`]). Removed
/// corpses have left native `Enemies`, so native slot choice, unique HP and
/// the count synchronization never see them, and the hot roster cannot be
/// checked against one fixed identity table. The retained counts date every
/// death instead: every living rat's call count is the completed call count
/// (held at [`RAT_CALL_COUNT_CAP`]), and a corpse keeps the count it died
/// with. The first two calls are always the first two summons (a call
/// completes without a summon only when all five slots are held,
/// `<CallForBackup>d__41` `0x373d98` IL_00d5-IL_010f, which takes two
/// summons), so an older rat was in `Enemies` at the first or second summon
/// exactly when it is alive or its count is at least that ordinal. From the
/// third summon on the capped count cannot date a corpse that reached 3: it
/// may have died before or after that summon. On that footing this validator
/// checks, with `S` the number of summoned rows:
///
/// * rows are sorted by (native slot index, uid): `SortEnemiesBySlotName`
///   orders `Enemies` by index, and a summon that reuses a corpse's slot is
///   inserted after that older corpse;
/// * uids are distinct; summoned uids exceed the initial three and, when the
///   creation counter is untracked, are exactly 3, 4, 5 in creation order
///   (below the counter when it is tracked);
/// * the k-th summon took the LAST native slot no rat present at it held,
///   and its max HP differs from every rat present at it; every max HP is in
///   band. For the third summon and later, a corpse whose count reached the
///   cap may or may not have been present: it may hold a later slot, and is
///   not asked for a distinct max HP;
/// * every living rat holds one count: `S` while `S < 2`, 2 or 3 when
///   `S == 2` (3 only after a call that found no slot), 3 from there on. No
///   corpse's count is above the living bound, and the k-th summon's corpse
///   holds at least `min(k, 3)`;
/// * UseOnlyOnce records selection rather than execution, so a rat killed
///   after selecting CALL_FOR_BACKUP can make that census exceed `S`; it may
///   never be lower. Any number of living rats may hold the call pending
///   (#3823: before the stun this was at most one).
pub(crate) fn rat_roster_is_valid(state: &HotState) -> bool {
    rat_roster_is_valid_within(state, false)
}

/// [`rat_roster_is_valid`], or with `mid_side` the same roster inside an
/// enemy side, where a rat summoned earlier in the side stands with no
/// intent yet (`rat_spawn_fresh`, #3747). Only a second CALL_FOR_BACKUP in
/// one side reads it that way (#3823, [`spawn_rat`]).
fn rat_roster_is_valid_within(state: &HotState, mid_side: bool) -> bool {
    let rats: Vec<&HotMonster> = state
        .monsters
        .iter()
        .filter(|monster| monster.kind == MonsterKind::TwoTailedRat)
        .collect();
    if rats.len() != state.monsters.len() || !(3..=3 + RAT_MAX_SUMMONS).contains(&rats.len()) {
        return false;
    }
    let Ok(call_count) = u8::try_from(rats.len() - 3) else {
        return false;
    };
    // The highest count any rat of this roster can hold, and the lowest a
    // living one can.
    let count_top = if call_count < 2 {
        call_count
    } else {
        RAT_CALL_COUNT_CAP
    };
    let count_floor = call_count.min(RAT_CALL_COUNT_CAP);
    let mut keys = Vec::with_capacity(rats.len());
    for rat in &rats {
        let Some(native) = rat_native_slot(rat.slot) else {
            return false;
        };
        keys.push((native, rat.uid));
    }
    if keys.windows(2).any(|pair| pair[0] >= pair[1]) {
        return false;
    }
    let mut uids: Vec<u32> = rats.iter().map(|rat| rat.uid).collect();
    uids.sort_unstable();
    uids.dedup();
    if uids.len() != rats.len() || uids[..3] != [0, 1, 2] {
        return false;
    }
    for initial in 0..3_u32 {
        if !rats
            .iter()
            .any(|rat| rat.uid == initial && rat.slot == initial as i32)
        {
            return false;
        }
    }
    let summoned = &uids[3..];
    let uid_identity = match state.fanouts.next_creature_uid() {
        0 => summoned.iter().copied().eq(3..3 + u32::from(call_count)),
        tracked => summoned.iter().all(|uid| *uid < tracked),
    };
    if !uid_identity {
        return false;
    }
    // `TwoTailedRat` `GetValueIfAscension(8, 18..22, 17..21)` (#2539).
    if rats
        .iter()
        .any(|rat| !native_hp_in_band(state, MonsterKind::TwoTailedRat, rat.max_hp))
    {
        return false;
    }
    let initial_hps: Vec<i32> = (0..3_u32)
        .filter_map(|uid| rats.iter().find(|rat| rat.uid == uid))
        .map(|rat| rat.max_hp)
        .collect();
    if initial_hps[0] == initial_hps[1]
        || initial_hps[0] == initial_hps[2]
        || initial_hps[1] == initial_hps[2]
    {
        return false;
    }
    for (ordinal, &uid) in summoned.iter().enumerate() {
        let rat = rats
            .iter()
            .find(|rat| rat.uid == uid)
            .expect("summoned uid is a roster uid");
        let native = rat_native_slot(rat.slot).expect("validated rat slot");
        let Ok(ordinal) = u8::try_from(ordinal + 1) else {
            return false;
        };
        // An older rat was in `Enemies` at this summon when it is alive now
        // or, for the first two summons, its retained count reached this
        // summon's ordinal. From the third on a corpse at the cap is only
        // possibly present.
        let dated = ordinal < RAT_CALL_COUNT_CAP;
        let present: Vec<&&HotMonster> = rats
            .iter()
            .filter(|other| {
                other.uid < uid
                    && (other.hp > 0 || (dated && other.rat_call_for_backup_count() >= ordinal))
            })
            .collect();
        let possibly: Vec<&&HotMonster> = rats
            .iter()
            .filter(|other| {
                other.uid < uid
                    && other.hp <= 0
                    && !dated
                    && other.rat_call_for_backup_count() >= RAT_CALL_COUNT_CAP
            })
            .collect();
        if present
            .iter()
            .any(|other| other.slot == rat.slot || other.max_hp == rat.max_hp)
        {
            return false;
        }
        if ((native + 1)..RAT_SLOTS).any(|later| {
            !present
                .iter()
                .chain(possibly.iter())
                .any(|other| rat_native_slot(other.slot) == Some(later))
        }) {
            return false;
        }
        let count = rat.rat_call_for_backup_count();
        if rat.hp <= 0 && count < ordinal.min(RAT_CALL_COUNT_CAP) {
            return false;
        }
    }

    let mut used_calls = 0_u8;
    let mut living_count = None;
    for &rat in &rats {
        if !rat_ai_state_is_valid(rat, mid_side) {
            return false;
        }
        let machine = rat.random_ai;
        let used = machine.once_len() == 1
            && machine.once_at(0) == Some(super::turn::RAT_CALL_FOR_BACKUP_INDEX);
        if used {
            used_calls += 1;
        }
        let count = rat.rat_call_for_backup_count();
        if rat.hp > 0 && *living_count.get_or_insert(count) != count {
            return false;
        }
        if (rat.hp > 0 && count < count_floor)
            || count > count_top
            || rat.rat_turns_until_summonable() > 2
            || rat.rat_turns_until_summonable() == i32::MIN
            || (rat.rat_spawn_fresh() && !mid_side)
        {
            return false;
        }
    }
    if call_count == 0
        && rats
            .iter()
            .all(|rat| rat.rat_turns_until_summonable() == 2 && rat.random_ai.log_len() == 1)
    {
        let starters: Vec<u8> = rats
            .iter()
            .map(|rat| rat.random_ai.next().expect("validated rat intent"))
            .collect();
        let cyclic_rotations = [
            [
                super::turn::RAT_SCRATCH_INDEX,
                super::turn::RAT_DISEASE_BITE_INDEX,
                super::turn::RAT_SCREECH_INDEX,
            ],
            [
                super::turn::RAT_DISEASE_BITE_INDEX,
                super::turn::RAT_SCREECH_INDEX,
                super::turn::RAT_SCRATCH_INDEX,
            ],
            [
                super::turn::RAT_SCREECH_INDEX,
                super::turn::RAT_SCRATCH_INDEX,
                super::turn::RAT_DISEASE_BITE_INDEX,
            ],
        ];
        if !cyclic_rotations
            .iter()
            .any(|rotation| starters.as_slice() == rotation)
        {
            return false;
        }
    }
    used_calls >= call_count
}

fn require_rat_roster(state: &HotState, owner_uid: u32) -> Result<(), EngineRefusal> {
    if !rat_roster_is_valid_within(state, true)
        || state
            .monsters
            .iter()
            .filter(|monster| monster.uid == owner_uid)
            .count()
            != 1
    {
        return Err(EngineRefusal::MalformedArgs(
            "Two-Tailed Rat roster/AI lifecycle",
        ));
    }
    Ok(())
}

/// TwoTailedRat CALL_FOR_BACKUP (#3146).
///
/// v0.111.0 (`sts2.dll` SHA-256 `9cb4f1ad…12b4`) `<CallForBackup>d__41::
/// MoveNext` (RVA `0x373d98`):
///
/// * `IL_00c1`-`IL_00ce`: nothing happens once `CombatState.IsLiveCombat` is
///   false (`history.over` here).
/// * `IL_00d3`-`IL_00fa`: `nextSlot = Encounter.Slots.LastOrDefault(s =>
///   Enemies.All(e => e.SlotName != s), string.Empty)` (predicate
///   `<>c__DisplayClass41_0::<CallForBackup>b__0` RVA `0x373c70`, inner
///   `<>c__DisplayClass41_1::<CallForBackup>b__4` RVA `0x373cc1`). A dead rat
///   has left `Enemies` ([`in_native_enemies`]), so its slot is free again,
///   and the LAST free native index wins ([`rat_native_slot`]).
/// * `IL_0177`-`IL_0183`: `CreatureCmd.Add<TwoTailedRat>(CombatState,
///   nextSlot)`. `CombatState::CreateCreature` (`0x137074`) rolls unique HP
///   against `_enemies` only ([`roll_unique_hp_native_enemies`]) before
///   stamping the creation uid ([`allocate_creature_uid`]), and
///   `CombatManager::AddCreature` (`0x1360f8` `IL_0046`) re-sorts `Enemies` by
///   slot index, which [`insert_rat_native_ordered`] reproduces (a reused
///   corpse slot sorts the newcomer after the older corpse row).
/// * `IL_01e1`-`IL_025d`: `rats = Enemies.Select(e => e.Monster)
///   .OfType<TwoTailedRat>().ToList()` (`<CallForBackup>b__41_1` `0x373c56`),
///   `max = rats.Max(r => r.CallForBackupCount + 1)` (`b__41_2` `0x373c5e`)
///   and `rats.ForEach(r => r.CallForBackupCount = max)` (`b__3` `0x373cab`).
///   Only living rats (and the newcomer) are synchronized; a corpse keeps the
///   count it died with.
///
/// When `nextSlot` is empty (`IL_010a`-`IL_010f` branches to `IL_01e1`)
/// native skips the Wait and the Add and still runs the synchronization.
/// `TwoTailedRat::CanSummon` (`0xc42c8`, `IL_0022`-`IL_0044`) selects
/// CALL_FOR_BACKUP only while `GetNextSlot` is non-empty and only a summon
/// fills a slot, so with no stun the path is unreachable: no teammate may
/// hold the call pending (`IL_0048`-`IL_009b`). A stunned teammate's parked
/// call is hidden from that test (#3823), so two rats can hold the call
/// together, the first fills the last slot and the second finds none. The
/// counts then rise with no newcomer, as native's do.
///
/// The count is held at [`RAT_CALL_COUNT_CAP`].
///
/// The newcomer (TurnsUntilSummonable 2) does NOT roll its first intent here
/// (#3747). `CombatManager/<AfterCreatureAdded>d__113::MoveNext` (RVA
/// `0x3f2dfc`) awaits `Creature::AfterAddedToRoom` (IL_0017-IL_001c) and
/// then calls `MonsterModel::RollMove` (IL_00cb) only for an enemy
/// (IL_0077-IL_0081) added while `CombatState.CurrentSide == 1`
/// (IL_0084-IL_008f); `CombatSide` is `None` 0, `Player` 1, `Enemy` 2, and
/// CALL_FOR_BACKUP is a monster move, performed on the enemy side. So the rat
/// is published with no intent, and its first `MonsterAi` draw is the one
/// every enemy takes in `CombatManager/<StartTurn>d__100::MoveNext`'s
/// `Enemies` walk (`0x3f781c` IL_036b-IL_03a0 ->
/// `Creature::PrepareForNextTurn` `0x11d85c` IL_002a), at its own slot
/// position: [`super::turn::prepare_random_ai`]. The fresh latch marks that
/// intent-less form until the walk rolls it.
///
/// Rolling at spawn instead is the same draw only while the newcomer sorts
/// ahead of every living rat, which holds for a first summon into native
/// `second`. A newcomer that takes a corpse's later slot rolls AFTER the
/// rats ahead of it (census witness `4SAVTZ7KQ8NW` node 13: the summoner in
/// `fourth` drew first, the newcomer in `fifth` second).
pub(crate) fn spawn_rat(state: &mut HotState, owner_uid: u32) -> Result<(), EngineRefusal> {
    require_rat_roster(state, owner_uid)?;
    let owner = state
        .monsters
        .iter()
        .find(|monster| monster.uid == owner_uid)
        .expect("validated rat owner uid");
    if owner.kind != MonsterKind::TwoTailedRat {
        return Err(EngineRefusal::MalformedArgs("summon_rat owner"));
    }
    if owner.random_ai.next() != Some(super::turn::RAT_CALL_FOR_BACKUP_INDEX)
        || owner.random_ai.once_len() != 1
        || owner.random_ai.once_at(0) != Some(super::turn::RAT_CALL_FOR_BACKUP_INDEX)
    {
        return Err(EngineRefusal::MalformedArgs("summon_rat owner intent"));
    }
    if state.history.over || owner.hp <= 0 {
        return Ok(());
    }
    let mut held = Vec::new();
    for monster in state.monsters.iter() {
        if in_native_enemies(monster)? {
            held.push(
                rat_native_slot(monster.slot).ok_or(EngineRefusal::MalformedArgs(
                    "Two-Tailed Rat roster/AI lifecycle",
                ))?,
            );
        }
    }
    if let Some(native) = (0..RAT_SLOTS).rev().find(|slot| !held.contains(slot)) {
        if state
            .monsters
            .iter()
            .filter(|monster| monster.kind == MonsterKind::TwoTailedRat)
            .count()
            >= 3 + RAT_MAX_SUMMONS
        {
            return Err(EngineRefusal::MalformedArgs(
                "Two-Tailed Rat roster/AI lifecycle",
            ));
        }
        let (low, high) = native_hp_band(state, MonsterKind::TwoTailedRat)?;
        let hp = roll_unique_hp_native_enemies(state, low, high)?;
        let uid = allocate_creature_uid(state)?;
        let mut rat = HotMonster::new(MonsterKind::TwoTailedRat, hp);
        rat.max_hp = hp;
        rat.slot = rat_slot_label(native);
        rat.uid = uid;
        rat.set_rat_turns_until_summonable(2);
        rat.set_rat_spawn_fresh(true);
        insert_rat_native_ordered(state, rat);
        fur_coat_after_opponent_added(state, uid)?;
    }

    let mut top = 0_u8;
    for monster in state.monsters.iter() {
        if monster.kind == MonsterKind::TwoTailedRat && in_native_enemies(monster)? {
            top = top.max(
                monster
                    .rat_call_for_backup_count()
                    .checked_add(1)
                    .ok_or(EngineRefusal::CounterOverflow("rat call-for-backup count"))?,
            );
        }
    }
    let top = top.min(RAT_CALL_COUNT_CAP);
    for monster in state.monsters_mut().iter_mut() {
        if monster.kind == MonsterKind::TwoTailedRat
            && monster.hp > 0
            && !monster.set_rat_call_for_backup_count(top)
        {
            return Err(EngineRefusal::CounterOverflow("rat call-for-backup count"));
        }
    }
    Ok(())
}

/// Insert a rat at its native `Enemies` position: after every row whose
/// native slot index ([`rat_native_slot`]) is not greater, so a newcomer in a
/// corpse's freed slot sorts after that older corpse.
fn insert_rat_native_ordered(state: &mut HotState, rat: HotMonster) {
    let native = rat_native_slot(rat.slot);
    let roster = state.monsters_mut();
    let position = roster
        .iter()
        .position(|candidate| rat_native_slot(candidate.slot) > native)
        .unwrap_or(roster.len());
    roster.insert(position, rat);
}

/// Resolve Ovicopter's parked SUMMON_BRANCH after the complete enemy side-end
/// death/duration walk (`_resolve_ovicopter_after_side_end`, frozen Python, deleted #2827).
pub(crate) fn resolve_ovicopter_after_side_end(state: &mut HotState) -> Result<(), EngineRefusal> {
    if !state
        .monsters
        .iter()
        .any(|monster| matches!(monster.kind, MonsterKind::Ovicopter | MonsterKind::ToughEgg))
    {
        return Ok(());
    }
    if !ovicopter_roster_is_valid(state) {
        return Err(EngineRefusal::MalformedArgs("Ovicopter roster lifecycle"));
    }
    let alive = state
        .monsters
        .iter()
        .filter(|monster| monster.hp > 0)
        .count();
    if let Some(parent) = state
        .monsters_mut()
        .iter_mut()
        .find(|monster| monster.kind == MonsterKind::Ovicopter)
        && parent.hp > 0
        && parent.loop_pos == OVICOPTER_BRANCH_POS
    {
        parent.loop_pos = if alive <= 3 { 0 } else { 3 };
    }
    Ok(())
}

/// `BattlewornDummyTimeLimitPower`'s amount on every Battle Friend's entry
/// (#3357). `BattleFriendV1`/`V2`/`V3` `<AfterAddedToRoom>d__6::MoveNext`
/// (RVA `0x3536cc` / `0x3537c0` / `0x3538b4`) each load `ldc.i4.3` at IL_0028
/// and pass it to `PowerCmd.Apply<BattlewornDummyTimeLimitPower>` at IL_0031,
/// the owner's own `Creature` as target and applier.
pub(crate) const BATTLEWORN_TIME_LIMIT: i32 = 3;

/// The three Battleworn Dummy event monsters.
pub(crate) fn is_battle_friend(kind: MonsterKind) -> bool {
    matches!(
        kind,
        MonsterKind::BattleFriendV1 | MonsterKind::BattleFriendV2 | MonsterKind::BattleFriendV3
    )
}

/// Whether a roster holding a Battle Friend (or its timer) is the shape the
/// timer's lifecycle is modeled for (#3357).
///
/// `BattlewornDummyEventV{1,2,3}Encounter::GenerateMonsters` (RVA `0xd2f85` /
/// `0xd2fb6` / `0xd2fe7`) build exactly one Battle Friend, and nothing in the
/// build summons beside it. `BattleFriendV*::GenerateMoveStateMachine` (V2
/// RVA `0xaf45c`) is a lone `NOTHING_MOVE` whose body is
/// `Task.CompletedTask` (V2 `<>c::<GenerateMoveStateMachine>b__5_0` RVA
/// `0x3537b6`) and which is its own `FollowUpState`, so the dummy never
/// writes anything. Its only listener is the intrinsic
/// `BattlewornDummyTimeLimitPower`, applied at [`BATTLEWORN_TIME_LIMIT`] and
/// only ever decremented by one (see
/// [`battleworn_time_limit_after_enemy_side_end`]) until its escape removes
/// the owner, so a live timer is an `Int` in `1..=3`, and it lives only on a
/// Battle Friend. A roster with neither is vacuously valid.
pub(crate) fn battleworn_roster_is_valid(state: &HotState) -> bool {
    let carries_timer = |monster: &HotMonster| {
        monster.powers.value(PowerId::BattlewornTimeLimit) != 0
            || monster
                .powers
                .as_slice()
                .iter()
                .any(|slot| slot.key == PowerId::BattlewornTimeLimit)
    };
    if !state
        .monsters
        .iter()
        .any(|monster| is_battle_friend(monster.kind) || carries_timer(monster))
    {
        return true;
    }
    let [dummy] = state.monsters.as_slice() else {
        return false;
    };
    is_battle_friend(dummy.kind)
        && dummy.hp > 0
        && dummy.powers.as_slice().iter().any(|slot| {
            slot.key == PowerId::BattlewornTimeLimit
                && slot.wire == SlotWire::Int
                && (1..=BATTLEWORN_TIME_LIMIT).contains(&slot.value)
        })
}

/// The scoring split a Battleworn Dummy event fight needs between a kill and
/// a timeout (#3369, Sean 2026-09-27: the solver prefers the kill).
///
/// Both terminals are won combats to the engine. At expiry
/// [`battleworn_time_limit_after_enemy_side_end`] escapes the dummy and
/// `CombatManager/<CheckWinCondition>d__125::MoveNext` RVA `0x3f3644`
/// IL_004a-IL_0058 takes the victory path. Only the event tells them apart:
/// `BattlewornDummy/<Resume>d__11::MoveNext` RVA `0x379528` IL_0039 reads
/// `RanOutOfTime`, which the timer sets just before its escape
/// (`<AfterSideTurnEnd>d__4` RVA `0x3356f4` IL_00a5-IL_00c0), and shows the
/// DEFEAT page instead of paying the reward. So a timeout forfeits the
/// reward and a kill keeps it.
///
/// The engine already holds that latch as its roster. A kill leaves the dead
/// dummy in `monsters`. The escape removes it, and the validated lone-dummy
/// roster ([`battleworn_roster_is_valid`]) is then empty. The empty roster
/// alone does not name the fight: a lone Thieving Hopper's escape empties
/// one too. So the fight is read once from the root, where the living
/// dummy stands, and each terminal is then read from its roster. No state
/// field is added.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BattlewornObjective {
    battleworn: bool,
}

impl BattlewornObjective {
    /// Read the fight from a live root or any live pre-terminal state. The
    /// dummy is on the roster there, because only the timer or a kill ends
    /// its fight.
    #[must_use]
    pub fn of_root(root: &HotState) -> Self {
        Self {
            battleworn: root
                .monsters
                .iter()
                .any(|monster| is_battle_friend(monster.kind)),
        }
    }

    /// Whether this is a Battleworn Dummy event fight.
    #[must_use]
    pub fn is_battleworn(self) -> bool {
        self.battleworn
    }

    /// Whether `terminal` is a won combat whose dummy escaped on its timer:
    /// the player survived and the roster no longer holds the dummy, so the
    /// event pays nothing. False for a kill, a loss, a live state, and every
    /// fight that is not a Battleworn Dummy fight.
    #[must_use]
    pub fn timed_out(self, terminal: &HotState) -> bool {
        self.battleworn
            && terminal.history.over
            && terminal.hp > 0
            && !terminal
                .monsters
                .iter()
                .any(|monster| is_battle_friend(monster.kind))
    }
}

/// `BattlewornDummyTimeLimitPower.AfterSideTurnEnd` on the enemy side's end
/// (#3357). Returns whether the timer ended the combat.
///
/// v0.111.0 `BattlewornDummyTimeLimitPower` (`get_Type` RVA `0x9fb39` = 1,
/// a buff; `get_StackType` RVA `0x9fb3c` = 1, a counter) overrides only
/// `AfterSideTurnEnd` (RVA `0x9fb40`). Its body,
/// `<AfterSideTurnEnd>d__4::MoveNext` RVA `0x3356f4`:
///
/// * IL_0024-IL_0037 — `leave` unless `participants.Contains(Owner)`: the
///   dummy's own (enemy) side only.
/// * IL_003c-IL_0043 — `Amount > 1`: `PowerCmd.Decrement(this)` (IL_0046),
///   which is `<Decrement>d__4::MoveNext` RVA `0x3f025c` IL_0016-IL_0029,
///   `ModifyAmount(power, -1, applier null, cardSource null, silent false)`
///   — the same null-applier amount change Slumber's decrement takes, so the
///   represented `AfterPowerAmountChanged` walk is authenticated before the
///   write.
/// * IL_00a5-IL_00c0 — otherwise, when the encounter is a
///   `BattlewornDummyEventEncounter`, `set_RanOutOfTime(true)` (RVA
///   `0xd2f18`), then IL_00c5-IL_00cc `CreatureCmd.Escape(Owner, true)`.
///
/// `CreatureCmd::Escape` RVA `0x132750` is synchronous: it returns on a dead
/// or non-live owner (IL_000c-IL_0034), then `RemoveAllPowersInternalExcept`
/// (IL_0036, no hooks) and `CombatManager.RemoveCreature` (IL_0075) — the
/// removal without Before/AfterDeath that Fat Gremlin's FLEE takes
/// ([`crate::moves::normal::escape`]). With the lone dummy gone,
/// `CombatManager/<CheckWinCondition>d__125::MoveNext` RVA `0x3f3644`
/// IL_004a-IL_0058 takes `EndCombatInternal`, the victory path (a loss is
/// `ProcessPendingLoss`, IL_0037): the combat is won, and the event's
/// `<Resume>d__11::MoveNext` RVA `0x379528` IL_0039 reads `RanOutOfTime` to
/// show its DEFEAT page instead of paying the reward. That event-level latch
/// is exactly "the terminal roster is empty rather than holding a dead
/// dummy", so it needs no field of its own here.
///
/// The timer is intrinsic — the first power the owner acquires — so it is
/// the first of the owner's `AfterSideTurnEnd` listeners, ahead of every
/// player-applied power (the capture's `BATTLEWORN_DUMMY_TIME_LIMIT_POWER`
/// row always leads the dummy's power list). Doom has already run at
/// `BeforeSideTurnEnd`, so a Doom kill still wins before the timer is read.
pub(crate) fn battleworn_time_limit_after_enemy_side_end(
    state: &mut HotState,
    index: usize,
    events: &mut Vec<Event>,
) -> Result<bool, EngineRefusal> {
    let Some(monster) = state.monsters.get(index) else {
        return Ok(false);
    };
    // Admission confines the timer to a Battle Friend (its power gate) and
    // nothing in the build applies it later, so the kind is the whole test
    // on this every-turn path.
    if !is_battle_friend(monster.kind) {
        return Ok(false);
    }
    if !battleworn_roster_is_valid(state) {
        return Err(EngineRefusal::MalformedArgs(
            "Battleworn Dummy timer lifecycle",
        ));
    }
    let amount = state.monsters[index]
        .powers
        .value(PowerId::BattlewornTimeLimit);
    if amount > 1 {
        super::damage::null_applier_power_amount_changed_is_exact(state)?;
        let monster = &mut state.monsters_mut()[index];
        monster
            .powers
            .set(PowerId::BattlewornTimeLimit, SlotWire::Int, amount - 1);
        super::damage::note_power(
            events,
            Subject::Monster(monster.uid),
            PowerId::BattlewornTimeLimit,
            amount - 1,
        );
        return Ok(false);
    }
    // The validated roster is the lone dummy, so its removal empties it.
    state.monsters_mut().remove(index);
    state.history.over = true;
    events.push(Event::CombatOver { player_won: true });
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #3856: `note_hopper_transformed`, one arm at a time. Only a uid that
    /// owns a master row is recorded, once, in ascending order; a generated
    /// card and a fight with no Hopper deck record nothing.
    #[test]
    fn note_hopper_transformed_records_only_master_rows_in_ascending_order() {
        let row = |uid| HopperDeckRow {
            card: crate::hot::HotCard {
                uid,
                atom: crate::catalog::CardAtom::default(),
                flags: 0,
            },
            state: crate::hot::CardInstanceState::default(),
        };
        let mut state = HotState::at_defaults();
        note_hopper_transformed(&mut state, 3);
        assert!(state.card_states.hopper().is_none());

        state
            .card_states
            .set_hopper(Some(crate::hot::HopperDeckState {
                master: vec![row(1), row(3), row(5)],
                history: Vec::new(),
                transformed: Vec::new(),
            }));
        note_hopper_transformed(&mut state, 40);
        assert!(state.card_states.hopper().unwrap().transformed.is_empty());
        for uid in [5, 1, 5, 40, 1] {
            note_hopper_transformed(&mut state, uid);
        }
        assert_eq!(state.card_states.hopper().unwrap().transformed, [1, 5]);
        assert_eq!(state.card_states.hopper().unwrap().master.len(), 3);
    }

    fn lone_battle_friend(kind: MonsterKind, timer: i32) -> HotState {
        let mut state = HotState::at_defaults();
        let mut dummy = HotMonster::new(kind, 150);
        dummy.max_hp = 150;
        dummy
            .powers
            .set(PowerId::BattlewornTimeLimit, SlotWire::Int, timer);
        state.monsters_mut().push(dummy);
        state
    }

    /// #3369: the kill/timeout split is read from the roster, scoped to a
    /// root that holds a Battle Friend. A lone Hopper's escape also empties
    /// its roster and stays a plain win; a loss or a live state is never a
    /// timeout.
    #[test]
    fn battleworn_objective_splits_a_kill_from_a_timeout_only_in_its_own_fight() {
        for kind in [
            MonsterKind::BattleFriendV1,
            MonsterKind::BattleFriendV2,
            MonsterKind::BattleFriendV3,
        ] {
            let mut root = lone_battle_friend(kind, BATTLEWORN_TIME_LIMIT);
            root.hp = 50;
            let objective = BattlewornObjective::of_root(&root);
            assert!(objective.is_battleworn());
            assert!(!objective.timed_out(&root), "live");

            let mut killed = root.clone();
            killed.monsters_mut()[0].hp = 0;
            killed.history.over = true;
            assert!(!objective.timed_out(&killed), "a dead dummy is a kill");

            let mut escaped = root.clone();
            escaped.monsters_mut().clear();
            escaped.history.over = true;
            assert!(
                objective.timed_out(&escaped),
                "an empty roster is the escape"
            );

            let mut lost = escaped.clone();
            lost.hp = 0;
            assert!(!objective.timed_out(&lost), "a loss is no timeout");

            let mut not_over = escaped.clone();
            not_over.history.over = false;
            assert!(!objective.timed_out(&not_over), "only a terminal times out");
        }

        let mut hopper_root = HotState::at_defaults();
        hopper_root.hp = 50;
        hopper_root
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::ThievingHopper, 80));
        let plain = BattlewornObjective::of_root(&hopper_root);
        assert!(!plain.is_battleworn());
        assert_eq!(plain, BattlewornObjective::default());
        let mut fled = hopper_root.clone();
        fled.monsters_mut().clear();
        fled.history.over = true;
        assert!(
            !plain.timed_out(&fled),
            "another fight's escape is unchanged"
        );
    }

    /// #3357: the timer's lifecycle is modeled for exactly the lone Battle
    /// Friend carrying a live `Int` timer in `1..=3`; each other shape fails.
    #[test]
    fn battleworn_roster_is_the_lone_battle_friend_with_a_live_timer() {
        assert!(battleworn_roster_is_valid(&HotState::at_defaults()));
        for kind in [
            MonsterKind::BattleFriendV1,
            MonsterKind::BattleFriendV2,
            MonsterKind::BattleFriendV3,
        ] {
            for timer in 1..=BATTLEWORN_TIME_LIMIT {
                assert!(battleworn_roster_is_valid(&lone_battle_friend(kind, timer)));
            }
            assert!(!battleworn_roster_is_valid(&lone_battle_friend(kind, 0)));
            assert!(!battleworn_roster_is_valid(&lone_battle_friend(
                kind,
                BATTLEWORN_TIME_LIMIT + 1
            )));
        }
        let mut bool_wire = lone_battle_friend(MonsterKind::BattleFriendV2, 1);
        bool_wire.monsters_mut()[0]
            .powers
            .set(PowerId::BattlewornTimeLimit, SlotWire::Bool, 1);
        assert!(!battleworn_roster_is_valid(&bool_wire));

        let mut dead = lone_battle_friend(MonsterKind::BattleFriendV2, 2);
        dead.monsters_mut()[0].hp = 0;
        assert!(!battleworn_roster_is_valid(&dead));

        let mut pair = lone_battle_friend(MonsterKind::BattleFriendV2, 2);
        let mut second = HotMonster::new(MonsterKind::Toadpole, 20);
        second.uid = 1;
        second.slot = 1;
        pair.monsters_mut().push(second);
        assert!(!battleworn_roster_is_valid(&pair));

        let foreign = lone_battle_friend(MonsterKind::Toadpole, 2);
        assert!(!battleworn_roster_is_valid(&foreign));
    }

    /// #3366: `get_PlowAmount` RVA `0xb0c33` is `GetValueIfAscension(9, 160,
    /// 150)`, and the Plow owner predicates read that tier, not a fixed 160.
    #[test]
    fn ceremonial_beast_plow_threshold_is_the_ascension_nine_tier() {
        for (ascension, max_hp, threshold) in
            [(0, 252, 150), (8, 262, 150), (9, 262, 160), (10, 262, 160)]
        {
            let mut state = HotState::at_defaults();
            assert!(state.fanouts.set_ascension(ascension));
            assert_eq!(ceremonial_beast_plow_threshold(&state), threshold);
            let mut beast = HotMonster::new(MonsterKind::CeremonialBeast, threshold + 1);
            beast.max_hp = max_hp;
            beast.loop_pos = 1;
            beast
                .powers
                .set(PowerId::PlowThreshold, SlotWire::Int, threshold);
            state.monsters_mut().push(beast);
            assert!(ceremonial_beast_state_is_valid(&state), "A{ascension}");
            assert!(beast_plow_damage_owner_is_valid(&state, 0), "A{ascension}");
            // #3823: Plow at or below its threshold is the STAMP that landed
            // on a Beast already that low (native trace `bst_h1low`).
            state.monsters_mut()[0].hp = threshold;
            assert!(ceremonial_beast_state_is_valid(&state), "A{ascension}");
            let other = 310 - threshold;
            state.monsters_mut()[0].hp = 200;
            state.monsters_mut()[0]
                .powers
                .set(PowerId::PlowThreshold, SlotWire::Int, other);
            assert!(!ceremonial_beast_state_is_valid(&state), "A{ascension}");
            assert!(!beast_plow_damage_owner_is_valid(&state, 0), "A{ascension}");
        }
    }

    fn lone_globe_head(ascension: u8) -> HotState {
        let mut state = HotState::at_defaults();
        assert!(state.fanouts.set_ascension(ascension));
        let mut head = HotMonster::new(MonsterKind::GlobeHead, 148);
        head.max_hp = 148;
        state.monsters_mut().push(head);
        state
    }

    /// #3300: `get_GalvanicPowerAmount` RVA `0xb5ce6` is
    /// `GetValueIfAscension(9, 8, 6)`.
    #[test]
    fn globe_head_galvanic_amount_is_the_ascension_nine_tier() {
        assert_eq!(globe_head_galvanic_amount(&lone_globe_head(0)), Some(6));
        assert_eq!(globe_head_galvanic_amount(&lone_globe_head(8)), Some(6));
        assert_eq!(globe_head_galvanic_amount(&lone_globe_head(9)), Some(8));
        assert_eq!(globe_head_galvanic_amount(&lone_globe_head(10)), Some(8));
    }

    /// #3300: every conjunct of the Power-is-Galvanized proof is load-bearing.
    #[test]
    fn globe_head_galvanic_proof_refuses_each_foreign_affliction_state() {
        assert!(globe_head_galvanic_state_is_exact(&lone_globe_head(10)));

        let mut pair = lone_globe_head(10);
        let mut second = HotMonster::new(MonsterKind::Toadpole, 20);
        second.uid = 1;
        second.slot = 1;
        pair.monsters_mut().push(second);
        assert!(!globe_head_galvanic_state_is_exact(&pair));

        let mut other = HotState::at_defaults();
        other
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::Toadpole, 20));
        assert!(!globe_head_galvanic_state_is_exact(&other));

        for power in [
            PowerId::HexPower,
            PowerId::ChainsOfBinding,
            PowerId::Smoggy,
            PowerId::SmoggyFresh,
            PowerId::SmogLock,
            PowerId::Tangled,
        ] {
            let mut state = lone_globe_head(10);
            state.powers.set(power, SlotWire::Int, 1);
            assert!(!globe_head_galvanic_state_is_exact(&state), "{power:?}");
        }

        let mut ringing = lone_globe_head(10);
        ringing.set_ringing(true);
        assert!(!globe_head_galvanic_state_is_exact(&ringing));

        let mut bound = lone_globe_head(10);
        assert!(bound.set_bound_afflictions_this_turn(1));
        assert!(!globe_head_galvanic_state_is_exact(&bound));

        for flag in [CARD_FLAG_BOUND, CARD_FLAG_HEXED, CARD_FLAG_RINGING] {
            let mut state = lone_globe_head(10);
            state
                .piles
                .get_mut(PileId::Discard)
                .make_mut()
                .push(crate::hot::HotCard {
                    uid: 0,
                    atom: 0,
                    flags: CARD_FLAG_DEFAULT_PHYSICAL_STATE | flag,
                });
            assert!(!globe_head_galvanic_state_is_exact(&state), "{flag:#x}");
        }
    }

    /// A Fabricator (slot 2, uid 0) after one aggro spawn of Zapbot, plus the
    /// given bots as `(kind, slot, uid, hp, max_hp)`.
    fn fabricator_roster(bots: &[(MonsterKind, i32, u32, i32, i32)]) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 200;
        state.max_hp = 200;
        let mut owner = HotMonster::new(MonsterKind::Fabricator, 155);
        owner.max_hp = 155;
        owner.slot = 2;
        owner.last_spawned = Some(MonsterKind::Zapbot);
        assert!(owner.random_ai.set_next(Some(FABRICATOR_STRIKE_INDEX)));
        assert!(
            owner
                .random_ai
                .set_log(&[FABRICATOR_STRIKE_INDEX, FABRICATOR_STRIKE_INDEX])
        );
        state.monsters_mut().push(owner);
        for &(kind, slot, uid, hp, max_hp) in bots {
            let mut bot = HotMonster::new(kind, hp);
            bot.max_hp = max_hp;
            bot.slot = slot;
            bot.uid = uid;
            bot.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
            if kind == MonsterKind::Zapbot && hp > 0 {
                bot.powers.set(PowerId::HighVoltage, SlotWire::Int, 2);
            }
            insert_slot_ordered(&mut state, bot);
        }
        assert!(fabricator_state_is_valid(&state));
        state
    }

    fn roster(state: &HotState) -> Vec<(MonsterKind, i32, u32, bool)> {
        state
            .monsters
            .iter()
            .map(|monster| (monster.kind, monster.slot, monster.uid, monster.hp > 0))
            .collect()
    }

    /// #3123 (census witness `f08d5c27ac8e5859`, step 17): the bot in slot
    /// `bot1` dies and has left native `Enemies`, so `GetNextSlot` hands the
    /// next spawn that slot; the new bot sorts ahead of the live Zapbot while
    /// its uid is still the next creation ordinal.
    #[test]
    fn fabricator_spawn_reuses_a_dead_bots_slot() {
        let mut state = fabricator_roster(&[
            (MonsterKind::Noisebot, 0, 1, 0, 23),
            (MonsterKind::Zapbot, 1, 2, 20, 20),
        ]);
        spawn_fabricator_bot(&mut state, 0, &FABRICATOR_AGGRO_BOTS).unwrap();
        assert_eq!(
            roster(&state),
            [
                (MonsterKind::Noisebot, 0, 1, false),
                (MonsterKind::Stabbot, 0, 3, true),
                (MonsterKind::Zapbot, 1, 2, true),
                (MonsterKind::Fabricator, 2, 0, true),
            ]
        );

        // A dead bot in a later slot does not pull the spawn past an earlier
        // never-used one: first free slot in `get_Slots` order wins.
        let mut state = fabricator_roster(&[
            (MonsterKind::Guardbot, 0, 1, 12, 21),
            (MonsterKind::Noisebot, 3, 2, 0, 23),
        ]);
        spawn_fabricator_bot(&mut state, 0, &FABRICATOR_AGGRO_BOTS).unwrap();
        assert_eq!(
            roster(&state),
            [
                (MonsterKind::Guardbot, 0, 1, true),
                (MonsterKind::Stabbot, 1, 3, true),
                (MonsterKind::Fabricator, 2, 0, true),
                (MonsterKind::Noisebot, 3, 2, false),
            ]
        );
    }

    /// With no dead bot, the spawn takes the first never-held slot; with
    /// every slot held by a living creature, `GetNextSlot` falls back to
    /// `string.Empty`, which sorts first (`-1`).
    #[test]
    fn fabricator_spawn_without_a_free_dead_slot() {
        let mut state = fabricator_roster(&[
            (MonsterKind::Guardbot, 0, 1, 21, 21),
            (MonsterKind::Zapbot, 1, 2, 20, 20),
        ]);
        spawn_fabricator_bot(&mut state, 0, &FABRICATOR_AGGRO_BOTS).unwrap();
        assert_eq!(
            roster(&state),
            [
                (MonsterKind::Guardbot, 0, 1, true),
                (MonsterKind::Zapbot, 1, 2, true),
                (MonsterKind::Fabricator, 2, 0, true),
                (MonsterKind::Stabbot, 3, 3, true),
            ]
        );

        let mut full = fabricator_roster(&[
            (MonsterKind::Guardbot, 0, 1, 21, 21),
            (MonsterKind::Zapbot, 1, 2, 20, 20),
            (MonsterKind::Noisebot, 3, 3, 23, 23),
            (MonsterKind::Guardbot, 4, 4, 20, 20),
        ]);
        spawn_fabricator_bot(&mut full, 0, &FABRICATOR_AGGRO_BOTS).unwrap();
        assert_eq!(roster(&full)[0], (MonsterKind::Stabbot, -1, 5, true));
    }

    /// `SetUniqueMonsterHpValue` excludes only the max HP of creatures still
    /// in native `Enemies`: a corpse's max HP is free again, a live one's is
    /// not. Candidates here are {10, 11}; the live 11 leaves exactly {10}.
    /// Counting the corpse too would empty the set and fall back to the full
    /// range.
    #[test]
    fn unique_hp_ignores_removed_corpses() {
        let mut state = HotState::at_defaults();
        let mut corpse = HotMonster::new(MonsterKind::Noisebot, 0);
        corpse.max_hp = 10;
        let mut live = HotMonster::new(MonsterKind::Zapbot, 11);
        live.max_hp = 11;
        live.uid = 1;
        live.slot = 1;
        state.monsters_mut().push(corpse);
        state.monsters_mut().push(live);
        for _ in 0..8 {
            assert_eq!(roll_unique_hp_native_enemies(&mut state, 10, 11), Ok(10));
        }
    }

    /// The two represented keep-on-death vetoes leave a corpse in native
    /// `Enemies`; the spawn seam refuses rather than guess its slot.
    #[test]
    fn spawn_seam_refuses_a_corpse_with_a_death_veto() {
        for power in [PowerId::Adaptable, PowerId::PainfulStabs] {
            let mut state = HotState::at_defaults();
            let mut corpse = HotMonster::new(MonsterKind::Noisebot, 0);
            corpse.powers.set(power, SlotWire::Int, 1);
            state.monsters_mut().push(corpse);
            assert_eq!(
                native_enemy_slots(&state),
                Err(EngineRefusal::MalformedArgs(
                    "spawn seam: corpse kept in Enemies by a death veto"
                ))
            );
            assert!(roll_unique_hp_native_enemies(&mut state, 1, 2).is_err());
        }
    }

    /// Living Fog's BLOAT also takes `GetNextSlot`: an exploded bomb frees
    /// `bomb1` for the next one.
    #[test]
    fn gas_bomb_reuses_a_dead_bombs_slot() {
        let mut state = HotState::at_defaults();
        let mut fog = HotMonster::new(MonsterKind::LivingFog, 80);
        fog.max_hp = 80;
        fog.slot = 5;
        let mut dead = HotMonster::new(MonsterKind::GasBomb, 0);
        dead.max_hp = 8;
        dead.slot = 0;
        dead.uid = 1;
        let mut live = HotMonster::new(MonsterKind::GasBomb, 8);
        live.max_hp = 8;
        live.slot = 1;
        live.uid = 2;
        state.monsters_mut().push(dead);
        state.monsters_mut().push(live);
        state.monsters_mut().push(fog);
        spawn_gas_bomb(&mut state, 0).unwrap();
        let spawned = state
            .monsters
            .iter()
            .find(|monster| monster.uid == 3)
            .unwrap();
        assert_eq!((spawned.kind, spawned.slot), (MonsterKind::GasBomb, 0));
        assert_eq!(state.monsters[1].uid, 3);
    }

    #[test]
    fn aeonglass_constructor_consumes_one_niche_draw_and_orders_startup_state() {
        let mut builder = crate::catalog::CatalogBuilder::new();
        builder.intern_monster(MonsterKind::Aeonglass).unwrap();
        builder
            .intern_reachable(CardIdentity {
                id: CardId::Wither,
                upgrade: 0,
                enchantment: None,
            })
            .unwrap();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.max_hp = 70;
        state.exact_piles = true;
        state.rng.set(
            RngStream::Niche,
            RngStreamState {
                words: [11, 22, 33, 44],
                counter: 7,
            },
        );
        let before = state.rng.get(RngStream::Niche);
        let mut events = Vec::new();
        construct_aeonglass_boss(&mut state, &catalog, &mut events).unwrap();
        assert_eq!(state.rng.get(RngStream::Niche).counter, before.counter + 1);
        assert_eq!(state.monsters[0].hp, AEONGLASS_HP);
        assert_eq!(state.fanouts.withering_cards_left(), 6);
        assert_eq!(state.monsters[0].powers.value(PowerId::Artifact), 3);
        assert!(matches!(
            events.as_slice(),
            [Event::PowerChanged {
                subject: Subject::Monster(0),
                power: PowerId::Artifact,
                amount: 3,
            }]
        ));

        let mut malformed = HotState::at_defaults();
        malformed.hp = 70;
        malformed.max_hp = 70;
        let state_before = malformed.clone();
        let mut retained = vec![Event::TurnBegan { turn: 48 }];
        let retained_before = retained.clone();
        assert!(construct_aeonglass_boss(&mut malformed, &catalog, &mut retained).is_err());
        assert_eq!(malformed, state_before);
        assert_eq!(retained, retained_before);

        let mut bad_niche = HotState::at_defaults();
        bad_niche.hp = 70;
        bad_niche.max_hp = 70;
        bad_niche.exact_piles = true;
        let niche_before = bad_niche.clone();
        let mut retained = vec![Event::TurnBegan { turn: 48 }];
        let retained_before = retained.clone();
        assert!(construct_aeonglass_boss(&mut bad_niche, &catalog, &mut retained).is_err());
        assert_eq!(bad_niche, niche_before);
        assert_eq!(retained, retained_before);
    }

    fn test_subject_state(form: u8, reviving: bool) -> HotState {
        let mut state = HotState::at_defaults();
        let (hp, loop_pos, max_hp, adaptable, painful, nemesis, enrage) = match (form, reviving) {
            (0, false) => (TEST_SUBJECT_FIRST_HP, 1, TEST_SUBJECT_FIRST_HP, 1, 0, 0, 3),
            (0, true) => (0, 0, TEST_SUBJECT_FIRST_HP, 1, 0, 0, 0),
            (1, false) => (
                TEST_SUBJECT_SECOND_HP,
                3,
                TEST_SUBJECT_SECOND_HP,
                1,
                1,
                0,
                0,
            ),
            (1, true) => (0, 0, TEST_SUBJECT_SECOND_HP, 1, 1, 0, 0),
            (2, false) => (TEST_SUBJECT_THIRD_HP, 4, TEST_SUBJECT_THIRD_HP, 0, 0, 1, 0),
            _ => unreachable!(),
        };
        let mut owner = HotMonster::new(MonsterKind::TestSubject, hp);
        owner.max_hp = max_hp;
        owner.loop_pos = loop_pos;
        assert!(owner.set_test_subject_respawns(form));
        owner.set_test_subject_adaptable_reviving(reviving);
        owner
            .powers
            .set(PowerId::Adaptable, SlotWire::Int, adaptable);
        owner
            .powers
            .set(PowerId::PainfulStabs, SlotWire::Int, painful);
        owner.powers.set(PowerId::Nemesis, SlotWire::Int, nemesis);
        owner.powers.set(PowerId::Enrage, SlotWire::Int, enrage);
        state.monsters_mut().push(owner);
        state
    }

    fn rat_state() -> HotState {
        let mut state = HotState::at_defaults();
        for (position, intent) in [
            super::super::turn::RAT_CALL_FOR_BACKUP_INDEX,
            super::super::turn::RAT_DISEASE_BITE_INDEX,
            super::super::turn::RAT_SCREECH_INDEX,
        ]
        .into_iter()
        .enumerate()
        {
            let mut rat = HotMonster::new(MonsterKind::TwoTailedRat, 18 + position as i32);
            rat.max_hp = rat.hp;
            rat.slot = position as i32;
            rat.uid = position as u32;
            rat.set_rat_turns_until_summonable(if position == 0 { 0 } else { 2 });
            assert!(rat.random_ai.set_next(Some(intent)));
            if position == 0 {
                assert!(rat.random_ai.set_log(&[
                    super::super::turn::RAT_SCRATCH_INDEX,
                    super::super::turn::RAT_DISEASE_BITE_INDEX,
                    intent,
                ]));
                assert!(
                    rat.random_ai
                        .set_once(&[super::super::turn::RAT_CALL_FOR_BACKUP_INDEX])
                );
            } else {
                assert!(rat.random_ai.set_log(&[intent]));
            }
            state.monsters_mut().push(rat);
        }
        state
    }

    fn fabricator_state() -> HotState {
        let mut state = HotState::at_defaults();
        let mut owner = HotMonster::new(MonsterKind::Fabricator, 155);
        owner.max_hp = 155;
        owner.slot = 2;
        assert!(owner.random_ai.set_next(Some(FABRICATOR_FABRICATE_INDEX)));
        assert!(owner.random_ai.set_log(&[FABRICATOR_FABRICATE_INDEX]));
        state.monsters_mut().push(owner);
        state
    }

    #[test]
    fn test_subject_form_and_retained_corpse_census_is_exact() {
        for (form, reviving) in [(0, false), (0, true), (1, false), (1, true), (2, false)] {
            let state = test_subject_state(form, reviving);
            assert!(test_subject_state_is_valid(&state), "{form}/{reviving}");
        }

        let mut third = test_subject_state(2, false);
        third.monsters_mut()[0].set_test_subject_nemesis_apply_intangible(true);
        third.monsters_mut()[0]
            .powers
            .set(PowerId::Intangible, SlotWire::Int, 1);
        assert!(test_subject_state_is_valid(&third));

        for loop_pos in [1, 2] {
            let mut first = test_subject_state(0, false);
            first.monsters_mut()[0].loop_pos = loop_pos;
            assert!(test_subject_state_is_valid(&first));
        }
        for loop_pos in [4, 5, 6] {
            let mut third = test_subject_state(2, false);
            third.monsters_mut()[0].loop_pos = loop_pos;
            assert!(test_subject_state_is_valid(&third));
        }
    }

    #[test]
    fn test_subject_private_state_mutations_fail_closed() {
        let baseline = test_subject_state(1, false);
        let mutations: &[fn(&mut HotState)] = &[
            |state| state.monsters_mut()[0].uid = 1,
            |state| state.monsters_mut()[0].max_hp = TEST_SUBJECT_SECOND_HP - 1,
            |state| state.monsters_mut()[0].loop_pos = 2,
            |state| state.monsters_mut()[0].spawn_noop = true,
            |state| state.monsters_mut()[0].curl_up_card_uid = 7,
            |state| state.monsters_mut()[0].ritual_fresh = true,
            |state| state.monsters_mut()[0].revive_stage |= 0b0001_0000,
            |state| {
                state.monsters_mut()[0]
                    .powers
                    .set(PowerId::PainfulStabs, SlotWire::Int, 0)
            },
            |state| {
                state.monsters_mut()[0]
                    .powers
                    .set(PowerId::Nemesis, SlotWire::Int, 1)
            },
            |state| state.monsters_mut()[0].set_test_subject_adaptable_reviving(true),
            |state| state.monsters_mut()[0].set_test_subject_nemesis_apply_intangible(true),
        ];
        for mutate in mutations {
            let mut state = baseline.clone();
            mutate(&mut state);
            assert!(!test_subject_state_is_valid(&state));
        }

        let mut wrong_owner = baseline.clone();
        wrong_owner.monsters_mut()[0].kind = MonsterKind::CalcifiedCultist;
        assert!(!test_subject_state_is_valid(&wrong_owner));

        let mut teammate = baseline;
        teammate
            .monsters_mut()
            .push(HotMonster::new(MonsterKind::CalcifiedCultist, 1));
        assert!(!test_subject_state_is_valid(&teammate));
    }

    #[test]
    fn adaptable_death_latch_is_atomic_and_preserves_retained_form_state() {
        for form in [0, 1] {
            let mut state = test_subject_state(form, false);
            let owner = &mut state.monsters_mut()[0];
            owner.hp = 0;
            owner.powers.set(PowerId::Enrage, SlotWire::Int, 0);
            latch_test_subject_after_death(&mut state, 0).unwrap();
            assert_eq!(state.monsters[0].loop_pos, 0);
            assert!(state.monsters[0].test_subject_adaptable_reviving());
            assert_eq!(state.monsters[0].test_subject_respawns(), form);
            assert!(test_subject_state_is_valid(&state));
        }

        let mut malformed = test_subject_state(0, false);
        malformed.monsters_mut()[0].hp = 0;
        malformed.monsters_mut()[0]
            .powers
            .set(PowerId::Enrage, SlotWire::Int, 0);
        malformed.monsters_mut()[0].uid = 7;
        let before = malformed.clone();
        assert!(latch_test_subject_after_death(&mut malformed, 0).is_err());
        assert_eq!(malformed, before);
    }

    #[test]
    fn nemesis_side_end_alternates_one_and_zero_without_touching_form() {
        let mut state = test_subject_state(2, false);
        assert_eq!(test_subject_nemesis_side_end(&mut state, 0).unwrap(), 1);
        assert!(state.monsters[0].test_subject_nemesis_apply_intangible());
        assert_eq!(state.monsters[0].powers.value(PowerId::Intangible), 1);
        assert_eq!(state.monsters[0].test_subject_respawns(), 2);
        assert_eq!(state.monsters[0].loop_pos, 4);

        assert_eq!(test_subject_nemesis_side_end(&mut state, 0).unwrap(), 0);
        assert!(!state.monsters[0].test_subject_nemesis_apply_intangible());
        assert_eq!(state.monsters[0].powers.value(PowerId::Intangible), 0);
        assert_eq!(state.monsters[0].test_subject_respawns(), 2);
        assert_eq!(state.monsters[0].loop_pos, 4);
    }

    fn stocked_axebot() -> HotState {
        let mut state = HotState::at_defaults();
        let mut owner = HotMonster::new(MonsterKind::Axebot, 0);
        owner.max_hp = 76;
        owner.powers.set(PowerId::Stock, SlotWire::Int, 2);
        state.monsters_mut().push(owner);
        state
    }

    #[test]
    fn axebot_stock_replaces_twice_then_allows_terminal_death() {
        let mut state = stocked_axebot();
        let niche_before = state.rng.get(RngStream::Niche).counter;
        let mut events = Vec::new();

        crate::engine::damage::finish_monster_death(&mut state, 0, &mut events).unwrap();
        let first = &state.monsters[0];
        assert_eq!(
            (first.kind, first.slot, first.uid, first.loop_pos),
            (MonsterKind::Axebot, 0, 1, 2)
        );
        assert!((86..=96).contains(&first.max_hp));
        assert_eq!(first.hp, first.max_hp);
        assert_eq!(first.powers.value(PowerId::Stock), 1);
        assert_eq!(first.powers.value(PowerId::Strength), 0);
        assert_eq!(state.rng.get(RngStream::Niche).counter, niche_before + 1);
        assert!(!state.history.over);

        state.monsters_mut()[0].hp = 0;
        crate::engine::damage::finish_monster_death(&mut state, 0, &mut events).unwrap();
        let second = &state.monsters[0];
        assert_eq!(
            (second.kind, second.slot, second.uid, second.loop_pos),
            (MonsterKind::Axebot, 0, 2, 2)
        );
        assert!((96..=106).contains(&second.max_hp));
        assert_eq!(second.hp, second.max_hp);
        assert_eq!(second.powers.value(PowerId::Stock), 0);
        assert_eq!(state.rng.get(RngStream::Niche).counter, niche_before + 2);
        assert!(!state.history.over);

        state.monsters_mut()[0].hp = 0;
        crate::engine::damage::finish_monster_death(&mut state, 0, &mut events).unwrap();
        assert!(state.history.over);
        assert_eq!(state.monsters[0].hp, 0);
    }

    #[test]
    fn malformed_axebot_stock_refuses_before_death_mutation_or_rng() {
        let mut state = stocked_axebot();
        state.monsters_mut()[0].uid = 9;
        let before = state.clone();
        let mut events = Vec::new();
        assert_eq!(
            crate::engine::damage::finish_monster_death(&mut state, 0, &mut events),
            Err(EngineRefusal::MalformedArgs("Axebot Stock lifecycle"))
        );
        assert_eq!(state, before);
        assert!(events.is_empty());
    }

    #[test]
    fn fabricator_spawns_share_selection_state_and_preserve_stream_slot_uid_order() {
        let mut state = fabricator_state();
        assert!(fabricator_state_is_valid(&state));
        let ai_before = state.rng.get(RngStream::Ai).counter;
        let niche_before = state.rng.get(RngStream::Niche).counter;

        spawn_fabricator_bot(&mut state, 0, &FABRICATOR_DEFENSE_BOTS).unwrap();
        spawn_fabricator_bot(&mut state, 0, &FABRICATOR_AGGRO_BOTS).unwrap();
        assert_eq!(state.rng.get(RngStream::Ai).counter, ai_before + 2);
        assert_eq!(state.rng.get(RngStream::Niche).counter, niche_before + 2);
        assert_eq!(
            state
                .monsters
                .iter()
                .map(|monster| (monster.kind, monster.slot, monster.uid, monster.max_hp))
                .collect::<Vec<_>>(),
            [
                (MonsterKind::Guardbot, 0, 1, 17),
                (MonsterKind::Zapbot, 1, 2, 19),
                (MonsterKind::Fabricator, 2, 0, 155),
            ]
        );
        let owner = state
            .monsters
            .iter()
            .find(|monster| monster.uid == 0)
            .unwrap();
        assert_eq!(owner.last_spawned, Some(MonsterKind::Zapbot));
        for bot in state
            .monsters
            .iter()
            .filter(|monster| monster.kind != MonsterKind::Fabricator)
        {
            assert_eq!(bot.powers.value(PowerId::Secondary), 1);
            assert_eq!(
                bot.powers.value(PowerId::HighVoltage),
                if bot.kind == MonsterKind::Zapbot {
                    2
                } else {
                    0
                }
            );
        }

        // The prior aggressive selection is filtered, but the singleton
        // `NextItem` still spends one AI draw.
        spawn_fabricator_bot(&mut state, 0, &FABRICATOR_AGGRO_BOTS).unwrap();
        assert_eq!(
            state
                .monsters
                .iter()
                .find(|monster| monster.uid == 0)
                .unwrap()
                .last_spawned,
            Some(MonsterKind::Stabbot)
        );
        assert_eq!(state.rng.get(RngStream::Ai).counter, ai_before + 3);
    }

    /// Once every named slot is held the spawns fall back to the empty slot
    /// (`-1`); a bot that then dies frees its named slot for the next spawn
    /// (#3123: the frozen Python retained dead slots, which native
    /// `GetNextSlot` over `Enemies` does not).
    #[test]
    fn fabricator_reuses_dead_slots_after_empty_slot_fallback() {
        let mut state = fabricator_state();
        for pool in [
            &FABRICATOR_DEFENSE_BOTS,
            &FABRICATOR_AGGRO_BOTS,
            &FABRICATOR_DEFENSE_BOTS,
            &FABRICATOR_AGGRO_BOTS,
            &FABRICATOR_DEFENSE_BOTS,
            &FABRICATOR_AGGRO_BOTS,
        ] {
            spawn_fabricator_bot(&mut state, 0, pool).unwrap();
        }
        assert_eq!(
            state
                .monsters
                .iter()
                .map(|monster| (monster.slot, monster.uid))
                .collect::<Vec<_>>(),
            [(-1, 5), (-1, 6), (0, 1), (1, 2), (2, 0), (3, 3), (4, 4)]
        );
        state
            .monsters_mut()
            .iter_mut()
            .find(|monster| monster.uid == 1)
            .unwrap()
            .hp = 0;
        spawn_fabricator_bot(&mut state, 0, &FABRICATOR_DEFENSE_BOTS).unwrap();
        assert_eq!(
            state
                .monsters
                .iter()
                .map(|monster| (monster.slot, monster.uid, monster.hp > 0))
                .collect::<Vec<_>>(),
            [
                (-1, 5, true),
                (-1, 6, true),
                (0, 1, false),
                (0, 7, true),
                (1, 2, true),
                (2, 0, true),
                (3, 3, true),
                (4, 4, true),
            ]
        );
    }

    #[test]
    fn zapbot_death_removes_high_voltage_but_retains_minion_and_ordinary_powers() {
        let mut state = HotState::at_defaults();
        let mut zap = HotMonster::new(MonsterKind::Zapbot, 0);
        zap.max_hp = 19;
        zap.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
        zap.powers.set(PowerId::HighVoltage, SlotWire::Int, 2);
        zap.powers.set(PowerId::Strength, SlotWire::Int, 6);
        zap.powers.set(PowerId::Weak, SlotWire::Int, 2);
        state.monsters_mut().push(zap);
        crate::engine::damage::finish_monster_death(&mut state, 0, &mut Vec::new()).unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::HighVoltage), 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Secondary), 1);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 6);
        assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 2);
        assert!(fabricator_state_is_valid(&state));
    }

    #[test]
    fn fabricator_primary_death_cascades_bot_high_voltage_before_terminal() {
        let mut state = HotState::at_defaults();
        let mut zap = HotMonster::new(MonsterKind::Zapbot, 19);
        zap.max_hp = 19;
        zap.slot = 0;
        zap.uid = 1;
        zap.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
        zap.powers.set(PowerId::HighVoltage, SlotWire::Int, 2);
        zap.powers.set(PowerId::Strength, SlotWire::Int, 6);
        zap.powers.set(PowerId::Weak, SlotWire::Int, 2);
        let mut owner = HotMonster::new(MonsterKind::Fabricator, 0);
        owner.max_hp = 155;
        owner.slot = 2;
        owner.last_spawned = Some(MonsterKind::Zapbot);
        assert!(
            owner
                .random_ai
                .set_next(Some(FABRICATOR_DISINTEGRATE_INDEX))
        );
        assert!(
            owner
                .random_ai
                .set_log(&[FABRICATOR_FABRICATE_INDEX, FABRICATOR_DISINTEGRATE_INDEX,])
        );
        state.monsters_mut().extend([zap, owner]);
        assert!(fabricator_state_is_valid(&state));
        let mut events = Vec::new();

        crate::engine::damage::finish_monster_death(&mut state, 1, &mut events).unwrap();

        assert_eq!(state.monsters[0].hp, 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::HighVoltage), 0);
        assert_eq!(state.monsters[0].powers.value(PowerId::Secondary), 1);
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 6);
        assert_eq!(state.monsters[0].powers.value(PowerId::Weak), 2);
        assert!(state.history.over);
        assert_eq!(
            events.last(),
            Some(&crate::engine::Event::CombatOver { player_won: true })
        );
    }

    #[test]
    fn batch_b_spawns_cannot_enter_the_four_fixed_attack_rosters() {
        // Python `monster_attack_player` validates Queen, Possess, Kin, and
        // Kaiser rosters before every hit (frozen Python, deleted #2827). Batch B's
        // complete owner -> spawn closure is disjoint on both sides: none of
        // those fixed rosters can contain a spawning owner initially, and no
        // spawned identity can become one of their required members mid-fight.
        const SPAWNS: &[(MonsterKind, MonsterKind)] = &[
            (MonsterKind::LivingFog, MonsterKind::GasBomb),
            (MonsterKind::Ovicopter, MonsterKind::ToughEgg),
            (MonsterKind::TheObscura, MonsterKind::Parafright),
            (MonsterKind::Fogmog, MonsterKind::EyeWithTeeth),
        ];
        const FIXED_ATTACK_ROSTERS: &[MonsterKind] = &[
            MonsterKind::Queen,
            MonsterKind::TorchHeadAmalgam,
            MonsterKind::TheLost,
            MonsterKind::TheForgotten,
            MonsterKind::KinFollower,
            MonsterKind::KinPriest,
            MonsterKind::Crusher,
            MonsterKind::Rocket,
        ];

        assert_eq!(SPAWNS.len(), 4, "classify every Batch B spawn edge");
        for (owner, spawned) in SPAWNS {
            assert!(!FIXED_ATTACK_ROSTERS.contains(owner));
            assert!(!FIXED_ATTACK_ROSTERS.contains(spawned));
        }
    }

    fn ovicopter_state() -> HotState {
        let mut state = HotState::at_defaults();
        let mut parent = HotMonster::new(MonsterKind::Ovicopter, 126);
        parent.max_hp = 126;
        parent.slot = 5;
        state.monsters_mut().push(parent);
        state
    }

    #[test]
    fn egg_spawn_hatch_and_branch_preserve_identity_and_rng_order() {
        let mut state = ovicopter_state();
        let niche_before = state.rng.get(RngStream::Niche).counter;
        for _ in 0..3 {
            spawn_tough_egg(&mut state, 0).unwrap();
        }
        assert_eq!(state.rng.get(RngStream::Niche).counter, niche_before + 3);
        assert_eq!(
            state
                .monsters
                .iter()
                .map(|monster| (monster.kind, monster.slot, monster.uid))
                .collect::<Vec<_>>(),
            [
                (MonsterKind::ToughEgg, 2, 3),
                (MonsterKind::ToughEgg, 3, 2),
                (MonsterKind::ToughEgg, 4, 1),
                (MonsterKind::Ovicopter, 5, 0),
            ]
        );
        let egg_uid = 1;
        let egg_index = state
            .monsters
            .iter()
            .position(|monster| monster.uid == egg_uid)
            .unwrap();
        state.monsters_mut()[egg_index]
            .powers
            .set(PowerId::HatchPower, SlotWire::Int, 1);
        state.monsters_mut()[egg_index]
            .powers
            .set(PowerId::Poison, SlotWire::Int, 5);
        state.monsters_mut()[egg_index].poison_uid = 0;
        state.next_poison_uid = 1;
        state.monsters_mut()[egg_index].block = 7;
        hatch_tough_egg(&mut state, egg_uid).unwrap();
        let egg = state
            .monsters
            .iter()
            .find(|monster| monster.uid == egg_uid)
            .unwrap();
        assert_eq!((egg.slot, egg.uid, egg.block, egg.loop_pos), (4, 1, 7, 1));
        assert_eq!(egg.powers.value(PowerId::Secondary), 1);
        assert_eq!(egg.powers.value(PowerId::IsHatched), 1);
        assert_eq!((egg.powers.value(PowerId::Poison), egg.poison_uid), (0, -1));
        assert_eq!(state.next_poison_uid, 1);
        assert!((20..=23).contains(&egg.hp));

        state
            .monsters_mut()
            .iter_mut()
            .find(|monster| monster.kind == MonsterKind::Ovicopter)
            .unwrap()
            .loop_pos = OVICOPTER_BRANCH_POS;
        resolve_ovicopter_after_side_end(&mut state).unwrap();
        assert_eq!(state.monsters.last().unwrap().loop_pos, 3);
    }

    /// #3823: an unhatched egg whose HatchPower ran out under a stun is a
    /// valid roster member and still hatches; a HatchPower above 2, or one
    /// left on a hatched egg, is not.
    #[test]
    fn a_stunned_out_hatch_counter_is_valid_and_still_hatches() {
        let mut state = ovicopter_state();
        spawn_tough_egg(&mut state, 0).unwrap();
        let egg = state
            .monsters
            .iter()
            .position(|monster| monster.kind == MonsterKind::ToughEgg)
            .unwrap();
        // The side end of the STUNNED turn decrements 1 to nothing.
        state.monsters_mut()[egg]
            .powers
            .set(PowerId::HatchPower, SlotWire::Int, 0);
        assert_eq!(state.monsters[egg].powers.value(PowerId::HatchPower), 0);
        assert!(ovicopter_roster_is_valid(&state));
        state.monsters_mut()[egg]
            .powers
            .set(PowerId::HatchPower, SlotWire::Int, 3);
        assert!(!ovicopter_roster_is_valid(&state));
        state.monsters_mut()[egg]
            .powers
            .set(PowerId::HatchPower, SlotWire::Int, 2);
        assert!(
            hatch_tough_egg(&mut state, 1).is_err(),
            "HATCH with the counter at 2"
        );
        state.monsters_mut()[egg]
            .powers
            .set(PowerId::HatchPower, SlotWire::Int, 0);
        hatch_tough_egg(&mut state, 1).unwrap();
        let hatched = &state.monsters[egg];
        assert_eq!(
            (hatched.loop_pos, hatched.powers.value(PowerId::IsHatched)),
            (1, 1)
        );
        state.monsters_mut()[egg]
            .powers
            .set(PowerId::HatchPower, SlotWire::Int, 1);
        assert!(
            !ovicopter_roster_is_valid(&state),
            "a counter on a hatched egg"
        );
    }

    /// #3823: Ovicopter and Tough Egg take the generic stun on every loop
    /// row; the Ovicopter's SUMMON_BRANCH sentinel names no parked telegraph.
    #[test]
    fn ovicopter_and_tough_egg_park_every_row_but_the_branch_sentinel() {
        for kind in [MonsterKind::Ovicopter, MonsterKind::ToughEgg] {
            assert!(generic_stun_owner_is_eligible(kind), "{kind:?}");
            assert!(
                generic_stun_owner_loop_is_fully_resolvable(kind),
                "{kind:?}"
            );
        }
        assert_eq!(
            parked_telegraph(MonsterKind::Ovicopter, OVICOPTER_BRANCH_POS),
            None
        );
        assert!(parked_telegraph(MonsterKind::Ovicopter, OVICOPTER_BRANCH_POS - 1).is_some());
    }

    #[test]
    fn ovicopter_reuses_dead_slots_with_new_creation_uids() {
        let mut state = ovicopter_state();
        for _ in 0..3 {
            spawn_tough_egg(&mut state, 0).unwrap();
        }
        for egg in state
            .monsters_mut()
            .iter_mut()
            .filter(|m| matches!(m.uid, 1 | 3))
        {
            egg.hp = 0;
            egg.powers = Slots::new();
            egg.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
        }
        let before = state.rng.get(RngStream::Niche).counter;
        for _ in 0..3 {
            spawn_tough_egg(&mut state, 0).unwrap();
        }
        assert_eq!(state.rng.get(RngStream::Niche).counter, before + 3);
        assert_eq!(
            state
                .monsters
                .iter()
                .map(|m| (m.uid, m.slot))
                .collect::<Vec<_>>(),
            [(6, 1), (5, 2), (2, 3), (4, 4), (0, 5)]
        );
        assert!(ovicopter_roster_is_valid(&state));
        state.monsters_mut()[0].slot = 3;
        assert!(
            !ovicopter_roster_is_valid(&state),
            "live eggs cannot share a slot"
        );
    }

    #[test]
    fn gas_bomb_and_illusion_spawns_are_slot_ordered_and_spend_one_draw() {
        let mut fog = HotState::at_defaults();
        let mut owner = HotMonster::new(MonsterKind::LivingFog, 100);
        owner.max_hp = 100;
        owner.slot = 5;
        fog.monsters_mut().push(owner);
        let before = fog.rng.get(RngStream::Niche).counter;
        spawn_gas_bomb(&mut fog, 0).unwrap();
        assert_eq!(fog.rng.get(RngStream::Niche).counter, before + 1);
        assert_eq!(
            fog.monsters
                .iter()
                .map(|monster| (monster.kind, monster.slot, monster.uid, monster.hp))
                .collect::<Vec<_>>(),
            [
                (MonsterKind::GasBomb, 0, 1, 8),
                (MonsterKind::LivingFog, 5, 0, 100),
            ]
        );

        let mut obscura = HotState::at_defaults();
        let mut owner = HotMonster::new(MonsterKind::TheObscura, 129);
        owner.max_hp = 129;
        owner.slot = 1;
        obscura.monsters_mut().push(owner);
        spawn_illusion(&mut obscura, 0, MonsterKind::Parafright, 21).unwrap();
        assert_eq!(obscura.monsters[0].kind, MonsterKind::Parafright);
        assert!(is_secondary_enemy(&obscura.monsters[0]));
    }

    /// Hand the CALL_FOR_BACKUP intent to rat `uid` (tus 0, one-shot used).
    fn rat_calls(state: &mut HotState, uid: u32, log: [u8; 2]) {
        let rat = state
            .monsters_mut()
            .iter_mut()
            .find(|rat| rat.uid == uid)
            .unwrap();
        rat.set_rat_turns_until_summonable(0);
        let call = super::super::turn::RAT_CALL_FOR_BACKUP_INDEX;
        assert!(rat.random_ai.set_next(Some(call)));
        assert!(rat.random_ai.set_log(&[log[0], log[1], call]));
        assert!(rat.random_ai.set_once(&[call]));
    }

    /// Retire rat `uid`'s pending call to Scratch after it has summoned.
    fn rat_retires_call(state: &mut HotState, uid: u32) {
        let rat = state
            .monsters_mut()
            .iter_mut()
            .find(|rat| rat.uid == uid)
            .unwrap();
        assert!(
            rat.random_ai
                .set_next(Some(super::super::turn::RAT_SCRATCH_INDEX))
        );
        assert!(
            rat.random_ai
                .push_log(super::super::turn::RAT_SCRATCH_INDEX)
        );
    }

    /// The newcomer's part of the `StartTurn` walk (#3747): a fresh rat was
    /// published without an intent, rolls its first one here, and the latch
    /// clears. The other rats keep the intents these tests hand them.
    fn clear_fresh(state: &mut HotState) {
        for index in 0..state.monsters.len() {
            if state.monsters[index].rat_spawn_fresh() {
                super::super::turn::roll_rat_intent(state, index).unwrap();
                state.monsters_mut()[index].set_rat_spawn_fresh(false);
            }
        }
    }

    fn rat_identity(state: &HotState) -> Vec<(i32, u32, bool, u8)> {
        state
            .monsters
            .iter()
            .map(|rat| {
                (
                    rat.slot,
                    rat.uid,
                    rat.hp > 0,
                    rat.rat_call_for_backup_count(),
                )
            })
            .collect()
    }

    /// With no corpse the first summon takes native `second` (label 4), the
    /// LAST free `get_Slots` entry, and `SortEnemiesBySlotName` puts it ahead
    /// of the initial `third`..`fifth` rats (#3146). One Niche draw for HP,
    /// one AI draw for the newcomer's intent, and every living rat's count
    /// becomes 1.
    #[test]
    fn rat_spawn_uses_last_free_slot_fresh_uid_unique_hp_and_two_streams() {
        let mut state = rat_state();
        assert!(rat_roster_is_valid(&state));
        let niche_before = state.rng.get(RngStream::Niche).counter;
        let ai_before = state.rng.get(RngStream::Ai).counter;

        spawn_rat(&mut state, 0).unwrap();

        assert_eq!(state.rng.get(RngStream::Niche).counter, niche_before + 1);
        assert_eq!(
            state.rng.get(RngStream::Ai).counter,
            ai_before,
            "an enemy-side Add rolls no intent (#3747)"
        );
        assert_eq!(
            rat_identity(&state),
            [
                (4, 3, true, 1),
                (0, 0, true, 1),
                (1, 1, true, 1),
                (2, 2, true, 1)
            ]
        );
        let spawned = &state.monsters[0];
        assert!((18..=22).contains(&spawned.hp));
        assert!(![18, 19, 20].contains(&spawned.max_hp));
        assert_eq!(spawned.rat_turns_until_summonable(), 2);
        assert!(spawned.rat_spawn_fresh());
        assert_eq!(spawned.random_ai.next(), None);
        assert_eq!(spawned.random_ai.log_len(), 0);
        assert!(rat_ai_state_is_valid(spawned, true));
        assert!(!rat_ai_state_is_valid(spawned, false));

        // `fresh_roll` is an in-phase latch. The `StartTurn` walk rolls the
        // newcomer's first intent and clears it, which returns to an
        // admissible roster.
        clear_fresh(&mut state);
        assert_eq!(state.rng.get(RngStream::Ai).counter, ai_before + 1);
        let spawned = &state.monsters[0];
        assert!(matches!(
            spawned.random_ai.next(),
            Some(
                super::super::turn::RAT_SCRATCH_INDEX
                    | super::super::turn::RAT_DISEASE_BITE_INDEX
                    | super::super::turn::RAT_SCREECH_INDEX
            )
        ));
        assert_eq!(spawned.random_ai.log_len(), 1);
        assert_eq!(spawned.random_ai.once_len(), 0);
        assert!(rat_roster_is_valid(&state));
    }

    /// #3146 (census witness `faf12d6d4ca2e06b`): a dead rat has left native
    /// `Enemies`, so CALL_FOR_BACKUP's `LastOrDefault` hands its slot to the
    /// newcomer, the unique-HP roll no longer reserves its max HP, and the
    /// count synchronization leaves the corpse's count alone. A later dead
    /// summon keeps the count it died with.
    #[test]
    fn rat_spawn_reuses_a_dead_rats_slot_hp_and_skips_its_count() {
        let mut state = rat_state();
        // uid 1: label 1 = native `fourth`, max HP 19.
        state.monsters_mut()[1].hp = 0;
        assert!(rat_roster_is_valid(&state));
        let (low, high) = native_hp_band(&state, MonsterKind::TwoTailedRat).unwrap();
        let mut native = state.clone();
        let expected = roll_unique_hp_over(&mut native, low, high, &[18, 20]).unwrap();
        let mut all_rows = state.clone();
        let with_corpse = roll_unique_hp_over(&mut all_rows, low, high, &[18, 19, 20]).unwrap();
        assert_ne!(
            expected, with_corpse,
            "the seed must separate the two candidate sets"
        );

        spawn_rat(&mut state, 0).unwrap();
        assert_eq!(
            rat_identity(&state),
            [
                (0, 0, true, 1),
                (1, 1, false, 0),
                (1, 3, true, 1),
                (2, 2, true, 1)
            ]
        );
        assert_eq!(state.monsters[2].max_hp, expected);
        clear_fresh(&mut state);
        assert!(rat_roster_is_valid(&state));

        // The newcomer dies; rat 2 then calls. Free native slots are now
        // `first`, `second` and `fourth`: `fourth` (label 1) again.
        rat_retires_call(&mut state, 0);
        state.monsters_mut()[2].hp = 0;
        rat_calls(
            &mut state,
            2,
            [
                super::super::turn::RAT_SCREECH_INDEX,
                super::super::turn::RAT_SCRATCH_INDEX,
            ],
        );
        assert!(rat_roster_is_valid(&state));
        spawn_rat(&mut state, 2).unwrap();
        assert_eq!(
            rat_identity(&state),
            [
                (0, 0, true, 2),
                (1, 1, false, 0),
                (1, 3, false, 1),
                (1, 4, true, 2),
                (2, 2, true, 2)
            ]
        );
        clear_fresh(&mut state);
        assert!(rat_roster_is_valid(&state));
    }

    /// Five living rats hold every slot. CALL_FOR_BACKUP cannot be selected
    /// then (`CanSummon`), but a call parked under a stun is still performed
    /// (#3823): native skips the Add and still synchronizes the counts
    /// (`<CallForBackup>d__41` `0x373d98` IL_010a-IL_010f, IL_01e1-IL_025d).
    /// No row is added, no stream is drawn, the count rises to the cap, and
    /// the roster is one the validator accepts with two summons and count 3.
    #[test]
    fn rat_call_with_every_slot_held_raises_the_counts_and_summons_nobody() {
        let mut state = rat_state();
        spawn_rat(&mut state, 0).unwrap();
        clear_fresh(&mut state);
        rat_retires_call(&mut state, 0);
        rat_calls(
            &mut state,
            1,
            [
                super::super::turn::RAT_DISEASE_BITE_INDEX,
                super::super::turn::RAT_SCRATCH_INDEX,
            ],
        );
        spawn_rat(&mut state, 1).unwrap();
        clear_fresh(&mut state);
        assert_eq!(
            state
                .monsters
                .iter()
                .map(|rat| (rat.slot, rat.uid))
                .collect::<Vec<_>>(),
            [(3, 4), (4, 3), (0, 0), (1, 1), (2, 2)]
        );
        rat_retires_call(&mut state, 1);
        rat_calls(
            &mut state,
            3,
            [
                super::super::turn::RAT_SCRATCH_INDEX,
                super::super::turn::RAT_DISEASE_BITE_INDEX,
            ],
        );
        assert!(rat_roster_is_valid(&state));
        let before = state.clone();
        spawn_rat(&mut state, 3).unwrap();
        assert_eq!(state.monsters.len(), 5);
        assert_eq!(state.rng, before.rng);
        assert!(
            state
                .monsters
                .iter()
                .all(|rat| rat.rat_call_for_backup_count() == RAT_CALL_COUNT_CAP)
        );
        assert!(rat_roster_is_valid(&state));

        // A further call (one more rat was parked under a stun) leaves the
        // count at the cap: every value from 3 up reads the same in
        // `CanSummon`.
        rat_retires_call(&mut state, 3);
        rat_calls(
            &mut state,
            4,
            [
                super::super::turn::RAT_SCRATCH_INDEX,
                super::super::turn::RAT_DISEASE_BITE_INDEX,
            ],
        );
        spawn_rat(&mut state, 4).unwrap();
        assert!(
            state
                .monsters
                .iter()
                .all(|rat| rat.rat_call_for_backup_count() == RAT_CALL_COUNT_CAP)
        );
        assert!(rat_roster_is_valid(&state));
    }

    /// #3823: two rats hold CALL_FOR_BACKUP together (one was parked under a
    /// stun while the other rolled), and both perform it in one enemy side.
    /// The second call reads a roster that holds the first call's newcomer
    /// with no intent yet, summons into the next free slot, and the counts
    /// go to 2. After the `StartTurn` walk the roster is admissible.
    #[test]
    fn two_rats_call_for_backup_in_one_enemy_side() {
        let mut state = rat_state();
        let log = [
            super::super::turn::RAT_DISEASE_BITE_INDEX,
            super::super::turn::RAT_SCRATCH_INDEX,
        ];
        rat_calls(&mut state, 0, log);
        rat_calls(&mut state, 1, log);
        assert!(
            rat_roster_is_valid(&state),
            "two living rats may hold the call"
        );
        spawn_rat(&mut state, 0).unwrap();
        assert!(!rat_roster_is_valid(&state), "a fresh rat is mid-side only");
        spawn_rat(&mut state, 1).unwrap();
        assert_eq!(
            rat_identity(&state),
            [
                (3, 4, true, 2),
                (4, 3, true, 2),
                (0, 0, true, 2),
                (1, 1, true, 2),
                (2, 2, true, 2)
            ]
        );
        clear_fresh(&mut state);
        rat_retires_call(&mut state, 0);
        rat_retires_call(&mut state, 1);
        assert!(rat_roster_is_valid(&state));
    }

    /// #3823: the living rats hold one count, inside the band the number of
    /// summoned rows allows, and a corpse that reached the cap is not dated
    /// against the third summon or a later one.
    #[test]
    fn rat_roster_counts_follow_the_summoned_rows() {
        let mut state = rat_state();
        // No summon: every count is 0.
        assert!(state.monsters_mut()[0].set_rat_call_for_backup_count(1));
        assert!(!rat_roster_is_valid(&state));
        let mut state = rat_state();
        spawn_rat(&mut state, 0).unwrap();
        clear_fresh(&mut state);
        rat_retires_call(&mut state, 0);
        assert!(rat_roster_is_valid(&state));
        // One summon: 1, never 2, and never two different living counts.
        for index in 0..4 {
            let mut changed = state.clone();
            assert!(changed.monsters_mut()[index].set_rat_call_for_backup_count(2));
            assert!(!rat_roster_is_valid(&changed), "rat {index}");
        }
        let mut all = state.clone();
        for rat in all.monsters_mut().iter_mut() {
            assert!(rat.set_rat_call_for_backup_count(2));
        }
        assert!(!rat_roster_is_valid(&all));
        // A rat that died after the summon keeps the count it died with: 1
        // here. 0 would date its death before the summon, which then took a
        // slot that was not the last free one; 2 is above the living bound.
        let mut dead = state.clone();
        dead.monsters_mut()[2].hp = 0;
        assert!(rat_roster_is_valid(&dead));
        assert!(dead.monsters_mut()[2].set_rat_call_for_backup_count(0));
        assert!(!rat_roster_is_valid(&dead));
        assert!(dead.monsters_mut()[2].set_rat_call_for_backup_count(2));
        assert!(!rat_roster_is_valid(&dead));
    }

    /// Each necessary condition the reworked validator enforces on a summoned
    /// row rejects its own mutation of an otherwise valid corpse-reuse roster.
    #[test]
    fn rat_roster_rejects_each_summon_lineage_violation() {
        let mut state = rat_state();
        state.monsters_mut()[1].hp = 0;
        spawn_rat(&mut state, 0).unwrap();
        clear_fresh(&mut state);
        assert!(rat_roster_is_valid(&state));
        let newcomer = 2;
        for mutation in 0..8 {
            let mut changed = state.clone();
            match mutation {
                // Not the LAST free slot: native `second` (label 4) while
                // `fourth` was free.
                0 => {
                    let mut rat = changed.monsters_mut().remove(newcomer);
                    rat.slot = 4;
                    changed.monsters_mut().insert(0, rat);
                }
                // A live older rat's slot.
                1 => {
                    let mut rat = changed.monsters_mut().remove(newcomer);
                    rat.slot = 2;
                    changed.monsters_mut().push(rat);
                }
                // A live older rat's max HP.
                2 => changed.monsters_mut()[newcomer].max_hp = 18,
                // A live rat off the completed summon count.
                3 => {
                    assert!(changed.monsters_mut()[0].set_rat_call_for_backup_count(0));
                }
                // A dead summon below its own creation count.
                4 => {
                    changed.monsters_mut()[newcomer].hp = 0;
                    assert!(changed.monsters_mut()[newcomer].set_rat_call_for_backup_count(0));
                }
                // A corpse above the completed summon count.
                5 => {
                    assert!(changed.monsters_mut()[1].set_rat_call_for_backup_count(2));
                }
                // An untracked counter makes the summon uid exactly 3.
                6 => changed.monsters_mut()[newcomer].uid = 5,
                // The newcomer sorted ahead of the older corpse in its slot.
                7 => changed.monsters_mut().swap(1, 2),
                _ => unreachable!(),
            }
            assert!(!rat_roster_is_valid(&changed), "mutation {mutation}");
        }
    }

    #[test]
    fn rat_roster_rejects_each_identity_count_and_ai_sensitivity() {
        let state = rat_state();
        for mutation in 0..8 {
            let mut changed = state.clone();
            match mutation {
                0 => changed.monsters_mut()[1].slot = 0,
                1 => changed.monsters_mut()[1].uid = 0,
                2 => changed.monsters_mut()[1].max_hp = 18,
                3 => {
                    assert!(changed.monsters_mut()[1].set_rat_call_for_backup_count(1));
                }
                4 => changed.monsters_mut()[1].set_rat_spawn_fresh(true),
                5 => {
                    changed.monsters_mut().swap(0, 1);
                }
                6 => changed.monsters_mut()[1].set_rat_turns_until_summonable(i32::MIN),
                7 => {
                    changed.monsters_mut()[0].set_rat_turns_until_summonable(2);
                    assert!(
                        changed.monsters_mut()[0]
                            .random_ai
                            .set_next(Some(super::super::turn::RAT_SCRATCH_INDEX))
                    );
                    assert!(
                        changed.monsters_mut()[0]
                            .random_ai
                            .set_log(&[super::super::turn::RAT_SCRATCH_INDEX])
                    );
                    assert!(changed.monsters_mut()[0].random_ai.set_once(&[]));
                    assert!(
                        changed.monsters_mut()[1]
                            .random_ai
                            .set_next(Some(super::super::turn::RAT_SCREECH_INDEX))
                    );
                    assert!(
                        changed.monsters_mut()[1]
                            .random_ai
                            .set_log(&[super::super::turn::RAT_SCREECH_INDEX])
                    );
                }
                _ => unreachable!(),
            }
            assert!(!rat_roster_is_valid(&changed), "mutation {mutation}");
        }
    }

    fn queen_state() -> HotState {
        let mut state = HotState::at_defaults();
        let mut amalgam = HotMonster::new(MonsterKind::TorchHeadAmalgam, TORCH_HEAD_AMALGAM_HP);
        amalgam.max_hp = TORCH_HEAD_AMALGAM_HP;
        amalgam.powers.set(PowerId::Secondary, SlotWire::Bool, 1);
        let mut queen = HotMonster::new(MonsterKind::Queen, QUEEN_HP);
        queen.max_hp = QUEEN_HP;
        queen.slot = 1;
        queen.uid = 1;
        state.monsters_mut().extend([amalgam, queen]);
        state
    }

    #[test]
    fn gremlin_merc_private_lane_rejects_negative_returned_gold() {
        let mut state = HotState::at_defaults();
        let mut merc = HotMonster::new(MonsterKind::GremlinMerc, 53);
        merc.max_hp = 53;
        assert!(merc.gremlin_merc_private_state_is_exact());
        merc.forge_waterfall_steam_payload(-1);
        assert!(!merc.gremlin_merc_private_state_is_exact());
        state.monsters_mut().push(merc);
        assert!(!gremlin_merc_state_is_valid(&state));
    }

    #[test]
    fn queen_stable_and_internal_latch_languages_are_disjoint_and_complete() {
        let live = queen_state();
        assert!(queen_roster_state_is_valid(&live));

        let mut transient = live.clone();
        transient.monsters_mut()[0].hp = 0;
        assert!(!queen_roster_state_is_valid(&transient));
        assert!(queen_roster_internal_state_is_valid(&transient));

        let mut retained = transient.clone();
        retained.monsters_mut()[1].loop_pos = 2;
        retained.monsters_mut()[1].set_queen_amalgam_dead(true);
        retained.monsters_mut()[1].set_queen_burn_bright_retained(true);
        assert!(queen_roster_state_is_valid(&retained));
        retained.monsters_mut()[1].set_queen_burn_bright_retained(false);
        assert!(!queen_roster_state_is_valid(&retained));
        assert!(queen_roster_internal_state_is_valid(&retained));

        for (dead, retained) in [(false, false), (true, false), (true, true)] {
            let mut both_dead = queen_state();
            both_dead.monsters_mut()[0].hp = 0;
            both_dead.monsters_mut()[1].hp = 0;
            both_dead.monsters_mut()[1].set_queen_amalgam_dead(dead);
            both_dead.monsters_mut()[1].set_queen_burn_bright_retained(retained);
            assert!(queen_roster_state_is_valid(&both_dead));
        }

        let mut forbidden = queen_state();
        forbidden.monsters_mut()[1].hp = 0;
        assert!(!queen_roster_state_is_valid(&forbidden));
        assert!(queen_roster_internal_state_is_valid(&forbidden));
    }

    #[test]
    fn queen_death_listener_latches_each_branch_and_rejects_duplicate_delivery() {
        for loop_pos in [0, 1, 2, 3, 4, 5] {
            let mut state = queen_state();
            state.monsters_mut()[0].hp = 0;
            state.monsters_mut()[1].loop_pos = loop_pos;
            queen_after_monster_death(&mut state, MonsterKind::TorchHeadAmalgam).unwrap();
            let queen = &state.monsters[1];
            assert!(queen.queen_amalgam_dead());
            assert!(!queen.queen_burn_bright_retained());
            assert_eq!(queen.loop_pos, if loop_pos == 2 { 5 } else { loop_pos });
            assert_eq!(
                queen_after_monster_death(&mut state, MonsterKind::TorchHeadAmalgam),
                Err(EngineRefusal::MalformedArgs(
                    "Queen duplicate Amalgam death latch"
                )),
                "loop {loop_pos}",
            );
        }

        let mut stunned = queen_state();
        stunned.monsters_mut()[0].hp = 0;
        stunned.monsters_mut()[1].loop_pos = 2;
        stunned.monsters_mut()[1].override_state = MonsterOverride::Stunned;
        stunned.monsters_mut()[1].forced_follow_up = MonsterFollowUp::QueenBurnBright;
        queen_after_monster_death(&mut stunned, MonsterKind::TorchHeadAmalgam).unwrap();
        assert!(stunned.monsters[1].queen_amalgam_dead());
        assert!(stunned.monsters[1].queen_burn_bright_retained());
        assert_eq!(stunned.monsters[1].loop_pos, 2);
    }

    /// `FurCoat/<BeforeCombatStart>d__26::MoveNext` (`0x325360`): in a marked
    /// room every hittable enemy goes to 1 HP through
    /// `SetCurrentHpInternal` (`0x11d734`, `Math.Min(amount, MaxHp)`), with
    /// max HP untouched. Witnesses each arm of
    /// [`fur_coat_before_combat_start`]: an inactive flag writes nothing, a
    /// living enemy goes to 1 whatever its HP, and a dead one is not hittable.
    #[test]
    fn fur_coat_sets_every_living_enemy_to_one_hp_only_in_a_marked_room() {
        let roster = |active: bool| {
            let mut state = HotState::at_defaults();
            state.set_fur_coat_active(active);
            for (uid, hp) in [(0, 40), (1, 0), (2, 1)] {
                let mut monster = HotMonster::new(MonsterKind::Toadpole, hp);
                monster.uid = uid;
                monster.max_hp = 40;
                state.monsters_mut().push(monster);
            }
            fur_coat_before_combat_start(&mut state);
            state
                .monsters
                .iter()
                .map(|monster| (monster.hp, monster.max_hp))
                .collect::<Vec<_>>()
        };
        assert_eq!(roster(true), [(1, 40), (0, 40), (1, 40)]);
        assert_eq!(roster(false), [(40, 40), (0, 40), (1, 40)]);
    }

    #[test]
    fn philosophers_stone_buffs_each_new_living_opponent_once() {
        let mut state = HotState::at_defaults();
        state.set_philosophers_stone_owned(true);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 20);
        monster.uid = 9;
        state.monsters_mut().push(monster);

        fur_coat_after_opponent_added(&mut state, 9).unwrap();
        assert_eq!(state.monsters[0].powers.value(PowerId::Strength), 1);

        state.history.over = true;
        let mut ended = HotMonster::new(MonsterKind::Toadpole, 20);
        ended.uid = 10;
        state.monsters_mut().push(ended);
        fur_coat_after_opponent_added(&mut state, 10).unwrap();
        assert_eq!(state.monsters[1].powers.value(PowerId::Strength), 0);
    }

    /// #3934: the Murderous modifier's `AfterCreatureAddedToCombat`. A living
    /// added opponent takes three Strength with a null applier, on top of
    /// Philosopher's Stone's one when both are held; a dead one, and any one
    /// added to a combat that is over, takes nothing; without the modifier
    /// the row is absent.
    #[test]
    fn murderous_gives_each_new_living_opponent_three_strength() {
        let added = |state: &mut HotState, uid: u32, hp: i32| {
            let mut monster = HotMonster::new(MonsterKind::Toadpole, 20);
            monster.uid = uid;
            monster.hp = hp;
            state.monsters_mut().push(monster);
            fur_coat_after_opponent_added(state, uid).unwrap();
            state
                .monsters
                .last()
                .unwrap()
                .powers
                .value(PowerId::Strength)
        };
        let mut state = HotState::at_defaults();
        assert_eq!(added(&mut state, 9, 20), 0);

        state.fanouts.set_murderous(true);
        assert_eq!(added(&mut state, 10, 20), MURDEROUS_STRENGTH);
        assert_eq!(MURDEROUS_STRENGTH, 3);
        // Dead on arrival: nothing to apply to.
        assert_eq!(added(&mut state, 11, 0), 0);

        state.set_philosophers_stone_owned(true);
        assert_eq!(added(&mut state, 12, 20), 4);
        // The earlier arrivals are not buffed again.
        assert_eq!(state.monsters[1].powers.value(PowerId::Strength), 3);

        state.history.over = true;
        assert_eq!(added(&mut state, 13, 20), 0);
    }

    // ---- #2647 slice A: the generic recipient stun -------------------------

    fn stunnable(kind: MonsterKind, loop_pos: i32) -> HotState {
        let mut state = HotState::at_defaults();
        let mut monster = HotMonster::new(kind, 40);
        monster.uid = 3;
        monster.loop_pos = loop_pos;
        state.monsters_mut().push(monster);
        state
    }

    /// `parked_telegraph` replaces a two-kind hand table with a read of the
    /// generated loop, so it owes agreement with that table on every row it
    /// covered — all eleven, plus each machine's out-of-range neighbours.
    #[test]
    fn parked_telegraph_agrees_with_queen_amalgam_table() {
        let mut covered = 0;
        for kind in [MonsterKind::Queen, MonsterKind::TorchHeadAmalgam] {
            for loop_pos in -1..=8 {
                let table = queen_amalgam_follow_up(kind, loop_pos);
                assert_eq!(
                    parked_telegraph(kind, loop_pos),
                    table,
                    "{kind:?} {loop_pos}"
                );
                covered += i32::from(table.is_some());
            }
        }
        assert_eq!(covered, 11);
    }

    /// Every `None` reason, one case each (#2647 §5.5).
    #[test]
    fn parked_telegraph_refuses_every_ambiguous_telegraph() {
        // A random-AI kind has no deterministic loop to index at all. One
        // outside `GENERIC_STUN_RANDOM_TABLE_KINDS` has no parked telegraph;
        // one inside parks the row `random_ai.next` names, at `loop_pos` 0
        // only (#3823).
        assert!(crate::content_tables::monster_loop(MonsterKind::Mawler).is_none());
        for kind in MonsterKind::ALL.into_iter().filter(|kind| {
            crate::content_tables::random_moves(*kind).is_some()
                && !GENERIC_STUN_RANDOM_TABLE_KINDS.contains(kind)
        }) {
            assert_eq!(parked_telegraph(kind, 0), None, "{kind:?}");
        }
        assert_eq!(
            parked_telegraph(MonsterKind::TheObscura, 0),
            Some(MonsterFollowUp::ParkedTelegraph)
        );
        assert_eq!(
            parked_telegraph(MonsterKind::TwoTailedRat, 0),
            Some(MonsterFollowUp::ParkedTelegraph)
        );
        assert_eq!(parked_telegraph(MonsterKind::TwoTailedRat, 1), None);
        assert_eq!(
            parked_telegraph(MonsterKind::Mawler, 0),
            Some(MonsterFollowUp::ParkedTelegraph)
        );
        assert_eq!(parked_telegraph(MonsterKind::Mawler, 1), None);
        // Slumbering Beetle's executable table is the private native loop the
        // catalog synthesizes, not a generated one.
        assert_eq!(parked_telegraph(MonsterKind::SlumberingBeetle, 0), None);
        // Out of range, both directions.
        assert_eq!(parked_telegraph(MonsterKind::BowlbugEgg, 1), None);
        assert_eq!(parked_telegraph(MonsterKind::BowlbugEgg, -1), None);
        assert_eq!(
            parked_telegraph(MonsterKind::BowlbugEgg, 0),
            Some(MonsterFollowUp::EggBite)
        );
        // A row whose name is outside the closed vocabulary is no longer a
        // refusal (#3823): the loop parks the generic token, in range only.
        let toadpole = crate::content_tables::monster_loop(MonsterKind::Toadpole).unwrap();
        assert!(MonsterFollowUp::from_str(toadpole[0].name).is_none());
        assert_eq!(
            parked_telegraph(MonsterKind::Toadpole, 0),
            Some(MonsterFollowUp::ParkedTelegraph)
        );
        assert_eq!(
            parked_telegraph(MonsterKind::Toadpole, toadpole.len() as i32),
            None
        );
        assert_eq!(parked_telegraph(MonsterKind::Toadpole, -1), None);
        assert_eq!(
            parked_telegraph(MonsterKind::BowlbugSilk, 0),
            Some(MonsterFollowUp::SilkToxicSpit)
        );
        assert_eq!(
            parked_telegraph(MonsterKind::BowlbugSilk, 1),
            Some(MonsterFollowUp::SilkThrash)
        );
    }

    /// A duplicated move name does not identify one index to restore, so it
    /// must not resolve — whatever the vocabulary says about the name.
    ///
    /// Exercised on an explicit loop because no v0.111.0 generated loop repeats
    /// a name (the companion test below). Both rows here name a real
    /// vocabulary member, so only the duplication can be what refuses.
    #[test]
    fn parked_telegraph_refuses_a_duplicated_move_name() {
        const REPEATED: &[crate::content_tables::Move] = &[
            crate::content_tables::Move {
                name: "BITE",
                kind: crate::ids::MoveKind::AttackBlock,
                args: &[],
                repeats: crate::content_tables::Repeats::Absent,
            },
            crate::content_tables::Move {
                name: "BUFF",
                kind: crate::ids::MoveKind::BuffStrength,
                args: &[],
                repeats: crate::content_tables::Repeats::Absent,
            },
            crate::content_tables::Move {
                name: "BITE",
                kind: crate::ids::MoveKind::AttackBlock,
                args: &[],
                repeats: crate::content_tables::Repeats::Absent,
            },
        ];
        assert_eq!(parked_telegraph_in(REPEATED, 0), None);
        assert_eq!(parked_telegraph_in(REPEATED, 2), None);
        // The unique sibling still resolves, so this refuses the ambiguous row
        // rather than the whole machine.
        assert_eq!(
            parked_telegraph_in(REPEATED, 1),
            Some(MonsterFollowUp::NectarBuff)
        );
    }

    /// Why the arm above needs a synthetic loop, and the tripwire for a build
    /// that changes the answer: a repeated name would make this a live case.
    #[test]
    fn no_generated_loop_repeats_a_move_name() {
        for (kind, rows) in crate::content_tables::LOOPS {
            for row in rows {
                assert_eq!(
                    rows.iter().filter(|other| other.name == row.name).count(),
                    1,
                    "{kind:?} repeats {}",
                    row.name
                );
            }
        }
    }

    /// #2647 §6 witness 10b — the pin the enum has never had.
    ///
    /// `MonsterFollowUp` is a vocabulary of move NAMES, and nothing before this
    /// checked that a member names a move the generated tables actually carry:
    /// `every_wire_vocabulary_is_ascending_and_round_trips` is structural and
    /// the boundary only checks membership. Slice A resolves members by reading
    /// the loops, so a member naming no real row would silently never resolve.
    #[test]
    fn every_monster_follow_up_name_exists_in_a_generated_loop() {
        for follow_up in MonsterFollowUp::ALL {
            let name = follow_up.as_str();
            // `None`, and Living Shield's deferred-successor receipt, which is
            // a pseudo-name rather than an interrupting forced move.
            if name.is_empty() || name == "LIVING_SHIELD_SLAM_RECEIPT" {
                continue;
            }
            // #3823: the generic token names "the row `loop_pos` names", not
            // a row; `the_named_parked_telegraph_kinds_are_pinned` pins that
            // no generated row is spelled like it.
            if follow_up == MonsterFollowUp::ParkedTelegraph {
                continue;
            }
            // Slumbering Beetle's ROLL_OUT_MOVE is its catalog-private
            // executable row (`catalog.rs` `BEETLE_ROLLOUT`); pin it there.
            // #3823: the sleep moves are not loop rows either. Each is the
            // machine's initial state, which a sleeping Lagavulin or Beetle
            // shows while its power lives (`lagavulin_sleep_is_parked`,
            // `beetle_snore_is_parked` cite the IL); the intents name them.
            if follow_up == MonsterFollowUp::LagavulinSleep {
                assert_eq!(name, crate::engine::turn::LAGAVULIN_SLEEP_MOVE);
                continue;
            }
            if follow_up == MonsterFollowUp::BeetleSnore {
                assert_eq!(name, "SNORE_MOVE");
                assert_eq!(MonsterOverride::BeetleSnore.as_str(), "SNORE");
                continue;
            }
            if follow_up == MonsterFollowUp::BeetleRollOut {
                let mut builder = crate::catalog::CatalogBuilder::new();
                builder
                    .intern_monster(MonsterKind::SlumberingBeetle)
                    .unwrap();
                let catalog = builder.build();
                let rows = catalog.moves(MonsterKind::SlumberingBeetle);
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].kind, crate::ids::MoveKind::AttackStrength);
                assert!(
                    crate::content_tables::monster_loop(MonsterKind::SlumberingBeetle).is_none()
                );
                continue;
            }
            assert!(
                crate::content_tables::LOOPS
                    .iter()
                    .any(|(_, rows)| rows.iter().any(|row| row.name == name)),
                "{name} names no generated loop row"
            );
        }
    }

    /// #3823: each kind's own unperformed stun drops a Whistle's, and the
    /// drop leaves the monster exactly as it was. Native traces `rck_pb1s2`,
    /// `slg_k1s1`, `bst_hb2s`, `lag_b0st2s`, `bee_b0st3s3`, `tun_br3s3`.
    #[test]
    fn a_whistle_on_a_kinds_own_stun_is_dropped() {
        use MonsterFollowUp as F;
        use MonsterOverride as O;
        let cases = [
            (MonsterKind::BowlbugRock, O::Dizzy, F::None),
            (MonsterKind::CorpseSlug, O::RavenousStun, F::SlugWhipSlap),
            (MonsterKind::CorpseSlug, O::RavenousStun, F::SlugGlomp),
            (MonsterKind::CorpseSlug, O::RavenousStun, F::SlugGoop),
            (MonsterKind::CeremonialBeast, O::BeastStun, F::BeastCry),
            (
                MonsterKind::LagavulinMatriarch,
                O::LagavulinWakeUp,
                F::LagavulinSlash,
            ),
            (MonsterKind::SlumberingBeetle, O::BeetleWake, F::None),
            (MonsterKind::TerrorEel, O::Stunned, F::None),
            (MonsterKind::Tunneler, O::Dizzy, F::None),
        ];
        for (kind, override_state, follow_up) in cases {
            let mut state = stunnable(kind, 0);
            state.monsters_mut()[0].override_state = override_state;
            state.monsters_mut()[0].forced_follow_up = follow_up;
            assert!(own_stun_is_unperformed(&state.monsters[0]), "{kind:?}");
            let before = state.monsters[0].clone();
            assert_eq!(install_generic_stun(&mut state, 0), Ok(()), "{kind:?}");
            assert_eq!(state.monsters[0], before, "{kind:?}");
        }
        // A burrowed Tunneler's DIZZY is not a state the break leaves, and
        // another kind's DIZZY is not its own stun.
        let mut burrowed = stunnable(MonsterKind::Tunneler, 0);
        burrowed.monsters_mut()[0].override_state = O::Dizzy;
        burrowed.monsters_mut()[0]
            .powers
            .set(PowerId::Burrowed, SlotWire::Bool, 1);
        assert!(!own_stun_is_unperformed(&burrowed.monsters[0]));
        let mut slug = stunnable(MonsterKind::CorpseSlug, 0);
        slug.monsters_mut()[0].override_state = O::Dizzy;
        assert!(!own_stun_is_unperformed(&slug.monsters[0]));
        // The Eel's follow-up-free STUNNED with Shriek still standing is no
        // state the Shriek break leaves.
        let mut shrieking = stunnable(MonsterKind::TerrorEel, 0);
        shrieking.monsters_mut()[0].override_state = O::Stunned;
        shrieking.monsters_mut()[0]
            .powers
            .set(PowerId::Shriek, SlotWire::Bool, 1);
        assert!(!own_stun_is_unperformed(&shrieking.monsters[0]));
        assert_eq!(
            install_generic_stun(&mut shrieking, 0),
            Err(EngineRefusal::MalformedArgs("generic stun existing state"))
        );
    }

    /// #3823: a Whistle on the Terror Eel. Over a loop row it parks the row
    /// (never the follow-up-free STUNNED of its Terror chain); over a showing
    /// TERROR_MOVE it writes that chain's own STUNNED, which TERROR_MOVE
    /// follows. Native traces `eel_s2`, `eel_hb2s3`.
    #[test]
    fn a_whistle_on_the_terror_eel_parks_its_row_or_its_terror() {
        for loop_pos in [0, 1] {
            let mut state = stunnable(MonsterKind::TerrorEel, loop_pos);
            assert_eq!(install_generic_stun(&mut state, 0), Ok(()));
            let eel = &state.monsters[0];
            assert_eq!(eel.override_state, MonsterOverride::Stunned);
            assert_ne!(eel.forced_follow_up, MonsterFollowUp::None, "{loop_pos}");
            assert!(generic_stun_parks_this_row(eel), "{loop_pos}");
        }
        let mut terror = stunnable(MonsterKind::TerrorEel, 1);
        terror.monsters_mut()[0].override_state = MonsterOverride::Terror;
        assert_eq!(install_generic_stun(&mut terror, 0), Ok(()));
        assert_eq!(
            (
                terror.monsters[0].override_state,
                terror.monsters[0].forced_follow_up,
                terror.monsters[0].loop_pos
            ),
            (MonsterOverride::Stunned, MonsterFollowUp::None, 1)
        );
        assert!(own_stun_is_unperformed(&terror.monsters[0]));
        // TERROR with Shriek standing is no state the game reaches (Shriek
        // removes itself as it stuns): refused, not overwritten.
        let mut forged = stunnable(MonsterKind::TerrorEel, 0);
        forged.monsters_mut()[0].override_state = MonsterOverride::Terror;
        forged.monsters_mut()[0]
            .powers
            .set(PowerId::Shriek, SlotWire::Bool, 1);
        assert_eq!(
            install_generic_stun(&mut forged, 0),
            Err(EngineRefusal::MalformedArgs("Terror Eel stun state"))
        );
        assert_eq!(forged.monsters[0].override_state, MonsterOverride::Terror);
    }

    /// #3823: a Whistle on the Waterfall Giant's must-perform ABOUT_TO_BLOW is
    /// dropped, one on EXPLODE parks it, and before the death the loop parks
    /// as any other. Native traces `wfg_k2s2`, `wfg_k2s3`, `wfg_s2`.
    #[test]
    fn a_whistle_on_the_waterfall_giant_after_its_death() {
        let mut about = stunnable(MonsterKind::WaterfallGiant, WATERFALL_ABOUT_TO_BLOW_INDEX);
        about.monsters_mut()[0].set_is_about_to_blow(true);
        let before = about.monsters[0].clone();
        assert_eq!(install_generic_stun(&mut about, 0), Ok(()));
        assert_eq!(about.monsters[0], before);
        let mut explode = stunnable(
            MonsterKind::WaterfallGiant,
            WATERFALL_ABOUT_TO_BLOW_INDEX + 1,
        );
        explode.monsters_mut()[0].set_is_about_to_blow(true);
        assert_eq!(install_generic_stun(&mut explode, 0), Ok(()));
        assert!(generic_stun_parks_this_row(&explode.monsters[0]));
        let mut alive = stunnable(MonsterKind::WaterfallGiant, 1);
        assert_eq!(install_generic_stun(&mut alive, 0), Ok(()));
        assert!(generic_stun_parks_this_row(&alive.monsters[0]));
    }

    /// #3823: a stun over a sleeping Lagavulin parks SLEEP_MOVE, a second one
    /// is dropped, and the resumed SLEEP_MOVE (no override) parks again.
    /// Native traces `lag_s1`, `lag_ss2`, `lag_s3s4`.
    #[test]
    fn a_stun_parks_the_lagavulin_sleep_move() {
        let mut state = stunnable(MonsterKind::LagavulinMatriarch, 0);
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Asleep, SlotWire::Int, 2);
        assert_eq!(install_generic_stun(&mut state, 0), Ok(()));
        let parked = (
            state.monsters[0].override_state,
            state.monsters[0].forced_follow_up,
        );
        assert_eq!(
            parked,
            (MonsterOverride::Stunned, MonsterFollowUp::LagavulinSleep)
        );
        assert!(lagavulin_sleep_is_parked(&state.monsters[0]));
        assert!(generic_stun_state_is_exact(&state, 0));
        assert_eq!(install_generic_stun(&mut state, 0), Ok(()), "dropped");
        assert_eq!(state.monsters[0].override_state, MonsterOverride::Stunned);
        // The resumed SLEEP_MOVE after Asleep ran out.
        let mut resumed = stunnable(MonsterKind::LagavulinMatriarch, 0);
        resumed.monsters_mut()[0].forced_follow_up = MonsterFollowUp::LagavulinSleep;
        assert!(lagavulin_sleep_is_parked(&resumed.monsters[0]));
        assert!(!generic_stun_state_is_exact(&resumed, 0), "no stun yet");
        assert_eq!(install_generic_stun(&mut resumed, 0), Ok(()));
        assert_eq!(resumed.monsters[0].override_state, MonsterOverride::Stunned);
        // Off the SLASH row the parked SLEEP_MOVE is not a state.
        resumed.monsters_mut()[0].loop_pos = 1;
        assert!(!lagavulin_sleep_is_parked(&resumed.monsters[0]));
        // Awake, a Whistle parks the loop row like any generic owner.
        let mut awake = stunnable(MonsterKind::LagavulinMatriarch, 1);
        assert_eq!(install_generic_stun(&mut awake, 0), Ok(()));
        assert!(generic_stun_parks_this_row(&awake.monsters[0]));
    }

    /// #3823: the Beetle's stun parks SNORE_MOVE asleep or resumed, ROLL_OUT
    /// awake (also with the Plating a dropped wake left), and drops on its own
    /// WAKE or a standing stun. Native traces `bee_s2`, `bee_s3s4`,
    /// `bee_s3b0st3s4`, `bee_b0st3s3`.
    #[test]
    fn the_beetle_stun_parks_the_move_it_shows() {
        let mut asleep = stunnable(MonsterKind::SlumberingBeetle, 0);
        asleep.monsters_mut()[0].override_state = MonsterOverride::BeetleSnore;
        asleep.monsters_mut()[0]
            .powers
            .set(PowerId::Slumber, SlotWire::Int, 2);
        assert_eq!(install_generic_stun(&mut asleep, 0), Ok(()));
        assert_eq!(
            (
                asleep.monsters[0].override_state,
                asleep.monsters[0].forced_follow_up
            ),
            (MonsterOverride::Stunned, MonsterFollowUp::BeetleSnore)
        );
        assert!(beetle_state_is_valid(&asleep.monsters[0]));
        let before = asleep.monsters[0].clone();
        assert_eq!(install_generic_stun(&mut asleep, 0), Ok(()));
        assert_eq!(asleep.monsters[0], before, "a second stun is dropped");

        let mut resumed = stunnable(MonsterKind::SlumberingBeetle, 0);
        resumed.monsters_mut()[0].forced_follow_up = MonsterFollowUp::BeetleSnore;
        assert!(beetle_state_is_valid(&resumed.monsters[0]));
        assert_eq!(install_imbalanced_self_stun(&mut resumed, 0), Ok(()));
        assert_eq!(
            resumed.monsters[0].forced_follow_up,
            MonsterFollowUp::BeetleSnore
        );

        let mut plated = stunnable(MonsterKind::SlumberingBeetle, 0);
        plated.monsters_mut()[0]
            .powers
            .set(PowerId::Mplating, SlotWire::Int, 16);
        assert!(beetle_state_is_valid(&plated.monsters[0]), "a dropped wake");
        assert_eq!(install_generic_stun(&mut plated, 0), Ok(()));
        assert!(beetle_rollout_stun_is_exact(&plated.monsters[0]));

        let mut malformed = stunnable(MonsterKind::SlumberingBeetle, 0);
        malformed.monsters_mut()[0].override_state = MonsterOverride::BeetleSnore;
        assert_eq!(
            install_generic_stun(&mut malformed, 0),
            Err(EngineRefusal::MalformedArgs("Slumbering Beetle stun state")),
            "SNORE with no Slumber"
        );
        let mut corpse = stunnable(MonsterKind::SlumberingBeetle, 0);
        corpse.monsters_mut()[0].hp = 0;
        assert_eq!(install_generic_stun(&mut corpse, 0), Ok(()));
        assert_eq!(corpse.monsters[0].override_state, MonsterOverride::None);
    }

    /// The bespoke owners keep their own arms; the generic state is not a
    /// second reader of their wire states (#2647 §5.7).
    #[test]
    fn generic_stun_refuses_every_bespoke_owner() {
        for kind in GENERIC_STUN_BESPOKE_KINDS {
            assert!(!generic_stun_owner_is_eligible(kind), "{kind:?}");
        }
        // The Queen roster keeps its own two-kind table's reader.
        let mut state = stunnable(MonsterKind::Queen, 0);
        assert_eq!(
            install_generic_stun(&mut state, 0),
            Err(EngineRefusal::MalformedArgs("generic stun owner"))
        );
        assert_eq!(state.monsters[0].override_state, MonsterOverride::None);
        // Every generated loop that branches conditionally is either a
        // bespoke owner (the Queen) or read from IL and listed (#3823): a
        // new one must be put in one of the two by name.
        let branching: Vec<MonsterKind> = crate::content_tables::LOOPS
            .iter()
            .filter(|(_, rows)| {
                rows.iter().any(|row| {
                    matches!(row.repeats, crate::content_tables::Repeats::Conditional(_))
                })
            })
            .map(|(kind, _)| *kind)
            .collect();
        assert_eq!(
            branching,
            [
                MonsterKind::FrogKnight,
                MonsterKind::KnowledgeDemon,
                MonsterKind::LivingShield,
                MonsterKind::Queen,
            ]
        );
        for kind in branching {
            assert_ne!(
                GENERIC_STUN_BESPOKE_KINDS.contains(&kind),
                GENERIC_STUN_CONDITIONAL_KINDS.contains(&kind),
                "{kind:?}"
            );
        }
    }

    /// The exact set of owners the generic stun admits (#3823).
    ///
    /// Eligibility is **derived** from the generated tables
    /// ([`generic_stun_owner_is_eligible`]): a generated loop, no random
    /// table, no conditional successor, no branch move kind, and not a bespoke
    /// owner. So a table change can widen Whistle's admission wall, the
    /// Imbalanced-recipient gate and `monster_act`'s generic arm at once: the
    /// new-reader-invalidates-old-shortcut class (#1432). This pin is the
    /// tripwire. The native side of every member is the classification table
    /// in `solver/invariant-walks/2026-10-12-issue3823-plain-cycle.md`: each
    /// machine is a cycle of `MoveState`s after its opening roll, with no
    /// second writer of the move.
    /// #3823: each stunned turn ticks Escape Artist without moving the
    /// Hopper's machine, so the reachable pairs are a range below the
    /// unstunned diagonal, one higher on the enemy side before the tick; and
    /// a Whistle's stun reads as the row it parks while the Flutter stun keeps
    /// its own pair.
    #[test]
    fn the_hopper_escape_artist_lags_its_machine_by_the_stunned_turns() {
        for loop_pos in 0..=4 {
            let diagonal = THIEVING_HOPPER_ESCAPE_ARTIST - loop_pos;
            for escape_artist in 0..=6 {
                assert_eq!(
                    hopper_escape_artist_is_reachable(escape_artist, loop_pos, false),
                    (1..=diagonal).contains(&escape_artist),
                    "{loop_pos} {escape_artist}"
                );
                assert_eq!(
                    hopper_escape_artist_is_reachable(escape_artist, loop_pos, true),
                    (1..=diagonal + 1).contains(&escape_artist),
                    "{loop_pos} {escape_artist} pre-tick"
                );
            }
        }
        let mut hopper = HotMonster::new(MonsterKind::ThievingHopper, 40);
        hopper.loop_pos = 2;
        hopper.override_state = MonsterOverride::Stunned;
        hopper.forced_follow_up = MonsterFollowUp::ParkedTelegraph;
        assert_eq!(
            hopper_machine_shape(&hopper),
            (MonsterOverride::None, MonsterFollowUp::None)
        );
        hopper.forced_follow_up = MonsterFollowUp::HopperNab;
        assert_eq!(
            hopper_machine_shape(&hopper),
            (MonsterOverride::Stunned, MonsterFollowUp::HopperNab)
        );
    }

    #[test]
    fn the_generic_stun_owner_set_is_pinned() {
        use MonsterKind::*;
        // Read through the shared predicate rather than re-deriving the
        // conjunction here: #2693 B1 made this same set the gate on which
        // creatures may carry an Imbalanced attachment, and a pin that
        // computed it independently could drift from the gate it is meant to
        // be pinning.
        let eligible: Vec<MonsterKind> = MonsterKind::ALL
            .into_iter()
            .filter(|kind| generic_stun_owner_loop_is_fully_resolvable(*kind))
            .collect();
        assert_eq!(
            eligible,
            vec![
                Aeonglass,
                AssassinRubyRaider,
                Axebot,
                AxeRubyRaider,
                BattleFriendV1,
                BattleFriendV2,
                BattleFriendV3,
                BowlbugEgg,
                BowlbugNectar,
                BowlbugRock,
                BowlbugSilk,
                BruteRubyRaider,
                BygoneEffigy,
                Byrdonis,
                CalcifiedCultist,
                CeremonialBeast,
                Chomper,
                CorpseSlug,
                CrossbowRubyRaider,
                Crusher,
                CubexConstruct,
                DampCultist,
                DecimillipedeSegment,
                DevotedSculptor,
                Entomancer,
                Exoskeleton,
                EyeWithTeeth,
                Fabricator,
                FakeMerchantMonster,
                FatGremlin,
                FlailKnight,
                Flyconid,
                Fogmog,
                FossilStalker,
                FrogKnight,
                FuzzyWurmCrawler,
                GasBomb,
                GlobeHead,
                GremlinMerc,
                Guardbot,
                HauntedShip,
                HunterKiller,
                InfestedPrism,
                Inklet,
                KinFollower,
                KinPriest,
                KnowledgeDemon,
                LagavulinMatriarch,
                LeafSlimeM,
                LeafSlimeS,
                LivingFog,
                LivingShield,
                LouseProgenitor,
                MagiKnight,
                Mawler,
                MechaKnight,
                MysteriousKnight,
                Myte,
                Nibbit,
                Noisebot,
                Ovicopter,
                OwlMagistrate,
                Parafright,
                PhantasmalGardener,
                PhrogParasite,
                PunchConstruct,
                Rocket,
                ScrollOfBiting,
                Seapunk,
                SewerClam,
                ShrinkerBeetle,
                SkulkingColony,
                SlimedBerserker,
                SlitheringStrangler,
                SludgeSpinner,
                SnappingJaxfruit,
                SneakyGremlin,
                SoulFysh,
                SoulNexus,
                SpectralKnight,
                SpinyToad,
                Stabbot,
                TerrorEel,
                TestSubject,
                TheForgotten,
                TheInsatiable,
                TheLost,
                TheObscura,
                ThievingHopper,
                Toadpole,
                ToughEgg,
                TrackerRubyRaider,
                Tunneler,
                TurretOperator,
                TwigSlimeM,
                TwigSlimeS,
                TwoTailedRat,
                Vantom,
                VineShambler,
                WaterfallGiant,
                Wriggler,
                Zapbot,
            ],
            "The set of owners the generic stun admits has changed.\n\
             This is an ADMISSION WIDENING, not a refactor: every kind in this \
             set becomes reachable by `stun_target`, by `valid_override`'s \
             generic disjunct, by the Imbalanced-recipient gate and by \
             `monster_act`'s generic arm. A new member owes the native reading \
             of its machine (a cycle of `MoveState`s after the opening roll, \
             or a random table whose log readers accept a move logged twice; \
             no `SetMoveImmediate`/`ForceCurrentState`/`Stun` writer, no \
             must-perform state), a row in the stun sweep witness and its \
             native traces in `engine::native_stun_traces`."
        );
        // Every eligible kind resolves at every row. A random table (#3823)
        // has one answer, at `loop_pos` 0: the row `random_ai.next` names.
        for kind in eligible {
            let Some(rows) = crate::content_tables::monster_loop(kind) else {
                assert!(GENERIC_STUN_RANDOM_TABLE_KINDS.contains(&kind), "{kind:?}");
                assert_eq!(
                    parked_telegraph(kind, 0),
                    Some(MonsterFollowUp::ParkedTelegraph)
                );
                assert_eq!(parked_telegraph(kind, 1), None);
                assert_eq!(parked_telegraph(kind, -1), None);
                continue;
            };
            for index in 0..rows.len() as i32 {
                assert!(parked_telegraph(kind, index).is_some(), "{kind:?} {index}");
            }
            assert_eq!(parked_telegraph(kind, rows.len() as i32), None);
            assert_eq!(parked_telegraph(kind, -1), None);
        }
    }

    /// Which loops keep a named follow-up on the wire (#3823).
    ///
    /// A loop whose every row is in the named vocabulary parks the row's
    /// name, as it did before the generic token existed, so no document
    /// written before this change reads differently. Every other loop parks
    /// [`MonsterFollowUp::ParkedTelegraph`]. Adding a name to the vocabulary
    /// can move a kind from the token to its names, which would change that
    /// kind's wire spelling; this pin is where that shows.
    #[test]
    fn the_named_parked_telegraph_kinds_are_pinned() {
        let named: Vec<MonsterKind> = crate::content_tables::LOOPS
            .iter()
            .filter(|(_, rows)| loop_rows_are_all_named(rows))
            .map(|(kind, _)| *kind)
            .collect();
        assert_eq!(
            named,
            vec![
                MonsterKind::BowlbugEgg,
                MonsterKind::BowlbugNectar,
                MonsterKind::BowlbugSilk,
                // Bespoke (the Ravenous stun restores by name), so never a
                // generic owner; listed because its three rows are named.
                MonsterKind::CorpseSlug,
                MonsterKind::MechaKnight,
                MonsterKind::OwlMagistrate,
                MonsterKind::Queen,
                MonsterKind::TorchHeadAmalgam,
            ]
        );
        // No generated row is spelled like the token, so the token can never
        // be read as a row name.
        assert!(
            crate::content_tables::LOOPS
                .iter()
                .all(|(_, rows)| rows.iter().all(|row| row.name != "PARKED_TELEGRAPH"))
        );
        // A token loop: the same answer at every row, and none out of range.
        assert_eq!(
            parked_telegraph(MonsterKind::Toadpole, 0),
            Some(MonsterFollowUp::ParkedTelegraph)
        );
        assert_eq!(
            parked_telegraph(MonsterKind::Toadpole, 2),
            Some(MonsterFollowUp::ParkedTelegraph)
        );
        assert_eq!(parked_telegraph(MonsterKind::Toadpole, 3), None);
    }

    /// #2647 open question 6 / §5.10, re-derived for the widened carrier set.
    ///
    /// Slice A's argument that listener order on `Hook.AfterDamageGiven` is
    /// unobservable leaned on "no admitted roster puts both monster-ownable
    /// listeners on one creature". #2693 B1 widens the Imbalanced carrier
    /// set, so that clause had to be re-established rather than inherited.
    ///
    /// It survives, and this is the checkable form of why: `PaperCutsPower`
    /// is `SCROLL_OF_BITING`'s `initial_powers` and nothing else's, carried
    /// **by kind** with no `PowerId` and therefore no wire field a document
    /// could use to put it on another creature — and Scroll of Biting is not
    /// an Imbalanced carrier. The carrier set is
    /// [`IMBALANCED_GENERIC_CARRIER_KINDS`], its own list since #3823 and no
    /// longer the generic stun owner set: Scroll of Biting takes Whistle's
    /// stun (a player-side `CreatureCmd::Stun`, which is no
    /// `AfterDamageGiven` listener) and must still never carry Imbalanced.
    ///
    /// This lives in `monsters.rs` rather than beside the listener because
    /// `engine/damage.rs` is a hot module, and reading a generated content
    /// string there would trip the D2 text-dispatch contract.
    #[test]
    fn no_creature_can_carry_both_monster_owned_after_damage_given_listeners() {
        let paper_cuts_owners: Vec<MonsterKind> = MonsterKind::ALL
            .into_iter()
            .filter(|kind| {
                crate::content_tables::MONSTER_MODELS
                    .get(*kind as usize)
                    .is_some_and(|model| {
                        model
                            .initial_powers
                            .iter()
                            .any(|(name, _)| *name == "PaperCutsPower")
                    })
            })
            .collect();
        assert_eq!(paper_cuts_owners, vec![MonsterKind::ScrollOfBiting]);

        assert_ne!(MonsterKind::ScrollOfBiting, MonsterKind::BowlbugRock);
        assert!(
            !IMBALANCED_GENERIC_CARRIER_KINDS.contains(&MonsterKind::ScrollOfBiting),
            "Scroll of Biting became an eligible Imbalanced carrier: #2647 \
             open question 6 reopens, because one creature could then own two \
             AfterDamageGiven listeners and their order would be observable.",
        );
    }

    /// The carrier set is exactly "Bowlbug Rock, or a ledger attachment".
    ///
    /// Replaces slice A's `owner_carries_imbalanced_implies_bowlbug`, which
    /// pinned the listener's non-Bowlbug arm as *unreachable*. #2693 B1 makes
    /// it reachable, so the pin becomes the statement that the only two ways
    /// in are the ones above — still by total enumeration over every
    /// `MonsterKind`, so a kind that silently acquired the power by some
    /// third route would fail here.
    #[test]
    fn owner_carries_imbalanced_is_bowlbug_or_an_attachment() {
        let mut by_kind = 0;
        for kind in MonsterKind::ALL {
            let bare = HotMonster::new(kind, 10);
            if owner_carries_imbalanced(&bare) {
                assert_eq!(kind, MonsterKind::BowlbugRock, "{kind:?} carries by kind");
                by_kind += 1;
            }

            // The attachment route is open to every kind at the ledger level
            // — admission, not this predicate, is what narrows it to a
            // representable carrier.
            let mut attached = HotMonster::new(kind, 10);
            attached
                .misery_debuff_order
                .push_attachment(crate::hot::AttachmentRecord {
                    power: crate::hot::AttachedPowerModel::Imbalanced,
                    applier: crate::hot::Applier::Monster(1),
                    amount: 1,
                });
            assert!(owner_carries_imbalanced(&attached), "{kind:?} attached");

            // A different model in the ledger is not this power. Without this
            // the predicate could be reading "any attachment at all" and
            // every assertion above would still pass.
            let mut other = HotMonster::new(kind, 10);
            other
                .misery_debuff_order
                .push_attachment(crate::hot::AttachmentRecord {
                    power: crate::hot::AttachedPowerModel::Strength,
                    applier: crate::hot::Applier::Player,
                    amount: -3,
                });
            assert_eq!(
                owner_carries_imbalanced(&other),
                kind == MonsterKind::BowlbugRock,
                "{kind:?} with a non-Imbalanced attachment",
            );
        }
        assert_eq!(by_kind, 1);
    }

    /// `StunInternal` `0x11d7cc` IL_0057–IL_0078 plus `SetMoveImmediate`
    /// `0x825f0` IL_000c–IL_001c: the unperformed dynamic state cannot
    /// transition away and `force` is false, so a second stun is dropped —
    /// silently, with no refusal and no state change (#2647 §1.4.2 / §5.6).
    #[test]
    fn second_generic_stun_before_the_first_performs_is_a_silent_noop() {
        let mut state = stunnable(MonsterKind::BowlbugNectar, 1);
        install_generic_stun(&mut state, 0).unwrap();
        assert_eq!(state.monsters[0].override_state, MonsterOverride::Stunned);
        assert_eq!(
            state.monsters[0].forced_follow_up,
            MonsterFollowUp::NectarBuff
        );
        assert_eq!(state.monsters[0].loop_pos, 1);
        let before = state.clone();
        install_generic_stun(&mut state, 0).unwrap();
        assert_eq!(state, before);
    }

    /// A stun landing over an override that is not this one refuses rather than
    /// overwriting a state with a different reader (#2647 §5.6).
    #[test]
    fn generic_stun_over_a_foreign_override_refuses() {
        for (override_state, follow_up) in [
            (MonsterOverride::Dizzy, MonsterFollowUp::None),
            (MonsterOverride::Terror, MonsterFollowUp::None),
            (MonsterOverride::None, MonsterFollowUp::EggBite),
        ] {
            let mut state = stunnable(MonsterKind::BowlbugEgg, 0);
            state.monsters_mut()[0].override_state = override_state;
            state.monsters_mut()[0].forced_follow_up = follow_up;
            let before = state.clone();
            assert_eq!(
                install_generic_stun(&mut state, 0),
                Err(EngineRefusal::MalformedArgs(
                    "generic stun foreign override"
                ))
            );
            assert_eq!(state, before);
        }
        // A STUNNED state that is not the generic shape belongs to another
        // reader, so this stun may not claim the silent no-op over it either.
        let mut state = stunnable(MonsterKind::BowlbugEgg, 0);
        state.monsters_mut()[0].override_state = MonsterOverride::Stunned;
        state.monsters_mut()[0].forced_follow_up = MonsterFollowUp::QueenEnrage;
        assert_eq!(
            install_generic_stun(&mut state, 0),
            Err(EngineRefusal::MalformedArgs("generic stun existing state"))
        );
    }

    /// `StunInternal` IL_001f (no combat state) and IL_0028 (corpse) both
    /// return silently.
    #[test]
    fn generic_stun_on_a_dead_absent_or_finished_owner_is_a_noop() {
        let mut dead = stunnable(MonsterKind::BowlbugEgg, 0);
        dead.monsters_mut()[0].hp = 0;
        let before = dead.clone();
        install_generic_stun(&mut dead, 0).unwrap();
        assert_eq!(dead, before);

        let mut over = stunnable(MonsterKind::BowlbugEgg, 0);
        over.history.over = true;
        let before = over.clone();
        install_generic_stun(&mut over, 0).unwrap();
        assert_eq!(over, before);

        let mut absent = stunnable(MonsterKind::BowlbugEgg, 0);
        let before = absent.clone();
        install_generic_stun(&mut absent, 7).unwrap();
        assert_eq!(absent, before);
    }

    /// An eligible owner whose `loop_pos` names no unique telegraph refuses
    /// rather than guessing one (#2647 §5.5).
    #[test]
    fn generic_stun_refuses_an_owner_with_no_unique_telegraph() {
        let mut state = stunnable(MonsterKind::BowlbugEgg, 4);
        let before = state.clone();
        assert_eq!(
            install_generic_stun(&mut state, 0),
            Err(EngineRefusal::MalformedArgs("generic stun target machine"))
        );
        assert_eq!(state, before);
    }

    /// #3823 / #3209: each reason Whistle's stun is not representable has its
    /// own name, and a branch move kind is ineligible whatever the vocabulary
    /// holds.
    #[test]
    fn whistle_stun_refusals_are_named_by_reason() {
        let refusal = |kind: MonsterKind, edit: &dyn Fn(&mut HotMonster)| {
            let mut builder = crate::catalog::CatalogBuilder::new();
            builder.intern_monster(kind).unwrap();
            let catalog = builder.build();
            let mut state = HotState::at_defaults();
            let mut monster = HotMonster::new(kind, 40);
            edit(&mut monster);
            state.monsters_mut().push(monster);
            whistle_stun_target_refusal(&state, &catalog, &state.monsters[0])
        };
        let plain = |_: &mut HotMonster| {};
        for kind in [MonsterKind::OwlMagistrate, MonsterKind::MechaKnight] {
            assert_eq!(refusal(kind, &plain), None, "{kind:?}");
            assert_eq!(
                refusal(kind, &|monster| monster.revive_stage = 1),
                Some(WHISTLE_STUN_INSTANCE),
                "{kind:?}"
            );
            // #3823: `spawn_noop` is the SPAWNED_MOVE telegraph and parks
            // like any other. (On a kind that cannot carry it, admission's
            // own `spawn_noop owner` wall is what refuses.)
            assert_eq!(refusal(kind, &|monster| monster.spawn_noop = true), None);
            // A corpse is a silent no-op, never a state to represent.
            assert_eq!(refusal(kind, &|monster| monster.hp = 0), None);
        }
        // #3823: every bespoke kind but the Queen roster (named by its own
        // arm first) takes the generic stun or its own, so no kind is named
        // as a bespoke machine any more.
        for kind in [MonsterKind::TerrorEel, MonsterKind::DecimillipedeSegment] {
            assert_eq!(refusal(kind, &plain), None, "{kind:?}");
        }
        // A dead segment (DEAD, REATTACH, or killed under a stun) is a
        // corpse to `StunInternal`.
        for stage in [
            SEGMENT_REVIVE_DEAD,
            SEGMENT_REVIVE_REATTACH,
            SEGMENT_REVIVE_STUN_KEPT,
        ] {
            assert_eq!(
                refusal(MonsterKind::DecimillipedeSegment, &|monster| {
                    monster.hp = 0;
                    monster.revive_stage = stage;
                }),
                None,
                "{stage}"
            );
        }
        assert!(
            MonsterKind::ALL
                .into_iter()
                .filter(|kind| GENERIC_STUN_BESPOKE_KINDS.contains(kind))
                .all(|kind| matches!(kind, MonsterKind::Queen | MonsterKind::TorchHeadAmalgam))
        );
        // #3823: every random table and every conditional successor is a
        // generic owner now. What is left outside both lists is a kind with
        // no generated loop at all (its table is the private native loop).
        let branching: Vec<MonsterKind> = MonsterKind::ALL
            .into_iter()
            .filter(|kind| {
                !GENERIC_STUN_BESPOKE_KINDS.contains(kind) && !generic_stun_owner_is_eligible(*kind)
            })
            .collect();
        for kind in &branching {
            assert!(
                crate::content_tables::random_moves(*kind).is_none()
                    && crate::content_tables::monster_loop(*kind).is_none(),
                "{kind:?}"
            );
        }
        // #3823: the Slumbering Beetle, whose table is its private ROLL_OUT
        // row, takes its own stun (`install_beetle_stun`), so no refusal.
        assert_eq!(branching, vec![MonsterKind::SlumberingBeetle]);
        for kind in branching {
            assert_eq!(refusal(kind, &plain), None, "{kind:?}");
        }
        // #3823: the Obscura's RAND and Fogmog's in-row BRANCH read no log a
        // stun's duplicate can move, and their illusions take the stun too.
        for kind in [
            MonsterKind::TheObscura,
            MonsterKind::Fogmog,
            MonsterKind::Parafright,
            MonsterKind::EyeWithTeeth,
        ] {
            assert!(generic_stun_owner_is_eligible(kind), "{kind:?}");
        }
        for kind in [
            MonsterKind::Fogmog,
            MonsterKind::Parafright,
            MonsterKind::EyeWithTeeth,
        ] {
            assert_eq!(refusal(kind, &plain), None, "{kind:?}");
            // A corpse waiting on REVIVE_MOVE is a silent no-op.
            assert_eq!(
                refusal(kind, &|monster| {
                    monster.hp = 0;
                    monster.revive_stage = ILLUSION_REVIVING;
                }),
                None,
                "{kind:?}"
            );
        }
        // #3823: Living Shield's conditional successor is read by the roll
        // after SHIELD_SLAM performs, never by a stun, so it is a generic
        // owner (`GENERIC_STUN_CONDITIONAL_KINDS`).
        assert!(generic_stun_owner_is_eligible(MonsterKind::LivingShield));
        assert!(generic_stun_owner_is_eligible(MonsterKind::FrogKnight));
        assert!(generic_stun_owner_is_eligible(MonsterKind::KnowledgeDemon));
        // The demon's curse counter shares the `revive_stage` lane and is not
        // a row: a Whistle is representable at every counter.
        for counter in 0..=3 {
            assert_eq!(
                refusal(MonsterKind::KnowledgeDemon, &|monster| {
                    assert!(monster.set_knowledge_demon_curse_counter(counter));
                    monster.loop_pos = i32::from(counter.min(1));
                }),
                None,
                "counter {counter}"
            );
        }
        assert_eq!(refusal(MonsterKind::FrogKnight, &plain), None);
        assert_eq!(refusal(MonsterKind::LivingShield, &plain), None);
        // #3823: Scroll of Biting's CHEW -> `rand` is a branch move kind the
        // generic stun does cover (the resume sets `scroll_chew_repeated`).
        assert!(generic_stun_owner_is_eligible(MonsterKind::ScrollOfBiting));
        assert_eq!(refusal(MonsterKind::ScrollOfBiting, &plain), None);
        // #3823: a plain loop needs no vocabulary. Turret Operator
        // (`GenerateMoveStateMachine` RVA `0xc3ab8` is three plain
        // `MoveState`s) is representable by itself; the roots that hold it
        // still refuse on the Living Shield beside it.
        assert!(generic_stun_owner_is_eligible(MonsterKind::TurretOperator));
        assert_eq!(refusal(MonsterKind::TurretOperator, &plain), None);
        // #3823: the Ovicopter's deferred conditional rides inside a move
        // kind, read only after TENDERIZER performs; the stun covers it.
        assert_eq!(refusal(MonsterKind::Ovicopter, &plain), None);
        // #3823: the Tough Egg's `ForceCurrentState` is only for an egg
        // added already hatched, which no fight does.
        assert_eq!(refusal(MonsterKind::ToughEgg, &plain), None);
        // #3823: a death writer of the move with `force: true` replaces a
        // standing stun, and its must-perform state drops one (the Waterfall
        // Giant's ABOUT_TO_BLOW, the Test Subject's corpse-shown RESPAWN).
        for kind in [MonsterKind::WaterfallGiant, MonsterKind::TestSubject] {
            assert_eq!(refusal(kind, &plain), None, "{kind:?}");
        }
        assert_eq!(
            refusal(MonsterKind::Queen, &plain),
            Some(WHISTLE_QUEEN_AMALGAM)
        );
    }

    /// The exactness twin refuses a corpse and a mismatched parked successor,
    /// which is what makes a hydrated document's state checkable.
    #[test]
    fn generic_stun_state_is_exact_pins_liveness_and_the_parked_row() {
        let mut state = stunnable(MonsterKind::BowlbugNectar, 2);
        install_generic_stun(&mut state, 0).unwrap();
        assert!(generic_stun_state_is_exact(&state, 0));
        assert_eq!(
            state.monsters[0].forced_follow_up,
            MonsterFollowUp::NectarThrashTwo
        );

        let mut moved = state.clone();
        moved.monsters_mut()[0].loop_pos = 0;
        assert!(!generic_stun_state_is_exact(&moved, 0));

        let mut corpse = state.clone();
        corpse.monsters_mut()[0].hp = 0;
        assert!(!generic_stun_state_is_exact(&corpse, 0));

        assert!(!generic_stun_state_is_exact(&state, 1));
    }

    // ---- #2539: every roster pin and spawn follows the fight's ascension ----

    fn at_ascension(ascension: u8) -> HotState {
        let mut state = HotState::at_defaults();
        assert!(state.fanouts.set_ascension(ascension));
        state
    }

    fn singleton(ascension: u8, kind: MonsterKind, max_hp: i32) -> HotState {
        let mut state = at_ascension(ascension);
        let mut owner = HotMonster::new(kind, max_hp);
        owner.max_hp = max_hp;
        state.monsters_mut().push(owner);
        state
    }

    /// The A10 fixture constants tests build states with are exactly
    /// `MONSTER_MODELS`' at-or-above tier, and differ from its below tier —
    /// so a fixture can never pass a validator at a tier it does not model.
    #[test]
    fn a10_fixture_hps_are_the_modeled_tier() {
        use crate::encounters::{MODELED_ASCENSION, fixed_hp};
        for (value, kind) in [
            (THE_LOST_HP, MonsterKind::TheLost),
            (THE_FORGOTTEN_HP, MonsterKind::TheForgotten),
            (SOUL_FYSH_HP, MonsterKind::SoulFysh),
            (ENTOMANCER_HP, MonsterKind::Entomancer),
            (TORCH_HEAD_AMALGAM_HP, MonsterKind::TorchHeadAmalgam),
            (QUEEN_HP, MonsterKind::Queen),
            (THIEVING_HOPPER_HP, MonsterKind::ThievingHopper),
            (AEONGLASS_HP, MonsterKind::Aeonglass),
            (THE_INSATIABLE_HP, MonsterKind::TheInsatiable),
        ] {
            assert_eq!(fixed_hp(kind, MODELED_ASCENSION), Some(value), "{kind:?}");
            assert_ne!(fixed_hp(kind, 7), Some(value), "{kind:?} is tiered");
        }
        assert_eq!(
            (
                TEST_SUBJECT_FIRST_HP,
                TEST_SUBJECT_SECOND_HP,
                TEST_SUBJECT_THIRD_HP,
                TEST_SUBJECT_ENRAGE
            ),
            (111, 212, 313, 3)
        );
        assert_eq!(
            i64::from(crate::engine::admission::VANTOM_SLIPPERY),
            crate::encounters::initial_power(
                MonsterKind::Vantom,
                "SlipperyPower",
                MODELED_ASCENSION
            )
            .unwrap()
        );
        assert_eq!(
            i64::from(crate::engine::admission::CORPSE_SLUG_RAVENOUS),
            crate::encounters::initial_power(
                MonsterKind::CorpseSlug,
                "RavenousPower",
                MODELED_ASCENSION
            )
            .unwrap()
        );
    }

    /// #2852: the Axebot, Sewer Clam and Test Subject numbers this file used to
    /// carry by hand are now read from `MONSTER_MODELS`. The literals below
    /// are those former constants; the routed values must equal them at every
    /// ascension, so the reroute is pinned as behaviour-neutral rather than
    /// asserted. A generator change that moves a row fails here.
    #[test]
    fn model_routed_numbers_equal_the_former_hand_read_constants() {
        use crate::encounters::{fixed_hp, hp_band, initial_power, tier};
        const fn former(gate: u8, at_or_above: i64, below: i64) -> AscensionTier {
            AscensionTier {
                gate: Some(gate),
                at_or_above,
                below,
            }
        }
        let axebot_min = former(8, 76, 70);
        let axebot_max = former(8, 86, 78);
        let sewer_clam_plating = former(8, 9, 8);
        let test_subject_forms = [
            former(8, 111, 100),
            former(8, 212, 200),
            former(8, 313, 300),
        ];
        let test_subject_enrage_amount = former(9, 3, 2);

        // The engine distinguishes ascensions only through `tier`'s gate
        // comparison, so the whole `u8` domain is swept: both sides of every
        // gate, and everything past the top of the ladder. This compares the
        // tier-selected values the engine consumes, through the constants.
        for ascension in 0..=u8::MAX {
            let at = |value: AscensionTier| tier(value, ascension) as i32;
            assert_eq!(
                (at(AXEBOT_MIN_HP), at(AXEBOT_MAX_HP)),
                (at(axebot_min), at(axebot_max)),
                "A{ascension}"
            );
            assert_eq!(
                hp_band(MonsterKind::Axebot, ascension).unwrap(),
                (at(axebot_min), at(axebot_max)),
                "A{ascension}"
            );
            assert_eq!(
                at(SEWER_CLAM_PLATING),
                at(sewer_clam_plating),
                "A{ascension}"
            );
            assert_eq!(
                initial_power(MonsterKind::SewerClam, "PlatingPower", ascension).unwrap(),
                i64::from(at(sewer_clam_plating)),
                "A{ascension}"
            );
            for (form, former_hp) in test_subject_forms.iter().enumerate() {
                assert_eq!(
                    at(TEST_SUBJECT_FORM_HP[form]),
                    at(*former_hp),
                    "A{ascension} form {form}"
                );
            }
            assert_eq!(
                fixed_hp(MonsterKind::TestSubject, ascension),
                Some(at(test_subject_forms[0])),
                "A{ascension}: the row's band is the single first-form HP"
            );
            assert_eq!(
                at(TEST_SUBJECT_ENRAGE_AMOUNT),
                at(test_subject_enrage_amount),
                "A{ascension}"
            );
            assert_eq!(
                initial_power(MonsterKind::TestSubject, "EnragePower", ascension).unwrap(),
                i64::from(at(test_subject_enrage_amount)),
                "A{ascension}"
            );
        }

        // The same through the accessors the validators and transitions call,
        // at each ascension a fight can be set to, on either side of both
        // gates (8 for HP and Plating, 9 for Enrage).
        for ascension in [0, 7, 8, 9, 10] {
            let state = at_ascension(ascension);
            let at = |value: AscensionTier| tier(value, ascension) as i32;
            assert_eq!(
                monster_plating_cap(&state, MonsterKind::SewerClam),
                Some(at(sewer_clam_plating)),
                "A{ascension}"
            );
            for (form, former_hp) in test_subject_forms.iter().enumerate() {
                assert_eq!(
                    test_subject_form_hp(&state, form),
                    Some(at(*former_hp)),
                    "A{ascension} form {form}"
                );
            }
            assert_eq!(
                test_subject_enrage(&state),
                at(test_subject_enrage_amount),
                "A{ascension}"
            );
        }
        assert_eq!(AXEBOT_RESPAWN_HP_BONUS, 10);
        assert_eq!(test_subject_form_hp(&HotState::at_defaults(), 3), None);
    }

    /// Soul Fysh, the issue's own example (`GetValueIfAscension(8, 221, 211)`),
    /// and the other fixed singletons: each pin admits exactly its tier.
    #[test]
    fn a_fixed_hp_pin_admits_only_the_fights_tier() {
        type Validator = fn(&HotState) -> bool;
        type Dress = fn(&mut HotMonster);
        let cases: [(MonsterKind, i32, i32, Validator, Dress); 3] = [
            (
                MonsterKind::SoulFysh,
                221,
                211,
                soul_fysh_state_is_valid,
                |_| {},
            ),
            (
                MonsterKind::Entomancer,
                165,
                145,
                entomancer_state_is_valid,
                |owner| owner.powers.set(PowerId::Hive, SlotWire::Int, 1),
            ),
            (
                MonsterKind::TheInsatiable,
                341,
                321,
                insatiable_state_is_valid,
                |_| {},
            ),
        ];
        for (kind, above, below, valid, dress) in cases {
            for (ascension, max_hp, admitted) in [
                (7, below, true),
                (7, above, false),
                (8, above, true),
                (8, below, false),
                (10, above, true),
                (10, below, false),
            ] {
                let mut state = singleton(ascension, kind, max_hp);
                dress(&mut state.monsters_mut()[0]);
                assert_eq!(
                    valid(&state),
                    admitted,
                    "{kind:?} max_hp {max_hp} at A{ascension}"
                );
            }
        }
    }

    #[test]
    fn the_kaiser_pair_and_turret_pair_are_pinned_at_the_fights_tier() {
        for (ascension, pair, admitted) in [
            (7, (209, 199), true),
            (7, (219, 209), false),
            (8, (219, 209), true),
            (8, (209, 199), false),
        ] {
            let mut state = at_ascension(ascension);
            for (index, (kind, hp)) in [
                (MonsterKind::Crusher, pair.0),
                (MonsterKind::Rocket, pair.1),
            ]
            .into_iter()
            .enumerate()
            {
                let mut crab = HotMonster::new(kind, hp);
                crab.max_hp = hp;
                crab.slot = index as i32;
                crab.uid = index as u32;
                crab.set_crab_rage(true);
                state.monsters_mut().push(crab);
            }
            assert!(state.fanouts.set_kaiser_facing(0));
            assert_eq!(
                kaiser_roster_is_valid(&state, None),
                admitted,
                "{pair:?} at A{ascension}"
            );
        }
        for (ascension, shield_hp, operator_hp, admitted) in
            [(7, 55, 41, true), (7, 65, 51, false), (8, 65, 51, true)]
        {
            let mut state = at_ascension(ascension);
            let mut shield = HotMonster::new(MonsterKind::LivingShield, shield_hp);
            shield.max_hp = shield_hp;
            shield.powers.set(PowerId::Rampart, SlotWire::Int, 25);
            let mut operator = HotMonster::new(MonsterKind::TurretOperator, operator_hp);
            operator.max_hp = operator_hp;
            operator.slot = 1;
            operator.uid = 1;
            state.monsters_mut().push(shield);
            state.monsters_mut().push(operator);
            assert_eq!(
                turret_operator_state_is_valid(&state),
                admitted,
                "Turret Operator at A{ascension}"
            );
        }
    }

    /// Test Subject's form HPs and Enrage are read from IL because the
    /// manifest row is refused: first form `(8, 111, 100)`, Enrage
    /// `(9, 3, 2)`. At A8 the HP is the upper tier but Enrage the lower.
    #[test]
    fn test_subject_forms_and_enrage_follow_their_own_gates() {
        for (ascension, hp, enrage) in [(7, 100, 2), (8, 111, 2), (9, 111, 3)] {
            let mut state = test_subject_state(0, false);
            assert!(state.fanouts.set_ascension(ascension));
            let owner = &mut state.monsters_mut()[0];
            owner.hp = hp;
            owner.max_hp = hp;
            owner.powers.set(PowerId::Enrage, SlotWire::Int, enrage);
            assert!(test_subject_state_is_valid(&state), "A{ascension}");
            assert_eq!(
                test_subject_form_hp(&state, 1),
                Some(if ascension >= 8 { 212 } else { 200 })
            );
            assert_eq!(
                test_subject_form_hp(&state, 2),
                Some(if ascension >= 8 { 313 } else { 300 })
            );
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Enrage, SlotWire::Int, enrage + 1);
            assert!(
                !test_subject_state_is_valid(&state),
                "wrong Enrage at A{ascension}"
            );
        }
    }

    /// Each Plating owner's ceiling is its entry amount at the fight's tier:
    /// Frog Knight `(8, 19, 15)`, Slumbering Beetle `(8, 18, 15)`, Sewer Clam
    /// `(8, 9, 8)` (its model row, #2852), and the untiered Lagavulin 12 / Mysterious 6.
    #[test]
    fn plating_caps_and_the_frog_knights_countdown_follow_the_tier() {
        let below = at_ascension(7);
        let above = at_ascension(8);
        for (kind, low, high) in [
            (MonsterKind::FrogKnight, 15, 19),
            (MonsterKind::SlumberingBeetle, 15, 18),
            (MonsterKind::SewerClam, 8, 9),
            (MonsterKind::LagavulinMatriarch, 12, 12),
            (MonsterKind::MysteriousKnight, 6, 6),
        ] {
            assert_eq!(monster_plating_cap(&below, kind), Some(low), "{kind:?}");
            assert_eq!(monster_plating_cap(&above, kind), Some(high), "{kind:?}");
        }
        assert_eq!(monster_plating_cap(&above, MonsterKind::Toadpole), None);

        for (ascension, hp, plating, admitted) in [
            (7, 191, 15, true),
            (7, 199, 19, false),
            (7, 191, 19, false),
            (8, 199, 19, true),
        ] {
            let mut state = singleton(ascension, MonsterKind::FrogKnight, hp);
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Mplating, SlotWire::Int, plating);
            assert_eq!(
                frog_knight_state_is_valid(&state),
                admitted,
                "Frog Knight {hp}/{plating} at A{ascension}"
            );
        }

        // #3823: the Frog Knight's machine may carry exactly the generic
        // stun, on any of its four rows; any other override, or a stun over
        // a follow-up the row does not name, is still refused.
        for loop_pos in 0..4 {
            let mut state = singleton(8, MonsterKind::FrogKnight, 199);
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Mplating, SlotWire::Int, 19);
            state.monsters_mut()[0].loop_pos = loop_pos;
            assert!(frog_knight_state_is_valid(&state), "row {loop_pos}");
            state.monsters_mut()[0].override_state = MonsterOverride::Stunned;
            assert!(!frog_knight_state_is_valid(&state), "row {loop_pos}");
            state.monsters_mut()[0].forced_follow_up = MonsterFollowUp::ParkedTelegraph;
            assert!(frog_knight_state_is_valid(&state), "row {loop_pos}");
            state.monsters_mut()[0].override_state = MonsterOverride::None;
            assert!(!frog_knight_state_is_valid(&state), "row {loop_pos}");
        }
    }

    #[test]
    fn mid_fight_spawns_roll_the_fights_bands() {
        // Gas Bomb `(8, 8, 7)`.
        for (ascension, hp) in [(7, 7), (8, 8)] {
            let mut fog = at_ascension(ascension);
            let mut owner = HotMonster::new(MonsterKind::LivingFog, 80);
            owner.max_hp = 80;
            owner.slot = 5;
            fog.monsters_mut().push(owner);
            spawn_gas_bomb(&mut fog, 0).unwrap();
            assert_eq!(fog.monsters[0].max_hp, hp, "Gas Bomb at A{ascension}");
        }
        // Two-Tailed Rat `(8, 18..22, 17..21)`: the draw is over the band, so
        // a band's top value only appears at its own tier.
        let mut seen = std::collections::BTreeSet::new();
        for seed in 0..40 {
            let mut state = rat_state();
            assert!(state.fanouts.set_ascension(7));
            state.rng.set(
                RngStream::Niche,
                RngStreamState {
                    words: Xoshiro256StarStar::from_seed(seed).words,
                    counter: 0,
                },
            );
            spawn_rat(&mut state, 0).unwrap();
            seen.insert(state.monsters.last().unwrap().max_hp);
        }
        assert!(seen.iter().all(|hp| (17..=21).contains(hp)), "{seen:?}");
        // Tough Egg `(8, 15..19, 14..18)` and its hatchling `(8, 20..23, 19..22)`.
        let mut eggs = std::collections::BTreeSet::new();
        let mut hatchlings = std::collections::BTreeSet::new();
        for seed in 0..40 {
            let mut state = ovicopter_state();
            assert!(state.fanouts.set_ascension(7));
            state.rng.set(
                RngStream::Niche,
                RngStreamState {
                    words: Xoshiro256StarStar::from_seed(seed).words,
                    counter: 0,
                },
            );
            spawn_tough_egg(&mut state, 0).unwrap();
            let egg_uid = state.monsters[0].uid;
            eggs.insert(state.monsters[0].max_hp);
            state.monsters_mut()[0]
                .powers
                .set(PowerId::HatchPower, SlotWire::Int, 1);
            hatch_tough_egg(&mut state, egg_uid).unwrap();
            hatchlings.insert(state.monsters[0].max_hp);
        }
        assert!(eggs.iter().all(|hp| (14..=18).contains(hp)), "{eggs:?}");
        assert!(eggs.contains(&14));
        assert!(
            hatchlings.iter().all(|hp| (19..=22).contains(hp)),
            "{hatchlings:?}"
        );
        assert!(hatchlings.contains(&19));
    }

    /// Axebot's band is `GetValueIfAscension(8, 76, 70)`..`(8, 86, 78)` plus
    /// ten per respawn, in both the validator and the replacement roll.
    #[test]
    fn axebot_bands_follow_the_tier_including_the_respawn_bonus() {
        for (ascension, max_hp, admitted) in
            [(7, 70, true), (7, 79, false), (8, 79, true), (8, 70, false)]
        {
            let mut state = singleton(ascension, MonsterKind::Axebot, max_hp);
            state.monsters_mut()[0]
                .powers
                .set(PowerId::Stock, SlotWire::Int, AXEBOT_STOCK);
            assert_eq!(
                axebot_state_is_valid(&state),
                admitted,
                "Axebot {max_hp} at A{ascension}"
            );
        }
        let mut state = singleton(7, MonsterKind::Axebot, 75);
        state.monsters_mut()[0].hp = 0;
        state.monsters_mut()[0]
            .powers
            .set(PowerId::Stock, SlotWire::Int, AXEBOT_STOCK);
        let mut events = Vec::new();
        respawn_axebot(&mut state, 0, &mut events).unwrap();
        let replacement = &state.monsters[0];
        assert!(
            (80..=88).contains(&replacement.max_hp),
            "first A7 respawn rolls 70+10..78+10, got {}",
            replacement.max_hp
        );
    }

    /// #3756, site witness `AUNPC3BZ101H` node 37: the dying Axebot is still
    /// in `_enemies` when Stock creates its replacement, so its max HP is not
    /// a candidate. A first respawn after a body at the top of the A8+ band
    /// (86) draws from 87..=96, ten values; a body below the next band (80)
    /// leaves all eleven.
    #[test]
    fn a_stock_respawn_reserves_the_dying_bodys_max_hp() {
        for seed in 0..64_u64 {
            for (corpse_max, low, count) in [(86, 87, 10), (80, 86, 11)] {
                let mut state = singleton(8, MonsterKind::Axebot, corpse_max);
                state.monsters_mut()[0].hp = 0;
                state.monsters_mut()[0]
                    .powers
                    .set(PowerId::Stock, SlotWire::Int, AXEBOT_STOCK);
                seeded_niche(&mut state, seed);
                let mut expected = Xoshiro256StarStar::from_seed(seed);
                let pick = expected.next_bounded(count).unwrap();
                respawn_axebot(&mut state, 0, &mut Vec::new()).unwrap();
                let replacement = &state.monsters[0];
                assert_eq!(
                    replacement.max_hp,
                    low + pick,
                    "seed {seed}, corpse {corpse_max}"
                );
                assert_eq!(replacement.hp, replacement.max_hp);
                assert_ne!(replacement.max_hp, corpse_max);
                assert_eq!(state.rng.get(RngStream::Niche).counter, 1);
            }
        }
    }

    /// Two-Tailed Rat `GetValueIfAscension(8, 18..22, 17..21)`: the roster
    /// validator admits a rat at 17 only below A8, and 22 only at A8+.
    #[test]
    fn the_rat_roster_band_is_the_fights_tier() {
        for (ascension, hp, admitted) in
            [(7, 17, true), (7, 22, false), (8, 22, true), (8, 17, false)]
        {
            let mut state = rat_state();
            assert!(state.fanouts.set_ascension(ascension));
            let rat = &mut state.monsters_mut()[1];
            rat.max_hp = hp;
            rat.hp = hp;
            assert_eq!(
                rat_roster_is_valid(&state),
                admitted,
                "rat {hp} at A{ascension}"
            );
        }
    }

    fn seeded_niche(state: &mut HotState, seed: u64) {
        state.rng.set(
            RngStream::Niche,
            RngStreamState {
                words: Xoshiro256StarStar::from_seed(seed).words,
                counter: 0,
            },
        );
    }

    /// #3039: an untracked counter is `max(uid) + 1` and stays zero; a
    /// tracked one hands out consecutive ids and advances; an Osty under an
    /// untracked counter, or a counter behind a live uid, refuses by name.
    #[test]
    fn creature_uid_allocation_tracks_the_native_counter_or_refuses() {
        let mut untracked = HotState::at_defaults();
        let mut phrog = HotMonster::new(MonsterKind::PhrogParasite, 1);
        phrog.max_hp = 66;
        untracked.monsters_mut().push(phrog);
        assert_eq!(allocate_creature_uids(&mut untracked, 4), Ok(1));
        assert_eq!(untracked.fanouts.next_creature_uid(), 0);

        let mut tracked = untracked.clone();
        tracked.fanouts.set_next_creature_uid(2);
        assert_eq!(allocate_creature_uids(&mut tracked, 4), Ok(2));
        assert_eq!(tracked.fanouts.next_creature_uid(), 6);
        assert_eq!(allocate_creature_uid(&mut tracked), Ok(6));
        assert_eq!(tracked.fanouts.next_creature_uid(), 7);

        let mut osty = untracked.clone();
        osty.fanouts.mutate_pet(|pet| pet.summon(5)).unwrap();
        let before = osty.clone();
        assert_eq!(
            allocate_creature_uid(&mut osty),
            Err(EngineRefusal::MalformedArgs(
                "creature id counter untracked with a live pet"
            ))
        );
        assert_eq!(osty, before);

        let mut behind = untracked.clone();
        behind.monsters_mut()[0].uid = 4;
        behind.fanouts.set_next_creature_uid(3);
        assert_eq!(
            allocate_creature_uid(&mut behind),
            Err(EngineRefusal::MalformedArgs(
                "creature id counter behind a monster uid"
            ))
        );
        assert_eq!(behind.fanouts.next_creature_uid(), 3);
    }

    /// #3039, `OstyCmd/<Summon>d__0` `0x3ee040`: only a fresh Osty
    /// (`AddPet<Osty>`, `IL_01e9`) takes a creature id; growing a live one or
    /// reviving the retained corpse (`AddPetInternal`, `IL_01d4`) does not.
    #[test]
    fn only_a_fresh_osty_advances_a_tracked_creature_counter() {
        let mut state = HotState::at_defaults();
        state.fanouts.set_next_creature_uid(3);
        state.fanouts.mutate_pet(|pet| pet.summon(5)).unwrap();
        assert_eq!(state.fanouts.next_creature_uid(), 4);
        state.fanouts.mutate_pet(|pet| pet.summon(2)).unwrap();
        assert_eq!(state.fanouts.next_creature_uid(), 4, "a live re-summon");
        state
            .fanouts
            .mutate_pet(|pet| pet.lose_hp(7).map(|_| ()))
            .unwrap();
        assert!(state.fanouts.pet().corpse());
        state.fanouts.mutate_pet(|pet| pet.summon(5)).unwrap();
        assert_eq!(state.fanouts.next_creature_uid(), 4, "a revive");
        // A zero summon creates nothing.
        let mut zero = HotState::at_defaults();
        zero.fanouts.set_next_creature_uid(3);
        zero.fanouts.mutate_pet(|pet| pet.summon(0)).unwrap();
        assert_eq!(zero.fanouts.next_creature_uid(), 3);
        // Untracked stays untracked.
        let mut untracked = HotState::at_defaults();
        untracked.fanouts.mutate_pet(|pet| pet.summon(5)).unwrap();
        assert_eq!(untracked.fanouts.next_creature_uid(), 0);
    }

    fn opening_catalog(relics: &[crate::ids::RelicId]) -> Catalog {
        let mut builder = crate::catalog::CatalogBuilder::new();
        builder.set_relics(relics).unwrap();
        builder.build()
    }

    fn phrog_roster() -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 80;
        state.max_hp = 80;
        let mut phrog = HotMonster::new(MonsterKind::PhrogParasite, 66);
        phrog.max_hp = 66;
        state.monsters_mut().push(phrog);
        state
    }

    /// #3039: the combat-start pets take the ids after the encounter's
    /// enemies, so the counter is seeded to `enemies + pets` once they exist,
    /// and a pet-less opening leaves it at the zero default.
    #[test]
    fn combat_start_pets_seed_the_creature_counter() {
        use crate::ids::RelicId;
        for (relics, expected) in [
            (&[][..], 0),
            (&[RelicId::RelicByrdpip][..], 2),
            (&[RelicId::RelicPaelsLegion][..], 2),
            (&[RelicId::RelicBoundPhylactery][..], 2),
            (
                &[RelicId::RelicByrdpip, RelicId::RelicBoundPhylactery][..],
                3,
            ),
        ] {
            let catalog = opening_catalog(relics);
            let mut state = phrog_roster();
            let mut events = Vec::new();
            crate::engine::fire_before_combat_start(&catalog, &mut state, &mut events).unwrap();
            assert_eq!(state.fanouts.next_creature_uid(), expected, "{relics:?}");
        }
    }

    /// #3039 witness: in a Byrdpip Phrog Parasite fight the Phrog is uid 0,
    /// the pet holds the next id, and the four Wrigglers take 2..=5 (native
    /// .mcr targets 3..=6). The pet-less fight is unchanged (1..=4).
    #[test]
    fn wrigglers_take_uids_after_a_combat_start_pet() {
        use crate::ids::RelicId;
        for (relics, uids, next) in [
            (&[][..], [1, 2, 3, 4], 0),
            (&[RelicId::RelicByrdpip][..], [2, 3, 4, 5], 6),
            (&[RelicId::RelicBoundPhylactery][..], [2, 3, 4, 5], 6),
        ] {
            let catalog = opening_catalog(relics);
            let mut state = phrog_roster();
            seeded_niche(&mut state, 7);
            let mut events = Vec::new();
            crate::engine::fire_before_combat_start(&catalog, &mut state, &mut events).unwrap();
            state.monsters_mut()[0].hp = 0;
            crate::engine::damage::finish_monster_death(&mut state, 0, &mut events).unwrap();
            let spawned: Vec<(MonsterKind, u32)> = state.monsters[1..]
                .iter()
                .map(|monster| (monster.kind, monster.uid))
                .collect();
            assert_eq!(
                spawned,
                uids.map(|uid| (MonsterKind::Wriggler, uid)).to_vec(),
                "{relics:?}"
            );
            assert_eq!(state.fanouts.next_creature_uid(), next, "{relics:?}");
        }
    }

    fn surprise_state(seed: u64, next_creature_uid: u32) -> HotState {
        let mut state = HotState::at_defaults();
        state.hp = 70;
        state.max_hp = 70;
        let mut merc = HotMonster::new(MonsterKind::GremlinMerc, 0);
        merc.max_hp = 51;
        state.monsters_mut().push(merc);
        seeded_niche(&mut state, seed);
        state.fanouts.set_next_creature_uid(next_creature_uid);
        state
    }

    /// Native Surprise rolls: Fat first, then Sneaky, each unique against
    /// `_enemies` = {Merc} only (#3040). Returns `(fat, sneaky, buggy)`,
    /// where `buggy` is the pre-#3040 Sneaky that also excluded Fat.
    fn surprise_rolls(state: &HotState) -> (i32, i32, i32) {
        let merc = state.monsters[0].max_hp;
        let live = state.rng.get(RngStream::Niche);
        let mut rng = Xoshiro256StarStar {
            words: live.words,
            counter: live.counter,
        };
        let pick = |rng: &mut Xoshiro256StarStar, kind, excluded: &[i32]| {
            let (low, high) = native_hp_band(state, kind).unwrap();
            let candidates: Vec<i32> = (low..=high).filter(|hp| !excluded.contains(hp)).collect();
            let bound = i32::try_from(candidates.len()).unwrap();
            candidates[usize::try_from(rng.next_bounded(bound).unwrap()).unwrap()]
        };
        let fat = pick(&mut rng, MonsterKind::FatGremlin, &[merc]);
        let mut buggy_rng = Xoshiro256StarStar {
            words: rng.words,
            counter: rng.counter,
        };
        let sneaky = pick(&mut rng, MonsterKind::SneakyGremlin, &[merc]);
        let buggy = pick(&mut buggy_rng, MonsterKind::SneakyGremlin, &[merc, fat]);
        (fat, sneaky, buggy)
    }

    /// #3040 witness (`SurprisePower/<AfterDeath>d__4` `0x347044`): Fat's
    /// creation at `IL_0063` does not add it to `_enemies` before Sneaky's
    /// `CreateCreature` (`IL_0198`), so a Fat roll inside Sneaky's band does
    /// not shift Sneaky's pick. A Fat roll outside the band is the control.
    #[test]
    fn surprise_rolls_sneaky_unique_against_the_merc_only() {
        let (sneaky_low, sneaky_high) =
            native_hp_band(&HotState::at_defaults(), MonsterKind::SneakyGremlin).unwrap();
        let mut overlap_changed = None;
        let mut control = None;
        for seed in 0..4096_u64 {
            let state = surprise_state(seed, 0);
            let (fat, sneaky, buggy) = surprise_rolls(&state);
            let in_band = (sneaky_low..=sneaky_high).contains(&fat);
            if in_band && sneaky != buggy && overlap_changed.is_none() {
                overlap_changed = Some((seed, fat, sneaky, buggy));
            }
            if !in_band && control.is_none() {
                assert_eq!(sneaky, buggy);
                control = Some((seed, fat, sneaky));
            }
        }
        let (seed, fat, sneaky, buggy) = overlap_changed.expect("an overlapping Fat roll");
        assert_ne!(sneaky, buggy);
        let (control_seed, control_fat, control_sneaky) = control.expect("a control roll");
        for (seed, fat, sneaky) in [
            (seed, fat, sneaky),
            (control_seed, control_fat, control_sneaky),
        ] {
            let mut state = surprise_state(seed, 0);
            spawn_gremlin_merc_pair(&mut state, 0).unwrap();
            let [_, spawned_sneaky, spawned_fat] = state.monsters.as_slice() else {
                panic!("Surprise publishes Sneaky and Fat");
            };
            assert_eq!(
                (spawned_fat.max_hp, spawned_sneaky.max_hp),
                (fat, sneaky),
                "seed {seed}"
            );
            assert!(gremlin_merc_state_is_valid(&state), "seed {seed}");
        }
    }

    /// #3040: Sneaky may share Fat's max HP, since neither roll sees the
    /// other; the lineage stays valid.
    #[test]
    fn surprise_children_may_share_max_hp() {
        let (sneaky_low, sneaky_high) =
            native_hp_band(&HotState::at_defaults(), MonsterKind::SneakyGremlin).unwrap();
        let seed = (0..4096_u64)
            .find(|seed| {
                let (fat, sneaky, _) = surprise_rolls(&surprise_state(*seed, 0));
                fat == sneaky && (sneaky_low..=sneaky_high).contains(&fat)
            })
            .expect("a shared roll");
        let mut state = surprise_state(seed, 0);
        spawn_gremlin_merc_pair(&mut state, 0).unwrap();
        assert_eq!(state.monsters[1].max_hp, state.monsters[2].max_hp);
        assert!(gremlin_merc_state_is_valid(&state));
        let mut merc_twin = state.clone();
        merc_twin.monsters_mut()[1].max_hp = merc_twin.monsters[0].max_hp;
        assert!(!gremlin_merc_state_is_valid(&merc_twin));
    }

    /// #3039 witness: another spawner with Byrdpip. The pet holds id 1, so
    /// Surprise's Fat is uid 2 and Sneaky 3 (untracked: 1 and 2), and the
    /// lineage validators read the shifted uids.
    #[test]
    fn surprise_children_take_uids_after_a_combat_start_pet() {
        for (counter, fat_uid, next) in [(0, 1, 0), (2, 2, 4)] {
            let mut state = surprise_state(11, counter);
            spawn_gremlin_merc_pair(&mut state, 0).unwrap();
            let [merc, sneaky, fat] = state.monsters.as_slice() else {
                panic!("Surprise publishes Sneaky and Fat");
            };
            assert_eq!(
                (merc.uid, sneaky.kind, sneaky.uid, fat.kind, fat.uid),
                (
                    0,
                    MonsterKind::SneakyGremlin,
                    fat_uid + 1,
                    MonsterKind::FatGremlin,
                    fat_uid
                )
            );
            assert_eq!(state.fanouts.next_creature_uid(), next);
            assert_eq!(gremlin_merc_fat_uid(&state), Some(fat_uid));
            assert!(gremlin_merc_state_is_valid(&state));

            // The other reading of the counter is not an exact lineage.
            let mut forged = state.clone();
            forged
                .fanouts
                .set_next_creature_uid(if counter == 0 { 5 } else { 0 });
            assert!(!gremlin_merc_state_is_valid(&forged));
        }
    }

    /// #3039: a tracked counter moves each Axebot respawn's uid past the pet.
    #[test]
    fn axebot_respawns_take_uids_after_a_combat_start_pet() {
        let mut state = stocked_axebot();
        state.fanouts.set_next_creature_uid(2);
        require_axebot_respawn(&state, 0).unwrap();
        let mut events = Vec::new();
        crate::engine::damage::finish_monster_death(&mut state, 0, &mut events).unwrap();
        assert_eq!(state.monsters[0].uid, 2);
        assert_eq!(state.fanouts.next_creature_uid(), 3);
        assert!(axebot_state_is_valid(&state));
        state.monsters_mut()[0].hp = 0;
        crate::engine::damage::finish_monster_death(&mut state, 0, &mut events).unwrap();
        assert_eq!(state.monsters[0].uid, 3);
        assert!(axebot_state_is_valid(&state));

        let mut forged = state.clone();
        forged.fanouts.set_next_creature_uid(0);
        assert!(!axebot_state_is_valid(&forged), "uid 3 after two respawns");
    }

    /// #3904: an Osty created after a spawned lineage holds the last id the
    /// counter handed out, so the lineage ends one earlier; one created
    /// before it changes nothing. Native numbers both orders in creation
    /// order (`tests/mid_fight_osty_creature_id.rs`).
    #[test]
    fn a_mid_fight_osty_above_a_lineage_moves_the_lineage_down_one() {
        // Surprise under an untracked counter: Fat 1, Sneaky 2.
        let mut state = surprise_state(11, 0);
        spawn_gremlin_merc_pair(&mut state, 0).unwrap();
        // Then a first Osty: creature uid 3, counter 4.
        state.fanouts.set_next_creature_uid(4);
        assert_eq!(tracked_lineage_start(&state, 2), Some(Some(2)));
        assert!(
            !gremlin_merc_state_is_valid(&state),
            "without the Osty's id the pair reads one too high"
        );
        state.fanouts.set_osty_creature_uid(3);
        assert!(is_osty_creature_uid(&state, 3) && !is_osty_creature_uid(&state, 2));
        assert_eq!(tracked_lineage_start(&state, 2), Some(Some(1)));
        assert_eq!(gremlin_merc_fat_uid(&state), Some(1));
        assert!(gremlin_merc_state_is_valid(&state));

        // An Osty created before the pair (uid 1, counter 2) sits below it.
        let mut below = surprise_state(11, 2);
        below.fanouts.set_osty_creature_uid(1);
        spawn_gremlin_merc_pair(&mut below, 0).unwrap();
        assert_eq!(tracked_lineage_start(&below, 2), Some(Some(2)));
        assert!(gremlin_merc_state_is_valid(&below));

        // No recorded Osty: uid 0 is an enemy's, never an Osty's.
        assert!(!is_osty_creature_uid(&surprise_state(11, 0), 0));
    }

    /// #3904: the same for Axebot's bodies. An Osty created after the first
    /// respawn is not the live body, and an Osty's id between two bodies is
    /// not a body a frozen target can name.
    #[test]
    fn a_mid_fight_osty_is_never_an_axebot_body() {
        // First respawn untracked: the body is uid 1. Then the Osty: uid 2.
        let mut state = stocked_axebot();
        let mut events = Vec::new();
        crate::engine::damage::finish_monster_death(&mut state, 0, &mut events).unwrap();
        assert_eq!(state.monsters[0].uid, 1);
        state.fanouts.set_next_creature_uid(3);
        assert!(
            !axebot_state_is_valid(&state),
            "the counter alone says uid 2"
        );
        state.fanouts.set_osty_creature_uid(2);
        assert!(axebot_state_is_valid(&state));
        // The second respawn takes uid 3; the body it replaced is 1, and the
        // Osty's 2 is not a body.
        state.monsters_mut()[0].hp = 0;
        crate::engine::damage::finish_monster_death(&mut state, 0, &mut events).unwrap();
        assert_eq!(state.monsters[0].uid, 3);
        assert!(axebot_state_is_valid(&state));
        assert_eq!(axebot_replacement_of(&state, 0, 1), Some(0));
        assert_eq!(axebot_replacement_of(&state, 0, 2), None, "the Osty");
    }

    /// #3747: a frozen CardPlay target resolves to the live Axebot only when
    /// it names the body that Axebot replaced.
    #[test]
    fn axebot_replacement_of_names_only_the_body_the_live_one_replaced() {
        for tracked in [false, true] {
            let mut state = stocked_axebot();
            if tracked {
                state.fanouts.set_next_creature_uid(2);
            }
            // A corpse awaiting its respawn is not a valid singleton.
            assert_eq!(axebot_replacement_of(&state, 0, 0), None);
            // The first body alive: nothing has been replaced.
            let mut first = state.clone();
            first.monsters_mut()[0].hp = first.monsters[0].max_hp;
            assert!(axebot_state_is_valid(&first));
            assert_eq!(axebot_replacement_of(&first, 0, 0), None);
            assert_eq!(axebot_replacement_of(&first, 0, 5), None);

            let mut events = Vec::new();
            crate::engine::damage::finish_monster_death(&mut state, 0, &mut events).unwrap();
            let second = state.monsters[0].uid;
            assert_eq!(second, if tracked { 2 } else { 1 });
            assert_eq!(axebot_replacement_of(&state, 0, 0), Some(0));
            assert_eq!(axebot_replacement_of(&state, 1, 0), None, "another slot");
            assert_eq!(axebot_replacement_of(&state, 0, second), None);
            assert_eq!(axebot_replacement_of(&state, 0, 7), None);

            state.monsters_mut()[0].hp = 0;
            crate::engine::damage::finish_monster_death(&mut state, 0, &mut events).unwrap();
            let third = state.monsters[0].uid;
            assert_eq!(third, second + 1);
            assert_eq!(axebot_replacement_of(&state, 0, second), Some(0));
            assert_eq!(axebot_replacement_of(&state, 0, 0), None, "two bodies back");
            assert_eq!(axebot_replacement_of(&state, 0, third), None);
            assert_eq!(axebot_replacement_of(&state, 0, third + 1), None);
            // A tracked counter cannot place the second body exactly: any
            // uid between the first body's and the live one's is admitted.
            assert_eq!(
                axebot_replacement_of(&state, 0, 1),
                (tracked || second == 1).then_some(0)
            );
        }
    }

    /// #3039 (I5): a root serialized before the counter existed carries a
    /// Byrdpip pet but no `next_creature_uid`. Its pet holds a creature id
    /// the state cannot place, so the Phrog's Wriggler spawn refuses by name
    /// and publishes nothing; the same pet-less legacy root still allocates
    /// `max(uid) + 1` as before.
    #[test]
    fn untracked_legacy_root_with_a_relic_pet_refuses_the_spawn() {
        use crate::boundary::HotBoundary;
        use crate::ids::RelicId;
        for (relics, expected) in [
            (&[RelicId::RelicByrdpip][..], None),
            (&[RelicId::RelicPaelsLegion][..], None),
            (&[][..], Some([1_u32, 2, 3, 4])),
        ] {
            let catalog = opening_catalog(relics);
            let mut legacy = phrog_roster();
            seeded_niche(&mut legacy, 7);
            if relics.contains(&RelicId::RelicPaelsLegion) {
                // The per-fight cooldown the opening seeds for its owner.
                assert!(legacy.fanouts.set_paels_legion_cooldown(0));
            }
            let document = HotBoundary::try_to_canonical(&legacy, &catalog).unwrap();
            assert!(!document.player.contains_key("next_creature_uid"));
            let mut state = HotBoundary::from_canonical(&document, &catalog).unwrap();
            assert_eq!(state.fanouts.next_creature_uid(), 0);
            assert_eq!(
                usize::from(state.fanouts.relic_pet_creatures()),
                relics.len()
            );
            state.monsters_mut()[0].hp = 0;
            let before = state.clone();
            let mut events = Vec::new();
            let result = crate::engine::damage::finish_monster_death(&mut state, 0, &mut events);
            match expected {
                None => {
                    assert_eq!(
                        result,
                        Err(EngineRefusal::MalformedArgs(
                            "creature id counter untracked with a live pet"
                        )),
                        "{relics:?}"
                    );
                    assert_eq!(state, before, "{relics:?}");
                }
                Some(uids) => {
                    result.unwrap();
                    let spawned: Vec<u32> = state.monsters[1..]
                        .iter()
                        .map(|monster| monster.uid)
                        .collect();
                    assert_eq!(spawned, uids.to_vec());
                }
            }
        }
    }
}

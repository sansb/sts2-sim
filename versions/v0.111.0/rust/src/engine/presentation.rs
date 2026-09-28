//! Opt-in presentation marks inside one wire action (review rows).
//!
//! A wire action is one `apply`, but a review reader sees the cards the game
//! played on its own inside that apply as separate events. The first such
//! source is Hellraiser: `HellraiserPower/<AfterCardDrawnEarly>d__7` RVA
//! `0x33c1a8` IL_011e-012e AutoPlays each drawn Strike, so an End turn that
//! draws Strikes into Hellraiser deals damage on the *next* turn's draw, and
//! a review that only snapshots after the apply shows it as End turn damage.
//! [`autoplay_hellraiser_strike`](super::play::autoplay_hellraiser_strike)
//! marks where each such play begins and ends, so the review producer can
//! split the row and name the card.
//!
//! This is presentation, not parity: the marks are not native checkpoints
//! ([`super::native_checkpoint`] is what the `.mcr` census compares), they
//! carry a visible-state summary rather than a canonical projection (a state
//! in the middle of a Draw frame need not project), and nothing reads them
//! back into the engine. Recording is off unless a caller wraps a transition
//! in [`record`]; the observation sites cost one thread-local read when off.

use std::cell::RefCell;

use serde_json::{Value, json};

use crate::catalog::Catalog;
use crate::hot::{HotCard, HotState, PileId};

/// Which edge of an automatic play a mark sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresentationMarkKind {
    /// The card is about to play; the state has not yet been touched by it.
    AutoPlayBegin,
    /// The card's play (and anything nested in it) has finished.
    AutoPlayEnd,
}

impl PresentationMarkKind {
    /// The stable wire name `diff-serve` reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AutoPlayBegin => "autoplay_begin",
            Self::AutoPlayEnd => "autoplay_end",
        }
    }
}

/// One recorded mark: its edge, which power played the card, the card, and
/// the visible state at that edge.
#[derive(Clone, Debug, PartialEq)]
pub struct PresentationMark {
    /// Begin or end.
    pub kind: PresentationMarkKind,
    /// The owning source, e.g. `"Hellraiser"`.
    pub source: &'static str,
    /// The card as `{uid, id, upgrade, enchantment}`.
    pub card: Value,
    /// [`visible_state`] at this edge.
    pub state: Value,
    /// Finished card plays this combat at this edge: with `kind` and the
    /// card's uid, what identifies a mark across a probe and its commit.
    plays_finished: i32,
}

impl PresentationMark {
    /// The mark as its wire object.
    pub fn to_json(&self) -> Value {
        json!({
            "kind": self.kind.as_str(),
            "source": self.source,
            "card": self.card,
            "state": self.state,
        })
    }
}

thread_local! {
    static RECORDER: RefCell<Option<Vec<PresentationMark>>> = const { RefCell::new(None) };
}

/// Run `transition` with recording on and return the marks of the plays it
/// committed, in order. Nested calls restore the outer recorder.
///
/// The engine rehearses many commands on a cloned probe before running them
/// for real (Hellraiser's own draw hook does, `draw.rs`
/// `hellraiser_after_card_drawn_early`), and a probe passes the same
/// observation sites. A probe and its commit start from the same state, so
/// they leave marks with the same edge, card uid and finished-play count, and
/// the commit's always comes later. Keeping only the last mark for each such
/// key therefore keeps the committed sequence, whichever of the engine's
/// probe sites ran, without every site having to opt out.
pub fn record<T>(transition: impl FnOnce() -> T) -> (T, Vec<PresentationMark>) {
    let outer = RECORDER.with(|recorder| recorder.replace(Some(Vec::new())));
    let result = transition();
    let recorded = RECORDER
        .with(|recorder| recorder.replace(outer))
        .unwrap_or_default();
    (result, committed(recorded))
}

fn committed(recorded: Vec<PresentationMark>) -> Vec<PresentationMark> {
    let key = |mark: &PresentationMark| (mark.kind, mark.card["uid"].as_u64(), mark.plays_finished);
    let mut kept: Vec<PresentationMark> = Vec::with_capacity(recorded.len());
    for mark in recorded {
        kept.retain(|earlier| key(earlier) != key(&mark));
        kept.push(mark);
    }
    kept
}

/// [`record`] when `enabled`, else just `transition` with no marks.
pub fn record_if<T>(enabled: bool, transition: impl FnOnce() -> T) -> (T, Vec<PresentationMark>) {
    if enabled {
        record(transition)
    } else {
        (transition(), Vec::new())
    }
}

/// Whether a caller is recording; the observation sites test this before
/// building anything.
#[inline]
pub(crate) fn recording() -> bool {
    RECORDER.with(|recorder| recorder.borrow().is_some())
}

/// Record one mark when a caller is recording.
#[cold]
#[inline(never)]
pub(crate) fn observe_autoplay(
    kind: PresentationMarkKind,
    source: &'static str,
    state: &HotState,
    catalog: &Catalog,
    card: HotCard,
) {
    if !recording() {
        return;
    }
    let mark = PresentationMark {
        kind,
        source,
        card: card_json(catalog, card),
        state: visible_state(state, catalog),
        plays_finished: state.history.card_plays_finished_combat,
    };
    RECORDER.with(|recorder| {
        if let Some(recorded) = recorder.borrow_mut().as_mut() {
            recorded.push(mark);
        }
    });
}

fn card_json(catalog: &Catalog, card: HotCard) -> Value {
    let Some(spec) = catalog.spec(card.atom) else {
        return json!({"uid": card.uid, "id": null});
    };
    let identity = spec.identity;
    json!({
        "uid": card.uid,
        "id": identity.id.as_str(),
        "upgrade": identity.upgrade,
        "enchantment": identity
            .enchantment
            .map(|enchantment| json!([enchantment.id.as_str(), enchantment.amount])),
    })
}

/// What a review row shows: the player's hp, block, energy and hand, and each
/// monster's kind and hp, read straight off the hot state.
pub fn visible_state(state: &HotState, catalog: &Catalog) -> Value {
    json!({
        "turn": state.turn,
        "hp": state.hp,
        "block": state.block,
        "energy": state.energy,
        "hand": state
            .piles
            .get(PileId::Hand)
            .as_slice()
            .iter()
            .map(|card| card_json(catalog, *card))
            .collect::<Vec<_>>(),
        "monsters": state
            .monsters
            .iter()
            .map(|monster| json!({
                "uid": monster.uid,
                "kind": monster.kind.as_str(),
                "hp": monster.hp,
                "max_hp": monster.max_hp,
            }))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_outside_record_is_a_no_op_and_record_restores_the_outer_recorder() {
        let state = HotState::at_defaults();
        let catalog = crate::catalog::CatalogBuilder::new().build();
        let card = HotCard {
            uid: 7,
            atom: 0,
            flags: 0,
        };
        observe_autoplay(
            PresentationMarkKind::AutoPlayBegin,
            "Hellraiser",
            &state,
            &catalog,
            card,
        );
        assert!(!recording());
        let ((), outer) = record(|| {
            observe_autoplay(
                PresentationMarkKind::AutoPlayBegin,
                "Hellraiser",
                &state,
                &catalog,
                card,
            );
            let ((), inner) = record(|| {
                observe_autoplay(
                    PresentationMarkKind::AutoPlayEnd,
                    "Hellraiser",
                    &state,
                    &catalog,
                    card,
                );
            });
            assert_eq!(inner.len(), 1);
            observe_autoplay(
                PresentationMarkKind::AutoPlayEnd,
                "Hellraiser",
                &state,
                &catalog,
                card,
            );
        });
        let kinds: Vec<_> = outer.iter().map(|mark| mark.kind).collect();
        assert_eq!(
            kinds,
            [
                PresentationMarkKind::AutoPlayBegin,
                PresentationMarkKind::AutoPlayEnd
            ]
        );
        assert_eq!(outer[0].card["uid"], 7);
        assert_eq!(outer[0].state["hand"], json!([]));
    }

    /// The case the marks exist for: End turn's next-turn hand draw feeds a
    /// Strike to Hellraiser, which plays it on turn 2, inside the one apply.
    #[test]
    fn end_turn_marks_the_next_turns_hellraiser_strike_on_the_new_turn() {
        use crate::catalog::{CardIdentity, CatalogBuilder};
        use crate::engine::{Action, admission, apply_action};
        use crate::hot::{CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotMonster};
        use crate::ids::{CardId, MonsterKind, PowerId};
        use crate::powers::SlotWire;

        let plain = |id| CardIdentity {
            id,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let strike = builder
            .intern_reachable(plain(CardId::StrikeIronclad))
            .unwrap();
        let defend = builder
            .intern_reachable(plain(CardId::DefendIronclad))
            .unwrap();
        builder.intern_reachable(plain(CardId::Bash)).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 7;
        state.exact_piles = true;
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        let physical = CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        let mut draw = vec![HotCard {
            uid: 1,
            atom: strike,
            flags: physical,
        }];
        draw.extend((2..=6).map(|uid| HotCard {
            uid,
            atom: defend,
            flags: physical,
        }));
        state.piles.get_mut(PileId::Draw).make_mut().extend(draw);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);

        let (applied, marks) = record(|| apply_action(&state, &catalog, &Action::EndTurn));
        let after = applied.unwrap().state;

        let kinds: Vec<_> = marks.iter().map(|mark| mark.kind).collect();
        assert_eq!(
            kinds,
            [
                PresentationMarkKind::AutoPlayBegin,
                PresentationMarkKind::AutoPlayEnd
            ]
        );
        let (begin, end) = (&marks[0], &marks[1]);
        assert_eq!(begin.source, "Hellraiser");
        assert_eq!(begin.card["uid"], 1);
        assert_eq!(begin.card["id"], "STRIKE_IRONCLAD");
        // Both edges are on the new turn, before the rest of its hand.
        assert_eq!(begin.state["turn"], 2);
        assert_eq!(end.state["turn"], 2);
        assert_eq!(after.turn, 2);
        // The Strike is what damaged the Toadpole, and nothing after it did.
        assert_eq!(begin.state["monsters"][0]["hp"], 1_000);
        assert_eq!(end.state["monsters"][0]["hp"], 994);
        assert_eq!(after.monsters[0].hp, 994);
        // The played Strike was one of the five hand draws.
        assert_eq!(after.piles.get(PileId::Hand).len(), 4);

        // Recording observes; it never changes the transition.
        assert_eq!(
            apply_action(&state, &catalog, &Action::EndTurn)
                .unwrap()
                .state,
            after
        );
    }

    /// A Hellraiser Pommel Strike drawn at turn start draws a Strike that
    /// Hellraiser plays inside it: the marks nest, once each, in commit order.
    #[test]
    fn nested_hellraiser_plays_mark_once_each_in_commit_order() {
        use crate::catalog::{CardIdentity, CatalogBuilder};
        use crate::engine::{Action, admission, apply_action};
        use crate::hot::{CARD_FLAG_DEFAULT_PHYSICAL_STATE, HotMonster};
        use crate::ids::{CardId, MonsterKind, PowerId};
        use crate::powers::SlotWire;

        let plain = |id| CardIdentity {
            id,
            upgrade: 0,
            enchantment: None,
        };
        let mut builder = CatalogBuilder::new();
        let pommel = builder
            .intern_reachable(plain(CardId::PommelStrike))
            .unwrap();
        let strike = builder
            .intern_reachable(plain(CardId::StrikeIronclad))
            .unwrap();
        let defend = builder
            .intern_reachable(plain(CardId::DefendIronclad))
            .unwrap();
        builder.intern_reachable(plain(CardId::Bash)).unwrap();
        builder.intern_monster(MonsterKind::Toadpole).unwrap();
        builder.mark_persistent_action_replay_required();
        let catalog = builder.build();
        let mut state = HotState::at_defaults();
        state.hp = 50;
        state.max_hp = 50;
        state.energy = 3;
        state.player_phase = admission::PHASE_ORDINARY_ACTIONS;
        state.next_card_uid = 8;
        state.exact_piles = true;
        state.powers.set(PowerId::Hellraiser, SlotWire::Int, 1);
        crate::engine::turn::hydrate_after_side_turn_end_power_order_for_test(&mut state);
        let physical = CARD_FLAG_DEFAULT_PHYSICAL_STATE;
        let mut draw = vec![
            HotCard {
                uid: 1,
                atom: pommel,
                flags: physical,
            },
            HotCard {
                uid: 2,
                atom: strike,
                flags: physical,
            },
        ];
        draw.extend((3..=7).map(|uid| HotCard {
            uid,
            atom: defend,
            flags: physical,
        }));
        state.piles.get_mut(PileId::Draw).make_mut().extend(draw);
        let mut monster = HotMonster::new(MonsterKind::Toadpole, 1_000);
        monster.max_hp = 1_000;
        monster.loop_pos = 2;
        state.monsters = std::sync::Arc::new(vec![monster]);

        let (applied, marks) = record(|| apply_action(&state, &catalog, &Action::EndTurn));
        let after = applied.unwrap().state;

        let sequence: Vec<_> = marks
            .iter()
            .map(|mark| (mark.kind, mark.card["uid"].as_u64().unwrap()))
            .collect();
        assert_eq!(
            sequence,
            [
                (PresentationMarkKind::AutoPlayBegin, 1),
                (PresentationMarkKind::AutoPlayBegin, 2),
                (PresentationMarkKind::AutoPlayEnd, 2),
                (PresentationMarkKind::AutoPlayEnd, 1),
            ]
        );
        assert!(marks.iter().all(|mark| mark.state["turn"] == 2));
        assert_eq!(marks[3].state["monsters"][0]["hp"], after.monsters[0].hp);
    }
}

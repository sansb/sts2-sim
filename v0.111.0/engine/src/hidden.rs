//! Hidden information: what the player observes of a combat state (#3436).
//!
//! The engine steps concrete worlds: a [`HotState`] knows the draw pile's
//! order and every RNG stream's words, which the player does not. This module
//! is the search-layer observation model over that engine. It changes no
//! engine semantics: a sampled world is an ordinary state, stepped by the
//! certified transition.
//!
//! # The observation model
//!
//! **Hidden:**
//!
//! - the **order** of the draw pile, except positions the player knows
//!   ([`DrawKnowledge`]); its multiset is visible;
//! - the **words** of all nine combat RNG streams ([`STREAMS`]). Their draw
//!   counters are kept: they are bookkeeping the player could count, and the
//!   engine reads them.
//!
//! **Visible:** everything else in the state — hand, discard, exhaust and play
//! piles in order, energy, HP, block, powers, every monster's HP, powers and
//! current intent, potions, relic counters.
//!
//! # What the player knows about positions
//!
//! A card placed on the draw pile **deterministically** (top or bottom, e.g.
//! Shining Strike, Rebound, Glimmer's put-back) has a known position until it is
//! drawn or a reshuffle randomizes the pile. [`observe_step`] infers this from
//! the transition, not from the true order:
//!
//! - every random-position draw insertion consumes [`RngStream::Rng`]
//!   (`engine/cards.rs`: the three `next_bounded(len + 1)` inserts), and a
//!   reshuffle emits [`Event::Reshuffled`]. A card that enters the draw pile in
//!   a step that did neither was placed deterministically, so its position is
//!   known.
//! - In a step that did consume `Rng`, entering cards are treated as unknown.
//!   That can only *under*-claim knowledge (a deterministic top placement in
//!   the same step as a random insert is forgotten), never leak it. If known
//!   positions already exist, a random insert could land above them and make
//!   the knowledge conditional, so the step refuses
//!   ([`HiddenRefusal::RandomInsertOverKnownPositions`]).
//! - A reshuffle clears all knowledge. A card placed deterministically after
//!   the shuffle in the same step (Shining Strike's after-play top route
//!   following a mid-play reshuffle) is then treated as unknown: again an
//!   under-claim, never a leak. The one placement that follows **every**
//!   reshuffle is the Perfect Fit enchantment
//!   (`PerfectFit::ModifyShuffleOrder`, RVA `0xd6259`, cited in
//!   `engine/draw.rs`); a fight that can contain it refuses at its first
//!   reshuffle ([`HiddenRefusal::ReshufflePlacesKnownCards`]) rather than
//!   systematically under-claim.
//!
//! An exact account would need the engine to report each draw insertion and
//! its placement; that event does not exist yet.
//!
//! # What the IL says the player sees (v0.111.0, DLL `9cb4f1ad`)
//!
//! - The draw-pile view sorts its cards (rarity, then model id) before
//!   display: `NCardPileScreen::OnPileContentsChanged` RVA `0x1a7268`. Order is
//!   hidden. (`List.Sort` is unstable, so ties between identical-rank cards
//!   could in principle leak a trace of order; not modelled.)
//! - Choosing a card from the draw pile (`CardSelectCmd.FromCombatPile`,
//!   `0x3e5e84`; Tutor, Seeker Strike, Stratagem, ...) opens
//!   `NCombatPileCardSelectScreen`, whose `UpdatePileContents` (`0x24935c`)
//!   sorts likewise. Order is hidden.
//! - No card, relic or power reveals the top of the draw pile or its order,
//!   and the intent display shows only the next move.
//!
//! # Information-set key
//!
//! [`info_key`] hashes the state with the draw pile's unknown segment sorted
//! by uid and the stream words zeroed, plus the knowledge. Two worlds of one
//! information set always share a key. A key does **not** merge observationally
//! identical copies of a card (two Strikes with different uids): that is a
//! finer partition than the player's, so it costs search efficiency, never
//! correctness.

use std::hash::{Hash, Hasher};

use sha2::{Digest, Sha256};

use crate::catalog::Catalog;
use crate::engine::Event;
use crate::hot::{HotState, PileId, RngStream, RngStreamState};
use crate::ids::EnchantmentId;

/// The nine combat RNG streams, all hidden.
pub const STREAMS: [RngStream; 9] = [
    RngStream::Ai,
    RngStream::CombatOrbs,
    RngStream::EnergyCosts,
    RngStream::Generation,
    RngStream::Niche,
    RngStream::PotionGeneration,
    RngStream::Rng,
    RngStream::Sel,
    RngStream::Targets,
];

/// Draw-pile positions the player knows, beyond the pile's multiset.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct DrawKnowledge {
    /// uids known to sit at the top, top-most first.
    pub top: Vec<u32>,
    /// uids known to sit at the bottom, bottom-most last.
    pub bottom: Vec<u32>,
}

impl DrawKnowledge {
    /// No known positions.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Whether the player knows no positions.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.top.is_empty() && self.bottom.is_empty()
    }
}

/// Why a state or transition cannot be represented in the observation model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HiddenRefusal {
    /// The knowledge does not match the state's draw pile: a known uid is
    /// absent, or the known cards are not where the knowledge says.
    KnowledgeInconsistent,
    /// A random draw insertion while positions are known: the insert could
    /// land above a known card, so its position becomes conditional.
    RandomInsertOverKnownPositions,
    /// A reshuffle in a fight where the Perfect Fit enchantment can be live:
    /// it places cards on top after every shuffle, which this model cannot
    /// see.
    ReshufflePlacesKnownCards,
    /// A deterministic draw insertion landed between unknown cards: neither
    /// a top nor a bottom placement, so the model has no position for it.
    UnexplainedDrawInsertion,
    /// Two worlds of one information set offered different legal actions.
    ActionSetDependsOnHiddenState,
    /// A state carrying an action-replay receipt did not project to its
    /// canonical document, so its information set cannot be identified.
    ReceiptNotProjectable,
    /// The search root carries a receipt whose resumption replays the true
    /// predecessor (a parked draw): a sampled world would resume from the
    /// true, hidden draw order.
    RootReceiptReplaysTruePredecessor,
    /// A pending selection froze a Draw-pile view (Stratagem, Foregone
    /// Conclusion, a potion's finish list such as Droplet of Precognition)
    /// while the Draw pile holds two copies of one card. The view's sort
    /// (`selection::sort_native_draw_view`, stable on rarity then id) keeps
    /// tied copies in their hidden pile order, so the frozen list encodes it.
    FrozenDrawViewTies,
    /// Draw-pile knowledge would survive a step that consumed the `Rng`
    /// stream without adding a card or reshuffling: an unrecognized random
    /// reorder could have moved a known card.
    RngConsumedOverKnownPositions,
}

/// The draw pile's uids, top first.
fn draw_uids(state: &HotState) -> Vec<u32> {
    state
        .piles
        .get(PileId::Draw)
        .as_slice()
        .iter()
        .map(|card| card.uid)
        .collect()
}

/// Check that `know` describes `state`'s draw pile.
///
/// # Errors
///
/// [`HiddenRefusal::KnowledgeInconsistent`] when it does not.
pub fn check_knowledge(state: &HotState, know: &DrawKnowledge) -> Result<(), HiddenRefusal> {
    let draw = draw_uids(state);
    if know.top.len() + know.bottom.len() > draw.len()
        || !draw.starts_with(&know.top)
        || !draw.ends_with(&know.bottom)
    {
        return Err(HiddenRefusal::KnowledgeInconsistent);
    }
    Ok(())
}

/// The bounds of the unknown middle of a draw pile of `len` cards.
fn unknown_span(len: usize, know: &DrawKnowledge) -> std::ops::Range<usize> {
    know.top.len()..len - know.bottom.len()
}

/// The information set's canonical representative: unknown draw segment
/// sorted by uid, stream words zeroed (counters kept).
///
/// The result is for hashing. It is **not** a playable world: zero words are
/// the xoshiro absorbing state.
#[must_use]
pub fn canonical_representative(state: &HotState, know: &DrawKnowledge) -> HotState {
    let mut rep = state.clone();
    let span = unknown_span(rep.piles.get(PileId::Draw).len(), know);
    if span.len() > 1 {
        rep.piles.get_mut(PileId::Draw).make_mut()[span].sort_by_key(|card| card.uid);
    }
    for stream in STREAMS {
        let counter = rep.rng.get(stream).counter;
        rep.rng.set(
            stream,
            RngStreamState {
                words: [0; 4],
                counter,
            },
        );
    }
    rep
}

/// Whether `state` carries an action-replay receipt.
///
/// A receipt (`frame::Frame::ActionReplay`) holds the predecessor state
/// whole: its draw order and stream words included. The
/// fast representative ([`canonical_representative`]) does not reach inside
/// it, so such a state is identified by [`receipt_aware_key`] instead.
#[must_use]
pub fn has_replay_receipt(state: &HotState) -> bool {
    matches!(
        state.frames.as_slice().first(),
        Some(crate::frame::Frame::ActionReplay { .. })
    )
}

/// Whether the search may start from `root`: a receipt whose resumption
/// replays its embedded predecessor would let every sampled world resume
/// from the true draw order.
///
/// # Errors
///
/// [`HiddenRefusal::RootReceiptReplaysTruePredecessor`].
pub fn check_root(root: &HotState) -> Result<(), HiddenRefusal> {
    if has_replay_receipt(root) && crate::engine::puzzle::parked_draw_depth(root).is_some() {
        return Err(HiddenRefusal::RootReceiptReplaysTruePredecessor);
    }
    Ok(())
}

/// Whether `state` may be represented: refuses a frozen Draw-pile view that
/// could encode the hidden order of tied copies.
///
/// # Errors
///
/// [`HiddenRefusal::FrozenDrawViewTies`].
pub fn check_frozen_views(state: &HotState, catalog: &Catalog) -> Result<(), HiddenRefusal> {
    let Some(pending) = state.pending.as_deref() else {
        return Ok(());
    };
    let frames = &state.frames;
    let frozen_view = pending.stratagem_potion_record(frames).is_some()
        || pending.stratagem_draw_record(frames).is_some()
        || pending.foregone_before_hand_draw_record(frames).is_some()
        || pending.potion_finish_record(frames).is_some();
    if !frozen_view {
        return Ok(());
    }
    let mut ids: Vec<_> = state
        .piles
        .get(PileId::Draw)
        .as_slice()
        .iter()
        .filter_map(|card| catalog.spec(card.atom).map(|spec| spec.identity.id))
        .collect();
    ids.sort_unstable();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(HiddenRefusal::FrozenDrawViewTies);
    }
    Ok(())
}

/// Sort a canonical document's draw pile by uid over `span` (all of it when
/// `None`) and zero its stream words, recursing into every embedded
/// predecessor (whose draw pile is sorted whole: the predecessor's own
/// knowledge is not recorded, which merges information sets that differ only
/// in known positions of a past state — coarser, never leaking).
fn sanitize_document(value: &mut serde_json::Value, span: Option<(usize, usize)>) {
    use serde_json::Value;
    if let Some(Value::Array(draw)) = value.pointer_mut("/piles/draw") {
        let (top, bottom) = span.unwrap_or((0, 0));
        let end = draw.len().saturating_sub(bottom);
        if top < end {
            draw[top..end].sort_by_key(|card| card.get("uid").and_then(Value::as_u64));
        }
    }
    if let Some(Value::Object(streams)) = value.get_mut("rng") {
        for stream in streams.values_mut() {
            if let Some(words) = stream.get_mut("words") {
                *words = Value::Array(Vec::new());
            }
        }
    }
    fn nested(value: &mut Value) {
        match value {
            Value::Object(map) => {
                for (key, child) in map.iter_mut() {
                    if key == "predecessor" {
                        sanitize_document(child, None);
                    } else {
                        nested(child);
                    }
                }
            }
            Value::Array(items) => items.iter_mut().for_each(nested),
            _ => {}
        }
    }
    if let Value::Object(map) = value {
        for (key, child) in map.iter_mut() {
            if key != "piles" && key != "rng" {
                nested(child);
            }
        }
    }
}

/// The information-set key of a state that carries a replay receipt: its
/// canonical document with the hidden parts — the draw pile's unknown
/// segment's order and every stream's words, in the state and in every
/// embedded predecessor — erased.
///
/// # Errors
///
/// [`HiddenRefusal::ReceiptNotProjectable`] when the state does not project.
pub fn receipt_aware_key(
    state: &HotState,
    know: &DrawKnowledge,
    catalog: &Catalog,
) -> Result<InfoKey, HiddenRefusal> {
    let document = crate::boundary::HotBoundary::try_to_canonical(state, catalog)
        .map_err(|_| HiddenRefusal::ReceiptNotProjectable)?;
    let value =
        serde_json::to_value(&document).map_err(|_| HiddenRefusal::ReceiptNotProjectable)?;
    document_key(value, know)
}

/// [`receipt_aware_key`] over an already projected document.
fn document_key(
    mut value: serde_json::Value,
    know: &DrawKnowledge,
) -> Result<InfoKey, HiddenRefusal> {
    sanitize_document(&mut value, Some((know.top.len(), know.bottom.len())));
    let bytes = serde_json::to_vec(&value).map_err(|_| HiddenRefusal::ReceiptNotProjectable)?;
    let mut digest = Sha256::new();
    digest.update(b"sts-sim-infoset-receipt-v1");
    digest.update(&bytes);
    let mut knowledge = BufferWriter(Vec::new());
    know.hash(&mut knowledge);
    digest.update(&knowledge.0);
    Ok(InfoKey(digest.finalize().into()))
}

/// The fast identity parts of a state carrying a bottom receipt: the canonical
/// representative of the state with the receipt removed (every later record
/// rebased, `HotState::without_action_replay_root`), and the receipt's action
/// and answers. `None` when the state carries no receipt or the receipt
/// cannot be lifted out (use [`receipt_aware_key`] then).
///
/// The receipt's predecessor is left out on purpose: the caller pairs
/// these parts with the identity of the search node the predecessor *was*
/// (its origin), which is the predecessor's information set already
/// sanitized — no projection, no JSON.
#[must_use]
pub fn sole_receipt_parts(
    state: &HotState,
    know: &DrawKnowledge,
) -> Option<(HotState, ReceiptParts)> {
    if !has_replay_receipt(state) {
        return None;
    }
    let (rest, record) = state.without_action_replay_root()?;
    Some((
        canonical_representative(&rest, know),
        ReceiptParts {
            action: record.action,
            answers: record.answers,
        },
    ))
}

/// A receipt's action and answers: what [`sole_receipt_parts`] keeps of it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ReceiptParts {
    action: crate::hot::ActionReplayRootAction,
    answers: Vec<crate::hot::ActionReplayAnswer>,
}

/// The predecessor of `state`'s receipt, for telling a carried receipt from
/// a new one.
#[must_use]
pub fn receipt_predecessor(state: &HotState) -> Option<std::sync::Arc<HotState>> {
    let Some(crate::frame::Frame::ActionReplay { record }) =
        state.frames.as_slice().first().copied()
    else {
        return None;
    };
    state
        .frames
        .action_replay(record)
        .map(|record| record.predecessor)
}

/// A 256-bit information-set key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InfoKey(pub [u8; 32]);

/// Collects a derived `Hash` stream into one buffer, so SHA-256 runs once
/// over contiguous bytes instead of once per field (most writes are a few
/// bytes, and per-call digest overhead dominated the search profile).
struct BufferWriter(Vec<u8>);

impl Hasher for BufferWriter {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
    fn finish(&self) -> u64 {
        unreachable!("the key is the full digest")
    }
}

/// SHA-256 over `domain` and a value's derived `Hash` stream.
#[must_use]
pub fn digest_of(domain: &[u8], value: &impl Hash) -> InfoKey {
    thread_local! {
        static BUFFER: std::cell::RefCell<Vec<u8>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    BUFFER.with(|buffer| {
        let mut writer = BufferWriter(std::mem::take(&mut *buffer.borrow_mut()));
        writer.0.clear();
        writer.0.extend_from_slice(domain);
        value.hash(&mut writer);
        let key = InfoKey(Sha256::digest(&writer.0).into());
        *buffer.borrow_mut() = writer.0;
        key
    })
}

/// The information-set key of `state` under `know`.
#[must_use]
pub fn info_key(state: &HotState, know: &DrawKnowledge) -> InfoKey {
    let rep = canonical_representative(state, know);
    digest_of(b"sts-sim-infoset-v1", &(&rep, know))
}

/// SplitMix64: search-policy randomness, never a game stream.
#[derive(Clone, Debug)]
pub struct PolicyRng(pub u64);

impl PolicyRng {
    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..bound` (`bound > 0`).
    pub fn below(&mut self, bound: usize) -> usize {
        (self.next_u64() % bound as u64) as usize
    }

    /// Uniform in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// One world consistent with the information set of `state` under `know`.
///
/// The unknown draw segment is first put in canonical (uid) order, so the
/// true order cannot leak through the permutation, then Fisher-Yates shuffled
/// from `rng`. Every stream gets fresh nonzero words; counters are kept.
#[must_use]
pub fn sample_world(state: &HotState, know: &DrawKnowledge, rng: &mut PolicyRng) -> HotState {
    let mut world = state.clone();
    let span = unknown_span(world.piles.get(PileId::Draw).len(), know);
    if span.len() > 1 {
        let middle = &mut world.piles.get_mut(PileId::Draw).make_mut()[span];
        middle.sort_by_key(|card| card.uid);
        for i in (1..middle.len()).rev() {
            middle.swap(i, rng.below(i + 1));
        }
    }
    for stream in STREAMS {
        let counter = world.rng.get(stream).counter;
        let words = [
            rng.next_u64() | 1,
            rng.next_u64(),
            rng.next_u64(),
            rng.next_u64(),
        ];
        world.rng.set(stream, RngStreamState { words, counter });
    }
    world
}

/// Whether a reshuffle in this fight can be followed by a deterministic top
/// placement: a reachable card carries the Perfect Fit enchantment.
#[must_use]
pub fn reshuffle_places_known_cards(catalog: &Catalog) -> bool {
    catalog.reachable_specs().any(|spec| {
        spec.identity
            .enchantment
            .is_some_and(|enchantment| enchantment.id == EnchantmentId::PerfectFit)
    })
}

/// Update the player's draw-pile knowledge across one transition.
///
/// `pre` is the state the action was applied to, under `know`; `post` and
/// `events` are what the transition produced.
///
/// # Errors
///
/// The named refusals of the module documentation.
pub fn observe_step(
    catalog: &Catalog,
    pre: &HotState,
    know: &DrawKnowledge,
    post: &HotState,
    events: &[Event],
) -> Result<DrawKnowledge, HiddenRefusal> {
    observe_step_with(
        reshuffle_places_known_cards(catalog),
        pre,
        know,
        post,
        events,
    )
}

fn observe_step_with(
    reshuffle_places_known: bool,
    pre: &HotState,
    know: &DrawKnowledge,
    post: &HotState,
    events: &[Event],
) -> Result<DrawKnowledge, HiddenRefusal> {
    let before = draw_uids(pre);
    let after = draw_uids(post);
    let reshuffled = events
        .iter()
        .any(|event| matches!(event, Event::Reshuffled { .. }));
    let entered: Vec<u32> = after
        .iter()
        .copied()
        .filter(|uid| !before.contains(uid))
        .collect();
    if reshuffled {
        if reshuffle_places_known {
            return Err(HiddenRefusal::ReshufflePlacesKnownCards);
        }
        return Ok(DrawKnowledge::none());
    }
    // Known cards still in the pile keep their relative order; cards that left
    // (drawn, selected out, exhausted) leave the knowledge.
    let top: Vec<u32> = know
        .top
        .iter()
        .copied()
        .filter(|uid| after.contains(uid))
        .collect();
    let bottom: Vec<u32> = know
        .bottom
        .iter()
        .copied()
        .filter(|uid| after.contains(uid))
        .collect();
    let random_insert = post.rng.get(RngStream::Rng).counter != pre.rng.get(RngStream::Rng).counter;
    if entered.is_empty() {
        let next = DrawKnowledge { top, bottom };
        if random_insert && !next.is_empty() {
            return Err(HiddenRefusal::RngConsumedOverKnownPositions);
        }
        check_knowledge(post, &next)?;
        return Ok(next);
    }
    if random_insert {
        if !top.is_empty() || !bottom.is_empty() {
            return Err(HiddenRefusal::RandomInsertOverKnownPositions);
        }
        // Entering cards are unknown; nothing else is known.
        return Ok(DrawKnowledge::none());
    }
    // Deterministic placements: every entering card must extend the known
    // top run or the known bottom run.
    let top_run = after
        .iter()
        .take_while(|uid| entered.contains(uid) || top.contains(uid))
        .count();
    let bottom_run = after
        .iter()
        .rev()
        .take_while(|uid| entered.contains(uid) || bottom.contains(uid))
        .count();
    let next = if top_run + bottom_run >= after.len() {
        // Every card's position is known; record it all from the top.
        DrawKnowledge {
            top: after.clone(),
            bottom: Vec::new(),
        }
    } else {
        DrawKnowledge {
            top: after[..top_run].to_vec(),
            bottom: after[after.len() - bottom_run..].to_vec(),
        }
    };
    if entered
        .iter()
        .any(|uid| !next.top.contains(uid) && !next.bottom.contains(uid))
    {
        return Err(HiddenRefusal::UnexplainedDrawInsertion);
    }
    check_knowledge(post, &next)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hot::{HotCard, HotPile};

    fn with_draw(uids: &[u32]) -> HotState {
        let mut state = HotState::at_defaults();
        state.piles.set(
            PileId::Draw,
            HotPile::from_cards(
                uids.iter()
                    .map(|&uid| HotCard {
                        uid,
                        atom: Default::default(),
                        flags: 0,
                    })
                    .collect(),
            ),
        );
        for stream in STREAMS {
            state.rng.set(
                stream,
                RngStreamState {
                    words: [7, 8, 9, 10],
                    counter: 3,
                },
            );
        }
        state
    }

    #[test]
    fn a_receipt_key_ignores_hidden_order_and_words_inside_the_predecessor() {
        use serde_json::json;
        let doc = |draw: [u64; 3], words: u64, pred_draw: [u64; 3], pred_words: u64| {
            json!({
                "piles": {"draw": draw.map(|uid| json!({"id": "STRIKE", "uid": uid})),
                          "hand": [{"id": "DEFEND", "uid": 9}]},
                "rng": {"ai": {"counter": 2, "words": [words, 1, 2, 3]}},
                "continuations": [{
                    "tag": "action_replay",
                    "predecessor": {
                        "piles": {"draw": pred_draw.map(|uid| json!({"id": "STRIKE", "uid": uid}))},
                        "rng": {"ai": {"counter": 1, "words": [pred_words, 4, 5, 6]}},
                    },
                    "action": {"kind": "end"},
                    "answers": [],
                }],
            })
        };
        let none = DrawKnowledge::none();
        let a = document_key(doc([1, 2, 3], 7, [1, 2, 3], 8), &none).unwrap();
        let b = document_key(doc([3, 1, 2], 70, [2, 3, 1], 80), &none).unwrap();
        assert_eq!(a, b, "hidden order and words, outer and nested, are erased");
        // Visible differences still separate information sets: a counter,
        // and a known top position.
        let mut counted = doc([1, 2, 3], 7, [1, 2, 3], 8);
        counted["rng"]["ai"]["counter"] = json!(3);
        assert_ne!(a, document_key(counted, &none).unwrap());
        let top = DrawKnowledge {
            top: vec![3],
            bottom: vec![],
        };
        assert_ne!(
            document_key(doc([3, 1, 2], 7, [1, 2, 3], 8), &top).unwrap(),
            document_key(doc([1, 3, 2], 7, [1, 2, 3], 8), &top).unwrap(),
            "a different known top card is a different information set"
        );
    }

    /// `HotState::without_action_replay_root` is the exact inverse of
    /// `install_action_replay_root` on every receipt-carrying state reached
    /// by seeded walks over action-replay eval fixtures, and the walks reach
    /// some (the fast receipt identity in `ismcts` rests on it).
    #[test]
    fn lifting_a_receipt_out_and_back_in_is_the_identity() {
        use crate::boundary::HotBoundary;
        use crate::canonical::CanonicalStateV2;
        let fights = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../eval/fights");
        let (mut lifted, mut receipts) = (0usize, 0usize);
        let mut legal = crate::engine::LegalActionBuffer::new();
        let mut events = Vec::new();
        for id in [
            "f232d07ae4f96a5b",
            "f960d49055bd158f",
            "fdeafdfe845c3ff1",
            "f59ded663025a445",
        ] {
            let document: CanonicalStateV2 = serde_json::from_slice(
                &std::fs::read(fights.join(id).join("entry.canonical.json")).unwrap(),
            )
            .unwrap();
            let catalog = HotBoundary::catalog_from_canonical(&document).unwrap();
            let root = HotBoundary::from_canonical(&document, &catalog).unwrap();
            for walk in 0..12u64 {
                let mut rng = PolicyRng(walk);
                let mut state = root.clone();
                for _ in 0..60 {
                    if state.history.over {
                        break;
                    }
                    let actions = crate::engine::legal_actions_into(&state, &catalog, &mut legal);
                    if actions.is_empty() {
                        break;
                    }
                    let action = actions[rng.below(actions.len())];
                    let Ok(next) =
                        crate::engine::apply_action_into(&state, &catalog, &action, &mut events)
                    else {
                        break;
                    };
                    state = next;
                    if !has_replay_receipt(&state) {
                        continue;
                    }
                    receipts += 1;
                    if let Some((rest, record)) = state.without_action_replay_root() {
                        assert!(!has_replay_receipt(&rest));
                        let mut back = rest;
                        assert_eq!(back.install_action_replay_root(&record), Some(()));
                        assert!(back == state, "{id}: lift and reinstall changed the state");
                        lifted += 1;
                    }
                }
            }
        }
        eprintln!("receipt lift: {lifted} of {receipts} receipt states round-tripped");
        assert!(receipts > 20, "the walks reached only {receipts} receipts");
        // Every receipt lifts: a failure would silently send the search back
        // to the slow document key.
        assert_eq!(lifted, receipts, "a receipt did not lift out");
    }

    #[test]
    fn sampling_keeps_the_multiset_and_the_known_positions() {
        let state = with_draw(&[5, 1, 9, 3, 7, 2]);
        let know = DrawKnowledge {
            top: vec![5],
            bottom: vec![2],
        };
        let mut rng = PolicyRng(11);
        let mut orders = std::collections::BTreeSet::new();
        for _ in 0..200 {
            let world = sample_world(&state, &know, &mut rng);
            let draw = draw_uids(&world);
            assert_eq!(draw.first(), Some(&5));
            assert_eq!(draw.last(), Some(&2));
            let mut sorted = draw.clone();
            sorted.sort_unstable();
            assert_eq!(sorted, [1, 2, 3, 5, 7, 9]);
            for stream in STREAMS {
                let live = world.rng.get(stream);
                assert_eq!(live.counter, 3, "counters are kept");
                assert_ne!(live.words, [7, 8, 9, 10], "words are resampled");
                assert_ne!(live.words, [0; 4]);
            }
            orders.insert(draw);
        }
        assert_eq!(
            orders.len(),
            24,
            "every order of the 4 unknown cards occurs"
        );
    }

    #[test]
    fn the_key_and_the_sample_ignore_the_true_hidden_order_and_words() {
        let know = DrawKnowledge::none();
        let a = with_draw(&[1, 2, 3, 4]);
        let mut b = with_draw(&[3, 1, 4, 2]);
        b.rng.set(
            RngStream::Ai,
            RngStreamState {
                words: [1, 1, 1, 1],
                counter: 3,
            },
        );
        assert_eq!(info_key(&a, &know), info_key(&b, &know));
        assert_eq!(
            sample_world(&a, &know, &mut PolicyRng(5)),
            sample_world(&b, &know, &mut PolicyRng(5))
        );
        // A different counter or a different known position is a different
        // observation.
        let mut c = a.clone();
        c.rng.set(
            RngStream::Ai,
            RngStreamState {
                words: [7, 8, 9, 10],
                counter: 4,
            },
        );
        assert_ne!(info_key(&a, &know), info_key(&c, &know));
        let top = DrawKnowledge {
            top: vec![1],
            bottom: vec![],
        };
        assert_ne!(info_key(&a, &know), info_key(&a, &top));
    }

    #[test]
    fn knowledge_follows_draws_and_deterministic_placements() {
        let pre = with_draw(&[1, 2, 3, 4]);
        // A deterministic top placement (no Rng draw) is known.
        let post = with_draw(&[9, 1, 2, 3, 4]);
        let know = observe_step_with(false, &pre, &DrawKnowledge::none(), &post, &[]).unwrap();
        assert_eq!(know.top, [9]);
        assert!(know.bottom.is_empty());
        // Drawing the known card consumes the knowledge.
        let drawn = with_draw(&[1, 2, 3, 4]);
        let after_draw = observe_step_with(false, &post, &know, &drawn, &[]).unwrap();
        assert!(after_draw.is_empty());
        // A deterministic bottom placement is known too.
        let bottom = with_draw(&[1, 2, 3, 4, 8]);
        let know = observe_step_with(false, &pre, &DrawKnowledge::none(), &bottom, &[]).unwrap();
        assert_eq!(know.bottom, [8]);
        // A deterministic insert into the middle has no model position.
        let middle = with_draw(&[1, 2, 8, 3, 4]);
        assert_eq!(
            observe_step_with(false, &pre, &DrawKnowledge::none(), &middle, &[]),
            Err(HiddenRefusal::UnexplainedDrawInsertion)
        );
    }

    #[test]
    fn random_inserts_are_unknown_and_refuse_over_known_positions() {
        let pre = with_draw(&[1, 2, 3, 4]);
        let mut post = with_draw(&[1, 2, 8, 3, 4]);
        post.rng.set(
            RngStream::Rng,
            RngStreamState {
                words: [7, 8, 9, 10],
                counter: 4,
            },
        );
        assert_eq!(
            observe_step_with(false, &pre, &DrawKnowledge::none(), &post, &[]),
            Ok(DrawKnowledge::none())
        );
        // Even a random insert that lands on top is not knowledge.
        let mut on_top = with_draw(&[8, 1, 2, 3, 4]);
        on_top.rng = post.rng.clone();
        assert_eq!(
            observe_step_with(false, &pre, &DrawKnowledge::none(), &on_top, &[]),
            Ok(DrawKnowledge::none())
        );
        let known = DrawKnowledge {
            top: vec![1],
            bottom: vec![],
        };
        assert_eq!(
            observe_step_with(false, &pre, &known, &post, &[]),
            Err(HiddenRefusal::RandomInsertOverKnownPositions)
        );
    }

    #[test]
    fn a_reshuffle_clears_knowledge_and_refuses_under_perfect_fit() {
        let mut pre = with_draw(&[1]);
        pre.piles.set(
            PileId::Discard,
            HotPile::from_cards(vec![
                HotCard {
                    uid: 4,
                    atom: Default::default(),
                    flags: 0,
                },
                HotCard {
                    uid: 5,
                    atom: Default::default(),
                    flags: 0,
                },
            ]),
        );
        let known = DrawKnowledge {
            top: vec![1],
            bottom: vec![],
        };
        let reshuffle = [Event::Reshuffled { cards: 2 }];
        let post = with_draw(&[5, 4]);
        assert_eq!(
            observe_step_with(false, &pre, &known, &post, &reshuffle),
            Ok(DrawKnowledge::none())
        );
        assert_eq!(
            observe_step_with(true, &pre, &known, &post, &reshuffle),
            Err(HiddenRefusal::ReshufflePlacesKnownCards)
        );
    }
}

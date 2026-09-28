//! The one continuation-frame enum (PORT_PLAN.md D3).
//!
//! The v0.110.1 kernel implemented continuations twice, once per encounter
//! family. This is the single generic replacement: one enum, one variant per
//! Python continuation-frame class, held in a copy-on-write stack beside the
//! hot state.
//!
//! # Name mapping
//!
//! Mechanical, in both directions, with no per-frame table:
//!
//! * the canonical wire tag is the Python class name, because
//!   `project_state.py::_project_named_tuple` emits `type(value).__name__`;
//! * the [`FrameKind`] / [`Frame`] variant name is that class name with the
//!   trailing `Frame` removed — `JossSideEndFrame` ⇄ `JossSideEnd`,
//!   `FrozenAutoBatchFrame` ⇄ `FrozenAutoBatch`.
//!
//! [`FrameKind::NAMES`] is the class-name table, ascending, so parsing is a
//! binary search and rendering is an array index — the same convention the
//! generated `ids.rs` uses. `_validate_continuation_frame` exhaustively checks
//! the 37 `NamedTuple` frame classes (frozen Python, deleted #2827); its tag dispatch
//! begins. Those function-owned points are the durable inventory
//! seam.
//!
//! # The one tag with no class
//!
//! `_FRAME_PHASE` is a bare 7-tuple rather than a `NamedTuple`; the
//! `_validate_continuation_frame` phase arm begins (frozen Python, deleted #2827). The canonical
//! projector gives that positional exception the synthetic wire type
//! `PhaseFrame`, with fields named from the validator/unpacking order. R19
//! adds that same synthetic name to [`FrameKind`] and inhabits only the exact
//! AutoPost/normal/end-turn quotient needed by Stampede. Every other Phase
//! vocabulary remains a typed boundary refusal.
//!
//! # Representation, and refusal instead of width
//!
//! A variant is *inhabited* only where every field of the Python class maps
//! exactly onto interned ids and small integers. Everything else is declared
//! with the uninhabited [`Deferred`] payload: the variant exists so the
//! inventory is complete and so R0.5 fills it in a reviewable diff, but no
//! code — here or in the engine — can construct a lossy instance of it, and
//! `match` arms over it are statically unreachable. The boundary answers a
//! deferred frame with a typed refusal.
//!
//! Ten variants are inhabited in the current inventory. Each carries a closed
//! vocabulary the Python validator pins by literal enumeration, cited per
//! stage enum below; nothing here is inferred from a sampled corpus.

use crate::hot::WordRecordIndex;

/// The payload of a frame variant the hot representation does not model yet.
///
/// Uninhabited on purpose. A `Frame::Draw(_)` value cannot be built, so
/// the variant costs no size, admits no silent field loss, and turns every
/// handler for it into a compile-checked `match x {}`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Deferred {}

macro_rules! frame_kinds {
    ($(($variant:ident, $class:literal)),+ $(,)?) => {
        /// The wire identity of a continuation frame: one per Python class.
        #[repr(u8)]
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum FrameKind {
            $($variant),+
        }

        impl FrameKind {
            /// Number of variants.
            pub const COUNT: usize = [$(stringify!($variant)),+].len();

            /// Python class names, ascending — the binary-search table.
            pub const NAMES: [&'static str; Self::COUNT] = [$($class),+];

            /// Every variant, in discriminant order.
            pub const ALL: [FrameKind; Self::COUNT] = [$(FrameKind::$variant),+];

            /// The Python class name of this variant.
            pub const fn as_str(&self) -> &'static str {
                Self::NAMES[*self as usize]
            }

            /// Parse a Python class name. `O(log COUNT)`, no allocation.
            #[allow(clippy::should_implement_trait)]
            pub fn from_str(name: &str) -> Option<Self> {
                match Self::NAMES.binary_search(&name) {
                    Ok(index) => Some(Self::ALL[index]),
                    Err(_) => None,
                }
            }
        }
    };
}

frame_kinds![
    (ActionReplay, "ActionReplayFrame"),
    (AfterCardDrawnPower, "AfterCardDrawnPowerFrame"),
    (AfterCardExhaustedPower, "AfterCardExhaustedPowerFrame"),
    (AfterCardPlayed, "AfterCardPlayedFrame"),
    (AfterCardPlayedRelic, "AfterCardPlayedRelicFrame"),
    (AfterPowerAmountChanged, "AfterPowerAmountChangedFrame"),
    (AfterSideTurnEndPower, "AfterSideTurnEndPowerFrame"),
    (BeforeHandDrawPower, "BeforeHandDrawPowerFrame"),
    (BeforeHandDrawRelic, "BeforeHandDrawRelicFrame"),
    (CardFinish, "CardFinishFrame"),
    (CardPlay, "CardPlayFrame"),
    (CardSelector, "CardSelectorFrame"),
    (CompactTransform, "CompactTransformFrame"),
    (ConsumingShadowSideEnd, "ConsumingShadowSideEndFrame"),
    (DarkEmbraceSideEnd, "DarkEmbraceSideEndFrame"),
    (Draw, "DrawFrame"),
    (EnemyPhase, "EnemyPhaseFrame"),
    (FixedTransform, "FixedTransformFrame"),
    (FrozenAutoBatch, "FrozenAutoBatchFrame"),
    (GeneratedCardPower, "GeneratedCardPowerFrame"),
    (GeneratedCardRelic, "GeneratedCardRelicFrame"),
    (JossExhaust, "JossExhaustFrame"),
    (JossSideEnd, "JossSideEndFrame"),
    (LiveListener, "LiveListenerFrame"),
    (LoseMaxHp, "LoseMaxHpFrame"),
    (MonsterAttack, "MonsterAttackFrame"),
    (MonsterMove, "MonsterMoveFrame"),
    (Outbreak, "OutbreakFrame"),
    (PaelsEyeEnd, "PaelsEyeEndFrame"),
    (Phase, "PhaseFrame"),
    (PlayerAttack, "PlayerAttackFrame"),
    (PlayerDamageSuffix, "PlayerDamageSuffixFrame"),
    (PotionFinish, "PotionFinishFrame"),
    (Puzzle, "PuzzleFrame"),
    (RandomOrb, "RandomOrbFrame"),
    (RelicReturn, "RelicReturnFrame"),
    (SideSwitch, "SideSwitchFrame"),
    (TestSubjectAttack, "TestSubjectAttackFrame"),
    (ToastyMittens, "ToastyMittensFrame"),
    (TurnEnd, "TurnEndFrame"),
    (TurnStart, "TurnStartFrame"),
    (TurnStartHandChoice, "TurnStartHandChoiceFrame"),
];

macro_rules! stage_enum {
    ($(#[$meta:meta])* $name:ident { $(($variant:ident, $wire:literal)),+ $(,)? }) => {
        $(#[$meta])*
        #[repr(u8)]
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $($variant),+
        }

        impl $name {
            /// Number of variants.
            pub const COUNT: usize = [$(stringify!($variant)),+].len();
            /// Canonical stage names, ascending — the binary-search table.
            pub const NAMES: [&'static str; Self::COUNT] = [$($wire),+];
            /// Every variant, in discriminant order.
            pub const ALL: [$name; Self::COUNT] = [$($name::$variant),+];

            /// The canonical name of this variant.
            pub const fn as_str(&self) -> &'static str {
                Self::NAMES[*self as usize]
            }

            /// Parse a canonical name. `O(log COUNT)`, no allocation.
            #[allow(clippy::should_implement_trait)]
            pub fn from_str(name: &str) -> Option<Self> {
                match Self::NAMES.binary_search(&name) {
                    Ok(index) => Some(Self::ALL[index]),
                    Err(_) => None,
                }
            }
        }
    };
}

stage_enum! {
    /// `TurnStartFrame.stage`.
    ///
    /// Vocabulary pinned by `_validate_continuation_frame`'s turn-start arm:
    /// `frame.stage not in ("after_royal", "after_inferno", "after_crimson")`
    /// raises. The set is closed by the validator, not sampled.
    TurnStartStage {
        (AfterCrimson, "after_crimson"),
        (AfterInferno, "after_inferno"),
        (AfterRoyal, "after_royal"),
    }
}

stage_enum! {
    /// `ToastyMittensFrame.stage`.
    ///
    /// The validator admits the single value `"after_exhaust"`
    /// (`frame.stage != "after_exhaust"` raises), so the enum has one
    /// variant and the field is a placeholder for a vocabulary that may grow.
    ToastyMittensStage {
        (AfterExhaust, "after_exhaust"),
    }
}

stage_enum! {
    /// `PotionFinishFrame.stage`.
    ///
    /// Vocabulary pinned by the validator's potion-finish arm:
    /// `frame.stage not in ("effect", "top", "after_top")` raises.
    PotionFinishStage {
        (AfterTop, "after_top"),
        (Effect, "effect"),
        (Top, "top"),
    }
}

stage_enum! {
    /// `CardFinishFrame.stage` in the admitted result-route / Unceasing Top
    /// quotient. `pre_top` proves result routing has completed and waits for
    /// an AfterCardExhausted owner before Unceasing Top runs.
    CardFinishStage {
        (AfterTop, "after_top"),
        (PreTop, "pre_top"),
        (Route, "route"),
    }
}

/// One resumable engine frame.
///
/// Inhabited variants carry the whole Python field inventory of their class
/// (minus `tag`, which [`FrameKind`] already is). Deferred variants carry
/// [`Deferred`] and cannot exist.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Frame {
    /// Independently rooted whole-action replay witness for a persisted batch.
    ActionReplay {
        /// Stable start of this frame's self-describing replay record.
        record: WordRecordIndex,
    },
    /// Frozen ordinary AfterCardExhausted listener walk and exact producer
    /// return cursor. The record owns a suspended Dark Embrace Draw after the
    /// physical CardPlay frame may already have been popped.
    AfterCardExhaustedPower {
        /// Stable start of the self-describing word record.
        record: WordRecordIndex,
    },
    /// One frozen acquisition-ordered ordinary AfterCardDrawn listener walk.
    AfterCardDrawnPower {
        /// Stable start of this frame's self-describing word record.
        record: WordRecordIndex,
    },
    /// Deferred: latch tuples and a card payload.
    AfterCardPlayed(Deferred),
    /// Deferred: latch tuples, a card reference tuple, and an object slot.
    AfterCardPlayedRelic(Deferred),
    /// One frozen acquisition-ordered AfterPowerAmountChanged listener walk.
    AfterPowerAmountChanged {
        /// Stable start of this frame's self-describing word record.
        record: WordRecordIndex,
    },
    /// Frozen local-player object subwalk inside ordinary AfterSideTurnEnd.
    AfterSideTurnEndPower {
        /// Stable start of the self-describing object snapshot and cursor.
        record: WordRecordIndex,
    },
    /// Frozen `BeforeHandDraw` power walk parked at Foregone Conclusion's
    /// modal `FromCombatPile` selection.
    BeforeHandDrawPower {
        /// Stable start of the self-describing Foregone word record.
        record: WordRecordIndex,
    },
    /// Deferred: a generated-uid tuple and an open-vocabulary caller.
    BeforeHandDrawRelic(Deferred),
    /// Result-routed card wrapper retained across Unceasing Top's Draw.
    CardFinish {
        /// Stable start of the self-describing finish record.
        record: WordRecordIndex,
    },
    /// A persistent card play whose complete resumable state lives in the
    /// authenticated Frames-owned word record at `record`.
    CardPlay {
        /// Stable start of this frame's self-describing `CardPlay` record.
        record: WordRecordIndex,
    },
    /// Deferred: an open-vocabulary selector name.
    CardSelector(Deferred),
    /// Deferred: original-card payloads and result-uid tuples.
    CompactTransform(Deferred),
    /// Consuming Shadow's `AfterSideTurnEnd` suffix.
    ConsumingShadowSideEnd {
        /// `ConsumingShadowSideEndFrame.auth_uid`.
        auth_uid: u32,
    },
    /// Dark Embrace's deferred true-Ethereal `AfterSideTurnEnd` Draw suffix.
    /// Field-free in Python too; the rooted predecessor authenticates the
    /// complete turn-end prefix which produced the live private tally.
    DarkEmbraceSideEnd {
        /// The lower AfterSideTurnEndPower record retaining this exact old
        /// Dark Embrace object while its Draw child is parked.
        record: WordRecordIndex,
    },
    /// One suspended `CardPileCmd.Draw` whose complete loop cursor and
    /// immutable result prefix live in the authenticated word arena.
    Draw {
        /// Stable start of this frame's self-describing `Draw` record.
        record: WordRecordIndex,
    },
    /// One exact Knowledge Demon choice cursor parked inside the enemy ACT
    /// snapshot. Its actor and remaining UID tail live in the authenticated
    /// word record at `record`.
    EnemyPhase {
        /// Stable start of this frame's self-describing `EnemyPhase` record.
        record: WordRecordIndex,
    },
    /// Deferred: a selection-uid tuple and a source-key tuple.
    FixedTransform(Deferred),
    /// One authenticated frozen-card AutoBatch in the frame word arena.
    FrozenAutoBatch {
        /// Stable start of this frame's self-describing record.
        record: WordRecordIndex,
    },
    /// Deferred: listener tuples, a card payload, and object slots.
    GeneratedCardPower(Deferred),
    /// Deferred: a card payload and a listener-uid tuple.
    GeneratedCardRelic(Deferred),
    /// Deferred: a midnight-uid tuple.
    JossExhaust(Deferred),
    /// Joss Paper's `AfterSideTurnEnd` suffix. Field-free in Python too.
    JossSideEnd,
    /// Stampede's persistent live Hand-refilter loop.
    LiveListener {
        /// Stable start of this frame's self-describing record.
        record: WordRecordIndex,
    },
    /// Deferred: an open-vocabulary damage source.
    LoseMaxHp(Deferred),
    /// Deferred: a monster reference tuple and an open-vocabulary move name.
    MonsterAttack(Deferred),
    /// Deferred: monster and move reference tuples.
    MonsterMove(Deferred),
    /// Deferred: target tuples and a source-key tuple.
    Outbreak(Deferred),
    /// The persistent AutoPost normal listener snapshot.
    Phase {
        /// Stable start of this frame's self-describing record.
        record: WordRecordIndex,
    },
    /// Deferred: a remaining-uid tuple.
    PaelsEyeEnd(Deferred),
    /// Deferred: target/damage/result tuples and object slots.
    PlayerAttack(Deferred),
    /// Deferred: an Inferno target tuple.
    PlayerDamageSuffix(Deferred),
    /// `PotionModel.OnUseWrapper`'s effect-depth and post-body epilogue.
    PotionFinish {
        /// Stable start of the self-describing potion finish/body record.
        record: WordRecordIndex,
    },
    /// Deferred: a drawn-card tuple and caller locals.
    Puzzle(Deferred),
    /// Deferred: orb-kind vocabularies, an object payload, target tuples.
    RandomOrb(Deferred),
    /// Deferred: an open-vocabulary caller name.
    RelicReturn(Deferred),
    /// `SwitchSides` suffix after Disintegration's awaited Damage.
    /// Field-free in Python too.
    SideSwitch,
    /// Deferred: a monster reference tuple and a hit receipt.
    TestSubjectAttack(Deferred),
    /// The v0.110.1 `AfterPlayerTurnStart` suffix parked in an awaited
    /// Exhaust.
    ToastyMittens {
        /// `ToastyMittensFrame.card_uid`.
        card_uid: u32,
        /// `ToastyMittensFrame.stage`.
        stage: ToastyMittensStage,
    },
    /// Deferred: remaining-uid tuple.
    TurnEnd(Deferred),
    /// `StartTurn`'s cursor after one awaited self-Damage command.
    TurnStart {
        /// `TurnStartFrame.stage`.
        stage: TurnStartStage,
        /// `TurnStartFrame.crimson_block`.
        crimson_block: i32,
    },
    /// One Tools/Tyranny listener snapshot, pending choice, or serial effect.
    TurnStartHandChoice {
        /// Stable start of this frame's self-describing word record.
        record: WordRecordIndex,
    },
}

impl Frame {
    /// The wire identity of this frame.
    pub fn kind(&self) -> FrameKind {
        match *self {
            Frame::ActionReplay { .. } => FrameKind::ActionReplay,
            Frame::AfterCardExhaustedPower { .. } => FrameKind::AfterCardExhaustedPower,
            Frame::AfterCardDrawnPower { .. } => FrameKind::AfterCardDrawnPower,
            Frame::AfterCardPlayed(x) => match x {},
            Frame::AfterCardPlayedRelic(x) => match x {},
            Frame::AfterPowerAmountChanged { .. } => FrameKind::AfterPowerAmountChanged,
            Frame::AfterSideTurnEndPower { .. } => FrameKind::AfterSideTurnEndPower,
            Frame::BeforeHandDrawPower { .. } => FrameKind::BeforeHandDrawPower,
            Frame::BeforeHandDrawRelic(x) => match x {},
            Frame::CardFinish { .. } => FrameKind::CardFinish,
            Frame::CardPlay { .. } => FrameKind::CardPlay,
            Frame::CardSelector(x) => match x {},
            Frame::CompactTransform(x) => match x {},
            Frame::ConsumingShadowSideEnd { .. } => FrameKind::ConsumingShadowSideEnd,
            Frame::DarkEmbraceSideEnd { .. } => FrameKind::DarkEmbraceSideEnd,
            Frame::Draw { .. } => FrameKind::Draw,
            Frame::EnemyPhase { .. } => FrameKind::EnemyPhase,
            Frame::FixedTransform(x) => match x {},
            Frame::FrozenAutoBatch { .. } => FrameKind::FrozenAutoBatch,
            Frame::GeneratedCardPower(x) => match x {},
            Frame::GeneratedCardRelic(x) => match x {},
            Frame::JossExhaust(x) => match x {},
            Frame::JossSideEnd => FrameKind::JossSideEnd,
            Frame::LiveListener { .. } => FrameKind::LiveListener,
            Frame::LoseMaxHp(x) => match x {},
            Frame::MonsterAttack(x) => match x {},
            Frame::MonsterMove(x) => match x {},
            Frame::Outbreak(x) => match x {},
            Frame::Phase { .. } => FrameKind::Phase,
            Frame::PaelsEyeEnd(x) => match x {},
            Frame::PlayerAttack(x) => match x {},
            Frame::PlayerDamageSuffix(x) => match x {},
            Frame::PotionFinish { .. } => FrameKind::PotionFinish,
            Frame::Puzzle(x) => match x {},
            Frame::RandomOrb(x) => match x {},
            Frame::RelicReturn(x) => match x {},
            Frame::SideSwitch => FrameKind::SideSwitch,
            Frame::TestSubjectAttack(x) => match x {},
            Frame::ToastyMittens { .. } => FrameKind::ToastyMittens,
            Frame::TurnEnd(x) => match x {},
            Frame::TurnStart { .. } => FrameKind::TurnStart,
            Frame::TurnStartHandChoice { .. } => FrameKind::TurnStartHandChoice,
        }
    }
}

/// The `_FRAME_*` tag each inhabited kind carries in its Python `tag` field.
///
/// The tag is redundant with the class name on the wire (both are emitted),
/// so the boundary needs it to rebuild the field bag. Only inhabited kinds
/// have one here; a deferred kind never reaches the reconstruction path.
pub const fn frame_tag(kind: FrameKind) -> Option<&'static str> {
    match kind {
        FrameKind::ActionReplay => Some("action_replay"),
        FrameKind::AfterCardExhaustedPower => Some("after_card_exhausted_power_order"),
        FrameKind::AfterCardDrawnPower => Some("after_card_drawn_power_order"),
        FrameKind::AfterPowerAmountChanged => Some("after_power_amount_changed_power_order"),
        FrameKind::AfterSideTurnEndPower => Some("after_side_turn_end_power_order"),
        FrameKind::BeforeHandDrawPower => Some("before_hand_draw_power"),
        FrameKind::ConsumingShadowSideEnd => Some("consuming_shadow_side_end"),
        FrameKind::DarkEmbraceSideEnd => Some("dark_embrace_side_end"),
        FrameKind::CardFinish => Some("card_finish"),
        FrameKind::CardPlay => Some("card_play"),
        FrameKind::Draw => Some("draw_command"),
        FrameKind::EnemyPhase => Some("enemy_phase"),
        FrameKind::FrozenAutoBatch => Some("auto_batch"),
        FrameKind::JossSideEnd => Some("joss_side_end"),
        FrameKind::PotionFinish => Some("potion_finish"),
        FrameKind::LiveListener => Some("live_listener"),
        FrameKind::Phase => Some("phase"),
        FrameKind::SideSwitch => Some("side_switch"),
        FrameKind::ToastyMittens => Some("toasty_mittens"),
        FrameKind::TurnStart => Some("turn_start"),
        FrameKind::TurnStartHandChoice => Some("turn_start_hand_choice"),
        _ => None,
    }
}

/// One frame is an eight-byte marker/scalar record; choice-bound variable
/// payloads belong in the pending word arena rather than widening this enum.
const _: () = assert!(size_of::<Frame>() == 8);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_class_name_table_is_ascending_and_complete() {
        assert_eq!(FrameKind::COUNT, 42);
        assert!(FrameKind::NAMES.windows(2).all(|pair| pair[0] < pair[1]));
        for (index, kind) in FrameKind::ALL.iter().enumerate() {
            assert_eq!(*kind as usize, index);
            assert_eq!(kind.as_str(), FrameKind::NAMES[index]);
            assert_eq!(FrameKind::from_str(kind.as_str()), Some(*kind));
        }
    }

    #[test]
    fn every_class_name_ends_in_the_python_suffix() {
        for name in FrameKind::NAMES {
            assert!(name.ends_with("Frame"), "{name}");
        }
    }

    #[test]
    fn an_unknown_class_name_does_not_parse() {
        assert_eq!(FrameKind::from_str("FutureFrame"), None);
        assert_eq!(FrameKind::from_str(""), None);
    }

    #[test]
    fn every_inhabited_kind_has_a_tag_and_no_deferred_kind_does() {
        let inhabited = [
            Frame::ActionReplay {
                record: WordRecordIndex::from_raw_for_test(10),
            },
            Frame::AfterCardDrawnPower {
                record: WordRecordIndex::from_raw_for_test(19),
            },
            Frame::AfterCardExhaustedPower {
                record: WordRecordIndex::from_raw_for_test(24),
            },
            Frame::AfterPowerAmountChanged {
                record: WordRecordIndex::from_raw_for_test(21),
            },
            Frame::AfterSideTurnEndPower {
                record: WordRecordIndex::from_raw_for_test(22),
            },
            Frame::JossSideEnd,
            Frame::SideSwitch,
            Frame::ConsumingShadowSideEnd { auth_uid: 7 },
            Frame::DarkEmbraceSideEnd {
                record: WordRecordIndex::from_raw_for_test(23),
            },
            Frame::CardPlay {
                record: WordRecordIndex::from_raw_for_test(11),
            },
            Frame::CardFinish {
                record: WordRecordIndex::from_raw_for_test(20),
            },
            Frame::LiveListener {
                record: WordRecordIndex::from_raw_for_test(12),
            },
            Frame::Phase {
                record: WordRecordIndex::from_raw_for_test(13),
            },
            Frame::FrozenAutoBatch {
                record: WordRecordIndex::from_raw_for_test(14),
            },
            Frame::TurnStartHandChoice {
                record: WordRecordIndex::from_raw_for_test(15),
            },
            Frame::EnemyPhase {
                record: WordRecordIndex::from_raw_for_test(16),
            },
            Frame::ToastyMittens {
                card_uid: 3,
                stage: ToastyMittensStage::AfterExhaust,
            },
            Frame::TurnStart {
                stage: TurnStartStage::AfterCrimson,
                crimson_block: 4,
            },
            Frame::PotionFinish {
                record: WordRecordIndex::from_raw_for_test(17),
            },
            Frame::Draw {
                record: WordRecordIndex::from_raw_for_test(18),
            },
            Frame::BeforeHandDrawPower {
                record: WordRecordIndex::from_raw_for_test(25),
            },
        ];
        let tagged: Vec<FrameKind> = FrameKind::ALL
            .into_iter()
            .filter(|kind| frame_tag(*kind).is_some())
            .collect();
        let built: Vec<FrameKind> = inhabited.iter().map(Frame::kind).collect();
        assert_eq!(built.len(), 21);
        assert_eq!(FrameKind::COUNT - built.len(), 21);
        assert_eq!(tagged.len(), built.len());
        for kind in &built {
            assert!(tagged.contains(kind), "{kind:?}");
        }
    }

    #[test]
    fn puzzle_is_a_compile_time_deferred_example() {
        fn puzzle_payload(frame: Frame) -> Deferred {
            match frame {
                Frame::Puzzle(payload) => payload,
                _ => unreachable!(),
            }
        }

        let _: fn(Frame) -> Deferred = puzzle_payload;
        assert_eq!(frame_tag(FrameKind::Puzzle), None);
    }

    #[test]
    fn stage_vocabularies_are_ascending_and_round_trip() {
        assert!(
            TurnStartStage::NAMES
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        assert!(
            PotionFinishStage::NAMES
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        for stage in TurnStartStage::ALL {
            assert_eq!(TurnStartStage::from_str(stage.as_str()), Some(stage));
        }
        for stage in PotionFinishStage::ALL {
            assert_eq!(PotionFinishStage::from_str(stage.as_str()), Some(stage));
        }
        for stage in ToastyMittensStage::ALL {
            assert_eq!(ToastyMittensStage::from_str(stage.as_str()), Some(stage));
        }
        assert_eq!(TurnStartStage::from_str("after_nothing"), None);
    }

    #[test]
    fn a_frame_fits_its_budget_and_copies() {
        assert_eq!(size_of::<Frame>(), 8);
        let frame = Frame::TurnStart {
            stage: TurnStartStage::AfterRoyal,
            crimson_block: 9,
        };
        let copied = frame;
        assert_eq!(frame, copied);
    }

    #[test]
    fn deferred_variants_cost_nothing() {
        assert_eq!(size_of::<Deferred>(), 0);
        // Deferred variants add no payload width.
        assert_eq!(size_of::<Frame>(), 8);
    }
}

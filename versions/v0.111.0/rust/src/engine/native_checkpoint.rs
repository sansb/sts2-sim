//! Opt-in capture of the native checkpoints one wire action spans (#3242).
//!
//! A Rust wire action is one `apply`, but native can log a completed-action
//! checkpoint (`finished action execution …`) partway through the same work.
//! The `.mcr` certification census compares each such checkpoint against a
//! Rust state, so a checkpoint that falls inside one `apply` has to be
//! observed at its own boundary, not approximated by the state the wire ends
//! in. Two boundaries are inhabited:
//!
//! - [`NativeCheckpointKind::VoidFormEndTurnRequest`]: Void Form's OnPlay
//!   (`VoidForm/<OnPlay>d__5::MoveNext` RVA `0x3c69e8` IL_0131) calls
//!   `PlayerCmd::EndTurn` (RVA `0x13362f`), which only calls
//!   `SetReadyToEndTurn` (IL_0029). The PlayCardAction therefore finishes,
//!   and native writes its checkpoint, before the turn ends; Rust services
//!   the request inside the same `apply` (`service_void_form_end_turn`).
//! - [`NativeCheckpointKind::AutoPostHookFinished`]:
//!   `CombatManager/<EndPlayerTurnPhaseOneInternal>d__130::MoveNext` RVA
//!   `0x3f4a4c` sets Phase 4 (IL_01f1), builds ONE `HookPlayerChoiceContext`
//!   (IL_020c) for the whole `Hook.AfterAutoPostPlayPhaseEntered` dispatch
//!   (IL_022d; `Hook/<AfterAutoPostPlayPhaseEntered>d__57` RVA `0x3cb794`
//!   walks every listener under that one context) and waits for it
//!   (IL_023c, IL_0338) before the rest of the turn end. When a listener's
//!   task yields — Stampede's `CardCmd::AutoPlay`
//!   (`StampedePower/<AfterAutoPostPlayPhaseEntered>d__4` RVA `0x345e94`
//!   IL_00bf), Howl from Beyond, I Am Invincible — the context runs as a
//!   `GenericHookGameAction` whose checkpoint is written when the whole
//!   dispatch completes. Rust's boundary is the AutoPost Phase exhausting
//!   its listener list, before `finish_player_turn_after_auto_post`.
//!
//! One more boundary sits inside the opening's deal rather than an `apply`
//! (#3392):
//!
//! - [`NativeCheckpointKind::AfterPlayerTurnStart`]:
//!   `CombatManager/<StartTurn>d__100::MoveNext` RVA `0x3f781c` runs
//!   `Hook.AfterSideTurnStart` (IL_07bf) and each player's
//!   `OrbQueue.AfterTurnStart` (IL_08b5), writes the "After player turn
//!   start" checksum (IL_096f `ldstr`, IL_0975 `GenerateChecksum`), and only
//!   then calls `RunAutoPrePlayPhase` (IL_0b1e). So a turn-one AutoPre that
//!   moves state (Imbued's AutoPlay, #3381) runs AFTER the capture's opening
//!   checksum, while the opening Rust hands back has already run it. The
//!   boundary is observed only on turn one: the census compares no later
//!   turn-start checksum, so no `apply` ever reports it.
//!
//! Recording is off unless a caller wraps a transition in [`record`]; the
//! observation sites are cold paths only, so search and ordinary traffic pay
//! one thread-local read on the rare turns that reach them and nothing else.
//! Recording observes; it never changes the transition.

use std::cell::RefCell;

use crate::hot::HotState;

/// Which native checkpoint boundary a recorded state sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeCheckpointKind {
    /// The state after a Void Form play, before its requested EndTurn.
    VoidFormEndTurnRequest,
    /// The state after the whole AfterAutoPostPlayPhaseEntered dispatch.
    AutoPostHookFinished,
    /// Turn one's state at native's "After player turn start" checksum,
    /// before `RunAutoPrePlayPhase` (#3392).
    AfterPlayerTurnStart,
}

impl NativeCheckpointKind {
    /// The stable wire name `diff-serve` and `entry --native-checkpoints`
    /// report.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::VoidFormEndTurnRequest => "void_form_end_turn_request",
            Self::AutoPostHookFinished => "auto_post_hook_finished",
            Self::AfterPlayerTurnStart => "after_player_turn_start",
        }
    }
}

type Recorded = Vec<(NativeCheckpointKind, HotState)>;

thread_local! {
    static RECORDER: RefCell<Option<Recorded>> = const { RefCell::new(None) };
}

/// Run `transition` with recording on and return every boundary state it
/// passed, in order. Nested calls restore the outer recorder.
pub fn record<T>(transition: impl FnOnce() -> T) -> (T, Recorded) {
    let outer = RECORDER.with(|recorder| recorder.replace(Some(Vec::new())));
    let result = transition();
    let recorded = RECORDER
        .with(|recorder| recorder.replace(outer))
        .unwrap_or_default();
    (result, recorded)
}

/// Record `state` at `kind`'s boundary when a caller is recording.
#[cold]
#[inline(never)]
pub(crate) fn observe(kind: NativeCheckpointKind, state: &HotState) {
    RECORDER.with(|recorder| {
        if let Some(recorded) = recorder.borrow_mut().as_mut() {
            recorded.push((kind, state.clone()));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_outside_record_is_a_no_op_and_record_restores_the_outer_recorder() {
        let state = HotState::at_defaults();
        observe(NativeCheckpointKind::VoidFormEndTurnRequest, &state);
        let ((), outer) = record(|| {
            observe(NativeCheckpointKind::AutoPostHookFinished, &state);
            let ((), inner) = record(|| {
                observe(NativeCheckpointKind::VoidFormEndTurnRequest, &state);
            });
            assert_eq!(inner.len(), 1);
            assert_eq!(inner[0].0, NativeCheckpointKind::VoidFormEndTurnRequest);
            observe(NativeCheckpointKind::AutoPostHookFinished, &state);
        });
        let kinds: Vec<_> = outer.iter().map(|(kind, _)| *kind).collect();
        assert_eq!(
            kinds,
            [
                NativeCheckpointKind::AutoPostHookFinished,
                NativeCheckpointKind::AutoPostHookFinished,
            ]
        );
        let ((), after) = record(|| {});
        assert!(after.is_empty());
    }
}

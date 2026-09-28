//! Derived execution coverage: which kinds a run actually *exercised*.
//!
//! PORT_PLAN §4's coverage clause — "coverage is derived, not asserted" — needs
//! an engine-side answer to one question the differential cannot answer from
//! outside: *did this trajectory actually run that dispatch arm?* A smoke
//! config claiming a step kind is only evidence if some trajectory reached the
//! kind's body; otherwise a wave PR is green because untested, which is the
//! failure mode the gate exists to prevent.
//!
//! # What is recorded, and where
//!
//! | axis | recorded at | meaning |
//! |---|---|---|
//! | [`record_step`] | the one step-dispatch site (`engine::play::run_steps`) | a card step body ran |
//! | [`record_move`] | the one move-dispatch site (`engine::turn::monster_act`) | a monster move body ran |
//! | [`record_hook`] | [`crate::engine::fire_hook`] / `modifier_total` | a hook fire point was reached |
//! | [`record_relic`] | the table-driven relic interpreter | a relic rule with a satisfied guard ran |
//! | [`record_power`] | the transport, from [`crate::engine::Event::PowerChanged`] | a power's amount changed |
//!
//! The first four sit at dispatch, so they report what *ran*, never what a
//! table said might run — a card whose program short-circuits before a step
//! does not get credit for it. Powers are the deliberate exception: they have
//! no single dispatch site (a power is read by the pipeline and written by
//! whichever body owns it), so the honest engine-independent signal is the
//! `PowerChanged` event the transition already emits. "A power was exercised"
//! therefore means **its amount changed during the run**; a power that was only
//! read is not claimed. `diff-serve` feeds those events in through
//! [`record_events`].
//!
//! Recording is **off by default** and enabled per process by [`enable`]:
//! `sts-sim diff-serve` turns it on, everything else — searches, benchmarks,
//! the allocation pins — pays one relaxed atomic load per dispatch and nothing
//! else. Nothing here allocates per transition: the sets are fixed-size
//! bitsets in thread-local storage, allocated once when the thread first
//! records.
//!
//! Coverage is an *observation*, never an input: no engine decision reads it,
//! so enabling it cannot change a trajectory.
//!
//! This module is deliberately **not** in `hot_path_contract`'s `HOT_MODULES`,
//! for the same reason `boundary.rs` is exempt from the owned-text ban: the
//! recording path (the `record_*` functions) names no text and touches only
//! bitsets, while [`Coverage`]'s name lists are report-side, produced once per
//! trajectory at the wire boundary and never per node.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::engine::Event;
use crate::hooks::HookEvent;
use crate::ids::{MoveKind, PowerId, RelicId, StepKind};

/// Process-wide recording switch. Relaxed is right: the flag is set once
/// before any recording thread starts, and a missed record is impossible
/// because the recorder and the switch live in the same thread.
static ENABLED: AtomicBool = AtomicBool::new(false);

const fn words(count: usize) -> usize {
    count.div_ceil(64)
}

/// One thread's exercised sets.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Sets {
    steps: [u64; words(StepKind::COUNT)],
    moves: [u64; words(MoveKind::COUNT)],
    powers: [u64; words(PowerId::COUNT)],
    relics: [u64; words(RelicId::COUNT)],
    hooks: [u64; words(HookEvent::COUNT)],
    transitions: u64,
}

impl Sets {
    const fn new() -> Self {
        Self {
            steps: [0; words(StepKind::COUNT)],
            moves: [0; words(MoveKind::COUNT)],
            powers: [0; words(PowerId::COUNT)],
            relics: [0; words(RelicId::COUNT)],
            hooks: [0; words(HookEvent::COUNT)],
            transitions: 0,
        }
    }
}

thread_local! {
    static SETS: RefCell<Sets> = const { RefCell::new(Sets::new()) };
}

#[inline]
fn set_bit(words: &mut [u64], index: usize) {
    words[index / 64] |= 1u64 << (index % 64);
}

fn names<T: Copy>(words: &[u64], all: &[T], as_str: impl Fn(T) -> &'static str) -> Vec<String> {
    all.iter()
        .enumerate()
        .filter(|(index, _)| words[index / 64] & (1u64 << (index % 64)) != 0)
        .map(|(_, item)| as_str(*item).to_string())
        .collect()
}

/// Turn recording on for this process.
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

/// Whether recording is on.
#[inline]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Record that a card step body ran.
#[inline]
pub fn record_step(kind: StepKind) {
    if !is_enabled() {
        return;
    }
    SETS.with(|sets| set_bit(&mut sets.borrow_mut().steps, kind as usize));
}

/// Record that a monster move body ran.
#[inline]
pub fn record_move(kind: MoveKind) {
    if !is_enabled() {
        return;
    }
    SETS.with(|sets| set_bit(&mut sets.borrow_mut().moves, kind as usize));
}

/// Record that a hook fire point was reached (subscribers or not — reaching an
/// empty fire point is still evidence the turn structure ran through it).
#[inline]
pub fn record_hook(event: HookEvent) {
    if !is_enabled() {
        return;
    }
    SETS.with(|sets| set_bit(&mut sets.borrow_mut().hooks, event as usize));
}

/// Record that a power's amount changed.
#[inline]
pub fn record_power(power: PowerId) {
    if !is_enabled() {
        return;
    }
    SETS.with(|sets| set_bit(&mut sets.borrow_mut().powers, power as usize));
}

/// Record that a relic rule passed its guards and executed a body.
#[inline]
pub fn record_relic(relic: RelicId) {
    if !is_enabled() {
        return;
    }
    SETS.with(|sets| set_bit(&mut sets.borrow_mut().relics, relic as usize));
}

/// Record one completed transition and the powers its events changed.
///
/// The transport calls this once per applied action, outside the hot path.
pub fn record_events(events: &[Event]) {
    if !is_enabled() {
        return;
    }
    SETS.with(|sets| {
        let mut sets = sets.borrow_mut();
        sets.transitions += 1;
        for event in events {
            // Whose power changed does not matter to coverage: the question is
            // whether the power's body ran at all.
            if let Event::PowerChanged { power, .. } = event {
                set_bit(&mut sets.powers, *power as usize);
            }
        }
    });
}

/// Everything this thread has exercised since the last [`take`] / [`reset`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    /// Card step kinds whose bodies ran, in enum order.
    pub steps: Vec<String>,
    /// Monster move kinds whose bodies ran, in enum order.
    pub moves: Vec<String>,
    /// Powers whose amount changed, in enum order.
    pub powers: Vec<String>,
    /// Relics whose guarded bodies ran, in enum order.
    pub relics: Vec<String>,
    /// Hook events whose fire points were reached, in enum order.
    pub hooks: Vec<String>,
    /// Completed transitions the recorder saw.
    pub transitions: u64,
}

fn snapshot(sets: &Sets) -> Coverage {
    Coverage {
        steps: names(&sets.steps, &StepKind::ALL, |kind| kind.as_str()),
        moves: names(&sets.moves, &MoveKind::ALL, |kind| kind.as_str()),
        powers: names(&sets.powers, &PowerId::ALL, |power| power.as_str()),
        relics: names(&sets.relics, &RelicId::ALL, |relic| relic.as_str()),
        hooks: names(&sets.hooks, &HookEvent::ALL, |event| event.as_str()),
        transitions: sets.transitions,
    }
}

/// Read the exercised sets without clearing them.
pub fn report() -> Coverage {
    SETS.with(|sets| snapshot(&sets.borrow()))
}

/// Read the exercised sets and clear them — the per-trajectory form.
pub fn take() -> Coverage {
    SETS.with(|sets| {
        let mut sets = sets.borrow_mut();
        let coverage = snapshot(&sets);
        *sets = Sets::new();
        coverage
    })
}

/// Clear the exercised sets.
pub fn reset() {
    SETS.with(|sets| *sets.borrow_mut() = Sets::new());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Subject;

    /// Recording is process-global, so the tests that need it on run inside
    /// this one: two `#[test]`s toggling one switch in parallel would flake.
    #[test]
    fn recording_is_off_until_enabled_and_then_accumulates_until_taken() {
        // Off: nothing is recorded, and the report is empty.
        reset();
        record_step(StepKind::Attack);
        assert_eq!(report(), Coverage::default());

        enable();
        reset();
        record_step(StepKind::Attack);
        record_step(StepKind::Block);
        record_step(StepKind::Attack);
        record_move(MoveKind::SpitAttack);
        record_hook(HookEvent::AfterCardPlayed);
        record_power(PowerId::Vuln);
        record_relic(RelicId::RelicSai);
        record_events(&[Event::PowerChanged {
            subject: Subject::Monster(3),
            power: PowerId::Thorns,
            amount: 4,
        }]);

        let coverage = report();
        assert_eq!(coverage.steps, ["attack", "block"]);
        assert_eq!(coverage.moves, ["spit_attack"]);
        assert_eq!(coverage.powers, ["thorns", "vuln"]);
        assert_eq!(coverage.relics, [RelicId::RelicSai.as_str()]);
        assert_eq!(coverage.hooks, [HookEvent::AfterCardPlayed.as_str()]);
        assert_eq!(coverage.transitions, 1);

        // `take` reports then clears; a second take is empty.
        assert_eq!(take(), coverage);
        assert_eq!(take(), Coverage::default());

        // A kind never recorded never appears, which is the whole point: the
        // gate must be able to say "claimed but not exercised".
        record_step(StepKind::Vulnerable);
        assert_eq!(take().steps, ["vulnerable"]);

        ENABLED.store(false, Ordering::Relaxed);
        record_step(StepKind::Attack);
        assert_eq!(report(), Coverage::default());
    }

    #[test]
    fn the_bitsets_cover_every_variant_of_every_axis() {
        // A `words()` off-by-one would panic on the last variant rather than
        // silently dropping it, but the assertion is cheap and explicit.
        assert_eq!(words(StepKind::COUNT), StepKind::COUNT.div_ceil(64));
        assert!(words(StepKind::COUNT) * 64 >= StepKind::COUNT);
        assert!(words(MoveKind::COUNT) * 64 >= MoveKind::COUNT);
        assert!(words(PowerId::COUNT) * 64 >= PowerId::COUNT);
        assert_eq!(
            words(PowerId::COUNT),
            4,
            "PowerId coverage remains four words"
        );
        assert!(words(HookEvent::COUNT) * 64 >= HookEvent::COUNT);
        assert_eq!(StepKind::ALL.len(), StepKind::COUNT);
    }
}

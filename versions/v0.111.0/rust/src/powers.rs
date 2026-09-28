//! Sorted key/value slot vectors — the hot shape of powers and counters.
//!
//! PORT_PLAN.md D3: Python's 517 `State` fields and 106 `Monster` fields are
//! the *canonical* shape of powers, not the hot one. Hot entities carry
//! `O(powers active this fight)` sorted `(key, value)` slots instead, so a
//! power lookup is a binary search over a handful of cache-line-resident
//! entries and a whole entity's power set is one pointer.
//!
//! # Why copy-on-write rather than an inline small vector
//!
//! An inline cap-8 array of 8-byte slots costs 64+ bytes *per entity*. Every
//! [`crate::hot::HotMonster`] carries one, and monsters live in a
//! copy-on-write vector that is memcpy'd whenever any monster changes — so
//! the inline form would multiply the per-node copy cost by the roster size
//! for the sake of avoiding one atomic increment. `Arc<Vec<_>>` is 8 bytes,
//! clones in one increment regardless of how many powers are active, and
//! keeps both size budgets (`HotState` ≤ 208, `HotMonster` ≤ 96) comfortable.
//! Mutation goes through `Arc::make_mut`, exactly the kernel's pile pattern.
//!
//! # The wire tag
//!
//! Python distinguishes `0` from `False` on the wire (`project_state.py`
//! requires `type(value) is type(default)` for zero-default elision), and
//! roughly a sixth of the power-named fields are declared `bool`. Rather than
//! hand-transcribe a 166-entry int-or-bool table that could drift from
//! `combat_sim`, each slot records the tag it was admitted with. It is free:
//! `(K, i32)` for a `u16` key already pads to 8 bytes, and [`Slot`] spends
//! that padding byte instead of wasting it.

use std::sync::Arc;

/// How a slot's value appears in the canonical document.
///
/// Not a semantic property of the power — a faithful record of the JSON type
/// the boundary admitted, so the inverse projection restores it exactly.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum SlotWire {
    /// A JSON number; the canonical default is `0`.
    Int = 0,
    /// A JSON boolean; the canonical default is `false`.
    Bool = 1,
}

/// One `(key, value)` slot: an active power, or a relic-owned counter.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Slot<K> {
    /// The interned axis id (`PowerId`, `RelicId`, …).
    pub key: K,
    /// The canonical value tag.
    pub wire: SlotWire,
    /// The amount. Boolean slots hold `0` or `1`.
    pub value: i32,
}

/// An ascending-by-key, deduplicated, copy-on-write slot vector.
///
/// Ordering is an invariant of the type: every mutator restores it, so a
/// lookup is `O(log n)` and equality of two slot sets is a plain vector
/// comparison (no set semantics, no hashing, no iteration-order leak into a
/// digest).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slots<K>(Arc<Vec<Slot<K>>>);

impl<K> Default for Slots<K> {
    fn default() -> Self {
        Self(Arc::new(Vec::new()))
    }
}

impl<K: Copy + Ord> Slots<K> {
    /// An empty slot set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from an arbitrary iterator, sorting and rejecting duplicates.
    ///
    /// Returns `None` when a key repeats: two canonical fields cannot both
    /// claim one slot, and silently keeping the last would be exactly the
    /// drop this layer exists to prevent.
    pub fn from_unsorted(mut slots: Vec<Slot<K>>) -> Option<Self> {
        slots.sort_unstable_by_key(|slot| slot.key);
        if slots.windows(2).any(|pair| pair[0].key == pair[1].key) {
            return None;
        }
        Some(Self(Arc::new(slots)))
    }

    /// Ascending slots.
    pub fn as_slice(&self) -> &[Slot<K>] {
        self.0.as_slice()
    }

    /// Number of occupied slots.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no slot is occupied — the overwhelmingly common case.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The slot for `key`, if occupied. `O(log n)`.
    pub fn get(&self, key: K) -> Option<Slot<K>> {
        match self.0.binary_search_by(|slot| slot.key.cmp(&key)) {
            Ok(index) => Some(self.0[index]),
            Err(_) => None,
        }
    }

    /// The value for `key`, or `0` when the slot is vacant.
    ///
    /// Vacant and zero are the same state by construction: [`Self::set`]
    /// vacates a slot written to `0`, so there is exactly one representation
    /// of "this power is not active".
    pub fn value(&self, key: K) -> i32 {
        self.get(key).map_or(0, |slot| slot.value)
    }

    /// Write `key`, keeping the vector ascending; `0` vacates the slot.
    ///
    /// Copy-on-write: shared storage is cloned once, then mutated in place.
    pub fn set(&mut self, key: K, wire: SlotWire, value: i32) {
        let slots = Arc::make_mut(&mut self.0);
        match slots.binary_search_by(|slot| slot.key.cmp(&key)) {
            Ok(index) => {
                if value == 0 {
                    slots.remove(index);
                } else {
                    slots[index] = Slot { key, wire, value };
                }
            }
            Err(index) => {
                if value != 0 {
                    slots.insert(index, Slot { key, wire, value });
                }
            }
        }
    }
}

/// A slot vector is one pointer, whatever the key.
const _: () = assert!(size_of::<Slots<u16>>() == size_of::<usize>());
/// The wire tag rides in padding the key/value pair already wasted.
const _: () = assert!(size_of::<Slot<u16>>() == 8);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{PowerId, RelicId};

    fn int(key: PowerId, value: i32) -> Slot<PowerId> {
        Slot {
            key,
            wire: SlotWire::Int,
            value,
        }
    }

    #[test]
    fn an_empty_slot_set_reads_zero_everywhere() {
        let slots = Slots::<PowerId>::new();
        assert!(slots.is_empty());
        assert_eq!(slots.len(), 0);
        assert_eq!(slots.value(PowerId::Strength), 0);
        assert_eq!(slots.get(PowerId::Strength), None);
    }

    #[test]
    fn writes_keep_the_vector_ascending() {
        let mut slots = Slots::new();
        slots.set(PowerId::Weak, SlotWire::Int, 2);
        slots.set(PowerId::Artifact, SlotWire::Int, 1);
        slots.set(PowerId::Strength, SlotWire::Int, 3);
        let keys: Vec<PowerId> = slots.as_slice().iter().map(|s| s.key).collect();
        let mut sorted = keys.clone();
        sorted.sort_unstable();
        assert_eq!(keys, sorted);
        assert_eq!(slots.value(PowerId::Strength), 3);
    }

    #[test]
    fn writing_zero_vacates_the_slot() {
        let mut slots = Slots::new();
        slots.set(PowerId::Vigor, SlotWire::Int, 5);
        assert_eq!(slots.len(), 1);
        slots.set(PowerId::Vigor, SlotWire::Int, 0);
        assert!(slots.is_empty());
        // Vacating an already-vacant slot is a no-op, not an insertion of 0.
        slots.set(PowerId::Vigor, SlotWire::Int, 0);
        assert!(slots.is_empty());
    }

    #[test]
    fn a_write_does_not_disturb_an_existing_clone() {
        let mut original = Slots::new();
        original.set(PowerId::Poison, SlotWire::Int, 4);
        let snapshot = original.clone();
        original.set(PowerId::Poison, SlotWire::Int, 9);
        assert_eq!(snapshot.value(PowerId::Poison), 4);
        assert_eq!(original.value(PowerId::Poison), 9);
    }

    #[test]
    fn the_boolean_tag_survives_a_write() {
        let mut slots = Slots::new();
        slots.set(PowerId::IsHatched, SlotWire::Bool, 1);
        assert_eq!(slots.get(PowerId::IsHatched).unwrap().wire, SlotWire::Bool);
    }

    #[test]
    fn building_from_unsorted_slots_sorts_and_rejects_duplicates() {
        let built =
            Slots::from_unsorted(vec![int(PowerId::Weak, 1), int(PowerId::Artifact, 2)]).unwrap();
        assert_eq!(built.as_slice()[0].key, PowerId::Artifact);
        assert!(Slots::from_unsorted(vec![int(PowerId::Weak, 1), int(PowerId::Weak, 2)]).is_none());
    }

    #[test]
    fn the_same_type_carries_relic_counters() {
        let mut counters = Slots::<RelicId>::new();
        counters.set(RelicId::RelicBurningBlood, SlotWire::Int, 2);
        assert_eq!(counters.value(RelicId::RelicBurningBlood), 2);
        assert_eq!(size_of::<Slots<RelicId>>(), size_of::<Slots<PowerId>>());
    }
}

//! Shared by the Coach policy examples: draw-blind resampling of a combat
//! state. Not an example target itself (Cargo only discovers top-level files).
use sts_sim::hot::{HotState, PileId, RngStream, RngStreamState};

/// SplitMix64: policy-side randomness, independent of every game RNG stream.
pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e3779b97f4a7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^ (z >> 31)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

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

/// Replace everything the player cannot observe with policy-random values.
pub fn determinize(s: &HotState, rng: &mut Rng) -> HotState {
    let mut d = s.clone();
    let draw = d.piles.get_mut(PileId::Draw).make_mut();
    // Start from an order the player could know (card identity, then uid) so
    // the true draw order cannot leak through the permutation.
    draw.sort_by_key(|c| (c.atom, c.uid));
    for i in (1..draw.len()).rev() {
        draw.swap(i, rng.below(i + 1));
    }
    for stream in STREAMS {
        let old = d.rng.get(stream);
        let words = [rng.next() | 1, rng.next(), rng.next(), rng.next()];
        d.rng.set(
            stream,
            RngStreamState {
                words,
                counter: old.counter,
            },
        );
    }
    d
}

/// Eval mode: sample `index`'s fresh shuffle and fresh game RNG from a
/// captured root, keeping the captured opening hand. The seed depends only on
/// the index, so every policy is measured on the same shuffles.
pub fn resample(s: &HotState, index: u64) -> HotState {
    determinize(s, &mut Rng(0x5245_5341_u64.wrapping_add(index)))
}

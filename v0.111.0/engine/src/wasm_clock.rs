//! A monotonic clock for `wasm32` builds (#3469).
//!
//! `std::time::Instant::now()` panics on `wasm32-unknown-unknown`, and the
//! exact DFS reads the clock at every solve for its deadline and telemetry
//! (`exact_dfs::solve_seeded`). This module stands in for `Instant` there,
//! reading milliseconds from one host import instead of pulling in
//! `wasm-bindgen` (which the `web-time` crate would):
//!
//! ```text
//! (import "env" "sts_now_ms" (func (result f64)))
//! ```
//!
//! A host supplies it as `{ env: { sts_now_ms: () => performance.now() } }`.
//! The value must be monotonic milliseconds; `elapsed` clamps a backwards
//! step to zero rather than panicking. Native builds never compile this
//! module, so their clock is `std::time::Instant`, unchanged.

use std::time::Duration;

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn sts_now_ms() -> f64;
}

/// The subset of `std::time::Instant` that `exact_dfs` uses.
#[derive(Clone, Copy, Debug)]
pub struct Instant(f64);

impl Instant {
    #[must_use]
    pub fn now() -> Self {
        // SAFETY: a host import that takes no arguments and returns a number.
        Self(unsafe { sts_now_ms() })
    }

    #[must_use]
    pub fn elapsed(&self) -> Duration {
        Duration::from_secs_f64((Self::now().0 - self.0).max(0.0) / 1000.0)
    }
}

//! Rust port of the v0.111.0 STS2 combat simulator.
//!
//! Scaffold stage (#1285, part of #1282). The crate currently holds only the
//! build-agnostic parity primitives seeded from the v0.110.1 kernel — the .NET
//! RNG, the decimal semantics, the introsort port, and the allocation counter.
//! The engine, generated content ids, and the differential-serving binary
//! protocol arrive in later port issues.
//!
//! Placement and identity follow `PORT_PLAN.md` §2: this crate is keyed to the
//! game build whose Python `combat_sim.py` is its differential oracle. Parity is
//! always within a build, never across builds.

#[cfg(all(test, feature = "allocation-counting"))]
#[global_allocator]
static TEST_GLOBAL_ALLOCATOR: allocation::CountingAllocator = allocation::CountingAllocator;

pub mod allocation;
pub mod boundary;
pub mod canonical;
pub mod catalog;
pub mod content_tables;
pub mod coverage;
pub mod decimal;
pub mod dotnet_sort;
pub mod encounters;
pub mod engine;
pub mod entry;
pub mod exact_dfs;
pub mod exact_solve_v1;
pub mod frame;
pub mod hooks;
pub mod hot;
pub mod ids;
pub mod mcr;
pub mod moves;
pub(crate) mod pet;
pub mod powers;
pub mod recorded;
pub mod rng;
pub mod run_counters;
pub mod search;
pub mod solo_v1;
pub mod steps;
#[cfg(target_arch = "wasm32")]
pub mod wasm_clock;

# Exact-solve review adapter v1

`sts-sim exact-solve` is the only Python-to-Rust exact-search crossing.  It
accepts one complete `sts-sim-exact-solve-v1` JSON request on stdin and emits
one response on stdout.  Python must never call legal-actions or apply-action
per DFS node.

## Deployment

Build the release binary in the checked-out v0.111.0 crate, then point the
review-worker environment at that exact executable:

```sh
cargo build --locked --release --manifest-path versions/v0.111.0/rust/Cargo.toml --bin sts-sim
export STS_SIM_EXACT_SOLVER="$PWD/versions/v0.111.0/rust/target/release/sts-sim"
```

The worker inherits `STS_SIM_EXACT_SOLVER`, and `review_summary_v2.py` selects
it as its default `--rust-exact-solver`.  Without the environment setting,
the review CLI discovers only the checked-in release target; it does not guess
a random `sts-sim` on `PATH`. Stored review metadata records `exact_solver` as
`rust` for the actual solve and every sampled world. A missing binary or typed
Rust refusal produces a fail-closed refusal document; it never selects Python
search.

## Seed and alpha discipline

A `seed` is a complete terminal winning action line plus objective and digest.
Rust replays it through its engine before it may be returned after a deadline
or cancellation.  `alpha_final_hp` is separate and line-less: it may support
strict-below certified pruning later, but it can never become an achieved
answer.  Equal-HP potion/turn ties must remain searchable.

`exact_solve_v1::retained_solo_review_corpus_enumerates_adapter_admission_frontier`
is the checked-in review census. It walks every retained synthetic/MCR solo
row at a zero deadline, proving canonical conversion and engine admission for
the two Toadpoles rows and the recorded Mawler capture without paying for a
full search. Mawler carries the capture's exact 53-epoch unlock profile rather
than promoting it to a fully unlocked pool. Production run-only roots outside
those retained rows are not implicitly admitted: missing physical provenance
or unported random-AI behavior remains visible as a stable refusal, never a
silent Rust claim.

## Run-only cutover census

The fast adapter integration enumerates all six fights in the retained
`6P96T755CNZ3.run` example at a zero deadline. Slimes Weak and Nibbits Weak
cross the Rust boundary. Shrinker Beetle remains an
`engine_admission_refused` random-AI frontier. Mawler, Inklets, and Cubex
remain `canonical_boundary_refused` because the run-only reconstruction has a
dense potion inventory but no sparse physical slot identities. The recorded
Mawler capture is independently admitted above because it carries that exact
provenance. These refusals are deliberate: compacting unknown potion slots
would guess action identity.

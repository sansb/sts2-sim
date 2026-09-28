# Solo-v1 exact-solve authority

`sts-sim-solo-v1` is the deliberately narrow authority contract for the Rust
exact solver. Rust owns exact search after this gate admits an entry.

Rust may own transition and exact search only after a canonical v0.111.0 state
passes this gate. The state must use `sts-sim-canonical-v2`, carry no prior
refusal, explicitly name game build `v0.111.0`, and be structurally solo.
Presence of any projected `multiplayer_allies`, `multiplayer_player_order`,
`teammate_present`, `teammate_power_card_pending`,
`teammate_damage_pending`, or `teammate_damage_owner_key` field produces a
typed refusal. Canonical zero-default elision makes presence meaningful; even
a forged default-shaped value refuses rather than being interpreted
optimistically.

The checked-in [`fixtures/exact_solve_corpus_v1.json`](fixtures/exact_solve_corpus_v1.json)
freezes the historical Python result for two canonical solo entries: the
ordinary self-check state and the checksum-backed Mawler capture. The Python
oracle generator was retired after the Rust solver matched the corpus. Fast
checks rebuild entries, replay the recorded action lines, and verify
entry/final digests plus the stable review subset without searching.

Each recorded `play` names the live physical-card `uid` used by Rust's action
protocol. A nearby `diagnostic_card` id/upgrade label is for review only; it
is never a replay lookup key. The adapter rejects a missing or non-unique UID
mapping rather than guessing from that label.

Python retains input parsing, provenance, and review rendering. The review
worker selects the versioned Rust boundary for exact solving and fails closed
when the binary, build, or admission evidence is unavailable. Unsupported
multiplayer entries remain typed refusals; the retired Python search did not
implement that surface either.

# Simulator versioning — design record

Status: **partly implemented.** Produced by the #1258 review thread
(Sean + Sol + Opus, 2026-08-16); implementation began 2026-08-17. This file is
the durable record; the issue holds the round-by-round argument. See
"Implementation status" at the end for what is shipped, what is blocked, and
one premise below that turned out to be wrong.

**2026-09-25: the Python simulator is deleted** (#2827 item F). Everything
below that describes `combat_sim.py`, `solve_fight.py` or `content/` as live
is the record of how the bundle worked before that date; see "The Python
simulator is deleted" under "The Rust trees" for what changed.

## The finding

`sts2_rng.py` is version-aware. `combat_sim.py` is not. Its ~199 "build"
references are comments recording which DLL a mechanic was IL-verified against;
the only runtime use is a card-pool census guard. There is one combat
implementation — the current one — and nothing refuses or caveats an **older**
run. So a v0.104 run is reviewed with v0.104 RNG and v0.111 combat semantics,
and wherever a card or encounter was remodeled in between, the review produces a
confident number for a fight that did not play that way.

I5 does not catch this. I5 refuses what is **unmodeled**; this is
**mis-modeled for that version**. Every mechanic involved is modeled, just not
as that build behaved, so nothing raises and the card renders as an ordinary
certified/modeled review. The only existing detector is recorded-capture
replay: a `.mcr` is ground truth for how the fight actually behaved, so a
mismatch surfaces as `recorded_line_replay_failed`. **Do not relax the
required-replay rule** — degrading to a capture-free review would delete the
only detector and leave the silent wrong answer in its place.

`#323` is the same bug on the card-template axis, with measured numbers
(GIANT_ROCK 16→20, TAUNT 7→6 across the v0.108→v0.109 boundary). It folds into
this decision as blocked-by rather than standing alone.

## The decision

Version the simulator. Refuse any build whose combat semantics have not been
verified. Destroy every review document produced before simulator identity
existed.

**Option 1 (per-build forks) for behaviour; version-keyed artifacts for data.**
The repo already runs the data pattern successfully three times —
`sts2_rng.seeding_scheme`, `mcr_parser.TABLES_BY_VERSION` with its frozen
`mcr_tables_v0.108.0.json`, and the per-build vanilla whitelist. Those work
because they are version-keyed *artifacts*: diffable, no duplicated logic. A
fork earns its keep only where dispatch order and hook semantics change.

### What a "version of the sim" means

Not a directory. A bundle is admitted only when four things exist for that
build:

1. the archived DLL, pinned **by sha256** — not by version string, because a
   depot can be rebuilt under an unchanged string;
2. a completed version-impact triage: every changed CIL body classified
   behaviour-bearing or not (`solver/version-impact/v0.111.0.json` is the
   existing shape);
3. a harness oracle corpus generated against that DLL, green;
4. its own data artifacts regenerated — templates, censuses, `game_values.json`,
   `mcr_tables`, vanilla whitelist — with their hashes pinned in the manifest.

Anything short of that refuses. The manifest is the sole admission authority.

### Forward-only

A bundle is only a version of the sim if it can be *verified* against that
build's DLL; otherwise it is a snapshot of what we believed that week, and git
already has those.

| build | DLL archived | forkable |
| --- | --- | --- |
| v0.111.0 | yes | yes — the current model |
| v0.110.1, v0.109.1, v0.109.0, v0.107.1 | yes | later, at explicit cost |
| v0.108.0 | **no — deleted by Steam (#309)** | never |
| ≤ v0.107.0 | **no** | never |

All 355 runs in the local corpus are ≤ v0.107.0. They are not "not yet
backfilled"; they are permanently unreviewable at exactness. **The admitted set
starts at `{v0.111.0}` and grows forward only.**

Corollary: **archiving the DLL on every version bump is a load-bearing product
invariant, not hygiene.** One missed archive permanently deletes a build's
reviewability. It needs automation and an alarm, not a README instruction.

### Ground-truth pins are on an unadmittable build

`fight_states.json` (ZPJHU3WSH2, 9 fights) and `fight_states_7MA0PY7AD4.json`
(6 fights) are both `build_id: v0.108.0` — the one build whose DLL is gone.
Dozens of `test_batch*.py` modules read them. Today those v0.108 pins silently
constrain the v0.111 model.

Resolution: recapture on v0.111.0 **first**, then remove the v0.108 pins from
current-build gates.

> **Correction (2026-08-18).** An earlier version of this paragraph said the
> recapture could be done "via the harness oracle, so the replacement corpus is
> mostly generated rather than hand-played". That is wrong, and #1272 is
> blocked on a human because of it. The harness answers *(state, action) →
> outcome*; it cannot manufacture
> `test_byrdonis_ingame_validation_7ma0py7ad4`, whose whole value is that a
> **human played the fight** and we know the real result — the suite's only
> optimality proof on real ground truth. Nor can the fight be ported:
> `seeding_scheme` returns `v108` for that build and `v109` for v0.111.0, so
> the same seed derives a different shuffle. The pin is intrinsically v0.108
> because its RNG derivation changed, not its cards (its content closure
> intersects none of the v0.109/v0.110.1/v0.111.0 remodel sets). And there is
> no v0.111.0 evidence anywhere to recapture from: every `.mcr` is
> v0.108/v0.109/v0.110.1, every testdata `.run` is v0.108/v0.110.1, and all 355
> local runs are ≤ v0.107.0. Preserve the raw `.run`, `.mcr`, and
pin artifacts under a labelled `legacy_evidence/v0.108.0/` area — they cannot
certify an admitted v0.108 simulator, but they remain irreplaceable
observations that can falsify a future historical claim. Recapture-then-retire,
never the reverse.

### Isolation boundary

**Subprocess per build.** Everything is flat sys.path modules — 347 files
`import combat_sim` — and `sys.modules['combat_sim']` is a singleton, so two
bundles cannot coexist in one process. The review CLI already shells out per
fight, so the dispatcher reads raw build identity and selects the bundle path
*before* any semantic import. Package-ifying the flat imports is a large
migration that buys nothing over process isolation; defer it.

### What is in the bundle: fidelity vs. correctness

Two separate revisions, both in document identity:

- **`sim_revision` + bundle digest** — build-specific game and world-model
  fidelity: the engine, `content/**`, templates and censuses, the RNG scheme,
  decoder tables, and the behaviour-dependent parts of entry reconstruction.
  Frozen per build.
- **`pipeline_revision`** — shared search, estimator, parser envelope, and
  producer correctness. Not frozen per build, but *inside* the exactness claim.

"Wrong versus weaker" controls **invalidation, not file placement**. An unsound
prune, incomplete legal-action expansion, bad memo key, or incorrect terminal
comparison publishes a false optimum — wrong, not weaker, and it lives in shared
code. Every pipeline change therefore carries an explicit compatibility
classification: `equivalent`, `quality_only`, or `correctness_invalidating`.
A correctness-invalidating change revokes affected pipeline revisions **across
every build**. That classification must be declared and reviewed per change —
left implicit, `quality_only` becomes the default because it is the cheap one.

**PIMC splits along the same seam.** `infoset_sample.py` contains claims about
the game, not only about how hard we looked: the conditioning boundary ("the
entire order of the remaining Draw pile is hidden … revisit if such a mechanic
lands"), the `_FUTURE_STREAMS` enumeration, and derivation through `RunRngSet`.
Those are per-build world model → bundle. Sample count, seed schedule,
aggregation, and convergence policy are estimator → shared, recorded under
`pipeline_revision`. K and its uncertainty must stay explicit in the document,
or "we looked harder" is indistinguishable from "the answer changed".

### Lifecycle: `pending`, `admitted`, `revoked`

**No retirement** (Sean, 2026-08-16). Support every deliberately built and
admitted bundle. The set is bounded by admission effort spent, not by policy,
so it does not have the accidental-unboundedness problem a
document-population rule would.

- `pending` — a registry state, not a document status or a queue promise. A new
  release starts here. Copying the predecessor must never itself enable a build.
- `admitted` — gated per the four criteria above.
- `revoked` — per revision, terminal, for auditability. A corrected successor is
  a new revision.

Fidelity defects found later: classify impact **before** a bundle stays
admitted. If a defect may affect five bundles, each is proved unaffected or
moved to `revoked`; uncertainty cannot remain `admitted`. Repair is then
demand-driven — documents are regenerable, so laziness is free here and eager
N-way backports are not.

**Frozen bundles rot against their runtime.** A bundle must still run on
whatever interpreter and dependencies exist years later, so interpreter and
dependency identity belong in the execution attestation, and one deterministic
fixture per bundle should run on a slow cadence with a full output-fingerprint
comparison. That is the only ongoing CI cost immutability does not eliminate.

## Routing and refusal gates

`build_id` is the **sole routing key**. The capture's `gitCommit` and
`modelIdHash` are not a second key — they assert the route was right, and cost
nothing to check. Verify, do not route.

Gate order, on raw input before any semantic import:

1. refuse unsupported `game_mode`, non-empty `modifiers`, multiplayer, or
   non-vanilla provenance;
2. route exact `build_id` through the manifest — no ranges, no `latest`
   fallback, no default build;
3. where a capture exists, assert its version/commit/model hash agree with the
   routed artifact;
4. refuse any disagreement.

Two current shapes violate step 2 and must become enumerations:
`seeding_scheme` admits every `(major, minor) <= (0, 108)` and every `v0.109.x`
by range, and `sts2_rng`'s `build=` argument defaults to `GAME_BUILD_V0_108`
instead of being required. The range means all 355 corpus runs — down to
v0.99.1 — receive a scheme by extrapolation from a v0.108.0 verification, which
settles the "is the ≤v0.108 bucket verified?" question as **assumed, not
verified**.

The raw `.run` carries `game_mode`, `modifiers`, `platform_type`, and
`was_abandoned`; all four are dropped during `RunSummary` construction, so the
gate must read raw. The corpus has a live target: 1 of 355 runs is
`game_mode: daily` carrying `MODIFIER.HOARDER`, `MODIFIER.DRAFT`,
`MODIFIER.TERMINAL`. The review path presently consults **no** mod state at all
— no reference to `modded`, `affects_gameplay`, or `mods` in
`review_summary{,_v2}.py`, `review_provenance.py`, `tools/review_worker.py`, or
`relay_parser.py`.

### Vanilla is weaker than it looks

`src/vanilla.mjs` documents the limit: "balance-only mods that reuse vanilla ids
are undetectable" (#908). It is also deliberately **fail-open** — "a run for an
UNKNOWN build … is always accepted (EA patch lag must never block uploads)".
That is right for uploads and exactly inverted for review, so the review gate
must derive its own verdict rather than inherit the upload verdict.

By channel: a manual `.run` upload gets the ID whitelist only, so the honest
label is `provenance_unknown`, not vanilla. A mod-client upload declares
`mods[].affects_gameplay`, which covers ID-preserving balance mods subject to
client honesty — and the mod version string has already lied once
(0.5.0/0.5.1 reporting "0.4.3"). **Absence of evidence of mods is not evidence
of vanilla.**

## Straddling runs

Confirmed behaviour (Sean, from observation): close the game mid-run, update,
resume — the run continues under the new build with new card semantics, and the
game posts the run as X+1. So **`build_id` is the final build**, which also
explains the 10 start-order inversions in the local corpus that no start-stamp
reading could account for.

Label follows the game: post and analyse as X+1. But labelling and analysis
split — analysing every floor as X+1 is knowingly wrong for pre-switch floors.
Four detectors, in increasing power:

1. **Identify the population.** `start_time < release(build_id)` proves the run
   began before its own tagged build existed. One comparison; needs only a
   release-date table, which costs nothing to maintain at archive time.
2. **Localise the switch from the `.run` alone.** Every deck card and relic
   carries `floor_added_to_deck` — present on all 8,618 cards and 3,437 relics
   across the 355-run corpus, floors 1–48. Cross it with the per-build content
   id sets the vanilla whitelist already computes: an id removed or renamed in
   X+1 acquired at floor N is a lower bound on the switch; an X+1-only id at
   floor M is an upper bound. Renames are sharpest because the old name is
   impossible under the new build (#976 records four at the v0.107 boundary).
3. **Dual-bundle differential.** Run a flagged run under both X and X+1 and
   compare per fight. Where they agree, attribution does not matter; where they
   disagree is exactly the at-risk set. **Detection power is proportional to the
   harm** — where the builds do not differ there is nothing to get wrong. Needs
   bundle X to exist, so the first straddlers can only be flagged.
4. **Captures bracket, they need not cover.** Each `.mcr` carries `version`,
   `gitCommit`, `modelIdHash` for its own fight, and a run's build only
   increases, so the last capture on X and the first on X+1 bound the switch.

The goal beyond inference: **have the mod stamp build identity per fight or
floor directly.** A few bytes converts this into a recorded fact and covers
floors with no capture; captures then serve as the cross-check. Inference alone
cannot close capture coverage, nor the association step — `review_provenance.py`
refuses to bind a capture to a fight unless the combat-start snapshot uniquely
identifies one parsed fight by both global history depth and encounter identity.

### No end timestamp

Both schema versions in the corpus (8: 30 runs, 9: 325) carry only `start_time`
and `run_time` (playtime). There is **no end timestamp**, so a run's wall-clock
window cannot be bounded from the file — the otherwise obvious "refuse any run
spanning a release boundary" fallback is not computable.

One construction survives that: a *completed* run whose `build_id` is the
current head build cannot have straddled, because no newer build existed to
straddle into. Live and recent reviews are therefore sound by construction;
historical backfill is what needs detectors 1–4.

## The Rust trees

**Layout (corrected 2026-08-19).** There are now **two** Rust trees, and an
earlier version of this section described only the first while the second was
already live — the exact staleness this record exists to prevent, so the
correction is recorded rather than quietly applied.

- `versions/v0.110.1/rust/` — the **frozen kernel**, crate `sts-kernel`. Kept
  deliberately (PORT_PLAN §9.2) for search experiments and posterity; its
  Queen/Colony certification fixtures stay valuable as differential corpus
  entries. Immutable except for its own maintenance, and nothing in the port
  depends on it at runtime.
- `versions/v0.111.0/rust/` — the **active full port** (#1282), lib `sts_sim`,
  bin `sts-sim`. Keyed by the build whose Python sim is its differential
  oracle.

So `versions/` now does hold both trees for v0.111.0, which the earlier text
predicted would happen "when the kernel forks forward". What actually happened
is not a fork: the kernel stayed frozen at v0.110.1 and a new port crate was
started at v0.111.0.

The kernel (#1138) implemented much of this design independently, and its
vocabulary was adopted rather than duplicated. From
`versions/v0.110.1/rust/fixtures/queen_capability_inventory_v1.json`:

- `graduation_unit: "complete_reachable_archived_v0.110.1_queen_bundle"` — the
  graduation unit is explicitly build-scoped;
- `source` carries `game_build: v0.110.1`, `game_commit: db5d3552`,
  `model_id_hash: 0xea293d3f`, plus sha256 of the entry save and winning
  capture — the exact "verify, don't route" triple;
- `phase_status.current_build_v0.111.0: "explicitly_out_of_scope_and_refused"`;
- `mid_fight_fallback: forbidden`, admission all-or-nothing per solve world.

### What the three earlier "consequences to reconcile" became

1. **"It can never serve a production review" — resolved for the port, still
   true of the kernel.** The kernel is build-pinned to v0.110.1 and explicitly
   refuses v0.111.0, so its certified bundle targets a build outside the
   admitted set. That has stopped mattering for production because the port
   targets **v0.111.0, which is the admitted build**. The kernel's role is now
   explicitly historical, so neither of the old escape routes — admitting
   v0.110.1, or re-certifying the Queen bundle forward — is required.
2. **"Parity within a build, never across" — adopted, not merely proposed.**
   PORT_PLAN §2 states it directly: the crate is keyed by the build whose
   Python sim is its differential oracle, and `canonical.rs` cites this record
   for the same rule. Port correctness is "Rust matches Python-at-v0.111.0";
   sim correctness (Python vs the game) is a separate workstream that never
   blocks a port PR.
3. **The version treadmill — answered as a working assumption, not a policy.**
   PORT_PLAN §9.1 (Sean, 2026-08-17): finish on v0.111.0, since the
   differential oracle is per-build, and fork forward afterwards per this
   record. Deliberately not re-litigated mid-wave unless a bump forces it.

### Python does not retire when the port lands — SUPERSEDED 2026-09-16

**Authority flipped to Rust on 2026-09-16 (Sean, on #1282): `combat_sim.py` is
frozen at v0.111.0 and the oracle is the game's own capture checksums.** The
paragraph this section used to open with is kept verbatim below, because it
states the coupling that had to be broken before anything could move and
because every piece of v0.111.0 evidence in this repo was produced under it.

*The historical record, written while the port was in flight:*

> Worth stating plainly, because "we are moving to Rust" invites the opposite
> assumption. The port's oracle **is** Python at v0.111.0, so retiring Python
> would delete the thing that certifies the port. #1282 resolves this as:
> capture-based ground truth has to become the oracle before Python can
> retire, which is still open. Until then Python is both the modeling surface
> of record and the production search path, and every game-version bump costs
> two engines rather than one.

**What changed: the prerequisite that paragraph named now exists.** The game
writes per-action state checksums into every `.mcr` — nine RNG streams plus
full creature/pile/power state — #2470 landed the fixtures that carry them,
and `versions/v0.111.0/rust/tools/eval_suite.py census` measures Rust against
them *directly* rather than through Python. Capture-based ground truth stopped
being the open question and became an artifact, so the decision it gated was
taken before the next game build rather than after it.

#### The decision (Sean, 2026-09-16, #1282)

| | in force from 2026-09-16 |
|---|---|
| **D1 — Oracle** | Rust correctness is certified against the game's `.mcr` per-action checksums (the eval fixture tree plus the census lockstep, on the **release** binary) and the headless harness for targeted experiments. Python parity is advisory evidence on v0.111.0 only; the differential gates nothing on any later build |
| **D2 — Entry** | Rust builds its own root from the save/upload payload. Python's entry refusals stop gating Rust; Rust carries its own typed I5 refusals at that boundary, and the census measures admission from every capture rather than from the Python-admitted subset. #2069 and the Glam-on-Stone-Armor entry refusals are closed as superseded, not worked |
| **D3 — Modeling surface** | New mechanics are read from IL into **Rust** first, cited in the ported function's adjacent `///` comment, exactness-or-refusal unchanged. `versions/v0.111.0/solver/combat_sim.py` is **frozen**: no remodel for a new build, no new mechanics, bug fixes only where they change a v0.111.0 parity verdict |
| **D4 — Fork-forward** | Only the Rust crate and the parsing/projection layer fork to the next build; the Python engine does not fork |

What this does **not** change: I5 exactness-or-refusal, forward-only DLL
archival, the admission criteria, or the rule that a build without an archived
DLL can never be admitted. The lever is which engine is authoritative, not the
bar.

**Still open (item 3 of the proposal):** the post-release review blackout —
whether the accepted "unreviewable until the remodel lands" window stays as it
was, or whether reviews on a new build refuse until the checksum census
certifies each encounter. Default until Sean rules: unchanged, which is what
D1 produces anyway.

### The Python simulator is deleted — 2026-09-25 (#2827 item F)

Sean's 2026-09-24 decision on #2827: a **hard delete**, not an archive. Once
every production root, search and replay path ran in Rust (#2973, #2988,
#2995, #3009) and the `.mcr` certification census and the `rust port` lane
ran without the simulator (#3000, #3014, #3015, #3016, #3018), item F removed:

- `versions/v0.111.0/solver/combat_sim.py`, `solve_fight.py` and
  `content/**`, with the modules that existed only to feed them
  (`mcr_replay.py`, `python_fight_context.py`, `fixture_power_ledgers.py`,
  `coverage_report.py`, `accelerant_report.py`, `tools/mcr_validate.py`,
  `tools/build_coverage_matrix.py`, `tools/build_fight_states.py`,
  `tools/prefreeze_static_fanout.py`, the crate's `tools/project_state.py`,
  and the v0.110.1 kernel's Queen/Colony Python differentials and fixture
  builders);
- the test modules whose subject was the simulator. Tests whose subject
  survives (the run parser, the save adapter, `.mcr` decoding, `sts2_rng`,
  `dotnet_sort`, the IL censuses and templates) were kept with only their
  simulator dependency removed;
- the in-game `fight_states*.json` pins, which validated the Python engine on
  the unadmittable v0.108 build (see "Ground-truth pins are on an unadmittable
  build" above);
- `solver-slow-pins.yml` and the `post_submit` pytest marker: the lane's
  certifications were Python/Rust differentials, and its two process checks
  joined the fast gate;
- `content_refusals.json` and the Python coverage badge (`coverage.svg`,
  `COVERAGE.md`). `coverage_matrix.json` stays as frozen data, because the
  harness probes still read its statuses.

What replaces it: the Rust crate is the only engine (D3 above is now the
only surface, not a freeze), the `.mcr` certification census is the oracle
(D1), and every oracle-derived pin the crate still reads is frozen data
sha256-pinned by `rust/tools/frozen_oracle_data.py --check`, with no Python
regeneration path. `generate_content.py` replays only the frozen registry
snapshot (`--registry python` and `registry_snapshot.py --write/
--verify-replay` went with the simulator).

Two bundle consequences. `sim_identity.bundle_digest` moved (every stored
review's `metadata.simulator` stops matching, the designed invalidation), and
`SOLVER_INVARIANTS.md` and `ENCOUNTER_MECHANICS.md` became frozen records:
the IL they cite is still the evidence for the Rust ports that name them, but
new mechanics are cited beside the Rust function.

### The fork-forward question this record now owns — ANSWERED 2026-09-16

**The split line in "What is in the bundle: fidelity vs. correctness" binds
both trees, not just the Python one.** Forking the Rust crate forward at the
next release copies whatever the crate contains — and R2 intends to put search
in it (one `UctDomain` over the generic engine). That is precisely the
over-forking problem #1275 exists to fix on the Python side, arriving a second
time on the Rust side, and #1275 is scoped to Python only (it does not mention
Rust).

Two things followed, and D4 answers both:

- **the hoisted shared layer is language-neutral by placement** — one copy,
  consumed by whichever engine holds authority, rather than duplicated per
  language. With Python frozen there is exactly one consumer to be neutral
  towards, which is what makes the placement cheap to adopt now;
- **Rust certification targets head-and-follow**, not a pinned lagging build.
  It can, because the checksum oracle exists for every build the mod captures:
  certification no longer waits for a per-build Python oracle to be remodeled
  first.

A third question the answer forces, decided with it as **P2**: **per-build
*data* artifacts live beside the crate that consumes them** —
`versions/<new>/rust/data/` — and never in the frozen Python tree.
`mcr_tables.json`, the five ModelDb censuses and the vanilla-whitelist source
are per-build data, they are attested under `versions/v0.111.0/solver/`, and
D3 freezes that directory; writing a new build's copies into it would break
the freeze and redden the fast gate, while writing them nowhere leaves the new
build's captures undecodable. Their hashes are pinned in the consuming crate's
own attestation.

The deadline #1275 was worried about was met: this was decided **before** the
copy was made, not after.

## Sequence

0. Reconcile the manifest/code split-brain: `version-impact/v0.111.0.json` says
   `combat_admission: "globally refused for v0.111.0"` while
   `_V1110_PENDING_REMODEL_ISSUES = ()` makes the guard at `combat_sim.py:35533`
   dead code. Nothing unsafe is admitted, but the real authority is an empty
   tuple and the artifact that reads like the authority drifted from it inside
   one batch. Start from a state known to be self-consistent.
1. I11 plus an outer exact-build refusal; admitted set `{v0.111.0}`. Convert
   `seeding_scheme`'s ranges to an enumeration and drop its default `build=`.
2. Document/cache identity: `game_build`, game artifact identity, parser
   revision, `sim_revision` + bundle digest, `pipeline_revision`. Rename
   `metadata.solver_build`, which today holds `run.build_id` — the game build
   under a name that reads like the solver's.
3. Purge all pre-versioning review documents, plus the deferred-pending state so
   a refusal queues rather than terminates.
4. Automate and gate DLL archival.
5. Per-fight build attribution: the save-writer IL read, plus capture
   provenance.
6. Generate the v0.111 harness/pin corpus; move v0.108 evidence to
   `legacy_evidence/`.
7. Fork the fidelity closure behind subprocess dispatch; pin data hashes in the
   manifest.
8. Lifecycle, revocation, and per-fork CI path scoping. Frozen bundles must be
   immutable and path-filtered so the fast gate does not go N×; `TRIGGER_SET`,
   the external-surface guard, and the attestation manifest all have to learn
   about bundles.

Step 7 is `area:engine-core` and serializes against everything else touching
`combat_sim.py`; steps 1–5 are mostly outside the engine and can land in
parallel ahead of it.

## Open

- **Blocking, technical:** the save-writer IL read — can an in-progress run
  cross builds, and what stamps `build_id`? Decides whether release-spanning
  save-only reviews can ever be certified.
- **Blocking, product:** vanilla-provenance wording, given the channel
  asymmetry above.
- **Strategic:** captures self-attribute and only the mod client can report an
  ID-preserving balance mod. Both point at exact review being soundest for
  mod-client captured runs, and manual `.run` uploads possibly being
  structurally incapable of supporting a certified review — not an
  implementation gap, the input does not carry the facts. Consistent with the
  tracker-free / coach-paid split, but it changes what the review surface can
  promise most uploaders.
- **Accepted, not solved:** the post-release blackout. Every player who takes an
  update is unreviewable until the remodel lands — for v0.111.0, built
  2026-08-13T17:39, last remodel child (#1217) closed 2026-08-15T20:25, about
  2.5 days. Accepted by Sean; the lever is narrowing what reaches a human in
  version-impact triage, since the scanner already produces the classified
  changed-body list mechanically.

## Implementation status (2026-08-18)

**Shipped**

- The simulator is relocated to `versions/v0.111.0/solver/` and the Rust kernel
  to `versions/v0.110.1/rust/`, keyed by the build each is certified against
  (#1273, #1277, #1281). `solver/` now holds only cross-build material.
- **Exact-build admission (I11, #1263/#1264, PR #1295).**
  `versions/admitted_builds.json` is the sole authority; `admission.py` refuses
  an unadmitted build *and* a build this bundle does not model;
  `review_summary.load_fight_context` turns that into a structured
  `unadmitted_game_build` refusal card. A real v0.99.1 run now refuses instead
  of returning a confident number — the #1258 defect is fixed at the product
  boundary.
- **`seeding_scheme` by enumeration (#1265 partial, PR #1299).** The
  `<= (0, 108)` range extrapolated a derivation verified only at v0.108.0
  across nine minor versions and 100% of the local corpus. Both admission
  surfaces now refuse by default.
- **DLL archival is alarmed (#1269, PR #1298).** `archive_build.py --check`
  runs daily on the self-hosted runner and fails if the installed build is
  unarchived, or if an archived version string's installed DLL no longer
  matches. `index.json` records sha256, commit, and **release date** per
  build — the last of which #1271's straddling detector needs and previously
  had no source for.
- Slow pins moved off the PR path to post-submit (#1280), on the velocity
  argument rather than anything in this document.
- **Document identity (I12, #1267).** Every review document — card,
  benchmark-only card and refusal — carries `metadata.simulator`:
  `game_build` plus the archived artifact (`game_commit`, `dll_sha256`, read
  from the admission registry rather than restated), `sim_revision` +
  `bundle_digest`, `parser_revision`, `pipeline_revision`.
  `metadata.solver_build` is renamed `metadata.game_build`; it always held the
  run's game build. Contract: `REVIEW_PIPELINE_SCHEMA_V7.md`.

  Three things worth carrying forward:

  - **The digest is computed, the revisions are declared, and only the digest
    is load-bearing.** `bundle_digest` is a sha256 over the bundle's semantic
    surface, recomputed on every run, so it cannot go stale; the declared
    revisions are legible era markers whose forgetting costs legibility, not
    correctness. The surface is fail-closed the useful way — an unclassified
    file counts as semantic, so an oversight over-invalidates rather than
    leaving a stale document looking current.
  - **The derived guard immediately found something declaration had missed.**
    `test_sim_identity.py` derives the review CLI's real import closure and
    failed on its first run naming `tools/live_coach.py`, which `mcr_replay`
    imports for entry reconstruction. The surface was corrected, not the
    guard. This is the same shape as #1276's `TRIGGER_SET` derivation and is
    the pattern to reach for whenever a list has to stay true.
  - **The compatibility class is declared in
    `solver/pipeline_revisions.json`** and an entry without one refuses to
    load. Revision 1 is `correctness_invalidating`, which revokes every
    pre-identity document as a population — the input #1268 needs.

  Invalidation is automatic by hash (the worker folds the identity into
  `generator_config_hash`, so any change regenerates without a backfill) and
  queryable by hand (the identity is in the document body, which is how the
  #1268 purge will select). The worker also now refuses a CLI whose schema
  version is not its own and fails any document whose stamped identity differs
  from the one it keyed freshness on — the 2026-08-10 stale-worker failure
  made loud.

  Two things it deliberately does not do: the database column
  `fight_review_documents.solver_build` keeps its name (renaming it is a
  migration-then-merge-then-restart deploy, best carried with #1268's own
  migration, whose measurement query already reads that column), and nothing
  is purged.

  **#1275 inherits a requirement from this.** Search and the review producer
  still live inside the bundle, so `bundle_digest` covers them today and the
  declared `pipeline_revision` is belt-and-braces. The hoist must add a
  `pipeline_digest` over the shared layer, or the automatic half of I12
  narrows to fidelity alone at exactly the moment a second bundle exists.

**#1272: partly unblocked (2026-08-18).** Sean recorded a clean v0.111.0 run
(seed 6P96T755CNZ3, Ironclad A10) and it yields a working in-game validation
pin — `test_v0111_ingame_validation.py`, 303,699 states in ~140s against the
v0.108 byrdonis pin's ~728s. The recorded human outcome (5 turns, 21 damage) is
reachable in the modeled state space, 16 damage was the optimum, and a 3-turn
win existed.

Two things that run taught us, both worth keeping:

- **A single rest-site upgrade or purge poisons the whole run.** An earlier
  v0.111.0 run with four SMITH rests and four purges produced **0 of 18**
  reviewable fights — the upgrade cannot be assigned to a card copy, so every
  fight's entry deck is ambiguous. The clean run produces **6 of 9**.
- **Counter-consuming relics poison everything after them.** The three
  unreviewable fights all refuse on `RELIC.KUSARIGAMA` (#116).

Retiring the v0.108 pins is now possible but is a separate change: 244 files
reference them.

**Still blocked on #1272's remainder**

A single captured fight on v0.111.0 unblocks three things at once:

1. the **engine-level** admission check — `start_combat` has none, because
   ~460 call sites are built on the v0.108 `fight_states` pins;
2. **`start_combat(build=)`'s default of `GAME_BUILD_V0_108`** — the two
   build-dependent constants branch on an exact `v0.111.0` match, and 458 of
   460 call sites take the default, so this bundle serves the *historical*
   Regalite/Inky values and the v108 RNG scheme almost everywhere. Its own
   remodels are reached by two callers;
3. the **review fixtures** — no run in the review corpus is on an admitted
   build, so `test_review_summary.py` and `test_review_recorded_line.py` had to
   scope admission out.

- **The purge, and deferred-pending (I13, #1268).** The purge ran as DML on
  2026-08-19. The measurement is worth keeping, because it inverts the
  intuition this whole line of work started from:

  **8,519 documents; 1,143 cards; 1,006 of those cards (88%) on a build that
  is not `v0.111.0`.** Every card was v0.107.0 or newer — not one of the
  ~3,400 documents on v0.98.x–v0.106.x was a card. Old runs were never the
  danger: they hit an unmodeled mechanic, I5 fired, they refused. **The damage
  concentrated in the builds closest to current**, where every mechanic is
  modeled and nothing raises, so the review rendered an ordinary certified
  card whose numbers were off by whatever had been remodeled since. The bug
  was worst exactly where it was least visible, and it got worse as the model
  got better.

  The purge was also nearly free, because #1267's schema bump had already
  invalidated all 8,519 — deleting only decided whether a false card stayed
  visible while it waited hours in the regeneration queue.

  Deferred-pending then split refusal in two, on **whether the evidence
  survives, not how old the build is**. v0.108.0 is newer than v0.107.1 and
  permanently unreviewable, because Steam deleted its DLL while v0.107.1's was
  archived; a rule phrased as a version comparison would have deferred 291
  v0.108.0 cards forever and terminally refused 143 recoverable v0.107.1 ones.
  Four builds are now `pending` in the registry, and a run on one yields a
  `deferred` document rather than a refusal.

  What makes that work without a backfill: `admitted_builds_digest` joins
  producer identity, so **admitting a build restales every document by
  construction**. `pending` remains a registry state and explicitly not a
  queue promise, which the frontend copy is written to respect — neither "not
  reviewable" nor "coming soon".

**Not started:** the precondition gate (#1266), straddling detection (#1271),
the subprocess dispatcher and shared-layer hoist (#1275), lifecycle (#1274).

**#1275 is the one with a deadline.** The relocation deliberately carried
search and the review pipeline into the bundle, because over-forking is free
while there is exactly one. That stops being true the moment a second bundle
exists — the next release copies search too, and the hoist gets more expensive
with each copy.

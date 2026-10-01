# Rust port of combat_sim.py — architecture and process plan

Status: **decision record for #1282.** Charter: #1282 (the why and the scope).
Plan of record it refines: #1138. Coupled: #1283 (eval suite), #1276 (fast-gate
trigger set). Structural evidence below was surveyed 2026-08-17 against
`sim/v0.111.0/python/combat_sim.py` (61,464 lines) and
`versions/v0.110.1/rust/` (23,780 lines).

> **2026-09-25: the port's oracle is deleted.** #2827 item F hard-deleted
> `combat_sim.py`, `solve_fight.py` and `content/**` once every production
> root, search and replay ran in Rust. This file stays the decision record;
> where it describes the Python oracle, the differential, `start_combat` or
> the slow pins as live, it records how the port was run. The crate is the
> only engine, certified by the `.mcr` census (`tools/eval_suite.py census`),
> and the oracle outputs it still reads are frozen data pinned by
> `tools/frozen_oracle_data.py --check` (`sim/meta/SIM_VERSIONING.md` has the
> dated record).

The four goals this plan is built around, in priority order:

1. **Parallel waves** — 20–40 agents can port simultaneously without merge
   conflicts or serialized gates.
2. **No slow pre-submits** — a port PR's required checks run in minutes on
   parallel GitHub-hosted runners; anything slow is post-submit.
3. **No performance regressions** — by construction (layout rules enforced
   mechanically) and by measurement (deterministic pins at PR time, throughput
   floors post-submit).
4. **Mechanical porting** — every architectural decision is made here, once;
   a wave PR is translation against a fixed template, gated by the
   differential, with no design judgement required.

## 1. What is actually being ported

The survey finding that shapes everything: **combat_sim.py has no per-card,
per-relic, or per-monster functions.** Content is data; behavior lives in a
small number of giant dispatch chains. The porting unit is a *dispatcher
branch* (a "kind"), not a card.

| Domain | Data (already structured) | Behavior | Units | Typical size |
|---|---|---|---|---|
| Cards | `content/cards/*.py` (722 `add()` rows) + `card_templates.json` (304) | `_run_steps_inner` L59957–65282 | **314 step-kind branches** | median 10 lines |
| Monsters | `content/encounters/*.py` (89 builders, 66 loops) + 23 `*_MOVES` dicts | `monster_act` L44300–45707 | **91 move-kind branches** | median 10 lines |
| Potions | `content/potions.py` (id set) | `_apply_action_impl` L53324–54615 | **61 branches** | median 7 lines |
| Relics | 299 ids (117 census-inert + 18 effective template-only + 164 hand-authored active; 34 raw templates with 16 hand overlap) | 12-hook taxonomy enforced by `_relic_templates` L37989, template interpreter, ~155 `start_combat` blocks | ~30 hook fire points + per-relic blocks | small |
| Powers | 517 `State` fields + 106 `Monster` fields | 8 `_powers_*` fan-out fns | ordered fan-outs | 200–350 lines each |

85% of the 1,178 top-level symbols are ≤50 lines. Only 27 exceed 200 lines,
and those are almost all the shared engine core (damage pipeline, turn
structure, continuation machinery, `legal_actions`/`apply_action`) — exactly
the part that is **serialized foundation work, not wave work**.

What the existing kernel contributes, and what it does not:

- **Reused as-is:** `rng.rs`, `decimal.rs`, `dotnet_sort.rs` (parity
  primitives with existing pin corpora), `allocation.rs` + the
  per-simulation allocation classification, the `UctDomain` trait in
  `uct.rs`, the canonical-vs-hot split *as a concept*, Arc-based
  copy-on-write piles, the subprocess-JSON differential protocol with
  reproducible action prefixes.
- **Explicitly not extended:** the per-encounter fork pattern.
  Byrdonis/Queen/Colony are three parallel type families (~12,000 lines for
  two encounters and ~54 cards) with card effects dispatched by 54-arm
  string matches and relics as hardcoded struct fields checked inline.
  #1282's measured creep instances both live in that code. The port builds
  one generic engine; the v0.110.1 kernel stays frozen as the certified
  artifact it is, and search experiments (#1278/#1279) continue against it
  until the port overtakes it.

## 2. Placement and identity

- **Crate:** `sim/v0.111.0/engine/` — lib `sts_sim`, bin `sts-sim`.
  Keyed by the build whose Python sim is its differential oracle, per
  `sim/meta/SIM_VERSIONING.md` (parity is within a build, never across).
  The v0.110.1 crate (`sts-kernel`) is immutable except for its own
  maintenance; nothing in the port depends on it at runtime.
- **Oracle:** Python `combat_sim.py` at v0.111.0. Port correctness = "Rust
  matches Python", certified by the trajectory differential. Sim
  correctness (Python vs the game) is a separate workstream (#1282 §
  separation table) and never blocks a port PR.
- **Dependencies:** minimal, like the kernel — `serde`, `serde_json`,
  `sha2`, `rust_decimal`, plus (new, allowed) `smallvec` and `bitflags` if
  R0 wants them. No frameworks, no proc-macro-heavy deps, no rayon
  (thread fan-out stays hand-rolled). `Cargo.lock` committed, CI `--locked`.
  Set an explicit `[profile.release]` (`lto = "thin"`, `codegen-units = 1`,
  `panic = "abort"`) since benchmarks ship from release builds. The shipped
  surfaces return typed refusals and do not catch panics, so unwind support is
  dead release-image weight; keep dev/test on defaults for compile speed and
  the invariant tests that deliberately catch panics.

## 3. Architecture decisions

### D1. One generic engine; content is data + branch bodies

A single state/transition engine covering every encounter and character.
No per-encounter state types, catalogs, continuations, or manifests. The
generic seams that exist today (`UctDomain`) survive; everything below them
is replaced, not extended.

### D2. Ids are interned everywhere; strings exist only at the boundary

A generated `ids.rs` defines dense `u16` enums for every content axis:
`CardId`, `PowerId`, `RelicId`, `PotionId`, `MonsterKind`, `EncounterId`,
`StepKind`, `MoveKind`, `EnchantmentId` — generated from the Python content
registry (§5), with string↔enum tables used **only** during canonical
(de)serialization and evidence output.

Rules, mechanically enforced (§7):

- no `String`/`&str` comparison below the canonical boundary — dispatch and
  predicates use enums (`match` compiles to jump tables, O(1) in content
  size);
- event payloads carry interned ids; names are resolved on serialization;
- sort keys are `(u16, u8)` tuples, never cloned strings (the v0.110.1
  Colony reshuffle clones a `String` per element per sort — the exact
  anti-pattern).

### D3. State: canonical for the wire, hot for search, sized by the fight

Two representations, as in the kernel, but generic:

- **Canonical** (`canonical.rs` v2): serde struct mirroring the Python
  differential projection. String-keyed, ordered, deliberately slow. This
  is the wire format the differential compares and the entry format the
  admission gate consumes.
- **Hot**: the search-owned form, built from canonical at the boundary.
  Layout rules (the #1282 list, made concrete):
  - core scalars inline (hp/block/energy/turn/phase/flags);
  - piles are `Arc<Vec<HotCard>>` copy-on-write (proven design);
    `HotCard` stays ≤8 bytes `{uid: u32, atom: u16, flags: u16}`; mutable
    per-instance card state (afflictions, cost modifiers, enchantment
    state) lives in a copy-on-write side table keyed by uid — most cards
    in most states are unmodified and must cost nothing;
  - **powers are a sorted small-vector of `(PowerId, i32)` per entity**,
    not 517 struct fields: O(active powers this fight), cache-friendly
    linear scans, plus a per-category presence bitset so whole hook
    families are skipped in one test. Python's 517 `State` fields are the
    *canonical/wire* shape, not the hot shape;
  - monsters are `Arc<Vec<HotMonster>>` copy-on-write with dynamic count;
  - the continuation stack is one shared `Frame` enum (one variant per
    Python frame dataclass, ~45) in a COW vec, replacing the
    twice-implemented per-encounter continuation enums;
  - **hot-state inline size is pinned by `const` assertions**
    (`size_of::<HotState>()`, `size_of::<HotCard>()`,
    `size_of::<HotMonster>()`, `size_of::<FanoutState>()`,
    `size_of::<Frame>()` budgets). Exceeding a
    budget is a compile error on the PR that does it. Budgets are set in
    R0 from measured baselines (Queen's 656-byte string-laden hot state is
    the cautionary tale; Byrdonis's 144 bytes the aspiration);
  - no `String`, no `BTreeMap`, no per-node `Box` chains in any hot type.

### D4. Dispatch scaffolds are generated complete, waves fill bodies in disjoint files

The mechanical trick that makes 30+ parallel PRs conflict-free:

- At R0, a generator emits the **complete** dispatch surface: `StepKind`
  (314+ variants), `MoveKind` (91), potion, hook, and select-op dispatch —
  every arm delegating to a stub in a per-family module that returns
  `Err(Refusal::UnimplementedKind(kind))`.
- The top-level `match` files are written once and **never edited by wave
  PRs**. A wave PR replaces stub bodies in its own family file
  (`src/steps/attack_family.rs`, `src/moves/wave2.rs`, …) and flips
  nothing shared. Slice boundaries = file boundaries, so conflicts are
  impossible by construction — the same property CLAUDE.md already relies
  on for `content/` files.
- The three Python "files-within-a-file" that every wave would otherwise
  collide on (`_run_steps_inner`, `monster_act`, `_apply_action_impl`) are
  therefore born split in Rust.

Step arguments: card `steps` rows are heterogeneous tuples in Python. The
generated tables store them as `Step { kind: StepKind, args: &'static [Arg] }`
with `Arg` a small int/id/flag union. Each kind's implementation interprets
its args and the **admission gate validates arg shapes at combat start**
(refusing on surprise), so the per-play hot path never re-validates.

### D5. Hooks: registered subscriber lists, never scan-everything

Python's 12-hook relic taxonomy (`_RELIC_HOOKS_SUPPORTED`) plus the 8
`_powers_*` fan-outs are the spec; ordering is part of the spec (there is a
dedicated Python ordering test). The Rust framework is new code (nothing in
the kernel prefigures it):

- at combat start, build per-event subscriber lists from the relics and
  powers actually present — O(this fight), computed once, **not cloned per
  node** (immutable per-combat context lives beside the hot state, like
  the catalog);
- firing an event walks its subscriber list in the Python-defined order;
  an empty list is one branch;
- presence bitsets (relic categories, power categories) gate whole hook
  families;
- `start_combat`'s ~155 sequential relic blocks become per-relic
  `CombatStart` subscribers registered from per-family modules — which
  also disperses the one shared Python function every relic PR would have
  collided on.

### D6. Refusal is typed, admission is at entry, and refusal parity is gated

The I5 exactness culture ports over:

- `Refusal` is a typed enum naming the missing mechanic (kind, id, site) —
  never a string, never a panic, never an approximation;
- **admission is all-or-nothing at combat start** (per #1138): walk the
  entry closure (encounter, deck, relics, potions, enchantments); if any
  required kind/hook/id is unimplemented, refuse the whole fight with the
  full missing-list. The capability manifest is *derived* from the
  implemented-kind registry, replacing the kernel's hardcoded per-encounter
  manifests;
- the differential checks **refusal parity**: on entries/actions where
  Python raises `NotImplementedError`, Rust must refuse (and vice versa —
  a Rust refusal where Python proceeds is a red diff, not a skip). Matching
  is on occurrence and site, not message text.

### D7. Events carry interned payloads and allocate nothing per hit

`QueenEvent::MonsterHpChanged { source: String }` heap-allocated per damage
event and `queued_move.clone()` ran per hit — both scale with content, both
banned. The generic event type is `Copy`-able or near (interned ids +
integers); the sink either counts, buffers into a reused arena, or
serializes lazily at the canonical boundary. The differential's
event-token comparison serializes *outside* the hot loop.

## 4. The gating artifact: the generic trajectory differential

Generalizes `queen_differential.py` per #1282. This is R0's highest-leverage
deliverable and the gate every wave PR runs under.

**Design (deltas from the Queen version):**

- **Entries are synthesized, not hand-curated.** A seeded Python generator
  composes entry states directly: encounter × sampled deck (biased toward
  the family under test) × sampled relic/potion sets, constructed through
  `start_combat` and projected to canonical JSON. No archived save, no
  capture, no seven-phase fixture ceremony — port parity needs *valid*
  states, not *historical* ones. (Captures remain the sim-correctness
  oracle; out of scope here.)
- **One projection function.** A single canonical projection from
  `combat_sim.State` → wire JSON lives beside the driver, replacing the
  per-encounter bespoke projections. Versioned schema, shared by
  admission, differential, and future authority-flip machinery.
- **Uniform-random policy over legal actions, seeded.** The Queen corpus's
  weighted policies existed to *reach* hand-tagged states; the port gate
  wants breadth, and breadth comes from K × entries, not curated policies.
- **Compared every step:** legal-action lists (exact, ordered), full
  canonical projection, refusal parity (D6), terminal agreement. Any
  divergence reports the seed + reproducible action prefix.
- **Coverage is derived, not asserted.** The driver reports which
  `StepKind`s/`MoveKind`s/hooks each run exercised. Smoke configuration is
  a **directory of per-family files** discovered by glob (never one shared
  list — that would put a merge conflict in every wave PR): a wave PR adds
  its own `smoke/<family>.json` biasing the sampler toward its content,
  and the gate asserts every newly-claimed kind was actually exercised —
  "green because untested" is not green.
- **Persistent process.** `sts-sim diff-serve` speaks line-delimited JSON
  over stdin/stdout (load entry / legal / apply / project), so K
  trajectories cost one process, not K `cargo run`s.

**Landed at R0.7 (#1291):** `tools/differential.py`, the per-family configs in
`smoke/`, the `diff-serve` `coverage` command that makes the coverage check
derived, and both CI lanes. `DIFFERENTIAL.md` is the operator's document — the
config schema a wave PR writes, how to replay a divergence, and what red means
in each lane.

**Two scales:**

| lane | where | scale | role |
|---|---|---|---|
| PR smoke | ubuntu runner, every port PR (non-gating since 2026-08-24) | bounded K per touched family + a fixed cross-section corpus, target ≤ ~3 min | the per-PR parity report |
| nightly fuzz | mac-solver, **on demand only** (`workflow_dispatch`) | large K, all families, long trajectories | the dragnet, held for the validation phase; red = a filed divergence ticket, not a halt |

**The dragnet no longer fires on its own (#1548).** Per the 2026-08-23
decision record on #1282, parity is the destination, not the invariant:
continuous large-scale parity testing stops, so the daily scale-20 cron and
the per-merge scale-5 push run are both gone and `rust-port-nightly.yml` runs
only when someone dispatches it. Semantic divergence — the one class review
and static analysis cannot reach — is deferred to a dedicated validation
phase near parity, when the differential runs once against a mostly-complete
sim rather than continuously against a moving one. What remains continuous is
code review, build/lint/tests, targeted seam tests, and the static
refusal/projection censuses.

## 5. Codegen: content tables are generated, never hand-transcribed

A Python tool (`tools/generate_content.py` in the new crate) imports the
authoritative `combat_sim` registry and emits checked-in Rust:

- `src/ids.rs` — the dense enums + string tables (D2);
- `src/content_tables.rs` — card rows (cost, typed steps, tags, flags),
  monster loops and `*_MOVES` tables, encounter rosters, template-relic
  steps, generation pools, censuses-derived constants.

This deletes an entire class of wave work (the ~4,200 lines of module-level
literals and the 6,600-line `content/` package become one regenerable
artifact) and an entire class of bugs (typo'd constants). Freshness is
CI-enforced: the port workflow reruns the generator and `git diff
--exit-code`s the output (ubuntu already runs `combat_sim` imports for the
kernel differential today). Encounter *builder functions* (89, small, some
RNG-consuming) are code, not data — they are a normal wave slice.

### 5.1 Where the facts come from (`--source`, #2496)

"Generated from the registry" was the right call while Python was the
modeling surface of record. The 2026-09-15 decision-record addendum on #1282
breaks that assumption: **D3** freezes `combat_sim` at v0.111.0 and **D4**
forks only the crate, so a forked `sim/vNEXT/engine/` regenerating from a
frozen registry would reproduce *v0.111.0 content* on a new build. Codegen
therefore has two sources, selected by `--source`:

- `python` — the behaviour above, unchanged;
- `dll` — `tools/dll_content.py` reads the facts the game actually owns out
  of `sts2.dll` by static IL: the ModelId inventory behind `CardId` /
  `RelicId` / `PotionId` / `EnchantmentId`, the `CardRarity` vocabulary,
  per-card rarity / type / target type / canonical cost / canonical keywords
  / canonical tags, the native-Unplayable census, and the unlock-epoch
  universe.

The split is a **declared** artifact, not a residue: everything the assembly
cannot supply to a static read is named in `dll_content.MODELING_ANNEX` with
where it comes from instead (the op-language vocabularies, step programs,
move tables, template-relic rules, dispatch, the modeled `EncounterId` /
`MonsterKind` axes, and the upgrade-level deltas that live in `OnUpgrade`).
`dll_content.py --ledger` prints the whole boundary; `--report` diffs the two
sources fact by fact.

Under `--source dll` the identity axes are *reconciled*, not merely
preferred: content the assembly has and the registry does not — or the
reverse — raises, because on a new build that difference **is** the porting
work and must not be resolved by whichever source happened to win.

The acceptance bar is byte equality on v0.111.0
(`tools/test_dll_content_source.py`), because the next build offers nothing
to compare against. It is local-only — the DLL archive is gitignored, so the
test skips in CI and takes `--require-dll` when evidence is wanted.

**Default since 2026-09-16 (D3 in force): `--source auto`, which resolves to
`dll` wherever the archived v0.111.0 assembly and its IL reader are both
available, and otherwise falls back to `python` with a loud note on stderr
naming why.** The assembly is the authority under D3, so the authoritative
path is what an unqualified invocation gets; the fallback exists because the
archive is gitignored and CI's interpreter has no `dnfile`, and it announces
itself rather than silently downgrading. Every run prints the source it
actually used (`content source: <kind> — <description>`), so no artifact is
ever generated without its provenance on the record. `--source python` and
`--source dll` remain available and override the resolution.

## 6. CI: fast parallel pre-submit, heavy post-submit

### Python citation integrity

> **Retired (#2827 item F).** Item F hard-deletes `combat_sim.py`, so there
> is nothing left to cite by line. The line hints in `src/**/*.rs` were dropped
> in one mechanical pass that kept each Python function name, labelled as the
> frozen oracle ("frozen Python `_fn`, deleted #2827"), and
> `tools/check_python_citations.py` and its test were deleted with it. Native
> semantics are cited by IL RVA/offset in the `///` docs, as they already were
> for Rust-first mechanics. The rest of this section records the contract as
> it ran; its commands no longer exist.

Rust comments cite `combat_sim.py` with a Python function name and often an
`L<number>` navigation hint. The **function name is the load-bearing anchor**;
the line number is advisory and must not be treated as durable evidence. An
ordinary oracle insertion can move hundreds of otherwise-correct line hints
without changing the cited function or the Rust behavior.

`tools/check_python_citations.py` owns this contract. Its current-tree audit
parses raw line citations in `src/**/*.rs`, `DIFFERENTIAL.md`, and
`PORT_PLAN.md`; resolves backticked or `combat_sim.<name>` function anchors
against Python AST spans; and reports anchorless or out-of-span citations.
The two process documents are an explicit external surface so a new utility or
generated file containing test-shaped `L<number>` literals cannot silently
join the contract. Its revision-aware mode maps only
byte-for-byte unchanged Python lines across a Git diff, which detects drift
even when an old number still happens to fall inside a large function:

```text
python3 tools/check_python_citations.py --historical OLD NEW --report-only
python3 tools/check_python_citations.py --baseline OLD --fix
```

`--fix` first requires the Rust source tree to be unchanged from `OLD`, then
rewrites only an unchanged shifted target whose unambiguous named function
owns the line on both sides. A citation whose target was edited/deleted, lacks
a function anchor, uses a duplicate function name, or crosses function
ownership is a manual finding and is never guessed, deleted, or weakened.

The Rust-port workflow runs the focused checker tests, current audit, and
base-to-candidate revision audit in strict mode. The revision audit reconciles
an already-refreshed working comment by its number-masked citation line, so an
oracle insertion is red only until its hints are updated; unrelated Rust edits
do not defeat that proof. A manual restatement must leave a valid citation on
the corresponding edited Rust line, and a per-file raw-citation count decrease
is red unless Git proves a clean candidate directly restores an audited
baseline's parent tree. Both candidate and baseline must be single-parent
commits: refusing a merge baseline prevents its first parent from being an
unaudited, reversed-parent feature tip. The restored commit must differ from
the candidate, no skip-worktree or assume-unchanged index flag may hide disk
edits, and the complete candidate/restored trees must match. That exact tree
restores an unambiguous already-audited ancestor, so after the current-tree
strict audit the historical revision findings are accepted too, including
oracle code and citations deleted by the rollback. The proof uses commit
parent and complete tree identity, never a `This reverts commit` message, so
ordinary removal, mixed rollback-plus-edit commits, merge baselines, hidden or
visible dirty working trees, and missing history stay red. Git plumbing errors
are captured so declining the exception does not leak a misleading `fatal:`
above the real citation finding. The workflow fetches the baseline and its
parent for that comparison.

The merge-baseline refusal deliberately includes a legitimate
`git revert -m 1` of a GitHub-order merge. The push event provides the candidate
and `PUSH_BASE` (the merge), while `origin/main` already names the candidate; it
does not provide an older audited-mainline ref or attestation that identifies
which merge parent previously held `main`. Checking whether `baseline^1` is a
first-parent ancestor cannot fill that gap: it is true by construction for both
the legitimate merge and a reversed-parent merge whose first parent contains
an unaudited citation removal. If every other direct-parent, cleanliness, and
complete-tree premise holds, the tool therefore keeps the real
`citation_count_decreased` failure and adds a narrow `merge baseline`
explanation. A mixed rollback or unrelated citation failure gets no such
message. Supporting merge-commit restores requires a separate trusted
audited-mainline record; it must not be inferred from the existing Git graph.

Within the Rust-port CI workflow this
exception is deliberately reachable only on a direct push, where `HEAD` is the
real single-parent revert and `PUSH_BASE` is the reverted commit. (A local tool
invocation can provide the same single-parent shape.) On a `pull_request` event
GitHub checks out a synthetic two-parent merge ref, so the same predicate
refuses a revert PR.
Supporting revert-by-PR would require a separate checkout/ref design; it must
not be inferred by weakening the parent proof. A baseline-fetch infrastructure
failure emits a warning and
skips only the revision comparison. The current-tree strict audit always runs
first and still fails anchorless, ambiguous, or out-of-span citations.

**Pre-submit — required, GitHub-hosted ubuntu, parallel across PRs, target
under ~10 minutes wall:**

1. `cargo fmt --check`, `cargo clippy --locked -- -D warnings`;
2. `cargo test --locked` (unit + contract tests, includes the compile-time
   size pins and the deterministic allocation pins below);
3. codegen freshness (`generate_content.py` + `git diff --exit-code`);
4. coverage-badge freshness (`build_coverage_badge.py --check`): the README
   banner and `COVERAGE.md` are derived from the per-family `IMPLEMENTED`
   registries, so the check pins the structure exactly and asserts the
   committed banner never overstates the crate and never lags it by more
   than 5% of the total. Deliberately not a `git diff --exit-code`
   freshness check — that would make the banner a file every wave PR edits,
   which is the shared-file conflict §4 rejects. Refreshing it is one
   command and a one-actor, three-file PR;
5. differential smoke (§4), including the touched-family coverage check.

Nothing here touches the pytest lane, the self-hosted runner, or any
serialized resource; two hundred port PRs can gate concurrently.
Rust-cache (`Swatinem/rust-cache`) keeps compile time flat.

**Deterministic perf pins at PR time (fast, not noisy):**

- compile-time size assertions on hot types (D3) — free;
- **allocations-per-transition ceilings**: a cargo test under
  `--features allocation-counting` replays a fixed seeded workload and
  asserts exact/ceiling allocation counts. Allocation counts are
  deterministic — this is a reliable equality-style pin that catches
  structural creep (a `String` in an event, a clone in a hit loop) on the
  PR that introduces it, with zero benchmark noise;
- a grep-style contract test banning string comparison patterns in hot
  modules (the ~69-site regression the kernel accumulated).

**Post-submit — mac-solver, push-to-main, stop-the-line on red:**

- transitions/s and complete-playouts/s floors per eval-suite fight
  (#1283; until it lands, a synthetic fixed workload corpus), compared
  against committed baseline JSONs with an explicit tolerance;
- peak-RSS ceilings.

This mirrors the just-landed slow-pins precedent (#1280): throughput
measurement is real-hardware-only and noisy, so it never gates a PR; it
runs per-merge so a red run is attributable to a single commit.

The large-K differential fuzz used to run here too, per-merge and nightly.
It no longer runs automatically at all (#1548, above): perf floors are a
build property and stay stop-the-line, while parity is a destination checked
deliberately in the validation phase.

**Keeping port PRs out of the solver gate.** Today `TRIGGER_SET` is
`versions/**` + `solver/**`, so every port PR would queue a ~3-minute
solver suite run on the single self-hosted runner — a serialization tax on
exactly the wave we are trying to parallelize, guarding tests that read
nothing under the new crate. Before the wave starts, narrow the trigger set
to the surfaces the suite actually reads (`sim/*/python/**` plus the
enumerated v0.110.1 kernel couplings), and make the
`test_pytest_lane_process_contract` guard *derive* the external surface
instead of asserting a literal list — which is #1276's proposed fix; this
work folds into it. Port tests are cargo-native precisely so the solver
suite never grows a dependency on the new crate; if one ever becomes
necessary it goes through the #1276 guard deliberately.

## 7. Performance discipline

The rules (D2/D3/D5/D7) prevent the creep classes already observed; the
pins and floors (§6) are the guarantee. Division of labor:

- **compile-time**: size budgets on hot types;
- **PR-time, deterministic**: allocation ceilings, string-compare contract
  test, differential smoke;
- **post-submit, measured**: throughput floors, RSS ceilings, per
  eval-suite fight, on fixed hardware.

Baselines live as committed JSON beside the benchmarks (the kernel's
artifact-hash + environment schema is reused). A floor change must be an
explicit reviewed diff with a stated reason, never a side effect.

### Hot-type size budgets (set at R0.4, #1288; measured again at R0.5, #1289, and at slice 2, #1367)

Asserted by `const _: () = assert!(size_of::<T>() …)` in `src/hot.rs` and
`src/frame.rs`, so exceeding one is a compile error on the PR that does it.
Budgets carry headroom for the fields later slices claim; the measured column
is what the crate actually lays out on x86-64/aarch64.

| type | measured | budget | note |
|---|---|---|---|
| `HotCard` | 8 | **== 8** | an equality, not a budget (D3) |
| `HotState` | 224 | ≤ 224 | #1486 raised the ceiling from 208 for two pointer-sized consumers. Selection spent one word; Ringing's alignment step spent the other. #1674's ally key fit existing padding, while #1675 and #1562 put the pet and exact remote Player behind the existing `HotFanouts` handle. No inline reserve remains |
| `FanoutState` | 256 | **== 256** | out-of-line behind the one-word `HotFanouts` COW handle. #1675 measured 120 after adding the solo pet; #1562 measured 256 after adding the exact remote-Player/resource/listener quotient. Growth here is paid by the allocated-bytes performance ceiling rather than the `HotState` const ceiling |
| `HotMonster` | 96 | ≤ 96 | was 40 at R0.4, 48 at R0.5; slice 2 added `poison_uid`; later slices packed owner-disjoint private state through 88. #1675's distinct pet-dealer powered-result counter consumed the reviewed 96-byte cap. Multiplies by roster size — the roster vector is memcpy'd on any monster write |
| `HotHistory` | 52 | ≤ 52 | inline: every card play writes several of its counters, so an indirection would allocate per transition to save 52 bytes. **Raised from 32 at slice 2 and from 48 by #1374** — see below |
| `Frame` | 8 | ≤ 32 | box-free; a variant wanting more is a variant to rethink |
| `Slots<K>`, `HotPile`, `CardStates`, `Frames`, `HotRng`, `MiseryOrder`, `HotFanouts` | 8 | one pointer | copy-on-write handles |

**The one budget slice 2 moved, and why.** `HotHistory` went from `≤ 32` to
`≤ 48` (measured 44) when five `State` counters the slice's new primitives
write joined it: `zero_energy_attack_plays_started_this_turn` (#1314),
`non_hand_draws_this_turn` and `discarded_cards_this_turn` (the step-callable
Draw and `_discard_and_draw`), and `owner_cards_exhausted_combat` /
`owner_card_exhausted_this_turn` (the exhaust pile op). Each is a field
`project_state.py` emits whenever it is non-default, so *not* carrying one is
a divergence on the first transition that touches it rather than a saving —
#1314 anticipated exactly this ("adding a counter is a budget decision"). At
that point the enclosing `HotState` measured 168 bytes against its 200-byte
budget, and the block stayed inline for the reason it always had.

Issue #1374 adds `shiv_plays_finished_this_turn`, the canonical
`CardPlayFinished` filter used by generated Shivs. That widens `HotHistory`
from 48 to 52 bytes; together with that slice's other canonical fields,
`HotState` measured 184 bytes and its ≤200 budget was unchanged. #1394's orb
queue then measured 200 and #1406's card-event fan-out measured 208. #1480's
Stars balance and per-turn gain counter use existing padding, leaving the
measured state at 208.

Issue #1486 raised the deliberate ceiling to 224 before the next state-shape
slices landed. Selection's nullable copy-on-write pending handle (#1363) spent
one pointer-sized word: its closed tag, candidate UIDs and resumable payload
live behind `Option<Arc<PendingSelection>>`, whose non-null `Arc` niche is one
word (mechanically pinned in `hot.rs`). Ringing's canonical player latch later
crossed the final alignment step and spent the other word. The compile-time pin
therefore uses the full `≤224` ceiling and the runtime detector records
`HOT_STATE_MEASURED = 224`; there is no remaining inline roster reservation.

The second budget review (#1689) found that both named consumers had already
taken the out-of-line route instead of adding another `HotState` handle. #1675
put the singular solo pet in `FanoutState`; #1562 put the exact one-remote-
Player quotient needed by the supported two-player vehicle in that same block.
Both share the already-present one-word `HotFanouts` handle, and #1674's stable
remote key fits existing sub-word padding. #1701 adds the remote Colorless-pool
provenance bit without changing the measured 256-byte fanout block. Raising or
packing `HotState` without an inline consumer would only buy speculative
headroom, so the reviewed ceiling remains 224.

Out-of-line is a placement decision, not a claim that the payload is free.
`FanoutState` grew from 120 to 256 bytes in #1562 and its exact equality is
pinned in `hot.rs`. The post-#1701 performance measurement is 909.9 allocated
bytes/transition against the authorized 943.1 ceiling: 33.2 bytes (3.5%) of
measured margin. Any later fanout growth needs a fresh measurement just as an
inline field needs the size pins; `benchmarks/floors.json` does not move merely
to accommodate either one.

Waterfall (#1472) adds no `HotState` field; its exact private values live in the
separately budgeted `HotMonster`.

Reference points: the v0.110.1 kernel's 144-byte Byrdonis hot state is the
aspiration, its 656-byte text-laden Queen state (`queen.rs:1047`) the
cautionary tale.

One current `HotState::clone` with no pending selection is **12 atomic
increments** (5 piles + monster roster + player powers + card side table +
frame stack + the rng block + the orb queue + the card-event fan-out block)
plus a 224-byte memcpy. A live `Some(Arc<PendingSelection>)` adds one increment,
for **13**. Nothing walks a pile, a roster, a power set, an orb queue, a
listener order, or a stream.

**The RNG block is hot, not catalog-side.** R0.4 parked the xoshiro seed words
in the catalog on the theory that search never re-seeds mid-fight. That holds
for the *seed* but not the *state*: a reshuffle advances `s.rng`'s four words
in place (`combat_sim._rng_from` / `_rng_tuple`), and the canonical document
carries the advanced words, so a catalog-side copy re-emitted the entry words
after the first reshuffle. R0.5 moved the whole stream state into a
copy-on-write `HotRng` — which also shrank `HotState`, because 72 bytes of
inline counters became one pointer.

Raising a budget is an explicit reviewed diff with a stated reason, exactly
like a throughput floor — never a side effect of the change that needed it.

### Allocation ceilings

`allocations_per_transition_stay_under_the_ceiling` (`src/engine/mod.rs`,
`--features allocation-counting`) replays the checked-in scripted line eight
times with a reused event buffer and asserts an allocations-per-transition
ceiling. **Measured at R0.5: 10 per transition** (1,256 allocations across 120
transitions), ceiling 12. The measured window covers `apply_action_into` only;
the canonical projection and the legal-action enumeration are boundary and
enumeration work that allocate by design.

## 8. Sequencing

**R0 — foundation (serialized, `area:engine-core` discipline, 2–3 owners
max, ordered):**

| # | deliverable | notes |
|---|---|---|
| R0.1 | crate scaffold: `Cargo.toml`, seeded `rng`/`decimal`/`dotnet_sort`/`allocation`, CI workflow (§6 pre-submit) | small; unblocks everything |
| R0.2 | codegen: `ids.rs` + `content_tables.rs` + freshness check (§5) | defines every enum the waves name |
| R0.3 | canonical schema v2 + the one projection function (Python side) + `diff-serve` skeleton | the wire contract |
| R0.4 | hot state + catalog + boundary + continuation `Frame` enum (D3) | the hard layout work; complexity:high |
| R0.5 | engine core: damage pipeline (`player_attack`, `damage_monster`, `monster_attack_player`, death cascade), turn structure (`start_combat`/`begin_player_turn`/`end_player_turn`/`_run_enemy_phase`), `legal_actions`/`apply_action`, draw/shuffle | the 27 big functions, ported by hand, few PRs, sequenced. **Slice 1 landed** (#1289): the starter-deck-vs-TOADPOLE vertical, with `tools/gen_slice_pins.py` + `tests/slice_differential.rs` as its parity evidence and everything else refusing through `engine::admission` |
| R0.6 | hook dispatch framework (D5) + admission/refusal (D6) + generated dispatch scaffolds with refusal stubs (D4) | after this, every kind refuses loudly |
| R0.7 | generic trajectory differential (§4) + PR smoke + nightly fuzz wiring; trigger-set narrowing (#1276) | the gate goes live |

R0 exit criterion: a synthesized entry using only basic cards
(Strike/Defend-class) runs end-to-end in both engines, differential green,
all other content refusing with typed reasons, allocation pins and size
pins in place.

**R1 — the wave (parallel, `complexity:medium`, conflict-free by
construction):**

Slices are per-family files; each PR = implement 20–40 stub bodies + add
its own per-family smoke config + green differential. Estimated from the
survey: ~11 PRs of card step kinds, ~3 of monster move kinds, ~2 of
potions, ~3–4 of relic hook families + template interpreter, ~3–4 of
encounter builders, ~3 of the card-instance layer, ~2 of frame drivers,
plus the orb engine and generation subsystem as their own small runs —
**roughly 30 wave PRs** after R0. Wave workers follow a
`RUST_WAVE_PROMPT.md` (to be distilled from `solver/WAVE_PROMPT.md` at R1
kickoff, carrying the worktree-guard, branch-from-main, and one-Closes-
per-line lessons).

### R1 wave process (multi-vendor, GitHub as the only synchronization primitive)

Decided 2026-08-17 (Sean): workers may be Claude Code, Codex, or Gemini
Antigravity agents; the coordination substrate is GitHub issues, PRs, and
labels — no side channels, no shared memory, nothing vendor-specific. This
is already the repo's native pattern (claim convention, label-driven
high-tier review queue); the wave reuses the shapes rather than inventing
new ones.

- **Issues are pre-generated, one per family file.** A tool emits every
  wave issue mechanically from the kind inventory: exact kind list, Python
  source line ranges, target Rust file, smoke-config filename, and the
  embedded process contract (worktree guard, branch from current
  origin/main, one Closes per line, bun-not-npm, no pytest lane). Issue =
  complete worker prompt for any vendor. Disjointness is guaranteed at
  generation time, not policed at review time.
- **Claiming:** self-assign (or comment with a branch name where the
  vendor cannot assign). One issue per worker at a time. A claim older
  than 24h with no pushed branch is stale and may be re-claimed.
- **Review queue by label,** mirroring the high-tier protocol: worker gets
  the required checks green on the exact head SHA, applies
  `needs-wave-review`, and stops. A small fixed pool of review agents
  watches the label and either approves (swap to `wave-approved`, with a
  comment recording what was checked) or swaps to
  `wave-changes-requested` with findings. Re-label to re-enter the queue.
- **Workers merge their own PRs** once `wave-approved` + required checks
  green on the final head. Note the enforcement honesty: most agents act
  as the same GitHub account, so GitHub cannot mechanically require the
  approval — the labels are procedural. What IS mechanical is branch
  protection on the required checks (the port workflow, and the
  differential smoke within it), which is where the correctness weight
  deliberately sits. If stronger enforcement is wanted later, review
  agents get a separate bot account whose PR approval becomes a branch-
  protection requirement.
- **Review is contract-checking, not re-derivation.** The differential
  already proved behavioral parity; reviewers verify the cheap-to-check,
  expensive-to-miss things: no strings below the boundary, no shared-file
  edits, no new deps, refusal stubs not weakened, smoke config actually
  covers the claimed kinds, size/allocation pins untouched. That is what
  keeps review agents a *small fixed pool* and the token bill sublinear
  in N.
- **Escalations follow the claim convention:** a worker discovering a
  missing engine primitive files an issue, marks its PR blocked, and
  moves on (or takes another slice). Engine primitives land through the
  serialized engine lane, never inline in a wave PR. Expect the #138
  pilot's ~10–15% escalation rate, higher in the first batch.
- **Do not enable "require branches up to date" on main.** With
  file-disjoint PRs, merges compose without rebasing; requiring
  up-to-date branches would invalidate every queued PR on every merge —
  O(N²) CI runs for zero conflict protection the file layout doesn't
  already provide.

**How large can N be?** With the D4 layout plus per-family smoke configs,
wave PRs share **zero files**, so textual merge conflicts are eliminated
by construction and N is not conflict-bounded at all — it is bounded by
the serial capacities around the wave:

1. **escalation handling** — at ~10–15% of PRs needing an engine
   primitive, the serialized engine lane's throughput caps useful N
   early in the wave;
2. **review throughput** — a mechanical 20–40-kind PR with a green
   differential reviews in tens of minutes; 2 review agents sustain
   roughly 10–15 merges/day without queueing;
3. **token budget** — workers beyond the review+escalation pipeline's
   drain rate just build queue, which buys nothing.

Concretely: start batch 1 at **N = 4–6** (escalation-heavy, primitives
still shaking out), grow to **N = 10–15** once a batch completes with
escalations under ~10%, and treat ~20 as the ceiling where claim races
and coordination overhead outgrow the win. Workers are
`complexity:medium` tier (Claude Opus 5 / GPT-5.6 Sol High per the
triage matrix); Fable-tier effort is reserved for the engine lane and
escalation adjudication.

What is deliberately **not** ported: the ~250-function validation wall and
the provenance/evidence tuples. Those are Python-side modeling-process
artifacts (IL citations, batch-entry shape assertions). Their
refusal-bearing content folds into the admission gate; their evidence
content stays in Python — as the **frozen v0.111.0 record** of how that
build was modeled, not as a live surface.

**Amended 2026-09-16 (#1282, D1–D4 approved by Sean).** "Python remains the
modeling surface of record" was true when this paragraph was written and is
not true now: `sim/v0.111.0/python/combat_sim.py` is frozen at v0.111.0
and new mechanics are read from IL into **Rust** first (D3). The validation
wall and the evidence tuples are still not ported — that part is unchanged
— but what stays behind is a closed record rather than the place the next
mechanic gets modeled.

**R2 — search integration and beyond (out of wave scope):** one
`UctDomain` implementation over the generic engine, benchmark schema per
#1138, authority-flip machinery per `SIM_VERSIONING.md`. **Python retirement
is decided, not gated (Sean, 2026-09-16, #1282 D1–D4).** The condition the
original sentence named — capture-based ground truth becoming the oracle —
was met by the game's own per-action `.mcr` checksums plus the #2470 fixture
tree, so certification runs against those (D1) rather than against a
per-build Python oracle, and only the crate and the parsing/projection layer
fork forward (D4). Search authority had already flipped in #2450; this closes
the rest. The mechanics of the flip are §9's resolved decisions 4–7.

## 9. Resolved decisions (Sean, 2026-08-17; amended 2026-09-16)

1. **Version-bump policy during the port:** decide when it happens, with
   the working assumption of finishing on v0.111.0 (the differential
   oracle is per-build) and forking forward per `SIM_VERSIONING.md`
   afterwards. Not re-litigated mid-wave unless the bump forces it.
2. **The v0.110.1 kernel:** stays where it is, frozen, for search
   experiments and posterity. Its Queen/Colony certification fixtures
   remain valuable as differential corpus entries for the new engine.
3. **Wave staffing:** multi-vendor worker pool with GitHub as the sole
   synchronization primitive, N parallel workers + a small fixed review
   pool, workers merging on recorded approval — the full process and the
   N sizing rationale are §8's "R1 wave process".

### The authority flip (Sean, 2026-09-16, #1282)

Approved in full from the 2026-09-15 decision record and its addendum. These
supersede decision 1's "working assumption of finishing on v0.111.0 before
forking": the fork happens at the next build and only the crate forks.

4. **D1 — Oracle.** Rust engine correctness is certified against the game's
   own `.mcr` per-action checksums — the `sim/v0.111.0/eval/` fixture
   tree plus the `tools/eval_suite.py census` lockstep, measured on the
   **release** binary (#2472: dev-vs-release disagreement is a defect class,
   not noise) — and the headless harness for targeted experiments. Python
   parity becomes advisory evidence on v0.111.0 only; the trajectory
   differential of §4 gates nothing on any later build and stays as a
   v0.111.0 regression tool.
5. **D2 — Entry.** Rust builds its own root from the save/upload payload
   (the seeding scheme, counters, deck/relic/potion/unlock provenance)
   instead of loading a document Python projected. Python's entry refusals
   stop gating Rust; Rust carries its own typed I5 refusals at that boundary
   (D6's admission-at-entry rule, applied one layer further out), and the
   census measures admission from every capture rather than from the
   Python-admitted subset. This is the one large item and it is an
   `area:engine-core` lane of its own.
6. **D3 — Modeling surface.** New mechanics are read from IL into **Rust**
   first, cited in the ported function's adjacent `///` doc comment, with
   exactness-or-refusal (I5) unchanged — no `ENCOUNTER_MECHANICS.md` entry is
   owed for port work. `sim/v0.111.0/python/combat_sim.py` is **frozen**
   at v0.111.0: no remodel for a new build, no new mechanics, bug fixes only
   where they change a v0.111.0 parity verdict. P1 of this (codegen sourced
   from the DLL rather than from the frozen registry) landed in #2503 with
   byte-for-byte reproduction, and §5.1's `--source` default follows from it.
7. **D4 — Fork-forward.** Only the Rust crate and the parsing/projection
   layer fork to the next build; the Python engine does not fork. The shared
   layer is language-neutral by placement — one copy, consumed by the
   authority engine — and Rust certification targets **head-and-follow**
   rather than a pinned lagging build, which the checksum oracle makes
   possible because it exists for every build the mod captures. Per-build
   *data* artifacts (P2) live beside the crate that consumes them,
   `sim/<new>/engine/data/`, never written into the frozen Python tree.
   `solver/VERSION_BUMP_RUNBOOK.md` is the executable form of this.

**Open:** the post-release review blackout (item 3 of the proposal). Until
Sean rules, the default is unchanged — reviews on a new build refuse until
the checksum census certifies the encounter, which is what D1 produces
anyway.

# v0.111.0 eval fixture set

![eval coverage](coverage-badge.svg)

Real captured fights, build-keyed like the simulator (`solver/SIM_VERSIONING.md`).
This is the standing input to solver-quality work: sub-issue B writes optima
(`expected.json`) here, sub-issue C measures perf on the same roots, and the
port programme reads the census for its demand map (#2048, #1283, #1282).

**Since the 2026-09-16 authority flip (#1282, D1) this tree is the
certification, not a sample of it: Rust engine correctness means lockstep
against the capture's own per-action checksums, measured on the release
binary** — not parity with Python, which is advisory on v0.111.0 only.

## Coverage

![Eval suite coverage: boss and elite by character grid, and per-axis bars](COVERAGE.svg)

`eval_coverage.py` measures how much of fight-space the **certified** lines
cover. Each axis has one rule for "exercised": a card counts once it is
**played**, a relic when **held** entering a fight, and a potion when **used**.
Encounters are counted per tier.

The grid crosses every boss and elite with every character, empty or not;
that is the one cross product tracked, because the same boss plays
differently per class. An empty cell is coloured by why it is empty:

* **amber:** an A9/A10 capture exists but doesn't certify yet (engine work);
* **grey:** nobody has captured it at A9/A10 yet (a fight to play).

The amber/grey split comes from the census the tree was seeded from. It needs
the local corpus, so it is stored in `coverage.json` and carried forward.

Outputs: `coverage.json`, `COVERAGE.svg`, `coverage-badge.svg`. `seed`
regenerates all three, re-measuring the split from its census, and `add`
regenerates them from the stored split. `test_coverage_artifacts_are_fresh`
fails when they are stale.

The reference lists are a snapshot, `coverage_universe.json`, because their
sources sit outside the solver trigger set:

* cards and their owning pool: `data/cards.json`, and
  `data/game_values.json` `colors`, from the game's `AllCardPools`;
* relics: `data/relics.json`;
* potions and encounter titles: `data/loc_en.json`;
* encounters and tiers: `rust/ENCOUNTER_COVERAGE_CENSUS.json`.

Refresh the snapshot on a version bump:

```bash
python3 versions/v0.111.0/eval/eval_coverage.py                    # regenerate
python3 versions/v0.111.0/eval/eval_coverage.py --census C.json    # and re-measure the split
python3 versions/v0.111.0/eval/eval_coverage.py --refresh-universe # new build's lists
python3 versions/v0.111.0/eval/eval_coverage.py --check
```

## Layout

```
manifest.json                       every fixture: id, kind, categories, provenance, outcomes
fights/<opaque-id>/provenance.json  seed, node, encounter, character, ascension, build, sha256 pair
fights/<opaque-id>/entry.canonical.json   the sts-sim-canonical-v2 root
fights/<opaque-id>/human_line.json  canonical wire actions + a digest after each + terminal state
fights/<opaque-id>/refusal.json     for a refusal fixture: surface, class, verbatim detail, full blocker set
```

**Consent and privacy.** The captures were contributed by Sean and consenting
friends (recorded on #2048, 2026-09-05). A fixture carries an opaque id and
sha256 provenance — never a username, a capture file name, or raw `.mcr` /
`.save` bytes. `test_eval_suite.py` pins that.

## Kinds and categories

* `line` — Rust admits the root and the recorded human line replays exactly
  through it. The `certified` category is the subset whose every native
  checkpoint agrees (below); the rest are the acceptance set for the slices
  that still disagree with the game.
* `refusal` — the fight refuses, and the fixture records where, earliest
  surface first: `capture-pairing`, `rust-entry`, `rust-opening`,
  `rust-load`, `rust-line` (a recorded input with no exact Rust action) or
  `rust-lockstep` (a native checkpoint disagrees). These are pins in both
  directions: a slice that lands one of these classes turns
  `test_uncertified_roots_still_refuse_for_the_recorded_reason` red and the
  set is re-seeded. Since the #2915 re-seed every refusal names a Rust
  surface; the frozen oracle's `python-entry` / `python-line` are gone.
* `node:monster` / `node:elite` / `node:boss` name the room kind, so a
  harness can pick the hard fights without reading provenance. Every
  character carries a boss and an elite human line, except the named gaps in
  `test_eval_suite.KNOWN_LINE_GAPS` (today: the Regent boss, #3020).
* `human` is the baseline tag that licenses a solver-versus-human statement.
  It covers every A9/A10 human line, for all five characters (Sean,
  2026-09-23, #2915). From 2026-09-05 until then it was A10 Ironclad only,
  because the other characters' captures were low-ascension. Every fixture
  carries its ascension, so a low-ascension gap can never be reported as a
  baseline.

## How a fight is certified (#2999)

Everything below runs on the Rust engine alone; `eval_suite.py` imports no
part of the Python simulator.

1. **Root.** `sts-sim entry --save --opening` builds the whole root — entry
   facts and the opening. It is never spliced: the capture's first checksum
   (`After player turn start`) is *compared* with it (`opening_checkpoint`,
   plus the per-stream `opening_rng_drift` / `opening_counter_drift` the I6
   net reads), so an opening that consumed the wrong draws shows up rather
   than being overwritten.
2. **Recorded line.** Every recorded input is resolved to one of Rust's own
   wire actions, in order, by the production review's resolver
   (`solver/rust_replay.py`, #2988): the capture's own deal
   (`solver/mcr_native.py`) authenticates each `combat_card_index`, target
   ids are creation ordinals, a non-representative uid is applied as
   recorded, and every PlayerChoice goes through `_selection`. The census
   adds the SetupPlayerTurn Innate/IMBUED deal inversion the frozen
   `mcr_replay._deal_uid_map` made, multi-card choices, and exact state
   restoration between trial selections. `human` is `exact` when every input
   resolves and the combat ends exactly where the inputs do.

   `human` is `truncated` (`human_class` `capture_truncated`, #3277) when the
   capture itself stops mid-fight. The inputs run out with Rust's combat
   still live, every completed-action checkpoint agreed and was consumed,
   and the capture's final checksum shows a living player and a living
   enemy. Then one of two shapes (`truncation_shape`) must hold.
   `after_completed_action`: that final checksum is the completed-action
   checkpoint of the last recorded input, compared against Rust's state
   after it. `inside_player_decision`: the last input is a card play or
   potion that Rust leaves pending a choice, and its completed-action
   checkpoint is absent from the file. Either way, natively the game was
   waiting on a decision the file does not carry. The row carries
   `prefix_lockstep` in place of a `lockstep` verdict: a partial line is
   never certified. A combat Rust keeps alive after a final checkpoint that
   shows every enemy (or the player) dead stays
   `diverged:combat_not_complete`, and so does one whose last input is an
   end turn or whose checkpoints disagreed: an end turn runs a native enemy
   turn the file does not show, and a disagreeing line is not proved to have
   reached the capture's final state.
3. **Lockstep.** Every completed-action checkpoint the capture carries is
   compared as the line goes (`rust_replay.NativeChecks`): player resources,
   the live monster roster, each pile's ids and upgrades, and all nine RNG
   streams, with the player's pets — Osty against `player.ally`, relic pets
   against their fixed 9999-HP state — compared rather than miscounted as
   monsters. `lockstep_ok` is the certification; `checkpoint_mismatch` names
   the step and the field; `no_native_checkpoints` marks a capture that
   predates them.

`human_line.json` records Rust's wire actions and the engine's own
`differential_digest` after each. The tree was re-seeded from the Rust
census by #2915, including the two hand-added Insatiable fixtures, which were
re-`add`ed. Every root, line and step digest is therefore Rust's own, and
`verify` reproduces all of them.

## Selection policy: A9/A10 only (#2915)

Sean, 2026-09-23: **the suite holds A9 and A10 fights only.** A low-ascension
human line is too easy a target to say anything about solver quality. The
A0–A1 Defect set had no certified boss, and it hid a review-search weakness
that a single A10 Defect run exposed. `seed` applies `SEED_MIN_ASCENSION` to
line and refusal fixtures alike, then selects:

1. certified fights, at most `SEED_CERTIFIED_PER_ENCOUNTER` per
   (character, encounter), plus any further certified fight whose root holds
   a card, enchantment, relic, potion or monster kind no fixture so far
   certifies (`content_kinds`), so the cap never drops the only witness of a
   kind;
2. every recorded death;
3. every boss and elite encounter per character that has an exact line
   (certified first), then the potion, per-character and per-node quotas;
4. one refusal fixture per surface/class.

A `truncated` capture (#3277) is neither: it has no exact line, and no engine
surface refused it. `seed` leaves it out, and `add` refuses to write it.

Fixtures `add`ed with an explicit `--encounter` are kept across a re-seed
when they meet the floor, since the census cannot re-derive them.

## Upload source: `import-uploads` (#2915)

The mod's uploader keeps every uploaded capture locally, in
`~/Library/Application Support/SlayTheSpire2/relay_the_spire_uploader/review_provenance/`,
together with the entry projection the review pipeline resolves it against.
`import-uploads` writes each resolved room fight into the corpus as a
watcher-shaped triple under `<captures>/uploads/`:

* `SEED-NNN_mcr_<sha>.mcr` holds the capture's exact bytes;
* `SEED-NNN_save_<sha>.save` holds the capture's own embedded run with the
  uploader's unlock state, built by `rust_review.replay_capture_run`. This is
  the same input prod reviews root with `sts-sim entry --capture-run`;
* `SEED-NNN_upload_<sha>.json` names the `.run` fight's encounter.
  `derive_encounter` reads it first, and its presence makes `entry_input`
  root the pair with `--capture-run` rather than `--save`.

Fights the watcher corpus already holds are skipped, as are event combats,
unresolved provenance, and entry projections the review adapter rejects
(`deck_props`, `deck_upgrade`, `relic_membership`…). Each is counted by name
in the report. Friends' uploads live only in prod `fight_review_provenance`
rows, which Sean exports; agents do not read them.

```bash
python3 versions/v0.111.0/rust/tools/eval_suite.py import-uploads
```

## Regenerating

```bash
# the standing corpus report (the "done per encounter" table)
python3 versions/v0.111.0/rust/tools/eval_suite.py census \
    --captures ~/sts2-captures --census-json /tmp/census.json

# re-seed this tree from the corpus, by the documented breadth policy
python3 versions/v0.111.0/rust/tools/eval_suite.py seed --captures ~/sts2-captures

# one fixture from one capture pair
python3 versions/v0.111.0/rust/tools/eval_suite.py add \
    --mcr PATH.mcr --entry-save PATH.save

# re-derive every fixture from its provenance (locates the pair by sha256),
# and check every fixture's labels against the current census (#3270)
python3 versions/v0.111.0/rust/tools/eval_suite.py verify --captures ~/sts2-captures
```

`verify` makes two passes, and any finding from either exits non-zero.

1. **Digests.** It re-derives each rooted fixture's root, recorded line,
   step digests and terminal state. This pass skips any fixture with no
   `entry.canonical.json`, so on its own it cannot see a stale label.
2. **Labels (#3270).** It rebuilds each fixture's census row through the
   code `add` uses (`pair_row`), then compares the fixture with what `add`
   would write from that row. The manifest entry is checked field by field:
   kind, categories, `rust_root`, `rust_refusal_class`, `lockstep`,
   `entry_digest`, the refusal's class/surface/detail, and the
   turns/actions/HP/won summary. Every fixture file is compared too.

The label pass covers every fixture, including explicit-capture-pair
fixtures (re-rooted with their recorded `--encounter`), refusal fixtures with
no root, and unpaired captures. It lists each difference as `id: file.field:
committed X != census Y` under `label_differences`. It never repairs a
fixture: refresh the ones it names with `add`, then regenerate the roster
pins and coverage census as below.

One kind follows `seed` rather than `add`. A fight whose line replays but
whose checkpoints disagree stays a `rust-lockstep` refusal while the census
still names that refusal. Once it certifies, the refusal label is a
difference.

Both passes together take under a minute on the 532-fixture tree.
`--no-labels` runs only the digest pass. Like every command here, `verify`
needs the local corpus, so CI cannot run it. Run it with your head's release
binary before opening any PR that can move a verdict.

Every command needs a built engine
(`cargo build --locked --release --bin sts-sim`; pass `--binary` to name
another), since `verify` now re-derives the root and the line through Rust
too. The `--root-with` flag is gone: the `python` and `rust` (Rust entry,
Python opening) root sources were retired with the simulator (#2999), and
Rust's own opening is the only root.

Anything that rewrites `manifest.json` (`seed`, `add`) relabels the Rust
crate's roster pins, which record each corpus fight's fixture id keyed on
`(seed, node)`, and moves the `fixtures` column of the encounter coverage
census. The `rust port` lane checks both against the committed manifest, so
regenerate them in the same PR. The roster pins' cases are frozen Python
`make_monsters` measurements since #2999, so only their labels move:
`gen_roster_pins.py --relabel` rewrites them from the manifest with no
simulator. A pooled fixture that no pinned case covers (a fight the re-seed
added after the freeze) is listed in that pool's `unpinned_fixtures`; the
`.mcr` census, not a Python roster, certifies it.

```bash
python3 versions/v0.111.0/rust/tools/gen_roster_pins.py --relabel
python3 versions/v0.111.0/rust/tools/encounter_coverage_census.py \
    --json versions/v0.111.0/rust/ENCOUNTER_COVERAGE_CENSUS.json \
    --markdown versions/v0.111.0/rust/ENCOUNTER_COVERAGE_CENSUS.md
```

[`../rust/OPENING_CENSUS.md`](../rust/OPENING_CENSUS.md) is the last run of
the Python-vs-Rust opening parity census, retired with its tool by #2999 and
kept as frozen data. Per-fight opening coverage on the Rust root alone is the
census's own `stage` column (`opening_refused` rows name why).

## Solver research pilot (2026-09-17)

`search/` contains frozen development manifests, per-seed results, replay
validation, and a Rust/Python transition-throughput comparison. See
[`search/README.md`](search/README.md) for commands, selection policy, and
limits. Insatiable fixture `f143c7e6993ed27c` is now included.

**Terminology correction:** in the tree seeded before #2999 the `certified`
category and `provenance.checksummed` boolean do **not** establish complete
native checksum agreement. The former records Rust/Python digest lockstep;
the latter records use of the capture's opening checksum. A tree seeded by
the #2999 census means what "How a fight is certified" above says. Native certification remains the
required authority, but its result must be read separately. For Insatiable,
full native projection still refuses unprojected fields; the original human
line agrees on 33 limited numeric checkpoints and every Python/Rust action.
The generated alternatives still await in-game replay. An admitted root and
a passing human line do not guarantee coverage of every counterfactual path.

For a save missing the map's room kind, `add` accepts a paired, explicit
`--encounter ENCOUNTER.THE_INSATIABLE_BOSS --node-type boss`. Supply both from
capture/run evidence. The override is recorded in provenance and refuses a
conflicting encounter that can already be derived. Verification locates
capture pairs by content hash, including descriptively named files.

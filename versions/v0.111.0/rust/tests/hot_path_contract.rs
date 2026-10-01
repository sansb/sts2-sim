//! The mechanical half of PORT_PLAN.md D2/D3/§7: a source-reading contract
//! test over the hot-path modules.
//!
//! It is crude on purpose. The v0.110.1 kernel accumulated ~69 sites where
//! dispatch compared content text below the canonical boundary, and no design
//! document stopped it — a grep did not exist. This is that grep, run as a
//! test, on every pull request, so the regression has to be argued for in a
//! diff rather than arriving one call site at a time.
//!
//! What it pins:
//!
//! * the four hot-path modules contain no owned text type, no text equality,
//!   and no per-node boxed chain or map;
//! * `boundary.rs` may own text — it *is* the canonical side, and building a
//!   string-keyed document is its job — but it still may not dispatch on text
//!   equality.
//!
//! What it deliberately does not try to be: a parser. A rule this test cannot
//! express belongs in review, not in a cleverer regex.

/// The modules below the canonical boundary. Nothing here may name an owned
/// text type or compare text.
///
/// The engine core joins the layout modules here at R0.5: it is the code that
/// actually runs per search node, so it is the code the kernel's regression
/// would have shown up in first.
const HOT_MODULES: [(&str, &str); 10] = [
    ("src/hot.rs", include_str!("../src/hot.rs")),
    ("src/catalog.rs", include_str!("../src/catalog.rs")),
    ("src/powers.rs", include_str!("../src/powers.rs")),
    ("src/frame.rs", include_str!("../src/frame.rs")),
    ("src/hooks.rs", include_str!("../src/hooks.rs")),
    ("src/engine/mod.rs", include_str!("../src/engine/mod.rs")),
    (
        "src/engine/damage.rs",
        include_str!("../src/engine/damage.rs"),
    ),
    ("src/engine/play.rs", include_str!("../src/engine/play.rs")),
    ("src/engine/draw.rs", include_str!("../src/engine/draw.rs")),
    ("src/engine/turn.rs", include_str!("../src/engine/turn.rs")),
];

#[test]
fn public_action_replay_dispatch_stays_outlined() {
    let engine = HOT_MODULES
        .iter()
        .find_map(|(path, source)| (*path == "src/engine/mod.rs").then_some(*source))
        .expect("engine/mod.rs is a hot module");
    assert!(engine.contains("#[inline(never)]\npub fn apply_action_into("));
}

/// The generated dispatch trees (#1290). They are read from disk rather than
/// `include_str!`-ed one by one, because their whole point is that a wave PR
/// adds bodies to a family file without touching anything shared — including
/// this test. A hand-listed set here would be the shared file the D4 layout
/// exists to eliminate.
fn dispatch_sources() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    for tree in ["src/steps", "src/moves"] {
        let dir = root.join(tree);
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|error| panic!("{} is unreadable: {error}", dir.display()))
            .map(|entry| entry.expect("a readable directory entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
            .collect();
        entries.sort();
        for path in entries {
            let name = format!("{tree}/{}", path.file_name().unwrap().to_string_lossy());
            out.push((
                name,
                std::fs::read_to_string(&path).expect("a readable module"),
            ));
        }
    }
    out
}

/// Every Rust source below `src/`, sorted by its crate-relative path.
///
/// A whole-tree census keeps a new engine or move module from silently
/// becoming a second home for a StepCtx-owned powered-card attack body.
fn all_rust_sources() -> Vec<(String, String)> {
    fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("{} is unreadable: {error}", dir.display()))
            .map(|entry| entry.expect("a readable directory entry").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let name = path
                    .strip_prefix(root)
                    .expect("a source path below the crate root")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((
                    name,
                    std::fs::read_to_string(&path).expect("a readable Rust module"),
                ));
            }
        }
    }

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    walk(root, &root.join("src"), &mut out);
    out
}

/// Ignore inline unit-test modules when mechanically classifying executable
/// production sites. Family files use one final `mod tests {` section; this
/// established cut keeps test-only seam calls from inflating a live census.
/// Early `#[cfg(test)]` seam helpers deliberately remain visible to the scan:
/// the Monologue census below names its one raw hook-forgery helper explicitly,
/// so a new pre-module test mutator cannot become a blind spot.
fn production_source(source: &str) -> &str {
    source
        .split_once("\nmod tests {")
        .map_or(source, |(production, _)| production)
}

const BOUNDARY: (&str, &str) = ("src/boundary.rs", include_str!("../src/boundary.rs"));

/// The admission gate runs **once per fight**, beside the catalog builder and
/// off every per-node path, so it is allowed the ordered set it needs to
/// report a complete missing list. It is pinned separately rather than
/// exempted silently: the assertion below is that it stays boundary-time
/// work, not that its containers are invisible.
const ADMISSION: (&str, &str) = (
    "src/engine/admission.rs",
    include_str!("../src/engine/admission.rs"),
);

/// Owned, growable text. Interned ids are the hot representation of every
/// content axis (D2), so a hot module never needs one.
const OWNED_TEXT: &str = "String";

/// Text equality, in the two shapes the kernel actually regressed into:
/// a literal comparison, and a comparison against an id-ish field.
///
/// `.id ==` is deliberately conservative — it also catches an *interned*
/// comparison written that way. That is a feature: the fix is to destructure
/// the id out of its record, which reads better and cannot later be mistaken
/// for the text form it is banned for resembling.
const TEXT_EQUALITY: [&str; 2] = ["== \"", ".id =="];

/// Per-node indirection the layout rules ban outright.
const BANNED_CONTAINERS: [&str; 3] = ["BTreeMap", "HashMap", "Box<"];

/// `catalog.rs`'s interning map is the one sanctioned exception: it lives in
/// `CatalogBuilder`, which runs once per fight at the boundary and is gone by
/// the time the read-only `Catalog` exists. The allowance is per-module and
/// bounded, so a second map cannot slip in beside it.
const CATALOG_BUILDER_MAP_SITES: usize = 2;

fn occurrences(source: &str, needle: &str) -> usize {
    source.matches(needle).count()
}

fn production_call_sites(needle: &str) -> Vec<String> {
    let mut sites: Vec<_> = all_rust_sources()
        .into_iter()
        .flat_map(|(path, source)| {
            production_source(&source)
                .lines()
                .filter(|line| line.contains(needle) && !line.trim_start().starts_with("//"))
                .map(move |line| format!("{path}:{}", line.trim()))
                .collect::<Vec<_>>()
        })
        .collect();
    sites.sort();
    sites
}

fn assert_production_call_sites(needle: &str, expected: &[&str]) {
    let mut expected: Vec<_> = expected.iter().map(|site| (*site).to_owned()).collect();
    expected.sort();
    assert_eq!(
        production_call_sites(needle),
        expected,
        "{needle} call sites"
    );
}

#[test]
fn ordinary_step_runner_has_no_nested_pending_poll() {
    let source = include_str!("../src/engine/play.rs");
    assert!(
        source.contains("(ordinary, $ctx:ident) => {};")
            && source
                .contains("define_run_steps_with_precommit!(run_steps_with_precommit, ordinary);"),
        "the ordinary step runner must keep the pre-R27 branch-free post-dispatch shape"
    );
    assert!(
        source.contains("(persisted, $ctx:ident) => {")
            && source.contains("if $ctx.state.pending.is_some() {")
            && source.contains("return Ok(RunStepsOutcome::NestedBlocked);"),
        "only the cold persisted runner polls for a nested parked child"
    );
}

/// #2693 S1 — monster `PowerId::Strength` has exactly ONE writer.
///
/// The ordered attachment ledger records a monster's `StrengthPower`
/// position and applier, and it can only stay in step with the scalar if
/// every writer goes through `engine::damage::write_monster_strength`. A raw
/// `.set(PowerId::Strength, ...)` on a monster would move the amount while
/// leaving the row behind — silently, and only observably once a `Misery`
/// read the ledger, which is the hardest class of defect to find later.
///
/// The candidate set is DERIVED: `production_call_sites` walks every Rust
/// source below `src/` and cuts each at its final `mod tests {`, so a new
/// family file is covered by existing. What is written down is the
/// *expectation*, which is what makes this a change-detector — a new site has
/// to be argued for in a diff.
///
/// The three surviving sites outside the helper are all **player** Strength,
/// which #2693 S1 leaves alone by scope: Possess steals from the player
/// (`<PossessMove>`), `apply_signed_player_stat` is the player's own signed
/// stat writer, and the Ruined Helmet arm publishes the permanent remainder.
#[test]
fn monster_strength_writer_census_is_exact() {
    assert_production_call_sites(
        ".set(PowerId::Strength",
        &[
            // The one writer. Its body is the only place a monster's scalar
            // and its ledger row move together.
            "src/engine/damage.rs:.set(PowerId::Strength, SlotWire::Int, updated);",
            // Player-side, out of scope.
            "src/engine/damage.rs:state.powers.set(PowerId::Strength, SlotWire::Int, updated);",
            "src/engine/damage.rs:.set(PowerId::Strength, SlotWire::Int, permanent);",
            "src/moves/normal.rs:.set(PowerId::Strength, SlotWire::Int, player_strength);",
        ],
    );

    // The helper is where that one site lives, and nothing else may create,
    // restack or remove a ledger row for this family.
    assert_production_call_sites(
        "write_monster_strength(",
        &[
            "src/engine/damage.rs:pub(crate) fn write_monster_strength(",
            // The clone command, the delta publisher, and the Suck/Ravenous
            // wrapped calls, each spelled across several lines.
            "src/engine/damage.rs:write_monster_strength(",
            "src/engine/damage.rs:write_monster_strength(",
            // #3338/#3342: a fresh temporary-Strength wrapper (any producer)
            // on a retained-death owner under Sleight lands its nested
            // Strength (`<BeforeApplied>d__20` `0x348d20` IL_001d-IL_004b)
            // before the wrapper attaches (`0x3efbac` IL_0360), so the two
            // halves are split there.
            "src/engine/damage.rs:write_monster_strength(&mut state.monsters_mut()[target], strength, applier, upkeep);",
            "src/engine/damage.rs:write_monster_strength(&mut state.monsters_mut()[target], updated, applier, upkeep);",
            // Plow and the segment revive: the exactly-zero removal edge.
            "src/engine/damage.rs:write_monster_strength(monster, 0, crate::hot::Applier::None, upkeep);",
            "src/engine/damage.rs:write_monster_strength(monster, 0, crate::hot::Applier::None, upkeep);",
            // #2693 S2 folded the temporary-Strength wrapper lifecycle into
            // three writers of its own, and the nested `Apply<StrengthPower>`
            // each of them performs now lives inside those bodies rather than
            // at their callers. `write_monster_temp_strength_wrapper`'s
            // application carries the wrapper's own applier
            // (`<BeforeApplied>d__20::MoveNext` `0x348d20` IL_003e-IL_004b)...
            "src/engine/damage.rs:write_monster_strength(monster, strength, applier, upkeep);",
            // ...and `unwind_monster_temp_strength_wrappers` restores with the
            // OWNER (`<AfterSideTurnEnd>d__22::MoveNext` `0x348ba8`
            // IL_009d-IL_00c4), once per live wrapper in ledger order, or once
            // in aggregate where provenance was never recorded.
            "src/engine/damage.rs:write_monster_strength(monster, next, owner, upkeep);",
            "src/engine/damage.rs:write_monster_strength(monster, restored, owner, upkeep);",
            "src/engine/damage.rs:write_monster_strength(monster, restored, owner, upkeep);",
            // The self-buff helper's single delegation.
            "src/engine/damage.rs:write_monster_strength(monster, updated, owner, upkeep);",
            // `CrabRagePower/<AfterDeath>d__8::MoveNext` `0x337f54`
            // IL_0083-IL_008b applies its Strength 6 to the surviving sibling
            // with that sibling as the applier (#2654).
            "src/engine/monsters.rs:crate::engine::damage::write_monster_strength(",
            "src/engine/monsters.rs:super::damage::write_monster_strength(monster, strength, crate::hot::Applier::None, upkeep);",
            "src/engine/play.rs:super::damage::write_monster_strength(owner, strength, applier, upkeep);",
            "src/engine/turn.rs:super::damage::write_monster_strength(monster, strength, owner, upkeep);",
            "src/engine/turn.rs:super::damage::write_monster_strength(monster, strength, owner, upkeep);",
            "src/engine/turn.rs:super::damage::write_monster_strength(monster, strength, owner, upkeep);",
            "src/moves/boss.rs:crate::engine::damage::write_monster_strength(",
            "src/moves/spawned.rs:crate::engine::damage::write_monster_strength(",
            "src/steps/templates.rs:crate::engine::damage::write_monster_strength(",
        ],
    );
    // ...and the enemy self-buff convenience, which delegates rather than
    // writing. Every monster move that raises its own Strength is here.
    assert_eq!(
        production_call_sites("write_monster_self_strength(").len(),
        16,
        "the enemy self-buff writer census moved: {:#?}",
        production_call_sites("write_monster_self_strength("),
    );

    // The ledger mutators themselves: only the helper and the two clone
    // commands may touch an attachment row.
    for needle in [
        "push_attachment(",
        "set_attachment_amount(",
        "remove_attachment(",
    ] {
        for site in production_call_sites(needle) {
            assert!(
                site.starts_with("src/hot.rs:")
                    || site.starts_with("src/boundary.rs:")
                    || site.starts_with("src/engine/damage.rs:"),
                "{site} mutates the attachment ledger outside hot/boundary/damage",
            );
        }
    }
}

/// #2693 S2 — monster `PowerId::TempStrength` has exactly THREE writers, and
/// they are the only places a temporary-Strength wrapper row is born, stacked
/// or removed.
///
/// The same argument as the Strength census above, one family along. The
/// aggregate scalar and the concrete wrapper rows have to agree — admission
/// refuses a monster whose rows do not sum to it — and they can only agree if
/// no site moves one without the other. A raw
/// `.set(PowerId::TempStrength, …)` on a monster is exactly that defect, and
/// it would be invisible until a `Misery` read the ledger.
///
/// Three rather than one because the native lifecycle has three edges that
/// cannot share a body: an application
/// (`PowerCmd/<Apply>d__2::MoveNext` `0x3efbac`), the side-end restoration
/// that removes each wrapper and hands its amount back to Strength
/// (`TemporaryStrengthPower/<AfterSideTurnEnd>d__22::MoveNext` `0x348ba8`),
/// and the two whole-instance wipes that restore nothing (`PlowPower`'s
/// threshold and the segment revive). The expectation below is written down;
/// the search over `src/` is derived.
#[test]
fn monster_temp_strength_writer_census_is_exact() {
    assert_production_call_sites(
        ".set(PowerId::TempStrength",
        &[
            // `write_monster_temp_strength_wrapper` — one application.
            "src/engine/damage.rs:.set(PowerId::TempStrength, SlotWire::Int, temp);",
            // #3338/#3342 `temp_strength_wrapper_on_retained_owner` — the same
            // application's wrapper half, attached after its nested Strength
            // and any Sleight death (`0x3efbac` IL_0336-IL_0360), for every
            // producer. A known restack returns to the caller's shared writer.
            "src/engine/damage.rs:.set(PowerId::TempStrength, SlotWire::Int, temp);",
            // `unwind_monster_temp_strength_wrappers` — the side-end lifecycle.
            "src/engine/damage.rs:monster.powers.set(PowerId::TempStrength, SlotWire::Int, 0);",
            // `clear_monster_temp_strength_wrappers` — the whole-instance wipes.
            "src/engine/damage.rs:monster.powers.set(PowerId::TempStrength, SlotWire::Int, 0);",
        ],
    );

    // Every production caller of the three, so a fourth lifecycle edge has to
    // be argued for in a diff rather than appearing.
    assert_production_call_sites(
        "write_monster_temp_strength_wrapper(",
        &[
            "src/engine/damage.rs:pub(crate) fn write_monster_temp_strength_wrapper(",
            // The card/potion command seam, Monarch's Gaze's listener arm, and
            // Shackling Potion's null-card application.
            "src/engine/damage.rs:let (temporary, strength) = write_monster_temp_strength_wrapper(",
            "src/engine/damage.rs:let (temp, strength) = write_monster_temp_strength_wrapper(",
            "src/engine/damage.rs:let (temporary, strength) = write_monster_temp_strength_wrapper(",
        ],
    );
    assert_production_call_sites(
        "unwind_monster_temp_strength_wrappers(",
        &[
            "src/engine/damage.rs:pub(crate) fn unwind_monster_temp_strength_wrappers(",
            // The two death walks.
            "src/engine/damage.rs:unwind_monster_temp_strength_wrappers(monster, upkeep)?;",
            "src/engine/damage.rs:unwind_monster_temp_strength_wrappers(monster, upkeep)?;",
            // Enemy side end.
            "src/engine/turn.rs:super::damage::unwind_monster_temp_strength_wrappers(monster, upkeep)?;",
        ],
    );
    assert_production_call_sites(
        "clear_monster_temp_strength_wrappers(",
        &[
            "src/engine/damage.rs:pub(crate) fn clear_monster_temp_strength_wrappers(monster: &mut HotMonster, upkeep: bool) {",
            // Plow's threshold crossing and the segment revive.
            "src/engine/damage.rs:clear_monster_temp_strength_wrappers(monster, upkeep);",
            "src/engine/damage.rs:clear_monster_temp_strength_wrappers(monster, upkeep);",
        ],
    );

    // Every card-sourced application names its concrete native class through
    // the one derivation, rather than any caller picking a model inline.
    assert_production_call_sites(
        "apply_temp_strength_enemy(",
        &[
            "src/steps/shared.rs:pub(crate) fn apply_temp_strength_enemy(",
            "src/steps/shared.rs:PowerId::TempStrengthEnemy => apply_temp_strength_enemy(",
            "src/steps/shared.rs:apply_temp_strength_enemy(ctx.state, target, model, amount, ctx.events)?;",
            "src/steps/templates.rs:crate::steps::shared::apply_temp_strength_enemy(ctx.state, target, model, amount, ctx.events)",
        ],
    );
    assert_production_call_sites(
        "temp_strength_enemy_model(",
        &[
            "src/steps/shared.rs:pub(crate) fn temp_strength_enemy_model(",
            "src/steps/shared.rs:let model = temp_strength_enemy_model(ctx.spec.identity.id)?;",
            "src/steps/shared.rs:temp_strength_enemy_model(ctx.spec.identity.id)?,",
            "src/steps/templates.rs:let model = crate::steps::shared::temp_strength_enemy_model(ctx.spec.identity.id)?;",
        ],
    );
}

/// #2693 S4b — the producer surface the admission rule
/// `"unplaceable entering Strength beside a reachable reducer"` rests on.
///
/// That rule admits a root whose entering Strength has no placeable position
/// **because nothing reachable can drive it negative**, and a `Misery` only
/// selects a `StrengthPower` below zero. It is therefore an ABSENCE claim on a
/// shared admission surface — the #1432
/// *new-reader-invalidates-old-shortcut* class — and file-level disjointness
/// cannot see it: a new card, a new monster move or a new wrapper call site
/// that lowers a monster's Strength would make the predicate wrong in the
/// direction that **silently under-refuses**, and the late refusal #2693
/// closed would come back inside an admitted root.
///
/// So the candidate set here is DERIVED three ways and only the expectation is
/// written down:
///
/// 1. the five seams that can move a monster's Strength DOWN, walked over
///    every Rust source below `src/`;
/// 2. the `src/moves/**` writers, which is where the "no monster move lowers a
///    monster's Strength" half lives;
/// 3. the content rows, so a new card row using an existing lowering step kind
///    is visible even though the predicate would already classify it.
#[test]
fn monster_strength_lowering_producer_census_is_exact() {
    // (1) The seams. `write_monster_strength` itself is a SIGNED writer and is
    // censused above; these five are the ones that can only go down, and the
    // admission predicate's step-kind arms are exactly their dispatch owners.
    assert_production_call_sites(
        "apply_card_monster_strength_delta(",
        &[
            "src/engine/damage.rs:pub fn apply_card_monster_strength_delta(",
            // Shared Fate's exact pair (`0x3baaec` IL_015a-IL_0186).
            "src/steps/necrobinder_rare.rs:apply_card_monster_strength_delta(ctx.state, target, enemy, ctx.events)",
            // `power_all_serial`'s `StrengthEnemy` arm — Resonance.
            "src/steps/shared.rs:apply_card_monster_strength_delta(ctx.state, target, amount, ctx.events)?",
            // Malaise's X body (`0x3ab428` IL_00f6's `neg`).
            "src/steps/silent_rare.rs:apply_card_monster_strength_delta(ctx.state, target, strength_delta, ctx.events)?;",
        ],
    );
    assert_production_call_sites(
        "apply_temp_strength_enemy(",
        &[
            "src/steps/shared.rs:pub(crate) fn apply_temp_strength_enemy(",
            "src/steps/shared.rs:PowerId::TempStrengthEnemy => apply_temp_strength_enemy(",
            "src/steps/shared.rs:apply_temp_strength_enemy(ctx.state, target, model, amount, ctx.events)?;",
            "src/steps/templates.rs:crate::steps::shared::apply_temp_strength_enemy(ctx.state, target, model, amount, ctx.events)",
        ],
    );

    // (2) The roster surface. Every `src/moves/**` writer reaches the scalar
    // through `checked_monster_strength_successor`, so the sign lives in the
    // compiled row; what this pins is the SET of move kinds whose body writes
    // a monster's Strength at all, which is what
    // `admission::MOVE_KINDS_WRITING_MONSTER_STRENGTH` enumerates. A new one
    // appearing without a decision is the under-refusal.
    assert_eq!(
        move_kinds_writing_monster_strength(),
        [
            "AeonglassIntensity",
            "AttackSteal",
            "AttackStrength",
            "AttackStrengthBranch",
            "AxebotBootup",
            "BlockStrength",
            "BuffStrength",
            "BuffTeamStrength",
            "EntoSpit",
            "LouseCurlGrow",
            "Ponder",
            "PossessStrength",
            "QueenBurnBright",
            "SoulSiphon",
            "TestSubjectBurningGrowl",
            "WeakPlayerStrength",
            "Wriggle",
        ],
        "a monster move gained or lost a Strength writer; \
         `admission::MOVE_KINDS_WRITING_MONSTER_STRENGTH` and \
         `move_lowers_monster_strength` must be re-derived, or the #2693 S4b \
         admission gate silently under-refuses (#1432)",
    );

    // Nothing under `src/moves/**` may reach a DOWN-only seam at all: the
    // wrapper family is card- and potion-sourced, and a monster move that
    // acquired one would be a negative-Strength producer this predicate does
    // not know about.
    for needle in [
        "apply_card_monster_strength_delta(",
        "apply_temp_strength_enemy(",
        "write_monster_temp_strength_wrapper(",
    ] {
        for site in production_call_sites(needle) {
            assert!(
                !site.starts_with("src/moves/"),
                "{site}: a monster move reaches a monster-Strength REDUCER; \
                 the #2693 S4b producer predicate counts none, so it would \
                 under-refuse (#1432)",
            );
        }
    }

    // (3) The content rows. Exactly ten card identities carry a step kind the
    // predicate classifies as lowering, and exactly one move row of a
    // Strength-writing kind carries a negative integer at all.
    let mut lowering_cards = content_card_ids_with_step_kinds(&[
        "MalaiseX",
        "SharedFateExact",
        "TempStrengthEnemy",
        "AttackAllTempStrengthSnapshot",
        "MonarchsGaze",
    ]);
    // `PowerAllSerial` is sign-carrying, so only the rows whose power argument
    // is a monster-Strength reducer count.
    lowering_cards.extend(content_card_ids_with_power_all_serial_reducers());
    lowering_cards.sort();
    lowering_cards.dedup();
    assert_eq!(
        lowering_cards,
        [
            "CrushUnder",
            "DarkShackles",
            "DyingStar",
            "EnfeeblingTouch",
            "Malaise",
            "Mangle",
            "MonarchsGaze",
            "PiercingWail",
            "Resonance",
            "SharedFate",
        ],
        "the card producer set moved; re-derive \
         `admission::card_step_lowers_monster_strength` (#2693 S4b, #1432)",
    );
    assert_eq!(
        content_move_rows_with_a_negative_argument(&[
            "AeonglassIntensity",
            "AttackSteal",
            "AttackStrength",
            "AttackStrengthBranch",
            "AxebotBootup",
            "BlockStrength",
            "BuffTeamStrength",
            "LouseCurlGrow",
            "Ponder",
            "PossessStrength",
            "SoulSiphon",
            "TestSubjectBurningGrowl",
            "WeakPlayerStrength",
            "Wriggle",
        ]),
        ["SOUL_SIPHON_MOVE"],
        "a monster move row of a Strength-writing kind gained a negative \
         argument; `admission::move_lowers_monster_strength` excludes only \
         SOUL_SIPHON_MOVE, whose negatives are the PLAYER's Strength and \
         Dexterity (`0x3615f4` IL_00db), so a new one must be read before the \
         #2693 S4b gate can keep admitting past it (#1432)",
    );
}

/// The `MoveKind` names whose `moves::dispatch` arm reaches a monster-Strength
/// writer, sorted.
///
/// Derived in two steps, both from the tree: a least-fixed-point over the
/// top-level functions in `src/moves/**` that reach
/// `damage::write_monster_strength` / `write_monster_self_strength`, and then
/// the `MoveKind::X => …` arms of `src/moves/mod.rs` that call one of them.
/// The dispatch tree is closed inside `src/moves`, which is what makes the
/// fixed point exact rather than an approximation.
fn move_kinds_writing_monster_strength() -> Vec<String> {
    let mut bodies: Vec<(String, String)> = Vec::new();
    for (path, source) in all_rust_sources() {
        if !path.starts_with("src/moves/") {
            continue;
        }
        let mut current: Option<String> = None;
        let mut buffer = String::new();
        for line in production_source(&source).lines() {
            let head = line
                .strip_prefix("pub(crate) fn ")
                .or_else(|| line.strip_prefix("pub fn "))
                .or_else(|| line.strip_prefix("fn "));
            if let Some(rest) = head {
                if let Some(name) = current.take() {
                    bodies.push((name, std::mem::take(&mut buffer)));
                }
                current = rest.split(['(', '<']).next().map(str::to_owned);
            }
            if current.is_some() {
                buffer.push_str(line);
                buffer.push('\n');
            }
        }
        if let Some(name) = current {
            bodies.push((name, buffer));
        }
    }

    let mut reaching: Vec<String> = bodies
        .iter()
        .filter(|(_, body)| {
            body.contains("write_monster_self_strength(")
                || body.contains("write_monster_strength(")
        })
        .map(|(name, _)| name.clone())
        .collect();
    loop {
        let grown: Vec<String> = bodies
            .iter()
            .filter(|(name, body)| {
                !reaching.contains(name)
                    && reaching
                        .iter()
                        .any(|callee| body.contains(&format!("{callee}(")))
            })
            .map(|(name, _)| name.clone())
            .collect();
        if grown.is_empty() {
            break;
        }
        reaching.extend(grown);
    }

    let dispatch = include_str!("../src/moves/mod.rs");
    let mut kinds: Vec<String> = Vec::new();
    for line in dispatch.lines() {
        let Some(rest) = line.trim_start().strip_prefix("MoveKind::") else {
            continue;
        };
        let Some((kind, tail)) = rest.split_once(" => ") else {
            continue;
        };
        if reaching
            .iter()
            .any(|callee| tail.contains(&format!("{callee}(")))
        {
            kinds.push(kind.to_owned());
        }
    }
    kinds.sort();
    kinds.dedup();
    kinds
}

/// The `CardId` a generated `CardRow` declares, for the row-opening line.
///
/// Anchored on `id: CardId::` so a pool list or a `play_condition` naming a
/// card cannot be mistaken for the row's own identity.
fn content_row_card_id(line: &str) -> Option<String> {
    line.trim_start()
        .strip_prefix("id: CardId::")?
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .next()
        .map(str::to_owned)
}

/// Card ids whose generated rows contain any of `kinds`, derived from
/// `content_tables.rs`.
fn content_card_ids_with_step_kinds(kinds: &[&str]) -> Vec<String> {
    let source = include_str!("../src/content_tables.rs");
    let mut current: Option<String> = None;
    let mut out = Vec::new();
    for line in source.lines() {
        if let Some(card) = content_row_card_id(line) {
            current = Some(card);
        }
        for kind in kinds {
            if line.contains(&format!("StepKind::{kind},"))
                && let Some(card) = current.clone()
            {
                out.push(card);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Card ids whose generated rows contain a `PowerAllSerial` step whose power
/// argument is a monster-Strength reducer: `TempStrengthEnemy` at any amount,
/// or `StrengthEnemy` at a negative one.
fn content_card_ids_with_power_all_serial_reducers() -> Vec<String> {
    let source = include_str!("../src/content_tables.rs");
    let mut current: Option<String> = None;
    let mut out = Vec::new();
    for line in source.lines() {
        if let Some(card) = content_row_card_id(line) {
            current = Some(card);
        }
        if !line.contains("StepKind::PowerAllSerial,") {
            continue;
        }
        let reducer = line.contains("Arg::Power(PowerId::TempStrengthEnemy)")
            || (line.contains("Arg::Power(PowerId::StrengthEnemy)") && line.contains("Arg::I(-"));
        if reducer && let Some(card) = current.clone() {
            out.push(card);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The `name` of every generated move row whose kind is in `kinds` and which
/// carries a negative integer argument.
fn content_move_rows_with_a_negative_argument(kinds: &[&str]) -> Vec<String> {
    let source = include_str!("../src/content_tables.rs");
    let mut out = Vec::new();
    for line in source.lines() {
        let Some(rest) = line.split("Move { name: \"").nth(1) else {
            continue;
        };
        let Some((name, tail)) = rest.split_once('"') else {
            continue;
        };
        if !kinds
            .iter()
            .any(|kind| tail.contains(&format!("MoveKind::{kind},")))
        {
            continue;
        }
        if tail.contains("Arg::I(-") {
            out.push(name.to_owned());
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn monologue_production_writer_call_site_census_is_exact() {
    // Public dispatch has exactly one caller of the exact foundation. Keeping
    // both sites module-local prevents another family from registering
    // Monologue behind the post-body source guard.
    assert_production_call_sites(
        "monologue_foundation_exact(",
        &[
            "src/steps/regent_uncommon.rs:monologue_foundation_exact(ctx)",
            "src/steps/regent_uncommon.rs:fn monologue_foundation_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {",
        ],
    );

    // Every explicit reference is pinned, including readers, generated ids,
    // writer events and allowlists. That makes adding Monologue to an existing
    // generic `power` writer visible even if its setter call site is unchanged.
    // Repeated turn.rs rows are distinct native-object paths: attached and
    // detached callback carriers both perform the same removal/event writes.
    assert_production_call_sites(
        "PowerId::Monologue",
        &[
            "src/boundary.rs:let monologue_live = state.powers.value(PowerId::Monologue) > 0",
            "src/boundary.rs:let monologue_amount = state.powers.value(PowerId::Monologue);",
            "src/engine/admission.rs:PowerId::Monologue,",
            "src/engine/admission.rs:| PowerId::Monologue",
            "src/engine/turn.rs:.set(PowerId::Monologue, SlotWire::Int, remaining);",
            "src/engine/turn.rs:.set(PowerId::Monologue, SlotWire::Int, remaining);",
            "src/engine/turn.rs:if state.powers.value(PowerId::Monologue) > 0",
            "src/engine/turn.rs:if state.powers.value(PowerId::Monologue) <= 0 {",
            "src/engine/turn.rs:let mut remaining = state.powers.value(PowerId::Monologue);",
            "src/engine/turn.rs:.value(PowerId::Monologue)",
            "src/engine/turn.rs:super::damage::note_power(events, Subject::Player, PowerId::Monologue, 0);",
            "src/engine/turn.rs:super::damage::note_power(events, Subject::Player, PowerId::Monologue, 0);",
            "src/ids.rs:PowerId::Monologue, PowerId::Mplating, PowerId::NecroMastery, PowerId::Nemesis,",
            "src/steps/regent_uncommon.rs:crate::engine::damage::note_power(events, Subject::Player, PowerId::Monologue, 1);",
            "src/steps/regent_uncommon.rs:crate::engine::damage::note_power(ctx.events, Subject::Player, PowerId::Monologue, 1);",
            "src/steps/regent_uncommon.rs:let amount = match state.powers.get(PowerId::Monologue) {",
            "src/steps/regent_uncommon.rs:state.powers.get(PowerId::Monologue).is_some()",
            "src/steps/regent_uncommon.rs:let prior = state.powers.value(PowerId::Monologue);",
            "src/steps/regent_uncommon.rs:state.powers.set(PowerId::Monologue, SlotWire::Int, updated);",
        ],
    );

    // Amount may enter through the canonical boundary, be raised only by the
    // exact public foundation, and be removed only at side end. The broad
    // assignment needle prevents another whole-Slots replacement from hiding
    // behind a different constructor spelling.
    assert_production_call_sites(
        ".powers =",
        &[
            "src/boundary.rs:monster.powers = Slots::from_unsorted(powers).expect(\"canonical keys are unique\");",
            "src/boundary.rs:state.powers = Slots::from_unsorted(powers).expect(\"canonical keys are unique\");",
            "src/engine/damage.rs:monster.powers = Slots::new();",
            "src/engine/damage.rs:monster.powers = Slots::new();",
            "src/engine/monsters.rs:egg.powers = Slots::new();",
        ],
    );
    assert_production_call_sites(".fanouts =", &[]);
    assert_production_call_sites(
        "PowerId::Monologue, SlotWire::Int",
        &[
            "src/engine/turn.rs:.set(PowerId::Monologue, SlotWire::Int, remaining);",
            "src/engine/turn.rs:.set(PowerId::Monologue, SlotWire::Int, remaining);",
            "src/steps/regent_uncommon.rs:state.powers.set(PowerId::Monologue, SlotWire::Int, updated);",
        ],
    );
    assert_production_call_sites(
        "state.powers.set(power,",
        &[
            "src/engine/damage.rs:state.powers.set(power, SlotWire::Int, 0);",
            // #3159: the shared BlockNextTurn / SelfFormingClayPower apply.
            "src/engine/damage.rs:state.powers.set(power, SlotWire::Int, stacked);",
            "src/engine/damage.rs:state.powers.set(power, SlotWire::Int, updated);",
            "src/engine/damage.rs:state.powers.set(power, SlotWire::Int, updated);",
            "src/engine/damage.rs:state.powers.set(power, SlotWire::Int, updated);",
            "src/engine/play.rs:state.powers.set(power, SlotWire::Int, amount - 1);",
            "src/engine/relics.rs:state.powers.set(power, SlotWire::Int, updated);",
            "src/engine/turn.rs:state.powers.set(power, SlotWire::Int, 0);",
            "src/engine/turn.rs:state.powers.set(power, SlotWire::Int, 0);",
            // #3159: the shared delayed-Block AfterBlockCleared removal.
            "src/engine/turn.rs:state.powers.set(power, SlotWire::Int, 0);",
            "src/engine/turn.rs:state.powers.set(power, SlotWire::Int, amount - 1);",
            "src/engine/turn.rs:state.powers.set(power, SlotWire::Int, amount);",
            "src/engine/turn.rs:state.powers.set(power, SlotWire::Int, amount);",
            "src/engine/turn.rs:state.powers.set(power, SlotWire::Int, amount);",
            "src/engine/turn.rs:state.powers.set(power, SlotWire::Int, amount);",
            "src/engine/turn.rs:state.powers.set(power, SlotWire::Int, updated);",
            "src/engine/turn.rs:state.powers.set(power, wire, 0);",
            "src/steps/defect_orb.rs:ctx.state.powers.set(power, SlotWire::Int, updated);",
            "src/steps/ironclad_rare.rs:ctx.state.powers.set(power, SlotWire::Int, value);",
            "src/steps/ironclad_uncommon.rs:ctx.state.powers.set(power, SlotWire::Int, value);",
            "src/steps/neutral.rs:ctx.state.powers.set(power, SlotWire::Int, updated);",
            "src/steps/shared.rs:ctx.state.powers.set(power, SlotWire::Int, updated);",
            "src/steps/templates.rs:ctx.state.powers.set(power, SlotWire::Int, updated);",
            "src/steps/templates.rs:ctx.state.powers.set(power, SlotWire::Int, updated);",
        ],
    );

    // Boundary hydration remains the only compatibility hook registrar.
    // Runtime row allocation/removal maintains this aggregate mirror inside
    // HotFanouts. Bare method-name censuses also catch UFCS and function-
    // pointer uses, not just dotted calls.
    assert_production_call_sites(
        "register_monologue_hooks",
        &[
            "src/boundary.rs:if monologue_amount > 0 && !state.fanouts.register_monologue_hooks() {",
            "src/hot.rs:pub(crate) fn register_monologue_hooks(&mut self) -> bool {",
            "src/hot.rs:pub(crate) fn unregister_monologue_hooks(&mut self) {",
        ],
    );
    assert_production_call_sites(
        "unregister_monologue_hooks",
        &["src/hot.rs:pub(crate) fn unregister_monologue_hooks(&mut self) {"],
    );

    // Boundary hydration, the exact AfterCardPlayed callback, and side-end
    // cleanup are the complete ledger mutation census. The raw compact fields
    // remain writable only inside these two HotFanouts methods.
    assert_production_call_sites(
        "set_monologue_strength_applied",
        &[
            "src/boundary.rs:if !state.fanouts.set_monologue_strength_applied(applied) {",
            "src/hot.rs:pub(crate) fn set_monologue_strength_applied(&mut self, value: i32) -> bool {",
        ],
    );
    assert_production_call_sites(
        "monologue_strength_applied =",
        &[
            "src/hot.rs:Arc::make_mut(&mut self.0).monologue_strength_applied = value;",
            "src/hot.rs:state.monologue_strength_applied = applied;",
            "src/hot.rs:state.monologue_strength_applied = next_aggregate_applied;",
            "src/hot.rs:state.monologue_strength_applied = next_applied;",
        ],
    );
    assert_production_call_sites(
        "private_power_flags |= MONOLOGUE_HOOK_MASK",
        &[
            "src/hot.rs:Arc::make_mut(&mut self.0).private_power_flags |= MONOLOGUE_HOOK_MASK;",
            "src/hot.rs:state.private_power_flags |= MONOLOGUE_HOOK_MASK;",
            "src/hot.rs:state.private_power_flags |= MONOLOGUE_HOOK_MASK;",
        ],
    );
    assert_production_call_sites(
        "private_power_flags &= !MONOLOGUE_HOOK_MASK",
        &[
            "src/hot.rs:Arc::make_mut(&mut self.0).private_power_flags &= !MONOLOGUE_HOOK_MASK;",
            "src/hot.rs:state.private_power_flags &= !MONOLOGUE_HOOK_MASK;",
            "src/hot.rs:state.private_power_flags &= !MONOLOGUE_HOOK_MASK;",
        ],
    );
    assert_production_call_sites(
        "private_power_flags =",
        &["src/hot.rs:state.private_power_flags ="],
    );
    assert_production_call_sites(
        "forge_monologue_hook_flags_for_test",
        &["src/hot.rs:pub(crate) fn forge_monologue_hook_flags_for_test(&mut self, flags: u8) {"],
    );
}

#[test]
fn constrict_player_carrier_and_catalog_census_is_exact() {
    assert_production_call_sites(
        "self.steam_eruption_damage",
        &[
            "src/hot.rs:self.steam_eruption_damage",
            "src/hot.rs:self.steam_eruption_damage",
            "src/hot.rs:self.steam_eruption_damage",
            "src/hot.rs:&& self.steam_eruption_damage >= 0",
            "src/hot.rs:&& self.steam_eruption_damage >= 0",
            "src/hot.rs:&& self.steam_eruption_damage == 0",
            "src/hot.rs:self.steam_eruption_damage = damage;",
            "src/hot.rs:self.steam_eruption_damage = value;",
            "src/hot.rs:self.steam_eruption_damage = value;",
            "src/hot.rs:self.steam_eruption_damage = value;",
            "src/hot.rs:self.steam_eruption_damage != 0",
        ],
    );
    assert_production_call_sites(
        "set_waterfall_steam_eruption_damage(",
        &[
            "src/boundary.rs:let written = monster.set_waterfall_steam_eruption_damage(damage);",
            "src/hot.rs:pub(crate) fn set_waterfall_steam_eruption_damage(&mut self, damage: i32) -> bool {",
            "src/moves/boss.rs:let written = monster.set_waterfall_steam_eruption_damage(pressure);",
        ],
    );
    assert_production_call_sites(
        "set_constrict_amount(",
        &[
            "src/boundary.rs:let written = state.fanouts.set_constrict_amount(amount);",
            "src/engine/damage.rs:let written = state.fanouts.set_constrict_amount(0);",
            "src/hot.rs:pub(crate) fn set_constrict_amount(&mut self, amount: i32) -> bool {",
            "src/moves/spawned.rs:let written = ctx.state.fanouts.set_constrict_amount(updated);",
        ],
    );
    assert_production_call_sites(
        "constrict_amount()",
        &[
            "src/boundary.rs:&& state.fanouts.constrict_amount() % 3 == 0",
            "src/boundary.rs:&& state.fanouts.constrict_amount() > 0",
            "src/boundary.rs:.filter(|_| state.fanouts.constrict_amount() != 0)",
            "src/boundary.rs:Value::from(state.fanouts.constrict_amount()),",
            "src/engine/damage.rs:if dying_kind == MonsterKind::SlitheringStrangler && state.fanouts.constrict_amount() != 0 {",
            "src/engine/turn.rs:Token::Constrict => [state.fanouts.constrict_amount(), 0, 0, 0],",
            "src/engine/turn.rs:if kind == MonsterKind::SlitheringStrangler && state.fanouts.constrict_amount() != 0 {",
            "src/engine/turn.rs:let amount = state.fanouts.constrict_amount();",
            "src/moves/spawned.rs:&& state.fanouts.constrict_amount() % 3 == 0",
            "src/moves/spawned.rs:let prior = ctx.state.fanouts.constrict_amount();",
            "src/moves/spawned.rs:match state.fanouts.constrict_amount() {",
            "src/moves/spawned.rs:state.fanouts.constrict_amount() > 0",
            "src/moves/spawned.rs:state.fanouts.constrict_amount() > 0 && state.fanouts.constrict_amount() % 3 == 0",
        ],
    );
    assert_production_call_sites(
        "constrict_amount:",
        &[
            "src/hot.rs:constrict_amount: 0,",
            "src/hot.rs:constrict_amount: i32,",
        ],
    );
    assert_production_call_sites(
        ".constrict_amount =",
        &[
            "src/hot.rs:Arc::make_mut(&mut self.0).constrict_amount = amount;",
            "src/hot.rs:Arc::make_mut(&mut self.0).constrict_amount = amount;",
        ],
    );
    assert_production_call_sites(
        "constrict_player_exact(",
        &[
            "src/moves/spawned.rs:constrict_player_exact(ctx)",
            "src/moves/spawned.rs:fn constrict_player_exact(ctx: &mut MoveCtx<'_>) -> Result<(), EngineRefusal> {",
        ],
    );
    // The only amount producers are boundary hydration and the authenticated
    // public writer above; death can only clear it. The absent path must bind
    // the sole fixed opener before the first enemy phase can install it.
    // Pin the combined classifier's complete production call graph so every
    // shared mutation seam still performs the alien/owner check exactly once.
    assert_production_call_sites(
        "constrict_carrier_state(",
        &[
            "src/engine/admission.rs:match crate::moves::spawned::constrict_carrier_state(state) {",
            "src/engine/admission.rs:if crate::moves::spawned::constrict_carrier_state(state)",
            "src/engine/damage.rs:match crate::moves::spawned::constrict_carrier_state(state) {",
            "src/engine/turn.rs:match crate::moves::spawned::constrict_carrier_state(state) {",
            "src/engine/turn.rs:match crate::moves::spawned::constrict_carrier_state(state) {",
            "src/moves/spawned.rs:constrict_carrier_state(state) == ConstrictCarrierState::Malformed",
            "src/moves/spawned.rs:constrict_carrier_state(state) == ConstrictCarrierState::Reachable",
            "src/moves/spawned.rs:if constrict_carrier_state(state) != ConstrictCarrierState::Absent",
            "src/moves/spawned.rs:if constrict_carrier_state(state) != ConstrictCarrierState::Absent {",
            "src/moves/spawned.rs:pub(crate) fn constrict_carrier_state(state: &HotState) -> ConstrictCarrierState {",
        ],
    );
    assert_production_call_sites(
        "constrict_absent_strangler_entry_is_exact(",
        &[
            "src/engine/admission.rs:crate::moves::spawned::constrict_absent_strangler_entry_is_exact(state)",
            "src/engine/turn.rs:&& crate::moves::spawned::constrict_absent_strangler_entry_is_exact(state))",
            "src/moves/spawned.rs:pub(crate) fn constrict_absent_strangler_entry_is_exact(state: &HotState) -> bool {",
        ],
    );
    assert_production_call_sites(
        "constrict_absent_dead_strangler_history_is_exact(",
        &[
            "src/engine/admission.rs:crate::moves::spawned::constrict_absent_dead_strangler_history_is_exact(state)",
            "src/engine/turn.rs:&& crate::moves::spawned::constrict_absent_dead_strangler_history_is_exact(",
            "src/moves/spawned.rs:pub(crate) fn constrict_absent_dead_strangler_history_is_exact(state: &HotState) -> bool {",
        ],
    );
    assert_production_call_sites(
        "intern_private_constrict_strangler(",
        &[
            "src/boundary.rs:builder.intern_private_constrict_strangler()?;",
            "src/catalog.rs:pub(crate) fn intern_private_constrict_strangler(&mut self) -> Result<(), CatalogError> {",
        ],
    );
    assert_production_call_sites(
        "set_teammate_power_pending(",
        &[
            "src/boundary.rs:if pending.0 != 1 || !state.fanouts.set_teammate_power_pending(Some(pending)) {",
            "src/hot.rs:pub(crate) fn set_teammate_power_pending(&mut self, pending: Option<(u32, u32)>) -> bool {",
        ],
    );
    assert_production_call_sites(
        "teammate_power_pending_raw(",
        &[
            "src/boundary.rs:if let Some(pending) = state.fanouts.teammate_power_pending_raw() {",
            "src/engine/admission.rs:.teammate_power_pending_raw()",
            "src/engine/admission.rs:.teammate_power_pending_raw()",
            "src/engine/admission.rs:if state.fanouts.teammate_power_pending_raw().is_some() {",
            "src/engine/mod.rs:if state.fanouts.teammate_power_pending_raw().is_some() {",
            "src/engine/mod.rs:if state.fanouts.teammate_power_pending_raw().is_some() {",
            "src/hot.rs:pub(crate) fn teammate_power_pending_raw(&self) -> Option<(u32, u32)> {",
        ],
    );
    assert_production_call_sites(
        "teammate_power_pending(state.multiplayer_ally_key)",
        &[
            "src/boundary.rs:.teammate_power_pending(state.multiplayer_ally_key)",
            "src/boundary.rs:.teammate_power_pending(state.multiplayer_ally_key)",
        ],
    );
}

#[test]
fn void_form_private_writer_and_reader_call_site_census_is_exact() {
    // The exact foundation remains module-private and has exactly one public
    // family dispatch caller.
    assert_production_call_sites(
        "void_form_foundation_exact(",
        &[
            "src/steps/regent_uncommon.rs:fn void_form_foundation_exact(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {",
            "src/steps/regent_uncommon.rs:void_form_foundation_exact(ctx)",
        ],
    );

    // Pin every explicit power reference.  Together with the universal
    // `.powers =` and generic setter censuses above, this keeps the private
    // amount writer closed to the foundation while still naming every exact
    // reader and the generated id inventory.
    assert_production_call_sites(
        "PowerId::VoidForm",
        &[
            "src/boundary.rs:let amount = state.powers.value(PowerId::VoidForm);",
            "src/engine/admission.rs:PowerId::VoidForm,",
            "src/engine/admission.rs:| PowerId::VoidForm",
            "src/engine/play.rs:let amount = state.powers.value(PowerId::VoidForm);",
            "src/hot.rs:PowerId::VoidForm => Self::VoidForm,",
            "src/ids.rs:PowerId::Vicious, PowerId::Vigor, PowerId::Vital, PowerId::VoidForm, PowerId::Vuln,",
            "src/steps/regent_uncommon.rs:PowerId::VoidForm,",
            "src/steps/regent_uncommon.rs:crate::engine::damage::note_power(events, Subject::Player, PowerId::VoidForm, updated);",
            "src/steps/regent_uncommon.rs:let amount = match state.powers.get(PowerId::VoidForm) {",
            "src/steps/regent_uncommon.rs:let prior = state.powers.value(PowerId::VoidForm);",
            "src/steps/regent_uncommon.rs:state.powers.get(PowerId::VoidForm).is_some()",
            "src/steps/regent_uncommon.rs:state.powers.set(PowerId::VoidForm, SlotWire::Int, updated);",
        ],
    );

    // Bare names include each definition, so UFCS and function-pointer uses
    // cannot bypass review.  True request is written only by the foundation;
    // false only by the top-level service.  Counter writes are exactly the
    // native sentinel, last-in-series completion, and owner-side reset.
    assert_production_call_sites(
        "set_void_form_hooks_registered",
        &[
            "src/boundary.rs:state.fanouts.set_void_form_hooks_registered(true);",
            "src/hot.rs:pub(crate) fn set_void_form_hooks_registered(&mut self, registered: bool) {",
            "src/steps/regent_uncommon.rs:state.fanouts.set_void_form_hooks_registered(true);",
        ],
    );
    assert_production_call_sites(
        "set_void_form_end_turn_requested",
        &[
            "src/boundary.rs:state.fanouts.set_void_form_end_turn_requested(requested);",
            "src/engine/mod.rs:state.fanouts.set_void_form_end_turn_requested(false);",
            "src/hot.rs:pub(crate) fn set_void_form_end_turn_requested(&mut self, requested: bool) {",
            "src/steps/regent_uncommon.rs:state.fanouts.set_void_form_end_turn_requested(true);",
        ],
    );
    assert_production_call_sites(
        "set_void_form_cards_played_this_turn",
        &[
            "src/boundary.rs:.set_void_form_cards_played_this_turn(cards_played)",
            "src/engine/play.rs:.set_void_form_cards_played_this_turn(completed);",
            "src/engine/turn.rs:let written = state.fanouts.set_void_form_cards_played_this_turn(0);",
            "src/hot.rs:pub(crate) fn set_void_form_cards_played_this_turn(&mut self, value: i32) -> bool {",
            "src/steps/regent_uncommon.rs:.set_void_form_cards_played_this_turn(999_999_999);",
        ],
    );
    assert_production_call_sites(
        "void_form_cards_played_this_turn =",
        &["src/hot.rs:Arc::make_mut(&mut self.0).void_form_cards_played_this_turn = value;"],
    );

    // Every counter read is classified: callback write/overflow preflight,
    // active cost threshold, private quotient authentication, or action-entry
    // reachability.  There is no unclassified same-event reader whose order
    // could observe moving the callback write to the end of the represented
    // AfterCardPlayed suffix.
    assert_production_call_sites(
        "void_form_cards_played_this_turn()",
        &[
            "src/boundary.rs:Value::from(state.fanouts.void_form_cards_played_this_turn())",
            "src/engine/play.rs:.void_form_cards_played_this_turn()",
            "src/engine/play.rs:if amount <= 0 || state.fanouts.void_form_cards_played_this_turn() >= amount {",
            "src/steps/regent_uncommon.rs:let completed = state.fanouts.void_form_cards_played_this_turn();",
            "src/steps/regent_uncommon.rs:|| state.fanouts.void_form_cards_played_this_turn() != 0",
        ],
    );
    // Hook reads are exactly the inactive cost branch, active callback
    // branch, side-start reset, and the same validator/reachability pair.
    assert_production_call_sites(
        "void_form_hooks_are_registered()",
        &[
            "src/engine/play.rs:Token::VoidForm => state.fanouts.void_form_hooks_are_registered(),",
            "src/engine/play.rs:if state.fanouts.void_form_hooks_are_registered() {",
            "src/engine/play.rs:if state.fanouts.void_form_hooks_are_registered() {",
            "src/engine/play.rs:if state.fanouts.void_form_hooks_are_registered() {",
            "src/engine/play.rs:|| count(CardToken::VoidForm) != usize::from(state.fanouts.void_form_hooks_are_registered())",
            "src/engine/turn.rs:if state.fanouts.void_form_hooks_are_registered() {",
            "src/steps/regent_uncommon.rs:let hooks = state.fanouts.void_form_hooks_are_registered();",
            "src/steps/regent_uncommon.rs:|| state.fanouts.void_form_hooks_are_registered()",
        ],
    );
    // Request reads are confined to public action/start guards, top-level
    // service, and private validator/reachability. No card body or sibling
    // listener can consume it early.
    assert_production_call_sites(
        "void_form_end_turn_requested()",
        &[
            "src/boundary.rs:PlayerSlot::EndTurnRequested => Value::from(state.fanouts.void_form_end_turn_requested()),",
            "src/engine/admission.rs:if state.fanouts.void_form_end_turn_requested() && state.pending.is_none() {",
            // #3387: the top-level publish of a queued hook action refuses
            // beside a pending Void Form request; it consumes nothing.
            "src/engine/hook_action.rs:if state.fanouts.void_form_end_turn_requested() {",
            "src/engine/mod.rs:if next.fanouts.void_form_end_turn_requested()",
            "src/engine/mod.rs:if next.fanouts.void_form_end_turn_requested()",
            "src/engine/mod.rs:if state.fanouts.void_form_end_turn_requested() && state.pending.is_none() {",
            "src/engine/mod.rs:if state.fanouts.void_form_end_turn_requested() && state.pending.is_none() {",
            "src/engine/turn.rs:if state.fanouts.void_form_end_turn_requested() {",
            "src/engine/turn.rs:if state.fanouts.void_form_end_turn_requested() {",
            "src/steps/regent_uncommon.rs:let requested = state.fanouts.void_form_end_turn_requested();",
            "src/steps/regent_uncommon.rs:|| state.fanouts.void_form_end_turn_requested()",
        ],
    );
    assert_production_call_sites(
        "VOID_FORM_HOOKS_MASK",
        &[
            "src/hot.rs:*flags &= !VOID_FORM_HOOKS_MASK;",
            "src/hot.rs:*flags |= VOID_FORM_HOOKS_MASK;",
            "src/hot.rs:const VOID_FORM_HOOKS_MASK: u8 = 0b0100_0000;",
            "src/hot.rs:self.0.private_power_flags & VOID_FORM_HOOKS_MASK != 0",
        ],
    );
    assert_production_call_sites(
        "VOID_FORM_END_TURN_REQUESTED_MASK",
        &[
            "src/hot.rs:*flags &= !VOID_FORM_END_TURN_REQUESTED_MASK;",
            "src/hot.rs:*flags |= VOID_FORM_END_TURN_REQUESTED_MASK;",
            "src/hot.rs:const VOID_FORM_END_TURN_REQUESTED_MASK: u8 = 0b1000_0000;",
            "src/hot.rs:self.0.private_power_flags & VOID_FORM_END_TURN_REQUESTED_MASK != 0",
        ],
    );
    // Void Form's i32 ledger and the four compact fanout-order lengths consume
    // no new layout: result-location length uses the retired low pair plus bit
    // seven, alongside the Star Energy/local-generated lengths. Exact line
    // inventory prevents a future raw write from bypassing checked accessors.
    assert_production_call_sites(
        "turn_and_generated_lens",
        &[
            "src/hot.rs:turn_and_generated_lens: u8,",
            "src/hot.rs:turn_and_generated_lens: 0,",
            "src/hot.rs:(self.0.turn_and_generated_lens & STAR_ENERGY_RESET_LEN_MASK) >> STAR_ENERGY_RESET_LEN_SHIFT",
            "src/hot.rs:(self.0.turn_and_generated_lens & LOCAL_GENERATED_LEN_MASK) >> LOCAL_GENERATED_LEN_SHIFT",
            "src/hot.rs:(state.turn_and_generated_lens & LOCAL_GENERATED_LEN_MASK) >> LOCAL_GENERATED_LEN_SHIFT;",
            "src/hot.rs:let len = (state.turn_and_generated_lens & STAR_ENERGY_RESET_LEN_MASK)",
            "src/hot.rs:let len = result_location_len(self.0.turn_and_generated_lens);",
            "src/hot.rs:let len = usize::from(result_location_len(state.turn_and_generated_lens));",
            "src/hot.rs:let lengths = self.0.turn_and_generated_lens;",
            "src/hot.rs:let occupied = usize::from(result_location_len(state.turn_and_generated_lens));",
            "src/hot.rs:let packed_len = (state.turn_and_generated_lens & STAR_ENERGY_RESET_LEN_MASK)",
            "src/hot.rs:state.turn_and_generated_lens =",
            "src/hot.rs:state.turn_and_generated_lens =",
            "src/hot.rs:state.turn_and_generated_lens =",
            "src/hot.rs:state.turn_and_generated_lens = (state.turn_and_generated_lens",
            "src/hot.rs:state.turn_and_generated_lens = (state.turn_and_generated_lens",
            "src/hot.rs:state.turn_and_generated_lens = (state.turn_and_generated_lens",
            "src/hot.rs:state.turn_and_generated_lens = (state.turn_and_generated_lens & !LOCAL_GENERATED_LEN_MASK)",
            "src/hot.rs:state.turn_and_generated_lens = (state.turn_and_generated_lens & !LOCAL_GENERATED_LEN_MASK)",
            "src/hot.rs:with_result_location_len(state.turn_and_generated_lens, len as u8 - 1);",
            "src/hot.rs:with_result_location_len(state.turn_and_generated_lens, occupied as u8 + 1);",
            "src/hot.rs:with_result_location_len(state.turn_and_generated_lens, order.len() as u8);",
        ],
    );
    // Hello World's exact-pool provenance and closed snapshot delta, Calamity's
    // persistent hook-live mirror, and Entropy's full-order insertion ordinal
    // share the byte which previously held only the Hello provenance bool. Every
    // raw read/write remains inside HotState's accessors; admission is the sole
    // external raw-vocabulary validator. This census also prevents an accidental
    // Arc fanout write.
    assert_production_call_sites(
        "generated_power_flags",
        &[
            "src/engine/admission.rs:if !state.generated_power_flags_are_exact() || state.calamity_hook_is_live() != calamity_live {",
            "src/hot.rs:(self.generated_power_flags & ENTROPY_TURN_START_ORDINAL_MASK)",
            "src/hot.rs:(self.generated_power_flags & ENTROPY_TURN_START_ORDINAL_MASK)",
            "src/hot.rs:generated_power_flags: 0,",
            "src/hot.rs:generated_power_flags: u8,",
            "src/hot.rs:if self.generated_power_flags & !GENERATED_POWER_FLAGS_MASK != 0",
            "src/hot.rs:let dirty = self.generated_power_flags & HELLO_WORLD_SNAPSHOT_DIRTY_MASK != 0;",
            "src/hot.rs:pub(crate) fn generated_power_flags_are_exact(&self) -> bool {",
            "src/hot.rs:self.generated_power_flags & CALAMITY_HOOK_LIVE_MASK != 0",
            "src/hot.rs:self.generated_power_flags & CALL_OF_THE_VOID_GENERATION_POOL_MASK != 0",
            "src/hot.rs:self.generated_power_flags & HELLO_WORLD_GENERATION_POOL_MASK != 0",
            "src/hot.rs:self.generated_power_flags & HELLO_WORLD_SNAPSHOT_DIRTY_MASK == 0",
            "src/hot.rs:self.generated_power_flags &= !CALAMITY_HOOK_LIVE_MASK;",
            "src/hot.rs:self.generated_power_flags &= !ENTROPY_TURN_START_ORDINAL_MASK;",
            "src/hot.rs:self.generated_power_flags &= !HELLO_WORLD_SNAPSHOT_DIRTY_MASK;",
            "src/hot.rs:self.generated_power_flags &= !HELLO_WORLD_SNAPSHOT_DIRTY_MASK;",
            "src/hot.rs:self.generated_power_flags |= (ordinal as u8) << ENTROPY_TURN_START_ORDINAL_SHIFT;",
            "src/hot.rs:self.generated_power_flags |= CALAMITY_HOOK_LIVE_MASK;",
            "src/hot.rs:self.generated_power_flags |= CALL_OF_THE_VOID_GENERATION_POOL_MASK;",
            "src/hot.rs:self.generated_power_flags |= HELLO_WORLD_GENERATION_POOL_MASK;",
            "src/hot.rs:self.generated_power_flags |= HELLO_WORLD_SNAPSHOT_DIRTY_MASK;",
            "src/hot.rs:|| !self.generated_power_flags_are_exact()",
        ],
    );
    assert_production_call_sites(
        "publish_hello_world_generation_pool(",
        &[
            "src/boundary.rs:state.publish_hello_world_generation_pool();",
            "src/hot.rs:pub(crate) fn publish_hello_world_generation_pool(&mut self) {",
        ],
    );
    assert_production_call_sites(
        "set_hello_world_amount_on_turn_start(",
        &[
            "src/boundary.rs:if !state.set_hello_world_amount_on_turn_start(hello_world_current, hello_world_snapshot) {",
            "src/hot.rs:pub(crate) fn set_hello_world_amount_on_turn_start(",
            "src/steps/templates.rs:.set_hello_world_amount_on_turn_start(current, snapshot);",
        ],
    );
    assert_production_call_sites(
        "freeze_hello_world_amount_on_turn_start(",
        &[
            "src/engine/turn.rs:state.freeze_hello_world_amount_on_turn_start();",
            "src/hot.rs:pub(crate) fn freeze_hello_world_amount_on_turn_start(&mut self) {",
        ],
    );
    assert_production_call_sites(
        "set_calamity_hook_live(",
        &[
            "src/boundary.rs:state.set_calamity_hook_live(state.powers.value(PowerId::Calamity) > 0);",
            "src/hot.rs:pub(crate) fn set_calamity_hook_live(&mut self, live: bool) {",
            "src/steps/neutral.rs:ctx.state.set_calamity_hook_live(true);",
        ],
    );
    assert_production_call_sites(
        "intercept_covered_state",
        &[
            "src/hot.rs:intercept_covered_state: u8,",
            "src/hot.rs:intercept_covered_state: 0,",
            "src/hot.rs:let packed = self.0.intercept_covered_state;",
            "src/hot.rs:(self.0.intercept_covered_state & IMITATION_LEARNING_ORDER_MASK)",
            "src/hot.rs:let prior_order = (state.intercept_covered_state & IMITATION_LEARNING_ORDER_MASK)",
            "src/hot.rs:state.intercept_covered_state = (state.intercept_covered_state",
            "src/hot.rs:state.intercept_covered_state = (state.intercept_covered_state",
            "src/hot.rs:key < 2 && self.0.intercept_covered_state & (1 << key) != 0",
            "src/hot.rs:self.0.intercept_covered_state & INTERCEPT_COVERED_MASK",
            "src/hot.rs:(self.0.intercept_covered_state & INTERCEPT_COVERED_ORDER_MASK)",
            "src/hot.rs:if state.intercept_covered_state & bit != 0 {",
            "src/hot.rs:let prior_order = (state.intercept_covered_state & INTERCEPT_COVERED_ORDER_MASK)",
            "src/hot.rs:state.intercept_covered_state = (state.intercept_covered_state",
            "src/hot.rs:| ((state.intercept_covered_state & INTERCEPT_COVERED_MASK) | bit)",
            "src/hot.rs:let prior_order = (state.intercept_covered_state & INTERCEPT_COVERED_ORDER_MASK)",
            "src/hot.rs:state.intercept_covered_state = (state.intercept_covered_state",
        ],
    );

    // The former kind-bucket fanout is gone: synchronous and persisted paths
    // now enter the same per-instance semantics through separately pinned
    // seams. The one Hand token is the only authenticated absence between
    // manual SpendResources cost reads.
    assert_production_call_sites("after_card_played_fanouts(", &[]);
    assert_production_call_sites(
        "apply_monologue_instance_after_card_played_sync(",
        &[
            "src/engine/play.rs:apply_monologue_instance_after_card_played_sync(",
            "src/engine/play.rs:apply_monologue_instance_after_card_played_sync(state, catalog, latch, uid, events)?;",
            "src/engine/play.rs:fn apply_monologue_instance_after_card_played_sync(",
        ],
    );
    assert_production_call_sites(
        "enter_instanced_power_after_card_played(",
        &[
            "src/engine/play.rs:enter_instanced_power_after_card_played(",
            "src/engine/play.rs:fn enter_instanced_power_after_card_played(",
        ],
    );
    assert_production_call_sites(
        "apply_active_void_form_after_card_played(",
        &[
            "src/engine/play.rs:Token::VoidForm => apply_active_void_form_after_card_played(state, void_form_increments)?,",
            "src/engine/play.rs:fn apply_active_void_form_after_card_played(",
        ],
    );
    assert_production_call_sites(
        "void_form_listener_is_reachable(",
        &[
            "src/boundary.rs:if crate::steps::regent_uncommon::void_form_listener_is_reachable(state)",
            "src/engine/admission.rs:if crate::steps::regent_uncommon::void_form_listener_is_reachable(state)",
            "src/engine/mod.rs:if crate::steps::regent_uncommon::void_form_listener_is_reachable(state)",
            "src/engine/mod.rs:if crate::steps::regent_uncommon::void_form_listener_is_reachable(state)",
            "src/engine/mod.rs:if crate::steps::regent_uncommon::void_form_listener_is_reachable(state)",
            "src/engine/play.rs:crate::steps::regent_uncommon::void_form_listener_is_reachable(state);",
            "src/engine/play.rs:crate::steps::regent_uncommon::void_form_listener_is_reachable(state);",
            "src/engine/turn.rs:if crate::steps::regent_uncommon::void_form_listener_is_reachable(state) {",
            "src/engine/turn.rs:let void_form_reachable = crate::steps::regent_uncommon::void_form_listener_is_reachable(state);",
            "src/steps/regent_uncommon.rs:pub(crate) fn void_form_listener_is_reachable(state: &HotState) -> bool {",
        ],
    );
    assert_production_call_sites(
        "void_form_private_state_is_exact(",
        &[
            "src/boundary.rs:&& !crate::steps::regent_uncommon::void_form_private_state_is_exact(state)",
            "src/boundary.rs:if !crate::steps::regent_uncommon::void_form_private_state_is_exact(state) {",
            "src/engine/admission.rs:&& !crate::steps::regent_uncommon::void_form_private_state_is_exact(state)",
            "src/engine/mod.rs:if !crate::steps::regent_uncommon::void_form_private_state_is_exact(state) {",
            "src/engine/mod.rs:if !crate::steps::regent_uncommon::void_form_private_state_is_exact(state) {",
            "src/engine/mod.rs:if !crate::steps::regent_uncommon::void_form_private_state_is_exact(state) {",
            "src/engine/mod.rs:&& !crate::steps::regent_uncommon::void_form_private_state_is_exact(state)",
            "src/engine/mod.rs:&& !crate::steps::regent_uncommon::void_form_private_state_is_exact(state)",
            "src/engine/mod.rs:&& !crate::steps::regent_uncommon::void_form_private_state_is_exact(state)",
            "src/engine/play.rs:if !crate::steps::regent_uncommon::void_form_private_state_is_exact(state) {",
            "src/engine/play.rs:if !crate::steps::regent_uncommon::void_form_private_state_is_exact(state) {",
            "src/engine/play.rs:&& !crate::steps::regent_uncommon::void_form_private_state_is_exact(state)",
            "src/engine/play.rs:&& !crate::steps::regent_uncommon::void_form_private_state_is_exact(state)",
            "src/engine/turn.rs:if !crate::steps::regent_uncommon::void_form_private_state_is_exact(state) {",
            "src/engine/turn.rs:if !crate::steps::regent_uncommon::void_form_private_state_is_exact(state) {",
            "src/engine/turn.rs:if !crate::steps::regent_uncommon::void_form_private_state_is_exact(state) {",
            "src/steps/regent_uncommon.rs:pub(crate) fn void_form_private_state_is_exact(state: &HotState) -> bool {",
            "src/steps/regent_uncommon.rs:|| !void_form_private_state_is_exact(ctx.state)",
            "src/steps/regent_uncommon.rs:if !void_form_private_state_is_exact(&probe) {",
        ],
    );
    assert_production_call_sites(
        "service_void_form_end_turn(",
        &[
            "src/engine/mod.rs:fn service_void_form_end_turn(",
            "src/engine/mod.rs:service_void_form_end_turn(&mut next, catalog, events, false)?;",
            "src/engine/mod.rs:service_void_form_end_turn(&mut next, catalog, events, true)?;",
        ],
    );
    assert_production_call_sites(
        "resolved_star_cost_with_transit(",
        &[
            "src/engine/play.rs:&& resolved_star_cost_with_transit(state, card, spec, None)",
            "src/engine/play.rs:fn resolved_star_cost_with_transit(",
            "src/engine/play.rs:resolved_star_cost_with_transit(state, card, &spec, Some(PileId::Hand))",
            "src/engine/play.rs:resolved_star_cost_with_transit(state, card, spec, None)",
        ],
    );
    assert_production_call_sites(
        "resolved_star_cost_without_void_form(",
        &[
            "src/engine/play.rs:pub(crate) fn resolved_star_cost_without_void_form(",
            "src/engine/play.rs:let ordinary_cost = resolved_star_cost_without_void_form(state, card, spec);",
            "src/engine/potions.rs:|| super::play::resolved_star_cost_without_void_form(state, *card, spec)",
        ],
    );
    assert_production_call_sites(
        "void_form_zeroes_cost_active(",
        &[
            "src/engine/play.rs:fn void_form_zeroes_cost_active(",
            "src/engine/play.rs:if void_form_zeroes_cost_active(state, card, None) {",
            "src/engine/play.rs:if void_form_zeroes_cost_active(state, card, None)",
            "src/engine/play.rs:if void_form_zeroes_cost_active(state, card, transit_pile) {",
        ],
    );
    assert_production_call_sites(
        "resolved_energy_cost_with_active_void_form(",
        &[
            "src/engine/play.rs:fn resolved_energy_cost_with_active_void_form(",
            "src/engine/play.rs:return resolved_energy_cost_with_active_void_form(state, card, spec, without_free);",
        ],
    );
    assert_production_call_sites(
        "resolved_energy_cost_without_free_active_void_form(",
        &[
            "src/engine/play.rs:fn resolved_energy_cost_without_free_active_void_form(",
            "src/engine/play.rs:return resolved_energy_cost_without_free_active_void_form(state, card, spec, early);",
        ],
    );
    assert_production_call_sites(
        "resolved_star_cost_with_active_void_form(",
        &[
            "src/engine/play.rs:fn resolved_star_cost_with_active_void_form(",
            "src/engine/play.rs:return resolved_star_cost_with_active_void_form(state, card, transit_pile, ordinary_cost);",
        ],
    );

    // Legal-action enumeration reaches the cost reader for every candidate.
    // Each fixed-cost reader must branch on the cached hook bit and return its
    // cold final-result helper directly. Inactive Energy therefore remains a
    // frameless leaf/tail branch, while every active helper reaches the full
    // private validator through `void_form_zeroes_cost_active`.
    let play = HOT_MODULES
        .into_iter()
        .find_map(|(path, source)| (path == "src/engine/play.rs").then_some(source))
        .expect("engine/play.rs is a hot module");
    for (reader, cold_return) in [
        (
            "pub(crate) fn resolved_energy_cost(",
            "return resolved_energy_cost_with_active_void_form(",
        ),
        (
            "fn resolved_energy_cost_without_free(",
            "return resolved_energy_cost_without_free_active_void_form(",
        ),
        (
            "fn resolved_star_cost_with_transit(",
            "return resolved_star_cost_with_active_void_form(",
        ),
    ] {
        let body = &play[play.find(reader).expect("fixed-cost reader exists")..];
        let hook_gate = body
            .find("if state.fanouts.void_form_hooks_are_registered()")
            .expect("Void Form cached hook gate exists");
        let cold_return = body
            .find(cold_return)
            .expect("Void Form cold final-result return exists");
        assert!(
            hook_gate < cold_return,
            "inactive hook gate must control the cold final-result return"
        );
    }
    let active = &play[play
        .find("fn void_form_zeroes_cost_active(")
        .expect("Void Form active cost helper exists")..];
    assert!(
        active
            .contains("if !crate::steps::regent_uncommon::void_form_private_state_is_exact(state)")
    );

    let tender_branch = play
        .find("Token::Tender => apply_active_tender_after_card_played")
        .expect("Tender has one generalized object-ledger branch");
    let void_branch = play
        .find("Token::VoidForm => apply_active_void_form_after_card_played")
        .expect("Void Form has one generalized object-ledger branch");
    assert!(
        tender_branch < void_branch,
        "the exhaustive token dispatcher preserves native family order"
    );
}

/// `String` as a *type*, not as a fragment of a content identifier.
///
/// The generated dispatch trees name every content kind, and the build has a
/// `QUEEN_PUPPET_STRINGS` move — a substring match would have called that an
/// owned text type. An occurrence preceded by an identifier character is part
/// of a longer name and is not one.
fn owned_text_sites(source: &str) -> usize {
    source
        .match_indices(OWNED_TEXT)
        .filter(|(index, _)| {
            *index == 0
                || !source[..*index]
                    .chars()
                    .next_back()
                    .is_some_and(|previous| previous.is_alphanumeric() || previous == '_')
        })
        .count()
}

#[test]
fn hot_modules_name_no_owned_text_type() {
    for (path, source) in HOT_MODULES {
        assert_eq!(
            owned_text_sites(source),
            0,
            "{path} names an owned text type; ids are interned below the \
             canonical boundary (PORT_PLAN.md D2)"
        );
    }
}

#[test]
fn no_module_below_the_boundary_dispatches_on_text_equality() {
    for (path, source) in HOT_MODULES.into_iter().chain([BOUNDARY]) {
        for pattern in TEXT_EQUALITY {
            assert_eq!(
                occurrences(source, pattern),
                0,
                "{path} compares text ({pattern:?}); dispatch and predicates \
                 use enums (PORT_PLAN.md D2)"
            );
        }
    }
}

#[test]
fn hot_modules_carry_no_maps_or_boxed_chains() {
    for (path, source) in HOT_MODULES {
        let allowance = if path.ends_with("catalog.rs") {
            CATALOG_BUILDER_MAP_SITES
        } else {
            0
        };
        let found: usize = BANNED_CONTAINERS
            .iter()
            .map(|pattern| occurrences(source, pattern))
            .sum();
        assert_eq!(
            found, allowance,
            "{path} has {found} map/boxed-chain sites, expected {allowance} \
             (PORT_PLAN.md D3: no BTreeMap, no per-node Box chains in any hot \
             type; the catalog *builder* is the one sanctioned exception)"
        );
    }
}

#[test]
fn the_boundary_is_the_only_module_allowed_to_own_text() {
    // Not a vacuous assertion in the other direction: the boundary must
    // actually be building the string-keyed document, or the split above
    // would be measuring nothing.
    let (path, source) = BOUNDARY;
    assert!(
        owned_text_sites(source) > 0,
        "{path} owns no text at all — is it still the canonical side?"
    );
}

/// The dispatch trees are below the boundary too: a family body runs per play,
/// and its arguments are compiled, so it has no business naming a text type or
/// comparing one.
#[test]
fn the_dispatch_trees_name_no_owned_text_and_compare_none() {
    let sources = dispatch_sources();
    assert!(
        sources.len() >= 30,
        "only {} dispatch modules found; the tree scan is probably wrong",
        sources.len()
    );
    for (path, source) in &sources {
        assert_eq!(
            owned_text_sites(source),
            0,
            "{path} names an owned text type (PORT_PLAN.md D2)"
        );
        for pattern in TEXT_EQUALITY {
            assert_eq!(
                occurrences(source, pattern),
                0,
                "{path} compares text ({pattern:?}); the admission-time compiler \
                 exists so a step body never has to (D2, #1297)"
            );
        }
        let found: usize = BANNED_CONTAINERS
            .iter()
            .map(|pattern| occurrences(source, pattern))
            .sum();
        assert_eq!(found, 0, "{path} has {found} map/boxed-chain sites (D3)");
    }
}

/// D4 completeness *and* uniqueness, derived from the generated index rather
/// than from a list anybody maintains: every kind has exactly one family, that
/// family's module exists, and it carries a body with the dispatched name.
#[test]
fn every_kind_dispatches_to_exactly_one_family_body() {
    use sts_sim::ids::{MoveKind, StepKind};

    let sources: std::collections::BTreeMap<String, String> =
        dispatch_sources().into_iter().collect();

    /// The `match` arms of a generated `mod.rs`, as
    /// `(variant, family, function)` — derived from the dispatch itself, so
    /// the test cannot disagree with it about naming.
    fn arms(source: &str, enum_name: &str) -> Vec<(String, String, String)> {
        // Both rustfmt arm layouts: `K => f::g(ctx),` and the braced form a
        // long arm wraps into.
        let lines: Vec<&str> = source.lines().map(str::trim).collect();
        let mut out = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            let Some((left, right)) = line.split_once(" => ") else {
                continue;
            };
            let Some(variant) = left.strip_prefix(&format!("{enum_name}::")) else {
                continue;
            };
            let call = if right == "{" {
                lines.get(index + 1).copied().unwrap_or_default()
            } else {
                right
            };
            let Some((family, rest)) = call.split_once("::") else {
                continue;
            };
            let Some(function) = rest.split('(').next() else {
                continue;
            };
            out.push((
                variant.to_string(),
                family.to_string(),
                function.to_string(),
            ));
        }
        out
    }

    for (tree, enum_name, index, total) in [
        (
            "steps",
            "StepKind",
            sts_sim::steps::FAMILY_OF
                .iter()
                .map(|(kind, family)| (format!("{kind:?}"), *family))
                .collect::<Vec<(String, &str)>>(),
            StepKind::COUNT,
        ),
        (
            "moves",
            "MoveKind",
            sts_sim::moves::FAMILY_OF
                .iter()
                .map(|(kind, family)| (format!("{kind:?}"), *family))
                .collect::<Vec<(String, &str)>>(),
            MoveKind::COUNT,
        ),
    ] {
        let dispatch = sources
            .get(&format!("src/{tree}/mod.rs"))
            .expect("the generated dispatch module");
        let arms = arms(dispatch, enum_name);
        let mut seen = std::collections::BTreeSet::new();
        for (variant, family, function) in &arms {
            assert!(
                seen.insert(variant.clone()),
                "src/{tree}/mod.rs dispatches {enum_name}::{variant} twice"
            );
            let source = sources
                .get(&format!("src/{tree}/{family}.rs"))
                .unwrap_or_else(|| panic!("src/{tree}/{family}.rs is missing"));
            assert!(
                source.contains(&format!("fn {function}(")),
                "src/{tree}/{family}.rs has no body for {enum_name}::{variant}"
            );
        }
        assert_eq!(
            seen.len(),
            total,
            "src/{tree}/mod.rs covers {} of {total} {enum_name} variants",
            seen.len()
        );
        // The published index and the dispatch must name the same owner.
        let by_variant: std::collections::BTreeMap<&str, &str> = arms
            .iter()
            .map(|(variant, family, _)| (variant.as_str(), family.as_str()))
            .collect();
        for (variant, family) in &index {
            assert_eq!(
                by_variant.get(variant.as_str()),
                Some(family),
                "FAMILY_OF and the dispatch disagree about {enum_name}::{variant}"
            );
        }
        assert_eq!(index.len(), total);
    }

    // And the manifest's registries are subsets of what the index knows about:
    // a family cannot claim a kind that dispatches somewhere else.
    for (family, kinds) in sts_sim::steps::FAMILIES {
        for kind in kinds {
            let owner = sts_sim::steps::FAMILY_OF
                .iter()
                .find(|(indexed, _)| indexed == kind)
                .map(|(_, owner)| *owner);
            assert_eq!(
                owner,
                Some(family),
                "{:?} is claimed by the wrong family",
                kind.as_str()
            );
        }
    }
    for (family, kinds) in sts_sim::moves::FAMILIES {
        for kind in kinds {
            let owner = sts_sim::moves::FAMILY_OF
                .iter()
                .find(|(indexed, _)| indexed == kind)
                .map(|(_, owner)| *owner);
            assert_eq!(
                owner,
                Some(family),
                "{:?} is claimed by the wrong family",
                kind.as_str()
            );
        }
    }
}

/// D5: the manifest's `hooks_fired` list is a claim about the engine's source,
/// so it is checked against the source rather than trusted.
#[test]
fn every_declared_fire_point_exists_and_the_undeclared_ones_do_not() {
    use sts_sim::hooks::HookEvent;

    let engine: String = HOT_MODULES
        .iter()
        .filter(|(path, _)| path.starts_with("src/engine/"))
        .map(|(_, source)| *source)
        .chain(
            dispatch_sources()
                .iter()
                .map(|(_, source)| source.as_str())
                .collect::<Vec<&str>>(),
        )
        .collect::<Vec<&str>>()
        .join("\n");

    for event in sts_sim::engine::FIRE_POINTS {
        let site = format!("HookEvent::{event:?}");
        assert!(
            engine.contains(&site),
            "{site} is declared a fire point but never fired"
        );
    }
    for event in HookEvent::ALL {
        if sts_sim::engine::FIRE_POINTS.contains(&event) {
            continue;
        }
        let site = format!("HookEvent::{event:?},\n            state");
        assert!(
            !engine.contains(&site),
            "{event:?} is fired without being declared a fire point"
        );
    }
    // 15 until #2528 E4a, when entry synthesis moved into Rust and the two
    // combat-start hooks gained a fire site. Moving this number is a
    // three-part change on purpose — the array, this assertion, and
    // `FIRE_POINTS`' doc comment — so a hook cannot start firing as a side
    // effect of some other edit.
    assert_eq!(sts_sim::engine::FIRE_POINTS.len(), 17);
    for event in [HookEvent::AfterRoomEntered, HookEvent::BeforeCombatStart] {
        assert!(
            sts_sim::engine::FIRE_POINTS.contains(&event),
            "the opening fires {event:?}"
        );
    }
}

/// The two combat-start hooks have **different** subscriber domains, and the
/// difference is not visible in `FIRE_POINTS`.
///
/// `Hook/<AfterRoomEntered>d__72::MoveNext` (`0x3d0eec`) passes `ldnull` as the
/// child combat state, so no creature power can observe it;
/// `Hook/<BeforeCombatStart>d__18::MoveNext` (`0x3d2574`) passes the live
/// state, and walks the list **twice**. Wiring `AfterRoomEntered` to the shared
/// `fire_hook` would silently widen its domain to every power, which is exactly
/// the kind of change a length assertion cannot see — so it is pinned at the
/// source.
#[test]
fn the_two_combat_start_hooks_keep_their_distinct_dispatch() {
    let engine = HOT_MODULES
        .iter()
        .find_map(|(path, source)| (*path == "src/engine/mod.rs").then_some(*source))
        .expect("engine/mod.rs is a hot module");
    let after_room_entered = engine
        .split_once("pub(crate) fn fire_after_room_entered(")
        .map(|(_, body)| body.split("\npub").next().unwrap_or(body))
        .expect("the AfterRoomEntered fire point exists");
    assert!(
        !after_room_entered.contains("fire_hook("),
        "AfterRoomEntered must not route through the shared combat-state walk"
    );
    assert!(
        after_room_entered.contains("PowerHookNotModeled"),
        "AfterRoomEntered must refuse a power subscriber rather than fire it"
    );
    // #2827: Ghost Seed's room-entry body is not a compiled subscriber. It runs
    // after the compiled walk, so the fire point must reach it even when that
    // walk is empty — an early return on `!hooks.has(event)` would skip it for
    // every owner without a template AfterRoomEntered relic.
    assert!(
        after_room_entered.contains("cards::ghost_seed_after_room_entered(state, catalog)"),
        "AfterRoomEntered must run Ghost Seed's body after the compiled walk"
    );
    assert!(
        !after_room_entered.contains("if !hooks.has(event) {\n        return Ok(());"),
        "AfterRoomEntered must not return before Ghost Seed's body"
    );
    let before_combat_start = engine
        .split_once("pub(crate) fn fire_before_combat_start(")
        .map(|(_, body)| body.split("\n/// ").next().unwrap_or(body))
        .expect("the BeforeCombatStart fire point exists");
    // Until #2827 the second pass was a by-name `RelicPetrifiedToad` refusal
    // inline here. It is now the Toad's body, called by name, and that body
    // still reads ownership from the catalog rather than a compiled list.
    assert!(
        before_combat_start.contains("petrified_toad_before_combat_start_late("),
        "the BeforeCombatStartLate pass must still be executed, not skipped"
    );
    let late_pass = engine
        .split_once("fn petrified_toad_before_combat_start_late(")
        .map(|(_, body)| body.split("\n/// ").next().unwrap_or(body))
        .expect("the BeforeCombatStartLate body exists");
    assert!(
        late_pass.contains("owns(RelicId::RelicPetrifiedToad)"),
        "the BeforeCombatStartLate pass must be derived from catalog ownership"
    );
}

#[test]
fn the_admission_gate_is_boundary_time_work_only() {
    let (path, source) = ADMISSION;
    // Its containers are permitted, but only because nothing on a per-node
    // path calls it. `apply_action`/`legal_actions` must not: the engine's
    // contract is that admission happens once, at load.
    for hot in ["src/engine/play.rs", "src/engine/turn.rs"] {
        let (_, hot_source) = HOT_MODULES
            .iter()
            .find(|(name, _)| *name == hot)
            .expect("the hot module list names the engine bodies");
        assert!(
            !hot_source.contains("admission::admit") && !hot_source.contains(" admit("),
            "{hot} calls the admission gate on a per-node path"
        );
    }
    // And its registry must still be DERIVED from the generated dispatch
    // trees (D6). A literal list here — the kernel's mistake — would let the
    // gate and the dispatch disagree about what is modeled.
    assert!(
        source.contains("crate::steps::is_implemented")
            && source.contains("crate::moves::is_implemented"),
        "{path} no longer derives its registry from the generated dispatch trees"
    );
    assert!(
        source.contains("crate::steps::FAMILIES") && source.contains("crate::moves::FAMILIES"),
        "{path}'s capability manifest is no longer derived from the family registries"
    );
}

/// Coverage may only be recorded where a kind is actually dispatched (#1291).
///
/// The differential's whole claim is that a config's `must_cover` reports what
/// *ran*. A wave PR that wrote `coverage::record_step(StepKind::Mine)` into its
/// own family body could satisfy that assertion without the body ever
/// executing — the one cheap way to fake a green gate. So the recording calls
/// are pinned to the three dispatch/fire sites, and the family files, which no
/// shared test enumerates, may not record at all.
#[test]
fn only_the_dispatch_sites_record_coverage() {
    const CALLS: [&str; 4] = ["record_step", "record_move", "record_hook", "record_power"];
    const ALLOWED: [(&str, &str); 3] = [
        ("src/engine/play.rs", "record_step"),
        ("src/engine/turn.rs", "record_move"),
        ("src/engine/mod.rs", "record_hook"),
    ];
    for (path, source) in HOT_MODULES.into_iter().chain([BOUNDARY, ADMISSION]) {
        for call in CALLS {
            let permitted = ALLOWED
                .iter()
                .any(|(module, allowed)| *module == path && *allowed == call);
            assert_eq!(
                source.contains(&format!("coverage::{call}")),
                permitted,
                "{path} {} coverage::{call}",
                if permitted {
                    "no longer records"
                } else {
                    "must not record"
                }
            );
        }
    }
    for (path, source) in dispatch_sources() {
        assert!(
            !source.contains("coverage::record"),
            "{path} records coverage; only the dispatch sites may, or a family \
             could claim a kind whose body never ran"
        );
    }
}

/// Curl Up retains the exact physical card that dealt a powered hit. Every
/// executable card-attack body must therefore publish StepCtx's source uid;
/// one legacy call would silently turn that body's attacks into an
/// unrepresentable null source against Louse Progenitor.
#[test]
fn every_card_attack_site_carries_the_physical_source_uid() {
    let mut sites = 0;
    let mut per_call = std::collections::BTreeMap::<&str, usize>::new();
    for (path, source) in all_rust_sources() {
        let production = production_source(&source);
        let is_step_body = path.starts_with("src/steps/") && !path.ends_with("/mod.rs");
        if is_step_body {
            for line in production.lines().map(str::trim) {
                assert!(
                    !line.starts_with("player_attack("),
                    "{path} retains a powered attack with no physical source uid: {line}"
                );
            }
        }
        let compact: String = production
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect();
        // `player_attack_from_card` is a `Targeting(creature)` command;
        // `player_attack_all_from_card` is `TargetingAllOpponents`, which
        // re-resolves its receivers per hit (#3023). Both are powered card
        // attacks under the same physical-source contract.
        for call in ["player_attack_from_card(", "player_attack_all_from_card("] {
            // `player_attack_from_card(` is not a substring of the `_all_`
            // name, so the two counts are disjoint.
            let found = compact.matches(call).count();
            if path == "src/engine/damage.rs" {
                assert!(
                    compact.contains(&format!("pubfn{call}")),
                    "the shared powered-card attack entry point {call} moved"
                );
                assert_eq!(
                    found, 1,
                    "{path} must contain only the {call} function definition"
                );
                continue;
            }
            if !is_step_body {
                assert_eq!(
                    found, 0,
                    "{path} owns a powered-card attack body outside src/steps/"
                );
                continue;
            }
            for (index, _) in compact.match_indices(call) {
                sites += 1;
                *per_call.entry(call).or_insert(0) += 1;
                let args = &compact[index + call.len()..];
                assert!(
                    args.starts_with("ctx.state,(ctx.catalog,ctx.spec,ctx.source_uid),"),
                    "{path} has a card attack that does not publish the complete StepCtx source"
                );
            }
        }
    }
    // The private The Ball body foundation, split Forge-family bodies, and
    // Sovereign Blade's and Shiv's one-target/multi-target branches, plus
    // Fiend Fire's separately resumed post-Exhaust attack, contribute
    // fifty-five physical-source attack sites. Every branch remains under the
    // same exact source-UID contract even while private dispatch stays refused.
    // #3023 moved the eleven all-opponents bodies (attack_all, Whirlwind,
    // Stomp, Radiate, Thunderclap, Shatter, Meteor Shower, Crash Landing,
    // Pact's End, and the AllEnemies Shiv and Sovereign Blade branches) to
    // `player_attack_all_from_card`; the total is unchanged.
    assert_eq!(
        sites, 55,
        "the executable powered-card attack census drifted"
    );
    assert_eq!(
        per_call.get("player_attack_all_from_card(").copied(),
        Some(11),
        "the all-opponents powered-card attack census drifted"
    );
}

#[test]
fn every_pinned_module_was_actually_read() {
    for (path, source) in HOT_MODULES.into_iter().chain([BOUNDARY, ADMISSION]) {
        assert!(
            source.len() > 1_000,
            "{path} read as {} bytes; the include path is probably wrong",
            source.len()
        );
        assert!(
            source.contains("//!"),
            "{path} has no module documentation; the include path is probably \
             wrong"
        );
    }
}

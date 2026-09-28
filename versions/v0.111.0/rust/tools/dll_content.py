#!/usr/bin/env python3
"""DLL-sourced content facts for the `sts-sim` content codegen.

Design authority: the 2026-09-15 decision-record addendum on #1282, item
**P1**; this slice is #2496. Related: #2490 (update readiness), #1286 (the
generator this front-ends), PORT_PLAN.md §5.

Why this exists
---------------
`generate_content.py` read the Python `combat_sim` registry (since #2827 item
F, only its frozen snapshot) and emits `src/ids.rs` /
`src/content_tables.rs` from it. Under the addendum's **D3**
the Python engine is frozen at v0.111.0, and under **D4** only the crate forks
to the next build — so a forked `versions/vNEXT/rust/` re-running codegen
against a frozen registry would regenerate *v0.111.0 content*. The new build's
cards, rarities, costs, keywords and tags would silently never arrive.

This module re-sources those facts from `sts2.dll` itself, so `--source dll`
reads the build it is pointed at. Everything it cannot read is listed in
`MODELING_ANNEX` below rather than being silently taken from Python: the
boundary is a declared artifact, not an accident of which lookup happened to
be left alone.

What it reads, and how
----------------------
Static IL only — `dnfile` + `dncil`, no game boot, no harness, no pythonnet.
The ModelId inventory and the epoch list come from
`versions/v0.111.0/solver/tools/build_mcr_tables.py`, which is **build-agnostic
IL tooling parameterised by `--dll`**, not part of the Python simulator: it
reconstructs `ModelIdSerializationCache.Init` and is self-certifying, because
the `modelIdHash` it recomputes is the value the game itself writes into every
`.mcr` header. Importing it is therefore a dependency on a DLL *reader*, not
on the frozen modeling surface, and a forked crate can keep using it unchanged.

The per-card columns are read here, from each card class's own IL:

* `CardModel::.ctor` — the five named arguments
  `(canonicalEnergyCost, type, rarity, targetType, shouldShowInCardLibrary)`,
  taken from the unique base-constructor call site (the same fail-closed
  boundary `tools/extract_cards.py` and `solver/tools/census_cards.py` use);
* `get_CanonicalKeywords` — resolved along the base chain, decoded from either
  an `InitializeArray` RVA blob or the `stelem` pattern;
* `get_CanonicalTags` — the same decoding;
* `get_TargetType` — presence only: a class that overrides it has a target
  kind the constructor argument does not settle, so this module refuses those
  four cards to the annex instead of guessing.

Exactness (SOLVER_INVARIANTS.md I5)
-----------------------------------
Nothing here is approximated. A card whose constructor does not reach a unique
`CardModel::.ctor` is not admitted; an enum value with no name raises; a fact
this module does not read is named in `MODELING_ANNEX` and delegated, never
inferred. `STALE_REPO_DATA` records any place where today's committed tables
and the DLL disagree, so `--source dll` reproduces the committed bytes while
the disagreement stays visible and attributable — each entry is a defect with
its own issue, not a tolerance. It is **empty** since the three entries it
shipped with were fixed (#2500 the stale Salvo/Splash rarities, #2501 the
missing Blight Strike Strike tag).

The committed manifest (#2515)
------------------------------
Reading the assembly is local-only, and CI has to check codegen freshness
anyway. So `--source dll` does not hand its facts straight to the generator:
it writes them all to `versions/<build>/rust/data/dll_content.<build>.json` —
with the assembly's sha256, the game's own `modelIdHash`, the build and commit
from `solver/dll-archive/index.json`, and a self-hash — and generates from
that file. `--source manifest` replays it with no assembly and no `dnfile`,
which is what the `rust port` workflow runs. See the section above
`MANIFEST_SCHEMA` for why the forked crate needs this to have a real freshness
check at all.

Usage::

    python3 versions/v0.111.0/rust/tools/dll_content.py --ledger
    python3 versions/v0.111.0/rust/tools/dll_content.py --report [--dll PATH]

`--ledger` and everything on the manifest path are stdlib-only. Reading an
assembly needs `dnfile`/`dncil` — on this Mac that is `python3.12`, not the
rotating `python3` (3.14, no dnfile). The DLL archive is gitignored, so CI has
no `sts2.dll` and every assembly-bearing entry point here degrades to an
explicit skip rather than a failure.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import re
import sys

HERE = pathlib.Path(__file__).resolve()
RUST_DIR = HERE.parents[1]
BUILD_DIR = HERE.parents[2]
VERSIONS_DIR = HERE.parents[3]
REPO_ROOT = HERE.parents[4]
SOLVER_TOOLS = BUILD_DIR / "solver" / "tools"

#: The build this crate is certified against, and the exact assembly whose
#: facts the committed tables were generated from. Verified before any read.
CERTIFIED_BUILD = "v0.111.0"
CERTIFIED_DLL_SHA256 = (
    "9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4")

#: Where an archived assembly lives. `solver/dll-archive/` is gitignored, so
#: it exists only in a full local checkout — and, per its own README, only in
#: the MAIN checkout, never in a worktree. The Steam install is the last
#: fallback and is whatever build is currently installed, which stops being
#: v0.111.0 the moment the expected update lands; that is exactly why every
#: read here reports the sha256 it actually used.
ARCHIVE_DIR = REPO_ROOT / "solver" / "dll-archive"
STEAM_DLL = pathlib.Path.home() / (
    "Library/Application Support/Steam/steamapps/common/"
    "Slay the Spire 2/SlayTheSpire2.app/Contents/Resources/"
    "data_sts2_macos_arm64/sts2.dll")


def _main_checkout_archive():
    """`solver/dll-archive/` in the main checkout, seen from a worktree.

    `.git` in a worktree is a file pointing at
    `<main>/.git/worktrees/<name>`, so the common git dir's grandparent is the
    main working tree.
    """
    git_path = REPO_ROOT / ".git"
    if git_path.is_dir():
        return ARCHIVE_DIR
    try:
        pointer = git_path.read_text().split("gitdir:", 1)[1].strip()
    except (OSError, IndexError):
        return ARCHIVE_DIR
    common = pathlib.Path(pointer).resolve()
    # <main>/.git/worktrees/<name> -> <main>
    for parent in common.parents:
        if parent.name == ".git":
            return parent.parent / "solver" / "dll-archive"
    return ARCHIVE_DIR


# ---------------------------------------------------------------------------
# The declared source boundary
# ---------------------------------------------------------------------------

#: Facts `--source dll` reads out of `sts2.dll`. Key -> (DLL site, what it
#: feeds in the generated output). This is the positive half of the P1
#: contract: each of these follows the build the generator is pointed at.
DLL_SOURCED_FACTS = {
    "CardId": (
        "ModelId inventory, category CARD, admitted structurally: the class "
        "reaches a unique CardModel::.ctor. That excludes the seven Mock* "
        "test doubles and keeps DEPRECATED_CARD, which is real modeled "
        "content — no name filter is used here",
        "ids.rs CardId enum + CARD_IDS string table"),
    "RelicId": (
        "ModelId inventory, category RELIC, RELIC.-prefixed, unfiltered — "
        "the assembly declares no Mock relic, and DEPRECATED_RELIC is modeled",
        "ids.rs RelicId enum + RELIC_IDS string table"),
    "PotionId": (
        "ModelId inventory, category POTION, minus Deprecated*/Mock*",
        "ids.rs PotionId enum + POTION_IDS string table"),
    "EnchantmentId": (
        "ModelId inventory, category ENCHANTMENT, minus Deprecated*/Mock*",
        "ids.rs EnchantmentId enum + ENCHANTMENT_IDS string table"),
    "CardRarity vocabulary": (
        "the CardRarity enum's member names, lowercased",
        "content_tables.rs CardRarity enum + as_str"),
    "card.rarity": (
        "CardModel::.ctor argument `rarity`",
        "CARD_ROWS[].rarity"),
    "card.card_type": (
        "CardModel::.ctor argument `type`",
        "CARD_ROWS[].card_type and .is_status"),
    "card.target_type": (
        "CardModel::.ctor argument `targetType`, except the four classes that "
        "override get_TargetType (see MODELING_ANNEX)",
        "CARD_ROWS[].target_type"),
    "card.cost@0": (
        "CardModel::.ctor argument `canonicalEnergyCost`",
        "CARD_ROWS[].cost on upgrade-0 rows"),
    "card.keywords@0": (
        "get_CanonicalKeywords, resolved along the base chain",
        "CARD_ROWS[].exhausts/.ethereal/.innate/.retain/.sly on upgrade-0 "
        "rows, and NATIVE_UNPLAYABLE_CARD_ROWS"),
    "card.tags": (
        "get_CanonicalTags, resolved along the base chain",
        "CARD_ROWS[].tags and .strike_tag"),
    "epochs": (
        "EpochModel..cctor ldtoken'd subclasses, each get_Id's string literal",
        "the unlock-epoch universe emitted by emit_splash_epochs"),
    "monster.initial_hp": (
        "MonsterModel::get_MinInitialHp / get_MaxInitialHp, resolved along "
        "the base chain; a plain `ldc; ret`, an "
        "AscensionHelper::GetValueIfAscension(gate, atOrAbove, below) triple, "
        "or a delegation to the sibling getter",
        "content_tables.rs MONSTER_MODELS[].min_initial_hp/.max_initial_hp, "
        "which the encounters/ roster builders roll HP from"),
    "monster.initial_powers": (
        "the `Apply<XPower>` call sites of each monster class's "
        "`<AfterAddedToRoom>d__N::MoveNext`, in body order, with each "
        "amount decoded from Decimal.One, an `ldc; newobj Decimal::.ctor` "
        "pair, or a constant getter reached through Decimal::op_Implicit",
        "content_tables.rs MONSTER_MODELS[].initial_powers, which the "
        "encounters/ roster builders read spawn-time power amounts from"),
    "monster.move_constants": (
        "every other AscensionHelper::GetValueIfAscension(gate, atOrAbove, "
        "below) site of each MONSTER class along its base chain: the "
        "`get_X` getters (a pure `constant`, or an `operand` of a larger "
        "expression) and the `inline` calls in nested move bodies (#2828)",
        "content_tables.rs `move_constants::*` and every `Arg::Tier` in the "
        "move tables, joined onto the tables by generate_content."
        "MOVE_CONSTANT_SITES; the catalog selects the fight's tier"),
    "monster.ctor_ints": (
        "the `ldarg.0; ldc.i4*; stfld <own field>` prologue of each monster "
        "class's `.ctor`, before its base constructor call — the compiler's "
        "integer field initialisers",
        "content_tables.rs MONSTER_CTOR_INTS, which the encounters/ roster "
        "builders read constructor-initialised roster state from"),
    "encounter.rng_draws": (
        "every `Rng::NextInt` call in each ENCOUNTER class's own "
        "`GenerateMonsters` that draws from `EncounterModel::get_Rng`, in "
        "body order, as `[lo, hi)` — `NextInt(max)` is `[0, max)` and "
        "`NextInt(min, max)` is `[min, max)`. Only literal `ldc` bounds are "
        "read; any other argument makes the entry a refusal",
        "content_tables.rs ENCOUNTER_RNG_DRAWS, which the encounters/ roster "
        "builders read their per-fight Encounter-stream draw bounds from "
        "(PunchOffEventEncounter's StartingHpReduction rolls, #2537)"),
    "card.multiplayer_constraint": (
        "get_MultiplayerConstraint along the base chain; CardModel's own "
        "0x7c874 is `None` and 44 classes override it to MultiplayerOnly",
        "CHARACTER_CARD_POOL_ROWS_V1101 membership"),
    "card.can_be_generated_in_combat": (
        "get_CanBeGeneratedInCombat along the base chain; CardModel's own "
        "0x7cd97 is `true` and 19 classes override it to false",
        "CHARACTER_CARD_POOL_ROWS_V1101 membership"),
    "potion.rarity": (
        "each PotionModel subclass's constant get_Rarity (PotionModel's own "
        "is abstract)",
        "POTION_GENERATION_ROWS_V1101[].rarity"),
    "potion.can_be_generated_in_combat": (
        "get_CanBeGeneratedInCombat along the base chain; PotionModel's own "
        "0x83120 is `true` and three potions override it to false",
        "POTION_GENERATION_ROWS_V1101[].can_be_generated_in_combat"),
    "character card pools": (
        "<Character>CardPool::GenerateAllCards — the literal ModelDb.Card<T> "
        "array, in the RNG-significant native order — paired with the "
        "per-card unlock epoch read out of FilterThroughEpochs' "
        "IsEpochRevealed<XEpoch> chain and each XEpoch::get_Cards list",
        "content_tables.rs CHARACTER_CARD_POOL_ROWS_V1101"),
    "colorless card pool": (
        "ColorlessCardPool::GenerateAllCards 0xf11a0's literal 65-element "
        "array, in native order, paired with the per-card unlock epoch out "
        "of FilterThroughEpochs 0xf13f4's IsEpochRevealed<Colorless<N>Epoch> "
        "chain — three rows at each of the five Colorless epochs",
        "content_tables.rs COLORLESS_CARD_POOL_ROWS_V1101"),
    "shared card pools": (
        "ModelDb::get_AllSharedCardPools 0x80e58's literal array order, minus "
        "Colorless (read above), each pool's GenerateAllCards literal array "
        "in native order; none overrides FilterThroughEpochs, so no row "
        "carries an epoch (#2734)",
        "content_tables.rs SHARED_CARD_POOL_MEMBERSHIP_V1110 and "
        "SHARED_CARD_POOL_ROWS_V1110"),
    "character potion pools": (
        "<Character>PotionPool::GenerateAllPotions, literally "
        "`return <Character>4Epoch.Potions`, with GetUnlockedPotions' single "
        "whole-pool epoch gate",
        "content_tables.rs CHARACTER_POTION_POOL_ROWS_V1101"),
    "shared potion pool": (
        "SharedPotionPool::GenerateAllPotions 0xade0c's literal array, with "
        "the per-row epochs GetUnlockedPotions 0xadfb4 removes",
        "content_tables.rs SHARED_POTION_POOL_ROWS_V1101"),
    "character pool order": (
        "ModelDb::get_AllCharacters 0x80ef6's literal array order — the "
        "order UnlockState::get_CharacterCardPools 0xd994 preserves",
        "content_tables.rs CHARACTER_POOL_ORDER_V1101"),
    "character unlock epochs": (
        "the <Character><N>Epoch classes in EpochModel.AllEpochs — the same "
        "classes FilterThroughEpochs and UnlockState::get_Characters test",
        "content_tables.rs CHARACTER_UNLOCK_EPOCHS_V1101"),
    "mad science variants": (
        "TinkerTime::ChooseRiderEffect 0xd08b4's three InitializeArray "
        "RiderEffect[] blobs, one per `ChosenCardType - 1` switch arm, plus "
        "MadScience::get_TargetType 0xe482e and get_GainsBlock 0xe483c's "
        "`TinkerTimeType == k` comparisons (#2942)",
        "content_tables.rs MAD_SCIENCE_VARIANT_ROWS: the legal saved "
        "(type, rider) domain and each variant's type, target and Nimble "
        "eligibility"),
}

#: Facts the DLL cannot supply to a *static* read, with where they come from
#: instead. Nothing may be quietly retained from the Python path: adding a
#: Python fallback without an entry here is what this dict exists to prevent.
MODELING_ANNEX = {
    # --- the port's own vocabulary, which has no DLL analogue at all -------
    "StepKind / MoveKind / SelectOp / FilterMode / StepWord": (
        "the op language the port compiles content into. These are names we "
        "invented; the game has no such enum.",
        "combat_sim CARDS/TEMPLATE_CARDS step tuples and the *_MOVES tables"),
    "PowerId": (
        "combat_sim.MODELED_POWER_FIELDS is a census of modeled *state "
        "fields* (`battleworn_time_limit`), not the game's POWER models "
        "(`BATTLEWORN_DUMMY_TIME_LIMIT_POWER`); 216 modeled fields against "
        "283 POWER entries, and the mapping is not one-to-one.",
        "combat_sim.MODELED_POWER_FIELDS"),
    "card step programs, monster move tables, template-relic rules, "
    "the relic ledger's classification, dispatch trees, the refusal ledger": (
        "these are the port's modeling of behaviour read out of IL by hand, "
        "not data the assembly exposes as a table.",
        "combat_sim, unchanged"),
    # --- axes the DLL validates but does not define ------------------------
    "EncounterId": (
        "the axis is combat_sim.SUPPORTED_ENCOUNTERS == ENCOUNTER_BUILDERS "
        "keys — which encounters the engine has a roster builder for. The "
        "DLL's 98 ENCOUNTER entries are the candidate universe, and the "
        "modeled 88 use an id form that drops the class's _ELITE suffix.",
        "combat_sim.SUPPORTED_ENCOUNTERS, cross-checked against the DLL "
        "inventory by `--report`"),
    "MonsterKind": (
        "the axis is the sim's roster vocabulary (105), not the DLL's 126 "
        "MONSTER entries: it excludes Mock*/Deprecated*/unmodeled kinds and "
        "adds one aggregate (DECIMILLIPEDE_SEGMENT) the game spells as three "
        "classes (_FRONT/_MIDDLE/_BACK).",
        "combat_sim Monster(...) sites + LOOPS/_RANDOM_MOVES keys, "
        "cross-checked against the DLL inventory by `--report`"),
    # --- per-card columns a static read cannot settle ----------------------
    "card.cost and card.keywords on upgrade>=1 rows": (
        "upgrade deltas live in each card's OnUpgrade body, and the live "
        "`CardEnergyCost._base` / live CardKeywords are what change (Mad "
        "Science gains Innate only on upgrade). 60 cards change cost and 48 "
        "rows change a keyword flag. Reading them needs the harness "
        "(`tools/extract_values.py`, pythonnet), not static IL.",
        "combat_sim.CARDS upgrade-1 rows"),
    "card.playable": (
        "27 of the 31 unplayable rows carry CardKeyword.Unplayable and ARE "
        "DLL-sourced. The other four (DISINTEGRATION, MIND_ROT, SLOTH, "
        "WASTE_AWAY) are Status cards with neither the keyword nor a CanPlay "
        "override — their refusal is a native/dynamic CanPlay the generated "
        "comment on NATIVE_UNPLAYABLE_CARD_ROWS already calls out.",
        "combat_sim.CARDS[].playable"),
    "card.target_type for the four get_TargetType overriders": (
        "MAD_SCIENCE, MALAISE, SHIV and SOVEREIGN_BLADE override the getter, "
        "so the constructor argument is not the effective target kind. "
        "card_templates.py resolves three of them to Dynamic and MALAISE to a "
        "concrete kind; that resolution is an IL body analysis, not a table "
        "read, and is deliberately not reimplemented here.",
        "combat_sim._CARD_TARGET_TYPE_BY_ID"),
    "card.name, .pool, .play_condition, .selects, .heal, .turn_end_*, "
    ".nimble_eligible, .star_cost, .star_x, .x_cost, .on_draw_energy_loss, "
    ".is_status_curse": (
        "per-card numbers and flags that live in each model's DynamicVarSet "
        "or in the port's own modeling. The DynamicVar half is readable, but "
        "only through the harness (`tools/extract_values.py`), which needs a "
        "booted CLR; out of scope for a static-IL slice and tracked as "
        "follow-up on #2496.",
        "combat_sim.CARDS rows"),
    "the solo-unplayable CanPlay census, the Splash per-character epoch "
    "profiles, the potion rarity-roll thresholds": (
        "derived from the harness card/potion pool probes and from Python AST "
        "censuses of live combat_sim conditions. The *epoch universe* those "
        "profiles draw from IS DLL-sourced (see DLL_SOURCED_FACTS).",
        "combat_sim._CARD_POOL_CENSUS and the AST censuses in "
        "generate_content.py"),
    "per-slot starter-move tables (PhantasmalGardener's INIT_MOVE branch, "
    "DecimillipedeSegment's StarterMoveIdx switch)": (
        "these live in branch structure, not in a table: "
        "`PhantasmalGardener::GenerateMoveStateMachine` picks the opening "
        "MoveState from the creature's slot, and "
        "`DecimillipedeSegment::GenerateMoveStateMachine` (0xb2c04) switches "
        "on `StarterMoveIdx % 3`. Reading the branch *shape* is an IL body "
        "analysis, which is what the Python builders' constants already "
        "record; the generator lifts those constants rather than "
        "re-deriving the branch, and the numbers they interact with (HP "
        "ranges, power amounts) ARE DLL-sourced above.",
        "content/encounters/*.py module constants -> "
        "ENCOUNTER_POOL_CONSTANTS"),
    "which of a monster's spawn-time powers the sim models as roster state": (
        "`monster.initial_powers` above reads every `Apply<XPower>` the "
        "assembly performs at AfterAddedToRoom. Which of them is roster "
        "state and which is engine behaviour is the port's modeling: "
        "SkulkingColony's HardenedShellPower 20 and BygoneEffigy's "
        "SlowPower 1 are modeled elsewhere, so their builders carry no "
        "initial power at all. The generator supplies amounts; the builder "
        "names the ones its Python counterpart sets.",
        "combat_sim.Monster fields, set by content/encounters/*.py"),
    "the solo-unplayable CanPlay census, the Splash per-character epoch "
    "profiles, the potion rarity-roll thresholds": (
        "derived from the harness card/potion pool probes and from Python AST "
        "censuses of live combat_sim conditions. The *epoch universe* those "
        "profiles draw from IS DLL-sourced, and since #2542 so are the "
        "generation pools themselves (see DLL_SOURCED_FACTS) — what is left "
        "here is the AST half and the `Rng.NextFloat` thresholds, which live "
        "in generator bodies rather than in a pool.",
        "combat_sim._CARD_POOL_CENSUS / _POTION_POOL_CENSUS and the AST "
        "censuses in generate_content.py"),
    "the CardType enum's own member list": (
        "hand-written in generate_content.TABLE_TYPES with explicit native "
        "values 1..6 rather than generated, so there is no lookup to re-source"
        " in this slice. `--report` checks it still matches the DLL enum.",
        "generate_content.TABLE_TYPES"),
}

#: Places where the DLL and the committed tables disagree TODAY. `--source
#: dll` applies these so the acceptance test is a byte-for-byte reproduction
#: rather than a silent content change; each entry is a defect to fix in its
#: own PR, and deleting one must make the reproduction test fail.
#:
#: Key -> (fact, DLL value, committed value, where the committed value comes
#: from, why it is wrong).
#:
#: Empty since #2500/#2501 were fixed (2026-09-15): `data/cards.json` was
#: regenerated from the archived v0.111.0 assembly (Salvo rare->uncommon,
#: Splash uncommon->rare) and the modeled Blight Strike row grew its native
#: Strike tag, so the DLL and the committed tables now agree on every column
#: this module reads. Keep the dict — a future build bump lands here first —
#: but an entry is a defect with an issue, never a tolerance.
STALE_REPO_DATA = {}


# ---------------------------------------------------------------------------
# Errors
# ---------------------------------------------------------------------------

class DllUnavailable(Exception):
    """No readable `sts2.dll`, or no `dnfile` on this interpreter."""


class DllRefusal(Exception):
    """A fact the DLL was asked for but cannot supply exactly (I5)."""


# ---------------------------------------------------------------------------
# Locating and verifying the assembly
# ---------------------------------------------------------------------------

def archived_certified_dll():
    """The archived `CERTIFIED_BUILD` assembly, or `None` if it is not here.

    Deliberately narrower than `default_dll_path()`: no `$STS2_DLL`, no Steam
    install. It answers "is the build this crate is certified against
    available on this host", which is what decides whether an unqualified
    codegen run may read the assembly (`generate_content.py --source auto`).
    The install cannot stand in for it — Steam updates in place, so the
    installed DLL stops being v0.111.0 the moment the next build ships.
    """
    for archive in (ARCHIVE_DIR, _main_checkout_archive()):
        archived = (archive / CERTIFIED_BUILD / "data_sts2_macos_arm64"
                    / "sts2.dll")
        if archived.exists():
            return archived
    return None


def default_dll_path():
    """The archived certified build, else `$STS2_DLL`, else the install.

    The archive is preferred over the Steam install on purpose: the acceptance
    test has to keep targeting v0.111.0 after the game updates in place.
    """
    archived = archived_certified_dll()
    if archived is not None:
        return archived
    env = os.environ.get("STS2_DLL")
    if env and pathlib.Path(env).exists():
        return pathlib.Path(env)
    if STEAM_DLL.exists():
        return STEAM_DLL
    return None


def sha256_of(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _import_mcr_tables():
    """Import the build-agnostic ModelId/epoch IL reader.

    Kept as a function so `--ledger` and the annex are readable on an
    interpreter with no `dnfile` (CI's python3 today).
    """
    if str(SOLVER_TOOLS) not in sys.path:
        sys.path.insert(0, str(SOLVER_TOOLS))
    try:
        import build_mcr_tables  # noqa: E402  (path set up above)
    except ImportError as exc:  # pragma: no cover - environment dependent
        raise DllUnavailable(
            f"cannot import the IL reader ({exc}); install dnfile/dncil and "
            "run this under python3.12 (the default python3 on this Mac is "
            "3.14 without dnfile)") from exc
    return build_mcr_tables


# ---------------------------------------------------------------------------
# The facts
# ---------------------------------------------------------------------------

#: Entry-name prefixes the game uses for its own test doubles and retired
#: content.
#:
#: The admission rule is NOT uniform across categories, and pretending it were
#: would be wrong in both directions. Measured on v0.111.0:
#:
#: * CARD   — structural rule only (see `card_ids`). `DEPRECATED_CARD` is
#:            real modeled content; the seven `Mock*` cards are not.
#: * RELIC  — no filter at all. The assembly declares no mock relic, and
#:            `RELIC.DEPRECATED_RELIC` is in `combat_sim.KNOWN_RELICS`.
#: * POTION, ENCHANTMENT — both prefixes excluded, which is what the modeled
#:            registries do.
#:
#: Every one of these is reconciled against the Python axis by
#: `DllContentSource`, so a build that changes the convention fails loudly
#: instead of quietly gaining or losing an enum member.
_EXCLUDED_PREFIXES = ("MOCK_", "DEPRECATED_")


def _strip_elite(entry):
    """`BYRDONIS_ELITE` -> `BYRDONIS`; everything else unchanged."""
    return entry[:-len("_ELITE")] if entry.endswith("_ELITE") else entry


class DllFacts:
    """Every DLL-sourced fact for one assembly, read once."""

    def __init__(self, dll_path=None, *, require_certified=False):
        path = pathlib.Path(dll_path) if dll_path else default_dll_path()
        if path is None or not path.exists():
            raise DllUnavailable(
                "no sts2.dll found: solver/dll-archive/ is gitignored and no "
                "Steam install or $STS2_DLL is present")
        mcr = _import_mcr_tables()
        self.path = path
        self.sha256 = sha256_of(path)
        if require_certified and self.sha256 != CERTIFIED_DLL_SHA256:
            raise DllUnavailable(
                f"{path} has sha256 {self.sha256}, not the certified "
                f"{CERTIFIED_BUILD} assembly {CERTIFIED_DLL_SHA256}")
        self._mcr = mcr
        self._dll = mcr.Dll(str(path))
        self._items, _collapsed = mcr.model_items(self._dll)
        self._field_rva = {
            row.Field.row_index: row.Rva
            for row in self._dll.md.FieldRva.rows}
        self._enums = {}
        self._rid_by_entry = {}
        self._inventory = {}
        for category, entry, rid, _full_name in self._items:
            self._inventory.setdefault(category, []).append(entry)
            self._rid_by_entry[(category, entry)] = rid
        self._cards = None

    # -- provenance --------------------------------------------------------

    @property
    def provenance(self):
        return {"dll": str(self.path), "sha256": self.sha256}

    def model_id_hash(self):
        """The game's own `modelIdHash`, recomputed from this assembly.

        Every `.mcr` header carries this value, so a captured replay is an
        external witness that the inventory below was reconstructed exactly.
        """
        chunks = []
        seen_categories, seen_entries = {"NONE"}, {"NONE"}
        for category, entry, _rid, _fn in self._items:
            seen_categories.add(category)
            seen_entries.add(entry)
            chunks.append(category.encode())
            chunks.append(entry.encode())
        seen_props = set()
        for _cat, _ent, rid, _fn in self._items:
            for _order, name in self._dll.saved_properties(rid):
                if name not in seen_props:
                    seen_props.add(name)
                    chunks.append(name.encode())
        seen_epochs = set()
        for epoch_id, _fn in self._mcr.epoch_items(self._dll):
            seen_epochs.add(epoch_id)
            chunks.append(epoch_id.encode())
        return self._mcr.xxhash32(chunks)

    # -- enums -------------------------------------------------------------

    def enum(self, name):
        if name not in self._enums:
            self._enums[name] = self._dll.enum_table(name)[0]
        return self._enums[name]

    def _enum_name(self, enum_name, value, site):
        table = self.enum(enum_name)
        if value not in table:
            raise DllRefusal(
                f"{site}: {enum_name} has no member with value {value!r}")
        return table[value]

    # -- the ModelId inventory --------------------------------------------

    def inventory(self, category):
        """Every entry slug in one ModelId category, ascending."""
        if category not in self._inventory:
            raise DllRefusal(f"the assembly has no ModelId category "
                             f"{category!r}")
        return sorted(self._inventory[category])

    def _admitted(self, category):
        return [entry for entry in self.inventory(category)
                if not entry.startswith(_EXCLUDED_PREFIXES)]

    def card_ids(self):
        """The 596 real card ids, by the assembly's own structural rule.

        A card is admitted when its class reaches a unique
        `CardModel::.ctor` — which is what makes it constructible content
        rather than a test double. No name heuristic is involved, and none
        would be right: the seven `Mock*` classes never call the base
        constructor, while `DEPRECATED_CARD` does and **is** a modeled card
        (it is in `combat_sim.CARDS`). That asymmetry is measured on
        v0.111.0, not assumed — the name filter the other categories use is
        deliberately not applied here.
        """
        return sorted(self._card_facts())

    def relic_ids(self):
        return ["RELIC." + entry for entry in self.inventory("RELIC")]

    def potion_ids(self):
        return self._admitted("POTION")

    def enchantment_ids(self):
        return self._admitted("ENCHANTMENT")

    def encounter_inventory(self):
        return self._admitted("ENCOUNTER")

    def encounter_spellings(self):
        """Every spelling of an ENCOUNTER entry the sim's axis may use.

        The sim usually drops the class's `_ELITE` suffix (`BYRDONIS`, not
        `BYRDONIS_ELITE`) but not always — `KNIGHTS_ELITE` and
        `MECHA_KNIGHT_ELITE` keep it. Both spellings are therefore accepted:
        this is a naming bridge for the annex cross-check only.
        `EncounterId` itself is modeled, not DLL-sourced (see
        MODELING_ANNEX), so the check that matters is one-directional — every
        modeled id must have an entry behind it.
        """
        spellings = set()
        for entry in self.encounter_inventory():
            spellings.add(entry)
            spellings.add(_strip_elite(entry))
        return spellings

    def monster_inventory(self):
        return self._admitted("MONSTER")

    def epochs(self):
        """`EpochModel.AllEpochs` — the closed unlock-epoch universe."""
        return sorted({epoch_id
                       for epoch_id, _fn in self._mcr.epoch_items(self._dll)})

    # -- IL helpers --------------------------------------------------------

    @staticmethod
    def _ldc(instruction):
        name = instruction.opcode.name
        if not name.startswith("ldc.i4"):
            return None
        if instruction.operand is not None:
            return int(instruction.operand)
        if name == "ldc.i4.m1":
            return -1
        if name[-1].isdigit():
            return int(name[-1])
        return None

    def _body(self, method_row):
        if not method_row.Rva:
            return None
        offset = self._dll.pe.get_offset_from_rva(method_row.Rva)
        return self._mcr.CilMethodBody(self._mcr.RawReader(self._dll.raw,
                                                           offset))

    def _base_chain(self, rid):
        chain, cursor = [], rid
        while cursor is not None:
            chain.append(cursor)
            cursor = self._dll.base_typedef_rid(cursor)
        return chain

    def _declared_method(self, rid, name):
        """(row, declaring type name) for the nearest declaration, or None."""
        for type_rid in self._base_chain(rid):
            typedef = self._dll.typedefs[type_rid - 1]
            for method in typedef.MethodList:
                if str(method.row.Name) == name and method.row.Rva:
                    return method.row, str(typedef.TypeName)
        return None

    def _overrides(self, rid, name):
        typedef = self._dll.typedefs[rid - 1]
        return any(str(method.row.Name) == name
                   for method in typedef.MethodList)

    def _enum_array(self, method_row, site):
        """Decode a `CardKeyword[]` / `CardTag[]` returning body.

        Three shapes occur: an `InitializeArray` RVA blob, the
        `ldc.i4 <idx>; ldc.i4 <val>; stelem` pattern, and a body with no
        `newarr` at all (the values are handed straight to a collection).
        """
        body = self._body(method_row)
        if body is None:
            raise DllRefusal(f"{site}: method has no readable body")
        instructions = list(body.instructions)
        if not any(i.opcode.name == "newarr" for i in instructions):
            return [v for v in (self._ldc(i) for i in instructions)
                    if v is not None]
        expected = 0
        for index, instruction in enumerate(instructions):
            if (instruction.opcode.name.startswith("ldc.i4")
                    and index + 1 < len(instructions)
                    and instructions[index + 1].opcode.name == "newarr"):
                expected = self._ldc(instruction)
            if instruction.opcode.name == "ldtoken" \
                    and instruction.operand is not None:
                token = instruction.operand.value
                if (token >> 24) & 0xFF != 0x04:  # not a Field token
                    continue
                rva = self._field_rva.get(token & 0xFFFFFF)
                if not rva or not expected:
                    continue
                offset = self._dll.pe.get_offset_from_rva(rva)
                return [int.from_bytes(
                    self._dll.raw[offset + 4 * i:offset + 4 * i + 4], "little")
                    for i in range(expected)]
        values = []
        for index, instruction in enumerate(instructions):
            if instruction.opcode.name.startswith("stelem") and index >= 2:
                value = self._ldc(instructions[index - 1])
                if value is not None:
                    values.append(value)
        return values

    # -- per-card facts ----------------------------------------------------

    #: The `CardModel` constructor's parameter names, in order. Pinned rather
    #: than assumed: the arguments are read positionally, so a reordered or
    #: extended base constructor must fail loudly instead of shifting rarity
    #: into targetType.
    CARD_CTOR_PARAMS = ("canonicalEnergyCost", "type", "rarity", "targetType",
                        "shouldShowInCardLibrary")

    def _card_facts(self):
        if self._cards is None:
            self._cards = {}
            for category, entry, rid, _fn in self._items:
                if category != "CARD":
                    continue
                facts = self._read_card(entry, rid)
                if facts is not None:
                    self._cards[entry] = facts
        return self._cards

    def _method_token_owner(self):
        if not hasattr(self, "_token_owner_cache"):
            md = self._dll.md
            by_rid = {}
            for index, typedef in enumerate(self._dll.typedefs):
                for method in typedef.MethodList:
                    by_rid[method.row_index] = index
            memberref = {}
            for index, row in enumerate(md.MemberRef.rows, start=1):
                cls = row.Class
                parent = "?"
                if cls.table and cls.table.name == "TypeRef":
                    parent = str(md.TypeRef.rows[cls.row_index - 1].TypeName)
                elif cls.table and cls.table.name == "TypeDef":
                    parent = str(self._dll.typedefs[cls.row_index - 1].TypeName)
                memberref[index] = (parent, str(row.Name))
            self._token_owner_cache = (by_rid, memberref)
        return self._token_owner_cache

    def _call_target(self, operand):
        """(declaring type name, method name, MethodDef rid or None)."""
        value = getattr(operand, "value", None)
        if value is None:
            return None
        by_rid, memberref = self._method_token_owner()
        table, rid = (value >> 24) & 0xFF, value & 0xFFFFFF
        if table == 0x06:  # MethodDef
            owner = by_rid.get(rid)
            name = str(self._dll.md.MethodDef.rows[rid - 1].Name)
            typename = (str(self._dll.typedefs[owner].TypeName)
                        if owner is not None else "?")
            return typename, name, rid
        if table == 0x0A:  # MemberRef
            typename, name = memberref.get(rid, ("?", "?"))
            return typename, name, None
        return None

    def _read_card(self, entry, rid):
        """Read one card class, or return None when it is not a real card."""
        typedef = self._dll.typedefs[rid - 1]
        ctor = next((m for m in typedef.MethodList
                     if str(m.row.Name) == ".ctor" and m.row.Rva), None)
        if ctor is None:
            return None
        body = self._body(ctor.row)
        if body is None:
            return None
        instructions = list(body.instructions)
        base_calls = [
            index for index, instruction in enumerate(instructions)
            if instruction.opcode.name in ("call", "callvirt")
            and (self._call_target(instruction.operand) or ("", "", None))[:2]
            == ("CardModel", ".ctor")]
        if len(base_calls) != 1:
            # Not a card the game constructs through CardModel — the Mock*
            # test cards. Excluded, not approximated.
            return None
        base_rid = self._call_target(instructions[base_calls[0]].operand)[2]
        params = tuple(
            str(p.row.Name)
            for p in self._dll.md.MethodDef.rows[base_rid - 1].ParamList)
        if params != self.CARD_CTOR_PARAMS:
            raise DllRefusal(
                f"card:{entry}: CardModel::.ctor parameters are {params!r}, "
                f"not the pinned {self.CARD_CTOR_PARAMS!r}; the positional "
                "read below is no longer valid")
        constants = [value for value in
                     (self._ldc(i) for i in instructions[:base_calls[0]])
                     if value is not None]
        if len(constants) < len(params):
            raise DllRefusal(
                f"card:{entry}: only {len(constants)} constants before the "
                f"CardModel::.ctor call, need {len(params)}")
        cost, type_value, rarity_value, target_value, _show = \
            constants[-len(params):]

        keyword_method = self._declared_method(rid, "get_CanonicalKeywords")
        keywords = frozenset()
        if keyword_method is not None:
            keywords = frozenset(
                self._enum_name("CardKeyword", value, f"card:{entry}.keywords")
                for value in self._enum_array(
                    keyword_method[0], f"card:{entry}.keywords")) - {"None"}
        tag_method = self._declared_method(rid, "get_CanonicalTags")
        tags = frozenset()
        if tag_method is not None:
            tags = frozenset(
                self._enum_name("CardTag", value, f"card:{entry}.tags")
                for value in self._enum_array(
                    tag_method[0], f"card:{entry}.tags")) - {"None"}

        return {
            "cost": cost,
            "type": self._enum_name("CardType", type_value,
                                    f"card:{entry}.type").lower(),
            "rarity": self._enum_name("CardRarity", rarity_value,
                                      f"card:{entry}.rarity").lower(),
            "target_type": self._enum_name("TargetType", target_value,
                                           f"card:{entry}.target_type"),
            "dynamic_target": self._overrides(rid, "get_TargetType"),
            "keywords": keywords,
            "tags": tuple(sorted(tags)),
            "multiplayer_constraint": self._enum_name(
                "CardMultiplayerConstraint",
                self._constant_int_getter(rid, "get_MultiplayerConstraint",
                                      f"card:{entry}"),
                f"card:{entry}.multiplayer_constraint"),
            "can_be_generated_in_combat": bool(self._constant_int_getter(
                rid, "get_CanBeGeneratedInCombat", f"card:{entry}")),
        }

    def _constant_int_getter(self, rid, name, site):
        """The single `ldc.i4` a constant property getter returns.

        Resolved along the base chain, so a class that does not override the
        property answers with `CardModel`'s / `PotionModel`'s own constant
        (`get_MultiplayerConstraint` `0x7c874` → `None`,
        `CardModel::get_CanBeGeneratedInCombat` `0x7cd97` → `true`,
        `PotionModel::get_CanBeGeneratedInCombat` `0x83120` → `true`). Every
        v0.111.0 declaration of these three is `ldc.i4.<n>; ret`; anything
        else — a computed body, a field read — is refused rather than guessed
        (I5), because a non-constant value would depend on live state this
        static read cannot see.
        """
        declared = self._declared_method(rid, name)
        if declared is None:
            raise DllRefusal(f"{site}: no {name} along the base chain")
        body = self._body(declared[0])
        if body is None:  # pragma: no cover - a declared method has a body
            raise DllRefusal(f"{site}: {name} has no readable body")
        instructions = list(body.instructions)
        value = self._ldc(instructions[0]) if instructions else None
        if (len(instructions) != 2 or value is None
                or instructions[1].opcode.name != "ret"):
            raise DllRefusal(
                f"{site}: {declared[1]}::{name} is not a constant getter "
                f"({[i.opcode.name for i in instructions][:6]!r}); its value "
                "depends on live state a static read cannot settle")
        return value

    def card(self, card_id):
        facts = self._card_facts().get(card_id)
        if facts is None:
            raise DllRefusal(
                f"card:{card_id} is not a constructible card in "
                f"{self.path.name} — the assembly has no such CARD ModelId, "
                "or its class does not reach CardModel::.ctor")
        return facts

    def card_type_members(self):
        """`CardType`'s native `(value, name)` pairs, `None` excluded.

        The generated `CardType` enum is hand-written with explicit native
        values rather than generated (see MODELING_ANNEX), so this exists to
        check that hand-written list rather than to feed it.
        """
        return sorted((value, name)
                      for value, name in self.enum("CardType").items()
                      if name != "None")

    def card_rarity_vocabulary(self):
        """The rarity names the admitted cards actually use, ascending."""
        used = {self.card(card_id)["rarity"] for card_id in self.card_ids()}
        known = {name.lower() for name in self.enum("CardRarity").values()}
        unknown = used - known
        if unknown:  # pragma: no cover - structurally impossible
            raise DllRefusal(f"card rarities outside the enum: {unknown}")
        return sorted(used)

    def native_unplayable_keys(self):
        """`(card_id, 0)` for every card whose canonical keywords carry
        `CardKeyword.Unplayable`.

        Upgrade level 0 only, and that is exact rather than a simplification:
        the keyword is canonical (declared on the model), while an upgraded
        row's keywords come from `OnUpgrade`, which this static read does not
        follow. Today every native-Unplayable row is an upgrade-0 row.
        """
        return [(card_id, 0) for card_id in self.card_ids()
                if "Unplayable" in self.card(card_id)["keywords"]]

    # -- per-monster model facts -------------------------------------------

    #: `AscensionHelper::GetValueIfAscension(ascension, atOrAbove, below)` is
    #: the single shape every ascension-tiered monster constant in this
    #: assembly uses. Pinned rather than assumed: the three arguments are read
    #: positionally, so a reordered helper must fail loudly instead of
    #: swapping the two tiers.
    ASCENSION_HELPER = ("AscensionHelper", "GetValueIfAscension")

    #: The state-machine method whose `Apply<XPower>` calls are a creature's
    #: spawn-time powers. Async, so the body lives in the nested
    #: `<AfterAddedToRoom>d__N::MoveNext` (the NestedClass table is the only
    #: way there; `dump_type.py` cannot reach it).
    SPAWN_HOOK = "AfterAddedToRoom"

    @staticmethod
    def _plain(value):
        """A constant with no ascension gate, in the tiered shape."""
        return {"gate": None, "at_or_above": value, "below": value}

    def _nested_state_machine(self, rid, hook):
        """The `<hook>d__N` TypeDef rid for the body `rid` actually runs.

        The async body is nested in the class that *declares* the hook, so the
        base chain is walked: the three `DecimillipedeSegment*` subclasses
        declare no `AfterAddedToRoom` of their own and inherit the base's,
        which is where their `ReattachPower` is applied.
        """
        pattern = re.compile(rf"^<{re.escape(hook)}>d__\d+$")
        for type_rid in self._base_chain(rid):
            owner = type_rid - 1
            found = [index for index, enclosing in self._dll.enclosing.items()
                     if enclosing == owner
                     and pattern.match(
                         str(self._dll.typedefs[index].TypeName))]
            if len(found) == 1:
                return found[0] + 1
            if found:
                raise DllRefusal(
                    f"{self._dll.typedefs[owner].TypeName}: {len(found)} "
                    f"nested <{hook}>d__N state machines")
        return None

    def _instructions(self, method_row):
        body = self._body(method_row)
        return [] if body is None else list(body.instructions)

    @staticmethod
    def _compressed(data, offset):
        """ECMA-335 II.23.2 compressed unsigned integer at `offset`."""
        if offset >= len(data):
            return None
        head = data[offset]
        if head < 0x80:
            return head
        if (head & 0xC0) == 0x80 and offset + 1 < len(data):
            return ((head & 0x3F) << 8) | data[offset + 1]
        if (head & 0xE0) == 0xC0 and offset + 3 < len(data):
            return (((head & 0x1F) << 24) | (data[offset + 1] << 16)
                    | (data[offset + 2] << 8) | data[offset + 3])
        return None

    def _generic_argument(self, operand):
        """The single type argument of a `MethodSpec` call, or `None`.

        `Apply<TerritorialPower>` is a generic method instantiation, so the
        power's class name is in the MethodSpec blob rather than in any
        MethodDef the other resolvers reach.
        """
        value = getattr(operand, "value", None)
        if value is None or (value >> 24) & 0xFF != 0x2B:  # not a MethodSpec
            return None
        row = self._dll.md.MethodSpec.rows[(value & 0xFFFFFF) - 1]
        if str(getattr(row.Method.row, "Name", "")) != "Apply":
            return None
        blob = row.Instantiation
        data = bytes(blob.value if hasattr(blob, "value") else blob)
        # GENERICINST, arity, then one CLASS/VALUETYPE coded TypeDefOrRef.
        if len(data) < 4 or data[0] != 0x0A or data[1] != 1 or \
                data[2] not in (0x11, 0x12):
            return None
        coded = self._compressed(data, 3)
        if coded is None:
            return None
        table, index = coded & 0x03, coded >> 2
        if not index:
            return None
        if table == 0:
            return str(self._dll.typedefs[index - 1].TypeName)
        if table == 1:
            return str(self._dll.md.TypeRef.rows[index - 1].TypeName)
        return None

    #: How many `ldarg.0; call <getter>; ret` hops `_constant_getter`
    #: follows before refusing; see its docstring.
    MAX_GETTER_DELEGATION = 2

    def _constant_getter(self, rid, name, site, _depth=0):
        """Decode a zero-argument int getter into the tiered shape.

        Exactly three bodies are admitted, and everything else is refused
        rather than approximated (I5):

        * `ldc <v>; ret` — an untiered constant;
        * `ldc <gate>; ldc <atOrAbove>; ldc <below>; call
          AscensionHelper::GetValueIfAscension; ret`;
        * `ldarg.0; callvirt <sibling getter>; ret` — a delegation, followed
          at most twice (`get_MaxInitialHp` delegating to `get_MinInitialHp`
          is how this assembly spells a fixed-HP monster).

        Why twice and not once (#2535): `TestSubject::get_MinInitialHp`
        (RVA `0xc0136`) is itself `ldarg.0; call
        TestSubject::get_FirstFormHp; ret`, and `get_FirstFormHp` (`0xc0146`)
        is the `GetValueIfAscension(8, 111, 100)` triple, so its
        `get_MaxInitialHp` (`0xc013e`, `callvirt
        MonsterModel::get_MinInitialHp`) reaches the constant in two hops.
        Every hop is still one of the three admitted bodies; the bound only
        keeps a cycle from recursing.
        """
        declared = self._declared_method(rid, name)
        if declared is None:
            raise DllRefusal(f"{site}: no {name} on the base chain")
        instructions = self._instructions(declared[0])
        opcodes = [i.opcode.name for i in instructions]
        if opcodes[-1:] != ["ret"]:
            raise DllRefusal(f"{site}: {name} has no readable body")
        values = [self._ldc(i) for i in instructions[:-1]]
        if len(values) == 1 and values[0] is not None:
            return self._plain(values[0])
        if (len(values) == 4 and values[3] is None
                and opcodes[3] in ("call", "callvirt")
                and None not in values[:3]):
            target = self._call_target(instructions[3].operand)
            if target and target[:2] == self.ASCENSION_HELPER:
                return {"gate": values[0], "at_or_above": values[1],
                        "below": values[2]}
            raise DllRefusal(
                f"{site}: {name} ends in a three-argument call to "
                f"{target[0] if target else '?'}::"
                f"{target[1] if target else '?'}, not the pinned "
                f"{self.ASCENSION_HELPER[0]}::{self.ASCENSION_HELPER[1]}")
        if (_depth < self.MAX_GETTER_DELEGATION and len(values) == 2
                and opcodes[0] in ("ldarg.0", "ldarg")
                and opcodes[1] in ("call", "callvirt")):
            target = self._call_target(instructions[1].operand)
            if target and target[1].startswith("get_"):
                return self._constant_getter(rid, target[1], site, _depth + 1)
        try:
            return self._evaluated_getter(rid, name, site)
        except DllRefusal as exc:
            raise DllRefusal(
                f"{site}: {name} is not a constant getter — its body is "
                f"{opcodes}; as a straight-line getter it refuses: "
                f"{exc}") from None

    #: How deep `_evaluated_getter` follows sibling getters. Axebot's HP chain
    #: (`get_MinInitialHp` -> `get_RespawnMaxHpBonus` -> `get_RespawnCount`
    #: -> `get_StockAmount`) is four levels; anything deeper refuses.
    EVALUATED_GETTER_DEPTH = 6

    def _evaluated_getter(self, rid, name, site, _seen=()):
        """Evaluate a straight-line int getter on a freshly constructed model.

        The fallback behind the three constant shapes `_constant_getter`
        admits (#2534). Axebot is the case it exists for:
        `Axebot::get_MinInitialHp` (`0xaed95`) is `GetValueIfAscension(8, 76,
        70) + get_RespawnMaxHpBonus()`, and that bonus bottoms out in
        `get_StockAmount` (`0xaedd3`), `_stockOverrideAmount
        .GetValueOrDefault(2)` — a value that is a constant exactly when the
        override field is unset.

        So this claims the value **a model reports before anything has set
        one of its nullable override fields**, and admits nothing it cannot
        evaluate exactly (I5). The body must be branch-free and built from:

        * `ldc.i4*` — a plain constant;
        * `ldc gate; ldc atOrAbove; ldc below; call
          AscensionHelper::GetValueIfAscension` over three plain constants;
        * `ldarg.0; call|callvirt get_X` — a sibling getter declared on this
          model's base chain, evaluated recursively (cycle- and
          depth-guarded);
        * `ldarg.0; ldflda F; ldc v; call GetValueOrDefault` — an unset
          `Nullable<int>` field reads its default `v`. Admitted only when no
          `.ctor` on the base chain mentions `F` at all, so a field
          initializer (which compiles into the constructor) cannot hide;
        * `add`, `sub`, `mul` over two values whose ascension gates agree
          (or where one side is ungated) — applied tier-wise, which is exact
          because a tiered constant combined with a plain one is still one
          `GetValueIfAscension` of the combined pair;
        * a final `ret` with exactly one value on the stack.

        Whether the unset-field premise holds for a particular *encounter*
        is not a fact about the model, so it is not settled here: the roster
        builder that spends the value cites the encounter's
        `GenerateMonsters` IL showing the setter is never called
        (`encounters::normal_b::build_axebots_normal`).
        """
        if name in _seen or len(_seen) >= self.EVALUATED_GETTER_DEPTH:
            raise DllRefusal(f"{site}: {name} recurses past {list(_seen)}")
        declared = self._declared_method(rid, name)
        if declared is None:
            raise DllRefusal(f"{site}: no {name} on the base chain")
        instructions = self._instructions(declared[0])
        chain_names = {str(self._dll.typedefs[r - 1].TypeName)
                       for r in self._base_chain(rid)}
        stack = []
        index = 0
        while index < len(instructions):
            instruction = instructions[index]
            opcode = instruction.opcode.name
            literal = self._ldc(instruction)
            if literal is not None:
                stack.append(self._plain(literal))
                index += 1
                continue
            following = instructions[index + 1:index + 4]
            if opcode == "ldarg.0" and following:
                step = following[0].opcode.name
                if step in ("call", "callvirt"):
                    target = self._call_target(following[0].operand)
                    if (target and target[1].startswith("get_")
                            and target[0] in chain_names):
                        stack.append(self._evaluated_getter(
                            rid, target[1], site, _seen + (name,)))
                        index += 2
                        continue
                if (step == "ldflda" and len(following) == 3
                        and self._ldc(following[1]) is not None
                        and following[2].opcode.name in ("call", "callvirt")):
                    target = self._call_target(following[2].operand)
                    if target and target[1] == "GetValueOrDefault":
                        self._require_unset_field(
                            rid, following[0].operand, site, name)
                        stack.append(self._plain(self._ldc(following[1])))
                        index += 4
                        continue
                raise DllRefusal(
                    f"{site}: {name} reads instance state at IL index "
                    f"{index} that is not a sibling getter or an unset "
                    "nullable override")
            if opcode in ("call", "callvirt"):
                target = self._call_target(instruction.operand)
                if (target and target[:2] == self.ASCENSION_HELPER
                        and len(stack) >= 3
                        and all(v["gate"] is None for v in stack[-3:])):
                    gate, at_or_above, below = (
                        v["at_or_above"] for v in stack[-3:])
                    del stack[-3:]
                    stack.append({"gate": gate, "at_or_above": at_or_above,
                                  "below": below})
                    index += 1
                    continue
                raise DllRefusal(
                    f"{site}: {name} calls "
                    f"{target[0] if target else '?'}::"
                    f"{target[1] if target else '?'}")
            if opcode in ("add", "sub", "mul") and len(stack) >= 2:
                right = stack.pop()
                left = stack.pop()
                stack.append(self._tiered_arithmetic(
                    opcode, left, right, f"{site}:{name}"))
                index += 1
                continue
            if (opcode == "ret" and index == len(instructions) - 1
                    and len(stack) == 1):
                return stack[0]
            raise DllRefusal(
                f"{site}: {name} has `{opcode}` at IL index {index}, outside "
                "the straight-line getter vocabulary")
        raise DllRefusal(f"{site}: {name} has no final `ret`")

    @staticmethod
    def _tiered_arithmetic(opcode, left, right, site):
        """`left <opcode> right` on two tiered values, tier by tier."""
        gates = {left["gate"], right["gate"]} - {None}
        if len(gates) > 1:
            raise DllRefusal(
                f"{site}: `{opcode}` combines values gated at "
                f"{sorted(gates)}, which is not one tiered constant")
        apply = {"add": lambda a, b: a + b,
                 "sub": lambda a, b: a - b,
                 "mul": lambda a, b: a * b}[opcode]
        return {"gate": next(iter(gates), None),
                "at_or_above": apply(left["at_or_above"],
                                     right["at_or_above"]),
                "below": apply(left["below"], right["below"])}

    def _require_unset_field(self, rid, field, site, name):
        """Refuse unless no `.ctor` on the base chain touches `field`."""
        token = getattr(field, "value", None)
        if token is None:
            raise DllRefusal(f"{site}: {name} reads an unresolved field")
        for type_rid in self._base_chain(rid):
            typedef = self._dll.typedefs[type_rid - 1]
            for method in typedef.MethodList:
                if str(method.row.Name) != ".ctor" or not method.row.Rva:
                    continue
                for instruction in self._instructions(method.row):
                    if getattr(instruction.operand, "value", None) == token:
                        raise DllRefusal(
                            f"{site}: {name} reads a field "
                            f"{typedef.TypeName}::.ctor initialises")

    def _spawn_powers(self, rid, site):
        """`(power class, amount)` for each spawn-time `Apply<XPower>`.

        Body order is the application order, which is the order the powers
        become roster state. Four amount spellings occur and nothing else is
        admitted: `Decimal.One`, an `ldc; newobj Decimal::.ctor` pair, a
        constant getter widened through `Decimal::op_Implicit`, and a local
        holding one `GetValueIfAscension` tier widened the same way
        (`_tiered_local`, #2534).
        """
        nested = self._nested_state_machine(rid, self.SPAWN_HOOK)
        if nested is None:
            return []
        typedef = self._dll.typedefs[nested - 1]
        move_next = next((m for m in typedef.MethodList
                          if str(m.row.Name) == "MoveNext" and m.row.Rva),
                         None)
        if move_next is None:
            return []
        instructions = self._instructions(move_next.row)
        powers = []
        for index, instruction in enumerate(instructions):
            power = self._generic_argument(instruction.operand)
            if power is None:
                continue
            powers.append([power, self._spawn_amount(rid, instructions,
                                                     index, power, site)])
        return powers

    def _spawn_amount(self, rid, instructions, call_index, power, site):
        """The `Decimal` amount pushed for one `Apply<XPower>` call site."""
        window = instructions[max(0, call_index - 8):call_index]
        for index, instruction in enumerate(window):
            name = instruction.opcode.name
            if name == "ldsfld":
                target = self._call_target(instruction.operand)
                if target and target[:2] == ("Decimal", "One"):
                    return self._plain(1)
            if name == "newobj" and index:
                target = self._call_target(instruction.operand)
                literal = self._ldc(window[index - 1])
                if (target and target[0] == "Decimal"
                        and target[1] == ".ctor" and literal is not None):
                    return self._plain(literal)
            if name in ("call", "callvirt"):
                target = self._call_target(instruction.operand)
                if target and target[1] == "op_Implicit" and index:
                    getter = self._call_target(window[index - 1].operand)
                    if getter and getter[1].startswith("get_"):
                        return self._constant_getter(
                            rid, getter[1], f"{site}:{power}")
                    local = self._local_index(window[index - 1], "ldloc")
                    if local is not None:
                        return self._tiered_local(
                            instructions, local, f"{site}:{power}")
        raise DllRefusal(
            f"{site}: the Apply<{power}> amount is not one of the four "
            "admitted Decimal spellings")

    @staticmethod
    def _local_index(instruction, family):
        """The local slot of a `stloc*`/`ldloc*` (never `ldloca`), or None."""
        name = instruction.opcode.name
        if name in (f"{family}.0", f"{family}.1", f"{family}.2",
                    f"{family}.3"):
            return int(name[-1])
        if name in (family, f"{family}.s"):
            index = getattr(instruction.operand, "index", None)
            return index if isinstance(index, int) else None
        return None

    def _tiered_local(self, instructions, local, site):
        """The fourth `Apply<XPower>` amount spelling (#2534).

        `SewerClam/<AfterAddedToRoom>d__9::MoveNext` (`0x368f9c`) computes the
        amount inline — `ldc.i4.8; ldc.i4.s 9; ldc.i4.8; call
        AscensionHelper::GetValueIfAscension; stloc.2` (IL_007f-IL_0088) — and
        widens the LOCAL through `Decimal::op_Implicit` (IL_0094-IL_0095).
        Admitted only when the local has exactly one store in the whole body
        and that store is immediately that four-instruction tier, so no
        control-flow path can reach the `ldloc` holding anything else.
        """
        stores = [index for index, instruction in enumerate(instructions)
                  if self._local_index(instruction, "stloc") == local]
        if len(stores) == 1 and stores[0] >= 4:
            head = instructions[stores[0] - 4:stores[0]]
            values = [self._ldc(i) for i in head[:3]]
            target = (self._call_target(head[3].operand)
                      if head[3].opcode.name in ("call", "callvirt")
                      else None)
            if (None not in values and target
                    and target[:2] == self.ASCENSION_HELPER):
                return {"gate": values[0], "at_or_above": values[1],
                        "below": values[2]}
        raise DllRefusal(
            f"{site}: the widened local {local} is not stored exactly once "
            f"from a GetValueIfAscension tier ({len(stores)} store(s))")

    def monster_models(self):
        """Per-`MONSTER` initial HP and spawn-time powers, by ModelId entry.

        Keyed by the assembly's own MONSTER entry slug, not by the sim's
        `MonsterKind` axis: this is what the assembly says, and the join onto
        the modeled axis (including the three
        `DECIMILLIPEDE_SEGMENT_FRONT/_MIDDLE/_BACK` classes the sim carries as
        one aggregate kind) is the generator's.

        A class this reader cannot decode exactly is recorded as
        `{"refused": <reason>}` rather than dropped or guessed: the frontier
        stays visible, and a roster builder that needs a refused row fails at
        codegen instead of shipping an approximation (I5).
        """
        models = {}
        for category, entry, rid, _full_name in self._items:
            if category != "MONSTER":
                continue
            site = f"monster:{entry}"
            try:
                models[entry] = {
                    "min_initial_hp": self._constant_getter(
                        rid, "get_MinInitialHp", site),
                    "max_initial_hp": self._constant_getter(
                        rid, "get_MaxInitialHp", site),
                    "initial_powers": self._spawn_powers(rid, site),
                }
            except DllRefusal as exc:
                models[entry] = {"refused": str(exc)}
        return models

    #: The two getters `monster_models` already reads; every other tiered
    #: site of a monster class is a `monster_move_constants` row.
    HP_GETTERS = ("get_MinInitialHp", "get_MaxInitialHp")

    def _tier_sites(self, method_row, site):
        """`[(IL offset, gate, atOrAbove, below)]` for every
        `GetValueIfAscension` call in one body.

        Each call must be immediately preceded by three `ldc.i4*`: the only
        spelling this assembly uses. A call reached any other way is refused
        (I5), because its arguments would have to be inferred.
        """
        instructions = self._instructions(method_row)
        found = []
        for index, instruction in enumerate(instructions):
            if instruction.opcode.name not in ("call", "callvirt"):
                continue
            target = self._call_target(instruction.operand)
            if not target or target[:2] != self.ASCENSION_HELPER:
                continue
            values = [self._ldc(i) for i in instructions[max(0, index - 3):index]]
            if len(values) != 3 or None in values:
                raise DllRefusal(
                    f"{site}: a GetValueIfAscension call at IL_"
                    f"{instruction.offset:04x} is not preceded by three "
                    "integer literals")
            found.append((instruction.offset, *values))
        return found

    def monster_move_constants(self):
        """Every tiered constant of each `MONSTER` class except its HP.

        #2828. Keyed by the assembly's MONSTER entry, like `monster_models`,
        and read along the base chain up to `MonsterModel` (the three
        `DecimillipedeSegment*` classes and `MysteriousKnight` inherit their
        move getters). Three shapes, each recorded with its RVA so a
        consumer can cite it:

        * `constant` — a `get_X` body that is exactly
          `ldc; ldc; ldc; call GetValueIfAscension; ret`;
        * `operand` — a `get_X` body whose tier is one operand of a larger
          expression (`TheForgotten::get_DreadDamage` adds the owner's own
          Dexterity). The triple is exact; the rest of the body is the
          consumer's to model;
        * `inline` — a call inside any other method body (a nested
          `MoveNext`), keyed `<Type>::<Method>@IL_xxxx`.

        What each row feeds is modeling, not assembly data: the generator's
        `MOVE_CONSTANT_SITES` joins them onto the move tables and accounts for
        every row.
        """
        out = {}
        for category, entry, rid, _full_name in self._items:
            if category != "MONSTER":
                continue
            site = f"monster:{entry}"
            rows = {}
            try:
                for type_rid in self._base_chain(rid):
                    typedef = self._dll.typedefs[type_rid - 1]
                    type_name = str(typedef.TypeName)
                    if type_name == "MonsterModel":
                        break
                    owners = [type_rid - 1] + sorted(
                        index for index, enclosing in self._dll.enclosing.items()
                        if enclosing == type_rid - 1)
                    for owner in owners:
                        owner_def = self._dll.typedefs[owner]
                        owner_name = str(owner_def.TypeName)
                        for method in owner_def.MethodList:
                            row = method.row
                            name = str(row.Name)
                            if not row.Rva or name in self.HP_GETTERS:
                                continue
                            sites = self._tier_sites(
                                row, f"{site}:{owner_name}::{name}")
                            if not sites:
                                continue
                            getter = (owner == type_rid - 1
                                      and name.startswith("get_"))
                            if getter and name[4:] in rows:
                                continue  # overridden lower in the chain
                            if getter and len(sites) != 1:
                                raise DllRefusal(
                                    f"{site}: {name} holds {len(sites)} "
                                    "GetValueIfAscension calls")
                            for offset, gate, above, below in sites:
                                if getter:
                                    key = name[4:]
                                    count = len(self._instructions(row))
                                    shape = ("constant" if count == 5
                                             else "operand")
                                else:
                                    key = f"{owner_name}::{name}@IL_{offset:04x}"
                                    shape = "inline"
                                rows[key] = {
                                    "gate": gate, "at_or_above": above,
                                    "below": below, "rva": f"0x{row.Rva:x}",
                                    "declared_by": type_name, "shape": shape}
                out[entry] = dict(sorted(rows.items()))
            except DllRefusal as exc:
                out[entry] = {"refused": str(exc)}
        return out

    def _ctor_int_stores(self, rid):
        """`{field: value}` for the `ldarg.0; ldc.i4*; stfld <own field>`
        triples a class's `.ctor` runs before its base constructor call.

        Only that straight-line prologue is read — it is where the C# compiler
        puts field initialisers (`private int _turnsUntilSummonable = 2;`),
        and it runs before any base-class code can observe the object. The
        first instruction outside the triple shape ends the read: an
        initialiser that is not an integer literal is not a fact this reader
        claims, so it is absent rather than guessed (I5), and a builder that
        asks for it refuses by name.
        """
        typedef = self._dll.typedefs[rid - 1]
        ctor = next((m for m in typedef.MethodList
                     if str(m.row.Name) == ".ctor" and m.row.Rva), None)
        if ctor is None:
            return {}
        own = {field.row_index for field in typedef.FieldList}
        instructions = self._instructions(ctor.row)
        stores = {}
        index = 0
        while index + 2 < len(instructions):
            head, literal, store = instructions[index:index + 3]
            value = self._ldc(literal)
            token = getattr(store.operand, "value", None)
            if (head.opcode.name != "ldarg.0" or value is None
                    or store.opcode.name != "stfld" or token is None
                    or (token >> 24) & 0xFF != 0x04
                    or (token & 0xFFFFFF) not in own):
                break
            name = str(self._dll.md.Field.rows[(token & 0xFFFFFF) - 1].Name)
            stores[name] = value
            index += 3
        return stores

    def monster_ctor_ints(self):
        """Per-`MONSTER` integer field initialisers, by ModelId entry.

        Only classes whose `.ctor` prologue stores at least one integer
        literal appear. `TwoTailedRat::.ctor` (v0.111.0 `0xc445f`) is the
        reason this exists: `_turnsUntilSummonable = 2` is roster state
        (`combat_sim.Monster.tus`), and before this table it had no source a
        roster builder could read without typing the `2` in (PORT_PLAN §5).
        """
        out = {}
        for category, entry, rid, _full_name in self._items:
            if category != "MONSTER":
                continue
            stores = self._ctor_int_stores(rid)
            if stores:
                out[entry] = dict(sorted(stores.items()))
        return dict(sorted(out.items()))

    def encounter_rng_draws(self):
        """Per-`ENCOUNTER` literal Encounter-stream draw bounds, by entry.

        Reads the class's OWN `GenerateMonsters` body (never a base's: the
        per-fight draws are what that one body does) and records each
        `Rng::NextInt` whose receiver is `EncounterModel::get_Rng`, in body
        order, as `[lo, hi)`. The receiver anchors the stack shape, so the
        one- and two-argument overloads are told apart by how many `ldc`
        literals sit between `get_Rng` and the call: one is `NextInt(max)`
        (`[0, max)`), two is `NextInt(min, max)`. Anything else — a computed
        bound, a draw with no `get_Rng` receiver in view — records a refusal
        for that encounter rather than a guess (I5).

        Only encounters with at least one such draw appear. `Rng::NextItem`
        and `Rng::NextBool` are not bounds and are not read here.
        """
        draws = {}
        for category, entry, rid, _full_name in self._items:
            if category != "ENCOUNTER":
                continue
            typedef = self._dll.typedefs[rid - 1]
            if not any(str(m.row.Name) == "GenerateMonsters" and m.row.Rva
                       for m in typedef.MethodList):
                continue
            instructions, _rva = self._method_instructions(
                rid, "GenerateMonsters")
            found, refused = [], None
            for index, instruction in enumerate(instructions):
                if instruction.opcode.name not in ("call", "callvirt"):
                    continue
                target = self._call_target(instruction.operand)
                if not target or target[:2] != ("Rng", "NextInt"):
                    continue
                literals, cursor = [], index - 1
                while cursor >= 0 and self._ldc(instructions[cursor]) \
                        is not None:
                    literals.insert(0, self._ldc(instructions[cursor]))
                    cursor -= 1
                receiver = (self._call_target(instructions[cursor].operand)
                            if cursor >= 0 else None)
                if (not receiver or receiver[1] != "get_Rng"
                        or len(literals) not in (1, 2)):
                    refused = (
                        f"encounter:{entry}: GenerateMonsters draws "
                        f"Rng::NextInt at IL_{instruction.offset:04x} with "
                        "bounds that are not one or two ldc literals on the "
                        "EncounterModel::get_Rng receiver")
                    break
                found.append([0, literals[0]] if len(literals) == 1
                             else literals)
            if refused is not None:
                draws[entry] = {"refused": refused}
            elif found:
                draws[entry] = {"draws": found}
        return draws
    # -- per-potion facts --------------------------------------------------

    def potions(self):
        """`{potion_id: {rarity, can_be_generated_in_combat}}`.

        `PotionModel::get_Rarity` is abstract and every concrete potion
        declares it as a constant getter; `get_CanBeGeneratedInCombat`
        defaults to `true` on `PotionModel` `0x83120` and is overridden by
        the three combat-ineligible potions (Fairy in a Bottle `0xac3cc`,
        Fruit Juice, Regen Potion). Both are read through
        `_constant_getter`, so a computed body refuses rather than guesses.
        """
        if getattr(self, "_potion_facts", None) is None:
            rows = {}
            for potion_id in self.potion_ids():
                rid = self._rid_by_entry[("POTION", potion_id)]
                site = f"potion:{potion_id}"
                rows[potion_id] = {
                    "rarity": self._enum_name(
                        "PotionRarity",
                        self._constant_int_getter(rid, "get_Rarity", site),
                        f"{site}.rarity"),
                    "can_be_generated_in_combat": bool(
                        self._constant_int_getter(
                            rid, "get_CanBeGeneratedInCombat", site)),
                }
            self._potion_facts = rows
        return self._potion_facts

    # -- generation pools (#2542) ------------------------------------------
    #
    # Every card- and potion-generation pool in the game is one of these two
    # shapes, and both are plain static IL:
    #
    #   `<Character>CardPool::GenerateAllCards` is a literal
    #   `new CardModel[N]{ ModelDb.Card<Aggression>(), ... }` — the canonical
    #   pool order, which is RNG-significant because every generator shuffles
    #   or indexes this exact sequence.
    #
    #   `<Character>CardPool::FilterThroughEpochs(unlock, cards)` is a chain
    #   of `if (!unlock.IsEpochRevealed<XEpoch>()) cards.RemoveAll(c =>
    #   XEpoch.Cards.Any(...))`, and `XEpoch::get_Cards` is another literal
    #   `ModelDb.Card<T>()` array. So each card's unlock epoch is the epoch
    #   whose `Cards` list names it, and `None` means unconditionally
    #   unlocked.
    #
    # The potion side is the same two methods under different names
    # (`GenerateAllPotions` / `GetUnlockedPotions`), except that a character
    # potion pool is gated as a whole: `IroncladPotionPool::GenerateAllPotions`
    # `0xadd30` is literally `return Ironclad4Epoch.Potions`, and
    # `GetUnlockedPotions` `0xadd38` returns it only when that one epoch is
    # revealed.

    #: `ModelDb::get_AllCharacters` `0x80ef6` — the five characters in the
    #: literal order the array is built in. `UnlockState::get_Characters`
    #: `0xd870` starts from this list and removes the four gated ones, and
    #: `get_CharacterCardPools` `0xd994` is that list `.Select(c => c.CardPool)`
    #: — so this is the order every multi-pool concatenation preserves.
    CHARACTER_POOL_ORDER_SITE = "ModelDb::get_AllCharacters"

    def _generic_type_args(self, token):
        """The TypeDef rids a MethodSpec token instantiates.

        `GenerateAllCards` reaches each card through `ModelDb.Card<T>()`, so
        the pool membership lives in the generic argument rather than in any
        operand a plain instruction read exposes. The signature blob is
        decoded here rather than approximated: `GENERICINST`, an argument
        count, then one `ELEMENT_TYPE_CLASS`/`VALUETYPE` + compressed
        `TypeDefOrRef`. Anything else raises (I5).
        """
        def uncompressed(data, index):
            head = data[index]
            if head & 0x80 == 0:
                return head, index + 1
            if head & 0xC0 == 0x80:
                return ((head & 0x3F) << 8) | data[index + 1], index + 2
            return (((head & 0x1F) << 24) | (data[index + 1] << 16)
                    | (data[index + 2] << 8) | data[index + 3]), index + 4

        row = self._dll.md.MethodSpec.rows[(token & 0xFFFFFF) - 1]
        data = row.Instantiation.raw_data
        _size, cursor = uncompressed(data, 0)
        if data[cursor] != 0x0A:
            raise DllRefusal(
                f"MethodSpec {token:#x} is not a GENERICINST signature")
        cursor += 1
        count, cursor = uncompressed(data, cursor)
        args = []
        for _ in range(count):
            if data[cursor] not in (0x11, 0x12):
                raise DllRefusal(
                    f"MethodSpec {token:#x} instantiates a non-class type")
            cursor += 1
            coded, cursor = uncompressed(data, cursor)
            if coded & 3 != 0:
                raise DllRefusal(
                    f"MethodSpec {token:#x} names a TypeRef, not a TypeDef")
            args.append(coded >> 2)
        method = row.Method
        if method.table.name == "MethodDef":
            name = str(self._dll.md.MethodDef.rows[
                method.row_index - 1].Name)
        else:
            name = str(self._dll.md.MemberRef.rows[
                method.row_index - 1].Name)
        return name, args

    def _type_rid(self, type_name):
        if not hasattr(self, "_type_rid_cache"):
            cache = {}
            for index, typedef in enumerate(self._dll.typedefs):
                cache.setdefault(str(typedef.TypeName), index + 1)
            self._type_rid_cache = cache
        rid = self._type_rid_cache.get(type_name)
        if rid is None:
            raise DllRefusal(f"{self.path.name} declares no type {type_name}")
        return rid

    def _method_owner(self, method_rid):
        if not hasattr(self, "_method_owner_cache"):
            cache = {}
            for index, typedef in enumerate(self._dll.typedefs):
                for method in typedef.MethodList:
                    cache[method.row_index] = index + 1
            self._method_owner_cache = cache
        return self._method_owner_cache.get(method_rid)

    def _method_instructions(self, type_rid, name):
        typedef = self._dll.typedefs[type_rid - 1]
        methods = [m for m in typedef.MethodList
                   if str(m.row.Name) == name and m.row.Rva]
        if len(methods) != 1:
            raise DllRefusal(
                f"{str(typedef.TypeName)}::{name}: {len(methods)} bodies, "
                "expected exactly one")
        body = self._body(methods[0].row)
        if body is None:  # pragma: no cover - Rva was checked above
            raise DllRefusal(f"{str(typedef.TypeName)}::{name} has no body")
        return list(body.instructions), methods[0].row.Rva

    def _model_entries(self, instructions, factory, category):
        """The ModelId entries a literal `ModelDb.<factory><T>()` array names."""
        entries = []
        for instruction in instructions:
            operand = getattr(instruction.operand, "value", None)
            if operand is None or (operand >> 24) & 0xFF != 0x2B:
                continue
            name, args = self._generic_type_args(operand)
            if name != factory:
                continue
            entry = next(
                (slug for cat, slug, rid, _fn in self._items
                 if cat == category and rid == args[0]), None)
            if entry is None:
                raise DllRefusal(
                    f"{factory}<{str(self._dll.typedefs[args[0] - 1].TypeName)}"
                    f"> is not a {category} ModelId")
            entries.append(entry)
        return entries

    def _epoch_id_by_type(self):
        if not hasattr(self, "_epoch_id_cache"):
            self._epoch_id_cache = {
                full_name.rsplit(".", 1)[-1]: epoch_id
                for epoch_id, full_name in self._mcr.epoch_items(self._dll)}
        return self._epoch_id_cache

    def _reads_epoch_members(self, method_rid, member_getter):
        """The epoch TypeDef rids whose member list a method body reads."""
        body = self._body(self._dll.md.MethodDef.rows[method_rid - 1])
        if body is None:  # pragma: no cover - callers pass real bodies
            return set()
        owners = set()
        for instruction in list(body.instructions):
            value = getattr(instruction.operand, "value", None)
            if value is None or (value >> 24) & 0xFF != 0x06:
                continue
            rid = value & 0xFFFFFF
            if str(self._dll.md.MethodDef.rows[rid - 1].Name) == member_getter:
                owners.add(self._method_owner(rid))
        return owners

    def _epoch_gates(self, pool_rid, gate_method, member_getter, factory,
                     category):
        """`{entry: epoch_id}` for one pool's `IsEpochRevealed` chain.

        Two removal shapes occur and both are accepted, because they are the
        same statement compiled differently: `FilterThroughEpochs` uses
        `RemoveAll(<lambda over XEpoch.Cards>)`, so the read is behind an
        `ldftn`, while `SharedPotionPool::GetUnlockedPotions` `0xadfb4`
        `foreach`es `XEpoch.Potions` and calls `Remove` directly. What is
        *not* accepted is a test whose removal reads a different epoch than
        the one tested, or a test with no removal at all — either would mean
        the gate structure changed and the per-row epochs below are no longer
        what the game applies (I5).
        """
        pool_name = str(self._dll.typedefs[pool_rid - 1].TypeName)
        instructions, _rva = self._method_instructions(pool_rid, gate_method)
        epoch_ids = self._epoch_id_by_type()
        gates, pending = {}, None

        def close(reads):
            nonlocal pending
            if pending is None:
                raise DllRefusal(
                    f"{pool_name}::{gate_method}: a removal with no "
                    "preceding IsEpochRevealed test")
            if reads != {pending}:
                raise DllRefusal(
                    f"{pool_name}::{gate_method}: the removal for "
                    f"{str(self._dll.typedefs[pending - 1].TypeName)} reads "
                    f"{sorted(reads)!r} instead")
            epoch_type = str(self._dll.typedefs[pending - 1].TypeName)
            epoch_id = epoch_ids.get(epoch_type)
            if epoch_id is None:
                raise DllRefusal(
                    f"{epoch_type} is not in EpochModel.AllEpochs")
            gated, _unused = self._method_instructions(pending, member_getter)
            for entry in self._model_entries(gated, factory, category):
                if entry in gates:
                    raise DllRefusal(
                        f"{entry} is gated by two epochs "
                        f"({gates[entry]} and {epoch_id})")
                gates[entry] = epoch_id
            pending = None

        for instruction in instructions:
            operand = getattr(instruction.operand, "value", None)
            if operand is None:
                continue
            table = (operand >> 24) & 0xFF
            if table == 0x2B:
                name, args = self._generic_type_args(operand)
                if name == "IsEpochRevealed":
                    if pending is not None:
                        raise DllRefusal(
                            f"{pool_name}::{gate_method}: an IsEpochRevealed "
                            "test with no removal")
                    pending = args[0]
            elif table == 0x06:
                rid = operand & 0xFFFFFF
                if instruction.opcode.name == "ldftn":
                    close(self._reads_epoch_members(rid, member_getter))
                elif str(self._dll.md.MethodDef.rows[
                        rid - 1].Name) == member_getter:
                    close({self._method_owner(rid)})
        if pending is not None:
            raise DllRefusal(
                f"{pool_name}::{gate_method}: an IsEpochRevealed test with "
                "no removal")
        return gates

    def _whole_pool_epoch_gate(self, pool_rid, gate_method):
        """The single epoch that gates an entire pool, or a refusal.

        `<Character>PotionPool::GetUnlockedPotions` `0xadd38` is
        `IsEpochRevealed<X>() ? GenerateAllPotions() : Empty<PotionModel>()` —
        no per-row removal at all. Anything with more than one test, or with
        a removal, is refused rather than flattened.
        """
        pool_name = str(self._dll.typedefs[pool_rid - 1].TypeName)
        instructions, _rva = self._method_instructions(pool_rid, gate_method)
        tested, saw_empty = [], False
        for instruction in instructions:
            operand = getattr(instruction.operand, "value", None)
            if operand is None or (operand >> 24) & 0xFF != 0x2B:
                continue
            name, args = self._generic_type_args(operand)
            if name == "IsEpochRevealed":
                tested.append(args[0])
            elif name == "Empty":
                saw_empty = True
        if len(tested) != 1 or not saw_empty:
            raise DllRefusal(
                f"{pool_name}::{gate_method} is not a single whole-pool "
                f"epoch gate ({len(tested)} tests, empty branch {saw_empty})")
        epoch_type = str(self._dll.typedefs[tested[0] - 1].TypeName)
        epoch_id = self._epoch_id_by_type().get(epoch_type)
        if epoch_id is None:
            raise DllRefusal(f"{epoch_type} is not in EpochModel.AllEpochs")
        return epoch_id

    def character_pool_order(self):
        """`ModelDb.AllCharacters`, in the literal array order `0x80ef6`."""
        instructions, _rva = self._method_instructions(
            self._type_rid("ModelDb"), "get_AllCharacters")
        order = [str(self._dll.typedefs[args[0] - 1].TypeName).upper()
                 for name, args in (
                     self._generic_type_args(value)
                     for value in (
                         getattr(i.operand, "value", None)
                         for i in instructions)
                     if value is not None and (value >> 24) & 0xFF == 0x2B)
                 if name == "Character"]
        if len(order) != len(set(order)) or not order:
            raise DllRefusal(f"ModelDb.AllCharacters is malformed: {order!r}")
        return order

    def character_unlock_epochs(self):
        """`{CHARACTER: [epoch ids ascending]}` for the five card pools.

        A character's epochs are exactly the `EpochModel` subclasses whose
        class name is the character's name followed by a digit — the same
        `Ironclad2Epoch` / `Defect7Epoch` classes `FilterThroughEpochs` and
        `UnlockState::get_Characters` `0xd870` name directly. Derived from the
        epoch inventory rather than by parsing the id strings, so a renamed
        epoch is a missing type rather than a silently dropped row.
        """
        rows = {}
        for epoch_id, full_name in self._mcr.epoch_items(self._dll):
            class_name = full_name.rsplit(".", 1)[-1]
            if not class_name.endswith("Epoch"):
                continue
            stem = class_name[:-len("Epoch")]
            if not stem or not stem[-1].isdigit():
                continue
            character = stem.rstrip("0123456789").upper()
            rows.setdefault(character, []).append(epoch_id)
        return {character: sorted(epochs)
                for character, epochs in rows.items()
                if character in set(self.character_pool_order())}

    def character_card_pools(self):
        """`{CHARACTER: [[card_id, unlock_epoch | None], ...]}`, native order.

        The membership and its order come from
        `<Character>CardPool::GenerateAllCards`; the per-row epoch comes from
        `FilterThroughEpochs`. Nothing is filtered here — the rarity, type,
        `MultiplayerConstraint` and `CanBeGeneratedInCombat` predicates each
        generator applies are per-card columns on the `cards` rows, and the
        epoch predicate depends on the fight's recorded profile. This is the
        raw pool the game builds, and every projection is derived from it.
        """
        pools = {}
        for character in self.character_pool_order():
            pool_rid = self._type_rid(character.capitalize() + "CardPool")
            members, _rva = self._method_instructions(pool_rid, "GenerateAllCards")
            order = self._model_entries(members, "Card", "CARD")
            if len(order) != len(set(order)) or not order:
                raise DllRefusal(
                    f"{character}CardPool.GenerateAllCards repeats a card")
            gates = self._epoch_gates(
                pool_rid, "FilterThroughEpochs", "get_Cards", "Card", "CARD")
            unknown = sorted(set(gates) - set(order))
            if unknown:
                raise DllRefusal(
                    f"{character}CardPool gates cards outside its own pool: "
                    f"{unknown!r}")
            pools[character] = [[card_id, gates.get(card_id)]
                                for card_id in order]
        return pools

    def colorless_card_pool(self):
        """`[[card_id, unlock_epoch | None], ...]` for `ColorlessCardPool`.

        The same two-method shape as a character pool, under the one pool
        every character shares: `ColorlessCardPool::GenerateAllCards`
        `0xf11a0` is a literal 65-element `ModelDb.Card<T>()` array whose
        order is RNG-significant, and `FilterThroughEpochs` `0xf13f4` is the
        `IsEpochRevealed<Colorless<N>Epoch>` chain that removes the fifteen
        gated rows — three at each of the five Colorless epochs, matching the
        three-per-epoch structure every character pool has.

        Read here rather than taken from `card_pool_census.json` because that
        harness artifact never recorded the Colorless pool at all, and read as
        a pool rather than as a flat id list because the per-row epoch is
        exactly what #2512's partial-unlock slice needs: `COLORLESS_CARD_IDS`
        froze the fully-unlocked projection, which is only one profile's
        answer.
        """
        pool_rid = self._type_rid("ColorlessCardPool")
        members, _rva = self._method_instructions(pool_rid, "GenerateAllCards")
        order = self._model_entries(members, "Card", "CARD")
        if len(order) != len(set(order)) or not order:
            raise DllRefusal(
                "ColorlessCardPool.GenerateAllCards repeats a card")
        gates = self._epoch_gates(
            pool_rid, "FilterThroughEpochs", "get_Cards", "Card", "CARD")
        unknown = sorted(set(gates) - set(order))
        if unknown:
            raise DllRefusal(
                f"ColorlessCardPool gates cards outside its own pool: "
                f"{unknown!r}")
        return [[card_id, gates.get(card_id)] for card_id in order]

    def shared_card_pools(self):
        """`[[POOL, [card_id, ...]], ...]` for every shared pool but Colorless.

        #2734. `ModelDb::get_AllCardPools` (RVA `0x80e24`) is
        `AllCharacterCardPools.Concat(AllSharedCardPools)`, and
        `get_AllSharedCardPools` (RVA `0x80e58`) is a literal seven-element
        `ModelDb.CardPool<T>()` array: Colorless, Curse, Deprecated, Event,
        Quest, Status, Token. `CardModel::get_Pool` (RVA `0x7c878`) searches
        that whole list, so a Shiv's or a Regret's pool is one of these — and
        `CardFactory::GetDefaultTransformationOptions` (RVA `0x112960`
        IL_0041) transforms every card outside its Colorless exclusion set
        through exactly that pool. Colorless is read by `colorless_card_pool`
        above (it has epochs) and is only checked for here.

        The other six declare no `FilterThroughEpochs`/`GetUnlockedCards` of
        their own, so they inherit `CardPoolModel::FilterThroughEpochs` (RVA
        `0x7e5c5`, `ldarg.2; ToList; ret` — the identity): no per-row epoch
        exists to record. A pool that ever grows its own override, or stops
        deriving directly from `CardPoolModel`, is refused rather than read
        as ungated (I5). Membership order is the RNG-significant
        `GenerateAllCards` array order.
        """
        instructions, _rva = self._method_instructions(
            self._type_rid("ModelDb"), "get_AllSharedCardPools")
        order = [str(self._dll.typedefs[args[0] - 1].TypeName)
                 for name, args in (
                     self._generic_type_args(value)
                     for value in (
                         getattr(i.operand, "value", None)
                         for i in instructions)
                     if value is not None and (value >> 24) & 0xFF == 0x2B)
                 if name == "CardPool"]
        if (len(order) != len(set(order)) or "ColorlessCardPool" not in order
                or not all(n.endswith("CardPool") for n in order)):
            raise DllRefusal(
                f"ModelDb.AllSharedCardPools is malformed: {order!r}")
        base = self._type_rid("CardPoolModel")
        pools = []
        for pool_name in order:
            if pool_name == "ColorlessCardPool":
                continue
            pool_rid = self._type_rid(pool_name)
            if self._base_chain(pool_rid)[1:2] != [base]:
                raise DllRefusal(
                    f"{pool_name} does not derive directly from CardPoolModel")
            for gate in ("FilterThroughEpochs", "GetUnlockedCards",
                         "get_AllCards"):
                if self._overrides(pool_rid, gate):
                    raise DllRefusal(
                        f"{pool_name} overrides {gate}, so its rows may be "
                        "epoch-gated; read it the way colorless_card_pool is")
            members, _rva = self._method_instructions(
                pool_rid, "GenerateAllCards")
            entries = self._model_entries(members, "Card", "CARD")
            if len(entries) != len(set(entries)) or not entries:
                raise DllRefusal(
                    f"{pool_name}.GenerateAllCards repeats a card or is empty")
            pools.append([pool_name[:-len("CardPool")].upper(), entries])
        return pools

    def character_potion_pools(self):
        """`{CHARACTER: {"epoch": id, "potions": [...]}}`, native order.

        `<Character>PotionPool::GenerateAllPotions` is literally
        `return <Character>4Epoch.Potions`, and `GetUnlockedPotions` returns
        an empty list unless that single epoch is revealed — so unlike the
        card pools, a character potion pool is gated whole rather than
        per row.
        """
        pools = {}
        for character in self.character_pool_order():
            pool_rid = self._type_rid(character.capitalize() + "PotionPool")
            members, _rva = self._method_instructions(pool_rid, "GenerateAllPotions")
            reads = self._reads_epoch_members(
                next(m.row_index
                     for m in self._dll.typedefs[pool_rid - 1].MethodList
                     if str(m.row.Name) == "GenerateAllPotions" and m.row.Rva),
                "get_Potions")
            potions = []
            for epoch_rid in sorted(reads):
                gated, _unused = self._method_instructions(epoch_rid, "get_Potions")
                potions.extend(
                    self._model_entries(gated, "Potion", "POTION"))
            epoch = self._whole_pool_epoch_gate(
                pool_rid, "GetUnlockedPotions")
            if (len(reads) != 1
                    or self._epoch_id_by_type()[str(self._dll.typedefs[
                        next(iter(reads)) - 1].TypeName)] != epoch
                    or not potions or len(potions) != len(set(potions))
                    or self._model_entries(members, "Potion", "POTION")):
                raise DllRefusal(
                    f"{character}PotionPool::GenerateAllPotions is not a bare "
                    f"`return {epoch}.Potions`: {sorted(reads)!r}/{potions!r}")
            pools[character] = {"epoch": epoch, "potions": potions}
        return pools

    def shared_potion_pool(self):
        """`[[potion_id, unlock_epoch | None], ...]` for `SharedPotionPool`."""
        pool_rid = self._type_rid("SharedPotionPool")
        members, _rva = self._method_instructions(pool_rid, "GenerateAllPotions")
        order = self._model_entries(members, "Potion", "POTION")
        if len(order) != len(set(order)) or not order:
            raise DllRefusal("SharedPotionPool.GenerateAllPotions repeats")
        gates = self._epoch_gates(
            pool_rid, "GetUnlockedPotions", "get_Potions", "Potion", "POTION")
        unknown = sorted(set(gates) - set(order))
        if unknown:
            raise DllRefusal(
                f"SharedPotionPool gates potions outside itself: {unknown!r}")
        return [[potion_id, gates.get(potion_id)] for potion_id in order]

    def mad_science_variants(self):
        """Every saved Tinker Time variant of Mad Science, in native order (#2942).

        `TinkerTime::ChooseRiderEffect` switches on `ChosenCardType - 1` and
        each arm builds a three-element `RiderEffect[]` from an
        `InitializeArray` blob; `TinkerTime/<RiderChosen>d__15::MoveNext`
        then stores the chosen type and rider into a fresh `MadScience`
        (`set_TinkerTimeType` / `set_TinkerTimeRider`). Those two are the
        card's only `[SavedProperty]` members, so this list is the complete
        legal domain a save can carry. Each row also records the card's
        effective `get_TargetType` and `get_GainsBlock` for that type, read
        from the two getters' `type == k` comparisons. Any IL shape other
        than the one read here is a `DllRefusal`, never a guess.
        """
        card_types = self.enum("CardType")
        riders = self.enum("RiderEffect")
        targets = self.enum("TargetType")
        event_rid = self._type_rid("TinkerTime")
        choose, choose_rva = self._method_instructions(
            event_rid, "ChooseRiderEffect")
        switch_index = next(
            (index for index, instruction in enumerate(choose)
             if instruction.opcode.name == "switch"), None)
        # `call get_ChosenCardType; stloc.3; ldloc.3; ldc.i4.1; sub; switch`
        if (switch_index is None or switch_index < 5
                or choose[switch_index - 1].opcode.name != "sub"
                or self._ldc(choose[switch_index - 2]) != 1
                or (self._call_target(choose[switch_index - 5].operand)
                    or ("", "", None))[1] != "get_ChosenCardType"):
            raise DllRefusal(
                "TinkerTime::ChooseRiderEffect no longer switches on "
                "`ChosenCardType - 1`")
        arms = list(choose[switch_index].operand)
        by_offset = {instruction.offset: index
                     for index, instruction in enumerate(choose)}
        variants = []
        for arm, target in enumerate(arms):
            start = by_offset.get(target)
            window = choose[start:start + 4] if start is not None else []
            if (len(window) != 4
                    or window[1].opcode.name != "newarr"
                    or window[2].opcode.name != "dup"
                    or window[3].opcode.name != "ldtoken"):
                raise DllRefusal(
                    f"TinkerTime::ChooseRiderEffect arm {arm} is not "
                    "`ldc.i4 n; newarr RiderEffect; dup; ldtoken <blob>`")
            count = self._ldc(window[0])
            token = window[3].operand.value
            rva = self._field_rva.get(token & 0xFFFFFF)
            if (token >> 24) & 0xFF != 0x04 or not rva or not count:
                raise DllRefusal(
                    f"TinkerTime::ChooseRiderEffect arm {arm} names no field "
                    "RVA blob")
            offset = self._dll.pe.get_offset_from_rva(rva)
            values = [int.from_bytes(
                self._dll.raw[offset + 4 * i:offset + 4 * i + 4], "little")
                for i in range(count)]
            card_type = arm + 1
            if card_type not in card_types:
                raise DllRefusal(
                    f"TinkerTime arm {arm} names no CardType {card_type}")
            for value in values:
                if value not in riders or value == 0:
                    raise DllRefusal(
                        f"TinkerTime arm {arm} names rider {value}, which is "
                        "not a RiderEffect or is RiderEffect.None")
                variants.append({
                    "type": card_types[card_type],
                    "type_value": card_type,
                    "rider": riders[value],
                    "rider_value": value,
                })
        card_rid = self._type_rid("MadScience")
        target_body, target_rva = self._method_instructions(
            card_rid, "get_TargetType")
        names = [instruction.opcode.name for instruction in target_body]
        if (names[:2] != ["ldarg.0", "call"] or len(names) != 8
                or names[3] != "beq.s" or names[5] != "ret"
                or names[7] != "ret"
                or self._call_target(target_body[1].operand)[1]
                != "get_TinkerTimeType"):
            raise DllRefusal(
                "MadScience::get_TargetType is no longer `TinkerTimeType == k "
                "? a : b`")
        target_when, target_else = (self._ldc(target_body[6]),
                                    self._ldc(target_body[4]))
        compared = self._ldc(target_body[2])
        block_body, block_rva = self._method_instructions(
            card_rid, "get_GainsBlock")
        names = [instruction.opcode.name for instruction in block_body]
        if (names != ["ldarg.0", "call", names[2], "ceq", "ret"]
                or self._call_target(block_body[1].operand)[1]
                != "get_TinkerTimeType"):
            raise DllRefusal(
                "MadScience::get_GainsBlock is no longer `TinkerTimeType == k`")
        block_type = self._ldc(block_body[2])
        saved = [name for _order, name in self._dll.saved_properties(card_rid)]
        if saved != ["TinkerTimeType", "TinkerTimeRider"]:
            raise DllRefusal(
                f"MadScience saves {saved!r}, not exactly TinkerTimeType and "
                "TinkerTimeRider")
        for row in variants:
            target = target_when if row["type_value"] == compared \
                else target_else
            if target not in targets:
                raise DllRefusal(f"MadScience target {target} is no TargetType")
            row["target_type"] = targets[target]
            row["gains_block"] = row["type_value"] == block_type
        return {
            "il": {
                "TinkerTime::ChooseRiderEffect": hex(choose_rva),
                "MadScience::get_TargetType": hex(target_rva),
                "MadScience::get_GainsBlock": hex(block_rva),
            },
            "saved_properties": saved,
            "variants": variants,
        }


# ---------------------------------------------------------------------------
# The committed content manifest
# ---------------------------------------------------------------------------
#
# Why a manifest exists at all (#2515, resolving the item #2513 left open).
#
# The `rust port` workflow's *Codegen freshness* step re-derives `ids.rs` and
# `content_tables.rs` in CI and diffs them against the committed files. In CI
# neither content source above is usable on a forked crate: the archive is
# gitignored and the runner's `python3` has no `dnfile`, so `--source dll`
# cannot run there, while D3 freezes `combat_sim` at v0.111.0, so on a
# `versions/vNEXT/rust/` fork `--source python` would re-derive *v0.111.0
# content* and diff it against tables generated from the NEW assembly. The
# step would go red for a legitimate reason and stop being a freshness check.
#
# So the DLL read is split in two. `--source dll` writes every fact it
# extracted into `data/dll_content.<build>.json` — committed, reviewable in
# the diff, carrying the assembly's sha256 and the game's own `modelIdHash` —
# and generates from THAT. `--source manifest` replays it with no assembly and
# no `dnfile`, which is what CI runs. The DLL is still the authority; the
# manifest is the authority's committed testimony, and `--require-dll`
# (runbook step 11) is where the two are proven equal on a host that holds the
# archive.

#: Schema of `versions/<build>/rust/data/dll_content.<build>.json`.
MANIFEST_SCHEMA = "sts-sim-dll-content-manifest/v1"

#: Every fact key the manifest must carry. Derived from what `DllFacts`
#: exposes rather than from what today's generator happens to call: a manifest
#: that silently omitted a key would send that lookup back to the frozen
#: registry, which is the exact failure this module exists to prevent.
MANIFEST_FACT_KEYS = (
    "card_ids",
    "card_rarity_vocabulary",
    "card_type_members",
    "cards",
    "character_card_pools",
    "character_pool_order",
    "character_potion_pools",
    "character_unlock_epochs",
    "colorless_card_pool",
    "enchantment_ids",
    "encounter_inventory",
    "encounter_rng_draws",
    "epochs",
    "mad_science_variants",
    "monster_ctor_ints",
    "monster_inventory",
    "monster_models",
    "monster_move_constants",
    "native_unplayable_keys",
    "potion_ids",
    "potions",
    "relic_ids",
    "shared_card_pools",
    "shared_potion_pool",
)

#: The per-card columns every `cards` row must carry, mirroring the dict
#: `DllFacts._read_card` returns.
MANIFEST_CARD_COLUMNS = ("can_be_generated_in_combat", "cost",
                         "dynamic_target", "keywords",
                         "multiplayer_constraint", "rarity", "tags",
                         "target_type", "type")

#: The per-potion columns every `potions` row must carry.
MANIFEST_POTION_COLUMNS = ("can_be_generated_in_combat", "rarity")


class ManifestUnavailable(DllUnavailable):
    """No committed manifest for this build in this checkout.

    A subclass of `DllUnavailable` so `--source auto` can walk past it to the
    next source the same way it walks past a missing archive. A manifest that
    is *present but wrong* is a `DllRefusal` instead, and never falls back.
    """


def default_manifest_path(build=CERTIFIED_BUILD):
    """P2: per-build data beside the crate that consumes it."""
    return RUST_DIR / "data" / f"dll_content.{build}.json"


def archive_record(dll_sha256):
    """The committed archive-index row for one assembly, by sha256.

    Provenance comes from `solver/dll-archive/index.json`, not from where the
    file happened to sit: a path is host-specific, and the index is the repo's
    own record of which build a sha256 *is*. An unarchived assembly is refused
    rather than attributed — the runbook archives first (I11, and v0.108.0 is
    gone because someone did not), and a manifest whose build and commit were
    guessed is worse than no manifest.
    """
    index_path = ARCHIVE_DIR / "index.json"
    if not index_path.exists():
        raise DllRefusal(
            f"{index_path} is missing, so sha256 {dll_sha256} cannot be "
            "attributed to a build")
    index = json.loads(index_path.read_text())
    matches = sorted(
        (build, row) for build, row in (index.get("builds") or {}).items()
        if row.get("dll_sha256") == dll_sha256)
    if len(matches) != 1:
        raise DllRefusal(
            f"sha256 {dll_sha256} matches {len(matches)} rows in "
            f"{index_path.name}; archive the build first (see "
            "solver/dll-archive/README.md). The manifest's build and commit "
            "are read from the index, never guessed")
    build, row = matches[0]
    return {
        "build": build,
        "archived_at": row.get("archived_at"),
        "built_at": row.get("built_at"),
        "commit": row.get("commit"),
    }


def manifest_facts(facts):
    """Every fact `DllContentSource` reads, in a deterministic JSON shape.

    Sets become sorted lists and tuples become lists, so the serialization is
    a pure function of the assembly — no `PYTHONHASHSEED`, no wall clock, no
    checkout path. That is what lets the committed file be diffed for
    freshness rather than merely regenerated.
    """
    cards = {}
    for card_id in facts.card_ids():
        card = facts.card(card_id)
        cards[card_id] = {
            "cost": card["cost"],
            "dynamic_target": bool(card["dynamic_target"]),
            "keywords": sorted(card["keywords"]),
            "rarity": card["rarity"],
            "tags": sorted(card["tags"]),
            "target_type": card["target_type"],
            "type": card["type"],
            "multiplayer_constraint": card["multiplayer_constraint"],
            "can_be_generated_in_combat": bool(
                card["can_be_generated_in_combat"]),
        }
    return {
        "card_ids": list(facts.card_ids()),
        "card_rarity_vocabulary": list(facts.card_rarity_vocabulary()),
        "card_type_members": [[value, name]
                              for value, name in facts.card_type_members()],
        "cards": cards,
        "character_card_pools": {
            character: [[card_id, epoch] for card_id, epoch in rows]
            for character, rows in facts.character_card_pools().items()},
        "character_pool_order": list(facts.character_pool_order()),
        "character_potion_pools": {
            character: {"epoch": row["epoch"],
                        "potions": list(row["potions"])}
            for character, row in facts.character_potion_pools().items()},
        "character_unlock_epochs": {
            character: list(epochs)
            for character, epochs in facts.character_unlock_epochs().items()},
        "colorless_card_pool": [[card_id, epoch]
                                for card_id, epoch
                                in facts.colorless_card_pool()],
        "potions": {
            potion_id: {
                "rarity": row["rarity"],
                "can_be_generated_in_combat": bool(
                    row["can_be_generated_in_combat"]),
            }
            for potion_id, row in facts.potions().items()},
        "shared_card_pools": [[pool, list(members)]
                              for pool, members in facts.shared_card_pools()],
        "shared_potion_pool": [[potion_id, epoch]
                               for potion_id, epoch
                               in facts.shared_potion_pool()],
        "enchantment_ids": list(facts.enchantment_ids()),
        "encounter_inventory": list(facts.encounter_inventory()),
        "encounter_rng_draws": facts.encounter_rng_draws(),
        "epochs": list(facts.epochs()),
        "mad_science_variants": facts.mad_science_variants(),
        "monster_inventory": list(facts.monster_inventory()),
        "monster_models": facts.monster_models(),
        "monster_move_constants": facts.monster_move_constants(),
        "monster_ctor_ints": facts.monster_ctor_ints(),
        "native_unplayable_keys": [
            [card_id, upgrade]
            for card_id, upgrade in facts.native_unplayable_keys()],
        "potion_ids": list(facts.potion_ids()),
        "relic_ids": list(facts.relic_ids()),
    }


def manifest_self_hash(manifest):
    """sha256 over the manifest's canonical payload, `self_sha256` excluded."""
    payload = {key: value for key, value in manifest.items()
               if key != "self_sha256"}
    return hashlib.sha256(
        json.dumps(payload, sort_keys=True, separators=(",", ":"),
                   ensure_ascii=False).encode("utf-8")).hexdigest()


def build_manifest(facts):
    """The complete manifest document for one assembly."""
    record = archive_record(facts.sha256)
    body = {
        "schema": MANIFEST_SCHEMA,
        "build": record["build"],
        "facts": manifest_facts(facts),
        "provenance": {
            "archived_at": record["archived_at"],
            "built_at": record["built_at"],
            "commit": record["commit"],
            "dll": facts.path.name,
            "dll_sha256": facts.sha256,
            "generated_by": (
                f"versions/{CERTIFIED_BUILD}/rust/tools/generate_content.py "
                "--source dll"),
            "model_id_hash": facts.model_id_hash(),
        },
    }
    return dict(body, self_sha256=manifest_self_hash(body))


def manifest_text(manifest):
    """The exact bytes the manifest is committed as."""
    return json.dumps(manifest, sort_keys=True, indent=2,
                      ensure_ascii=False) + "\n"


class ManifestFacts:
    """Every DLL-sourced fact for one build, read from the committed manifest.

    The interface is exactly the part of `DllFacts` that `DllContentSource`,
    `--report` and the acceptance tests use — deliberately, so the manifest is
    a drop-in for the assembly and no lookup can quietly take a different
    route depending on which source is in play. Nothing here needs `dnfile`
    or an `sts2.dll`, which is the whole point.

    Three things are verified before any fact is served, and each is a
    `DllRefusal` (never a fallback): the schema, the `self_sha256` over the
    payload, and the presence of every key in `MANIFEST_FACT_KEYS` plus every
    column in `MANIFEST_CARD_COLUMNS` on every card row. A hand-edited
    manifest therefore fails loudly rather than shipping a fabricated content
    table.
    """

    name = "manifest"

    def __init__(self, path=None, *, require_certified=False, payload=None):
        path = pathlib.Path(path) if path else default_manifest_path()
        if payload is None:
            if not path.exists():
                raise ManifestUnavailable(
                    f"no committed content manifest at {path}; write one with "
                    "`generate_content.py --source dll` on a host holding the "
                    f"archived {CERTIFIED_BUILD} assembly")
            try:
                payload = json.loads(path.read_text())
            except (OSError, ValueError) as exc:
                raise DllRefusal(
                    f"{path} is not readable JSON: {exc}") from exc
        self.path = path
        schema = payload.get("schema")
        if schema != MANIFEST_SCHEMA:
            raise DllRefusal(
                f"{path.name} declares schema {schema!r}, not "
                f"{MANIFEST_SCHEMA!r}")
        recorded, actual = payload.get("self_sha256"), \
            manifest_self_hash(payload)
        if recorded != actual:
            raise DllRefusal(
                f"{path.name} records self_sha256 {recorded!r} but its "
                f"payload hashes to {actual!r}: it was edited by hand. "
                "Regenerate it from the assembly (`--source dll`); never "
                "patch a content manifest in place")
        self.manifest = payload
        self.provenance_record = payload.get("provenance") or {}
        self.build = payload.get("build")
        self.sha256 = self.provenance_record.get("dll_sha256")
        if require_certified and self.sha256 != CERTIFIED_DLL_SHA256:
            raise DllRefusal(
                f"{path.name} records dll sha256 {self.sha256}, not the "
                f"certified {CERTIFIED_BUILD} assembly "
                f"{CERTIFIED_DLL_SHA256}")
        self.facts_payload = payload.get("facts") or {}
        missing = [key for key in MANIFEST_FACT_KEYS
                   if key not in self.facts_payload]
        if missing:
            raise DllRefusal(
                f"{path.name} omits DLL-sourced facts {missing}; a manifest "
                "missing a fact would send that lookup back to the frozen "
                "registry, which is what this file exists to prevent")
        if sorted(self.facts_payload["cards"]) != \
                sorted(self.facts_payload["card_ids"]):
            raise DllRefusal(
                f"{path.name}: the `cards` rows and the `card_ids` axis "
                "disagree")
        for card_id, row in sorted(self.facts_payload["cards"].items()):
            absent = [column for column in MANIFEST_CARD_COLUMNS
                      if column not in row]
            if absent:
                raise DllRefusal(
                    f"{path.name}: card {card_id} has no {absent}")
        if sorted(self.facts_payload["potions"]) != \
                sorted(self.facts_payload["potion_ids"]):
            raise DllRefusal(
                f"{path.name}: the `potions` rows and the `potion_ids` axis "
                "disagree")
        for potion_id, row in sorted(self.facts_payload["potions"].items()):
            absent = [column for column in MANIFEST_POTION_COLUMNS
                      if column not in row]
            if absent:
                raise DllRefusal(
                    f"{path.name}: potion {potion_id} has no {absent}")
        pooled = set(self.facts_payload["character_card_pools"])
        if pooled != set(self.facts_payload["character_pool_order"]):
            raise DllRefusal(
                f"{path.name}: `character_card_pools` and "
                "`character_pool_order` name different characters")

    # -- provenance --------------------------------------------------------

    @property
    def provenance(self):
        return {"manifest": str(self.path), "sha256": self.sha256}

    def model_id_hash(self):
        """The `modelIdHash` recorded when the assembly was read.

        Recomputed from the DLL by `DllFacts`; carried here because it is the
        value the game itself writes into every `.mcr` header, so a captured
        replay remains an external witness for the inventory in this file.
        """
        return self.provenance_record.get("model_id_hash")

    # -- the recorded inventory -------------------------------------------

    def card_ids(self):
        return list(self.facts_payload["card_ids"])

    def relic_ids(self):
        return list(self.facts_payload["relic_ids"])

    def potion_ids(self):
        return list(self.facts_payload["potion_ids"])

    def enchantment_ids(self):
        return list(self.facts_payload["enchantment_ids"])

    def encounter_inventory(self):
        return list(self.facts_payload["encounter_inventory"])

    def encounter_spellings(self):
        spellings = set()
        for entry in self.encounter_inventory():
            spellings.add(entry)
            spellings.add(_strip_elite(entry))
        return spellings

    def monster_inventory(self):
        return list(self.facts_payload["monster_inventory"])

    def monster_models(self):
        return dict(self.facts_payload["monster_models"])

    def monster_move_constants(self):
        return dict(self.facts_payload["monster_move_constants"])

    def monster_ctor_ints(self):
        return dict(self.facts_payload["monster_ctor_ints"])

    def encounter_rng_draws(self):
        return dict(self.facts_payload["encounter_rng_draws"])

    def epochs(self):
        return list(self.facts_payload["epochs"])

    # -- card columns ------------------------------------------------------

    def card(self, card_id):
        row = self.facts_payload["cards"].get(card_id)
        if row is None:
            raise DllRefusal(
                f"card:{card_id} has no row in {self.path.name}; the manifest "
                f"was written from a build that has no such card")
        return row

    def card_type_members(self):
        return sorted((value, name)
                      for value, name in
                      self.facts_payload["card_type_members"])

    def card_rarity_vocabulary(self):
        return list(self.facts_payload["card_rarity_vocabulary"])

    def native_unplayable_keys(self):
        return [(card_id, upgrade) for card_id, upgrade
                in self.facts_payload["native_unplayable_keys"]]

    # -- potion columns and the generation pools (#2542) -------------------

    def potions(self):
        return {potion_id: dict(row) for potion_id, row
                in self.facts_payload["potions"].items()}

    def character_pool_order(self):
        return list(self.facts_payload["character_pool_order"])

    def character_unlock_epochs(self):
        return {character: list(epochs) for character, epochs
                in self.facts_payload["character_unlock_epochs"].items()}

    def character_card_pools(self):
        return {character: [[card_id, epoch] for card_id, epoch in rows]
                for character, rows
                in self.facts_payload["character_card_pools"].items()}

    def colorless_card_pool(self):
        return [[card_id, epoch]
                for card_id, epoch in self.facts_payload["colorless_card_pool"]]

    def character_potion_pools(self):
        return {character: {"epoch": row["epoch"],
                            "potions": list(row["potions"])}
                for character, row
                in self.facts_payload["character_potion_pools"].items()}

    def shared_potion_pool(self):
        return [[potion_id, epoch] for potion_id, epoch
                in self.facts_payload["shared_potion_pool"]]

    def shared_card_pools(self):
        return [[pool, list(members)] for pool, members
                in self.facts_payload["shared_card_pools"]]

    def mad_science_variants(self):
        return json.loads(json.dumps(
            self.facts_payload["mad_science_variants"]))


def compare_manifest(facts, path=None):
    """Fact-by-fact comparison of a committed manifest against an assembly.

    Returns `(committed_or_None, notes)`. The notes are empty exactly when the
    manifest reproduces what this assembly says, key for key.
    """
    path = pathlib.Path(path) if path else default_manifest_path()
    if not path.exists():
        return None, [f"no committed manifest at {path}"]
    committed = ManifestFacts(path)
    fresh = build_manifest(facts)
    notes = []
    if committed.sha256 != facts.sha256:
        notes.append(
            f"records assembly sha256 {committed.sha256}, this one is "
            f"{facts.sha256}")
    if committed.model_id_hash() != facts.model_id_hash():
        notes.append(
            f"records modelIdHash {committed.model_id_hash()}, this assembly "
            f"recomputes {facts.model_id_hash()}")
    for key in MANIFEST_FACT_KEYS:
        recorded, current = committed.facts_payload.get(key), \
            fresh["facts"][key]
        if recorded == current:
            continue
        if key in ("cards", "potions"):
            recorded = recorded or {}
            moved = sorted(
                model_id for model_id in set(recorded) | set(current)
                if recorded.get(model_id) != current.get(model_id))
            notes.append(f"{key}: {len(moved)} rows differ {moved[:12]}")
        else:
            notes.append(f"{key}: differs")
    return committed, notes


# ---------------------------------------------------------------------------
# Content sources
# ---------------------------------------------------------------------------

#: The five keyword-backed booleans on a card row, and the CardKeyword each
#: one mirrors.
KEYWORD_COLUMNS = {
    "exhausts": "Exhaust",
    "ethereal": "Ethereal",
    "innate": "Innate",
    "retain": "Retain",
    "sly": "Sly",
}


def x_cost_canonical(ctor_cost, x_cost):
    """The live canonical energy cost for a ``canonicalEnergyCost`` argument.

    The manifest records the ``CardModel::.ctor`` argument as written, but
    what the game spends and modifies is ``CardEnergyCost._base``. v111
    ``CardEnergyCost::.ctor`` RVA 0x11e002 branches on ``get_CostsX``
    (IL_0022, ``brtrue`` IL_0027) and stores ``ldc.i4.0`` (IL_002c) into
    ``Canonical`` for an X-cost card, then copies ``Canonical`` into
    ``_base`` (IL_0039). Cascade passes -1 (``Cascade::.ctor`` RVA 0xdab73
    IL_0002) and so lands on 0 (#3147). ``x_cost`` is the modeling annex's
    ``HasEnergyCostX`` column: the manifest does not read that getter.
    """
    return 0 if x_cost else ctor_cost


class PythonSource:
    """Every fact comes from the frozen v0.111.0 Python registry.

    Since #2827 item F that registry is only the committed snapshot
    (`data/python_registry.v0.111.0.json`); the simulator it was recorded
    from is deleted. `--source auto` lands here only when neither the
    assembly nor the manifest is readable, and says so. Its methods are the
    complete list of lookups `--source dll` can redirect — anything not
    routed through here is, by construction, in `MODELING_ANNEX`.
    """

    name = "python"

    def describe(self):
        return ("the frozen Python registry snapshot "
                "(data/python_registry.v0.111.0.json)")

    # -- id axes -----------------------------------------------------------

    def card_ids(self, cs):
        return sorted({key[0] for key in cs.CARDS})

    def relic_ids(self, cs):
        return sorted(cs.KNOWN_RELICS)

    def potion_ids(self, cs):
        return sorted(cs.KNOWN_POTIONS)

    def enchantment_ids(self, cs):
        return sorted(cs.KNOWN_ENCHANTMENTS)

    # -- card columns ------------------------------------------------------

    def card_rarity_vocabulary(self, cs):
        return sorted(set(cs._CARD_RARITY_BY_ID.values()))

    def card_rarity(self, cs, card_id):
        return cs._CARD_RARITY_BY_ID[card_id]

    def card_type(self, cs, card_id):
        return cs._CARD_TYPE_BY_ID[card_id]

    def card_cost(self, cs, card_id, upgrade, row):
        return row.cost

    def card_target_type(self, cs, card_id, row):
        return row.target_type

    def card_tags(self, cs, card_id, row):
        return sorted(row.tags)

    def card_strike_tag(self, cs, card_id, row):
        return bool(row.strike_tag)

    def card_keyword(self, cs, card_id, upgrade, row, column):
        return bool(getattr(row, column))

    def native_unplayable_keys(self, cs):
        return sorted(cs._NATIVE_UNPLAYABLE_CARD_KEYS)

    # -- per-monster model facts -------------------------------------------

    def monster_initial_hp(self, cs):
        """`{MONSTER entry: {min_initial_hp, max_initial_hp}}`.

        The registry has no analogue to lift: `combat_sim` spends these
        numbers as literals inside the `content/encounters/*.py` roster
        builders, never as a table. So the `python` path reads the committed
        manifest — the same precedent `epoch_universe` above already sets by
        reading `mcr_tables.json`, and for the same reason: a DLL-derived
        artifact that is committed is a legitimate source on a path that has
        no assembly. `--source dll` re-derives it and is the authority.
        """
        return self._committed_monster_models()

    def monster_initial_powers(self, cs):
        """`{MONSTER entry: {initial_powers}}`; see `monster_initial_hp`."""
        return self._committed_monster_models()

    def monster_ctor_ints(self, cs):
        """`{MONSTER entry: {field: int}}`; see `monster_initial_hp`.

        `combat_sim` has no table of these either — `Monster.tus`'s `2` is a
        keyword argument at each construction site — so the `python` path
        reads the committed manifest too.
        """
        return ManifestFacts().monster_ctor_ints()

    @staticmethod
    def _committed_monster_models():
        return ManifestFacts().monster_models()

    def monster_move_constants(self, cs):
        """`{MONSTER entry: {getter or inline site: tier}}` (#2828).

        As with `monster_initial_hp`, the registry holds these only as
        `AscensionTier` literals scattered through the move tables, never as
        a per-class table, so the `python` path reads the committed manifest.
        """
        return ManifestFacts().monster_move_constants()

    def encounter_rng_draws(self, cs):
        """`{ENCOUNTER entry: {draws} | {refused}}`; see `monster_initial_hp`.

        `content/encounters/*.py` spells these bounds as call-site literals
        (`rng.next_int(2, 10)`), never as a table, so the `python` path reads
        the committed manifest for the same reason the monster models do.
        """
        return ManifestFacts().encounter_rng_draws()
    # -- generation pools (#2542) ------------------------------------------
    #
    # On the Python path these come from the harness oracle probes
    # (`_CARD_POOL_CENSUS`, `_POTION_POOL_CENSUS`), which is where they lived
    # before #2542. The DLL path re-sources every one of them; the two agree
    # row for row on v0.111.0, which is what `test_dll_content_source.py`
    # asserts.

    @staticmethod
    def _pool_row(cs, character, card_id):
        for row in cs._CARD_POOL_CENSUS["pools"][character]["cards"]:
            if row["id"].removeprefix("CARD.") == card_id:
                return row
        return None

    def _any_pool_row(self, cs, card_id):
        for character in cs._CARD_POOL_CENSUS["character_pool_order"]:
            row = self._pool_row(cs, character, card_id)
            if row is not None:
                return row
        # #2512: a Colorless card is in no CHARACTER pool, so the harness
        # census cannot answer for it — but `colorless_card_pool.json`
        # carries the same two columns for all 65 Colorless rows, read from
        # the same assembly. Before that file existed this raised, which is
        # why `--source python` could not emit the Colorless pool table.
        for row in cs._COLORLESS_CARD_POOL_CENSUS["cards"]:
            if row["id"].removeprefix("CARD.") == card_id:
                return row
        raise DllRefusal(
            f"card:{card_id} is in no character or Colorless card pool, so "
            "neither committed census records its generation columns; only "
            "the assembly can answer for it (`--source dll`)")

    def card_multiplayer_constraint(self, cs, card_id):
        return self._any_pool_row(cs, card_id)["multiplayer_constraint"]

    def card_can_be_generated_in_combat(self, cs, card_id):
        return bool(
            self._any_pool_row(cs, card_id)["can_be_generated_in_combat"])

    @staticmethod
    def _potion_row(cs, potion_id):
        row = cs._CHARACTER_POTION_POOL_CENSUS["rows"].get("POTION." + potion_id)
        if row is not None:
            return row
        raise DllRefusal(
            f"potion:{potion_id} is in no generation potion pool, so the "
            "census records neither of its generation columns")

    def potion_rarity(self, cs, potion_id):
        return self._potion_row(cs, potion_id)["rarity"]

    def potion_can_be_generated_in_combat(self, cs, potion_id):
        return bool(
            self._potion_row(cs, potion_id)["can_be_generated_in_combat"])

    def character_pool_order(self, cs):
        return list(cs._CARD_POOL_CENSUS["character_pool_order"])

    def character_unlock_epochs(self, cs):
        return {character: sorted(epochs) for character, epochs
                in cs._CARD_POOL_CENSUS["character_unlock_epochs"].items()}

    def character_card_pools(self, cs):
        return {
            character: [[row["id"].removeprefix("CARD."),
                         row["unlock_epoch"]]
                        for row in cs._CARD_POOL_CENSUS[
                            "pools"][character]["cards"]]
            for character in cs._CARD_POOL_CENSUS["character_pool_order"]}

    def colorless_card_pool(self, cs):
        return [[row["id"].removeprefix("CARD."), row["unlock_epoch"]]
                for row in cs._COLORLESS_CARD_POOL_CENSUS["cards"]]

    def character_potion_pools(self, cs):
        census = cs._CHARACTER_POTION_POOL_CENSUS
        return {character: {"epoch": row["epoch"],
                            "potions": [potion.removeprefix("POTION.")
                                        for potion in row["potions"]]}
                for character, row in census["pools"].items()}

    def shared_potion_pool(self, cs):
        return [[potion_id.removeprefix("POTION."), epoch]
                for potion_id, epoch
                in cs._CHARACTER_POTION_POOL_CENSUS["shared"]]

    def shared_card_pools(self, cs):
        """`[[POOL, [[card_id, multiplayer_constraint, in_combat], ...]]]`.

        #2734. The registry never recorded the six non-Colorless shared pools
        (`combat_sim` hand-wrote its Entropy token/status tables instead), so
        the `python` path reads the committed manifest, the precedent
        `monster_ctor_ints` sets. The two generation columns ride along
        because `card_can_be_generated_in_combat` above can only answer for
        character and Colorless rows on this path.
        """
        facts = ManifestFacts()
        return [[pool, [[card_id,
                         facts.card(card_id)["multiplayer_constraint"],
                         bool(facts.card(card_id)[
                             "can_be_generated_in_combat"])]
                        for card_id in members]]
                for pool, members in facts.shared_card_pools()]

    def mad_science_variants(self, cs):
        """The saved Tinker Time domain (#2942).

        The frozen registry never recorded it: `combat_sim` hand-wrote
        `_MAD_SCIENCE_LEGAL_RIDERS`. So the `python` path reads the committed
        manifest, the `shared_card_pools` precedent above.
        """
        return ManifestFacts().mad_science_variants()

    # -- other -------------------------------------------------------------

    def epoch_universe(self, cs, solver_dir):
        tables = json.loads((solver_dir / "mcr_tables.json").read_text())
        return tables["epochs"], tables


class DllContentSource(PythonSource):
    """`--source dll`: the facts in `DLL_SOURCED_FACTS` come from `sts2.dll`.

    Everything else is inherited from `PythonSource` — deliberately, and only
    for the entries named in `MODELING_ANNEX`. Where an axis exists on both
    sides the DLL value is not merely preferred, it is *reconciled*: a
    disagreement raises `DllRefusal` naming the difference, because on the
    next build a disagreement is exactly the porting work that must not be
    skipped silently.
    """

    name = "dll"

    def __init__(self, facts):
        self.facts = facts

    def describe(self):
        return (f"{self.facts.path.name} sha256 {self.facts.sha256} "
                f"(+ the Python modeling annex)")

    # -- reconciliation ----------------------------------------------------

    @staticmethod
    def _reconcile(axis, dll_values, python_values):
        dll_set, python_set = set(dll_values), set(python_values)
        if dll_set == python_set:
            return list(dll_values)
        raise DllRefusal(
            f"{axis}: the assembly and the modeled registry disagree. "
            f"In the DLL only: {sorted(dll_set - python_set)}. "
            f"Modeled only: {sorted(python_set - dll_set)}. "
            "On a new build the first list is content to port and the second "
            "is content that was removed; neither may be resolved here.")

    def card_ids(self, cs):
        return self._reconcile("CardId", self.facts.card_ids(),
                               super().card_ids(cs))

    def relic_ids(self, cs):
        return self._reconcile("RelicId", self.facts.relic_ids(),
                               super().relic_ids(cs))

    def potion_ids(self, cs):
        return self._reconcile("PotionId", self.facts.potion_ids(),
                               super().potion_ids(cs))

    def enchantment_ids(self, cs):
        return self._reconcile("EnchantmentId", self.facts.enchantment_ids(),
                               super().enchantment_ids(cs))

    # -- card columns ------------------------------------------------------

    def card_rarity_vocabulary(self, cs):
        return self.facts.card_rarity_vocabulary()

    def card_rarity(self, cs, card_id):
        stale = STALE_REPO_DATA.get((card_id, "rarity"))
        if stale is not None:
            return stale[1]
        return self.facts.card(card_id)["rarity"]

    def card_type(self, cs, card_id):
        return self.facts.card(card_id)["type"]

    def card_cost(self, cs, card_id, upgrade, row):
        if upgrade != 0:
            # MODELING_ANNEX: upgrade deltas live in OnUpgrade.
            return super().card_cost(cs, card_id, upgrade, row)
        return x_cost_canonical(self.facts.card(card_id)["cost"], row.x_cost)

    def card_target_type(self, cs, card_id, row):
        if self.facts.card(card_id)["dynamic_target"]:
            # MODELING_ANNEX: the constructor argument is not the effective
            # target kind for a class that overrides the getter.
            return super().card_target_type(cs, card_id, row)
        return self.facts.card(card_id)["target_type"]

    def card_tags(self, cs, card_id, row):
        stale = STALE_REPO_DATA.get((card_id, "tags"))
        if stale is not None:
            return sorted(stale[1])
        return sorted(self.facts.card(card_id)["tags"])

    def card_strike_tag(self, cs, card_id, row):
        return "Strike" in self.card_tags(cs, card_id, row)

    def card_keyword(self, cs, card_id, upgrade, row, column):
        if upgrade != 0:
            # MODELING_ANNEX: upgrade deltas live in OnUpgrade.
            return super().card_keyword(cs, card_id, upgrade, row, column)
        return KEYWORD_COLUMNS[column] in self.facts.card(card_id)["keywords"]

    def native_unplayable_keys(self, cs):
        return sorted(self.facts.native_unplayable_keys())

    # -- per-monster model facts -------------------------------------------

    def monster_initial_hp(self, cs):
        return self.facts.monster_models()

    def monster_initial_powers(self, cs):
        return self.facts.monster_models()

    def monster_move_constants(self, cs):
        return self.facts.monster_move_constants()

    def monster_ctor_ints(self, cs):
        return self.facts.monster_ctor_ints()

    def encounter_rng_draws(self, cs):
        return self.facts.encounter_rng_draws()
    # -- generation pools (#2542) ------------------------------------------

    def card_multiplayer_constraint(self, cs, card_id):
        return self.facts.card(card_id)["multiplayer_constraint"]

    def card_can_be_generated_in_combat(self, cs, card_id):
        return bool(self.facts.card(card_id)["can_be_generated_in_combat"])

    def potion_rarity(self, cs, potion_id):
        return self.facts.potions()[potion_id]["rarity"]

    def potion_can_be_generated_in_combat(self, cs, potion_id):
        return bool(
            self.facts.potions()[potion_id]["can_be_generated_in_combat"])

    def character_pool_order(self, cs):
        return self._reconcile(
            "character pool order", self.facts.character_pool_order(),
            super().character_pool_order(cs))

    def character_unlock_epochs(self, cs):
        return self.facts.character_unlock_epochs()

    def character_card_pools(self, cs):
        return self.facts.character_card_pools()

    def colorless_card_pool(self, cs):
        return self.facts.colorless_card_pool()

    def character_potion_pools(self, cs):
        return self.facts.character_potion_pools()

    def shared_potion_pool(self, cs):
        return self.facts.shared_potion_pool()

    def shared_card_pools(self, cs):
        return [[pool, [[card_id,
                         self.facts.card(card_id)["multiplayer_constraint"],
                         bool(self.facts.card(card_id)[
                             "can_be_generated_in_combat"])]
                        for card_id in members]]
                for pool, members in self.facts.shared_card_pools()]

    def mad_science_variants(self, cs):
        return self.facts.mad_science_variants()

    # -- other -------------------------------------------------------------

    def epoch_universe(self, cs, solver_dir):
        _python_epochs, tables = super().epoch_universe(cs, solver_dir)
        return self._reconcile("epochs", self.facts.epochs(),
                               tables["epochs"]), tables


class ManifestContentSource(DllContentSource):
    """`--source manifest`: the DLL's facts, replayed from the committed file.

    Deliberately a *subclass* of `DllContentSource` rather than a sibling with
    its own copy of the lookups. Every routed fact must resolve to the same
    implementation on both paths, so the manifest path cannot silently take a
    registry value for a column the DLL path sources — the identical failure
    mode `EveryClaimedDllFactIsActuallyRouted` exists to catch on the DLL
    path. Only the provenance line differs.

    The facts object is `ManifestFacts`, which needs neither `dnfile` nor an
    `sts2.dll`. That is what makes the `rust port` workflow's codegen
    freshness step a real check again on a forked crate (#2515).
    """

    name = "manifest"

    def describe(self):
        record = self.facts.provenance_record
        return (f"{self.facts.path.name} — {record.get('dll')} sha256 "
                f"{self.facts.sha256}, modelIdHash "
                f"{record.get('model_id_hash')}, build {self.facts.build} "
                f"({record.get('commit')}) (+ the Python modeling annex)")


def make_source(kind, dll_path=None, *, require_certified=False,
                manifest=None):
    """`"python"`/`"dll"`/`"manifest"` -> the matching content source."""
    if kind == "python":
        return PythonSource()
    if kind == "dll":
        return DllContentSource(
            DllFacts(dll_path, require_certified=require_certified))
    if kind == "manifest":
        return ManifestContentSource(
            ManifestFacts(manifest, require_certified=require_certified))
    raise ValueError(f"unknown content source {kind!r}")


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

def print_ledger(stream=sys.stdout):
    print("DLL-SOURCED FACTS (follow the assembly `--source dll` is pointed "
          "at)", file=stream)
    for fact, (where, feeds) in DLL_SOURCED_FACTS.items():
        print(f"\n  {fact}\n    from: {where}\n    into: {feeds}", file=stream)
    print("\n\nMODELING ANNEX (not in the assembly as a readable table; "
          "declared, never silently retained)", file=stream)
    for fact, (why, where) in MODELING_ANNEX.items():
        print(f"\n  {fact}\n    why:  {why}\n    from: {where}", file=stream)
    print("\n\nSTALE REPO DATA (the DLL and the committed tables disagree "
          "today; applied so `--source dll` reproduces committed bytes)",
          file=stream)
    for (card_id, fact), entry in STALE_REPO_DATA.items():
        dll_value, committed, origin, why = entry
        print(f"\n  {card_id}.{fact}: DLL {dll_value!r} vs committed "
              f"{committed!r} (from {origin})\n    {why}", file=stream)
    print("\n\nCOMMITTED MANIFEST (`--source manifest`: the DLL-sourced half "
          "above, replayed with no assembly and no dnfile)", file=stream)
    print(f"\n  file:   {default_manifest_path()}"
          f"\n  schema: {MANIFEST_SCHEMA}"
          f"\n  keys:   {', '.join(MANIFEST_FACT_KEYS)}"
          "\n  write:  `generate_content.py --source dll` writes it and then "
          "generates from it, so a fact\n          missing here fails the DLL "
          "path too, not only the manifest path",
          file=stream)


def _report(facts, cs):
    python = PythonSource()
    rows = []

    def compare(label, dll_values, python_values):
        dll_set, python_set = set(dll_values), set(python_values)
        rows.append((label, len(dll_set), len(python_set),
                     sorted(dll_set - python_set),
                     sorted(python_set - dll_set)))

    compare("CardId", facts.card_ids(), python.card_ids(cs))
    compare("RelicId", facts.relic_ids(), python.relic_ids(cs))
    compare("PotionId", facts.potion_ids(), python.potion_ids(cs))
    compare("EnchantmentId", facts.enchantment_ids(),
            python.enchantment_ids(cs))
    compare("epochs", facts.epochs(),
            python.epoch_universe(cs, BUILD_DIR / "solver")[0])
    print(f"assembly: {facts.path}")
    print(f"sha256:   {facts.sha256}"
          + ("  (certified v0.111.0)"
             if facts.sha256 == CERTIFIED_DLL_SHA256 else "  (NOT certified)"))
    print(f"modelIdHash: {facts.model_id_hash()}")
    print()
    for label, n_dll, n_python, only_dll, only_python in rows:
        status = "agree" if not only_dll and not only_python else "DIFFER"
        print(f"{label:44s} dll={n_dll:4d} python={n_python:4d}  {status}")
        if only_dll:
            print(f"    dll only:    {only_dll[:12]}")
        if only_python:
            print(f"    python only: {only_python[:12]}")

    # Annex axes: the DLL does not DEFINE these, but it does BOUND them, and
    # the bound is one-directional on purpose. A modeled id with no entry
    # behind it is a defect; an entry with no model is just unported content.
    import generate_content  # local: generate_content imports this module

    print()
    for label, modeled, backing, known_aggregates in (
            ("EncounterId", sorted(cs.SUPPORTED_ENCOUNTERS),
             facts.encounter_spellings(), ()),
            ("MonsterKind", generate_content.monster_kinds(cs),
             set(facts.monster_inventory()), ("DECIMILLIPEDE_SEGMENT",))):
        unbacked = [name for name in modeled
                    if name not in backing and name not in known_aggregates]
        print(f"{label} (annex: modeled, DLL-bounded)  modeled="
              f"{len(modeled)} unbacked={len(unbacked)}"
              + (f"  {unbacked[:12]}" if unbacked else "")
              + (f"  [declared aggregates: {list(known_aggregates)}]"
                 if known_aggregates else ""))

    mismatches = {"rarity": [], "type": [], "cost@0": [], "target_type": [],
                  "tags": [], "keywords@0": []}
    for card_id in facts.card_ids():
        card = facts.card(card_id)
        if card["rarity"] != python.card_rarity(cs, card_id):
            mismatches["rarity"].append(
                (card_id, card["rarity"], python.card_rarity(cs, card_id)))
        if card["type"] != python.card_type(cs, card_id):
            mismatches["type"].append(
                (card_id, card["type"], python.card_type(cs, card_id)))
        row = cs.CARDS.get((card_id, 0))
        if row is None:
            continue
        cost = x_cost_canonical(card["cost"], row.x_cost)
        if cost != row.cost:
            mismatches["cost@0"].append((card_id, cost, row.cost))
        if not card["dynamic_target"] \
                and card["target_type"] != row.target_type:
            mismatches["target_type"].append(
                (card_id, card["target_type"], row.target_type))
        if sorted(card["tags"]) != sorted(row.tags):
            mismatches["tags"].append(
                (card_id, sorted(card["tags"]), sorted(row.tags)))
        for column, keyword in KEYWORD_COLUMNS.items():
            if (keyword in card["keywords"]) != bool(getattr(row, column)):
                mismatches["keywords@0"].append(
                    (card_id, column, keyword in card["keywords"]))

    print("\nper-card columns, DLL vs the modeled registry:")
    for column, found in mismatches.items():
        print(f"  {column:14s} {len(found):3d} disagreements"
              + (f"  {found[:6]}" if found else ""))
    # The hand-written half of the boundary: `CardType`'s members live in
    # generate_content.TABLE_TYPES with explicit native values, so nothing
    # regenerates them and nothing else would notice them rotting.
    emitted = generate_content.TABLE_TYPES
    drifted = [f"{name} = {value}"
               for value, name in facts.card_type_members()
               if f"    {name} = {value}," not in emitted]
    print("\nhand-written CardType enum vs the assembly: "
          + ("matches" if not drifted else f"DRIFTED {drifted}"))

    print("\ndeclared stale-repo-data entries: "
          f"{len(STALE_REPO_DATA)} (see --ledger)")

    # The committed manifest is what CI generates from, so a drift between it
    # and the assembly is a drift between what CI checks and what the build
    # actually contains. This is the only place both are readable at once.
    committed, notes = compare_manifest(facts)
    print("\ncommitted content manifest vs this assembly: ", end="")
    if committed is None:
        print(f"ABSENT — {notes[0]}")
    elif not notes:
        print(f"{committed.path.name} reproduces every fact "
              f"({len(MANIFEST_FACT_KEYS)} keys, "
              f"{len(committed.card_ids())} cards)")
    else:
        print("DIFFERS")
        for note in notes:
            print(f"    {note}")
        print("    re-run: generate_content.py --source dll")
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--ledger", action="store_true",
                        help="print the declared source boundary and exit "
                             "(needs no DLL and no dnfile)")
    parser.add_argument("--report", action="store_true",
                        help="read the assembly and compare every DLL-sourced "
                             "fact against the frozen Python registry "
                             "snapshot and against the committed manifest")
    parser.add_argument("--dll", default=None,
                        help="assembly to read (default: the archived "
                             f"{CERTIFIED_BUILD} build, else $STS2_DLL, else "
                             "the Steam install)")
    args = parser.parse_args(argv)

    if args.ledger or not args.report:
        print_ledger()
        if not args.report:
            return 0
        print("\n" + "=" * 72 + "\n")

    try:
        facts = DllFacts(args.dll)
    except DllUnavailable as exc:
        print(f"SKIP: {exc}", file=sys.stderr)
        return 0

    # The registry side is the frozen v0.111.0 Python registry recorded as
    # data (`data/python_registry.v0.111.0.json`, #2827 item D), the same
    # object `generate_content.py --registry frozen` replays. #2999 moved the
    # report off `import combat_sim`, which item F deletes; on the certified
    # assembly the two produce byte-identical reports (measured in the #2999
    # walk). An attribute or census the snapshot never recorded raises
    # `FrozenRegistryMiss` naming it rather than reporting a guess.
    import registry_snapshot  # noqa: E402  (a sibling of this module)

    return _report(facts, registry_snapshot.load_snapshot())


if __name__ == "__main__":
    raise SystemExit(main())

"""The eval fixture set: manifest integrity, provenance, and Rust lockstep.

The fixtures under `versions/v0.111.0/eval/` are real captured fights (#2048;
consent recorded there on 2026-09-05). They carry an opaque id, the canonical
entry, sha256 provenance of the capture pair, and — for a fight whose recorded
human line replays exactly in Python — that line in canonical wire form with a
`differential_digest` pinned after every single action.

What runs everywhere: the manifest and the fixture files have to agree, every
recorded root digest has to reproduce from the stored canonical entry, and
every category the seed set claims has to be populated.

What is deliberately opt-in:

* `STS2_EVAL_RUST_LOCKSTEP=1` replays each certified line through the Rust
  engine. It needs `versions/v0.111.0/rust/target/release/sts-sim`, which is
  a build artifact of a crate the fast gate's TRIGGER_SET deliberately
  excludes (#1276). Binding the solver suite to whatever binary happens to be
  in a shared checkout would report divergences that are really staleness, so
  the evidence is produced on demand and recorded in the PR.
* `STS2_EVAL_REDERIVE=1` re-derives every fixture from its provenance through
  the tool's own `verify` path. It needs the local capture corpus
  (`~/sts2-captures`, or `STS2_CAPTURES`), which exists on no CI checkout.
"""

from __future__ import annotations

import json
import os
import pathlib
import subprocess
import sys

import pytest

HERE = pathlib.Path(__file__).resolve().parent
VERSION_DIR = HERE.parent
RUST_DIR = VERSION_DIR / "rust"
EVAL_DIR = VERSION_DIR / "eval"
MANIFEST_PATH = EVAL_DIR / "manifest.json"
FIGHTS_DIR = EVAL_DIR / "fights"
EVAL_TOOL = RUST_DIR / "tools" / "eval_suite.py"
ENGINE_BINARY = RUST_DIR / "target" / "release" / "sts-sim"

sys.path.insert(0, str(RUST_DIR / "tools"))

import eval_suite  # noqa: E402
# The simulator-free document layer (#2827 item D): this module needs
# the digest and the build stamp, never the projector (#2999).
import canonical_document  # noqa: E402

# Every category the seed set is required to populate. `human` is the baseline
# tag that licenses a solver-versus-human statement. It belongs only to fights
# whose ascension makes the comparison meaningful, A9/A10 for every character
# since 2026-09-23 (#2915), so it is pinned separately from the per-character
# coverage tags.
REQUIRED_CATEGORIES = (
    "short", "long", "human", "solvable", "certified", "death", "potions",
    "refusal",
    "multi:IRONCLAD", "multi:SILENT", "multi:DEFECT", "multi:NECROBINDER",
    "multi:REGENT", "node:monster", "node:elite", "node:boss",
)
CHARACTERS = ("IRONCLAD", "SILENT", "DEFECT", "NECROBINDER", "REGENT")
#: (character, node) pairs with no certifiable A9/A10 human line yet, each
#: naming the issue that tracks the engine gap.
KNOWN_LINE_GAPS: dict = {}
# `refusal:rust-opening` left this list with #3381: #3379 and #3381 certified
# the last fixtures refused at the opening, so none remain to require.
# `refusal:rust-line` left it with the group-7 divergence wave: no A9/A10
# corpus line diverges any more (see
# `test_refusal_fixtures_name_a_surface_and_quote_the_refusal`).
REQUIRED_REFUSAL_SURFACES = (
    "refusal:rust-load",
)
# A fixture is provenance and projections, never a player: no capture file
# names, no home directories, no wall-clock run identity.
FORBIDDEN_SUBSTRINGS = (
    "/Users/", "sts2-captures", "start_time", "steamid", "76561198",
    ".mcr", ".save", "username",
)


def _manifest() -> dict:
    return json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))


def _fixture(fight_id: str, name: str) -> dict:
    return json.loads(
        (FIGHTS_DIR / fight_id / name).read_text(encoding="utf-8"))


def test_manifest_and_fixture_directories_agree():
    manifest = _manifest()
    assert manifest["schema"] == "sts-eval-manifest-v1"
    assert manifest["build"] == canonical_document.GAME_BUILD
    assert "consent" in manifest and "opaque ids" in manifest["consent"]

    ids = [fight["id"] for fight in manifest["fights"]]
    assert ids == sorted(ids), "manifest fights are not id-ordered"
    assert len(ids) == len(set(ids)), "duplicate fixture id"
    assert {path.name for path in FIGHTS_DIR.iterdir() if path.is_dir()} == \
        set(ids), "fixture directories and manifest disagree"

    for fight in manifest["fights"]:
        directory = FIGHTS_DIR / fight["id"]
        on_disk = sorted(path.name for path in directory.iterdir())
        assert on_disk == sorted(fight["files"]), fight["id"]
        assert "provenance.json" in on_disk, fight["id"]
        assert fight["kind"] in ("line", "refusal"), fight["id"]
        if fight["kind"] == "line":
            assert {"entry.canonical.json", "human_line.json"} <= set(on_disk)
        else:
            assert "refusal.json" in on_disk, fight["id"]


def test_provenance_is_complete_and_carries_no_identity():
    manifest = _manifest()
    for fight in manifest["fights"]:
        provenance = _fixture(fight["id"], "provenance.json")
        assert provenance["schema"] == "sts-eval-provenance-v1"
        assert provenance["build"] == canonical_document.GAME_BUILD
        assert provenance["character"] == fight["character"]
        assert provenance["ascension"] == fight["ascension"]
        assert provenance["seed"] == fight["seed"]
        assert provenance["node"] == fight["node"]
        assert len(provenance["capture_sha256"]) == 64
        if provenance.get("save_sha256") is not None:
            assert len(provenance["save_sha256"]) == 64
        # The id is opaque: it is derived from the capture identity, and
        # nothing about the fixture reveals who played it.
        assert fight["id"].startswith("f") and len(fight["id"]) == 16

    # Every stored fixture document, plus the manifest's own per-fight rows.
    # (The manifest's consent note deliberately says the word "usernames".)
    payloads = [(path.name, path.read_text(encoding="utf-8"))
                for path in sorted(FIGHTS_DIR.rglob("*.json"))]
    payloads.append(("manifest.json:fights", json.dumps(manifest["fights"])))
    for name, text in payloads:
        for forbidden in FORBIDDEN_SUBSTRINGS:
            assert forbidden not in text, (name, forbidden)


def test_every_root_digest_reproduces_from_its_canonical_entry():
    """The manifest's digest is the one the canonical root actually hashes to.

    This is the CI-runnable half of "every root reproduces from provenance":
    it needs no capture corpus, and it fails the moment a fixture is edited by
    hand or the canonical schema moves under the stored documents.
    """
    manifest = _manifest()
    rooted = 0
    for fight in manifest["fights"]:
        if "entry.canonical.json" not in fight["files"]:
            continue
        rooted += 1
        entry = _fixture(fight["id"], "entry.canonical.json")
        assert entry["schema"] == canonical_document.SCHEMA, fight["id"]
        assert entry["game_build"] == canonical_document.GAME_BUILD, fight["id"]
        digest = canonical_document.differential_digest(entry)
        assert digest == fight["entry_digest"], fight["id"]
        provenance = _fixture(fight["id"], "provenance.json")
        assert digest == provenance["entry_digest"], fight["id"]
    assert rooted >= 27, rooted


def test_human_lines_pin_every_action_and_its_digest():
    manifest = _manifest()
    lines = [fight for fight in manifest["fights"] if fight["kind"] == "line"]
    assert len(lines) >= 27, len(lines)
    for fight in lines:
        line = _fixture(fight["id"], "human_line.json")
        assert line["schema"] == "sts-eval-human-line-v1"
        assert line["entry_digest"] == fight["entry_digest"], fight["id"]
        assert len(line["actions"]) == len(line["step_digests"]) == \
            fight["actions"], fight["id"]
        assert line["actions"], fight["id"]
        for action in line["actions"]:
            assert action["kind"] in (
                "play", "end", "potion", "select", "enemy_choice"), action
            assert set(action) <= {
                "kind", "uid", "target", "selection", "answer", "slot",
                "index", "card", "choice"}, action
        for digest in line["step_digests"]:
            assert len(digest) == 64, fight["id"]
        terminal = line["terminal"]
        assert terminal["hp"] == fight["terminal_hp"], fight["id"]
        assert terminal["won"] == fight["won"], fight["id"]
        assert terminal["turn"] == fight["turns"], fight["id"]
        # A death is a real outcome the suite needs; a won fight ends alive.
        assert terminal["won"] == (terminal["over"] and terminal["hp"] > 0)


def test_refusal_fixtures_name_a_surface_and_quote_the_refusal():
    manifest = _manifest()
    refusals = [fight for fight in manifest["fights"]
                if fight["kind"] == "refusal"]
    # No count floor: every fix that certifies a refusal fixture shrinks this
    # set (#3367 and #3375 took it from 11 to 9). The bar is the surface set
    # asserted below — each surface keeps at least one quoted refusal.
    assert refusals
    surfaces = set()
    for fight in refusals:
        document = _fixture(fight["id"], "refusal.json")
        assert document["schema"] == "sts-eval-refusal-v1"
        # Since the #2915 re-seed every fixture is Rust-seeded, so no
        # frozen-oracle `python-*` surface remains.
        assert document["surface"] in eval_suite.REFUSAL_SURFACES, fight["id"]
        assert document["detail"], fight["id"]
        assert document["class"] == fight["refusal_class"], fight["id"]
        if document["surface"] == "rust-load":
            assert document["kind"], fight["id"]
        surfaces.add(document["surface"])
    # `rust-opening` left with #3381 (see REQUIRED_REFUSAL_SURFACES).
    # `rust-line` (a recorded line Rust refuses mid-fight) left the A9/A10
    # fixture tree when the group-7 divergence wave certified its last three
    # members (f5350e1315c43ecf, f9eaf3c40e7d1fbf, ff4902c21ff9b9c5). The one
    # corpus line still diverging on that head (fb421c3b04df9382) is below
    # A9, so it cannot be a fixture. The surfaces that still hold a refusal keep
    # their bar. A new rust-line refusal fixture must still quote its refusal,
    # which the loop above checks.
    assert {"rust-load"} <= surfaces


def test_every_seed_category_is_populated():
    manifest = _manifest()
    categories = manifest["categories"]
    by_id = {fight["id"]: fight for fight in manifest["fights"]}
    for name in REQUIRED_CATEGORIES + REQUIRED_REFUSAL_SURFACES:
        assert categories.get(name), name
        for fight_id in categories[name]:
            assert name in by_id[fight_id]["categories"], (name, fight_id)
    for fight in manifest["fights"]:
        for name in fight["categories"]:
            assert fight["id"] in categories[name], (name, fight["id"])

    # The `human` baseline tag is every A9/A10 human line, for every
    # character (Sean, 2026-09-23, #2915): a low-ascension fight is a
    # fixture, never a baseline, and a refusal has no line to compare with.
    for fight in manifest["fights"]:
        expected = (fight["kind"] == "line"
                    and fight["ascension"] >= eval_suite.SEED_MIN_ASCENSION)
        assert ("human" in fight["categories"]) == expected, fight["id"]
    for character in CHARACTERS:
        assert set(categories["human"]) & set(categories[f"multi:{character}"]), character
    # `certified` means Rust replayed the line, so the root must be admitted.
    for fight_id in categories["certified"]:
        assert by_id[fight_id]["rust_root"] == "admitted", fight_id
        assert by_id[fight_id]["lockstep"].startswith("lockstep_ok"), fight_id


def test_every_fixture_is_ascension_nine_or_ten():
    """The suite holds A9/A10 fights only (Sean, 2026-09-23, #2915)."""
    for fight in _manifest()["fights"]:
        assert fight["ascension"] >= eval_suite.SEED_MIN_ASCENSION, fight["id"]


def test_every_character_has_a_boss_and_an_elite_line():
    """#2915: the A0-A1 Defect set had no certified boss and hid a search
    weakness one A10 run exposed. Every character keeps a boss and an elite
    human line, so a solver change is measured on hard fights for all five."""
    manifest = _manifest()
    lines = {fight["id"] for fight in manifest["fights"]
             if fight["kind"] == "line"}
    categories = manifest["categories"]
    for character in CHARACTERS:
        mine = set(categories[f"multi:{character}"]) & lines
        for node in ("boss", "elite"):
            have = bool(mine & set(categories[f"node:{node}"]))
            if (character, node) in KNOWN_LINE_GAPS:
                # A named gap must still be a gap: once it closes, drop it.
                assert not have, (character, node, KNOWN_LINE_GAPS[(character, node)])
            else:
                assert have, (character, node)


def _seed_row(fight_id, character="DEFECT", encounter="ENCOUNTER.A",
              node_type="monster", certified=True, ascension=10, cards=()):
    return {
        "id": fight_id, "character": character, "encounter": encounter,
        "node_type": node_type, "ascension": ascension, "stage": "rooted",
        "human": "exact", "_line": {"actions": []}, "won": True, "turns": 3,
        "lockstep": "lockstep_ok" if certified else "checkpoint_mismatch",
        "_document": {"piles": {"draw": [{"id": card} for card in cards]},
                      "player": {}, "monsters": []},
    }


def test_certified_fights_are_capped_per_encounter_unless_they_add_a_kind():
    cap = eval_suite.SEED_CERTIFIED_PER_ENCOUNTER
    rows = [_seed_row(f"f{i:015d}", cards=("STRIKE",))
            for i in range(cap + 2)]
    rows.append(_seed_row("f999999999999999", cards=("STRIKE", "GLASSWORK")))
    chosen = {row["id"] for row in eval_suite.select_line_rows(rows, target=0)}
    assert len(chosen) == cap + 1
    # The only root holding GLASSWORK survives the cap; the kind is its witness.
    assert "f999999999999999" in chosen


def test_the_cap_keeps_fixtures_already_in_the_tree():
    """A re-seed replaces a fixture only when it must: a new capture with a
    lower id does not displace a committed fixture of the same encounter,
    and a committed certified fixture is kept while it certifies (the
    id-ordered cap had swapped ten still-certified fixtures)."""
    cap = eval_suite.SEED_CERTIFIED_PER_ENCOUNTER
    committed = [f"f9{i:014d}" for i in range(cap)]
    newcomers = [f"f0{i:014d}" for i in range(cap)]
    rows = [_seed_row(i, cards=("STRIKE",)) for i in committed + newcomers]
    chosen = {row["id"] for row in eval_suite.select_line_rows(
        rows, target=0, prefer=frozenset(committed))}
    assert chosen == set(committed)
    # Without the preference, the lower ids win, as before.
    chosen = {row["id"] for row in eval_suite.select_line_rows(rows, target=0)}
    assert chosen == set(newcomers)
    # A committed certified fixture stays even past the cap (an engine fix can
    # certify more of an encounter than the cap admits); only additions are
    # capped.
    over = [f"f9{i:014d}" for i in range(cap + 2)]
    rows = [_seed_row(i, cards=("STRIKE",)) for i in over + newcomers]
    chosen = {row["id"] for row in eval_suite.select_line_rows(
        rows, target=0, prefer=frozenset(over))}
    assert chosen == set(over)


def test_every_boss_and_elite_encounter_is_taken_certified_first():
    rows = [
        _seed_row("f000000000000001", node_type="boss",
                  encounter="ENCOUNTER.B", certified=False),
        _seed_row("f000000000000002", node_type="boss",
                  encounter="ENCOUNTER.B", certified=True),
        _seed_row("f000000000000003", node_type="elite",
                  encounter="ENCOUNTER.E", certified=False),
    ]
    chosen = {row["id"] for row in eval_suite.select_line_rows(rows, target=0)}
    # The certified boss line satisfies B; its uncertified twin is not added.
    assert "f000000000000002" in chosen
    # An uncertified elite is still taken when it is the encounter's only line.
    assert "f000000000000003" in chosen
    assert "f000000000000001" not in chosen


def test_seed_selection_drops_fights_below_the_ascension_floor():
    rows = [_seed_row("f000000000000010", ascension=10),
            _seed_row("f000000000000011", ascension=8, encounter="ENCOUNTER.Z")]
    chosen = {row["id"] for row, _ in eval_suite.select_seed_rows(rows)}
    assert chosen == {"f000000000000010"}


def test_seed_keeps_hand_added_fixtures_above_the_floor(tmp_path):
    """`seed` cannot re-derive an `add --encounter` fixture from the corpus,
    so it keeps one at or above the floor instead of deleting it (#2915)."""
    fights = []
    for fight_id, ascension, source in (
            ("f000000000000020", 9, "explicit_capture_pair"),
            ("f000000000000021", 8, "explicit_capture_pair"),
            ("f000000000000022", 10, None)):
        directory = tmp_path / "fights" / fight_id
        directory.mkdir(parents=True)
        provenance = {"ascension": ascension, "node_type": "boss"}
        if source:
            provenance["encounter_source"] = source
        (directory / "provenance.json").write_text(json.dumps(provenance))
        fights.append({"id": fight_id, "ascension": ascension, "kind": "line",
                       "categories": []})
    (tmp_path / "manifest.json").write_text(json.dumps({"fights": fights}))
    manifest = eval_suite.seed_eval_set({"rows": [], "summary": {}}, tmp_path)
    assert [fight["id"] for fight in manifest["fights"]] == ["f000000000000020"]
    assert manifest["fights"][0]["categories"] == ["human", "node:boss"]
    assert sorted(p.name for p in (tmp_path / "fights").iterdir()) == [
        "f000000000000020"]


def test_an_upload_sidecar_names_the_encounter_and_the_entry_input(tmp_path):
    """`import-uploads` writes the `.run` fight's encounter beside the capture
    run; `derive_encounter` reads it before the map, and its presence roots
    the pair with `--capture-run`, the input prod reviews use."""
    save = tmp_path / "SEED-012_save_abcdef012345.save"
    save.write_text("{}")
    assert eval_suite.entry_input(save) == "--save"
    sidecar = eval_suite.upload_sidecar_path(save)
    assert sidecar.name == "SEED-012_upload_abcdef012345.json"
    sidecar.write_text(json.dumps(
        {"encounter": "ENCOUNTER.WATERFALL_GIANT_BOSS", "node_type": "boss"}))
    assert eval_suite.derive_encounter({}, None, save, {}) == (
        "ENCOUNTER.WATERFALL_GIANT_BOSS", "boss")
    assert eval_suite.entry_input(save) == "--capture-run"


def _eval_coverage():
    sys.path.insert(0, str(EVAL_DIR))
    import eval_coverage  # noqa: PLC0415
    return eval_coverage


def test_coverage_artifacts_are_fresh():
    """`eval/coverage.json` and its two SVGs match a fresh render of the
    committed fixtures and universe snapshot. `seed` and `add` regenerate
    them; a hand edit of the manifest owes `eval/eval_coverage.py`."""
    assert _eval_coverage().stale() == []


def test_coverage_grid_draws_every_boss_elite_and_character():
    """No row or column is dropped for being empty (Sean, 2026-09-25): the
    empty cells are the point, since they say what to capture or port next."""
    module = _eval_coverage()
    universe = json.loads(module.UNIVERSE.read_text())
    coverage = json.loads(module.OUT_JSON.read_text())
    for tier in ("boss", "elite"):
        expected = [e["key"] for e in universe["encounters"] if e["tier"] == tier]
        assert [row["key"] for row in coverage["grid"][tier]] == expected, tier
        for row in coverage["grid"][tier]:
            assert set(row["cells"]) == set(CHARACTERS), row["key"]
    # The captured / not-captured split is carried from the census; an empty
    # corpus section would silently grey out every uncertified cell.
    assert coverage["corpus"]["measured_fights"] > 0
    assert coverage["corpus"]["cells"]


def test_seed_and_add_refresh_coverage_only_for_the_committed_tree(tmp_path):
    """`refresh_coverage` runs after every manifest write. A scratch
    `--eval-dir` gets nothing, and without a census the committed tree
    re-renders byte for byte (the stored split is carried forward)."""
    module = _eval_coverage()
    before = {name: (EVAL_DIR / name).read_bytes()
              for name in (module.OUT_JSON.name, module.OUT_SVG.name,
                           module.OUT_BADGE.name)}
    eval_suite.refresh_coverage(tmp_path)
    assert list(tmp_path.iterdir()) == []
    eval_suite.refresh_coverage(EVAL_DIR)
    assert {name: (EVAL_DIR / name).read_bytes() for name in before} == before


def _reset_order_replay(powers, context=None):
    context = context or eval_suite.OPENING_CHECKPOINT
    return {"checksums": [{"context": context, "full_state": {"creatures": [
        {"monster_id": "MONSTER.X"},
        {"player_id": 1, "powers": powers}]}}]}


def test_the_root_takes_the_captures_own_after_energy_reset_order():
    """#3020: the census roots with the checkpoint's order, as the production
    review does. The order is explicit even when empty, it is set only when
    Rust's listener amounts equal the checkpoint's, and every other outcome is
    named rather than spliced."""
    document = {"player": {"genesis": 2, "radiance": 1}}
    replay = _reset_order_replay([
        {"id": "RADIANCE_POWER", "amount": 1}, {"id": "STRENGTH_POWER", "amount": 3},
        {"id": "GENESIS_POWER", "amount": 2}])
    rooted, status = eval_suite.with_native_reset_order(document, replay)
    assert status == "native_checkpoint"
    assert rooted["player"]["after_energy_reset_order"] == ["radiance", "genesis"]
    assert "after_energy_reset_order" not in document["player"]  # not mutated

    empty, status = eval_suite.with_native_reset_order(
        {"player": {}}, _reset_order_replay([]))
    assert (empty["player"]["after_energy_reset_order"], status) == ([], "native_checkpoint")

    for replay, expected in (
            (_reset_order_replay([{"id": "GENESIS_POWER", "amount": 3},
                                  {"id": "RADIANCE_POWER", "amount": 1}]),
             "listener_amount_mismatch"),
            (_reset_order_replay([], context="After enemy turn end"),
             "no_opening_checkpoint"),
            ({"checksums": []}, "no_opening_checkpoint"),
            (_reset_order_replay([{"id": "GENESIS_POWER", "amount": 0}]),
             "native_order_malformed")):
        unchanged, status = eval_suite.with_native_reset_order(document, replay)
        assert (unchanged is document, status) == (True, expected)

    disagree = {"player": dict(document["player"],
                               after_energy_reset_order=["genesis", "radiance"])}
    kept, status = eval_suite.with_native_reset_order(disagree, replay=_reset_order_replay([
        {"id": "RADIANCE_POWER", "amount": 1}, {"id": "GENESIS_POWER", "amount": 2}]))
    assert (kept is disagree, status) == (True, "rust_order_disagrees")


def test_the_deal_leaves_out_cards_the_opening_created():
    """#3089: Gremlin Horn's Dazed (and any card the opening creates) sits in
    hand or draw with a uid after the deck. The capture's first cycle is the
    shuffled deck only, so the deal matches the deck cards and leaves the
    created ones to the shared allocator. Innate still inverts on the piles
    as Rust laid them out."""
    card = lambda cid, uid: {"id": cid, "uid": uid, "upgrade": 0}
    root = {"player": {"innate_min_draw": 1}, "piles": {
        "hand": [card("INNATE", 2), card("DAZED", 3), card("A", 0)],
        "draw": [card("DAZED", 4), card("B", 1)]}}
    recorded = [("A", 0), ("B", 0), ("INNATE", 0)]
    assert eval_suite._dealt_uids(root, recorded) == [0, 1, 2]
    # A missing deck card is still refused, not papered over by a created one.
    short = {"player": {"innate_min_draw": 0}, "piles": {
        "hand": [card("A", 0), card("DAZED", 3)], "draw": []}}
    with pytest.raises(ValueError, match="deck size differs"):
        eval_suite._dealt_uids(short, [("A", 0), ("B", 0)])


def test_the_deal_undoes_jeweled_masks_pre_deal_power_lift():
    """#3170: Jeweled Mask lifts a deck Power into the Hand after numbering;
    the moved card keeps its uid, so uid order is the layout before the lift."""
    card = lambda cid, uid: {"id": cid, "uid": uid, "upgrade": 0}
    root = {"player": {"innate_min_draw": 0,
                       "relics_entering": ["RELIC.JEWELED_MASK"]},
            "piles": {"hand": [card("POWER", 2), card("A", 0), card("B", 1)],
                      "draw": [card("C", 3)]}}
    recorded = [("A", 0), ("B", 0), ("POWER", 0), ("C", 0)]
    assert eval_suite._dealt_uids(root, recorded) == [0, 1, 2, 3]
    # Without the relic the same layout is refused, as before: the reorder is
    # this relic's alone.
    root["player"]["relics_entering"] = []
    with pytest.raises(ValueError, match="identities differ"):
        eval_suite._dealt_uids(root, recorded)


def test_census_summary_travels_with_the_fixture_set():
    """The manifest records the corpus measurement the seed set came from."""
    census = _manifest()["census"]
    assert census["fights"] >= 500
    assert census["rooted"] >= 300
    assert census["rust_admitted"] >= 1
    assert census["human"]["exact"] >= 150
    assert census["lockstep"], "no lockstep verdicts recorded"


# ---------------------------------------------------------------------------
# The hybrid root source and its two-sided opening-parity gate (#2693)
#
# Everything below runs on synthetic documents: no engine binary, no capture
# corpus, no cargo, and — deliberately — no read of anything under
# `versions/v0.111.0/rust/src/`. `eval_suite.py` and `project_state.py` are
# individually listed in the fast gate's TRIGGER_SET, so importing them adds
# no new external coupling; the crate's `src/` tree is NOT in that set
# (#1276), so the half of this surface that is a claim about `boundary.rs` —
# the `RUST_ONLY_PROVENANCE_SLOTS` pin and its mutation controls — lives in
# `versions/v0.111.0/rust/tools/test_eval_suite_hybrid_root.py`, which the
# `rust port` lane runs on exactly the changes that can invalidate it.
# ---------------------------------------------------------------------------


def test_the_only_root_source_is_rusts_own_opening():
    """#2999: the frozen-Python and Rust-entry/Python-opening sources retired."""
    assert eval_suite.ROOT_SOURCES == ("rust_opening",)
    help_text = subprocess.run(
        [sys.executable, str(EVAL_TOOL), "--help"],
        capture_output=True, text=True, check=True).stdout
    assert "--root-with" not in help_text


def test_the_registry_names_the_slot_the_gate_exempts():
    """The exemption set the gate uses, as the tool holds it.

    Whether it AGREES with `boundary.rs` is pinned in the `rust port` lane —
    see the section comment above — because that is a claim about a file this
    lane's trigger set deliberately does not watch.
    """
    assert eval_suite.RUST_ONLY_PROVENANCE_SLOTS == (
        "power_attachments", "scroll_chew_repeated")
    # The exemption is registry-driven, so a differently-named ledger is NOT
    # exempt. This is the behaviour half of the #1432 concern and needs no
    # crate read at all.
    agrees, paths = eval_suite.opening_parity(
        {"monsters": [{"hp": 3}]},
        {"monsters": [{"hp": 3, "future_ledger": [{"power": "X"}]}]})
    assert agrees is False and paths == ["monsters[0].future_ledger"], paths


def test_rust_only_slots_are_stripped_at_any_depth_and_nothing_else_is():
    document = {
        "power_attachments": ["top level too"],
        "monsters": [
            {"hp": 10, "power_attachments": [{"power": "STRENGTH"}]},
            {"hp": 4, "powers": {"weak": 1}},
        ],
        "player": {"hp": 50},
    }
    assert eval_suite.strip_rust_only_slots(document) == {
        "monsters": [{"hp": 10}, {"hp": 4, "powers": {"weak": 1}}],
        "player": {"hp": 50},
    }
    # Stripping never inspects a value, so a slot's CONTENTS cannot change the
    # outcome, and a document without the slot is returned unchanged.
    plain = {"monsters": [{"hp": 10}]}
    assert eval_suite.strip_rust_only_slots(plain) == plain


def test_the_gate_accepts_a_difference_that_is_only_a_rust_only_slot():
    python_root = {"schema": "s", "monsters": [{"hp": 10}, {"hp": 4}]}
    rust_root = {
        "schema": "s",
        "monsters": [
            {"hp": 10, "power_attachments": [
                {"power": "STRENGTH", "amount": 1, "applier": "unknown"}]},
            {"hp": 4},
        ],
    }
    agrees, paths = eval_suite.opening_parity(python_root, rust_root)
    assert agrees is True and paths == []


def test_the_gate_rejects_any_other_difference_and_names_the_path():
    """The whole point: a real divergence must not ride in on the exemption."""
    python_root = {"schema": "s", "monsters": [{"hp": 10, "weak": 1}]}
    rust_root = {
        "schema": "s",
        "monsters": [{"hp": 9, "weak": 1,
                      "power_attachments": [{"power": "STRENGTH"}]}],
    }
    agrees, paths = eval_suite.opening_parity(python_root, rust_root)
    assert agrees is False
    assert paths == ["monsters[0].hp"], paths
    # A field present on one side only is a differing path, not a silent pass.
    agrees, paths = eval_suite.opening_parity(
        {"monsters": [{"hp": 10}]},
        {"monsters": [{"hp": 10, "strength": 1}]})
    assert agrees is False and paths == ["monsters[0].strength"], paths
    # So is a roster of a different length.
    agrees, paths = eval_suite.opening_parity(
        {"monsters": [{"hp": 10}]}, {"monsters": []})
    assert agrees is False and paths == ["monsters[]: length 1 vs 0"], paths


def test_the_gate_reports_paths_but_never_values():
    """Census rows travel into PR bodies; the contents stay in the documents."""
    _, paths = eval_suite.opening_parity(
        {"player": {"name_like_field": "SECRET"}},
        {"player": {"name_like_field": "OTHER"}})
    assert paths == ["player.name_like_field"]
    assert not any("SECRET" in path or "OTHER" in path for path in paths)


def test_the_gate_compares_types_not_just_values():
    """#2790: `1 == True` in Python, and the gate must not believe it.

    The two documents are the input to `canonical_json` — the bytes both
    engines digest — where `1` and `true` are different documents. Comparing
    them with Python `==` passed `fc4cf049784d8f31` TERROR_EEL_ELITE, whose
    `monsters[0].shriek` Rust emitted as `int 1` against frozen Python's
    `bool True`, and the hybrid source then substituted Rust's root on a
    fight whose per-action lockstep diverged everywhere.
    """
    agrees, paths = eval_suite.opening_parity(
        {"monsters": [{"hp": 150, "shriek": True}]},
        {"monsters": [{"hp": 150, "shriek": 1}]})
    assert agrees is False, "int 1 vs bool True must not pass the gate"
    assert paths == ["monsters[0].shriek (bool vs int)"], paths
    # `int` vs `float` is the same identity (`1 == 1.0`) and is also a
    # mismatch: `1` and `1.0` are different canonical bytes.
    agrees, paths = eval_suite.opening_parity(
        {"player": {"block": 1}}, {"player": {"block": 1.0}})
    assert agrees is False and paths == ["player.block (int vs float)"], paths
    # And the `0`/`False` half, which zero-default elision makes just as
    # reachable: `_is_default` requires `type(value) is type(default)`.
    agrees, paths = eval_suite.opening_parity(
        {"monsters": [{"shriek": False}]}, {"monsters": [{"shriek": 0}]})
    assert agrees is False and paths == ["monsters[0].shriek (bool vs int)"]


def test_an_identical_pair_still_passes_the_type_strict_gate():
    document = {
        "schema": "sts-sim-canonical-v2",
        "player": {"hp": 68, "block": 0, "alive": True, "relics": ["BURNING"]},
        "monsters": [{"hp": 150, "shriek": True, "move_log": ["SHRIEK"]}],
        "piles": {"draw": [{"id": "STRIKE", "uid": 0}]},
        "rng": {"shuffle": {"counter": 3}},
    }
    round_tripped = json.loads(json.dumps(document))
    assert eval_suite.opening_parity(round_tripped, round_tripped) == (True, [])
    assert eval_suite.type_strict_equal(
        round_tripped, json.loads(json.dumps(document)))


def test_the_type_pair_is_a_type_name_never_a_value():
    _, paths = eval_suite.opening_parity(
        {"player": {"name_like_field": "SECRET"}},
        {"player": {"name_like_field": 3}})
    assert paths == ["player.name_like_field (str vs int)"], paths
    assert "SECRET" not in paths[0]


def test_the_census_imports_no_simulator():
    """#2999: the certification census survives the simulator's deletion.

    Imported in a fresh interpreter, because this process may already hold
    `combat_sim` from another test module. `mcr_replay`, `live_coach`,
    `project_state` and `mcr_validate` all import the simulator at load time,
    so none of them may be reached either.
    """
    probe = (
        "import sys; sys.path.insert(0, %r); import eval_suite; "
        "bad = sorted(m for m in ('combat_sim', 'solve_fight', 'mcr_replay', "
        "'live_coach', 'project_state', 'mcr_validate', 'content') "
        "if m in sys.modules); print(bad); sys.exit(1 if bad else 0)"
        % str(RUST_DIR / "tools"))
    result = subprocess.run([sys.executable, "-c", probe],
                            capture_output=True, text=True, check=False)
    assert result.returncode == 0, result.stdout + result.stderr


def test_the_python_root_sources_are_retired():
    """`--root-with` is gone: every root is Rust's own opening (#2999)."""
    result = subprocess.run(
        [sys.executable, str(EVAL_TOOL), "seed",
         "--root-with", "python", "--captures", str(RUST_DIR)],
        capture_output=True, text=True, check=False)
    assert result.returncode != 0
    assert "unrecognized arguments: --root-with" in result.stderr


# ---------------------------------------------------------------------------
# #3025: a native checkpoint mismatch BEFORE the human line diverges
#
# Synthetic: a scripted engine session and a scripted checkpoint walker, so
# no binary, no corpus. The shape is the 2J7Y9YVCXKYU Radiate fights
# (`fdbbfac0`, `f8de4ede`): a completed-action checkpoint disagrees with the
# game, and a later recorded input then fails to resolve.
# ---------------------------------------------------------------------------


class _ScriptedSession:
    """Answers `load`/`legal`/`apply`/`project` for a potion then an end."""

    def __init__(self):
        self.state = {"player": {"turn": 1, "hp": 50}, "monsters": [],
                      "piles": {"hand": []}}
        self.applied = []

    def load(self, _document):
        return {"ok": {"digest": "root"}}

    def ask(self, request):
        if request["cmd"] == "legal":
            return {"actions": [{"kind": "potion", "slot": 0},
                                {"kind": "end"}]}
        if request["cmd"] == "apply":
            self.applied.append(request["action"])
            if request["action"]["kind"] == "end":
                self.state = dict(self.state, player=dict(
                    self.state["player"], turn=2))
            return {"digest": f"d{len(self.applied)}"}
        if request["cmd"] == "project":
            return {"state": self.state}
        raise AssertionError(request)


def _mismatching_checks(field: str):
    class _Checks:
        """Agrees on nothing: the first completed action disagrees."""

        def __init__(self, _replay):
            self.validated = 0

        def start(self, _event):
            pass

        def stage(self, _applied):
            return None  # no checkpoint inside the apply (#3242)

        def no_decision(self, _event):
            pass

        def completed(self, _state):
            raise ValueError(f"recorded replay differs from native {field}")

        def finish(self):
            raise AssertionError("a diverged line never reaches finish")

    return _Checks


def _diverging_replay():
    # Input 0 completes (and its checkpoint disagrees); input 2's recorded
    # turn number is not the engine's, so the line diverges at step 2.
    return {"events": [
        {"event_type": "Action", "action": {
            "type": "NetUsePotionAction", "potion_index": 0}},
        {"event_type": "Action", "action": {
            "type": "NetEndPlayerTurnAction", "turn_number": 1}},
        {"event_type": "Action", "action": {
            "type": "NetEndPlayerTurnAction", "turn_number": 7}},
    ]}


def test_a_checkpoint_mismatch_before_a_divergence_travels_with_it(
        monkeypatch):
    monkeypatch.setattr(eval_suite, "_deal_uid_map",
                        lambda root, replay: (lambda index: index))
    monkeypatch.setattr(eval_suite, "_CensusChecks",
                        _mismatching_checks("monsters"))
    root = {"player": {"turn": 1}, "monsters": [], "piles": {"hand": []}}
    with pytest.raises(eval_suite.LineDiverged) as caught:
        eval_suite.replay_recorded_line(_ScriptedSession(), root,
                                        _diverging_replay())
    exc = caught.value
    assert (exc.check, exc.step) == ("turn_number", 2)
    assert exc.checkpoint_mismatch == {
        "step": 1, "detail": "recorded replay differs from native monsters"}
    assert exc.native_checkpoints == 0

    # The census row keeps the divergence as it was and adds the mismatch.
    row = {"checksummed": True, "opening_checkpoint": "match"}
    verdict = eval_suite.divergence_checkpoint_verdict(row, exc)
    assert verdict == {
        "native_checkpoints_before_divergence": 0,
        "pre_divergence_lockstep": "checkpoint_mismatch",
        "pre_divergence_mismatch_step": 1,
        "pre_divergence_mismatch_field": "monsters",
        "pre_divergence_mismatch_detail":
            "recorded replay differs from native monsters",
    }
    assert "lockstep" not in verdict  # certification keeps its meaning


def test_a_divergence_with_agreeing_checkpoints_reports_no_mismatch():
    exc = eval_suite.LineDiverged(3, 2, "not_legal", "x",
                                  checkpoint_mismatch=None,
                                  native_checkpoints=2)
    verdict = eval_suite.divergence_checkpoint_verdict(
        {"checksummed": True, "opening_checkpoint": "match"}, exc)
    assert verdict == {"native_checkpoints_before_divergence": 2,
                       "pre_divergence_lockstep":
                           "no_mismatch_before_divergence"}
    # The opening checkpoint is step 0 and outranks a later mismatch.
    opening = eval_suite.divergence_checkpoint_verdict(
        {"checksummed": True, "opening_checkpoint": "mismatch",
         "opening_checkpoint_detail":
             "ValueError: recorded replay differs from native player state"},
        eval_suite.LineDiverged(3, 2, "not_legal", "x",
                                checkpoint_mismatch={"step": 2, "detail": "y"}))
    assert opening["pre_divergence_mismatch_step"] == 0
    assert opening["pre_divergence_mismatch_field"] == "player_state"
    # No native checkpoints: measured and named, never a mismatch.
    assert eval_suite.divergence_checkpoint_verdict({}, exc) == {
        "pre_divergence_lockstep": "no_native_checkpoints"}
    assert eval_suite.checkpoint_mismatch_field(
        "recorded checkpoint action identity mismatch") == "action_identity"
    assert eval_suite.checkpoint_mismatch_field("anything else") == "other"


def test_the_census_counts_mismatches_before_a_divergence():
    rows = [
        {"stage": "rooted", "rust": "admitted", "human": "exact",
         "lockstep": "checkpoint_mismatch", "checksummed": True},
        {"stage": "rooted", "rust": "admitted", "human": "diverged",
         "checksummed": True,
         "pre_divergence_lockstep": "checkpoint_mismatch",
         "pre_divergence_mismatch_field": "monsters"},
        {"stage": "rooted", "rust": "admitted", "human": "diverged",
         "checksummed": True,
         "pre_divergence_lockstep": "no_mismatch_before_divergence"},
    ]
    summary = eval_suite.census_summary(rows)
    # The certification tally is unchanged; the new counts sit beside it.
    assert summary["lockstep"] == {"checkpoint_mismatch": 1}
    assert summary["lockstep_before_divergence"] == {
        "checkpoint_mismatch": 1, "no_mismatch_before_divergence": 1}
    assert summary["checkpoint_mismatch_before_divergence_fields"] == {
        "monsters": 1}
    assert summary["checkpoint_mismatch_known"] == 2
    markdown = eval_suite.census_markdown(
        {"captures": "c", "build": "b", "summary": summary})
    assert "| 1 + 1 = 2 |" in markdown
    assert "Checkpoint mismatch before a divergence, by field" in markdown


# ---------------------------------------------------------------------------
# #3277: a capture archived mid-fight is `truncated`, not a divergence
# ---------------------------------------------------------------------------


class _LiveSession:
    """A fight Rust never ends: a potion and an end turn, both no-ops.

    Every apply reports no intermediate native checkpoint (#3242), so the
    real `NativeChecks` compare each completed action at the post-apply
    state.
    """

    def __init__(self, potion_opens_choice=False):
        self.state = {"player": {"turn": 1, "hp": 40}, "monsters": [],
                      "piles": {"hand": []}}
        self.potion_opens_choice = potion_opens_choice

    def load(self, _document):
        return {"ok": {"digest": "root"}}

    def ask(self, request):
        if request["cmd"] == "legal":
            return {"actions": [{"kind": "potion", "slot": 0},
                                {"kind": "end"}]}
        if request["cmd"] == "apply":
            if request["action"]["kind"] == "potion" \
                    and self.potion_opens_choice:
                self.state = dict(self.state, player=dict(
                    self.state["player"], pending={"kind": "choose"}))
            if request["action"]["kind"] == "end":
                self.state = dict(self.state, player=dict(
                    self.state["player"], turn=2))
            return {"digest": "d", "native_checkpoints": []}
        if request["cmd"] == "project":
            return {"state": self.state}
        raise AssertionError(request)


def _potion_event():
    return {"event_type": "GameAction", "action": {
        "type": "NetUsePotionAction", "potion_index": 0}}


def _potion_checkpoint(enemy_hp, player_hp=40, context=None):
    return {"checksum": {"id": 1, "value": 0},
            "context": context or ("finished action execution "
                                   "UsePotionAction 1  index: 0 target: "),
            "full_state": {"creatures": [
                {"monster_id": None, "player_id": 7, "current_hp": player_hp,
                 "max_hp": 50, "block": 0},
                {"monster_id": "OSTY", "player_id": None, "current_hp": 3,
                 "max_hp": 9, "block": 0},
                {"monster_id": "QUEEN", "player_id": None,
                 "current_hp": enemy_hp, "max_hp": 419, "block": 0}]}}


@pytest.fixture
def _agreeing_checkpoints(monkeypatch):
    """The real `_CensusChecks`, with every state comparison agreeing."""
    monkeypatch.setattr(eval_suite, "_deal_uid_map",
                        lambda root, replay: (lambda index: index))
    compared = []
    monkeypatch.setattr(eval_suite, "check_native_checkpoint",
                        lambda state, native: compared.append(native))
    monkeypatch.setattr(eval_suite, "power_mismatch_rows",
                        lambda state, native: [])
    return compared


def _replay_live(events, checksums, potion_opens_choice=False):
    root = {"player": {"turn": 1, "hp": 40}, "monsters": [],
            "piles": {"hand": []}}
    return eval_suite.replay_recorded_line(
        _LiveSession(potion_opens_choice), root,
        {"events": events, "checksums": checksums})


def test_a_capture_whose_final_checkpoint_is_live_is_truncated(
        _agreeing_checkpoints):
    """The Test Subject / Queen / Phrog Parasite shape (#3277): every input
    applied, every checkpoint agreed, and the capture's last checkpoint,
    after the last input, still shows the Queen alive."""
    with pytest.raises(eval_suite.CaptureTruncated) as caught:
        _replay_live([_potion_event()], [_potion_checkpoint(enemy_hp=395)])
    exc = caught.value
    assert (exc.check, exc.step, exc.native_checkpoints) == (
        "capture_truncated", 1, 1)
    assert exc.shape == "after_completed_action"
    assert len(_agreeing_checkpoints) == 1  # the final checkpoint WAS compared

    row = {"checksummed": True, "opening_checkpoint": "match"}
    fields = eval_suite.truncated_row_fields(row, exc)
    assert fields["human"] == "truncated"
    assert fields["human_class"] == "capture_truncated"
    assert fields["prefix_lockstep"] == "prefix_agrees"
    assert fields["truncation_shape"] == "after_completed_action"
    assert "lockstep" not in fields  # a partial line is never certified
    # An opening that disagreed stays visible on the truncated row.
    opening = eval_suite.truncated_row_fields(
        {"checksummed": True, "opening_checkpoint": "mismatch",
         "opening_checkpoint_detail": "x"}, exc)
    assert (opening["prefix_lockstep"], opening["prefix_mismatch_step"]) == (
        "checkpoint_mismatch", 0)


def test_a_combat_rust_keeps_alive_past_a_dead_final_checkpoint_is_not_complete(
        _agreeing_checkpoints):
    """The engine defect the class must not hide: natively every enemy is
    dead at the capture's final checkpoint, and Rust's combat goes on."""
    with pytest.raises(eval_suite.LineDiverged) as caught:
        _replay_live([_potion_event()], [_potion_checkpoint(enemy_hp=0)])
    assert not isinstance(caught.value, eval_suite.CaptureTruncated)
    assert caught.value.check == "combat_not_complete"
    # A dead player at the final checkpoint is also a native combat end.
    with pytest.raises(eval_suite.LineDiverged) as caught:
        _replay_live([_potion_event()],
                     [_potion_checkpoint(enemy_hp=395, player_hp=0)])
    assert caught.value.check == "combat_not_complete"


def test_truncation_is_never_guessed_past_the_final_checkpoint(
        _agreeing_checkpoints):
    end = {"event_type": "GameAction", "action": {
        "type": "NetEndPlayerTurnAction", "turn_number": 1}}
    # An end turn after the last checkpoint: the enemy turn it ran natively
    # is unrecorded, so the live checkpoint no longer describes the end.
    with pytest.raises(eval_suite.LineDiverged) as caught:
        _replay_live([_potion_event(), end], [_potion_checkpoint(395)])
    assert caught.value.check == "combat_not_complete"
    # A turn-boundary record after the completed action is the capture's
    # final checksum: not a completed-action checkpoint, so no claim.
    with pytest.raises(eval_suite.LineDiverged) as caught:
        _replay_live([_potion_event(), end], [
            _potion_checkpoint(395),
            _potion_checkpoint(395, context="After enemy turn start")])
    assert caught.value.check == "combat_not_complete"
    # A capture without checkpoints proves nothing either way.
    with pytest.raises(eval_suite.LineDiverged) as caught:
        _replay_live([_potion_event()], [])
    assert caught.value.check == "combat_not_complete"


def test_a_capture_that_stops_inside_a_player_decision_is_truncated(
        _agreeing_checkpoints):
    """The Phrog Parasite shape (#3277): the last input is a play Rust
    leaves pending a choice, its completed-action checkpoint is not in the
    file, and the capture's final checksum (a turn start) shows the fight
    live."""
    turn_start = _potion_checkpoint(41, context="After player turn start")
    with pytest.raises(eval_suite.CaptureTruncated) as caught:
        _replay_live([_potion_event()], [turn_start],
                     potion_opens_choice=True)
    assert caught.value.shape == "inside_player_decision"
    assert caught.value.native_checkpoints == 0
    # The same play, when the file DOES hold its completed checkpoint: the
    # game finished it without a choice, so Rust's pending choice is the
    # engine's and nothing is claimed.
    with pytest.raises(eval_suite.LineDiverged) as caught:
        _replay_live([_potion_event()], [turn_start, _potion_checkpoint(41)],
                     potion_opens_choice=True)
    assert not isinstance(caught.value, eval_suite.CaptureTruncated)
    assert caught.value.check == "combat_not_complete"
    # And a final checksum showing the enemy dead proves nothing live.
    with pytest.raises(eval_suite.LineDiverged) as caught:
        _replay_live([_potion_event()],
                     [_potion_checkpoint(0, context="After player turn start")],
                     potion_opens_choice=True)
    assert caught.value.check == "combat_not_complete"


def test_a_checkpoint_mismatch_keeps_a_live_capture_out_of_truncated(
        monkeypatch):
    """Rust must have AGREED with the live final checkpoint: a line the game
    already disagreed with is not proved to have reached that point."""
    monkeypatch.setattr(eval_suite, "_deal_uid_map",
                        lambda root, replay: (lambda index: index))
    monkeypatch.setattr(eval_suite, "_CensusChecks",
                        _mismatching_checks("monsters"))
    with pytest.raises(eval_suite.LineDiverged) as caught:
        _replay_live([_potion_event()], [_potion_checkpoint(395)])
    assert caught.value.check == "combat_not_complete"
    assert caught.value.checkpoint_mismatch["step"] == 1


def _truncated_row(fight_id="f3277000000000001"):
    return {
        "id": fight_id, "seed": "S", "node": 48, "character": "SILENT",
        "encounter": "ENCOUNTER.QUEEN_BOSS", "node_type": "boss",
        "ascension": 10, "stage": "rooted", "rust": "admitted",
        "checksummed": True, "capture_sha256": "c" * 64,
        "save_sha256": "s" * 64, "entry_digest": "e" * 64,
        "_document": {"piles": {}, "player": {}, "monsters": []},
        "human": "truncated", "human_class": "capture_truncated",
        "human_detail": "t6 capture_truncated: capture ends mid-fight",
        "human_step": 73, "prefix_lockstep": "prefix_agrees",
        "truncation_shape": "after_completed_action",
    }


def test_a_truncated_capture_is_excluded_from_the_seed_set():
    """Neither a line (it is not exact) nor a refusal (no engine surface
    refused it): `seed` leaves it out, and `add` refuses to write it."""
    truncated = _truncated_row()
    diverged = dict(_truncated_row("f3277000000000002"), human="diverged",
                    human_class="diverged:combat_not_complete",
                    human_detail="t6 combat_not_complete: x")
    assert eval_suite.refusal_facet(truncated) is None
    chosen = {row["id"]: kind for row, kind in
              eval_suite.select_seed_rows([truncated, diverged])}
    assert chosen == {"f3277000000000002": "refusal"}
    for kind in ("line", "refusal"):
        with pytest.raises(eval_suite.EvalRefusal, match="mid-fight"):
            eval_suite.fixture_documents(truncated, kind)


def test_the_census_counts_truncated_captures_apart_from_divergences():
    rows = [_truncated_row(),
            dict(_truncated_row("f3277000000000002"), human="diverged",
                 human_class="diverged:combat_not_complete",
                 pre_divergence_lockstep="no_mismatch_before_divergence")]
    summary = eval_suite.census_summary(rows)
    assert summary["capture_truncated"] == 1
    assert summary["human"] == {"diverged": 1, "truncated": 1}
    assert summary["human_divergence_classes"] == {
        "diverged:combat_not_complete": 1}
    assert summary["lockstep_before_truncation"] == {"prefix_agrees": 1}
    assert summary["capture_truncated_shapes"] == {"after_completed_action": 1}
    assert summary["lockstep"] == {}  # never certified, never a verdict
    markdown = eval_suite.census_markdown(
        {"captures": "c", "build": "b", "summary": summary})
    assert "| capture ends mid-fight (truncated, uncertified, #3277) | 1 |" \
        in markdown


# `eval_suite.py --self-test` is deliberately NOT invoked here: it re-derives
# the Rust-only registry from `boundary.rs`, so running it from this lane would
# smuggle the same untracked coupling back in. The `rust port` lane runs it
# beside the pin it belongs to.


# ---------------------------------------------------------------------------
# Opt-in: the Rust engine and the local capture corpus
# ---------------------------------------------------------------------------


class _DiffServe:
    """A minimal diff-serve client, so this module imports no port tooling."""

    def __init__(self, binary: pathlib.Path):
        self.process = subprocess.Popen(
            [str(binary), "diff-serve"], stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, text=True, bufsize=1)
        greeting = json.loads(self.process.stdout.readline())
        assert greeting.get("protocol") == "diff-serve-v1", greeting

    def request(self, payload: dict) -> dict:
        self.process.stdin.write(json.dumps(payload) + "\n")
        self.process.stdin.flush()
        return json.loads(self.process.stdout.readline())

    def close(self) -> None:
        try:
            self.request({"cmd": "quit"})
        except Exception:  # noqa: BLE001 - closing must not mask a result
            self.process.kill()
        self.process.wait(timeout=30)


rust_lockstep_optin = pytest.mark.skipif(
    os.environ.get("STS2_EVAL_RUST_LOCKSTEP") != "1"
    or not ENGINE_BINARY.is_file(),
    reason="set STS2_EVAL_RUST_LOCKSTEP=1 with a built "
           "versions/v0.111.0/rust/target/release/sts-sim")


@rust_lockstep_optin
def test_certified_lines_replay_lockstep_clean_through_rust():
    """Each certified line, action by action, digest-checked after every one.

    Rust's `legal` lists one representative uid per group of physically
    identical cards while `apply` accepts any member of the group, so the
    recorded uid is applied as recorded — requiring it to appear in `legal`
    reports a divergence that does not exist.
    """
    manifest = _manifest()
    certified = [fight for fight in manifest["fights"]
                 if "certified" in fight["categories"]]
    assert certified
    rust = _DiffServe(ENGINE_BINARY)
    try:
        for fight in certified:
            entry = _fixture(fight["id"], "entry.canonical.json")
            line = _fixture(fight["id"], "human_line.json")
            response = rust.request({"cmd": "load", "entry": entry})
            assert "ok" in response, (fight["id"], response)
            assert response["ok"]["digest"] == fight["entry_digest"]
            for step, (action, digest) in enumerate(
                    zip(line["actions"], line["step_digests"])):
                applied = rust.request({"cmd": "apply", "action": action})
                assert "ok" in applied, (fight["id"], step, applied)
                assert applied["ok"]["digest"] == digest, (fight["id"], step)
    finally:
        rust.close()


@rust_lockstep_optin
def test_uncertified_roots_still_refuse_for_the_recorded_reason():
    """The refusal fixtures are pins too: a boundary slice that lands one of
    these classes should turn this red and be re-seeded, not silently drift.
    """
    manifest = _manifest()
    rust = _DiffServe(ENGINE_BINARY)
    try:
        for fight in manifest["fights"]:
            if fight.get("rust_root") != "refused":
                continue
            if "entry.canonical.json" not in fight["files"]:
                continue
            entry = _fixture(fight["id"], "entry.canonical.json")
            response = rust.request({"cmd": "load", "entry": entry})
            assert "ok" not in response, (fight["id"], "now admitted: re-seed")
    finally:
        rust.close()


@pytest.mark.skipif(
    os.environ.get("STS2_EVAL_REDERIVE") != "1",
    reason="set STS2_EVAL_REDERIVE=1 with the local capture corpus present")
def test_every_fixture_re_derives_from_its_provenance():
    """The tool's `verify`: locate the capture pair by sha256 and rebuild."""
    captures = pathlib.Path(
        os.environ.get("STS2_CAPTURES")
        or (pathlib.Path.home() / "sts2-captures")).expanduser()
    if not captures.is_dir():
        pytest.skip(f"no capture corpus at {captures}")
    result = subprocess.run(
        [sys.executable, str(EVAL_TOOL), "verify",
         "--captures", str(captures)],
        capture_output=True, text=True, check=False)
    assert result.returncode == 0, result.stdout + result.stderr
    report = json.loads(result.stdout)
    assert report["problems"] == []
    assert report["rooted_checked"] >= 27


class _ResolvingSession:
    """`resolve_selection` only: the census's k-card path asks nothing else."""

    def __init__(self, action):
        self.action, self.requests = action, []

    def ask(self, payload):
        self.requests.append(payload)
        assert payload["cmd"] == "resolve_selection", payload
        return {"action": self.action}


def test_multi_card_selection_takes_the_engine_answer_without_enumerating():
    """#3125: Gambling Chip's 4-card discard (326 offered answers) resolves
    through `engine::recorded_selection_answer` in one read-only request."""
    wire = {"kind": "select", "answer": {"kind": "option_index", "index": 290}}
    session = _ResolvingSession(wire)
    assert eval_suite._multi_card_selection(session, {}, [19, 12, 13, 11]) == wire
    assert session.requests == [
        {"cmd": "resolve_selection", "uids": [19, 12, 13, 11]}]


def test_multi_card_selection_with_no_engine_answer_diverges_by_name():
    with pytest.raises(ValueError, match="no unique supported Rust answer"):
        eval_suite._multi_card_selection(_ResolvingSession(None), {}, [7, 8])

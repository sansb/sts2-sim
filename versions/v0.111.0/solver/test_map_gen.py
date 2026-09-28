"""Tests for map_gen.py — exact v0.111.0 act-map generation from seed (#1846).

Validation layers:
1. Engine-oracle exactness: testdata/map_gen_engine_maps_v0111.json holds 84
   maps (28 seeds x 3 acts, ascensions 0-20, both act-1 families) dumped
   from the REAL engine's StandardActMap.CreateFor by
   harness/probe_act_map.py on build v0.111.0. The generator must
   reproduce every node, edge, point type, can_modify flag, START-set
   order, and child ENUMERATION order (HashSet insertion order) —
   byte-exact, no tolerance.
2. Fixture saves: every current-build (schema-20) *.save in testdata whose
   run has no map-modifying model live must round-trip against its
   serialized saved_map. TQM88QFMHSQR act2 is pinned as the known
   Spoils-Map divergence (CARD.SPOILS_MAP in deck replaces the act map
   with a SpoilsActMap — a modeled refusal, not a generator gap).
3. I5 refusals: unmodeled act families raise, never approximate.

Run: solver/tools/pytest_lane.sh -- versions/v0.111.0/solver/test_map_gen.py
"""

import json
import pathlib
import sys

import pytest

HERE = pathlib.Path(__file__).parent
TD = HERE / "testdata"
sys.path.insert(0, str(HERE))

import map_gen  # noqa: E402

ENGINE_FIXTURE = TD / "map_gen_engine_maps_v0111.json"

# Current-build (schema-20) saves with a StandardActMap to round-trip.
# (Schema-18 saves predate the current save dumper; their saved_map JSON
# is unreliable — see the invariant walk for #1846.)
SCHEMA20_STANDARD_SAVES = [
    ("1ZEPE4MX6919_queen_entry.save", 2),
    ("6P96T755CNZ3_mawler_entry.save", 0),
    ("89SJD17KYUEH_queen_entry.save", 2),
    ("TQM88QFMHSQR_colony_entry.save", 0),
    ("TQM88QFMHSQR_eel_entry.save", 0),
    ("TQM88QFMHSQR_fabricator_entry.save", 2),
    ("TQM88QFMHSQR_gardeners_entry.save", 0),
    ("TQM88QFMHSQR_scrolls3_entry.save", 2),
    ("TQM88QFMHSQR_shop_entry.save", 0),
    ("TQM88QFMHSQR_sludge_entry.save", 0),
]


def _gen_shape(m):
    """Order-sensitive canonical form of a generated map."""
    return {
        "points": {
            (p.col, p.row): (p.point_type, p.can_be_modified,
                             [(c.col, c.row) for c in p.children])
            for p in m._all_points()},
        "start_children": [(c.col, c.row) for c in m.start.children],
        "start_coords": [(p.col, p.row) for p in m.start_map_points],
        "boss": (m.boss.col, m.boss.row, m.boss.point_type),
        "start": (m.start.col, m.start.row, m.start.point_type),
    }


def _engine_shape(e):
    return {
        "points": {
            tuple(p["coord"]): (p["type"], p["can_modify"],
                                [tuple(c) for c in p["children"]])
            for p in e["points"]},
        "start_children": [tuple(c) for c in e["start"]["children"]],
        "start_coords": [tuple(c) for c in e["start_coords"]],
        "boss": (*e["boss"]["coord"], e["boss"]["type"]),
        "start": (*e["start"]["coord"], e["start"]["type"]),
    }


def _engine_cases():
    return json.loads(ENGINE_FIXTURE.read_text())


@pytest.mark.parametrize(
    "case", _engine_cases(),
    ids=lambda e: f"{e['seed']}-act{e['act_index'] + 1}-asc{e['ascension']}")
def test_engine_oracle_exact(case):
    m = map_gen.generate_act_map(
        case["seed"], case["act_index"], case["act_id"],
        ascension=case["ascension"], started_with_neow=True,
        multiplayer=False, has_second_boss=False)
    assert _gen_shape(m) == _engine_shape(case)


def test_engine_fixture_breadth():
    cases = _engine_cases()
    assert len(cases) == 84
    assert {c["act_id"] for c in cases} == {
        "ACT.UNDERDOCKS", "ACT.OVERGROWTH", "ACT.HIVE", "ACT.GLORY"}
    assert {c["ascension"] for c in cases} == {0, 1, 5, 10, 15, 20}


def _save_map_shape(sm):
    """Order-insensitive form for serialized saves (coords are
    {col,row} dicts there): the save dumper does not preserve enumeration
    order guarantees, and can_modify is dropped on reload — compare
    coords, types, and edge sets."""
    return {
        (p["coord"]["col"], p["coord"]["row"]): (
            p["type"],
            tuple(sorted((c["col"], c["row"]) for c in p["children"])))
        for p in sm["points"]}


def _gen_save_shape(m):
    return {
        (p.col, p.row): (map_gen.TYPE_NAMES[p.point_type],
                         tuple(sorted((c.col, c.row) for c in p.children)))
        for p in m._all_points()}


@pytest.mark.parametrize("name,act_idx", SCHEMA20_STANDARD_SAVES)
def test_schema20_fixture_saves(name, act_idx):
    s = json.loads((TD / name).read_text())
    assert s["schema_version"] == 20
    act = s["acts"][act_idx]
    sm = act["saved_map"]
    assert sm is not None
    m = map_gen.generate_act_map(
        s["rng"]["seed"], act_idx, act["id"],
        ascension=s["ascension"],
        started_with_neow=s["extra_fields"]["started_with_neow"],
        multiplayer=len(s["players"]) > 1,
        has_second_boss=sm.get("second_boss") is not None)
    assert _gen_save_shape(m) == _save_map_shape(sm)
    assert (m.COLS, m.map_length) == (sm["width"], sm["height"])
    assert sorted((p.col, p.row) for p in m.start_map_points) == \
        sorted((c["col"], c["row"]) for c in sm["start_coords"])


def test_spoils_map_divergence_pinned():
    """TQM88QFMHSQR act2 was generated with CARD.SPOILS_MAP in the deck:
    the act map is a SpoilsActMap (own "spoils_map" stream). The STANDARD
    generator must NOT match it — the spoils model must instead
    (test_spoils_matches_hopper_fixture_save). If this ever passes, the
    fixture or the variant disambiguation is wrong."""
    s = json.loads((TD / "TQM88QFMHSQR_hopper_entry.save").read_text())
    deck = [c.get("id") for c in s["players"][0]["deck"]]
    assert "CARD.SPOILS_MAP" in deck
    assert "CARD.SPOILS_MAP" in map_gen.REFUSAL_MODEL_IDS
    sm = s["acts"][1]["saved_map"]
    m = map_gen.generate_act_map(
        s["rng"]["seed"], 1, "ACT.HIVE", ascension=s["ascension"])
    assert _gen_save_shape(m) != _save_map_shape(sm)


def test_mechanics_doc_entry():
    mechanics = (HERE / "ENCOUNTER_MECHANICS.md").read_text()
    assert "Act-map generation from seed (#1846" in mechanics
    assert "SortedDictionary keyed with StringComparer.Ordinal" in mechanics
    assert "map_gen_engine_maps_v0111.json" in mechanics
    assert "Map-hook variants: SpoilsActMap and Big Game Hunter" in mechanics
    assert "map_gen_variant_maps_v0111.json" in mechanics


VARIANT_FIXTURE = TD / "map_gen_variant_maps_v0111.json"


def _variant_cases():
    return json.loads(VARIANT_FIXTURE.read_text())


@pytest.mark.parametrize(
    "case", _variant_cases(),
    ids=lambda e: f"{e['kind']}-{e['seed']}-act{e['act_index'] + 1}")
def test_variant_engine_oracle_exact(case):
    """SpoilsActMap (#1868) and Big Game Hunter regeneration (#1869):
    48 engine dumps from harness/probe_variant_maps.py — spoils via the
    exact `new SpoilsActMap(state, null)` the card hook installs, bgh via
    the REAL relic model's ModifyGeneratedMap over a fresh standard map."""
    if case["kind"] == "spoils":
        m = map_gen.generate_spoils_act_map(
            case["seed"], case["act_id"], ascension=case["ascension"])
    else:
        m = map_gen.generate_bgh_act_map(
            case["seed"], case["act_index"], case["act_id"],
            ascension=case["ascension"],
            has_second_boss="second_boss" in case)
    assert _gen_shape(m) == _engine_shape(case)


def test_variant_fixture_breadth():
    cases = _variant_cases()
    assert len(cases) == 48
    kinds = {c["kind"] for c in cases}
    assert kinds == {"spoils", "bgh"}
    assert sum(1 for c in cases if c["kind"] == "spoils") == 12


def test_spoils_matches_hopper_fixture_save():
    """The TQM88QFMHSQR act-2 saved_map IS a real SpoilsActMap; the model
    must now round-trip it (previously the pinned refusal divergence)."""
    s = json.loads((TD / "TQM88QFMHSQR_hopper_entry.save").read_text())
    sm = s["acts"][1]["saved_map"]
    m = map_gen.generate_spoils_act_map(
        s["rng"]["seed"], "ACT.HIVE", ascension=s["ascension"])
    assert _gen_save_shape(m) == _save_map_shape(sm)


def test_unmodeled_act_refuses():
    with pytest.raises(NotImplementedError):
        map_gen.generate_act_map("SEEDMAP111", 0, "ACT.DOES_NOT_EXIST",
                                 ascension=0)


def test_started_without_neow_start_is_monster():
    """RunManager::<GenerateMap>d__188 (RVA 0x30d3dc): a run that did not
    start with Neow gets a Monster start point on act 1 only."""
    with_neow = map_gen.generate_act_map(
        "SEEDMAP111", 0, "ACT.UNDERDOCKS", ascension=0,
        started_with_neow=True)
    without = map_gen.generate_act_map(
        "SEEDMAP111", 0, "ACT.UNDERDOCKS", ascension=0,
        started_with_neow=False)
    assert with_neow.start.point_type == map_gen.ANCIENT
    assert without.start.point_type == map_gen.MONSTER
    act2 = map_gen.generate_act_map(
        "SEEDMAP111", 1, "ACT.HIVE", ascension=0, started_with_neow=False)
    assert act2.start.point_type == map_gen.ANCIENT


def test_second_boss_point():
    m = map_gen.generate_act_map(
        "SEEDMAP111", 2, "ACT.GLORY", ascension=10, has_second_boss=True)
    assert m.second_boss is not None
    assert (m.second_boss.col, m.second_boss.row) == (3, m.map_length + 1)
    assert m.second_boss.point_type == map_gen.BOSS
    assert m.second_boss in list(m.boss.children)
    # the extra point must not perturb the grid itself
    base = map_gen.generate_act_map(
        "SEEDMAP111", 2, "ACT.GLORY", ascension=10, has_second_boss=False)
    assert _gen_save_shape(m) == _gen_save_shape(base)


def test_visited_path_traces_generated_map():
    """Product-level guard: a run's map_point_history must trace a valid
    root-to-boss walk through the generated map. Uses the TQM save's own
    per-player visited coords (act 1)."""
    s = json.loads((TD / "TQM88QFMHSQR_sludge_entry.save").read_text())
    visited = s["visited_map_coords"]
    assert visited and visited[0] == {"col": 3, "row": 0}
    m = map_gen.generate_act_map(
        s["rng"]["seed"], 0, s["acts"][0]["id"], ascension=s["ascension"],
        started_with_neow=s["extra_fields"]["started_with_neow"])
    pts = {(p.col, p.row): p for p in m._all_points()}
    pts[(m.start.col, m.start.row)] = m.start
    pts[(m.boss.col, m.boss.row)] = m.boss
    prev = None
    for c in visited:
        cur = pts[(c["col"], c["row"])]
        if prev is not None:
            assert cur in list(prev.children), \
                f"visited step {(c['col'], c['row'])} not a child of " \
                f"{(prev.col, prev.row)}"
        prev = cur

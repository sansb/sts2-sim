"""Exact v0.111.0 act-map generator: topology + point types from the run seed.

Reimplements MegaCrit.Sts2.Core.Map.StandardActMap and its helpers so the
full act map — every node, edge, and icon, including the paths the player
did NOT take — can be reconstructed from a run's seed string alone.

IL provenance (all reads against the archived v0.111.0 sts2.dll,
sha 9cb4f1ad..., via versions/v0.111.0/solver/tools/dump_il.py):

  StandardActMap::CreateFor            RVA 0xfbc68  — map stream is
      `new Rng(RunRngSet.Seed, $"act_{actIndex+1}_map")`, i.e. an
      independently seeded named stream per act; NOT the UpFront stream.
  StandardActMap::.ctor                RVA 0xfbb58  — draw order:
      counts = act.GetMapPointTypes(rng) -> GenerateMap -> AssignPointTypes
      -> MapPathPruning.PruneAndRepair -> CenterGrid -> Spread -> Straighten
  StandardActMap::GenerateMap          RVA 0xfbf60
  StandardActMap::PathGenerate         RVA 0xfbd2c
  StandardActMap::GenerateNextCoord    RVA 0xfbd6c
  StandardActMap::HasInvalidCrossover  RVA 0xfbeac
  StandardActMap::AssignPointTypes     RVA 0xfc054
  StandardActMap::AssignRemainingTypesToRandomPoints RVA 0xfc3d8
  StandardActMap::GetNextValidPointType RVA 0xfc490
  StandardActMap::IsValid*             RVA 0xfc4d9-0xfc640 (+ .cctor 0xfc69c)
  MapPathPruning::*                    RVA 0xf8ecc-0xf98ec
  MapPostProcessing::*                 RVA 0xfa288-0xfa7e4
  MapPointTypeCounts::.ctor            RVA 0xfa17c (elites/shops), and
      StandardRandomUnknownCount       RVA 0xfa16d = NextGaussianInt(12,1,10,14)
  Rng::NextGaussianInt                 RVA 0x5edcc — Box-Muller
      (1-NextDouble twice), Math.Round (half-to-even), retry until in range.
  ActModel::GetNumberOfRooms           RVA 0x7a650 — BaseNumberOfRooms
      (Underdocks/Overgrowth 15, Hive 14, Glory 13) minus 1 in multiplayer.
  Underdocks/Overgrowth::GetMapPointTypes RVA 0xf4e8c/0xf4c4c —
      rests = NextGaussianInt(7,1,6,7); unknowns = StandardRandomUnknownCount.
  Hive::GetMapPointTypes               RVA 0xf48f4 —
      rests = NextGaussianInt(6,1,6,7); unknowns = Standard - 1.
  Glory::GetMapPointTypes              RVA 0xf4690 —
      rests = NextInt(5,7);            unknowns = Standard - 1.
  RunManager::GenerateMap async body (<GenerateMap>d__188::MoveNext,
      RVA 0x30d3dc) — fresh maps use act.CreateMap(state, false) (treasure
      row is never elite-replaced); after generation, if the run did not
      start with Neow and this is act 1, the starting point becomes Monster.
      Hook::ModifyGeneratedMap runs afterwards: in v0.111.0 the only
      topology-changing hooks are BigGameHunter::ModifyGeneratedMap
      (RVA 0xc5fb8, full regeneration with modified counts) and
      SpoilsMap::ModifyGeneratedMapLate. Callers must refuse when those
      models are live (I5) — see REFUSAL_MODEL_IDS.

MapPoint.Children / MapPoint.parents / ActMap.startMapPoints are .NET
HashSet<MapPoint>. In the generation flow every Add precedes every Remove,
so insertion-order iteration (a Python dict-backed set) reproduces .NET
enumeration exactly; no freelist modeling is needed.

MapPointType: 0 Unassigned, 1 Unknown, 2 Shop, 3 Treasure, 4 RestSite,
5 Monster, 6 Elite, 7 Boss, 8 Ancient (mcr_tables.json enum, matches the
IL constants).
"""
from __future__ import annotations

import math

from sts2_rng import GAME_BUILD_V0_111_0, Rng, RunRngSet

UNASSIGNED, UNKNOWN, SHOP, TREASURE, REST = 0, 1, 2, 3, 4
MONSTER, ELITE, BOSS, ANCIENT = 5, 6, 7, 8

TYPE_NAMES = {
    UNASSIGNED: "unassigned", UNKNOWN: "unknown", SHOP: "shop",
    TREASURE: "treasure", REST: "rest_site", MONSTER: "monster",
    ELITE: "elite", BOSS: "boss", ANCIENT: "ancient",
}

# ActModel::get_BaseNumberOfRooms per act family (v0.111.0).
BASE_ROOMS = {
    "ACT.UNDERDOCKS": 15,
    "ACT.OVERGROWTH": 15,
    "ACT.HIVE": 14,
    "ACT.GLORY": 13,
}

# StandardActMap::.cctor (RVA 0xfc69c)
_LOWER_RESTRICTED = {REST, ELITE}           # rows 1-5
_UPPER_RESTRICTED = {REST}                  # rows >= map_length-3
_PARENT_RESTRICTED = {ELITE, REST, TREASURE, SHOP}
_CHILD_RESTRICTED = {ELITE, REST, TREASURE, SHOP}
_SIBLING_RESTRICTED = {REST, MONSTER, UNKNOWN, ELITE, SHOP}

# Models whose ModifyGeneratedMap/-Late hooks change the map. A caller that
# sees one of these live at act entry must refuse rather than approximate.
REFUSAL_MODEL_IDS = ("RELIC.BIG_GAME_HUNTER", "CARD.SPOILS_MAP")


class MapGenRefusal(NotImplementedError):
    pass


class OrderedSet:
    """.NET HashSet<T> enumeration semantics: entries array in insertion
    order, removed slots go on a LIFO freelist and are reused by later
    adds (so an add after a remove lands at the REMOVED position, not the
    end). StandardActMap only ever adds before it removes, but
    SpoilsActMap::RedirectToTreasure re-points live parent/child sets —
    remove-then-add — and path enumeration order feeds the prune RNG, so
    the freelist behaviour is load-bearing there."""

    def __init__(self):
        self._slots: list = []
        self._index: dict[int, int] = {}
        self._free: list[int] = []

    def add(self, p):
        if id(p) in self._index:
            return
        if self._free:
            i = self._free.pop()
            self._slots[i] = p
        else:
            i = len(self._slots)
            self._slots.append(p)
        self._index[id(p)] = i

    def discard(self, p):
        i = self._index.pop(id(p), None)
        if i is not None:
            self._slots[i] = None
            self._free.append(i)

    def __contains__(self, p):
        return id(p) in self._index

    def __iter__(self):
        return iter([p for p in self._slots if p is not None])

    def __len__(self):
        return len(self._index)


class MapPoint:
    __slots__ = ("col", "row", "point_type", "can_be_modified",
                 "parents", "children")

    def __init__(self, col: int, row: int):
        self.col = col
        self.row = row
        self.point_type = UNASSIGNED
        self.can_be_modified = True
        self.parents = OrderedSet()
        self.children = OrderedSet()

    # MapPoint::AddChildPoint / RemoveChildPoint (RVA 0xf9a5b / 0xf9a77)
    def add_child(self, child: "MapPoint"):
        self.children.add(child)
        child.parents.add(self)

    def remove_child(self, child: "MapPoint"):
        self.children.discard(child)
        child.parents.discard(self)

    def __repr__(self):
        return f"P[{self.col},{self.row}]{TYPE_NAMES[self.point_type]}"


def next_gaussian_int(rng: Rng, mean: int, std_dev: int,
                      lo: int, hi: int) -> int:
    """Rng::NextGaussianInt (RVA 0x5edcc). Two draws per attempt; .NET
    Math.Round is half-to-even, which Python's round() matches."""
    while True:
        u1 = 1.0 - rng.next_double()
        u2 = 1.0 - rng.next_double()
        z = math.sqrt(-2.0 * math.log(u1)) * math.sin(2.0 * math.pi * u2)
        r = round(mean + std_dev * z)
        if lo <= r <= hi:
            return r


def stable_shuffle(items: list, rng: Rng, key=None) -> None:
    """ListExtensions::StableShuffle (RVA 0x1131a4): sort into canonical
    order (default comparer), then Fisher-Yates. Keys are unique at every
    call site in map generation, so introsort instability cannot bite."""
    items.sort(key=key)
    rng.shuffle(items)


class MapPointTypeCounts:
    """MapPointTypeCounts::.ctor (RVA 0xfa17c)."""

    def __init__(self, num_unknowns: int, num_rests: int, ascension: int):
        self.ignore_rules: set[int] = set()
        # ldc.r4 5.0 * ldc.r4 1.600000023841858 in float32, then
        # Math.Round on the double widening -> 8 at A1+, else 5.
        mult = 1.600000023841858 if ascension >= 1 else 1.0
        self.num_elites = round(5.0 * mult)
        self.num_shops = 3
        self.num_unknowns = num_unknowns
        self.num_rests = num_rests


def _point_type_counts(act_id: str, rng: Rng,
                       ascension: int) -> MapPointTypeCounts:
    """Per-act GetMapPointTypes overrides. Draw order: rests first, then
    unknowns (both from the act_N_map stream)."""
    if act_id in ("ACT.UNDERDOCKS", "ACT.OVERGROWTH"):
        rests = next_gaussian_int(rng, 7, 1, 6, 7)
        unknowns = next_gaussian_int(rng, 12, 1, 10, 14)
    elif act_id == "ACT.HIVE":
        rests = next_gaussian_int(rng, 6, 1, 6, 7)
        unknowns = next_gaussian_int(rng, 12, 1, 10, 14) - 1
    elif act_id == "ACT.GLORY":
        rests = rng.next_int(5, 7)
        unknowns = next_gaussian_int(rng, 12, 1, 10, 14) - 1
    else:
        raise MapGenRefusal(f"unmodeled act {act_id!r} (I5)")
    return MapPointTypeCounts(unknowns, rests, ascension)


class GeneratedActMap:
    """StandardActMap ported field-for-field. Grid is cols x rows
    (7 x map_length); row 0 is always empty (the start node lives outside
    the grid at (3,0)), the boss at (3, map_length)."""

    COLS = 7

    def __init__(self, rng: Rng, act_id: str, ascension: int,
                 multiplayer: bool, has_second_boss: bool,
                 counts_override: MapPointTypeCounts | None = None):
        self._rng = rng
        rooms = BASE_ROOMS.get(act_id)
        if rooms is None:
            raise MapGenRefusal(f"unmodeled act {act_id!r} (I5)")
        if multiplayer:
            rooms -= 1
        self.map_length = rooms + 1
        self.grid: list[list[MapPoint | None]] = [
            [None] * self.map_length for _ in range(self.COLS)]
        # non-null override (Big Game Hunter's derived counts) skips the
        # Gaussian draws entirely, exactly like the ctor's null check
        self.counts = (counts_override if counts_override is not None
                       else _point_type_counts(act_id, rng, ascension))
        self.boss = MapPoint(self.COLS // 2, self.map_length)
        self.start = MapPoint(self.COLS // 2, 0)
        self.second_boss = (MapPoint(self.COLS // 2, self.map_length + 1)
                            if has_second_boss else None)
        self.start_map_points = OrderedSet()
        self._generate_map()
        self._assign_point_types()
        self._prune_and_repair()
        self._center_grid()
        self._spread_adjacent()
        self._straighten_paths()

    # -- generation -------------------------------------------------------

    def _get_or_create(self, col: int, row: int) -> MapPoint:
        p = self.grid[col][row]
        if p is None:
            p = MapPoint(col, row)
            self.grid[col][row] = p
        return p

    def _generate_map(self):
        rng = self._rng
        for i in range(7):
            p = self._get_or_create(rng.next_int(0, 7), 1)
            if i == 1:
                while p in self.start_map_points:
                    p = self._get_or_create(rng.next_int(0, 7), 1)
            if p not in self.start_map_points:
                self.start_map_points.add(p)
            self._path_generate(p)
        for p in self._row_points(self.map_length - 1):
            p.add_child(self.boss)
        if self.second_boss is not None:
            self.boss.add_child(self.second_boss)
        for p in self._row_points(1):
            self.start.add_child(p)

    def _row_points(self, row: int):
        return [self.grid[c][row] for c in range(self.COLS)
                if self.grid[c][row] is not None]

    def _path_generate(self, start: MapPoint):
        cur = start
        while cur.row < self.map_length - 1:
            col, row = self._generate_next_coord(cur)
            nxt = self._get_or_create(col, row)
            cur.add_child(nxt)
            cur = nxt

    def _generate_next_coord(self, cur: MapPoint) -> tuple[int, int]:
        lo = max(0, cur.col - 1)
        hi = min(cur.col + 1, 6)
        deltas = [-1, 0, 1]
        stable_shuffle(deltas, self._rng)
        for d in deltas:
            new_col = lo if d == -1 else (cur.col if d == 0 else hi)
            if not self._has_invalid_crossover(cur, new_col):
                return new_col, cur.row + 1
        raise RuntimeError("Cannot find next node")

    def _has_invalid_crossover(self, cur: MapPoint, new_col: int) -> bool:
        diff = new_col - cur.col
        if diff == 0:
            return False
        other = self.grid[new_col][cur.row]
        if other is None:
            return False
        for child in other.children:
            if child.col - other.col == -diff:
                return True
        return False

    # -- point types ------------------------------------------------------

    def _all_points(self):
        """ActMap::GetAllMapPoints iteration order: column-major."""
        for c in range(self.COLS):
            for r in range(self.map_length):
                p = self.grid[c][r]
                if p is not None:
                    yield p

    def _assign_point_types(self):
        for p in self._row_points(self.map_length - 1):
            p.point_type = REST
            p.can_be_modified = False
        # replaceTreasureWithElites is always false from RunManager
        for p in self._row_points(self.map_length - 7):
            p.point_type = TREASURE
            p.can_be_modified = False
        for p in self._row_points(1):
            p.point_type = MONSTER
            p.can_be_modified = False
        queue = ([REST] * self.counts.num_rests
                 + [SHOP] * self.counts.num_shops
                 + [ELITE] * self.counts.num_elites
                 + [UNKNOWN] * self.counts.num_unknowns)
        self._assign_remaining(queue)
        for p in self._all_points():
            if p.point_type == UNASSIGNED:
                p.point_type = MONSTER
        self.boss.point_type = BOSS
        self.start.point_type = ANCIENT
        if self.second_boss is not None:
            self.second_boss.point_type = BOSS

    def _assign_remaining(self, queue: list[int]):
        for _ in range(3):
            cands = [p for p in self._all_points()
                     if p.point_type == UNASSIGNED]
            stable_shuffle(cands, self._rng, key=lambda p: (p.col, p.row))
            for p in cands:
                if not queue:
                    break
                p.point_type = self._next_valid_type(queue, p)
            if not queue:
                break

    def _next_valid_type(self, queue: list[int], p: MapPoint) -> int:
        for _ in range(len(queue)):
            t = queue.pop(0)
            if t in self.counts.ignore_rules:
                return t
            if self._is_valid_type(t, p):
                return t
            queue.append(t)
        return UNASSIGNED

    def _is_valid_type(self, t: int, p: MapPoint) -> bool:
        if p.row >= self.map_length - 3 and t in _UPPER_RESTRICTED:
            return False
        if p.row < 6 and t in _LOWER_RESTRICTED:
            return False
        if t in _PARENT_RESTRICTED and any(
                x.point_type == t
                for x in list(p.parents) + list(p.children)):
            return False
        if t in _CHILD_RESTRICTED and any(
                c.point_type == t for c in p.children):
            return False
        if t in _SIBLING_RESTRICTED and any(
                s.point_type == t for s in self._siblings(p)):
            return False
        return True

    def _siblings(self, p: MapPoint):
        for parent in p.parents:
            for c in parent.children:
                if c is not p:
                    yield c

    # -- pruning (MapPathPruning) ----------------------------------------

    def _prune_and_repair(self):
        for _ in range(3):
            self._prune_duplicate_segments()
            if not self._repair_pruned_point_types():
                break

    def _prune_duplicate_segments(self):
        count = 0
        segs = self._find_matching_segments()
        while self._prune_paths(segs):
            count += 1
            if count > 50:
                raise RuntimeError(
                    f"Unable to prune matching segments in {count} iterations")
            segs = self._find_matching_segments()

    def _find_all_paths(self, p: MapPoint) -> list[list[MapPoint]]:
        if p.point_type == BOSS:
            return [[p]]
        out = []
        for child in p.children:
            for sub in self._find_all_paths(child):
                out.append([p] + sub)
        return out

    def _find_matching_segments(self) -> list[list[list[MapPoint]]]:
        # The engine collects segments in a SortedDictionary<string, ...>
        # keyed with StringComparer.Ordinal (FindMatchingSegments,
        # RVA 0xf90a8): duplicate lists come out KEY-SORTED, not in
        # insertion order. Python's plain str ordering matches ordinal
        # comparison for these ASCII keys.
        paths = self._find_all_paths(self.start)
        d: dict[str, list[list[MapPoint]]] = {}
        for path in paths:
            self._add_segments(path, d)
        return [v for _, v in sorted(d.items()) if len(v) > 1]

    @staticmethod
    def _is_valid_segment_start(p: MapPoint) -> bool:
        return len(p.children) > 1 or p.row == 0

    @staticmethod
    def _is_valid_segment_end(p: MapPoint) -> bool:
        return len(p.parents) >= 2

    def _add_segments(self, path: list[MapPoint],
                      d: dict[str, list[list[MapPoint]]]):
        for i in range(len(path) - 1):
            if not self._is_valid_segment_start(path[i]):
                continue
            length = 2
            while length < len(path) - i:
                end = path[i + length]
                if self._is_valid_segment_end(end):
                    seg = path[i:i + length + 1]
                    key = self._segment_key(seg)
                    if key not in d:
                        d[key] = [seg]
                    elif not any(self._overlapping(x, seg) for x in d[key]):
                        d[key].append(seg)
                length += 1

    @staticmethod
    def _segment_key(seg: list[MapPoint]) -> str:
        first, last = seg[0], seg[-1]
        if first.row == 0:
            hdr = f"{first.row}-{last.col},{last.row}-"
        else:
            hdr = f"{first.col},{first.row}-{last.col},{last.row}-"
        return hdr + ",".join(str(p.point_type) for p in seg)

    @staticmethod
    def _overlapping(a: list[MapPoint], b: list[MapPoint]) -> bool:
        if len(a) < 3 or len(b) < 3:
            return False
        for i in range(1, len(a) - 1):
            if a[i].col == b[i].col and a[i].row == b[i].row:
                return True
        return False

    def _prune_paths(self, seg_lists: list[list[list[MapPoint]]]) -> bool:
        for seg_list in seg_lists:
            self._rng.shuffle(seg_list)          # UnstableShuffle
            if self._prune_all_but_last(seg_list):
                return True
            if self._break_relationship_in_any(seg_list):
                return True
        return False

    def _prune_all_but_last(self, seg_list: list[list[MapPoint]]) -> bool:
        pruned = 0
        for seg in seg_list:
            if pruned == len(seg_list) - 1:
                return pruned > 0
            if self._prune_segment(seg):
                pruned += 1
        return pruned > 0

    def _is_in_map(self, p: MapPoint) -> bool:
        if (0 <= p.col < self.COLS and 0 <= p.row < self.map_length
                and self.grid[p.col][p.row] is not None):
            return True
        return p.point_type in (BOSS, ANCIENT)

    def _is_removed(self, p: MapPoint) -> bool:
        if not (0 <= p.col < self.COLS and 0 <= p.row < self.map_length):
            return False
        return self.grid[p.col][p.row] is None

    def _prune_segment(self, seg: list[MapPoint]) -> bool:
        removed = False
        for i in range(len(seg) - 1):
            p = seg[i]
            if not self._is_in_map(p):
                return True
            if len(p.children) > 1 or len(p.parents) > 1:
                continue
            if any(len(parent.children) == 1 and not self._is_removed(parent)
                   for parent in p.parents):
                continue
            rest = seg[i:]
            if any(len(x.children) > 1 and len(x.parents) == 1
                   for x in rest):
                continue
            if len(seg[-1].parents) == 1:
                return False
            if any(c not in seg and len(c.parents) == 1
                   for c in p.children):
                continue
            self._remove_point(p)
            removed = True
        return removed

    def _remove_point(self, p: MapPoint):
        self.grid[p.col][p.row] = None
        self.start_map_points.discard(p)
        for c in list(p.children):
            p.remove_child(c)
        for parent in list(p.parents):
            parent.remove_child(p)

    @staticmethod
    def _break_relationship_in_any(
            seg_list: list[list[MapPoint]]) -> bool:
        for seg in seg_list:
            changed = False
            for i in range(len(seg) - 1):
                a = seg[i]
                if len(a.children) < 2:
                    continue
                b = seg[i + 1]
                if len(b.parents) == 1:
                    continue
                a.remove_child(b)
                changed = True
            if changed:
                return True
        return False

    def _repair_pruned_point_types(self) -> bool:
        changed = False
        for t, target in ((SHOP, self.counts.num_shops),
                          (ELITE, self.counts.num_elites),
                          (REST, self.counts.num_rests),
                          (UNKNOWN, self.counts.num_unknowns)):
            changed |= self._repair_point_type(t, target)
        return changed

    def _repair_point_type(self, t: int, target: int) -> bool:
        current = sum(1 for p in self._all_points() if p.point_type == t)
        deficit = target - current
        if deficit <= 0:
            return False
        changed = False
        cands = [p for p in self._all_points()
                 if p.point_type == MONSTER and p.can_be_modified]
        stable_shuffle(cands, self._rng, key=lambda p: (p.col, p.row))
        for p in cands:
            if deficit == 0:
                break
            if self._is_valid_type(t, p):
                p.point_type = t
                deficit -= 1
                changed = True
        return changed

    # -- post-processing (MapPostProcessing) ------------------------------

    def _column_empty(self, col: int) -> bool:
        return all(self.grid[col][r] is None for r in range(self.map_length))

    def _center_grid(self):
        left = self._column_empty(0) and self._column_empty(1)
        right = (self._column_empty(self.COLS - 1)
                 and self._column_empty(self.COLS - 2))
        if left and not right:
            shift = -1
        elif right and not left:
            shift = 1
        else:
            return
        for row in range(self.map_length):
            cols = (range(self.COLS - 1, -1, -1) if shift > 0
                    else range(self.COLS))
            for col in cols:
                p = self.grid[col][row]
                self.grid[col][row] = None
                nc = col + shift
                if 0 <= nc < self.COLS:
                    self.grid[nc][row] = p
                    if p is not None:
                        p.col = nc

    def _allowed_positions(self, p: MapPoint) -> list[int]:
        allowed = set(range(self.COLS))
        for nbr in list(p.parents) + list(p.children):
            allowed &= {c for c in (nbr.col - 1, nbr.col, nbr.col + 1)
                        if 0 <= c < self.COLS}
        return sorted(allowed)

    @staticmethod
    def _gap(col: int, row_points: list[MapPoint], exclude: MapPoint) -> int:
        best = 2 ** 31 - 1
        for other in row_points:
            if other is exclude:
                continue
            best = min(best, abs(col - other.col))
        return best

    def _spread_adjacent(self):
        for row in range(self.map_length):
            row_points = self._row_points(row)
            while True:
                changed = False
                for p in row_points:
                    cur = p.col
                    best, best_gap = cur, self._gap(cur, row_points, p)
                    for pos in self._allowed_positions(p):
                        if pos == cur:
                            continue
                        occ = self.grid[pos][row]
                        if occ is not None and occ is not p:
                            continue
                        g = self._gap(pos, row_points, p)
                        if g > best_gap:
                            best, best_gap = pos, g
                    if best != cur:
                        self.grid[cur][row] = None
                        self.grid[best][row] = p
                        p.col = best
                        changed = True
                if not changed:
                    break

    def _straighten_paths(self):
        for row in range(self.map_length):
            for col in range(self.COLS):
                p = self.grid[col][row]
                if p is None:
                    continue
                if len(p.parents) != 1 or len(p.children) != 1:
                    continue
                parent = next(iter(p.parents))
                child = next(iter(p.children))
                move_right = p.col < child.col and p.col < parent.col
                move_left = p.col > child.col and p.col > parent.col
                if move_right:
                    if col < self.COLS - 1 and self.grid[col + 1][row] is None:
                        p.col = col + 1
                        self.grid[col][row] = None
                        self.grid[col + 1][row] = p
                elif move_left:
                    if col > 0 and self.grid[col - 1][row] is None:
                        p.col = col - 1
                        self.grid[col][row] = None
                        self.grid[col - 1][row] = p


class SpoilsActMapGen(GeneratedActMap):
    """SpoilsActMap (RVA 0xfacb8) — the hourglass replacement map the
    Spoils Map card installs for act index 1. Own stream
    Rng(Seed, "spoils_map"); same restriction sets and MapPathPruning as
    StandardActMap; NO post-processing (no center/spread/straighten) and
    never a second boss. The waist: every treasure-row point collapses
    into the single centre treasure node (RedirectToTreasure,
    RVA 0xfb44c), whose remove-then-add re-pointing is why OrderedSet
    models the .NET HashSet freelist."""

    def __init__(self, rng: Rng, act_id: str, ascension: int,
                 multiplayer: bool,
                 counts_override: MapPointTypeCounts | None = None):
        self._rng = rng
        rooms = BASE_ROOMS.get(act_id)
        if rooms is None:
            raise MapGenRefusal(f"unmodeled act {act_id!r} (I5)")
        if multiplayer:
            rooms -= 1
        self.map_length = rooms + 1
        self.grid = [[None] * self.map_length for _ in range(self.COLS)]
        self.counts = (counts_override if counts_override is not None
                       else _point_type_counts(act_id, rng, ascension))
        self.treasure_row = self.map_length - 7
        self.boss = MapPoint(self.COLS // 2, self.map_length)
        self.start = MapPoint(self.COLS // 2, 0)
        self.second_boss = None
        self.start_map_points = OrderedSet()
        self._generate_hourglass()
        self._assign_point_types()
        self._prune_and_repair()

    # -- generation (GenerateHourglassMap, RVA 0xfadfc) -------------------

    def _generate_hourglass(self):
        rng = self._rng
        if not (0 < self.treasure_row < self.map_length):
            raise RuntimeError("Treasure row is out of bounds")
        for i in range(7):
            p = self._get_or_create(rng.next_int(0, 7), 1)
            if i == 1:
                while p in self.start_map_points:
                    p = self._get_or_create(rng.next_int(0, 7), 1)
            self.start_map_points.add(p)
            self._path_generate(p)
        t = self._get_or_create(self.COLS // 2, self.treasure_row)
        t.point_type = TREASURE
        t.can_be_modified = False
        for p in list(self._row_points(self.treasure_row)):
            if p is not t:
                self._redirect_to_treasure(p, t)
        for c in range(self.COLS):  # ConnectRowToBoss
            p = self.grid[c][self.map_length - 1]
            if p is not None and self.boss not in p.children:
                p.add_child(self.boss)
        for c in range(self.COLS):  # ConnectRowToStart
            p = self.grid[c][1]
            if p is not None and p not in self.start.children:
                self.start.add_child(p)

    def _redirect_to_treasure(self, p: MapPoint, t: MapPoint):
        for parent in list(p.parents):
            parent.remove_child(p)
            parent.add_child(t)
        for child in list(p.children):
            p.remove_child(child)
            t.add_child(child)
        self.grid[p.col][p.row] = None

    # GetAllowedColumnsForRow (RVA 0xfb33c): the hourglass bounds — width
    # pinches to the centre column at the treasure row and near the boss.
    def _allowed_columns_for_row(self, row: int) -> tuple[int, int]:
        mid = self.COLS // 2
        dist_t = abs(row - self.treasure_row)
        dist_b = (self.map_length - 1) - row
        k = min(mid, max(0, dist_b) + 1)
        span = min(mid, min(dist_t, k))
        return max(0, mid - span), min(6, mid + span)

    @staticmethod
    def _centered_priority(col: int, mid: int) -> list[int]:
        # BuildCenteredPriorityList (RVA 0xfb29c): funnel toward centre.
        s = (mid > col) - (mid < col)
        out = []
        if s != 0:
            out.append(s)
        out.append(0)
        if s != 0:
            out.append(-s)
        if -1 not in out:
            out.append(-1)
        if 1 not in out:
            out.append(1)
        return out

    def _generate_next_coord(self, cur: MapPoint) -> tuple[int, int]:
        # GenerateNextCoord (RVA 0xfaf60)
        next_row = cur.row + 1
        lo, hi = self._allowed_columns_for_row(next_row)
        mid = self.COLS // 2
        deltas = [-1, 0, 1]
        dist = self.treasure_row - cur.row
        if dist > 3:
            stable_shuffle(deltas, self._rng)
        elif dist > 0:
            deltas = self._centered_priority(cur.col, mid)
        else:
            stable_shuffle(deltas, self._rng)
        for d in deltas:
            if d == -1:
                nc = max(0, cur.col - 1)
            elif d == 1:
                nc = min(6, cur.col + 1)
            else:
                nc = cur.col
            if nc < lo or nc > hi:
                continue
            if self._has_invalid_crossover(cur, nc):
                continue
            existing = (self.grid[nc][next_row]
                        if next_row < self.map_length else None)
            if existing is not None and cur not in existing.parents:
                if len(existing.parents) >= 3:  # merge cap
                    continue
            if cur is not self.start and len(cur.children) >= 3:
                # branch cap: only re-walk an edge that already exists
                if existing is None or existing not in cur.children:
                    continue
            if abs(nc - cur.col) > 1:
                raise RuntimeError("Invalid step")
            return nc, next_row
        # fallback: head for the centre within the allowed band
        c = min(max(mid, lo), hi)
        if abs(c - cur.col) > 1:
            step = (c > cur.col) - (c < cur.col)
            c = min(max(cur.col + step, lo), hi)
        if self._has_invalid_crossover(cur, c):
            c = min(max(cur.col, lo), hi)
        if abs(c - cur.col) > 1:
            raise RuntimeError("Fallback step exceeds adjacency")
        return c, next_row

    def _path_generate(self, start_point: MapPoint):
        cur = start_point
        while cur.row < self.map_length - 1:
            col, row = self._generate_next_coord(cur)
            nxt = self._get_or_create(col, row)
            cur.add_child(nxt)
            cur = nxt

    # -- point types (AssignPointTypes, RVA 0xfb5c4) ----------------------

    def _assign_point_types(self):
        for p in self._row_points(self.map_length - 1):
            p.point_type = REST
            p.can_be_modified = False
        for p in self._row_points(1):
            p.point_type = MONSTER
            p.can_be_modified = False
        queue = ([REST] * self.counts.num_rests
                 + [SHOP] * self.counts.num_shops
                 + [ELITE] * self.counts.num_elites
                 + [UNKNOWN] * self.counts.num_unknowns)
        self._assign_remaining(queue)
        for p in self._all_points():
            if p.point_type == UNASSIGNED:
                p.point_type = MONSTER
        self.boss.point_type = BOSS
        self.start.point_type = ANCIENT


def _bgh_counts(m: GeneratedActMap, ascension: int) -> MapPointTypeCounts:
    """BigGameHunter::ModifyGeneratedMap (RVA 0xc5fb8): counts derived
    from the FIRST-pass map, elites = round(existing elites * 2.5f)
    (half-to-even), elite placement rules ignored."""
    unknowns = sum(1 for p in m._all_points() if p.point_type == UNKNOWN)
    rests = sum(1 for p in m._all_points() if p.point_type == REST)
    elites = sum(1 for p in m._all_points() if p.point_type == ELITE)
    c = MapPointTypeCounts(unknowns, rests, ascension)
    c.num_elites = round(elites * 2.5)
    c.ignore_rules = {ELITE}
    return c


def generate_act_map(seed_string: str, act_index: int, act_id: str, *,
                     ascension: int, started_with_neow: bool = True,
                     multiplayer: bool = False,
                     has_second_boss: bool = False,
                     build: str = GAME_BUILD_V0_111_0) -> GeneratedActMap:
    """StandardActMap::CreateFor + the RunManager post-step. has_second_boss
    is true only for the final act at Ascension 10+ (RunManager::
    GenerateRooms sets SecondBoss under AscensionManager.HasLevel(10))."""
    run_set = RunRngSet(seed_string, build=build)
    rng = Rng.from_stream(run_set.seed, f"act_{act_index + 1}_map",
                          build=build)
    m = GeneratedActMap(rng, act_id, ascension, multiplayer, has_second_boss)
    if act_index == 0 and not started_with_neow:
        m.start.point_type = MONSTER
    return m


SPOILS_ACT_INDEX = 1  # SpoilsMap::AfterCreated (RVA 0xec692): constant


def generate_spoils_act_map(seed_string: str, act_id: str, *,
                            ascension: int, multiplayer: bool = False,
                            build: str = GAME_BUILD_V0_111_0
                            ) -> SpoilsActMapGen:
    """The act map when CARD.SPOILS_MAP sits in the deck at act-2 entry:
    SpoilsMap::ModifyGeneratedMap replaces the freshly generated map with
    `new SpoilsActMap(state, null)` for act index SPOILS_ACT_INDEX only.
    Callers that also see RELIC.BIG_GAME_HUNTER in the run must refuse:
    the combined outcome depends on run listener order, which this module
    does not model (I5)."""
    run_set = RunRngSet(seed_string, build=build)
    rng = Rng.from_stream(run_set.seed, "spoils_map", build=build)
    return SpoilsActMapGen(rng, act_id, ascension, multiplayer)


def generate_bgh_act_map(seed_string: str, act_index: int, act_id: str, *,
                         ascension: int, started_with_neow: bool = True,
                         multiplayer: bool = False,
                         has_second_boss: bool = False,
                         build: str = GAME_BUILD_V0_111_0
                         ) -> GeneratedActMap:
    """The act map when RELIC.BIG_GAME_HUNTER is held at act entry:
    the hook regenerates on a FRESH act_N_map stream with counts derived
    from the first-pass map (no Gaussian draws on the second pass).
    Combined with Spoils Map: refuse (see generate_spoils_act_map)."""
    first = generate_act_map(
        seed_string, act_index, act_id, ascension=ascension,
        started_with_neow=True, multiplayer=multiplayer,
        has_second_boss=has_second_boss, build=build)
    counts = _bgh_counts(first, ascension)
    run_set = RunRngSet(seed_string, build=build)
    rng = Rng.from_stream(run_set.seed, f"act_{act_index + 1}_map",
                          build=build)
    m = GeneratedActMap(rng, act_id, ascension, multiplayer,
                        has_second_boss, counts_override=counts)
    if act_index == 0 and not started_with_neow:
        m.start.point_type = MONSTER
    return m


def to_save_shape(m: GeneratedActMap) -> dict:
    """Render in the decoded-save `saved_map` shape for comparison."""
    def coord(p):
        return {"col": p.col, "row": p.row}

    def point(p):
        return {
            "coord": coord(p),
            "type": TYPE_NAMES[p.point_type],
            "can_modify": p.can_be_modified,
            "children": [coord(c) for c in p.children],
        }

    out = {
        "width": m.COLS,
        "height": m.map_length,
        "boss": point(m.boss),
        "start": point(m.start),
        "points": [point(p) for p in m._all_points()],
        "start_coords": [coord(p) for p in m.start_map_points],
    }
    if m.second_boss is not None:
        out["second_boss"] = point(m.second_boss)
    return out

"""
Slay the Spire 2 RNG — Python port of the post-overhaul (June 2026) system.

Reverse-engineered from the shipped sts2.dll via IL disassembly:
  - build v0.108.0 (commit 58694f64, 2026-07-02): the original port. The
    same build produced sim/v0.111.0/python/fight_states.json, so the v0.108 scheme
    applies to all pre-2026-07-16 captures/pins.
  - build v0.109.0 (commit c12f634d, 2026-07-16): per-stream SEED
    DERIVATION changed (issue #309); the core algorithm did not.
  - build v0.109.1 (commit c8c577f6, 2026-07-20): point-release
    verification found the cited RNG types and CIL bodies byte-identical,
    and all 70 existing engine-pinned hash/run/player vectors matched.
  - build v0.110.1 (commit db5d3552, 2026-07-31): the complete managed
    diff changed combat/content surfaces but not RNG derivation.  A fresh
    engine probe matched all 267 scalar hash/run/player observations and
    all 42 per-fight Encounter observations.  Only this exact build is
    admitted to the v109 RNG route. Combat prediction remained fail-closed
    until the remodel umbrella #794 was resolved and is now admitted.
  - build v0.111.0 (commit 41cef1ea, 2026-08-13): a fresh engine probe
    again matched all 267 scalar hash/run/player observations and all 42
    per-fight Encounter observations.  The exhaustive stream-getter scan
    found one new Niche consumer (BeautifulBracelet.AfterObtained). Batch 296
    pins its full-pool shuffle cost and explicit `.run` acquisition boundary;
    the seed derivation itself is exact.

Architecture (all names as in the game assembly):

  MegaRandom  (MegaCrit.Sts2.Core.Random.MegaRandom)
      xoshiro256** with the canonical splitmix64 seeding routine.
      Replaced C# System.Random in the ~June 19, 2026 RNG-overhaul patch.
      UNCHANGED in v0.109 (draw conversions byte-identical in IL; v0.109
      adds unrelated _incrDouble/_incrFloat fields, unused by Rng draws).

  Rng  (MegaCrit.Sts2.Core.Random.Rng)
      Wrapper adding a call counter. Every public draw method consumes
      exactly one 64-bit xoshiro output and bumps the counter by one, so
      save/load fast-forwards by replaying NextInt() `counter` times.
      (v0.109: `Counter` property became a `_counter` field; semantics
      unchanged. Save schema v19 serializes full xoshiro state instead.)

  RunRngSet / PlayerRngSet — named streams, seed derivation is
  BUILD-DEPENDENT (pass `build=` from release_info.json's "version"):

    v0.108 scheme (builds <= v0.108.x) — 32-bit, int32 wraparound:
        run_set_seed    = djb2(seed_string)
        player_set_seed = djb2(seed_string) + player_slot_index
        stream_seed     = int32(set_seed + djb2(snake_case(enum_name)))
        MegaRandom seed = stream_seed zero-extended via CIL conv.u8
      where djb2() is .NET's old "deterministic string hash" (two-lane
      djb2 variant, int32 wraparound) — deterministic_hash_code().

    v0.109 scheme (build v0.109.x) — 64-bit, uint64 wraparound:
        hash64(s)       = XxHash64(UTF8(s), seed=0)   (System.IO.Hashing)
        run_set_seed    = hash64(seed_string), EXCEPT seed strings starting
                          with "old": zext32(djb2(seed_string[3:]))
                          [RunRngSet::.ctor IL, RVA 0x50f70]
        player_set_seed = uint64(hash64(seed_string) + slot_index)
                          (no "old" escape) [Player::InitializeSeed 0x2c2bda]
        stream_seed     = uint64(set_seed + hash64(snake_case(enum_name)))
                          [RunRngSet::CreateRng 0x5100c -> Rng::.ctor 0x61be9]
        MegaRandom seed = the full 64-bit stream_seed (no truncation)
      IL cites are against sts2.dll v0.109.0 (sim/dll-archive/v0.109.0);
      every cited type and body was verified byte-identical in v0.109.1.
      GetDeterministicHashCode (RVA 0x2b6ba8) = XxHash64.HashToUInt64 over
      UTF-8 bytes with seed 0; the old hash survives as
      GetDeterministicHashCodeOld (RVA 0x2b6c38). Stream enum names are
      unchanged (RunRngType still has CombatOrbs; only the C# getter was
      renamed to CombatOrbGeneration).
      Harness-verified (build v0.109.0 commit c12f634d, fresh boot,
      sim/v0.111.0/python/harness/probe_rng_seeding.py): set seeds + first NextInt(100)
      draws of all 12 run streams and 3 player streams match the engine
      exactly for seeds TESTBATCH0 / ZPJHU3WSH2 / 8DVXPWUWRY / HQPAXCBS6P
      / oldZPJHU3WSH2 (slots 0 and 1 for player sets).
      The probe was rerun against build v0.109.1 commit c8c577f6 and all
      70 pinned comparisons matched the v0.109.0 observations exactly.

  Rng.for_encounter() — the PER-FIGHT Encounter stream. An ad-hoc Rng
      rather than a member of either named set, and NOT persisted in the
      save, so it has no recorded state to falsify a derivation against
      and always starts at counter 0. Same build split, same three addends
      [EncounterModel::GenerateMonstersWithSlots 0x22bc0c]:
        v0.108: int32 (run_set_seed + total_floor + djb2(Id.Entry))
        v0.109: uint64(run_set_seed + total_floor + hash64(Id.Entry))
      Engine-verified against v0.109.1 for 42 (seed, encounter, floor)
      cases (#637, sim/v0.111.0/python/harness/probe_encounter_stream.py). All six
      per-fight roster rolls in sim/v0.111.0/python/content/encounters/ reach it through
      _EncounterCtx.encounter_rng — never re-derive it inline.

  The exact verified builds v0.110.1 and v0.111.0 use the v109 RNG scheme.
  Other builds newer than v0.109 REFUSE until re-verified (I5:
  refuse-don't-guess).

The default is `build=GAME_BUILD_V0_108` so every pre-existing pin,
capture and test keeps the seeding it was recorded under; v0.109+ callers
must pass the build explicitly (thread it from release_info.json or the
capture's recorded build — never guess).

Everything here is pure Python with no dependencies.
"""

from __future__ import annotations

import re

MASK32 = 0xFFFFFFFF
MASK64 = 0xFFFFFFFFFFFFFFFF

# Game build identifiers (release_info.json "version") for the seeding
# switch. Pass the build a save/replay/capture was RECORDED under.
GAME_BUILD_V0_108 = "v0.108.0"
GAME_BUILD_V0_109 = "v0.109.0"
GAME_BUILD_V0_109_1 = "v0.109.1"
GAME_BUILD_V0_110_1 = "v0.110.1"
GAME_BUILD_V0_111_0 = "v0.111.0"

_BUILD_RE = re.compile(r"^v(\d+)\.(\d+)(?:\.(\d+))?$")


# Every build whose per-stream seeding has been verified against its own
# sts2.dll, and the scheme that verification established. This is an
# ENUMERATION on purpose (#1265, SOLVER_INVARIANTS.md I11): it replaced
# `(major, minor) <= (0, 108)`, which extrapolated a derivation verified at
# v0.108.0 across every earlier build -- nine minor versions, and 100% of the
# local run corpus, on no evidence. A build is added here only after an IL
# read plus a harness probe, never because it looks adjacent to one that was.
_VERIFIED_SEEDING = {
    # The original port. The same build produced fight_states.json, so the
    # v0.108 scheme is pinned by every pre-#309 replay.
    GAME_BUILD_V0_108: "v108",
    # #309: per-stream SEED derivation moved to 64-bit XxHash64.
    GAME_BUILD_V0_109: "v109",
    # Point release; #625 found zero managed changes from v0.109.0.
    GAME_BUILD_V0_109_1: "v109",
    # #793: exact re-verification against the archived DLL.
    GAME_BUILD_V0_110_1: "v109",
    # #1214: fresh engine probe, 267 scalar + 42 Encounter observations.
    GAME_BUILD_V0_111_0: "v109",
}


def seeding_scheme(build: str) -> str:
    """Return the verified per-stream seeding scheme for an exact build.

    Refuses anything not in ``_VERIFIED_SEEDING``. Older builds refuse for the
    same reason newer ones do: the scheme was never checked against their DLL.
    Guessing it silently derives every stream wrongly -- omitting the build on
    a v0.109 save once produced a 48 HP Nibbit instead of 44 and a diverging
    shuffle (2026-07-26 live session).
    """
    if not isinstance(build, str):
        raise NotImplementedError(
            f"unrecognized game build string {build!r}; expected e.g. "
            f"'v0.109.0' (release_info.json 'version')")
    build = build.strip()
    if not _BUILD_RE.match(build):
        raise NotImplementedError(
            f"unrecognized game build string {build!r}; expected e.g. "
            f"'v0.109.0' (release_info.json 'version')")
    scheme = _VERIFIED_SEEDING.get(build)
    if scheme is None:
        raise NotImplementedError(
            f"game build {build} has no verified RNG route: seeding must be "
            f"re-verified against its sts2.dll before predictions can be "
            f"trusted. Verified: {', '.join(sorted(_VERIFIED_SEEDING))} "
            f"(extend _VERIFIED_SEEDING after an IL read + harness check; "
            f"see #1214 / sim/v0.111.0/python/harness/probe_rng_seeding.py)")
    return scheme


# ---------------------------------------------------------------------------
# String hashing & stream names
# ---------------------------------------------------------------------------

def deterministic_hash_code(s: str) -> int:
    """v0.108 StringHelper.GetDeterministicHashCode — signed int32.
    (Still shipped in v0.109 as GetDeterministicHashCodeOld, RVA 0x2b6c38,
    used for the "old"-prefixed run-seed escape path.)

    h1 = h2 = 0x15051505; chars alternate between the two lanes
    (h = (h << 5) + h ^ ch, wrapping at 32 bits); result h1 + h2 * 1566083941.
    """
    h1 = h2 = 352654597
    i = 0
    n = len(s)
    while i < n:
        h1 = (((h1 << 5) + h1) & MASK32) ^ ord(s[i])
        if i == n - 1:
            break
        h2 = (((h2 << 5) + h2) & MASK32) ^ ord(s[i + 1])
        i += 2
    v = (h1 + h2 * 1566083941) & MASK32
    return v - 0x100000000 if v >= 0x80000000 else v


# --- canonical xxHash64 (XXH64), pure Python -------------------------------
# v0.109's StringHelper.GetDeterministicHashCode (RVA 0x2b6ba8) is
# System.IO.Hashing.XxHash64.HashToUInt64(UTF8(s), seed: 0), i.e. the
# canonical XXH64. Ported from the reference spec; engine-verified via
# sim/v0.111.0/python/harness/probe_rng_seeding.py (build v0.109.0 commit c12f634d).

_XXP1 = 0x9E3779B185EBCA87
_XXP2 = 0xC2B2AE3D27D4EB4F
_XXP3 = 0x165667B19E3779F9
_XXP4 = 0x85EBCA77C2B2AE63
_XXP5 = 0x27D4EB2F165667C5


def _xx_round(acc: int, inp: int) -> int:
    acc = (acc + inp * _XXP2) & MASK64
    acc = _rotl(acc, 31)
    return (acc * _XXP1) & MASK64


def _xx_merge_round(h: int, v: int) -> int:
    h ^= _xx_round(0, v)
    return (h * _XXP1 + _XXP4) & MASK64


def xxhash64(data: bytes, seed: int = 0) -> int:
    """Canonical XXH64 over bytes; returns uint64."""
    n = len(data)
    i = 0
    if n >= 32:
        v1 = (seed + _XXP1 + _XXP2) & MASK64
        v2 = (seed + _XXP2) & MASK64
        v3 = seed & MASK64
        v4 = (seed - _XXP1) & MASK64
        while i <= n - 32:
            v1 = _xx_round(v1, int.from_bytes(data[i:i + 8], "little"))
            v2 = _xx_round(v2, int.from_bytes(data[i + 8:i + 16], "little"))
            v3 = _xx_round(v3, int.from_bytes(data[i + 16:i + 24], "little"))
            v4 = _xx_round(v4, int.from_bytes(data[i + 24:i + 32], "little"))
            i += 32
        h = (_rotl(v1, 1) + _rotl(v2, 7) + _rotl(v3, 12)
             + _rotl(v4, 18)) & MASK64
        h = _xx_merge_round(h, v1)
        h = _xx_merge_round(h, v2)
        h = _xx_merge_round(h, v3)
        h = _xx_merge_round(h, v4)
    else:
        h = (seed + _XXP5) & MASK64
    h = (h + n) & MASK64
    while i + 8 <= n:
        h ^= _xx_round(0, int.from_bytes(data[i:i + 8], "little"))
        h = (_rotl(h, 27) * _XXP1 + _XXP4) & MASK64
        i += 8
    if i + 4 <= n:
        h ^= (int.from_bytes(data[i:i + 4], "little") * _XXP1) & MASK64
        h = (_rotl(h, 23) * _XXP2 + _XXP3) & MASK64
        i += 4
    while i < n:
        h ^= (data[i] * _XXP5) & MASK64
        h = (_rotl(h, 11) * _XXP1) & MASK64
        i += 1
    h ^= h >> 33
    h = (h * _XXP2) & MASK64
    h ^= h >> 29
    h = (h * _XXP3) & MASK64
    h ^= h >> 32
    return h


def deterministic_hash_code_v109(s: str) -> int:
    """v0.109 StringHelper.GetDeterministicHashCode — uint64.
    XxHash64.HashToUInt64(Encoding.UTF8.GetBytes(s), seed: 0)."""
    return xxhash64(s.encode("utf-8"), 0)


_CAMEL = re.compile(r"(?<=[A-Za-z0-9])([A-Z])")


def snake_case(name: str) -> str:
    """StringHelper.SnakeCase: regex ([A-Za-z0-9]|\\G(?!^))([A-Z]) -> $1_$2,
    then lowercase. For enum names (no consecutive caps, no digits before
    caps) this equals inserting _ before each non-initial uppercase letter."""
    return _CAMEL.sub(r"_\1", name.strip()).lower()


# Enum field names exactly as declared in the assembly.
RUN_RNG_STREAMS = [
    "UpFront",                 # 0
    "Shuffle",                 # 1
    "UnknownMapPoint",         # 2
    "CombatCardGeneration",    # 3
    "CombatPotionGeneration",  # 4
    "CombatCardSelection",     # 5
    "CombatEnergyCosts",       # 6
    "CombatTargets",           # 7
    "MonsterAi",               # 8
    "Niche",                   # 9
    "CombatOrbs",              # 10
    "TreasureRoomRelics",      # 11
]

PLAYER_RNG_STREAMS = [
    "Rewards",          # 0
    "Shops",            # 1
    "Transformations",  # 2
]


# ---------------------------------------------------------------------------
# MegaRandom = xoshiro256** seeded via splitmix64
# ---------------------------------------------------------------------------

def _splitmix64_next(state: int) -> tuple[int, int]:
    state = (state + 0x9E3779B97F4A7C15) & MASK64
    z = state
    z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & MASK64
    z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & MASK64
    return z ^ (z >> 31), state


def _rotl(x: int, k: int) -> int:
    return ((x << k) | (x >> (64 - k))) & MASK64


class MegaRandom:
    """xoshiro256**. Seed is a ulong; the game passes the Rng's int32 seed
    through CIL conv.u8, which zero-extends the 32-bit two's-complement
    pattern (so seed -1 becomes 0x00000000FFFFFFFF, not all-ones)."""

    __slots__ = ("s0", "s1", "s2", "s3")

    def __init__(self, seed: int):
        seed &= MASK64
        self.s0, seed = _splitmix64_next(seed)
        self.s1, seed = _splitmix64_next(seed)
        self.s2, seed = _splitmix64_next(seed)
        self.s3, seed = _splitmix64_next(seed)

    def next_ulong(self) -> int:
        result = (_rotl((self.s1 * 5) & MASK64, 7) * 9) & MASK64
        t = (self.s1 << 17) & MASK64
        self.s2 ^= self.s0
        self.s3 ^= self.s1
        self.s1 ^= self.s2
        self.s0 ^= self.s3
        self.s2 ^= t
        self.s3 = _rotl(self.s3, 45)
        return result

    # --- conversions exactly as compiled ---

    def next_double(self) -> float:
        return (self.next_ulong() >> 11) * 1.1102230246251565e-16  # 2^-53

    def next_float(self) -> float:
        import struct
        f = (self.next_ulong() >> 40) * 5.960464477539063e-08  # 2^-24
        return struct.unpack("f", struct.pack("f", f))[0]

    def next_int(self) -> int:
        return self.next_ulong() >> 33  # top 31 bits, always non-negative

    def next_bool(self) -> bool:
        return bool(self.next_ulong() & 0x8000000000000000)

    def next(self, max_or_min: int, max_value: int | None = None) -> int:
        """Next(max) or Next(min, max); max exclusive. One draw either way."""
        if max_value is None:
            if max_or_min < 1:
                raise ValueError("maxValue must be > 0")
            return int(self.next_double() * max_or_min)
        lo, hi = max_or_min, max_value
        if lo >= hi:
            raise ValueError("maxValue must be > minValue")
        rng_range = hi - lo
        # the C# code branches on range > int.MaxValue but both branches
        # compute (int/long)(NextDouble() * range) + lo
        return int(self.next_double() * rng_range) + lo


# ---------------------------------------------------------------------------
# Rng — counter-tracking wrapper (one counter tick == one xoshiro draw)
# ---------------------------------------------------------------------------

class Rng:
    def __init__(self, seed: int, counter: int = 0, *,
                 build: str = GAME_BUILD_V0_108):
        self.seed = seed
        self.build = build
        self.counter = 0
        if seeding_scheme(build) == "v108":
            # signed int32 seed in-game; conv.u8 zero-extension
            self._random = MegaRandom(seed & MASK32)
        else:
            # v0.109 Rng(UInt64 seed) [RVA 0x61b81]: full 64-bit seed
            self._random = MegaRandom(seed & MASK64)
        self.fast_forward(counter)

    @classmethod
    def from_stream(cls, set_seed: int, stream_name: str,
                    counter: int = 0, *,
                    build: str = GAME_BUILD_V0_108) -> "Rng":
        """Rng(seed, string name): seed + hash(name). stream_name must
        already be snake_case (the sets pass SnakeCase(enum.ToString())).
        v0.108: int32-wrapped djb2 add; v0.109: uint64-wrapped XxHash64
        add [Rng::.ctor RVA 0x61be9]."""
        if seeding_scheme(build) == "v108":
            seed = _wrap_i32(set_seed + deterministic_hash_code(stream_name))
        else:
            seed = (set_seed
                    + deterministic_hash_code_v109(stream_name)) & MASK64
        return cls(seed, counter, build=build)

    @classmethod
    def for_encounter(cls, set_seed: int, total_floor: int, entry: str, *,
                      build: str = GAME_BUILD_V0_108) -> "Rng":
        """The PER-FIGHT Encounter stream, seeded from the run set seed, the
        floor and the encounter's `Id.Entry` — NOT one of the named run
        streams, and NOT persisted in the save, so it needs no cross-fight
        counter accounting (it always starts at counter 0).

        `EncounterModel::GenerateMonstersWithSlots`, v0.109.1 RVA 0x22bc0c
        IL_002d-0054:

            ldarg.1; callvirt IRunState::get_Rng
                     callvirt RunRngSet::get_Seed        // UInt64
            ldarg.1; callvirt IRunState::get_TotalFloor  // Int32
                     conv.i8
                     add                                 // unchecked, 64-bit
            ldarg.0; call     AbstractModel::get_Id
                     callvirt ModelId::get_Entry         // String
                     call     StringHelper::GetDeterministicHashCode  // UInt64
                     add                                 // unchecked, 64-bit
                     newobj   Rng::.ctor(UInt64)         // RVA 0x61b81

        so under v0.109 ALL THREE parts are 64-bit: the set seed is the
        XxHash64 run seed, the entry hash is XxHash64 (the surviving djb2
        is `GetDeterministicHashCodeOld`, which this site does not call),
        and `Rng(UInt64)` hands the sum to `MegaRandom` untruncated.

        v0.108 kept the same three addends under int32 wraparound with the
        djb2 hash. Both routes are pinned in
        test_encounter_stream_seeding.py; the v0.109 route is
        engine-verified (42 (seed, encounter, floor) cases).

        `entry` is the ModelId entry — the part AFTER the dot, e.g.
        "SLIMES_WEAK" for "ENCOUNTER.SLIMES_WEAK".
        """
        if seeding_scheme(build) == "v108":
            seed = _wrap_i32(set_seed + total_floor
                             + deterministic_hash_code(entry))
        else:
            seed = (set_seed + total_floor
                    + deterministic_hash_code_v109(entry)) & MASK64
        return cls(seed, build=build)

    def fast_forward(self, target: int) -> None:
        if target < self.counter:
            raise ValueError(
                f"Cannot fast-forward an Rng counter to a lower number "
                f"(current = {self.counter}, target = {target})")
        while self.counter < target:
            self.counter += 1
            self._random.next_int()

    # every method below: counter += 1, exactly one underlying draw

    def next_bool(self) -> bool:
        self.counter += 1
        return self._random.next(2) == 0

    def next_int(self, a: int, b: int | None = None) -> int:
        """next_int(max_exclusive) or next_int(min_inclusive, max_exclusive).
        The two-arg form throws in-game when min >= max."""
        if b is not None and a >= b:
            raise ValueError("Minimum must be lower than maximum.")
        self.counter += 1
        return self._random.next(a) if b is None else self._random.next(a, b)

    def next_unsigned_int(self, a: int, b: int | None = None) -> int:
        if b is None:
            a, b = 0, a
        if a > b:
            raise ValueError("Minimum must be lower than maximum.")
        self.counter += 1
        return a + int(self._random.next_double() * (b - a))

    def next_double(self, lo: float = 0.0, hi: float | None = None) -> float:
        if hi is None:
            if lo == 0.0:  # parameterless NextDouble()
                self.counter += 1
                return self._random.next_double()
            lo, hi = 0.0, lo
        self.counter += 1
        return self._random.next_double() * (hi - lo) + lo

    def next_float(self, lo: float = 0.0, hi: float | None = None) -> float:
        import struct
        if hi is None:
            lo, hi = 0.0, lo
        self.counter += 1
        f = self._random.next_double() * (hi - lo) + lo
        return struct.unpack("f", struct.pack("f", f))[0]

    def shuffle(self, items: list) -> None:
        """Fisher-Yates from the top: for i = n-1 .. 1, swap i with
        next_int(i+1). Consumes n-1 draws/counter ticks for n items."""
        for i in range(len(items) - 1, 0, -1):
            j = self.next_int(i + 1)
            items[j], items[i] = items[i], items[j]

    def next_item(self, items: list):
        """Rng.NextItem over a materialized list. Empty list returns None
        WITHOUT consuming a draw; otherwise one next_int(0, n) draw."""
        if not items:
            return None
        return items[self.next_int(0, len(items))]


def _wrap_i32(v: int) -> int:
    v &= MASK32
    return v - 0x100000000 if v >= 0x80000000 else v


# ---------------------------------------------------------------------------
# Stream sets
# ---------------------------------------------------------------------------

class RunRngSet:
    """One per run, shared across players. seed_string is the run seed
    exactly as recorded in the .run file (e.g. "ZPJHU3WSH2"). Pass the
    game build the run was recorded under (default: v0.108 scheme, which
    keeps every pre-#309 pin/capture valid)."""

    def __init__(self, seed_string: str,
                 counters: dict[str, int] | None = None, *,
                 build: str = GAME_BUILD_V0_108):
        self.string_seed = seed_string
        self.build = build
        if seeding_scheme(build) == "v108":
            self.seed = deterministic_hash_code(seed_string)
        elif seed_string.startswith("old"):
            # v0.109 legacy escape [RunRngSet::.ctor RVA 0x50f70]: seed
            # strings prefixed "old" hash the REMAINDER with the old djb2,
            # zero-extended to uint64 via CIL conv.u8. Stream-name hashes
            # below still use XxHash64.
            self.seed = deterministic_hash_code(
                seed_string[len("old"):]) & MASK32
        else:
            self.seed = deterministic_hash_code_v109(seed_string)
        self.rngs: dict[str, Rng] = {}
        for name in RUN_RNG_STREAMS:
            counter = (counters or {}).get(name, 0)
            self.rngs[name] = Rng.from_stream(self.seed, snake_case(name),
                                              counter, build=build)

    def __getitem__(self, stream: str) -> Rng:
        return self.rngs[stream]


class PlayerRngSet:
    """One per player: seed = hash(seed_string) + player_slot_index.
    v0.109 [Player::InitializeSeed RVA 0x2c2bda]: XxHash64 hash, uint64
    add — and NO "old"-prefix escape (that path is run-set-only)."""

    def __init__(self, seed_string: str, slot_index: int = 0,
                 counters: dict[str, int] | None = None, *,
                 build: str = GAME_BUILD_V0_108):
        self.build = build
        if seeding_scheme(build) == "v108":
            self.seed = _wrap_i32(deterministic_hash_code(seed_string)
                                  + slot_index)
        else:
            self.seed = (deterministic_hash_code_v109(seed_string)
                         + slot_index) & MASK64
        self.rngs: dict[str, Rng] = {}
        for name in PLAYER_RNG_STREAMS:
            counter = (counters or {}).get(name, 0)
            self.rngs[name] = Rng.from_stream(self.seed, snake_case(name),
                                              counter, build=build)

    def __getitem__(self, stream: str) -> Rng:
        return self.rngs[stream]


if __name__ == "__main__":
    import sys
    seed = sys.argv[1] if len(sys.argv) > 1 else "ZPJHU3WSH2"
    build = sys.argv[2] if len(sys.argv) > 2 else GAME_BUILD_V0_108
    print(f'seed string: "{seed}"  (build {build})')
    rs = RunRngSet(seed, build=build)
    print(f"run-set seed (hash): {rs.seed}")
    for name in RUN_RNG_STREAMS:
        rng = rs[name]
        first = Rng(rng.seed, build=build).next_int(100)
        print(f"  {snake_case(name):26s} seed={rng.seed:>20d}  "
              f"first next_int(100)={first}")
    ps = PlayerRngSet(seed, 0, build=build)
    for name in PLAYER_RNG_STREAMS:
        print(f"  player0/{snake_case(name):18s} seed={ps[name].seed:>20d}")

"""
Monster intent replay — Python port of STS2's MonsterMoveStateMachine
(issue #63). IL-verified against sts2.dll build v0.108.0.

How the game rolls intents:
  - `Creature::PrepareForNextTurn` (once per creature per turn) and the
    initial `AfterCreatureAdded` hook call `MonsterModel::RollMove`, which
    walks the monster's state machine until it lands on a MoveState.
  - State kinds and their MonsterAi-stream consumption:
      MoveState               fixed FollowUpState        0 draws
      ConditionalBranchState  first branch whose cond>0  0 draws
      RandomBranchState       weighted NextFloat pick    1 draw per node
  - On the very first roll, if the machine's initial state is already a
    MoveState it is used as-is (`_performedFirstMove` logic).

So a monster consumes MonsterAi draws only if its graph contains
RandomBranchState nodes. Exactly 21 monsters do (see STREAM_CONSUMERS.md
addendum in RNG_FINDINGS); everything else — including Toadpole, Seapunk,
and the Act 1 elites TerrorEel / PhantasmalGardeners / SkulkingColony —
is fully deterministic given fight state.

Graphs below are transcribed from each monster's GenerateMoveStateMachine.
"""

from __future__ import annotations

from dataclasses import dataclass, field


@dataclass
class MoveState:
    id: str
    intent: str                       # human-readable summary
    follow_up: str = ""               # next state id after this move is taken


@dataclass
class ConditionalBranchState:
    id: str
    branches: list = field(default_factory=list)   # [(cond(creature)->bool, state_id)]


@dataclass
class RandomBranchState:
    id: str
    branches: list = field(default_factory=list)   # [(weight(creature)->float, state_id)]


class MoveStateMachine:
    """Mirrors MonsterMoveStateMachine.RollMove/FindNextMoveState."""

    def __init__(self, states: list, initial_id: str):
        self.states = {s.id: s for s in states}
        self.current = self.states[initial_id]
        self.performed_first_move = False

    def roll_move(self, creature=None, rng=None) -> MoveState:
        if not self.performed_first_move and isinstance(self.current, MoveState):
            self.performed_first_move = True
            return self.current
        while True:
            s = self.current
            if isinstance(s, MoveState):
                nxt = s.follow_up
            elif isinstance(s, ConditionalBranchState):
                nxt = next(sid for cond, sid in s.branches if cond(creature))
            elif isinstance(s, RandomBranchState):
                weights = [(w(creature), sid) for w, sid in s.branches]
                roll = rng.next_float(sum(w for w, _ in weights))  # 1 draw
                for w, sid in weights:
                    roll -= w
                    if roll <= 0.0:
                        nxt = sid
                        break
                else:
                    raise RuntimeError(f"no branch hit in {s.id}")
            self.current = self.states[nxt]
            if isinstance(self.current, MoveState):
                self.performed_first_move = True
                return self.current


# --- Toadpole (TOADPOLES_WEAK = front + back toadpole) --------------------
# INIT is conditional on position: front toad -> SPIKEN, back toad -> WHIRL.
# Move rotation: SPIKE_SPIT -> WHIRL -> SPIKEN -> SPIKE_SPIT -> ...

def toadpole_machine(ascension: int = 10) -> MoveStateMachine:
    # IL constants: SpikeSpit = GetValueIfAscension(9, 4, 3) dmg x3 hits,
    # Whirl = GetValueIfAscension(9, 8, 7), Spiken grants 2 spikes.
    # Confirmed in-game at A10 (2026-07-09): "front toad has spikes2 and
    # is attacking 4x3 on turn 2."
    spit = 4 if ascension >= 9 else 3
    whirl = 8 if ascension >= 9 else 7
    return MoveStateMachine([
        MoveState("SPIKE_SPIT_MOVE", f"multi-attack {spit}x3 (SpikeSpit)",
                  "WHIRL_MOVE"),
        MoveState("WHIRL_MOVE", f"attack {whirl} (Whirl)", "SPIKEN_MOVE"),
        MoveState("SPIKEN_MOVE", "buff: spikes 2 (Spiken)",
                  "SPIKE_SPIT_MOVE"),
        ConditionalBranchState("INIT_MOVE", [
            (lambda c: not c["is_front"], "WHIRL_MOVE"),
            (lambda c: c["is_front"], "SPIKEN_MOVE"),
        ]),
    ], "INIT_MOVE")


# --- Seapunk (SEAPUNK_WEAK = 1 seapunk) ------------------------------------
# Initial state IS a move: SEA_KICK -> SPINNING_KICK -> BUBBLE_BURP -> loop.

def seapunk_machine() -> MoveStateMachine:
    return MoveStateMachine([
        MoveState("SEA_KICK_MOVE", "attack (SeaKick)", "SPINNING_KICK_MOVE"),
        MoveState("SPINNING_KICK_MOVE", "multi-attack (SpinningKick)",
                  "BUBBLE_BURP_MOVE"),
        MoveState("BUBBLE_BURP_MOVE", "buff+block (BubbleBurp)",
                  "SEA_KICK_MOVE"),
    ], "SEA_KICK_MOVE")


def predict_intents(machine_factory, creature, turns: int,
                    rng=None) -> list[str]:
    m = machine_factory()
    return [m.roll_move(creature, rng).intent for _ in range(turns)]


# --- TwoTailedRat (TWO_TAILED_RATS_NORMAL = 3 rats) -----------------------
# The one random-AI Act 1 monster on the sample run's path (issue #63).
# IL facts (build v0.108.0):
#   - moves: SCRATCH (attack 9 @A9+ else 8, CannotRepeat), DISEASE_BITE
#     (attack 7 @A9+ else 6 — an attack and NOTHING else, the intent's
#     debuff icon is cosmetic; d__39 read in the #120 E1 pass,
#     CannotRepeat), SCREECH (Frail 1 on the player,
#     CannotRepeat + cooldown 3), CALL_FOR_BACKUP (summon, UseOnlyOnce
#     per rat; encounter-wide cap CallForBackupCount < 3)
#   - every move's follow-up -> RAND (RandomBranchState): 1 MonsterAi draw
#     per traversal. Weight lambdas: CanSummon ? (1/12,1/12,1/12,0.75)
#     : (1,1,1,0)  [floats exactly as compiled: 0.0833333358168602]
#   - CanSummon = TurnsUntilSummonable<=0 AND CallForBackupCount<3 AND
#     a free encounter slot AND no living teammate already intends backup
#   - TurnsUntilSummonable: ctor=2, decremented by each Scratch/Bite/
#     Screech execution
#   - starters: GenerateMonsters rolls k = encounterRng.NextInt(3) ONCE;
#     rats get (k, k+1, k+2) % 3 of [SCRATCH, DISEASE_BITE, SCREECH].
#     Summoned rats keep ctor StarterMoveIndex=-1 -> first roll goes
#     through RAND (1 draw at spawn).
#   - GetStateWeight constraint order: UseOnlyOnce -> in log ever => 0;
#     CannotRepeat -> last log entry == state => 0; cooldown ->
#     state in last `cooldown` log entries => 0; then multiply lambda.

RAT_MOVES = ["SCRATCH", "DISEASE_BITE", "SCREECH"]
RAT_LABEL = {"SCRATCH": "attack 9 (Scratch)",
             "DISEASE_BITE": "attack 7 (DiseaseBite)",
             "SCREECH": "debuff (Screech)",
             "CALL_FOR_BACKUP": "summon (CallForBackup)"}
_W_CAN = {"SCRATCH": 0.0833333358168602, "DISEASE_BITE": 0.0833333358168602,
          "SCREECH": 0.0833333358168602, "CALL_FOR_BACKUP": 0.75}
_W_CANT = {"SCRATCH": 1.0, "DISEASE_BITE": 1.0, "SCREECH": 1.0,
           "CALL_FOR_BACKUP": 0.0}


class Rat:
    def __init__(self, starter_index: int):
        self.starter = RAT_MOVES[starter_index % 3] if starter_index >= 0 else None
        self.log: list[str] = []
        self.turns_until_summonable = 2
        self.summoned_count_used = False   # UseOnlyOnce via log anyway
        self.intent: str | None = None
        self.alive = True

    def weight(self, move: str, can_summon: bool) -> float:
        if move == "CALL_FOR_BACKUP" and move in self.log:
            return 0.0                                    # UseOnlyOnce
        if move != "CALL_FOR_BACKUP" and self.log and self.log[-1] == move:
            return 0.0                                    # CannotRepeat
        if move == "SCREECH" and "SCREECH" in self.log[-3:]:
            return 0.0                                    # cooldown 3
        return (_W_CAN if can_summon else _W_CANT)[move]

    def roll(self, rng, can_summon: bool) -> str:
        """One RAND traversal = one MonsterAi draw."""
        order = ["SCRATCH", "DISEASE_BITE", "SCREECH", "CALL_FOR_BACKUP"]
        ws = [(self.weight(m, can_summon), m) for m in order]
        total = sum(w for w, _ in ws)
        r = rng.next_float(total)
        for w, m in ws:
            r -= w
            if r <= 0.0:
                return m
        return ws[-1][1]


def simulate_rat_fight(rng, k: int, turns: int, starter_logged: bool = True,
                       free_slots: int = 1, backup_cap: int = 3):
    """All-rats-alive simulation of TWO_TAILED_RATS_NORMAL intent rolls.
    Returns {turn: [intent per rat, rats indexed by slot]}. Assumes the
    player kills nothing (record-intents protocol) and roll order = slot
    order. `starter_logged`: whether the deterministic starter move enters
    StateLog. RESOLVED in-game 2026-07-09 (ZPJHU3WSH2 fight 4, turns 1-4
    matched exactly): starter IS logged (True), rolls go in slot order
    left->right, and a summoned rat's spawn-time roll is its next intent
    (no re-roll) — see test_random_monster_ai_confirmed_in_game."""
    rats = [Rat((k + i) % 3) for i in range(3)]
    backup_count = 0
    out = {}
    for rat in rats:
        rat.intent = rat.starter
        if starter_logged:
            rat.log.append(rat.starter)
    out[1] = [RAT_LABEL[r.intent] for r in rats]
    for turn in range(2, turns + 1):
        # previous turn's moves execute. Log semantics: moves are logged
        # when ROLLED (FindNextMoveState); rolled intents from turn 2 on
        # are always logged, the deterministic starter only under SL=Y
        # (and it was logged at init in that case).
        for rat in rats:
            if turn > 2:
                rat.log.append(rat.intent)
            if rat.intent != "CALL_FOR_BACKUP":
                rat.turns_until_summonable -= 1
            else:
                backup_count += 1
        # roll new intents in slot order
        for rat in rats:
            teammate_intends = any(o is not rat and o.intent == "CALL_FOR_BACKUP"
                                   and o.alive for o in rats)
            can = (rat.turns_until_summonable <= 0 and backup_count < backup_cap
                   and free_slots > 0 and not teammate_intends)
            rat.intent = rat.roll(rng, can)
        out[turn] = [RAT_LABEL[r.intent] for r in rats]
    return out


if __name__ == "__main__":
    print("ZPJHU3WSH2 — fight 1 (TOADPOLES_WEAK), predicted intents "
          "(deterministic, no MonsterAi draws):")
    for label, front in (("front toad", True), ("back toad", False)):
        seq = predict_intents(lambda: toadpole_machine(10),
                              {"is_front": front}, 4)
        for t, intent in enumerate(seq, 1):
            print(f"  turn {t}: {label:10s} {intent}")
    print("\nfight 2 (SEAPUNK_WEAK), predicted intents:")
    for t, intent in enumerate(
            predict_intents(seapunk_machine, {}, 4), 1):
        print(f"  turn {t}: seapunk    {intent}")

    import sys, pathlib
    sys.path.insert(0, str(pathlib.Path(__file__).parent))
    from sts2_rng import Rng, RunRngSet
    print("\nfight 4 (TWO_TAILED_RATS_NORMAL, 3 rats) — MonsterAi counter 0.")
    print("k = starter offset (read off turn-1 intents); SL = starter-logged hypothesis")
    for k in range(3):
        for sl in (True, False):
            rng = Rng(RunRngSet("ZPJHU3WSH2")["MonsterAi"].seed)
            # turns 1-3 only: a turn-3 summon spawns a 4th rat whose
            # initial RAND roll consumes a draw with unverified ordering,
            # so turn-4 predictions await post-hoc validation.
            table = simulate_rat_fight(rng, k, 3, starter_logged=sl)
            print(f"\n  k={k} SL={'Y' if sl else 'N'} (draws used: {rng.counter})")
            for t, intents in table.items():
                print(f"    turn {t}: " + " | ".join(
                    i.split(' (')[0] for i in intents))

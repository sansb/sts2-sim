"""Tests for the harness experiment kit (#307).

LOCAL-ONLY. This module is skipped unless BOTH conditions hold:
  * `pythonnet` is importable, and
  * RUN_HARNESS=1 is set in the environment, and the local game install +
    the gitignored `dotnet/` runtime are present.

That triple gate guarantees nothing here runs in CI (no pythonnet, no game DLL,
RUN_HARNESS unset) and that a plain `python3 -m pytest solver/ -q` on a dev
machine also SKIPS it -- important because the CLR is one-per-process, so we
never want a bare test run to boot it out from under other harness work. Run it
deliberately with:  RUN_HARNESS=1 python3 -m pytest solver/harness/test_experiment.py -q
"""
import os
from pathlib import Path

import pytest

pytest.importorskip("pythonnet")

if not os.environ.get("RUN_HARNESS"):
    pytest.skip("set RUN_HARNESS=1 to run the local headless-engine harness tests",
                allow_module_level=True)

_GAME = (Path.home() / "Library/Application Support/Steam/steamapps/common/"
         "Slay the Spire 2/SlayTheSpire2.app/Contents/Resources/"
         "data_sts2_macos_arm64")
_DOTNET = Path(__file__).parent / "dotnet"
if not (_GAME / "sts2.dll").exists() or not _DOTNET.exists():
    pytest.skip("harness needs the local game install + the dotnet/ runtime",
                allow_module_level=True)

import experiment  # noqa: E402  (boots the CoreCLR loader on import)


# -- line validation (single-turn contract) -- these need no combat run -------
def test_line_rejects_action_after_end():
    with pytest.raises(experiment.LineError):
        experiment._validate_line([("play", 0, 1), ("end",), ("play", 1, None)])


def test_line_rejects_second_end():
    with pytest.raises(experiment.LineError):
        experiment._validate_line([("end",), ("end",)])


def test_line_allows_plays_then_single_end():
    # playing cards then ONE final ("end",) to observe end-turn/enemy phase is allowed
    experiment._validate_line([("play", 0, 1), ("play", 0, 1), ("end",)])


def test_line_rejects_unknown_kind():
    with pytest.raises(experiment.LineError):
        experiment._validate_line([("draw", 1)])


def test_build_version_is_reported():
    bv = experiment.build_version()
    assert bv["version"] and bv["commit"], bv


# -- one real end-to-end run against the engine -------------------------------
def test_two_strikes_deal_expected_damage():
    report = experiment.run_experiment(
        character="Ironclad", ascension=0,
        deck=["CARD.STRIKE_IRONCLAD"] * 8, relics=[], potions=[],
        encounter="ENCOUNTER.CULTISTS_NORMAL", seed="TESTSEED",
        line=[("play", 0, 1), ("play", 0, 1), ("end",)],
    )
    assert report["build"]["version"] == "v0.110.1", report["build"]
    # two 6-damage Strikes into the first enemy (CombatId 1). Keys are ints
    # in-memory (they only stringify once the report is JSON-serialized).
    assert report["net_creature_delta"][1]["hp"] == -12, report["net_creature_delta"]
    # the end-turn reshuffle advances the Shuffle stream (a real RNG delta)
    assert report["net_rng_delta"].get("Shuffle", 0) > 0, report["net_rng_delta"]
    # CombatHistory is populated and starts with the opening deal
    assert report["combat_history"], "empty combat history"
    assert any(e["type"] == "DamageReceivedEntry" for e in report["combat_history"])


def _run_experiment_subprocess(spec, tmp_path):
    """Run one experiment in a FRESH process via the CLI and return its report.

    The CLR is one-per-process and `ModelDb.Init` is one-shot, so the single
    in-process real-run slot is already taken by test_two_strikes above. To add
    another real-engine run we spawn a fresh process (the same one-CLR-per-run
    model the CLI uses), then strip the game's own `[GAME] ...` log lines that
    precede the JSON on stdout.
    """
    import json
    import subprocess
    import sys

    spec_path = tmp_path / "spec.json"
    spec_path.write_text(json.dumps(spec))
    here = Path(__file__).parent
    proc = subprocess.run(
        [sys.executable, str(here / "experiment.py"), str(spec_path)],
        capture_output=True, text=True, cwd=str(here),
    )
    assert proc.returncode == 0, (
        # a native attack-anim segfault (the #326 claim) would surface here as a
        # nonzero / signal exit -- this assert IS the crash guard.
        f"experiment.py exited {proc.returncode}\nstderr:\n{proc.stderr}")
    # v0.110.1 emits an unprefixed Sentry GDExtension diagnostic before the
    # report in addition to the historical ``[GAME]`` lines. The CLI's
    # structured payload is pretty-printed and begins on the first line that
    # is exactly ``{``; discard every native diagnostic before that boundary.
    lines = proc.stdout.splitlines()
    try:
        json_start = lines.index("{")
    except ValueError as exc:
        raise AssertionError(f"experiment.py emitted no JSON\n{proc.stdout}") from exc
    body = "\n".join(lines[json_start:])
    return json.loads(body)


def test_enemy_attack_turn_runs_and_damages_player(tmp_path):
    """Regression guard for #326: an ENEMY ATTACK move must run through the
    engine-driven end-turn -> enemy-phase pipeline without a native segfault
    and land real damage on the player.

    #326 was filed (from a batch-1 branch cut before the #308/#318 engine-driven
    experiment pipeline landed) claiming every enemy-attack line segfaults the
    bare-CoreCLR harness on an unshimmed attack-anim native. It does NOT
    reproduce on the engine-driven pipeline: the existing shim table already
    covers the attack path. The prior suite only exercised CULTISTS_NORMAL
    (a turn-1 BUFF), so no test walked the enemy-attack branch -- this closes
    that gap. Cite: (build v0.109.0 c12f634d, seed RATSEED, line [("end",)]).
    """
    report = _run_experiment_subprocess({
        "character": "Ironclad", "ascension": 0,
        "deck": ["CARD.STRIKE_IRONCLAD"], "relics": [], "potions": [],
        "encounter": "ENCOUNTER.TWO_TAILED_RATS_NORMAL", "seed": "RATSEED",
        "line": [["end"]],
    }, tmp_path)
    # the enemy phase ran its real AI (MonsterAi stream advanced once per rat)
    assert report["net_rng_delta"].get("MonsterAi", 0) > 0, report["net_rng_delta"]
    # the player took real attack damage from the rats (seed-pinned: 8 + 6 = 14)
    assert report["net_creature_delta"]["0"]["hp"] == -14, report["net_creature_delta"]
    # and the engine recorded the incoming enemy hits in CombatHistory
    enemy_hits = [e for e in report["combat_history"]
                  if e["type"] == "DamageReceivedEntry" and e.get("side") == "Enemy"]
    assert enemy_hits, report["combat_history"]


def test_relic_grant_fires_turn1_hooks_exactly_once(tmp_path):
    """Regression guard for #343 + #344. #343: granting an extra relic must
    use the incremental AddRelicInternal path (PopulateRelics throws
    'already populated' post-run-creation). #344: the kit used to call
    StartTurn explicitly on top of the one StartCombatInternal already fires
    (post-#308), double-running every turn-1 player-side-turn-start hook --
    Diamond Diadem showed block 40 and its whole fire sequence twice. Fixed
    kit: initial block exactly 20 and the fire sequence exactly ONCE.
    Cite: (build v0.109.0 c12f634d, seed TESTSEED, line []).
    """
    report = _run_experiment_subprocess({
        "character": "Ironclad", "ascension": 0,
        "deck": ["CARD.STRIKE_IRONCLAD"] * 8,
        "relics": ["RELIC.DIAMOND_DIADEM"], "potions": [],
        "encounter": "ENCOUNTER.CULTISTS_NORMAL", "seed": "TESTSEED",
        "line": [],
    }, tmp_path)
    # 343: the grant worked at all (Diadem's hook produced block); 344: once.
    assert report["initial"]["creatures"]["0"]["block"] == 20, report["initial"]
    blocks = [e for e in report["combat_history"] if e["type"] == "BlockGainedEntry"]
    blurs = [e for e in report["combat_history"]
             if e["type"] == "PowerReceivedEntry" and "applied 1 BLUR_POWER" in e["text"]]
    assert len(blocks) == 1 and len(blurs) == 1, report["combat_history"]
    # exactly one combat-start shuffle (8 cards -> 7 Fisher-Yates draws):
    # the single engine-driven opening StartTurn dealt exactly one hand
    assert report["initial"]["rng_counters"]["Shuffle"] == 7, report["initial"]


def test_multihit_aoe_attack_resolves(tmp_path):
    """Regression guard for #339: a multi-hit AoE AttackCommand (Whirlwind,
    X=3 at 3 energy -> 3 hits x 5 damage on every enemy) must actually land
    its damage headlessly. Root cause was NOT the await path: Whirlwind's
    OnPlay reads SaveManager.Instance.PrefsSave.FastMode for its VFX-delay
    branch, PrefsSave was null headless, and the NRE was logged-and-swallowed
    by TaskHelper.LogTaskExceptions -- play 'Finished', zero damage. host.boot
    now installs the game's own InitPrefsDataForTest seam; the kit also raises
    SwallowedPlayError on any future started-without-finished play.
    Cite: (build v0.109.0 c12f634d, seed TESTSEED, line [play WHIRLWIND]).
    """
    report = _run_experiment_subprocess({
        "character": "Ironclad", "ascension": 0,
        "deck": ["CARD.WHIRLWIND"] + ["CARD.STRIKE_IRONCLAD"] * 4,
        "relics": [], "potions": [],
        "encounter": "ENCOUNTER.CULTISTS_NORMAL", "seed": "TESTSEED",
        "line": [["play", 1, None]],  # WHIRLWIND at hand idx 1 on this seed
    }, tmp_path)
    assert report["net_creature_delta"]["1"]["hp"] == -15, report["net_creature_delta"]
    assert report["net_creature_delta"]["2"]["hp"] == -15, report["net_creature_delta"]
    types = [e["type"] for e in report["combat_history"]]
    assert types.count("CardPlayStartedEntry") == types.count("CardPlayFinishedEntry") == 1, types
    # 3 hits on each of the 2 cultists, all recorded per-hit
    assert types.count("DamageReceivedEntry") == 6, types


def test_enemy_attack_into_block_runs_unblocked_remainder(tmp_path):
    """Regression guard for #326 (AfterDamageReceived surface): an incoming
    enemy hit with player block up must run the block-then-unblocked-remainder
    path without a segfault -- this is the path Reflect/The Gambit/FlameBarrier
    ordering work needs. Two Defends (5 block each) absorb 10 of the rats'
    14 damage; 4 lands. Cite: (build v0.109.0 c12f634d, seed RATSEED,
    line [play Defend, play Defend, end]).
    """
    report = _run_experiment_subprocess({
        "character": "Ironclad", "ascension": 0,
        "deck": ["CARD.DEFEND_IRONCLAD"] * 5, "relics": [], "potions": [],
        "encounter": "ENCOUNTER.TWO_TAILED_RATS_NORMAL", "seed": "RATSEED",
        "line": [["play", 0, None], ["play", 0, None], ["end"]],
    }, tmp_path)
    # 10 block absorbed, 4 unblocked -> net player hp -4 (block cleared to 0)
    assert report["net_creature_delta"]["0"]["hp"] == -4, report["net_creature_delta"]
    assert report["net_creature_delta"]["0"]["block"] == 0, report["net_creature_delta"]

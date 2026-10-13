#!/usr/bin/env python3
"""Native probe: a player choice that opens inside a relic's AfterCardPlayed
Draw (#3822).

The chain: the player plays a card; Game Piece (a Power play) or Iron Club
(every fourth card) draws inside `Hook.AfterCardPlayed`; a live Hellraiser
AutoPlays the drawn Strike; Seeker Strike or Sculpting Strike asks for a card.
The twin is a Stratagem reshuffle pick inside the same Draw. No captured fight
holds either, so these traces are the ground truth the Rust crate's tests
replay (`engine/fixtures/issue3822_native/`, read by
`engine/src/engine/puzzle/tests/after_card_played.rs`).

Every play goes through the game's own `PlayCardAction`. The choice is
answered through the `TestCardSelector` the game ships, pushed as the LOCAL
selector: `CardSelectCmd` still signals the play's choice context
(`<FromCombatPile>d__20` RVA `0x3e5e84` IL_0077-00ae), the action really
pauses in `GatheringPlayerChoice`, and only then is the local selector asked
(IL_0276-0303). `SetupForAsyncIndexSelection` hands back a pending
`TaskCompletionSource`, so the paused state is observed before the answer is
given.

Usage (local only; needs the harness venv and an archived game build, see
README.md):

    export STS2_GAME_DIR=<repo>/sim/dll-archive/v0.111.0/data_sts2_macos_arm64
    export STS2_HARNESS_DIR=<a harness directory with dotnet/ set up>  # optional
    venv/bin/python probe_relic_hook_choice.py list
    venv/bin/python probe_relic_hook_choice.py run seeker_basic 0
    venv/bin/python probe_relic_hook_choice.py drive --out DIR [NAME ...]
    STS2_WAITS=real venv/bin/python probe_relic_hook_choice.py drive --out DIR [NAME ...]

A script step `["play", "@END", None]` ends the turn through the game's own
`EndPlayerTurnAction` and runs the enemy turn and the next turn start;
`["play", "@POTION", slot]` uses a belt potion through `UsePotionAction`.

`run NAME ANSWERS` plays one scenario with a comma list of option indices
(one per choice, `-` for none) and prints `NAT <json>`. `drive` enumerates
every answer sequence the game accepts, one CLR process each, and writes one
merged trace per scenario.

`STS2_WAITS=real[:normal|fast|instant][:always]` runs the scenario with
`Cmd.Wait` as a timer from its first `snap` on (or from a `["real_waits"]`
step), the way a game someone is playing runs it; `drive` then writes
`NAME.waits.json`, with the mode and the concurrency report in it
(`real_waits.py`, README "Real waits", #3931). Unset, nothing changes: the
committed traces were recorded that way and still reproduce.

Each snapshot holds the player's HP, Block, Energy and powers, each living
monster, every pile in order, the Hand's resolved costs, the nine combat RNG
streams (counter and the four xoshiro words), and the CardPlay counts.
"""
import functools
import json
import os
import sys
import time
from pathlib import Path

print = functools.partial(print, flush=True)
HERE = Path(__file__).resolve().parent
DLL_SHA256 = "9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4"
D = "DEFEND_IRONCLAD"
S = "STRIKE_IRONCLAD"

# native RunRngSet stream -> the engine's RngStream wire key
STREAMS = {
    "Shuffle": "rng", "Niche": "niche", "MonsterAi": "ai",
    "CombatCardSelection": "sel", "CombatCardGeneration": "generation",
    "CombatTargets": "targets", "CombatEnergyCosts": "energy_costs",
    "CombatOrbGeneration": "combat_orbs",
    "CombatPotionGeneration": "potion_generation",
}


def _spec(relics, hand, draw, plays, discard=(), setup=(), energy=9, potions=(),
          encounter=None, seed=None):
    script = [["energy", energy],
              ["arrange", {"hand": list(hand), "draw": list(draw), "discard": list(discard)}]]
    script += [list(step) for step in setup]
    script.append(["snap", "root"])
    for play in plays:
        script.append(["play", play[0], play[1] if len(play) > 1 else None])
    script.append(["snap", "end"])
    spec = dict(character="Ironclad", relics=list(relics),
                deck=list(hand) + list(draw) + list(discard), script=script)
    if potions:
        # Only a potion scenario carries the key, so the others re-record
        # byte-identically.
        spec["potions"] = list(potions)
    if encounter:
        # Likewise: the default is ENCOUNTER.TOADPOLES_WEAK on seed W3822.
        spec["encounter"] = encounter
    if seed:
        spec["seed"] = seed
    return spec


def scenarios():
    """Hellraiser is itself a Power: its own play draws through Game Piece
    with the power already applied."""
    out = {}
    tail = ["BASH", "SHRUG_IT_OFF", "TRUE_GRIT", D]
    out["seeker_basic"] = _spec(
        ["GAME_PIECE"], ["HELLRAISER", "INFLAME"], [D, "SEEKER_STRIKE"] + tail,
        [["HELLRAISER"], ["INFLAME"]])
    out["seeker_plus"] = _spec(
        ["GAME_PIECE"], ["HELLRAISER", "INFLAME+"], [D, "SEEKER_STRIKE+"] + tail,
        [["HELLRAISER"], ["INFLAME+"]])
    out["sculpting_basic"] = _spec(
        ["GAME_PIECE"], ["HELLRAISER", "INFLAME", "BASH", "SHRUG_IT_OFF"],
        [D, "SCULPTING_STRIKE", "TRUE_GRIT", D], [["HELLRAISER"], ["INFLAME"]])
    out["sculpting_plus"] = _spec(
        ["GAME_PIECE"], ["HELLRAISER", "BASH", "SHRUG_IT_OFF"],
        ["SCULPTING_STRIKE+", "TRUE_GRIT", D], [["HELLRAISER"]])
    # One Hand card at the choice, and none.
    out["sculpting_one_option"] = _spec(
        ["GAME_PIECE"], ["HELLRAISER", "INFLAME"],
        ["SCULPTING_STRIKE", "TRUE_GRIT", D], [["HELLRAISER"]])
    out["sculpting_empty_hand"] = _spec(
        ["GAME_PIECE"], ["HELLRAISER"],
        ["SCULPTING_STRIKE", "TRUE_GRIT", D], [["HELLRAISER"]])
    # Nine Hand cards at the choice: the pick fills the Hand to its cap. (A
    # full Hand at the choice cannot be reached from a Hand play: the played
    # card and the drawn Strike have both left it.)
    out["seeker_hand_nine"] = _spec(
        ["GAME_PIECE"], ["HELLRAISER", D, D, D, D, S, S, S, S, "BASH"],
        ["SEEKER_STRIKE", "SHRUG_IT_OFF", "TRUE_GRIT", "INFLAME", D], [["HELLRAISER"]])
    # The Draw pile once the Strike has left it: one card, and none.
    out["seeker_one_in_draw"] = _spec(
        ["GAME_PIECE"], ["HELLRAISER", D], ["SEEKER_STRIKE", "TRUE_GRIT"], [["HELLRAISER"]])
    out["seeker_empty_draw"] = _spec(
        ["GAME_PIECE"], ["HELLRAISER", D], ["SEEKER_STRIKE"], [["HELLRAISER"]])
    # Another AfterCardPlayed listener before and after Game Piece. Rainbow
    # Ring completes its three types on the Power itself.
    for name, relics in (("ring_before", ["RAINBOW_RING", "GAME_PIECE"]),
                         ("ring_after", ["GAME_PIECE", "RAINBOW_RING"])):
        out[name] = _spec(relics, [S, D, "HELLRAISER"], ["SEEKER_STRIKE"] + tail,
                          [[S, 1], [D], ["HELLRAISER"]])
    for name, relics in (("mummified_before", ["MUMMIFIED_HAND", "GAME_PIECE"]),
                         ("mummified_after", ["GAME_PIECE", "MUMMIFIED_HAND"])):
        out[name] = _spec(relics, ["HELLRAISER", "BASH", "TRUE_GRIT"],
                          ["SEEKER_STRIKE", "UPPERCUT", "SHRUG_IT_OFF", "BLUDGEON", D],
                          [["HELLRAISER"]])
    # The AutoPlayed Strike kills the last enemy, or one of two.
    out["kill_last_enemy"] = _spec(
        ["GAME_PIECE"], [S, "HELLRAISER", D], ["SEEKER_STRIKE"] + tail,
        [[S, 1], ["HELLRAISER"]], setup=[["hp", 1, 3], ["hp", 2, 5]])
    out["kill_one_of_two"] = _spec(
        ["GAME_PIECE"], ["HELLRAISER", D], ["SEEKER_STRIKE"] + tail,
        [["HELLRAISER"]], setup=[["hp", 1, 5], ["hp", 2, 5]])
    # A second Power after the resume, which parks again when its draw is a
    # selecting Strike.
    out["second_power"] = _spec(
        ["GAME_PIECE"], ["HELLRAISER", "INFLAME", "INFLAME"],
        [D, "SEEKER_STRIKE", "SCULPTING_STRIKE", "BASH", "SHRUG_IT_OFF", "TRUE_GRIT"],
        [["HELLRAISER"], ["INFLAME"], ["INFLAME"]])
    # Iron Club: the fourth card of the fight is a Skill.
    out["iron_club"] = _spec(
        ["IRON_CLUB"], ["HELLRAISER", D, D, D],
        ["SEEKER_STRIKE", "BASH", "SHRUG_IT_OFF", "TRUE_GRIT", "INFLAME"],
        [["HELLRAISER"], [D], [D], [D]])
    # Both relics on one Power play, in each inventory order.
    both = ["TRUE_GRIT", "SEEKER_STRIKE", "SCULPTING_STRIKE", "BASH", "SHRUG_IT_OFF", "UPPERCUT"]
    out["piece_then_club"] = _spec(
        ["GAME_PIECE", "IRON_CLUB"], ["HELLRAISER", D, "INFLAME"], both,
        [["HELLRAISER"], [D], ["INFLAME"]])
    out["club_then_piece"] = _spec(
        ["IRON_CLUB", "GAME_PIECE"], ["HELLRAISER", D, "INFLAME"], both,
        [["HELLRAISER"], [D], ["INFLAME"]])
    # The Stratagem twin: the relic's draw reshuffles under a live Stratagem.
    pile = ["BASH", "SHRUG_IT_OFF", "TRUE_GRIT", "UPPERCUT", S]
    out["stratagem"] = _spec(
        ["GAME_PIECE"], ["STRATAGEM", "INFLAME"], [D],
        [["STRATAGEM"], ["INFLAME"]], discard=pile)
    out["stratagem_iron_club"] = _spec(
        ["IRON_CLUB"], ["STRATAGEM", D, D, D], [],
        [["STRATAGEM"], [D], [D], [D]], discard=pile)
    # Baseline: a Strike with no choice keeps the synchronous command.
    out["plain_strike"] = _spec(
        ["GAME_PIECE"], ["HELLRAISER", "INFLAME"], [D, S, "BASH", "SHRUG_IT_OFF"],
        [["HELLRAISER"], ["INFLAME"]])
    # ---- the independent review's scenarios (PR #3882) ----
    # R1: an ordinary (CardPlay-owned Draw) park first, then Iron Club's
    # receipt-owned park reached through that answer's continuation.
    out["r_trance_mixed"] = _spec(
        ["IRON_CLUB"], ["HELLRAISER", "BATTLE_TRANCE", D, "BASH"],
        [S, S, "SEEKER_STRIKE", "SCULPTING_STRIKE", "SHRUG_IT_OFF", "TRUE_GRIT", D, "UPPERCUT"],
        [["HELLRAISER"], ["BATTLE_TRANCE"]])
    # R2: Iron Club's Draw nested in a plain Strike that Hellraiser AutoPlays
    # out of Pommel Strike's own (card-program) Draw.
    out["r_pommel_club"] = _spec(
        ["IRON_CLUB"], ["HELLRAISER", D, D, "POMMEL_STRIKE", "BASH"],
        [S, "SEEKER_STRIKE", "SHRUG_IT_OFF", "TRUE_GRIT", D, "UPPERCUT"],
        [["HELLRAISER"], [D], [D], ["POMMEL_STRIKE", 1]])
    # R3: the outer Power is AutoPlayed by Havoc (the author's IL-only case).
    out["r_havoc_outer"] = _spec(
        ["GAME_PIECE"], ["HELLRAISER", "HAVOC", D],
        [D, "INFLAME", "SEEKER_STRIKE", "BASH", "SHRUG_IT_OFF", "TRUE_GRIT", D],
        [["HELLRAISER"], ["HAVOC"]])
    # R4: both relics and a third listener that counts attacks (Kunai would
    # need three; Ornamental Fan too) -- use Letter Opener? no: keep to a
    # Stratagem + Hellraiser double park inside ONE Game Piece Draw.
    out["r_stratagem_hellraiser"] = _spec(
        ["GAME_PIECE"], ["STRATAGEM", "HELLRAISER", "INFLAME", "BASH", D],
        [D, D],
        [["STRATAGEM"], ["HELLRAISER"], ["INFLAME"]],
        discard=["SEEKER_STRIKE", "SCULPTING_STRIKE", S, "SHRUG_IT_OFF", "TRUE_GRIT", "UPPERCUT"])
    # R5: Burst replays the third card; the replay is the fourth.
    out["r_burst_replay"] = _spec(
        ["IRON_CLUB"], ["HELLRAISER", "BURST", D, "BASH"],
        ["SEEKER_STRIKE", "SHRUG_IT_OFF", "TRUE_GRIT", D, "UPPERCUT"],
        [["HELLRAISER"], ["BURST"], [D]])
    # R6: the turn-start hand draw. The first drawn Strike is AutoPlayed and
    # is the fourth card; Iron Club's Draw is Seeker Strike.
    out["r_turnstart_club"] = _spec(
        ["IRON_CLUB"], ["HELLRAISER", D, D],
        [S, "SEEKER_STRIKE", "BASH", "SHRUG_IT_OFF", "TRUE_GRIT", D, "UPPERCUT", "INFLAME", D, D],
        [["HELLRAISER"], [D], [D], ["@END"]])
    # R8: Unceasing Top with the Hand empty while the relic's Draw nests a
    # Hellraiser AutoPlay. Where does the Top draw?
    out["r_top_nested_choice"] = _spec(
        ["GAME_PIECE", "UNCEASING_TOP"], ["HELLRAISER", "INFLAME"],
        [D, S, "SEEKER_STRIKE", "BASH", "SHRUG_IT_OFF", "TRUE_GRIT", D],
        [["HELLRAISER"], [D], ["INFLAME"]])
    out["r_top_mummified"] = _spec(
        ["GAME_PIECE", "MUMMIFIED_HAND", "UNCEASING_TOP"], ["HELLRAISER", "INFLAME"],
        [D, S, "BASH", "SHRUG_IT_OFF", "TRUE_GRIT", D],
        [["HELLRAISER"], [D], ["INFLAME"]])
    # R7: control for R6 without Iron Club: Seeker Strike drawn by the hand
    # draw itself (the pre-existing turn-start park).
    out["r_turnstart_plain"] = _spec(
        [], ["HELLRAISER", D, D],
        ["SEEKER_STRIKE", "BASH", "SHRUG_IT_OFF", "TRUE_GRIT", D, "UPPERCUT", "INFLAME", D, D],
        [["HELLRAISER"], [D], [D], ["@END"]])
    # ---- Unceasing Top and the card-or-potion effect depth (#3822) ----
    # `CombatManager.CheckForEmptyHand` returns while any card or potion
    # effect is executing, so the Top draws only once the outermost wrapper
    # has ended its effect and routed its card.
    top = ["UNCEASING_TOP"]
    # Control: a plain manual play of the last Hand card.
    out["top_plain"] = _spec(top, [D], ["BASH", "SHRUG_IT_OFF", "TRUE_GRIT", D], [[D]])
    # The Power is the last Hand card and Game Piece's Draw is a Strike that
    # Hellraiser AutoPlays inside the Power's walk. The Top's first draw is a
    # second Strike, AutoPlayed with no effect executing, so its own check
    # draws again.
    out["top_after_power"] = _spec(
        ["GAME_PIECE"] + top, ["HELLRAISER", "INFLAME"],
        [D, S, S, "BASH", "SHRUG_IT_OFF", "TRUE_GRIT", D],
        [["HELLRAISER"], [D], ["INFLAME"]])
    # Iron Club in Game Piece's place: the fourth card is the last Hand card.
    out["top_iron_club"] = _spec(
        ["IRON_CLUB"] + top, ["HELLRAISER", D, D, D],
        [S, "SEEKER_STRIKE", "BASH", "SHRUG_IT_OFF", "TRUE_GRIT", "UPPERCUT"],
        [["HELLRAISER"], [D], [D], [D]])
    # Havoc is the outer effect: the Strike it AutoPlays does not draw
    # through the Top; Havoc's own check does, after Havoc is routed.
    out["top_havoc"] = _spec(
        top, ["HELLRAISER", "HAVOC"], [S, S, "BASH", "SHRUG_IT_OFF", "TRUE_GRIT", D],
        [["HELLRAISER"], ["HAVOC"]])
    # A Burst replay: both bodies of Shrug It Off run inside one effect, and
    # each draws a Strike that Hellraiser AutoPlays.
    out["top_burst"] = _spec(
        top, ["HELLRAISER", "BURST", "SHRUG_IT_OFF"], [S, S, "BASH", D, "TRUE_GRIT", "UPPERCUT"],
        [["HELLRAISER"], ["BURST"], ["SHRUG_IT_OFF"]])
    # Hellraiser itself is the last Hand card: with no effect executing, the
    # Top's draw is a Strike whose AutoPlay empties the Hand again, so every
    # Strike's own check draws the next card. The second scenario ends that
    # chain on Seeker Strike's choice. (Swift Potion then draws normally.)
    out["top_chain"] = _spec(
        top, ["HELLRAISER"], [S, S, S, "BASH", "SHRUG_IT_OFF", "TRUE_GRIT", D],
        [["HELLRAISER"], ["@POTION", 0]], potions=["SWIFT_POTION"])
    out["top_chain_choice"] = _spec(
        top, ["HELLRAISER"], [S, S, S, "SEEKER_STRIKE", "BASH", "SHRUG_IT_OFF", "TRUE_GRIT", D],
        [["HELLRAISER"], ["@POTION", 0]], potions=["SWIFT_POTION"])
    # A potion is the outer effect. Distilled Chaos plays the top three Draw
    # cards: Fiend Fire exhausts the Hand, and neither it nor the two cards
    # after it draw through the Top; the potion's own check, after its effect
    # and `AfterPotionUsed`, does.
    out["top_potion"] = _spec(
        top, [D], ["FIEND_FIRE", S, D, "BASH", "SHRUG_IT_OFF", "TRUE_GRIT"],
        [["@POTION", 0]], potions=["DISTILLED_CHAOS"])
    # A potion wrapper nested in a card's effect: Bloodletting's HP loss is
    # lethal, Fairy in a Bottle is used from inside the card's body, and the
    # potion's own check finds the card's effect still executing. The Top
    # draws only after Bloodletting has reached the Discard pile.
    out["top_fairy"] = _spec(
        top, ["HELLRAISER", "BLOODLETTING"], [S, S, "BASH", "SHRUG_IT_OFF", "TRUE_GRIT", D],
        [["HELLRAISER"], ["BLOODLETTING"]], potions=["FAIRY_IN_A_BOTTLE"],
        setup=[["hp", 0, 2]])
    # Glowwater exhausts the Hand and draws ten: the two Strikes Hellraiser
    # AutoPlays out of that Draw leave the Hand empty inside the potion's
    # effect, and the Top draws nothing extra.
    out["top_potion_draw"] = _spec(
        top, ["HELLRAISER", D],
        [S, S, "BASH", "SHRUG_IT_OFF", "TRUE_GRIT", D, "UPPERCUT", "INFLAME", D, D, D, D],
        [["HELLRAISER"], ["@POTION", 0]], potions=["GLOWWATER_POTION"])
    # ---- Unceasing Top draws the card whose own play has just routed (#3756) ----
    # `OnPlayWrapper` awaits its own `CheckForEmptyHand` after routing the
    # card, so the Top can draw that card and a live Hellraiser AutoPlays it
    # inside the first wrapper's await.
    # Rebound puts the Strike on top of the Draw pile: one reentry, then the
    # Top's second draw is a Defend.
    out["top_rebound"] = _spec(
        top, ["HELLRAISER", "REBOUND", S], [D, D],
        [["HELLRAISER"], ["REBOUND", 1], [S, 1]])
    # Control: the Strike goes to Discard and the Top draws the Defend.
    out["top_strike_then_defend"] = _spec(
        top, ["HELLRAISER", S], [D], [["HELLRAISER"], [S, 1]])
    # A lone Strike: discarded, reshuffled, drawn, AutoPlayed, until the
    # combat ends. One manual play kills both enemies.
    lethal = [["hp", 1, 13], ["hp", 2, 20]]
    out["top_lone_strike"] = _spec(
        top, ["HELLRAISER", S], [], [["HELLRAISER"], [S, 1]], setup=lethal)
    # Two Strikes: every reshuffle orders two cards.
    out["top_two_strikes"] = _spec(
        top, ["HELLRAISER", S], [], [["HELLRAISER"], [S, 1]], discard=[S], setup=lethal)
    # Three, one of them Twin Strike.
    out["top_three_strikes"] = _spec(
        top, ["HELLRAISER", S], ["TWIN_STRIKE"], [["HELLRAISER"], [S, 1]], discard=[S],
        setup=[["hp", 1, 40], ["hp", 2, 40]])
    # The reentry, then the Top's next draw is Seeker Strike: the action
    # pauses with both of the Strike's wrappers still open.
    out["top_rebound_seeker"] = _spec(
        top, ["HELLRAISER", "REBOUND", S], ["SEEKER_STRIKE", D, "BASH", "TRUE_GRIT"],
        [["HELLRAISER"], ["REBOUND", 1], [S, 1]])
    # The loop with a choice in it: a Strike and Seeker Strike alone. Seeker
    # Strike finds nothing to offer while the Strike is in Play or Discard.
    out["top_loop_seeker"] = _spec(
        top, ["HELLRAISER", S], ["SEEKER_STRIKE"], [["HELLRAISER"], [S, 1]], setup=lethal)
    # Nostalgia returns the turn's first Attack to the top of the Draw pile.
    out["top_nostalgia"] = _spec(
        top, ["HELLRAISER", "NOSTALGIA", S], [D, D],
        [["HELLRAISER"], ["NOSTALGIA"], [S, 1]])
    # ---- Game Piece at its recorded place among the AfterCardPlayed relics (#3756) ----
    hell = [["power", "HellraiserPower", 1]]
    rest2 = ["BASH", "SHRUG_IT_OFF", "TRUE_GRIT", D, "UPPERCUT"]
    # Velvet Choker counts a play in its own AfterCardPlayed. The sixth card
    # is a Power; Game Piece draws a Strike and Hellraiser asks ShouldPlay.
    for name, relics in (("choker_after_piece", ["GAME_PIECE", "VELVET_CHOKER"]),
                         ("choker_before_piece", ["VELVET_CHOKER", "GAME_PIECE"])):
        out[name] = _spec(
            relics, [D, D, D, D, D, "INFLAME"], [S, S] + rest2,
            [[D], [D], [D], [D], [D], ["INFLAME"]], setup=hell, energy=20)
        # The fifth card is the Power: the count is short of six either way.
        out[name + "_fifth"] = _spec(
            relics, [D, D, D, D, "INFLAME", D], [S, S] + rest2,
            [[D], [D], [D], [D], ["INFLAME"]], setup=hell, energy=20)
    # Permafrost's Block under Juggernaut rolls a target; so does the Strike
    # Hellraiser plays out of Game Piece's draw. Strength tells the two hits
    # apart, and the weak Toadpole dies to the Strike alone.
    for name, relics in (("jug_piece_perma", ["GAME_PIECE", "PERMAFROST"]),
                         ("jug_perma_piece", ["PERMAFROST", "GAME_PIECE"])):
        out[name] = _spec(
            relics, ["JUGGERNAUT", D], [S, S] + rest2, [["JUGGERNAUT"]],
            setup=hell + [["power", "StrengthPower", 3], ["hp", 2, 8]], energy=20)
        out[name + "_plain"] = _spec(
            relics, ["JUGGERNAUT", D], [D, S] + rest2,
            [["JUGGERNAUT"]], energy=20)
    # ---- the loop against finite enemies, from the independent review of #3921 ----
    # Fresh seed, Hellraiser applied as a power, both Toadpoles at 150 HP
    # unless a kill is the point. Each relic counts every AutoPlay of the
    # loop at its own place; deaths, Seeker Strike parks with several
    # wrappers open, Havoc, Distilled Chaos and Swift Potion start it.
    hell = [["power", "HellraiserPower", 1]]
    big = [["hp", 1, 150], ["hp", 2, 150]]
    low = [["hp", 1, 13], ["hp", 2, 20]]

    def rv(name, relics, hand, draw, plays, discard=(), setup=(), seed="RV3921A", **kw):
        out[name] = _spec(relics, hand, draw, plays, discard=discard, setup=setup, seed=seed, **kw)
    one = [[S, 1]]
    rv("rv_base", top, [S], [S, S, S], one, [D], hell + big)
    rv("rv_kunai", top + ["KUNAI"], [S], [S] * 5, one, [D], hell + big)
    rv("rv_kunai_first", ["KUNAI"] + top, [S], [S] * 5, one, [D], hell + big)
    rv("rv_shuriken", top + ["SHURIKEN"], [S], [S] * 5, one, [D], hell + big)
    rv("rv_fan", ["ORNAMENTAL_FAN"] + top, [S], [S] * 5, one, [D], hell + big)
    rv("rv_nunchaku", top + ["NUNCHAKU"], [S], [S] * 10, one, [D], hell + big)
    rv("rv_pennib", ["PEN_NIB"] + top, [S], [S] * 10, one, [D], hell + big)
    rv("rv_pocketwatch", top + ["POCKETWATCH"], [S], [S, S], one + [["@END"]], [D], hell + big)
    rv("rv_all2", ["KUNAI", "SHURIKEN", "ORNAMENTAL_FAN"] + top + ["NUNCHAKU", "PEN_NIB", "LETTER_OPENER"],
       [S], [S] * 11, one, [D], hell + big)
    rv("rv_kill_one", top, [S], [S] * 4, one, [D], hell + [["hp", 1, 8], ["hp", 2, 150]])
    rv("rv_kill_one_b", top, [S], [S] * 4, one, [D], hell + [["hp", 1, 150], ["hp", 2, 8]])
    rv("rv_seeker_deep", top, [S], [], one, [S, S, "SEEKER_STRIKE"], hell + big)
    rv("rv_seeker_deep4", top, [S], [], one, [S, S, S, "SEEKER_STRIKE"], hell + big, seed="RV3921B")
    rv("rv_havoc", top, ["HAVOC"], [S], [["HAVOC"]], [D, S], hell + big)
    rv("rv_havoc_lone", top, ["HAVOC"], [S], [["HAVOC"]], [], hell + low)
    rv("rv_dchaos", top, [], [S, S, S], [["@POTION", 0]], [D], hell + big, potions=["DISTILLED_CHAOS"])
    rv("rv_dchaos_lone", top, [], [S], [["@POTION", 0]], [], hell + low, potions=["DISTILLED_CHAOS"])
    rv("rv_swift", top, [], [S, S], [["@POTION", 0]], [D], hell + big, potions=["SWIFT_POTION"])
    rv("rv_rebound_loop", top, ["REBOUND", S], [], [["REBOUND", 1], [S, 1]], [], hell + big)
    rv("rv_nostalgia_loop", top, ["NOSTALGIA", S], [], [["NOSTALGIA"], [S, 1]], [], hell + low)
    rv("rv_club", ["IRON_CLUB"] + top, [S], [S] * 6, one, [D], hell + big)
    rv("rv_club_last", top + ["IRON_CLUB"], [S], [S] * 6, one, [D, S], hell + big, seed="RV3921B")
    rv("rv_club_seeker", ["IRON_CLUB"] + top, [S], [S, S, S, "SEEKER_STRIKE", S, S], one, [D], hell + big)
    # Velvet Choker ends the loop at the turn's sixth card, before and after
    # the Top in the inventory (from the independent review of #3926).
    rv("rv_top_vc", top + ["VELVET_CHOKER"], [S], [S] * 8, one, [D], hell, seed="RV3926A")
    rv("rv_vc_top", ["VELVET_CHOKER"] + top, [S], [S] * 8, one, [D], hell, seed="RV3926A")
    rv("rv_vc_top_lone", ["VELVET_CHOKER"] + top, [S], [], one, [], hell, seed="RV3926A")
    # ---- Hellraiser's nine AutoPlays a turn against an infinite HP display (#3929) ----
    # `HellraiserPower/<AfterCardDrawnEarly>d__7` RVA `0x33c1a8` counts its
    # AutoPlays while every hittable enemy's `HpDisplay` is infinite and
    # stops after nine (IL_0063-00a6). Skulking Colony shows one once
    # Hardened Shell has taken its 20 for the turn; a Waterfall Giant about
    # to blow shows one for good; no hittable enemy at all counts too.
    cap = [["power", "HellraiserPower", 1]]
    colony = dict(encounter="ENCOUNTER.SKULKING_COLONY_ELITE", seed="RV3921A")
    giant = dict(encounter="ENCOUNTER.WATERFALL_GIANT_BOSS", seed="RV3921A")
    subject = dict(encounter="ENCOUNTER.TEST_SUBJECT_BOSS", seed="RV3921A")
    # The Giant dies once into its about-to-blow form when it holds Steam
    # Eruption (`SteamEruptionPower/<AfterDeath>d__4` RVA `0x34618c`).
    blow = cap + [["mpower", 1, "SteamEruptionPower", 20], ["hp", 1, 5]]
    strike = [[S, 1]]
    # Fifteen distinct Strikes under Unceasing Top: three plays fill the
    # shell, nine more are counted, and the next drawn Strike stays in Hand.
    out["cap_colony_15"] = _spec(top, [S], [S] * 14 + [D], strike, setup=cap, **colony)
    # Thirteen Strikes cycling: the Strike left in Hand comes out of a reshuffle.
    out["cap_colony_13"] = _spec(top, [S], [S] * 12, strike, discard=[D], setup=cap, **colony)
    # Control, short of the shell: Battle Trance draws three Strikes.
    out["cap_colony_trance"] = _spec(
        [], ["BATTLE_TRANCE"], [S] * 3, [["BATTLE_TRANCE"]], setup=cap, **colony)
    # The next turn: the shell and the display are cleared at the side start
    # and the count at the player's side end, so the hand draw plays again.
    out["cap_colony_next_turn"] = _spec(
        top, [S], [S] * 14 + [D], strike + [["@END"]], setup=cap, **colony)
    # A selecting Strike drawn while the count runs: Seeker Strike is the
    # second counted AutoPlay and pauses on its choice. The answer resumes
    # the same listener; the count is not taken again.
    out["cap_colony_seeker"] = _spec(
        top, [S], [S, S, S, S, "SEEKER_STRIKE", S, S, D, D, D], strike, setup=cap, **colony)
    # A lone Strike: the play that finishes is the thirteenth.
    out["cap_colony_lone"] = _spec(top, [S], [], strike, setup=cap, **colony)
    # The Giant about to blow: every draw after the kill is counted.
    out["cap_giant_15"] = _spec(top, [S], [S] * 14 + [D], strike, setup=blow, **giant)
    out["cap_giant_lone"] = _spec(top, [S], [], strike, setup=blow, **giant)
    # Its display never clears, so only Hellraiser's own side-end reset lets
    # the next turn's Strikes play.
    out["cap_giant_next_turn"] = _spec(
        top, [S], [S] * 14 + [D], strike + [["@END"]], setup=blow, **giant)
    # No hittable enemy: a Test Subject between forms. Its AutoPlays have no
    # target and discard the Strike unplayed, and they are counted all the
    # same: nine from three Swift Potions, then Offering's three stay in Hand.
    out["cap_subject_potions"] = _spec(
        [], [S, "OFFERING"], [S] * 13,
        strike + [["@POTION", 0], ["@POTION", 1], ["@POTION", 2], ["OFFERING"]],
        setup=cap + [["hp", 1, 5]], potions=["SWIFT_POTION"] * 3, **subject)
    out["cap_subject_trance"] = _spec(
        [], [S, "BATTLE_TRANCE"], [S] * 3, strike + [["BATTLE_TRANCE"]],
        setup=cap + [["hp", 1, 5]], **subject)
    out["cap_subject_lone"] = _spec(top, [S], [], strike, setup=cap + [["hp", 1, 5]], **subject)
    # ---- Gremlin Horn beside either relic (#3903) ----
    # The Horn's listener runs in `Hook.AfterDeath`, which gives it its own
    # choice context: a choice it reaches is a queued hook action, asked after
    # the PlayCardAction has finished and after that action's own choices.
    horn_tail = ["BASH", "SHRUG_IT_OFF", "TRUE_GRIT", D, "UPPERCUT"]
    weak = [["hp", 2, 5]]
    # Game Piece draws a Strike that kills; the Horn draws Seeker Strike.
    out["horn_piece"] = _spec(
        ["GAME_PIECE", "GREMLIN_HORN"], ["HELLRAISER", "INFLAME", D],
        [D, S, "SEEKER_STRIKE"] + horn_tail, [["HELLRAISER"], ["INFLAME"]], setup=weak)
    # The same kill, then Iron Club's own Draw on the Power parks the action
    # with the Horn's hook action queued behind it.
    out["horn_piece_club"] = _spec(
        ["GAME_PIECE", "IRON_CLUB", "GREMLIN_HORN"], ["HELLRAISER", D, "INFLAME", D],
        [D, S, "SEEKER_STRIKE", "SCULPTING_STRIKE"] + horn_tail,
        [["HELLRAISER"], [D], ["INFLAME"]], setup=weak)
    out["horn_club_piece"] = _spec(
        ["IRON_CLUB", "GAME_PIECE", "GREMLIN_HORN"], ["HELLRAISER", D, "INFLAME", D],
        [D, S, "SEEKER_STRIKE", "SCULPTING_STRIKE"] + horn_tail,
        [["HELLRAISER"], [D], ["INFLAME"]], setup=weak)
    # The Horn draws a plain Strike that is Iron Club's fourth card: the
    # relic's Draw runs on the Horn listener's context.
    out["horn_club_nested"] = _spec(
        ["IRON_CLUB", "GREMLIN_HORN"], ["HELLRAISER", D, D, "BASH", D],
        [S, "SEEKER_STRIKE"] + horn_tail,
        [["HELLRAISER"], [D], [D], ["BASH", 2]], setup=weak)
    # The Horn's Seeker Strike short-lists the two Draw cards left; Iron
    # Club then draws one of them, so one option is left when the hook
    # action starts and the game takes it without asking.
    out["horn_seeker_short"] = _spec(
        ["GAME_PIECE", "IRON_CLUB", "GREMLIN_HORN"], ["HELLRAISER", D, "INFLAME", D],
        [D, S, "SEEKER_STRIKE", "SCULPTING_STRIKE", "BASH"],
        [["HELLRAISER"], [D], ["INFLAME"]], setup=weak)
    # The Horn's Sculpting Strike asks over the Hand; Iron Club then draws a
    # card into it before the hook action starts.
    out["horn_sculpting_moved"] = _spec(
        ["GAME_PIECE", "IRON_CLUB", "GREMLIN_HORN"], ["HELLRAISER", D, "INFLAME", D],
        [D, S, "SCULPTING_STRIKE"] + horn_tail,
        [["HELLRAISER"], [D], ["INFLAME"]], setup=weak)
    # The relic's choice is answered first and the kill comes after it, in
    # the same action.
    out["horn_after_answer"] = _spec(
        ["GAME_PIECE", "IRON_CLUB", "GREMLIN_HORN"], ["HELLRAISER", D, "INFLAME", D],
        [D, "SEEKER_STRIKE", S, "SCULPTING_STRIKE"] + horn_tail,
        [["HELLRAISER"], [D], ["INFLAME"]], setup=[["hp", 1, 19], ["hp", 2, 19]])
    # Further scenarios from a file (`{name: spec}`, the shape `_spec` builds),
    # for a recording that is not one of this probe's own: README "Real waits"
    # gives the turn-end specs that way.
    extra = os.environ.get("STS2_PROBE_SPECS")
    if extra:
        out.update(json.loads(Path(extra).read_text()))
    return out


# --------------------------------------------------------------------------
# one scenario, one CLR process
# --------------------------------------------------------------------------
def run(name, answers):
    # host.py expects the .NET runtime beside it; a fresh worktree has none.
    sys.path.insert(0, os.environ.get("STS2_HARNESS_DIR", str(HERE)))
    import experiment as E
    import host
    import real_waits
    import System
    from experiment import _method, _model_by_id, call, find, prop
    from System import Array, Int32, Object, UInt32, UInt64

    waits = real_waits.from_env()

    def tick(f, seconds):
        """One pump: the plain ExecuteActions, or a real-waits frame jump, and what the game posted.

        The game runs on this thread only (`one_thread.py`), so there is no
        other thread to sleep for: `idle` waits for a `Task.Delay` to post,
        at most `seconds`.
        """
        waits.pump(f.ex)
        waits.idle(seconds)

    ALL = E.ALL
    spec = scenarios()[name]

    def fields(obj):
        out = {}
        t = obj.GetType()
        while t is not None:
            for f in t.GetFields(ALL):
                if not f.IsStatic:
                    out.setdefault(str(f.Name), f)
            t = t.BaseType
        return out

    def rng_state(run_state):
        rng = prop(run_state, "Rng")
        out = {}
        for native, key in STREAMS.items():
            stream = prop(rng, native)
            fs = fields(stream)
            words = None
            for f in fs.values():
                if "MegaRandom" in str(f.FieldType.Name):
                    inner = f.GetValue(stream)
                    words = [(n, int(str(m.GetValue(inner))))
                             for n, m in sorted(fields(inner).items())
                             if str(m.FieldType.Name) == "UInt64"]
            out[key] = {"counter": int(fs["_counter"].GetValue(stream)), "words": words}
        return out

    def card_label(c):
        label = str(prop(c, "Id")).split(".")[-1] + "+" * int(prop(c, "CurrentUpgradeLevel"))
        if "Ethereal" in [str(k) for k in prop(c, "Keywords")]:
            label += "[Ethereal]"
        return label

    def powers(creature):
        return sorted((str(p.GetType().Name), int(prop(p, "Amount")))
                      for p in prop(creature, "Powers"))

    piles = ("Hand", "DrawPile", "DiscardPile", "ExhaustPile", "PlayPile")

    def snap(f):
        pc = prop(f.player, "Creature")
        mons = [dict(id=int(prop(c, "CombatId")), hp=int(prop(c, "CurrentHp")),
                     max_hp=int(prop(c, "MaxHp")), block=int(prop(c, "Block")),
                     powers=powers(c))
                for c in prop(f.cs, "Creatures") if prop(c, "Monster") is not None]
        relics = {str(r.GetType().Name): int(prop(r, "CardsPlayed"))
                  for r in prop(f.player, "Relics") if str(r.GetType().Name) == "IronClub"}
        started = finished = 0
        for e in prop(prop(f.cm, "History"), "Entries"):
            kind = str(e.GetType().Name)
            started += kind == "CardPlayStartedEntry"
            finished += kind == "CardPlayFinishedEntry"
        extra = {}
        # Only a creature whose HP bar is not the ordinary one is listed, and
        # only a nonzero Hellraiser count, so the scenarios with neither
        # re-record byte-identically.
        display = {str(int(prop(c, "CombatId"))): str(prop(c, "HpDisplay"))
                   for c in prop(f.cs, "Creatures") if str(prop(c, "HpDisplay")) != "Normal"}
        if display:
            extra["hp_display"] = display
        for power in prop(pc, "Powers"):
            if str(power.GetType().Name) == "HellraiserPower":
                # `PowerModel._internalData` is the nested `HellraiserPower/Data`.
                data = fields(power)["_internalData"].GetValue(power)
                count = int(fields(data)["infiniteAutoPlaysThisTurn"].GetValue(data))
                if count:
                    extra["hellraiser_infinite_auto_plays"] = count
        if spec.get("encounter"):
            # A dead enemy the game keeps (a reviver between forms).
            dead = [int(prop(c, "CombatId")) for c in prop(f.cs, "Creatures")
                    if prop(c, "Monster") is not None and bool(prop(c, "IsDead"))]
            if dead:
                extra["dead"] = dead
        if spec.get("potions"):
            extra["potions"] = [None if slot is None else str(prop(slot, "Id")).split(".")[-1]
                                for slot in prop(f.player, "PotionSlots")]
        return dict(
            **extra,
            hp=int(prop(pc, "CurrentHp")), max_hp=int(prop(pc, "MaxHp")),
            block=int(prop(pc, "Block")), energy=int(prop(f.pcs, "Energy")),
            powers=powers(pc), mons=mons,
            hand_costs=[int(call(prop(c, "EnergyCost"), "GetResolved"))
                        for c in prop(prop(f.pcs, "Hand"), "Cards")],
            piles={p: [card_label(c) for c in prop(prop(f.pcs, p), "Cards")] for p in piles},
            relics=relics, plays_started=started, plays_finished=finished,
            over=bool(f.over), rng=rng_state(f.run))

    def arrange(f, layout):
        """Each named pile holds exactly the listed cards, index 0 first (the
        Draw pile's top). The scenario's deck is exactly those cards."""
        pool = [c for p in piles for c in prop(prop(f.pcs, p), "Cards")]
        for c in pool:
            pile = prop(c, "Pile")
            remove = _method(pile.GetType(), "RemoveInternal")
            remove.Invoke(pile, Array[Object]([c, True][:remove.GetParameters().Length]))
        for key, native in (("hand", "Hand"), ("draw", "DrawPile"), ("discard", "DiscardPile")):
            pile = prop(f.pcs, native)
            add = _method(pile.GetType(), "AddInternal", 3)
            for idx, label in enumerate(layout.get(key, [])):
                pick = next(c for c in pool if card_label(c) == label)
                pool.remove(pick)
                add.Invoke(pile, Array[Object]([pick, Int32(idx), True]))
        assert not pool, [card_label(c) for c in pool]

    def pump(f, action, want_pause, timeout_s=10.0):
        t0 = time.time()
        state = None
        while time.time() - t0 < timeout_s:
            state = str(prop(action, "State"))
            if state in ("Finished", "Cancelled"):
                return state
            if want_pause and state == "GatheringPlayerChoice":
                return state
            call(f.ex, "Unpause")
            tick(f, 0.02)
        waits.stalled_on("step timed out with the action in state " + str(state))
        return "timeout:" + str(state)

    E._boot_and_init()
    waits.install()
    # Headless has no loc tables: `CardModel.SelectionScreenPrompt` (RVA
    # `0x7c304`) throws on `LocString.Exists`, and `CardSelectorPrefs`' ctor
    # adds the count to that prompt's variables. Both are display-only.
    # Stratagem's prompt is `PowerModel.SelectionScreenPrompt`, the same shape.
    for owner in ("CardModel", "PowerModel"):
        host.patch_skip(find(owner), "get_SelectionScreenPrompt", find("LocString"), "uninit")
    host.patch_skip(find("LocString"), "AddObj")
    player = E._make_player(spec["character"])
    deck = prop(player, "Deck")
    _method(deck.GetType(), "Clear", 1).Invoke(deck, Array[Object]([False]))
    add = _method(deck.GetType(), "AddInternal", 3)
    for idx, label in enumerate(spec["deck"]):
        clone = call(_model_by_id("CARD." + label.rstrip("+"), "CardModel"), "MutableClone")
        for _ in range(label.count("+")):
            call(clone, "UpgradeInternal")
        add.Invoke(deck, Array[Object]([clone, Int32(idx), False]))
    E._grant_relics(player, ["RELIC." + r for r in spec["relics"]])
    E._grant_potions(player, ["POTION." + name for name in spec.get("potions", [])])
    f = E._Fight(player, spec["character"], 0, ["Glory"],
                 spec.get("encounter", "ENCOUNTER.TOADPOLES_WEAK"), spec.get("seed", "W3822"))
    selector = [c for c in find("TestCardSelector").GetConstructors(ALL)
                if not c.IsStatic][0].Invoke(Array[Object]([]))
    _method(find("CardSelectCmd"), "PushSelector").Invoke(None, Array[Object]([selector, True]))
    out = []
    pending = iter(answers)
    # Real waits start at the scenario's root: its setup ran headless.
    begin_at = next((i for i, s in enumerate(spec["script"]) if s[0] in ("real_waits", "snap")), 0)
    for index, step in enumerate(spec["script"]):
        kind = step[0]
        if index == begin_at:
            waits.begin(f)
        if kind == "real_waits":
            pass
        elif kind == "arrange":
            arrange(f, step[1])
        elif kind == "energy":
            _method(f.pcs.GetType(), "set_Energy").Invoke(f.pcs, Array[Object]([Int32(step[1])]))
        elif kind == "hp":
            for c in prop(f.cs, "Creatures"):
                if int(prop(c, "CombatId")) == step[1]:
                    call(c, "SetCurrentHpInternal", System.Decimal(step[2]))
        elif kind == "power":
            # Apply a power to the player through the game's own PowerCmd,
            # before the root snapshot.
            from System import Enum
            creature = prop(f.player, "Creature")
            hpc = find("HookPlayerChoiceContext")
            ga_type = find("GameActionType")
            ga_none = Enum.Parse(ga_type, list(Enum.GetNames(ga_type))[0])
            ctor = [c for c in hpc.GetConstructors(ALL)
                    if str(c.GetParameters()[0].ParameterType.Name) == "Player"][0]
            ctx = ctor.Invoke(Array[Object]([f.player, UInt64(0), ga_none]))
            apply = [m for m in find("PowerCmd").GetMethods(ALL)
                     if str(m.Name) == "Apply" and m.IsGenericMethod
                     and m.GetParameters().Length == 6
                     and str(m.GetParameters()[1].ParameterType.Name) == "Creature"][0]
            task = apply.MakeGenericMethod(find(step[1])).Invoke(None, Array[Object](
                [ctx, creature, System.Decimal(step[2]), creature, None, False]))
            waits.wait(f.ex, task)
        elif kind == "mpower":
            # The same for the creature with that CombatId, as its own applier.
            from System import Enum
            creature = f._creature_by_id(step[1])
            hpc = find("HookPlayerChoiceContext")
            ga_type = find("GameActionType")
            ga_none = Enum.Parse(ga_type, list(Enum.GetNames(ga_type))[0])
            ctor = [c for c in hpc.GetConstructors(ALL)
                    if str(c.GetParameters()[0].ParameterType.Name) == "Player"][0]
            ctx = ctor.Invoke(Array[Object]([f.player, UInt64(0), ga_none]))
            apply = [m for m in find("PowerCmd").GetMethods(ALL)
                     if str(m.Name) == "Apply" and m.IsGenericMethod
                     and m.GetParameters().Length == 6
                     and str(m.GetParameters()[1].ParameterType.Name) == "Creature"][0]
            task = apply.MakeGenericMethod(find(step[2])).Invoke(None, Array[Object](
                [ctx, creature, System.Decimal(step[3]), creature, None, False]))
            waits.wait(f.ex, task)
        elif kind == "pblock":
            # Block on the player, before the root snapshot.
            call(prop(f.player, "Creature"), "GainBlockInternal", System.Decimal(step[1]))
        elif kind == "kill":
            # `CreatureCmd.Kill` on the creature with that CombatId, then let the death settle.
            kill = [m for m in find("CreatureCmd").GetMethods(ALL) if str(m.Name) == "Kill"
                    and str(m.GetParameters()[0].ParameterType.Name) == "Creature"][0]
            waits.wait(f.ex, kill.Invoke(None, Array[Object](
                [f._creature_by_id(step[1])] + [False] * (kill.GetParameters().Length - 1))))
            for _ in range(10):
                call(f.ex, "Unpause")
                tick(f, 0.02)
        elif kind == "snap":
            out.append(dict(label=step[1], snap=snap(f)))
        elif kind == "play" and step[1] == "@END":
            # End the turn through the game's own EndPlayerTurnAction
            # and let the engine run the enemy turn and the next turn start.
            label = f"{index}:@END"
            sources = [call(selector, "SetupForAsyncIndexSelection") for _ in range(4)]
            queue_field = fields(selector)["_indicesToSelectTaskQueue"]

            def asked():
                return 4 - int(queue_field.GetValue(selector).Count)

            ept = [c for c in find("EndPlayerTurnAction").GetConstructors(ALL)
                   if not c.IsStatic][0]
            action = ept.Invoke(Array[Object]([f.player, Int32(int(prop(f.pcs, "TurnNumber")))]))
            turn0 = int(prop(f.pcs, "TurnNumber"))
            call(f.aqs, "EnqueueAction", action, UInt64(int(str(prop(f.player, "NetId")))))
            answered = 0
            t0 = time.time()
            state = "timeout"
            while time.time() - t0 < 30.0:
                call(f.ex, "Unpause")
                tick(f, 0.03)
                if asked() > answered:
                    # settle: pump a little more so anything the game runs
                    # while the choice is open has run
                    for _ in range(10):
                        call(f.ex, "Unpause")
                        tick(f, 0.02)
                    waits.drain(f.ex)
                    running = prop(f.ex, "CurrentlyRunningAction")
                    out.append(dict(
                        label=f"{label}:parked-{answered}", state="GatheringPlayerChoice",
                        running=(str(running.GetType().Name) + ":" + str(prop(running, "State")))
                        if running is not None else None,
                        side=str(prop(f.cs, "CurrentSide")), phase=str(prop(f.pcs, "Phase")),
                        snap=snap(f)))
                    answer = next(pending, None)
                    if answer is None:
                        out.append(dict(label="unanswered"))
                        print("NAT " + json.dumps(out))
                        os._exit(0)
                    picked = E.generic_list(System.Int32)
                    picked.Add(Int32(int(answer)))
                    call(sources[answered], "SetResult", picked)
                    answered += 1
                    continue
                if (str(prop(f.cs, "CurrentSide")) == "Player" and str(prop(f.pcs, "Phase")) == "Play"
                        and int(prop(f.pcs, "TurnNumber")) > turn0
                        and prop(f.ex, "CurrentlyRunningAction") is None):
                    # quiescent for a few more pumps
                    stable = True
                    waits.drain(f.ex)
                    for _ in range(10):
                        call(f.ex, "Unpause")
                        tick(f, 0.02)
                        if asked() > answered or prop(f.ex, "CurrentlyRunningAction") is not None:
                            stable = False
                            break
                    if stable:
                        state = "Finished"
                        break
                if f.over:
                    # The end of a combat still has waits to run out.
                    waits.drain(f.ex)
                    state = "Finished"
                    break
            if state == "timeout":
                waits.stalled_on("the end of the turn timed out")
            call(selector, "Cleanup")
            out.append(dict(label=f"{label}:done", state=state, snap=snap(f)))
        elif kind == "play" and step[1] == "@POTION":
            # Use a belt potion through the game's own UsePotionAction.
            label = f"{index}:@POTION"
            sources = [call(selector, "SetupForAsyncIndexSelection") for _ in range(4)]
            ctor = [c for c in find("UsePotionAction").GetConstructors(ALL)
                    if not c.IsStatic and c.GetParameters().Length == 5][0]
            action = ctor.Invoke(Array[Object]([f.player, UInt32(int(step[2])), None, None, True]))
            call(f.aqs, "EnqueueAction", action, UInt64(int(str(prop(f.player, "NetId")))))
            for park, source in enumerate(sources):
                state = pump(f, action, True)
                if state != "GatheringPlayerChoice":
                    break
                waits.drain(f.ex)
                out.append(dict(label=f"{label}:parked-{park}", state=state, snap=snap(f)))
                answer = next(pending, None)
                if answer is None:
                    out.append(dict(label="unanswered"))
                    print("NAT " + json.dumps(out))
                    os._exit(0)
                picked = E.generic_list(System.Int32)
                picked.Add(Int32(int(answer)))
                call(source, "SetResult", picked)
            state = pump(f, action, False)
            waits.drain(f.ex)
            call(selector, "Cleanup")
            out.append(dict(label=f"{label}:done", state=state, snap=snap(f)))
        elif kind == "play":
            label = f"{index}:{step[1]}"
            card = next(c for c in prop(prop(f.pcs, "Hand"), "Cards") if card_label(c) == step[1])
            # More pending answers than any one play of these scenarios asks.
            sources = [call(selector, "SetupForAsyncIndexSelection") for _ in range(4)]
            ctor = [c for c in find("PlayCardAction").GetConstructors(ALL)
                    if not c.IsStatic and c.GetParameters().Length == 2][0]
            action = ctor.Invoke(Array[Object]([card, f._creature_by_id(step[2])]))
            call(f.aqs, "EnqueueAction", action, UInt64(int(str(prop(f.player, "NetId")))))
            for park, source in enumerate(sources):
                state = pump(f, action, True)
                if state != "GatheringPlayerChoice":
                    break
                waits.drain(f.ex)
                out.append(dict(label=f"{label}:parked-{park}", state=state, snap=snap(f)))
                answer = next(pending, None)
                if answer is None:
                    # More choices than answers: say so and stop.
                    out.append(dict(label="unanswered"))
                    print("NAT " + json.dumps(out))
                    os._exit(0)
                picked = E.generic_list(System.Int32)
                picked.Add(Int32(int(answer)))
                call(source, "SetResult", picked)
            state = pump(f, action, False)
            waits.drain(f.ex)
            if "GREMLIN_HORN" in spec["relics"]:
                # A choice begun inside Gremlin Horn's AfterDeath listener is
                # a queued GenericHookGameAction: it is asked only once this
                # PlayCardAction has finished. Only a Horn scenario pumps
                # for it, so the others re-record byte-identically.
                queue_field = fields(selector)["_indicesToSelectTaskQueue"]
                answered = sum(f"{label}:parked-" in entry["label"] for entry in out)
                while answered < len(sources):
                    for _ in range(10):
                        call(f.ex, "Unpause")
                        tick(f, 0.02)
                    waits.drain(f.ex)
                    if len(sources) - int(queue_field.GetValue(selector).Count) <= answered:
                        break
                    running = prop(f.ex, "CurrentlyRunningAction")
                    out.append(dict(
                        label=f"{label}:parked-{answered}",
                        state=str(prop(running, "State")) if running is not None else None,
                        running=str(running.GetType().Name) if running is not None else None,
                        snap=snap(f)))
                    answer = next(pending, None)
                    if answer is None:
                        out.append(dict(label="unanswered"))
                        print("NAT " + json.dumps(out))
                        os._exit(0)
                    picked = E.generic_list(System.Int32)
                    picked.Add(Int32(int(answer)))
                    call(sources[answered], "SetResult", picked)
                    answered += 1
            call(selector, "Cleanup")
            out.append(dict(label=f"{label}:done", state=state, snap=snap(f)))
        else:
            raise SystemExit(f"bad step {step}")
    if waits.active:
        waits.drain(f.ex)
        out.append(dict(label="waits", report=waits.report()))
    print("NAT " + json.dumps(out))


# --------------------------------------------------------------------------
# every answer sequence of a scenario
# --------------------------------------------------------------------------
def _drive():
    """`probe_drive`, the harness module both native probes share for the parent half of `drive`."""
    sys.path.insert(0, os.environ.get("STS2_HARNESS_DIR", str(HERE)))
    import probe_drive
    return probe_drive


def _run_child(name, answers):
    # None only for an answer past the last option. A child that stalled,
    # threw anywhere else or printed no trace raises: never "no such option".
    return _drive().run_child(Path(__file__).resolve(), name, answers)


def drive(name):
    runs = _drive().explore(lambda answers: _run_child(name, answers))
    out = dict(build="v0.111.0", dll_sha256=DLL_SHA256, name=name,
               spec=scenarios()[name], runs=runs)
    if os.environ.get("STS2_WAITS", "off") not in ("", "off"):
        out["waits"] = os.environ["STS2_WAITS"]
    return out


def main(argv):
    if len(argv) >= 2 and argv[0] == "run":
        answers = [a for a in argv[2].split(",") if a != "-"] if len(argv) > 2 else []
        run(argv[1], answers)
    elif argv and argv[0] == "drive":
        out = Path(argv[argv.index("--out") + 1])
        names = [a for i, a in enumerate(argv[1:], 1)
                 if a != "--out" and argv[i - 1] != "--out"] or sorted(scenarios())
        out.mkdir(parents=True, exist_ok=True)
        for name in names:
            trace = drive(name)
            text = json.dumps(trace, separators=(",", ":"))
            suffix = "waits.json" if "waits" in trace else "json"
            (out / f"{name}.{suffix}").write_text(text + "\n")
            print(name, [run["answers"] for run in trace["runs"]])
    elif argv and argv[0] == "list":
        print("\n".join(sorted(scenarios())))
    else:
        raise SystemExit(__doc__)


if __name__ == "__main__":
    main(sys.argv[1:])

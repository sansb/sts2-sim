# Native traces: a choice inside Constrict's side-end damage (#3756)

Recorded from the archived v0.111.0 `sts2.dll` (SHA-256 `9cb4f1ad…`) during
the independent review of the change that built
`turn::side_end_prompt_starts_later_listeners`, with a scratch copy of the
#3822 probe. Each file holds its own
`spec` (the script that was run) and one run per answer sequence, with the
whole observable state at the root, at each pause and after each action.
They are replayed by
`src/engine/puzzle/tests/after_card_played/constrict_native.rs`.

Every scenario runs the real `ENCOUNTER.SLITHERING_STRANGLER_NORMAL` on seed
`RV3920A`: turn 1 with 60 Block and the Twig Slime killed, so the Strangler's
own CONSTRICT applies Constrict 3 and the Centennial Puzzle stays armed; then
the turn-2 layout is arranged, the `root` snapshot taken, and the game's own
`EndPlayerTurnAction` run with `TestCardSelector` as the local selector.

| file | mode | what it records |
|---|---|---|
| `a6_shadow_kill.json` | headless | Consuming Shadow's side-end evoke of a Lightning orb kills the only enemy while the Puzzle's Stratagem pick waits. The combat ends and the pick is never applied. The engine reproduces it. |
| `r4_blocked_grown.json` | headless | An earlier Dark Embrace's pick waits while Constrict's hit is fully blocked. The Puzzle does not fire and the turn end runs through. The engine reproduces it. |
| `r4b_grown_unblocked.json` | headless | The same root without the Block: the Puzzle fires beside the waiting pick. The engine refuses by name (`deferred hook action choice source moved`). |
| `d3_seeker_one_rage.headless.json` | headless | Hellraiser plays a drawn Seeker Strike whose select has one candidate and takes it without a prompt, with Rage 3 as a later side-end listener. The owner is at 71 HP at the next turn's first prompt. |
| `d3_seeker_one_rage.waits.json` | real waits | The same root and seed. The owner is at 65 HP at that prompt: Rage was removed before the listener's later Draws. |

## The two modes

**Headless** is the harness as it stands: `NonInteractiveMode` is active, so
`Cmd.Wait` returns at once (`Cmd/<Wait>d__1` RVA `0x3e8d28` IL_0019-0020) and
a hook listener never suspends before it signals a choice.

**Real waits** is the same scratch probe driven through the real-waits
harness module of #3931 (`harness/real_waits.py`), on Normal fast mode:
`Cmd.Wait` and `Cmd.CustomScaledWait` run as timers, so a listener that
reaches one has really suspended, as it has in play. Every snapshot is taken
with no timer pending, and each run ends with a `waits` entry reporting how
many timers were pending at once. It is not interactive play: the timers are
the harness's, not the game's frame clock, and there are no card tweens. The
review's first recording of this root used a Harmony emulation of the two
wait commands and gave the same 65 HP.

Where the two modes disagree, the engine does not take either order as the
game's: it refuses by name. The `d3` pair is that case
(`deferred-choice listener choice resolved without a prompt`); the reason is
in the doc comment of `turn::side_end_prompt_starts_later_listeners`.

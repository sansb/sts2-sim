"""#2512 — card/potion generation under a partial (or superset) profile.

Two claims, and the corpus carries a counterexample to the old behaviour for
each:

1. **A superset is fully unlocked.** `UnlockState.unlocked_epochs` carries
   every epoch a profile has revealed, card-pool or not, and
   `live_coach.card_pool_unlocked_epochs` passes it through verbatim. The
   gates compared that tuple for *equality* against the 39 character +
   Colorless epochs, so the 57-entry profile that is literally every epoch
   this build declares was misread as partial.
2. **A partial profile derives its own pool rather than refusing** — and,
   where the old code did not refuse, it answered with the wrong pool:
   `fully_unlocked_card_pool` is `live_coach.infernal_blade_pool_fully_
   unlocked`, which tests the three IRONCLAD epochs and nothing else.

Native authority for every epoch fact below is the archived v0.111.0
assembly, sha256
`9cb4f1ad8c9f284aa8fec3122ffd6d780bbf543d875c817abdd12ff63fbf12b4`.

Trimmed by #2827 item F: the tests here that exercised the deleted Python
simulator went with it; the ones that remain never reached it.
"""
import json
import pathlib


_HERE = pathlib.Path(__file__).parent
_COLORLESS = json.loads((_HERE / "colorless_card_pool.json").read_text())


# --- the Colorless pool ----------------------------------------------------


def test_the_colorless_pool_is_gated_three_rows_per_epoch():
    gated = [r for r in _COLORLESS["cards"] if r["unlock_epoch"]]
    assert len(gated) == 15
    for index in range(1, 6):
        epoch = f"COLORLESS{index}_EPOCH"
        assert len([r for r in gated if r["unlock_epoch"] == epoch]) == 3

"""
Predict the first-fight draw order for a fresh STS2 run from the seed alone.

Model (confirmed against ground truth on seed ZPJHU3WSH2, Ironclad A10,
build v0.108.0 — see issue #54):
  - draw pile starts as the deck exactly in save-file (acquisition) order
  - shuffled once at combat start by the run's `Shuffle` stream at counter 0
    (Rng.Shuffle = top-down Fisher-Yates, one draw per swap)
  - cards are drawn from the front of the shuffled list
  - hand display order (left to right) = draw order

The observed 11-card permutation matched exactly; the hypothesis space
(12 streams x 60 counters x 2 initial orders x 2 draw directions) contained
no other match.

Usage:
    python3 predict_opening_hand.py SEEDSTRING [--deck S,S,S,D,...]
"""

import sys
import pathlib

sys.path.insert(0, str(pathlib.Path(__file__).parent))
from sts2_rng import Rng, RunRngSet  # noqa: E402

# Fresh Ironclad @ A10+, in save-file order (5 Strike, 4 Defend, Bash,
# Ascender's Bane). A0-A9 would drop the Bane (untested).
IRONCLAD_A10 = ["Strike", "Strike", "Strike", "Strike", "Strike",
                "Defend", "Defend", "Defend", "Defend", "Bash",
                "AscendersBane"]


def predict(seed_string: str, deck: list[str] | None = None,
            shuffle_counter: int = 0) -> list[str]:
    """Return the full predicted draw order for one combat's first cycle."""
    deck = list(deck or IRONCLAD_A10)
    rs = RunRngSet(seed_string)
    rng = Rng(rs["Shuffle"].seed, counter=shuffle_counter)
    rng.shuffle(deck)
    return deck


if __name__ == "__main__":
    seed = sys.argv[1] if len(sys.argv) > 1 else "ZPJHU3WSH2"
    deck = None
    if "--deck" in sys.argv:
        deck = sys.argv[sys.argv.index("--deck") + 1].split(",")
    order = predict(seed, deck)
    print(f'seed "{seed}" — predicted first-fight draw order:')
    print(f"  turn 1 hand: {', '.join(order[:5])}")
    print(f"  turn 2 hand: {', '.join(order[5:10])}")
    if len(order) > 10:
        print(f"  turn 3 first draw(s): {', '.join(order[10:])}"
              f"  (then reshuffle — depends on play order)")

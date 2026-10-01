#!/usr/bin/env python3
"""The ``sts-sim-canonical-v2`` document layer, with no simulator import.

Everything here operates on an already-projected canonical document (a plain
JSON-shaped ``dict``) or names a constant of the wire schema. Nothing imports
``combat_sim``: the review worker, the eval suite, the opening census and the
Rust-side tools digest and compare documents that ``sts-sim`` emits, and none
of that needs the frozen Python State projector to exist (#2827 item D, which
prepares the hard delete of ``combat_sim.py`` in item F).

``project_state.py`` — the Python ``State`` -> document projector — used to
re-export every name below; item F deleted it with the simulator, so this is
the only Python home of the document layer.

Digest contract (unchanged; ``src/canonical.rs`` is the Rust twin)::

    sha256(json.dumps(doc, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False).encode("utf-8"))

``ensure_ascii=False`` is load-bearing: ``serde_json`` emits UTF-8 and does not
``\\u``-escape non-ASCII, so the Rust and Python byte streams agree only with
escaping disabled.

Standard library only.
"""

from __future__ import annotations

import hashlib
import json
from typing import Any

SCHEMA = "sts-sim-canonical-v2"

#: The build every v0.111.0 document is stamped with.
GAME_BUILD = "v0.111.0"

#: State fields that hold ordered piles of physical cards.
PILE_FIELDS = ("draw", "hand", "discard", "exhaust", "play")

#: State fields that hold a ``(s0, s1, s2, s3, counter)`` xoshiro stream.
#: Matched by name, not by shape: a five-int tuple is not proof of an RNG.
#: A stream added under a new name falls through to the generic bag, which is
#: a visible schema change, not a silent drop.
RNG_FIELDS = (
    "rng",
    "niche",
    "ai",
    "sel",
    "generation",
    "targets",
    "energy_costs",
    "combat_orbs",
    "potion_generation",
)

MONSTERS_FIELD = "monsters"
CONTINUATIONS_FIELD = "continuations"
ENEMY_PENDING_FIELD = "enemy_pending"

#: `_FRAME_PHASE` is the sole continuation without a NamedTuple class. Its
#: positions are the authoritative `_validate_continuation_frame` Phase arm
#: and `_advance_phase_frame` unpacking order. The synthetic class name keeps
#: the canonical frame envelope explicit without pretending the Python value
#: has fields it does not actually expose.
PHASE_FRAME_TYPE = "PhaseFrame"
PHASE_FRAME_FIELDS = (
    "tag",
    "phase",
    "subphase",
    "snapshot",
    "cursor",
    "terminal",
    "remaining",
)

# R26's restricted AutoBatch keeps the legacy five-field envelope while each
# entry carries one complete frozen physical-card payload. Rust re-derives and
# compares that payload and order from live piles; it is not a capture receipt.
AUTO_BATCH_FRAME_TYPE = "FrozenAutoBatchFrame"
AUTO_BATCH_FRAME_FIELDS = (
    "tag",
    "entries",
    "cursor",
    "force_exhaust",
    "source",
)

ACTION_REPLAY_FRAME_TYPE = "ActionReplayFrame"

#: Documented physical-card slots, in payload order from index 2.
CARD_SLOT_NAMES = (
    "enchantment",          # 2 immutable run enchantment payload
    "local_keywords",       # 3 combat-local keywords on this CardModel
    "transient_keywords",   # 4 turn-scoped keywords
    "enchantment_state",    # 5 mutable Momentum/Vigorous state
    "sovereign_blade",      # 6 mutable SovereignBlade fields
)
CARD_PHYSICAL_STATE_SLOT = 7    # ordered cost modifiers + damage growth
CARD_EXTRA_SLOT = 8             # opaque tagged CardModel-local rows


def canonical_json(document: dict[str, Any]) -> str:
    """The exact wire bytes both engines digest."""
    return json.dumps(document, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False)


def differential_digest(document: dict[str, Any]) -> str:
    return hashlib.sha256(canonical_json(document).encode("utf-8")).hexdigest()

#!/usr/bin/env python3
"""Mutation controls for the two Python multiplayer-card gates.

The generated census (#1613) pairs static identity sets with separately
evaluated *live* conditions, so the extractor is the only thing standing
between a widened or reshaped Python gate and a silently wrong Rust
`can_play`. It also pins the deliberate application-only Largesse delta
(#1688). Reading either source site is not enough: every guard below is
exercised against mutated Python and must refuse unsupported drift.

Two defects these controls exist to prevent, both found in review of #1623:

* the condition matcher checked the callee name but not the receiver, so
  ``len(_living_player_keys(probe)) <= 1`` was accepted *and relabelled* as
  the admitted-boundary constant;
* the operand loop skipped anything that was not exactly a two-conjunct
  ``And`` with the identity test first, so a widened gate written with
  reversed conjuncts was dropped while the membership pin stayed green.

Standard library only; run directly (see `.github/workflows/rust-port.yml`).
"""

from __future__ import annotations

import ast
import importlib.util
import pathlib
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve()
RUST_DIR = HERE.parents[1]


def _load_generator():
    spec = importlib.util.spec_from_file_location(
        "generate_content", RUST_DIR / "tools" / "generate_content.py"
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


GEN = _load_generator()

# A minimal `_card_can_play` with the same shape as the real one: a single
# `return not (...)` disjunction carrying the real multiplayer gate plus
# whichever mutation a case wants to inject.
TEMPLATE = '''
def _card_can_play(s, card, *, earring: bool = False):
    return not (
        not spec.playable
        or card[0] in {{"BEACON_OF_HOPE"}} and len(_living_player_keys(s)) <= 1
        {extra}
        or card_cost_in_state(s, card) > s.energy
    )
'''

AXES = {
    "CardId": {
        "BEACON_OF_HOPE",
        "ENTHRALLED",
        "LARGESSE",
        "OUTRAGE",
        "TAG_TEAM",
        "UNDERWORLD",
    }
}

CONTRACT_SOURCE = '''
def _card_can_play(s, card, *, earring: bool = False):
    return not (
        not spec.playable
        or card[0] in {"BEACON_OF_HOPE"}
        and len(_living_player_keys(s)) <= 1
    )


def _apply_action_impl(s, action):
    card = action[1]
    if card[0] in {"BEACON_OF_HOPE", "LARGESSE"}:
        if len(_living_player_keys(s)) <= 1:
            raise NotImplementedError("MultiplayerOnly")
        apply_body(s, card)
'''


def _census(extra: str):
    """Run the extractor over a synthesized `_card_can_play`."""
    directory = pathlib.Path(tempfile.mkdtemp())
    (directory / "combat_sim.py").write_text(TEMPLATE.format(extra=extra))
    original = GEN.SOLVER_DIR
    GEN.SOLVER_DIR = directory
    try:
        return GEN.solo_unplayable_census(None, AXES)
    finally:
        GEN.SOLVER_DIR = original


def _contract(source: str = CONTRACT_SOURCE):
    """Run the cross-gate contract over synthesized Python source."""
    directory = pathlib.Path(tempfile.mkdtemp())
    (directory / "combat_sim.py").write_text(source)
    original = GEN.SOLVER_DIR
    GEN.SOLVER_DIR = directory
    try:
        return GEN.multiplayer_gate_contract(None, AXES)
    finally:
        GEN.SOLVER_DIR = original


def _expr(source: str):
    return ast.parse(source, mode="eval").body


class SoloConstantReceivers(unittest.TestCase):
    """The condition must be about the live state `s`, not any name."""

    def test_the_real_forms_are_recognised(self) -> None:
        self.assertEqual(
            GEN._solo_constant_condition(_expr("len(_living_player_keys(s)) <= 1")),
            "len(_living_player_keys(s)) <= 1",
        )
        self.assertEqual(
            GEN._solo_constant_condition(_expr("not s.teammate_present")),
            "not s.teammate_present",
        )

    def test_a_different_receiver_is_refused(self) -> None:
        # Previously accepted AND relabelled as the `s` form, which would have
        # emitted a table whose doc comment asserts a false provenance.
        for source in (
            "len(_living_player_keys(probe)) <= 1",
            "len(_living_player_keys(other_state)) <= 1",
            "not other.teammate_present",
            "not probe.teammate_present",
        ):
            with self.subTest(source=source):
                self.assertIsNone(GEN._solo_constant_condition(_expr(source)))

    def test_a_different_arity_or_threshold_is_refused(self) -> None:
        for source in (
            "len(_living_player_keys(s, extra)) <= 1",
            "len(_living_player_keys(s=s)) <= 1",
            "len(_living_player_keys(s)) <= 2",
            "len(_other_helper(s)) <= 1",
        ):
            with self.subTest(source=source):
                self.assertIsNone(GEN._solo_constant_condition(_expr(source)))


class WidenedGatesFailClosed(unittest.TestCase):
    """A gate the extractor cannot classify must stop codegen, not be skipped."""

    def test_the_baseline_shape_still_extracts(self) -> None:
        self.assertEqual(
            _census(""), {"len(_living_player_keys(s)) <= 1": ["BEACON_OF_HOPE"]}
        )

    def test_conjunct_order_is_not_assumed(self) -> None:
        # Python may legitimately put the live condition first; that is the
        # same gate and must be picked up, not refused.
        census = _census(
            '\n        or len(_living_player_keys(s)) <= 1 and card[0] in {"TAG_TEAM"}'
        )
        self.assertEqual(
            census,
            {"len(_living_player_keys(s)) <= 1": ["BEACON_OF_HOPE", "TAG_TEAM"]},
        )

    def test_an_unclassifiable_widened_gate_raises(self) -> None:
        # Each of these previously produced a census that silently omitted
        # TAG_TEAM while the 31-member pin stayed green.
        for label, extra in (
            (
                "extra conjunct",
                '\n        or card[0] in {"TAG_TEAM"} and'
                " len(_living_player_keys(s)) <= 1 and s.hp > 0",
            ),
            (
                "unrecognised condition",
                '\n        or card[0] in {"TAG_TEAM"} and s.some_new_flag',
            ),
            (
                "wrong receiver",
                '\n        or card[0] in {"TAG_TEAM"} and'
                " len(_living_player_keys(probe)) <= 1",
            ),
            (
                "bare identity test",
                '\n        or card[0] in {"TAG_TEAM"}',
            ),
        ):
            with self.subTest(label=label):
                with self.assertRaises(GEN._SoloGateShapeChanged):
                    _census(extra)

    def test_an_identity_outside_the_card_axis_raises(self) -> None:
        with self.assertRaises(GEN._SoloGateShapeChanged):
            _census(
                '\n        or card[0] in {"NOT_A_REAL_CARD"} and'
                " len(_living_player_keys(s)) <= 1"
            )


class KnownNonGateOperands(unittest.TestCase):
    """The one `card[0]` operand that is deliberately not a gate."""

    def test_the_enthralled_lockout_is_tolerated(self) -> None:
        census = _census(
            "\n        or card[0] != 'ENTHRALLED'"
            " and any(candidate[0] == 'ENTHRALLED' for candidate in s.hand)"
        )
        self.assertEqual(
            census, {"len(_living_player_keys(s)) <= 1": ["BEACON_OF_HOPE"]}
        )

    def test_a_rewritten_enthralled_operand_raises(self) -> None:
        # The allowlist is keyed by exact source so a semantic rewrite
        # re-opens the classification rather than being reabsorbed.
        with self.assertRaises(GEN._SoloGateShapeChanged):
            _census(
                "\n        or card[0] != 'ENTHRALLED'"
                " and any(c[0] == 'ENTHRALLED' for c in s.draw)"
            )


class ApplyGateDeltaContract(unittest.TestCase):
    """The apply gate differs from CanPlay by exactly the named Largesse row."""

    def test_the_named_directional_delta_is_accepted(self) -> None:
        self.assertEqual(
            _contract(),
            {
                "can_play_census": {
                    "len(_living_player_keys(s)) <= 1": ["BEACON_OF_HOPE"]
                },
                "apply": ["BEACON_OF_HOPE", "LARGESSE"],
                "apply_only": ["LARGESSE"],
                "can_play_only": [],
            },
        )

    def test_membership_drift_in_either_direction_raises(self) -> None:
        mutations = {
            "Largesse removed from apply": CONTRACT_SOURCE.replace(
                ', "LARGESSE"', ""
            ),
            "new apply-only identity": CONTRACT_SOURCE.replace(
                '"BEACON_OF_HOPE", "LARGESSE"',
                '"BEACON_OF_HOPE", "LARGESSE", "TAG_TEAM"',
            ),
            "Largesse moved into CanPlay": CONTRACT_SOURCE.replace(
                'card[0] in {"BEACON_OF_HOPE"}\n        and',
                'card[0] in {"BEACON_OF_HOPE", "LARGESSE"}\n        and',
            ),
            "new CanPlay-only identity": CONTRACT_SOURCE.replace(
                'card[0] in {"BEACON_OF_HOPE"}\n        and',
                'card[0] in {"BEACON_OF_HOPE", "TAG_TEAM"}\n        and',
            ),
        }
        for label, source in mutations.items():
            with self.subTest(label=label):
                with self.assertRaises(GEN._SoloGateShapeChanged):
                    _contract(source)

    def test_apply_evaluation_point_drift_raises(self) -> None:
        mutations = {
            "wrong state": CONTRACT_SOURCE.replace(
                "if len(_living_player_keys(s)) <= 1:\n"
                "            raise NotImplementedError",
                "if len(_living_player_keys(probe)) <= 1:\n"
                "            raise NotImplementedError",
                1,
            ),
            "wrong exception": CONTRACT_SOURCE.replace(
                'raise NotImplementedError("MultiplayerOnly")',
                'raise ValueError("MultiplayerOnly")',
            ),
            "effect before refusal": CONTRACT_SOURCE.replace(
                "    if card[0] in {\"BEACON_OF_HOPE\", \"LARGESSE\"}:\n"
                "        if len(_living_player_keys(s)) <= 1:",
                "    if card[0] in {\"BEACON_OF_HOPE\", \"LARGESSE\"}:\n"
                "        apply_body(s, card)\n"
                "        if len(_living_player_keys(s)) <= 1:",
            ),
        }
        for label, source in mutations.items():
            with self.subTest(label=label):
                with self.assertRaises(GEN._SoloGateShapeChanged):
                    _contract(source)

    def test_duplicate_named_functions_raise(self) -> None:
        with self.assertRaises(GEN._SoloGateShapeChanged):
            _contract(CONTRACT_SOURCE + CONTRACT_SOURCE)


class RealSourceStillAgrees(unittest.TestCase):
    """The hardened extractor must not have changed what it emits.

    Since #2827 item D the "real source" is the frozen registry snapshot: the
    generator replays these two censuses from
    the committed `data/python_registry.*.json` rather than re-parsing
    `combat_sim.py`, which item F deletes. The pins are unchanged, and
    `registry_snapshot.py --verify-replay` is the pre-deletion proof that the
    recording equals a live parse.
    """

    @staticmethod
    def _recorded(name):
        registry = GEN.load_registry()
        censuses = object.__getattribute__(registry, "_censuses")
        keys = [key for key in censuses if key.startswith(f"{name}:")]
        if len(keys) != 1:
            raise AssertionError(f"expected one recorded {name}, got {keys}")
        return registry.census(keys[0])[0]

    def test_the_live_registry_yields_the_pinned_census(self) -> None:
        # The generator reaches the solo census only through the contract,
        # which carries it whole as `can_play_census`.
        census = self._recorded("multiplayer_gate_contract")["can_play_census"]
        self.assertEqual(len(census["len(_living_player_keys(s)) <= 1"]), 31)
        self.assertEqual(census["not s.teammate_present"], ["UNDERWORLD"])
        self.assertIn("INTERCEPT", census["len(_living_player_keys(s)) <= 1"])
        self.assertIn("SOULBOUND", census["len(_living_player_keys(s)) <= 1"])

    def test_the_live_apply_gate_has_only_the_named_largesse_delta(self) -> None:
        contract = self._recorded("multiplayer_gate_contract")
        can_play = contract["can_play_census"][
            "len(_living_player_keys(s)) <= 1"
        ]
        self.assertEqual(len(can_play), 31)
        self.assertEqual(len(contract["apply"]), 32)
        self.assertNotIn("LARGESSE", can_play)
        self.assertEqual(contract["apply_only"], ["LARGESSE"])
        self.assertEqual(contract["can_play_only"], [])


if __name__ == "__main__":
    unittest.main()

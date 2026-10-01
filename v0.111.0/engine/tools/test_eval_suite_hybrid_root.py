#!/usr/bin/env python3
"""The Rust-only provenance registry, pinned against the crate (#2693).

The two-sided opening-parity gate compares a Rust-built root with frozen
Python's and exempts only the wire slots Python structurally cannot emit.
That exemption set lives in `eval_suite.RUST_ONLY_PROVENANCE_SLOTS`, and its
correctness is a claim about `src/boundary.rs`, not about the tool. Since
#2999 the eval census roots every fight from Rust alone, and the same
change retired `opening_census.py`, the gate's last reader. The registry and
the gate stay pinned here until #2827 item F removes them together.

So this module lives in **this** lane rather than with the rest of
`test_eval_suite.py`. The solver fast gate's `TRIGGER_SET` deliberately
excludes `sim/v0.111.0/engine/**` (#1276), so a solver test that read
`boundary.rs` would be a pin nothing re-runs when `boundary.rs` changes —
exactly the silent coupling the fast gate's external-surface guard exists to
prevent. `rust port` triggers on `sim/v0.111.0/engine/**`, which is the
file this pin is about.

The pin is deliberately two-directional. A slot marked `// Rust-only:` in the
crate but missing from the registry makes the gate reject every root that
carries it, reporting a divergence that does not exist. A slot left in the
registry after the crate stops marking it makes the gate SUBTRACT a field
Python does emit — the #1432 *new-reader-invalidates-old-shortcut* class,
where the gate goes blind to a genuine opening-parity defect on that field.
Neither direction is visible in a green run.

Standard library only, no cargo, no corpus; run directly (see
`.github/workflows/rust-port.yml`).
"""

from __future__ import annotations

import json
import pathlib
import sys
import unittest

HERE = pathlib.Path(__file__).resolve()
TOOLS_DIR = HERE.parent
RUST_DIR = HERE.parents[1]
BOUNDARY_RS = RUST_DIR / "src" / "boundary.rs"

if str(TOOLS_DIR) not in sys.path:
    sys.path.insert(0, str(TOOLS_DIR))

import eval_suite  # noqa: E402


class TheRegistryAgreesWithBoundaryRs(unittest.TestCase):
    """`RUST_ONLY_PROVENANCE_SLOTS` is what `boundary.rs` says it is."""

    def test_the_registry_and_the_crate_name_the_same_slots(self) -> None:
        derived = eval_suite.derive_rust_only_slots()
        self.assertEqual(
            derived,
            tuple(sorted(eval_suite.RUST_ONLY_PROVENANCE_SLOTS)),
            "sim/v0.111.0/engine/src/boundary.rs and "
            "eval_suite.RUST_ONLY_PROVENANCE_SLOTS disagree about which wire "
            "slots the frozen Python oracle structurally cannot emit. Add the "
            "new slot to the registry deliberately — widening the gate's "
            "exemption set is a decision, not a refresh — or remove the stale "
            "one.")

    def test_the_registry_is_not_vacuously_empty(self) -> None:
        """An empty registry would make the gate a plain equality check.

        That is not wrong, but it is a different measurement, and #2693 exists
        because such slots exist today (#3026 added the second).
        """
        self.assertEqual(eval_suite.derive_rust_only_slots(),
                         ("power_attachments", "scroll_chew_repeated"))

    def test_the_marker_really_annotates_a_spec_in_that_file(self) -> None:
        text = BOUNDARY_RS.read_text(encoding="utf-8")
        self.assertIn("Rust-only", text)
        self.assertIn('"power_attachments"', text)
        # And the crate still says WHY, which is the fact the registry rests
        # on: there is no `combat_sim` counterpart at all.
        self.assertIn("never emits this key", text)

    def test_the_tools_own_self_test_checks_the_same_pin(self) -> None:
        """`eval_suite.py --self-test` fails on a stale registry too.

        `solver/tools/version_bump_readiness.py` runs that self-test, so the
        pin is also checked on the one day the whole tool tree forks forward.
        """
        source = (TOOLS_DIR / "eval_suite.py").read_text(encoding="utf-8")
        self.assertIn("derive_rust_only_slots()", source)


class TheDerivationCannotQuietlyMissASlot(unittest.TestCase):
    """Mutation controls: the derivation has to be able to fail."""

    def test_a_second_marked_slot_is_found(self) -> None:
        text = (
            '    // Rust-only: no Python counterpart.\n'
            '    spec(\n        "power_attachments",\n        X,\n    ),\n'
            '    spec("hp", Y, Z),\n'
            '    // Rust-only: also no Python counterpart.\n'
            '    spec("future_ledger", W, V),\n'
        )
        self.assertEqual(eval_suite.derive_rust_only_slots(text),
                         ("future_ledger", "power_attachments"))

    def test_an_unmarked_slot_is_not_found(self) -> None:
        self.assertEqual(
            eval_suite.derive_rust_only_slots('    spec("hp", Y, Z),\n'), ())

    def test_a_marker_with_no_spec_refuses_rather_than_dropping_it(self) -> None:
        with self.assertRaises(eval_suite.EvalRefusal):
            eval_suite.derive_rust_only_slots("// Rust-only: nothing follows\n")

    def test_the_marker_matches_the_crate_comment_style(self) -> None:
        """Both the single- and multi-line `spec(` spellings are read."""
        single = '// Rust-only: x\nspec("one", A, B),\n'
        multi = '// Rust-only: x\nspec(\n    "two",\n    A,\n    B,\n),\n'
        self.assertEqual(eval_suite.derive_rust_only_slots(single), ("one",))
        self.assertEqual(eval_suite.derive_rust_only_slots(multi), ("two",))


class TheGateSubtractsExactlyTheRegisteredSlots(unittest.TestCase):
    """The exemption is the registry's, not the caller's."""

    def test_an_unregistered_rust_only_looking_slot_still_fails_the_gate(self) -> None:
        """A plausible-looking new ledger is a mismatch until it is registered.

        This is the load-bearing half of the two-directional pin, stated as
        behaviour: the gate does not pattern-match on names that look like
        provenance.
        """
        agrees, paths = eval_suite.opening_parity(
            {"monsters": [{"hp": 3}]},
            {"monsters": [{"hp": 3, "future_ledger": [{"power": "X"}]}]})
        self.assertFalse(agrees)
        self.assertEqual(paths, ["monsters[0].future_ledger"])

    def test_a_registered_slot_is_subtracted_from_both_sides(self) -> None:
        # Not just "absent on the left": a document that carries the slot on
        # BOTH sides with different contents is also exempt, because the slot
        # is subtracted rather than compared.
        agrees, paths = eval_suite.opening_parity(
            {"monsters": [{"hp": 3, "power_attachments": []}]},
            {"monsters": [{"hp": 3,
                           "power_attachments": [{"power": "STRENGTH"}]}]})
        self.assertTrue(agrees)
        self.assertEqual(paths, [])

    def test_the_gate_is_not_fooled_by_a_deeper_difference(self) -> None:
        agrees, paths = eval_suite.opening_parity(
            {"piles": {"draw": [{"id": "STRIKE", "uid": 1}]}},
            {"piles": {"draw": [{"id": "STRIKE", "uid": 2}]}})
        self.assertFalse(agrees)
        self.assertEqual(paths, ["piles.draw[0].uid"])


class TheGateComparesTypesNotJustValues(unittest.TestCase):
    """#2790: `1 == True` in Python, and the gate may not believe it.

    The documents are the input to `canonical_json`, where `1` and `true` are
    different bytes and therefore different fights. A value-only compare let
    `fc4cf049784d8f31` TERROR_EEL_ELITE through with Rust emitting `int 1` for
    `monsters[0].shriek` against frozen Python's `bool True`, and the hybrid
    source substituted Rust's root on a fight whose per-action lockstep then
    diverged everywhere — the exact class this gate exists to catch.
    """

    def test_int_one_against_bool_true_is_a_mismatch_naming_both_types(self) -> None:
        agrees, paths = eval_suite.opening_parity(
            {"monsters": [{"hp": 150, "shriek": True}]},
            {"monsters": [{"hp": 150, "shriek": 1}]})
        self.assertFalse(agrees)
        self.assertEqual(paths, ["monsters[0].shriek (bool vs int)"])

    def test_int_against_float_is_a_mismatch_naming_both_types(self) -> None:
        agrees, paths = eval_suite.opening_parity(
            {"player": {"block": 1}}, {"player": {"block": 1.0}})
        self.assertFalse(agrees)
        self.assertEqual(paths, ["player.block (int vs float)"])

    def test_zero_against_false_is_a_mismatch(self) -> None:
        """The other half of the same Python identity, which elision makes live.

        `project_state`'s zero-default elision requires
        `type(value) is type(default)`, so a bool written as `0` is EMITTED
        where it should have elided — the `0`/`False` pair is as reachable as
        the `1`/`True` one.
        """
        agrees, paths = eval_suite.opening_parity(
            {"monsters": [{"shriek": False}]}, {"monsters": [{"shriek": 0}]})
        self.assertFalse(agrees)
        self.assertEqual(paths, ["monsters[0].shriek (bool vs int)"])

    def test_an_identical_pair_still_passes(self) -> None:
        document = {
            "schema": "sts-sim-canonical-v2",
            "player": {"hp": 68, "block": 0, "alive": True},
            "monsters": [{"hp": 150, "shriek": True, "move_log": ["SHRIEK"]}],
            "piles": {"draw": [{"id": "STRIKE", "uid": 0}]},
        }
        agrees, paths = eval_suite.opening_parity(
            json.loads(json.dumps(document)), json.loads(json.dumps(document)))
        self.assertTrue(agrees)
        self.assertEqual(paths, [])

    def test_the_type_pair_is_a_type_name_never_a_value(self) -> None:
        """Paths travel into PR bodies; `int`/`bool` name no content."""
        _, paths = eval_suite.opening_parity(
            {"player": {"name_like_field": "SECRET"}},
            {"player": {"name_like_field": 3}})
        self.assertEqual(paths, ["player.name_like_field (str vs int)"])
        self.assertNotIn("SECRET", paths[0])

    def test_the_decision_and_the_report_are_one_walk(self) -> None:
        """A pair the gate passes is a pair it had nothing to report about.

        Pinned as behaviour rather than by reading the source: the old gate
        decided with `==` and reported with a separate walk, which is how a
        decision and its explanation can drift apart.
        """
        for left, right in (
                ({"a": 1}, {"a": True}),
                ({"a": [1, 2]}, {"a": [1, 2]}),
                ({"a": {"b": 0}}, {"a": {"b": False}}),
                ({"a": None}, {"a": 0}),
        ):
            agrees, paths = eval_suite.opening_parity(left, right)
            self.assertEqual(agrees, not paths, (left, right, paths))
            self.assertEqual(
                agrees, eval_suite.type_strict_equal(left, right))


class TheAdvisoryTagIsTypeFaithfulToo(unittest.TestCase):
    """The same subtraction, asked through the digest rather than the walk.

    `mismatch_explained_by_rust_only_slots` compares
    `stripped_step_digest`s — a sha256 over `canonical_json`, which spells
    `true` and `1` differently — so it never had the `==` blindness. Pinned
    here so it cannot acquire it.
    """

    def test_an_int_bool_pair_survives_the_subtraction(self) -> None:
        stripped = eval_suite.strip_rust_only_slots
        left = stripped({"monsters": [{"shriek": True,
                                       "power_attachments": [{"p": 1}]}]})
        right = stripped({"monsters": [{"shriek": 1}]})
        self.assertEqual(left, right, "Python's == is the blindness itself")
        self.assertNotEqual(json.dumps(left, sort_keys=True),
                            json.dumps(right, sort_keys=True))
        self.assertFalse(eval_suite.type_strict_equal(left, right))


class TheRetiredAdvisoryVerdictStaysRetired(unittest.TestCase):
    """No `rust_only_provenance` verdict exists, deliberately.

    Naming one was the option Sean's 2026-09-22 decision record (#2751)
    REJECTED. Since #2999 the census has no Python-vs-Rust lockstep at all —
    its verdict is the capture's own native checkpoints — so the advisory
    tallies went with it; this keeps the rejected verdict from returning.
    """

    def test_the_lockstep_verdict_vocabulary_gains_no_new_member(self) -> None:
        source = (TOOLS_DIR / "eval_suite.py").read_text(encoding="utf-8")
        self.assertNotIn('verdict="rust_only_provenance"', source)
        self.assertNotIn('"rust_only_provenance"', source)


if __name__ == "__main__":
    unittest.main()

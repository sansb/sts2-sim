#!/usr/bin/env python3
"""Controls for the frozen Python registry and the frozen-oracle pins (#2827 D).

`generate_content.py --registry frozen` (what the `rust port` codegen
freshness step runs) replays the committed `data/python_registry.*.json` instead of
importing `combat_sim`. A byte-identical regeneration shows the replay agrees
with the committed tables today; these controls show it can FAIL — that a
replay which silently lost a fact, served a stale census, or accepted a hand
edit would go red rather than reproduce the right bytes by accident:

* the codec round-trips every value shape the registry uses, keeps `bool`
  apart from `int`, tuples apart from lists and sets apart from frozensets,
  and rebuilds one class object per recorded class so `type(v) is
  cs.AscensionTier` still holds;
* a value it cannot represent raises instead of being stringified;
* an attribute or census the recording never saw raises
  `FrozenRegistryMiss`, including a census whose inputs moved;
* a hand-edited snapshot is refused by its `self_sha256`;
* a resealed snapshot whose card program was mutated regenerates DIFFERENT
  tables, so the codegen freshness diff would catch it;
* `frozen_oracle_data.py --check` reports a mutated or missing frozen file.

Standard library only, no Python simulator. Run directly (see
`.github/workflows/rust-port.yml`).
"""

from __future__ import annotations

import collections
import contextlib
import dataclasses
import enum
import importlib.util
import io
import json
import pathlib
import shutil
import sys
import tempfile
import unittest
from fractions import Fraction

HERE = pathlib.Path(__file__).resolve()
TOOLS = HERE.parent
RUST = TOOLS.parent
if str(TOOLS) not in sys.path:
    sys.path.insert(0, str(TOOLS))

import frozen_oracle_data  # noqa: E402
import registry_snapshot as rs  # noqa: E402


def _generator():
    if "generate_content" in sys.modules:
        return sys.modules["generate_content"]
    spec = importlib.util.spec_from_file_location(
        "generate_content", TOOLS / "generate_content.py")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


@dataclasses.dataclass(frozen=True)
class Tier:
    gate: int
    at_or_above: int
    below: int


Pair = collections.namedtuple("Pair", "left right")


class Colour(enum.Enum):
    RED = 1
    BLUE = "b"


class Codec(unittest.TestCase):
    def roundtrip(self, value):
        encoder = rs.Encoder()
        encoded = json.loads(json.dumps(encoder.encode(value)))
        decoder = rs.Decoder(json.loads(json.dumps(encoder.classes)))
        return decoder, decoder.decode(encoded)

    def test_every_shape_survives_with_its_exact_type(self) -> None:
        value = {
            ("CARD", 0): (1, True, None, "s", 1.5, Fraction(2, 3)),
            "list": [1, (2, 3)],
            "set": {"a", "b"},
            "frozen": frozenset({("x", 1)}),
            "nested": {1: {False: []}},
        }
        _decoder, back = self.roundtrip(value)
        self.assertEqual(back, value)
        self.assertIs(type(back[("CARD", 0)]), tuple)
        self.assertIs(type(back[("CARD", 0)][1]), bool)
        self.assertIs(type(back["list"]), list)
        self.assertIs(type(back["list"][1]), tuple)
        self.assertIs(type(back["set"]), set)
        self.assertIs(type(back["frozen"]), frozenset)
        self.assertIs(type(back["nested"][1]), dict)
        self.assertIn(False, back["nested"][1])
        self.assertNotIn(0, {type(k) for k in back["nested"][1]})

    def test_classes_are_rebuilt_once_and_keep_identity(self) -> None:
        value = {"cls": Tier, "tiers": (Tier(9, 18, 16), Tier(8, 3, 2)),
                 "pair": Pair(1, (2,)), "colour": Colour.BLUE}
        _decoder, back = self.roundtrip(value)
        tier_cls = back["cls"]
        self.assertTrue(all(type(t) is tier_cls for t in back["tiers"]))
        self.assertEqual([(t.gate, t.at_or_above, t.below)
                          for t in back["tiers"]], [(9, 18, 16), (8, 3, 2)])
        self.assertEqual(dataclasses.replace(back["tiers"][0], below=1).below, 1)
        with self.assertRaises(dataclasses.FrozenInstanceError):
            back["tiers"][0].gate = 0
        self.assertEqual(back["pair"].right, (2,))
        self.assertEqual(back["colour"].name, "BLUE")
        self.assertEqual(back["colour"].value, "b")

    def test_an_unknown_value_raises_rather_than_stringifying(self) -> None:
        for value in (object(), len, lambda: 0, b"bytes"):
            with self.subTest(value=value), self.assertRaises(
                    rs.SnapshotCodecError):
                rs.Encoder().encode(value)


class TheCommittedSnapshot(unittest.TestCase):
    def test_it_loads_and_carries_the_generators_reads(self) -> None:
        registry = rs.load_snapshot()
        self.assertIn(("BASH", 0), registry.CARDS)
        self.assertIn("LOOPS", dir(registry))
        self.assertTrue(dataclasses.is_dataclass(registry.CARDS[("BASH", 0)]))

    def test_an_unrecorded_attribute_is_refused_by_name(self) -> None:
        registry = rs.load_snapshot()
        with self.assertRaises(rs.FrozenRegistryMiss) as raised:
            registry.apply_action  # noqa: B018 - the access is the test
        self.assertIn("apply_action", str(raised.exception))

    def test_a_census_whose_inputs_moved_is_refused(self) -> None:
        gc = _generator()
        registry = gc.load_registry()
        # The generator's own axes produced the recorded key; any other axes
        # must not be answered from the recording.
        with self.assertRaises(rs.FrozenRegistryMiss):
            gc.multiplayer_gate_contract(registry, {"CardId": ["BASH"]})

    def test_a_hand_edit_is_refused(self) -> None:
        text = rs.SNAPSHOT_PATH.read_text()
        edited = text.replace('"cost":2', '"cost":3', 1) if '"cost":2' in text \
            else text.replace("BASH", "BASI", 1)
        self.assertNotEqual(edited, text)
        with tempfile.TemporaryDirectory() as tmp:
            path = pathlib.Path(tmp) / rs.SNAPSHOT_PATH.name
            path.write_text(edited)
            with self.assertRaises(rs.SnapshotCodecError):
                rs.load_snapshot(path)

    def test_a_resealed_mutation_changes_the_generated_tables(self) -> None:
        payload = json.loads(rs.SNAPSHOT_PATH.read_text())
        cards = payload["attributes"]["CARDS"]["__dict__"]
        for key, card in cards:
            if key == ["BASH", 0]:
                steps = card["v"][2]
                self.assertEqual(steps[0], ["attack", 8, 1])
                steps[0] = ["attack", 9, 1]
                break
        else:
            self.fail("BASH is not in the recorded CARDS")
        gc = _generator()
        with tempfile.TemporaryDirectory() as tmp:
            snapshot = pathlib.Path(tmp) / rs.SNAPSHOT_PATH.name
            snapshot.write_text(rs.snapshot_text(payload))
            out = pathlib.Path(tmp) / "out"
            original = rs.SNAPSHOT_PATH
            rs.SNAPSHOT_PATH = snapshot
            # An earlier test in the session may have left entries behind.
            gc.REFUSALS.clear()
            gc.DYNAMIC_SITES.clear()
            try:
                with contextlib.redirect_stdout(io.StringIO()), \
                        contextlib.redirect_stderr(io.StringIO()):
                    gc.main(["--source", "manifest", "--registry", "frozen",
                             "--out-dir", str(out)])
            finally:
                rs.SNAPSHOT_PATH = original
                gc.REFUSALS.clear()
                gc.DYNAMIC_SITES.clear()
            generated = (out / "content_tables.rs").read_text()
        committed = (RUST / "src" / "content_tables.rs").read_text()
        changed = [(a, b) for a, b in zip(generated.splitlines(),
                                           committed.splitlines()) if a != b]
        self.assertEqual(len(generated.splitlines()),
                         len(committed.splitlines()))
        self.assertTrue(changed, "a mutated card program regenerated the "
                        "committed tables unchanged")
        # Exactly Bash's attack step moved, and nothing else did.
        self.assertEqual(changed, [(
            "            Step { kind: StepKind::Attack, args: &[Arg::I(9), Arg::I(1)] },",
            "            Step { kind: StepKind::Attack, args: &[Arg::I(8), Arg::I(1)] },",
        )])


class FrozenOracleData(unittest.TestCase):
    def test_the_committed_pins_hold(self) -> None:
        self.assertEqual(frozen_oracle_data.check(), [])

    def test_a_mutated_or_missing_file_is_reported(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = pathlib.Path(tmp)
            for relative in frozen_oracle_data.FROZEN:
                target = root / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(RUST / relative, target)
            mutated, missing = sorted(frozen_oracle_data.FROZEN)[:2]
            with (root / mutated).open("a") as handle:
                handle.write(" ")
            (root / missing).unlink()
            original = frozen_oracle_data.RUST_DIR
            frozen_oracle_data.RUST_DIR = root
            try:
                findings = frozen_oracle_data.check()
            finally:
                frozen_oracle_data.RUST_DIR = original
        self.assertEqual(len(findings), 2, findings)
        self.assertTrue(any(f.startswith(f"{mutated}: sha256") for f in findings))
        self.assertTrue(any(f.startswith(f"{missing}: missing") for f in findings))


if __name__ == "__main__":
    unittest.main()

"""Focused regressions for the lightweight card-constructor census (#674)."""
import json
from pathlib import Path
from types import SimpleNamespace

import pytest

from tools.census_cards import (
    card_model_ctor_constants, constructor_cost_type)


HERE = Path(__file__).parent


@pytest.mark.parametrize(("constants", "expected"), [
    ([2, 1, 4, 2, 1], (2, "attack")),
    ([1, 2, 4, 1, 1], (1, "skill")),
    ([-1, 6, 7, 1, 1], (-1, "quest")),
])
def test_constructor_cost_type_uses_one_exact_base_argument_slice(
        constants, expected):
    assert constructor_cost_type(constants) == expected


def _insn(opcode, operand=None):
    return SimpleNamespace(opcode=SimpleNamespace(name=opcode), operand=operand)


def test_card_model_call_boundary_skips_a_leading_mutable_field_constant():
    instructions = [
        _insn("ldarg.0"),
        _insn("ldc.i4.m1"),          # SpoilsMap._spoilsActIndex = -1
        _insn("stfld", "_spoilsActIndex"),
        _insn("ldarg.0"),
        _insn("ldc.i4.m1"),          # cost -1
        _insn("ldc.i4.6"),           # CardType.Quest
        _insn("ldc.i4.7"),           # rarity
        _insn("ldc.i4.1"),           # target
        _insn("ldc.i4.1"),           # pool
        _insn("call", "CardModel::.ctor"),
        _insn("ldc.i4.s", 42),       # unrelated post-base constant
    ]
    token_name = lambda operand: operand
    constants = card_model_ctor_constants(instructions, token_name)
    assert constants == [-1, 6, 7, 1, 1]
    assert constructor_cost_type(constants) == (-1, "quest")

    # The defect this guard replaces: ignoring the base-call boundary makes
    # the leading mutable field look like (cost, type) == (-1, -1).
    full_body = [
        value for value in (
            -1 if insn.opcode.name == "ldc.i4.m1"
            else (int(insn.opcode.name[-1])
                  if insn.opcode.name.startswith("ldc.i4.")
                  and insn.opcode.name[-1].isdigit()
                  else insn.operand
                  if insn.opcode.name in ("ldc.i4", "ldc.i4.s") else None)
            for insn in instructions)
        if isinstance(value, int)
    ]
    assert full_body[:2] == [-1, -1]


@pytest.mark.parametrize("calls", ([], ["CardModel::.ctor"] * 2))
def test_card_model_call_boundary_refuses_missing_or_duplicate_base_calls(calls):
    instructions = [_insn("ldc.i4.0") for _ in range(5)]
    instructions.extend(_insn("call", call) for call in calls)
    with pytest.raises(ValueError, match="CardModel ctor call shape"):
        card_model_ctor_constants(instructions, lambda operand: operand)


def test_checked_in_census_has_live_scythe_and_genetic_algorithm_metadata():
    census = json.loads((HERE / "cards_census.json").read_text())
    assert (census["CARD.THE_SCYTHE"]["cost"],
            census["CARD.THE_SCYTHE"]["type"]) == (2, "attack")
    assert (census["CARD.GENETIC_ALGORITHM"]["cost"],
            census["CARD.GENETIC_ALGORITHM"]["type"]) == (1, "skill")
    assert (census["CARD.SPOILS_MAP"]["cost"],
            census["CARD.SPOILS_MAP"]["type"]) == (-1, "quest")

    template_census = json.loads((HERE / "template_census.json").read_text())
    assert (template_census["CARD.SPOILS_MAP"]["cost"],
            template_census["CARD.SPOILS_MAP"]["type"]) == (-1, "quest")
    for filename in ("card_templates_raw.json", "card_templates.json"):
        templates = json.loads((HERE / filename).read_text())
        spoils = templates["cards"]["CARD.SPOILS_MAP"]
        assert (spoils["cost"], spoils["type"]) == (-1, "quest")
        assert "CARD.SPOILS_MAP" not in templates["refused"]

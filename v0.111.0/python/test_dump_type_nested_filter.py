"""Regression test for #3344: dump_type.py's nested-type filter.

``sim/v0.111.0/python/tools/dump_type.py`` accepts queries of the form
``Enclosing/Nested`` to disambiguate compiler-generated nested types (async
state machines, iterators) whose short names — ``<OnPlay>d__3`` and the like
— are duplicated across many unrelated enclosing classes. The bug: the old
filter matched a qualified query's *nested name* against every TypeDef in the
assembly, so ``InfernalBlade/<OnPlay>d__3`` printed every class that happened
to have an ``<OnPlay>d__3`` nested type, not just the one nested under
``InfernalBlade``. #2647/#3252/#3336 all hit this independently; one worker
was handed ``AsleepPower``'s body while asking for a different card's.

The fix, in ``resolve_nested_targets()``, walks the NestedClass table to
resolve the full enclosing chain and returns 1-based TypeDef row ids rather
than bare names, so the final match is by identity, not by a name that
several unrelated types can share.

dnfile/dncil (needed to read the real DLL) are only installed under
``/usr/local/bin/python3.12`` (see the tool's own guarded import and the
repo CLAUDE.md's IL-tools note) — not under the interpreter this fast-gate
suite runs under. So this test exercises ``resolve_nested_targets()``
directly against a synthetic NestedClass table shaped like the real bug
report: two different enclosing classes each contributing a same-named
nested type.
"""

from __future__ import annotations

import importlib.util
import pathlib
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
TOOL = ROOT / "v0.111.0" / "python" / "tools" / "dump_type.py"


def _module():
    name = "dump_type_3344"
    if name in sys.modules:
        return sys.modules[name]
    spec = importlib.util.spec_from_file_location(name, TOOL)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


# A synthetic TypeDef table (1-based rid == list index + 1), modeled on the
# issue's own example: two different cards each compile an
# ``<OnPlay>d__3`` nested state machine.
#
#   rid 1: InfernalBlade            (enclosing)
#   rid 2: InfernalBlade/<OnPlay>d__3   (nested, target)
#   rid 3: SomeOtherCard            (enclosing)
#   rid 4: SomeOtherCard/<OnPlay>d__3   (nested, same short name, decoy)
#   rid 5: SlumberPower                          (enclosing, unrelated query)
#   rid 6: SlumberPower/<AfterDamageReceived>d__4 (nested, unrelated query)
TYPE_NAMES = [
    "InfernalBlade",
    "<OnPlay>d__3",
    "SomeOtherCard",
    "<OnPlay>d__3",
    "SlumberPower",
    "<AfterDamageReceived>d__4",
]
NESTED_OF = {
    2: 1,  # <OnPlay>d__3 (rid 2) nested under InfernalBlade (rid 1)
    4: 3,  # <OnPlay>d__3 (rid 4) nested under SomeOtherCard (rid 3)
    6: 5,  # <AfterDamageReceived>d__4 (rid 6) nested under SlumberPower (rid 5)
}


def test_qualified_query_matches_only_the_named_enclosing_class():
    """InfernalBlade/<OnPlay>d__3 must resolve to rid 2 only, never rid 4."""
    resolve_nested_targets = _module().resolve_nested_targets

    remaining, qualified = resolve_nested_targets(
        {"InfernalBlade/<OnPlay>d__3"}, TYPE_NAMES, NESTED_OF
    )

    assert remaining == set()
    assert qualified == {2}


def test_two_same_named_nested_types_under_different_parents_stay_disjoint():
    """Querying both cards' same-named nested type must not cross-match."""
    resolve_nested_targets = _module().resolve_nested_targets

    remaining, qualified_a = resolve_nested_targets(
        {"InfernalBlade/<OnPlay>d__3"}, TYPE_NAMES, NESTED_OF
    )
    _, qualified_b = resolve_nested_targets(
        {"SomeOtherCard/<OnPlay>d__3"}, TYPE_NAMES, NESTED_OF
    )
    _, qualified_c = resolve_nested_targets(
        {"SlumberPower/<AfterDamageReceived>d__4"}, TYPE_NAMES, NESTED_OF
    )

    assert qualified_a == {2}
    assert qualified_b == {4}
    assert qualified_c == {6}
    # The historical bug: matching by nested name alone would have put both
    # same-named nested types in the result for either query.
    assert qualified_a.isdisjoint(qualified_b)


def test_bare_enclosing_query_lists_nested_types_without_qualifying_them():
    """'Enclosing/' with no nested name only lists — it must not print/queue
    a type outside that one enclosing class, and it does not add anything to
    the qualified set (nothing to disambiguate)."""
    resolve_nested_targets = _module().resolve_nested_targets

    listed = []
    remaining, qualified = resolve_nested_targets(
        {"InfernalBlade/"}, TYPE_NAMES, NESTED_OF, out=listed.append
    )

    assert remaining == set()
    assert qualified == set()
    assert listed == ["InfernalBlade/<OnPlay>d__3"]


def test_non_nested_query_is_left_untouched():
    """A plain, non-'/' target must pass through unchanged (behaviour for
    ordinary type-name queries is unaffected by the nested-type fix)."""
    resolve_nested_targets = _module().resolve_nested_targets

    remaining, qualified = resolve_nested_targets(
        {"InfernalBlade", "SlumberPower"}, TYPE_NAMES, NESTED_OF
    )

    assert remaining == {"InfernalBlade", "SlumberPower"}
    assert qualified == set()


def test_mixed_plain_and_qualified_targets():
    """A plain name alongside a qualified query: the plain name survives in
    `remaining` (matched by name in the caller's own final loop) and the
    qualified query resolves only its own rid."""
    resolve_nested_targets = _module().resolve_nested_targets

    remaining, qualified = resolve_nested_targets(
        {"SomeOtherCard", "InfernalBlade/<OnPlay>d__3"}, TYPE_NAMES, NESTED_OF
    )

    assert remaining == {"SomeOtherCard"}
    assert qualified == {2}

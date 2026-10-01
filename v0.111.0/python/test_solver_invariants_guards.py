"""Every file a SOLVER_INVARIANTS.md Guards field names must exist (#2772).

The registry names its executable guards per invariant, and until #2762 one
of those names was a phantom: I4's Guards field pointed at Python
`test_rust_exact_solve.py`, a file that never existed anywhere in the tree
(it entered in 5ff54b3d and survived until two sessions independently found
it). A guard registry naming a nonexistent file is one a reader cannot use,
and nothing was checking. This module checks.

**Scope.** Every paragraph whose first line is the bold field `**Guard:**`
or `**Guards:**` in `SOLVER_INVARIANTS.md`, from that line to the next blank
line (the field may start mid-paragraph, as I5's does, and may wrap over
several lines — the paragraph is the unit, not the line). The registry's
preamble sentence "Guards live in `test_combat_sim.py` unless noted" is
prose, not a Guards field, and is deliberately out of scope; so is every
other backticked filename elsewhere in the document.

**Resolution rule.** Inside a Guards paragraph, each backticked span is
classified by its suffix and by the nearest preceding standalone language
qualifier (the word ``Rust`` or ``Python``; the compound "Rust/Python" is not
a qualifier):

* ``*.py`` — a Python file claim. Resolves if the basename exists anywhere
  under `sim/v0.111.0/python/` or under `sim/v0.111.0/engine/tools/`
  (no Guards field names a file in the latter today; the rule covers it
  because the tree has Python there).
* ``*.rs`` — a Rust file claim. Resolves if the basename exists anywhere
  under `sim/v0.111.0/engine/`.
* a bare ``snake_case`` identifier under a ``Rust`` qualifier — a Rust
  module claim, as I4's `exact_dfs` / `exact_solve_v1` /
  `solo_v1_authority` are written. Resolves to
  `engine/src/**/<name>.rs`, `engine/src/**/<name>/mod.rs`, or
  `engine/tests/<name>.rs`.
* anything else is **not a file claim and is skipped**: bare identifiers are
  otherwise ambiguous between a test function
  (`test_truncation_matches_decimal_casts`), a module attribute
  (`shuffle_counter_caveats`), an attribute path (`Card.heal`), and a module
  (`rust_exact_solve`, which does happen to resolve). Unbackticked prose
  ("Mawler-admission tests", "the post-submit Rust/Python trajectory
  certifications") makes no file claim at all.

The rule is deliberately not widened to make a name pass: a file-shaped name
that resolves to nothing is a finding, not a reason to relax the classifier.

This module only READS the registry, so no sha256 attestation of it moves
(`attestations.json`, `test_attestations.py`).
"""

from pathlib import Path
import re

import pytest


HERE = Path(__file__).parent
REGISTRY = HERE / "SOLVER_INVARIANTS.md"

RUST_ROOT = HERE.parent / "engine"
PY_ROOTS = (HERE, RUST_ROOT / "tools")

_FIELD_LINE = re.compile(r"^\*\*Guards?:\*\*")
_BACKTICKED = re.compile(r"`([^`]+)`")
# Standalone language qualifier. "Rust/Python trajectory certifications" is
# prose about both and qualifies nothing, so slashes exclude a match.
_QUALIFIER = re.compile(r"(?<![\w/])(Rust|Python)(?![\w/])")
_BARE_IDENT = re.compile(r"[a-z][a-z0-9_]*")


def _guards_paragraphs(text):
    """The Guards field paragraphs, each as one whitespace-joined string."""
    lines = text.splitlines()
    paragraphs = []
    index = 0
    while index < len(lines):
        if _FIELD_LINE.match(lines[index]):
            block = []
            while (index < len(lines) and lines[index].strip()
                   and not lines[index].startswith("#")):
                block.append(lines[index].strip())
                index += 1
            paragraphs.append(" ".join(block))
        else:
            index += 1
    return paragraphs


def _file_claims(paragraph):
    """[(name, kind)] file claims in one Guards paragraph, in order."""
    claims = []
    qualifier = None
    cursor = 0
    for match in _BACKTICKED.finditer(paragraph):
        between = paragraph[cursor:match.start()]
        qualifiers = _QUALIFIER.findall(between)
        if qualifiers:
            qualifier = qualifiers[-1]
        cursor = match.end()
        name = " ".join(match.group(1).split())
        if name.endswith(".py"):
            claims.append((name, "python-file"))
        elif name.endswith(".rs"):
            claims.append((name, "rust-file"))
        elif qualifier == "Rust" and _BARE_IDENT.fullmatch(name):
            claims.append((name, "rust-module"))
    return claims


def _resolutions(name, kind):
    if kind == "python-file":
        found = []
        for root in PY_ROOTS:
            found.extend(sorted(root.rglob(name)))
        return found
    if kind == "rust-file":
        return sorted(RUST_ROOT.rglob(name))
    if kind == "rust-module":
        found = sorted((RUST_ROOT / "src").rglob(f"{name}.rs"))
        found += sorted((RUST_ROOT / "src").rglob(f"{name}/mod.rs"))
        candidate = RUST_ROOT / "tests" / f"{name}.rs"
        if candidate.is_file():
            found.append(candidate)
        return sorted(set(found))
    raise NotImplementedError(f"unclassified guard claim kind {kind!r}")


def _registry_claims():
    text = REGISTRY.read_text()
    claims = []
    for paragraph in _guards_paragraphs(text):
        claims.extend(_file_claims(paragraph))
    return claims


def test_every_guarded_file_exists():
    """The point of the module: no Guards field names a phantom file."""
    missing = []
    for name, kind in _registry_claims():
        if not _resolutions(name, kind):
            missing.append(f"{name} ({kind})")
    assert not missing, (
        "SOLVER_INVARIANTS.md Guards fields name files that do not exist: "
        + ", ".join(missing)
        + " — fix the registry entry (or the missing file); do not relax "
        "the resolution rule in this module's docstring to make it pass "
        "(#2762, #2772)")


def test_the_scan_is_not_vacuous():
    """A registry reformat must redden this module, not silently skip it.

    Every guard here is derived from the file rather than hard-coded, so
    the numbers move with the registry; what they pin is that the field
    regex, the paragraph walk and each branch of the classifier still
    match real text.
    """
    text = REGISTRY.read_text()
    field_lines = [line for line in text.splitlines()
                   if _FIELD_LINE.match(line)]
    paragraphs = _guards_paragraphs(text)
    assert len(paragraphs) == len(field_lines) >= 9, (
        len(paragraphs), len(field_lines))

    claims = _registry_claims()
    kinds = {kind for _, kind in claims}
    assert "python-file" in kinds, claims
    assert "rust-module" in kinds, claims
    # 12 until #2827 item F deleted five guarded Python modules with the
    # simulator; their Guards spans lost their backticks (not file claims).
    assert len(claims) >= 7, claims


def test_a_phantom_python_guard_is_detected():
    """The mutation control, kept as a test rather than a one-off edit.

    This is the pre-#2763 I4 field verbatim; `test_rust_exact_solve.py`
    never existed, and the classifier must call it out rather than skip it.
    """
    pre_2763_i4 = (
        "**Guards:** Rust `exact_dfs`, `exact_solve_v1`, "
        "`solo_v1_authority`, and Mawler-admission tests; Python "
        "`test_rust_exact_solve.py`, `test_exact_solve_corpus.py`, "
        "`test_python_exact_search_retired.py`, and the post-submit "
        "Rust/Python trajectory certifications."
    )
    claims = _file_claims(pre_2763_i4)
    assert ("test_rust_exact_solve.py", "python-file") in claims, claims
    unresolved = [name for name, kind in claims if not _resolutions(name, kind)]
    # `test_exact_solve_corpus.py` did exist until #2827 item F deleted it
    # with the Python simulator; it now resolves nowhere either.
    assert unresolved == [
        "test_rust_exact_solve.py", "test_exact_solve_corpus.py"], unresolved


def test_unbackticked_prose_makes_no_file_claim():
    """Prose naming no file is skipped, not guessed into a path."""
    assert _file_claims("**Guard:** structural (by construction).") == []
    assert _file_claims(
        "**Guard:** the refusal branches in `start_combat`; scans in "
        "`coverage_report.py`") == [("coverage_report.py", "python-file")]


def test_qualifier_does_not_leak_across_languages():
    """A bare identifier under `Python` is ambiguous and stays skipped."""
    claims = _file_claims(
        "**Guards:** Rust `exact_dfs`; Python `test_review_summary.py` "
        "(the `rust_exact_solve` wire lives there)")
    assert claims == [
        ("exact_dfs", "rust-module"),
        ("test_review_summary.py", "python-file"),
    ]


def test_unknown_claim_kind_fails_closed():
    with pytest.raises(NotImplementedError, match="unclassified"):
        _resolutions("whatever", "prose")

"""Invariant-walk filenames have a strict grammar and unique reservations.

`NN` is **optional and deprecated** (#1584). It was a shared counter that
concurrent lanes allocated optimistically, so two PRs writing a walk on the same
day collided by construction — and the loser was whichever merged second,
discovering it only after a ~10 minute solver-gate run. Five concurrent unit
lanes made that routine rather than exceptional.

New walks are `YYYY-MM-DD-slug.md`. The slug already carries the issue number,
so two lanes cannot collide without genuinely writing the same walk, and
uniqueness is enforced by the filesystem rather than by a counter. The 523
legacy numbered walks stay valid and keep their reservations checked, so the
historical record and every citation of it are untouched.

The slug must begin with a letter. That is what keeps `2026-08-01-007-alpha.md`
malformed rather than silently reinterpreting `007-alpha` as a slug once the
number became optional.
"""

from collections import defaultdict
from datetime import date as calendar_date
from pathlib import Path
import re

import pytest


HERE = Path(__file__).parent
WALKS = HERE.parents[2] / "solver" / "invariant-walks"
_WALK_NAME = re.compile(
    r"^(?P<date>\d{4}-\d{2}-\d{2})-"
    r"(?:(?P<number>\d{2}|[1-9]\d{2})-)?"
    r"(?P<slug>[a-z][a-z0-9]*(?:-[a-z0-9]+)*)\.md$"
)


def _assert_unique_walk_prefixes(paths):
    groups = defaultdict(list)
    malformed = []
    for name in sorted(path.name for path in paths):
        match = _WALK_NAME.fullmatch(name)
        if match is None:
            malformed.append(name)
            continue
        try:
            calendar_date.fromisoformat(match.group("date"))
        except ValueError:
            malformed.append(name)
            continue
        number = match.group("number")
        if number is None:
            # Numberless walks reserve nothing: the filename itself is the
            # identity, and the filesystem already forbids two of them.
            continue
        reservation = (match.group("date"), int(number))
        groups[reservation].append(name)

    if malformed:
        pytest.fail(
            "malformed invariant-walk filenames:\n" + "\n".join(malformed),
            pytrace=False,
        )

    collisions = tuple(
        (f"{date}-{number:02d}", tuple(sorted(names)))
        for (date, number), names in sorted(groups.items())
        if len(names) > 1
    )
    if collisions:
        # Name the remedy, not just the fault: whoever hits this is mid-merge
        # and has already paid a full gate cycle to find out.
        taken = {(date, number) for (date, number) in groups}
        details = []
        for prefix, names in collisions:
            date = prefix[:10]
            free = next(
                n for n in range(1, 1000) if (date, n) not in taken
            )
            details.append(
                f"{prefix}: {', '.join(names)}\n"
                f"    -> drop the number (preferred, #1584): "
                f"{date}-{names[0].split('-', 4)[-1]}\n"
                f"    -> or renumber to {date}-{free:02d}-..."
            )
        pytest.fail(
            "duplicate invariant-walk prefixes:\n" + "\n".join(details),
            pytrace=False,
        )


def test_invariant_walk_prefixes_are_unique():
    _assert_unique_walk_prefixes(WALKS.glob("*.md"))


def test_invariant_walk_prefix_guard_accepts_distinct_numbers():
    _assert_unique_walk_prefixes((
        Path("2026-08-01-01-alpha.md"),
        Path("2026-08-01-02-beta.md"),
        Path("2026-08-02-01-gamma.md"),
        Path("2026-08-02-100-legacy-epoch.md"),
        Path("2024-02-29-01-leap-day.md"),
    ))


@pytest.mark.parametrize(
    "name",
    (
        "not-an-invariant-walk.md",
        "2026-08-01-1-alpha.md",
        "2026-08-01-007-alpha.md",
        "2026-08-01-01-.md",
        "2026-13-01-01-invalid-month.md",
        "2026-04-31-01-invalid-day.md",
        "2025-02-29-01-non-leap-day.md",
        "2024-02-30-01-invalid-february-day.md",
        "0000-01-01-01-year-zero.md",
    ),
)
def test_invariant_walk_prefix_guard_rejects_malformed_names(name):
    with pytest.raises(pytest.fail.Exception) as exc_info:
        _assert_unique_walk_prefixes((Path(name),))
    assert str(exc_info.value) == f"malformed invariant-walk filenames:\n{name}"


def test_invariant_walk_prefix_guard_does_not_skip_malformed_names():
    paths = (
        Path("2026-08-01-01-alpha.md"),
        Path("not-an-invariant-walk.md"),
    )
    with pytest.raises(pytest.fail.Exception) as exc_info:
        _assert_unique_walk_prefixes(paths)
    assert str(exc_info.value) == (
        "malformed invariant-walk filenames:\n"
        "not-an-invariant-walk.md"
    )


def test_invariant_walk_prefix_guard_sorts_malformed_date_diagnostics():
    paths = (
        Path("2026-13-01-01-invalid-month.md"),
        Path("2026-08-01-01-valid.md"),
        Path("0000-01-01-01-year-zero.md"),
        Path("2025-02-29-01-non-leap-day.md"),
    )
    with pytest.raises(pytest.fail.Exception) as exc_info:
        _assert_unique_walk_prefixes(paths)
    assert str(exc_info.value) == (
        "malformed invariant-walk filenames:\n"
        "0000-01-01-01-year-zero.md\n"
        "2025-02-29-01-non-leap-day.md\n"
        "2026-13-01-01-invalid-month.md"
    )


def test_invariant_walk_prefix_guard_reports_collisions_deterministically():
    paths = (
        Path("2026-08-02-03-zeta.md"),
        Path("2026-08-01-02-zeta.md"),
        Path("2026-08-02-03-alpha.md"),
        Path("2026-08-01-02-alpha.md"),
    )
    with pytest.raises(pytest.fail.Exception) as exc_info:
        _assert_unique_walk_prefixes(paths)
    assert str(exc_info.value) == (
        "duplicate invariant-walk prefixes:\n"
        "2026-08-01-02: 2026-08-01-02-alpha.md, "
        "2026-08-01-02-zeta.md\n"
        "    -> drop the number (preferred, #1584): 2026-08-01-alpha.md\n"
        "    -> or renumber to 2026-08-01-01-...\n"
        "2026-08-02-03: 2026-08-02-03-alpha.md, "
        "2026-08-02-03-zeta.md\n"
        "    -> drop the number (preferred, #1584): 2026-08-02-alpha.md\n"
        "    -> or renumber to 2026-08-02-01-..."
    )


def test_numberless_walks_are_accepted_and_reserve_nothing():
    """The #1584 shape: two lanes, same day, no counter to collide on."""
    _assert_unique_walk_prefixes((
        Path("2026-08-25-issue1560-frog-axebot.md"),
        Path("2026-08-25-issue1613-multiplayer-only-canplay.md"),
        Path("2026-08-25-issue1560-two-tailed-rats.md"),
    ))


def test_numberless_and_legacy_numbered_walks_coexist():
    """523 legacy walks keep their reservations; new ones need none."""
    _assert_unique_walk_prefixes((
        Path("2026-08-25-154-issue1613-legacy.md"),
        Path("2026-08-25-155-issue1560-legacy.md"),
        Path("2026-08-25-issue1584-walk-numbers.md"),
        Path("2026-08-25-issue1548-fuzz-retirement.md"),
    ))


def test_a_numberless_walk_does_not_collide_with_a_numbered_one():
    """`2026-08-25-155-x.md` and `2026-08-25-issue155-y.md` are distinct."""
    _assert_unique_walk_prefixes((
        Path("2026-08-25-155-alpha.md"),
        Path("2026-08-25-issue155-beta.md"),
    ))


@pytest.mark.parametrize(
    "name",
    (
        # Still malformed with the number optional: a slug may not open with
        # digits, or `007-alpha` would quietly become a legal slug and the
        # grammar would have loosened as a side effect of #1584.
        "2026-08-01-1-alpha.md",
        "2026-08-01-007-alpha.md",
        "2026-08-01-0-alpha.md",
        "2026-08-01-42alpha.md",
    ),
)
def test_a_slug_may_not_begin_with_a_digit(name):
    with pytest.raises(pytest.fail.Exception) as exc_info:
        _assert_unique_walk_prefixes((Path(name),))
    assert str(exc_info.value) == f"malformed invariant-walk filenames:\n{name}"

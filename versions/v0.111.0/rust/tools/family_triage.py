#!/usr/bin/env python3
"""Derive Rust port family triage state from the checked-in source tree.

The generated family modules have two independent declarations of their
surface: ``mod.rs::FAMILY_OF`` owns every kind, while each family module's
``IMPLEMENTED`` slice plus its exact ``*KindNotModeled`` bodies describe the
current disposition.  This tool refuses to report a census unless those two
views partition every kind exactly once.  It also refuses if an implemented
kind remains directly named by its own ``*KindNotModeled`` marker in a real
family body.  That structural guarantee is not a claim that a runtime path was
exercised.

Selected escalated stubs additionally carry contiguous ``ESCALATED-ON`` doc
markers.  A separate required-stub registry makes their presence load-bearing,
and typed predicates are checked against the tokenized ``PowerId`` enum and
the exact player/monster admission allowlist named by the marker.  A predicate
turning false means the recorded escalation must be reassessed; it does not by
itself claim that the whole stub is now portable.

With ``--github-labels`` the open/closed wave issues are loaded through the
authenticated ``gh`` CLI.  ``--github-fixture`` accepts the same JSON array for
offline and mutation testing.  Label drift is diagnostic and makes the command
exit 1; source/fixture shape errors exit 2.
"""

from __future__ import annotations

import argparse
import dataclasses
import json
import pathlib
import re
import subprocess
import sys
import tempfile
from collections.abc import Iterable, Sequence


HERE = pathlib.Path(__file__).resolve()
DEFAULT_SRC = HERE.parents[1] / "src"
BOILERPLATE = "dispatch; the kind is read in"
ESCALATION_MARKER = "/// ESCALATED-ON: "
POWER_ALLOWLISTS = frozenset({"IMPLEMENTED_POWERS", "IMPLEMENTED_PLAYER_POWERS"})
REQUIRED_ESCALATION_MARKERS: frozenset[tuple[str, str]] = frozenset(
    {
        ("steps/ironclad_rare", "TankExact"),
        ("steps/necrobinder_uncommon", "SoulboundExact"),
        ("steps/neutral", "TagTeamExact"),
        ("steps/silent_rare", "FlankingExact"),
        ("steps/silent_uncommon", "ConcoctExact"),
        ("steps/templates", "Underworld"),
    }
)
WAVE_TITLE = re.compile(
    r"^Rust port wave: (?P<group>steps|moves)/(?P<family>[a-z0-9_]+)(?:\s|$)"
)


class TriageError(RuntimeError):
    """A source declaration or GitHub fixture is not safe to classify."""


@dataclasses.dataclass(frozen=True)
class Token:
    text: str
    kind: str
    start: int
    line: int


@dataclasses.dataclass(frozen=True)
class EscalationPredicate:
    kind: str
    power_variant: str
    allowlist: str | None = None

    def render(self) -> str:
        if self.kind == "power-variant-absent":
            return f"power-variant-absent(PowerId::{self.power_variant})"
        if self.kind == "power-not-admitted" and self.allowlist is not None:
            return (
                f"power-not-admitted({self.allowlist}, "
                f"PowerId::{self.power_variant})"
            )
        raise TriageError(f"unknown escalation predicate kind: {self.kind!r}")


@dataclasses.dataclass(frozen=True)
class Stub:
    kind: str
    function: str
    line: int
    disposition: str
    escalated_on: tuple[EscalationPredicate, ...]


@dataclasses.dataclass(frozen=True)
class FamilyBody:
    function: str
    line: int
    tokens: tuple[Token, ...]
    docs: str


@dataclasses.dataclass(frozen=True)
class Family:
    name: str
    implemented: tuple[str, ...]
    untriaged: tuple[Stub, ...]
    escalated: tuple[Stub, ...]

    @property
    def state(self) -> str:
        if self.untriaged:
            return "POOL"
        if self.escalated:
            return "engine-blocked"
        return "COMPLETE"


@dataclasses.dataclass(frozen=True)
class Issue:
    number: int
    title: str
    state: str
    labels: frozenset[str]
    url: str


@dataclasses.dataclass(frozen=True)
class Drift:
    family: str
    issue: int | None
    message: str


@dataclasses.dataclass(frozen=True)
class LabelAudit:
    drifts: tuple[Drift, ...]
    closed_history: tuple[Drift, ...]


def _raw_string(source: str, start: int) -> tuple[int, str] | None:
    """Return the end/value of a Rust raw string beginning at ``start``."""
    match = re.match(r'r(#{0,255})"', source[start:])
    if not match:
        return None
    hashes = match.group(1)
    content_start = start + match.end()
    terminator = '"' + hashes
    end = source.find(terminator, content_start)
    if end < 0:
        raise TriageError("unterminated Rust raw string")
    return end + len(terminator), source[content_start:end]


def lex_rust(source: str) -> list[Token]:
    """Lex the small Rust surface needed here, ignoring comments and literals.

    Positions and line numbers are retained so a function can be joined back
    to its immediately preceding ``///`` lines.  Nested block comments and raw
    strings are handled because generated files may legitimately contain both.
    """
    tokens: list[Token] = []
    index = 0
    line = 1
    length = len(source)
    punctuation = set("[](){}:,;=&!<>+-*/.%#|")
    while index < length:
        char = source[index]
        if char.isspace():
            line += char == "\n"
            index += 1
            continue
        if source.startswith("//", index):
            end = source.find("\n", index)
            index = length if end < 0 else end
            continue
        if source.startswith("/*", index):
            start_line = line
            depth = 1
            cursor = index + 2
            while cursor < length and depth:
                if source.startswith("/*", cursor):
                    depth += 1
                    cursor += 2
                elif source.startswith("*/", cursor):
                    depth -= 1
                    cursor += 2
                else:
                    line += source[cursor] == "\n"
                    cursor += 1
            if depth:
                raise TriageError(
                    f"unterminated Rust block comment starting at line {start_line}"
                )
            index = cursor
            continue
        raw = _raw_string(source, index) if char == "r" else None
        if raw:
            end, value = raw
            tokens.append(Token(value, "string", index, line))
            line += source[index:end].count("\n")
            index = end
            continue
        if char == '"':
            start = index
            start_line = line
            index += 1
            value: list[str] = []
            while index < length and source[index] != '"':
                if source[index] == "\\":
                    if index + 1 >= length:
                        raise TriageError(
                            f"unterminated Rust string starting at line {start_line}"
                        )
                    value.extend(source[index : index + 2])
                    index += 2
                else:
                    line += source[index] == "\n"
                    value.append(source[index])
                    index += 1
            if index >= length:
                raise TriageError(
                    f"unterminated Rust string starting at line {start_line}"
                )
            index += 1
            tokens.append(Token("".join(value), "string", start, start_line))
            continue
        if char.isalpha() or char == "_":
            match = re.match(r"[A-Za-z_][A-Za-z0-9_]*", source[index:])
            assert match
            text = match.group(0)
            tokens.append(Token(text, "ident", index, line))
            index += len(text)
            continue
        if char.isdigit():
            match = re.match(r"[0-9][A-Za-z0-9_]*", source[index:])
            assert match
            text = match.group(0)
            tokens.append(Token(text, "number", index, line))
            index += len(text)
            continue
        if source.startswith("::", index) or source.startswith("->", index):
            tokens.append(Token(source[index : index + 2], "punct", index, line))
            index += 2
            continue
        if char in punctuation:
            tokens.append(Token(char, "punct", index, line))
            index += 1
            continue
        if char == "'":
            # A lifetime has no closing apostrophe (`'a`, `'static`); a Rust
            # character does. Skip the latter atomically so `{`/`}` character
            # literals cannot corrupt balanced-function discovery.
            ident = re.match(r"'[A-Za-z_][A-Za-z0-9_]*", source[index:])
            if ident and not source.startswith("'", index + len(ident.group(0))):
                tokens.append(Token(char, "punct", index, line))
                index += 1
                continue
            start = index
            start_line = line
            index += 1
            while index < length:
                if source[index] == "\\":
                    index += 2
                    continue
                if source[index] == "'":
                    index += 1
                    tokens.append(Token("", "char", start, start_line))
                    break
                line += source[index] == "\n"
                index += 1
            else:
                raise TriageError(
                    f"unterminated Rust character starting at line {start_line}"
                )
            continue
        # The scanner does not interpret arbitrary function bodies. Retaining
        # unknown operators as punctuation keeps brace/function structure
        # visible without pretending to be a complete Rust parser.
        tokens.append(Token(char, "punct", index, line))
        index += 1
    return tokens


def _const_initializer(tokens: Sequence[Token], name: str) -> list[Token]:
    matches: list[list[Token]] = []
    brace_depth = 0
    for index, token in enumerate(tokens[:-1]):
        if token.text == "{":
            brace_depth += 1
            continue
        if token.text == "}":
            brace_depth -= 1
            continue
        if (
            brace_depth
            or token.text not in ("const", "static")
            or tokens[index + 1].text != name
        ):
            continue
        cursor = index + 2
        while cursor < len(tokens) and tokens[cursor].text != "=":
            cursor += 1
        if cursor == len(tokens):
            raise TriageError(f"const {name} has no initializer")
        cursor += 1
        if cursor < len(tokens) and tokens[cursor].text == "&":
            cursor += 1
        if cursor == len(tokens) or tokens[cursor].text != "[":
            raise TriageError(f"const {name} is not an array initializer")
        start = cursor + 1
        depth = 1
        cursor += 1
        while cursor < len(tokens) and depth:
            depth += tokens[cursor].text == "["
            depth -= tokens[cursor].text == "]"
            cursor += 1
        if depth:
            raise TriageError(f"const {name} has an unterminated initializer")
        matches.append(list(tokens[start : cursor - 1]))
    if len(matches) != 1:
        raise TriageError(f"expected one const {name}, found {len(matches)}")
    return matches[0]


def _parse_kind_list(tokens: Sequence[Token], kind_type: str) -> tuple[str, ...]:
    values: list[str] = []
    cursor = 0
    while cursor < len(tokens):
        if tokens[cursor].text == ",":
            cursor += 1
            continue
        expected = [kind_type, "::"]
        if [token.text for token in tokens[cursor : cursor + 2]] != expected:
            got = " ".join(token.text for token in tokens[cursor : cursor + 6])
            raise TriageError(f"unexpected IMPLEMENTED token sequence: {got!r}")
        if cursor + 2 >= len(tokens) or tokens[cursor + 2].kind != "ident":
            raise TriageError("IMPLEMENTED kind has no variant")
        values.append(tokens[cursor + 2].text)
        cursor += 3
        if cursor < len(tokens) and tokens[cursor].text != ",":
            raise TriageError("IMPLEMENTED entries must be comma-separated")
    if len(values) != len(set(values)):
        raise TriageError("IMPLEMENTED contains a duplicate kind")
    return tuple(values)


def _parse_enum_variants(tokens: Sequence[Token], enum_name: str) -> frozenset[str]:
    matches: list[list[Token]] = []
    for cursor, token in enumerate(tokens[:-1]):
        if token.text != "enum" or tokens[cursor + 1].text != enum_name:
            continue
        opening = cursor + 2
        while opening < len(tokens) and tokens[opening].text != "{":
            opening += 1
        if opening == len(tokens):
            raise TriageError(f"enum {enum_name} has no body")
        closing = _matching_brace(tokens, opening)
        matches.append(list(tokens[opening + 1 : closing]))
    if len(matches) != 1:
        raise TriageError(f"expected one enum {enum_name}, found {len(matches)}")

    entries: list[list[Token]] = []
    current: list[Token] = []
    depth = 0
    for token in matches[0]:
        if token.text in ("(", "[", "{"):
            depth += 1
        elif token.text in (")", "]", "}"):
            depth -= 1
        if token.text == "," and depth == 0:
            if current:
                entries.append(current)
                current = []
            continue
        current.append(token)
    if current:
        entries.append(current)

    variants: list[str] = []
    for entry in entries:
        shape = [token.text for token in entry]
        if not entry or entry[0].kind != "ident" or (
            len(entry) != 1
            and not (
                len(entry) == 3
                and entry[1].text == "="
                and entry[2].kind == "number"
            )
        ):
            raise TriageError(
                f"enum {enum_name} has unsupported variant shape: {' '.join(shape)!r}"
            )
        variants.append(entry[0].text)
    if len(variants) != len(set(variants)):
        raise TriageError(f"enum {enum_name} contains a duplicate variant")
    return frozenset(variants)


_POWER_VARIANT_ABSENT = re.compile(
    r"power-variant-absent\(PowerId::(?P<variant>[A-Za-z_][A-Za-z0-9_]*)\)"
)
_POWER_NOT_ADMITTED = re.compile(
    r"power-not-admitted\("
    r"(?P<allowlist>IMPLEMENTED_POWERS|IMPLEMENTED_PLAYER_POWERS), "
    r"PowerId::(?P<variant>[A-Za-z_][A-Za-z0-9_]*)\)"
)


def _parse_escalation_predicates(
    docs: str, where: str
) -> tuple[EscalationPredicate, ...]:
    predicates: list[EscalationPredicate] = []
    for line in docs.splitlines():
        if "ESCALATED-ON:" not in line:
            continue
        if not line.startswith(ESCALATION_MARKER):
            raise TriageError(f"{where} has malformed escalation marker: {line!r}")
        payload = line.removeprefix(ESCALATION_MARKER)
        if match := _POWER_VARIANT_ABSENT.fullmatch(payload):
            predicate = EscalationPredicate(
                "power-variant-absent", match.group("variant")
            )
        elif match := _POWER_NOT_ADMITTED.fullmatch(payload):
            predicate = EscalationPredicate(
                "power-not-admitted",
                match.group("variant"),
                match.group("allowlist"),
            )
        else:
            raise TriageError(
                f"{where} has unsupported escalation predicate: {payload!r}"
            )
        if predicate in predicates:
            raise TriageError(
                f"{where} repeats escalation predicate {predicate.render()!r}"
            )
        predicates.append(predicate)
    return tuple(predicates)


def _parse_family_of(tokens: Sequence[Token], kind_type: str) -> dict[str, str]:
    entries = _const_initializer(tokens, "FAMILY_OF")
    owners: dict[str, str] = {}
    cursor = 0
    while cursor < len(entries):
        if entries[cursor].text == ",":
            cursor += 1
            continue
        shape = ["(", kind_type, "::"]
        if [token.text for token in entries[cursor : cursor + 3]] != shape:
            got = " ".join(token.text for token in entries[cursor : cursor + 8])
            raise TriageError(f"unexpected FAMILY_OF entry: {got!r}")
        if cursor + 6 >= len(entries):
            raise TriageError("truncated FAMILY_OF entry")
        variant = entries[cursor + 3]
        comma = entries[cursor + 4]
        family = entries[cursor + 5]
        close = entries[cursor + 6]
        if (
            variant.kind != "ident"
            or comma.text != ","
            or family.kind != "string"
            or close.text != ")"
        ):
            raise TriageError("malformed FAMILY_OF entry")
        if variant.text in owners:
            raise TriageError(f"FAMILY_OF contains duplicate {kind_type}::{variant.text}")
        owners[variant.text] = family.text
        cursor += 7
        if cursor < len(entries) and entries[cursor].text != ",":
            raise TriageError("FAMILY_OF entries must be comma-separated")
    return owners


def _matching_brace(tokens: Sequence[Token], opening: int) -> int:
    depth = 0
    for cursor in range(opening, len(tokens)):
        depth += tokens[cursor].text == "{"
        depth -= tokens[cursor].text == "}"
        if depth == 0:
            return cursor
    raise TriageError(f"unterminated function body at line {tokens[opening].line}")


def _contiguous_docs(source: str, fn_line: int) -> str:
    lines = source.splitlines()
    cursor = fn_line - 2
    docs: list[str] = []
    while cursor >= 0 and re.match(r"^\s*///(?:[^/]|$)", lines[cursor]):
        docs.append(lines[cursor].strip())
        cursor -= 1
    return "\n".join(reversed(docs))


def _stub_kind(body: Sequence[Token], kind_type: str) -> str | None:
    texts = [token.text for token in body]
    if texts[:5] == ["let", "_", "=", "ctx", ";"]:
        texts = texts[5:]
    refusal = kind_type + "NotModeled"
    expected_prefix = [
        "Err",
        "(",
        "EngineRefusal",
        "::",
        refusal,
        "(",
        kind_type,
        "::",
    ]
    if texts[: len(expected_prefix)] != expected_prefix:
        return None
    if len(texts) not in (len(expected_prefix) + 3, len(expected_prefix) + 4):
        return None
    variant = texts[len(expected_prefix)]
    suffix = texts[len(expected_prefix) + 1 :]
    if suffix == [")", ")"] or suffix == [",", ")", ")"]:
        return variant
    return None


def _family_bodies(source: str) -> tuple[FamilyBody, ...]:
    tokens = lex_rust(source)
    bodies: list[FamilyBody] = []
    depth = 0
    cursor = 0
    while cursor < len(tokens):
        token = tokens[cursor]
        if token.text == "{":
            depth += 1
        elif token.text == "}":
            depth -= 1
        elif token.text == "fn" and depth == 0:
            # Generated dispatch bodies have this exact visibility. A private
            # helper that happens to produce the same refusal is not a family
            # stub; omitting a real body from this shape is caught below by
            # the independent FAMILY_OF partition.
            if [item.text for item in tokens[max(0, cursor - 4) : cursor]] != [
                "pub",
                "(",
                "crate",
                ")",
            ]:
                cursor += 1
                continue
            if cursor + 1 >= len(tokens) or tokens[cursor + 1].kind != "ident":
                raise TriageError(f"function without a name at line {token.line}")
            function = tokens[cursor + 1].text
            opening = cursor + 2
            while opening < len(tokens) and tokens[opening].text != "{":
                opening += 1
            if opening == len(tokens):
                raise TriageError(f"function {function} has no body")
            closing = _matching_brace(tokens, opening)
            bodies.append(
                FamilyBody(
                    function,
                    token.line,
                    tuple(tokens[opening + 1 : closing]),
                    _contiguous_docs(source, token.line),
                )
            )
            cursor = closing
        cursor += 1
    return tuple(bodies)


def _exact_stubs(bodies: Sequence[FamilyBody], kind_type: str) -> tuple[Stub, ...]:
    stubs: list[Stub] = []
    for body in bodies:
        predicates = _parse_escalation_predicates(body.docs, body.function)
        kind = _stub_kind(body.tokens, kind_type)
        if predicates and kind is None:
            raise TriageError(
                f"{body.function} has ESCALATED-ON but is not an exact {kind_type} stub"
            )
        if kind is not None:
            disposition = "untriaged" if BOILERPLATE in body.docs else "escalated"
            stubs.append(
                Stub(kind, body.function, body.line, disposition, predicates)
            )
    kinds = [stub.kind for stub in stubs]
    if len(kinds) != len(set(kinds)):
        raise TriageError("multiple exact stubs refuse the same kind")
    return tuple(stubs)


def _direct_not_modeled_kinds(
    bodies: Sequence[FamilyBody], kind_type: str
) -> frozenset[str]:
    """Return direct ``*KindNotModeled`` variants from real family bodies.

    Unlike exact-stub classification, this deliberately searches the entire
    balanced body.  An implemented body may retain validation or other work
    before refusing its own kind; that is still a false manifest claim.  The
    lexer has already discarded comments and literals, so prose and decoy
    strings cannot create evidence.
    """
    prefix = [
        "EngineRefusal",
        "::",
        kind_type + "NotModeled",
        "(",
        kind_type,
        "::",
    ]
    found: set[str] = set()
    for body in bodies:
        texts = [token.text for token in body.tokens]
        for cursor in range(len(texts) - len(prefix)):
            if texts[cursor : cursor + len(prefix)] != prefix:
                continue
            variant = texts[cursor + len(prefix)]
            suffix = texts[cursor + len(prefix) + 1 : cursor + len(prefix) + 3]
            if suffix[:1] == [")"] or suffix == [",", ")"]:
                found.add(variant)
    return frozenset(found)


def _validate_escalation_freshness(
    src_root: pathlib.Path,
    families: Sequence[Family],
    required: frozenset[tuple[str, str]],
) -> None:
    stubs = {
        (family.name, stub.kind): stub
        for family in families
        for stub in family.untriaged + family.escalated
    }
    marked = frozenset(
        key for key, stub in stubs.items() if stub.escalated_on
    )
    missing = sorted(required - marked)
    unregistered = sorted(marked - required)
    if missing or unregistered:
        raise TriageError(
            "escalation marker registry disagrees with exact stubs: "
            f"missing={missing}, unregistered={unregistered}"
        )
    for key in sorted(required):
        stub = stubs.get(key)
        if stub is None:
            raise TriageError(f"required escalation stub is missing: {key}")
        if stub.disposition != "escalated":
            raise TriageError(f"required escalation stub is not escalated: {key}")

    if not marked:
        return
    ids_path = src_root / "ids.rs"
    admission_path = src_root / "engine" / "admission.rs"
    try:
        power_variants = _parse_enum_variants(
            lex_rust(ids_path.read_text()), "PowerId"
        )
        admission_tokens = lex_rust(admission_path.read_text())
    except OSError as error:
        raise TriageError(f"cannot load escalation authority: {error}") from error
    allowlists = {
        name: frozenset(
            _parse_kind_list(_const_initializer(admission_tokens, name), "PowerId")
        )
        for name in POWER_ALLOWLISTS
    }

    for key in sorted(marked):
        stub = stubs[key]
        for predicate in stub.escalated_on:
            if predicate.kind == "power-variant-absent":
                still_blocked = predicate.power_variant not in power_variants
            elif predicate.kind == "power-not-admitted":
                if predicate.power_variant not in power_variants:
                    raise TriageError(
                        f"{key} uses admission predicate for absent "
                        f"PowerId::{predicate.power_variant}"
                    )
                assert predicate.allowlist in POWER_ALLOWLISTS
                still_blocked = (
                    predicate.power_variant not in allowlists[predicate.allowlist]
                )
            else:
                raise TriageError(
                    f"{key} has unknown escalation predicate: {predicate.kind!r}"
                )
            if not still_blocked:
                raise TriageError(
                    f"{key} escalation freshness expired: {predicate.render()}"
                )


def scan_tree(
    src_root: pathlib.Path,
    required_markers: frozenset[tuple[str, str]] = REQUIRED_ESCALATION_MARKERS,
) -> tuple[Family, ...]:
    families: list[Family] = []
    for group, kind_type in (("steps", "StepKind"), ("moves", "MoveKind")):
        directory = src_root / group
        if not directory.is_dir():
            raise TriageError(f"missing family directory: {directory}")
        mod_path = directory / "mod.rs"
        owners = _parse_family_of(lex_rust(mod_path.read_text()), kind_type)
        expected_names = set(owners.values())
        actual_paths = {
            path.stem: path
            for path in directory.glob("*.rs")
            if path.name != "mod.rs"
        }
        if set(actual_paths) != expected_names:
            missing = sorted(expected_names - set(actual_paths))
            extra = sorted(set(actual_paths) - expected_names)
            raise TriageError(
                f"{group} family files disagree with FAMILY_OF: "
                f"missing={missing}, extra={extra}"
            )
        for name in sorted(expected_names):
            source = actual_paths[name].read_text()
            tokens = lex_rust(source)
            implemented = _parse_kind_list(
                _const_initializer(tokens, "IMPLEMENTED"), kind_type
            )
            bodies = _family_bodies(source)
            stubs = _exact_stubs(bodies, kind_type)
            implemented_set = set(implemented)
            stub_set = {stub.kind for stub in stubs}
            overlap = sorted(implemented_set & stub_set)
            self_refusals = sorted(
                implemented_set & _direct_not_modeled_kinds(bodies, kind_type)
            )
            owned = {kind for kind, owner in owners.items() if owner == name}
            classified = implemented_set | stub_set
            if overlap or self_refusals or classified != owned:
                raise TriageError(
                    f"{group}/{name} does not partition FAMILY_OF: "
                    f"implemented_and_stub={overlap}, "
                    f"implemented_self_refusal={self_refusals}, "
                    f"unclassified={sorted(owned - classified)}, "
                    f"not_owned={sorted(classified - owned)}"
                )
            untriaged = tuple(stub for stub in stubs if stub.disposition == "untriaged")
            escalated = tuple(stub for stub in stubs if stub.disposition == "escalated")
            families.append(
                Family(
                    f"{group}/{name}",
                    tuple(sorted(implemented)),
                    tuple(sorted(untriaged, key=lambda stub: stub.kind)),
                    tuple(sorted(escalated, key=lambda stub: stub.kind)),
                )
            )
    result = tuple(families)
    _validate_escalation_freshness(src_root, result, required_markers)
    return result


def _issue_from_json(raw: object) -> Issue:
    if not isinstance(raw, dict):
        raise TriageError("each GitHub issue must be a JSON object")
    try:
        labels_raw = raw["labels"]
        if not isinstance(labels_raw, list):
            raise TypeError
        labels = frozenset(
            label["name"] if isinstance(label, dict) else label for label in labels_raw
        )
        if not all(isinstance(label, str) for label in labels):
            raise TypeError
        return Issue(
            int(raw["number"]),
            str(raw["title"]),
            str(raw["state"]),
            labels,
            str(raw.get("url", "")),
        )
    except (KeyError, TypeError, ValueError) as error:
        raise TriageError(f"malformed GitHub issue object: {raw!r}") from error


def load_issue_fixture(path: pathlib.Path) -> tuple[Issue, ...]:
    try:
        raw = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        raise TriageError(f"cannot load GitHub fixture {path}: {error}") from error
    if not isinstance(raw, list):
        raise TriageError("GitHub fixture root must be an issue array")
    return tuple(_issue_from_json(item) for item in raw)


def load_github_issues(repo: str | None) -> tuple[Issue, ...]:
    command = [
        "gh",
        "issue",
        "list",
        "--state",
        "all",
        "--label",
        "rust-port-wave",
        "--limit",
        "1000",
        "--json",
        "number,title,state,labels,url",
    ]
    if repo:
        command.extend(["--repo", repo])
    try:
        completed = subprocess.run(
            command, check=True, capture_output=True, text=True
        )
        raw = json.loads(completed.stdout)
    except FileNotFoundError as error:
        raise TriageError("authenticated gh CLI is required for --github-labels") from error
    except subprocess.CalledProcessError as error:
        detail = error.stderr.strip() or f"exit {error.returncode}"
        raise TriageError(f"gh issue list failed: {detail}") from error
    except json.JSONDecodeError as error:
        raise TriageError("gh issue list returned invalid JSON") from error
    if not isinstance(raw, list):
        raise TriageError("gh issue list did not return an issue array")
    return tuple(_issue_from_json(item) for item in raw)


def audit_labels(families: Iterable[Family], issues: Iterable[Issue]) -> LabelAudit:
    wave_issues: dict[str, Issue] = {}
    for issue in issues:
        if "rust-port-wave" not in issue.labels:
            continue
        match = WAVE_TITLE.match(issue.title)
        if not match:
            continue
        family = f"{match.group('group')}/{match.group('family')}"
        if family in wave_issues:
            raise TriageError(f"multiple Rust port wave issues map to {family}")
        wave_issues[family] = issue

    drifts: list[Drift] = []
    closed_history: list[Drift] = []
    expected_families = {family.name for family in families}
    for family in families:
        issue = wave_issues.get(family.name)
        if issue is None:
            drifts.append(Drift(family.name, None, "missing Rust port wave issue"))
            continue
        blocked = "engine-blocked" in issue.labels
        pooled = "unclaimed" in issue.labels
        claimed = "claimed" in issue.labels
        open_issue = issue.state.upper() == "OPEN"
        closed_issue = issue.state.upper() == "CLOSED"
        if family.state == "COMPLETE":
            routing_expected = not blocked and not pooled and not claimed
            expected = closed_issue and routing_expected
        elif family.state == "engine-blocked":
            routing_expected = blocked and not pooled and not claimed
            expected = open_issue and routing_expected
        else:
            # Claiming atomically replaces `unclaimed`; an active owner is a
            # legitimate POOL state and must not become false drift.
            routing_expected = not blocked and (pooled != claimed)
            expected = open_issue and routing_expected
        if not expected:
            difference = Drift(
                family.name,
                issue.number,
                f"derived {family.state}, issue-state:{issue.state.upper()}, labels="
                f"engine-blocked:{blocked},unclaimed:{pooled},claimed:{claimed}",
            )
            # Closed wave issues preserve the labels that described their last
            # open state. They are useful history, not a live pool-routing
            # defect. Keep the raw difference visible without failing drift
            # mode or demanding label archaeology.
            if family.state == "COMPLETE" and closed_issue and not routing_expected:
                closed_history.append(difference)
            else:
                drifts.append(difference)
    for family, issue in sorted(wave_issues.items()):
        if family not in expected_families:
            drifts.append(Drift(family, issue.number, "issue has no source family"))
    return LabelAudit(tuple(drifts), tuple(closed_history))


def label_drifts(families: Iterable[Family], issues: Iterable[Issue]) -> tuple[Drift, ...]:
    """Return actionable, open-issue drift (compatibility helper for callers)."""
    return audit_labels(families, issues).drifts


def _family_json(family: Family) -> dict[str, object]:
    def stub_json(stub: Stub) -> dict[str, object]:
        return {
            "kind": stub.kind,
            "function": stub.function,
            "line": stub.line,
            "escalated_on": [
                predicate.render() for predicate in stub.escalated_on
            ],
        }

    return {
        "family": family.name,
        "state": family.state,
        "implemented": list(family.implemented),
        "untriaged": [stub_json(stub) for stub in family.untriaged],
        "escalated": [stub_json(stub) for stub in family.escalated],
    }


def _totals(families: Iterable[Family]) -> dict[str, int]:
    families = tuple(families)
    return {
        "families": len(families),
        "implemented": sum(len(family.implemented) for family in families),
        "untriaged": sum(len(family.untriaged) for family in families),
        "escalated": sum(len(family.escalated) for family in families),
        "freshness_marked": sum(
            bool(stub.escalated_on)
            for family in families
            for stub in family.escalated
        ),
        "complete_families": sum(family.state == "COMPLETE" for family in families),
        "pool_families": sum(family.state == "POOL" for family in families),
        "engine_blocked_families": sum(
            family.state == "engine-blocked" for family in families
        ),
    }


def render_text(families: Sequence[Family], audit: LabelAudit | None) -> str:
    rows = ["family\timplemented\tuntriaged\tescalated\tfreshness\tstate"]
    for family in families:
        rows.append(
            f"{family.name}\t{len(family.implemented)}\t{len(family.untriaged)}\t"
            f"{len(family.escalated)}\t"
            f"{sum(bool(stub.escalated_on) for stub in family.escalated)}\t"
            f"{family.state}"
        )
    totals = _totals(families)
    rows.append(
        "TOTAL\t{implemented}\t{untriaged}\t{escalated}\t{freshness_marked}\t"
        "{complete_families} COMPLETE / {pool_families} POOL / "
        "{engine_blocked_families} engine-blocked".format(**totals)
    )
    if audit is not None:
        rows.append(f"LABEL_DRIFT\t{len(audit.drifts)}")
        rows.extend(
            f"DRIFT\t{drift.family}\t"
            f"{('#' + str(drift.issue)) if drift.issue else '-'}\t{drift.message}"
            for drift in audit.drifts
        )
        rows.append(f"CLOSED_LABEL_HISTORY\t{len(audit.closed_history)}")
        rows.extend(
            f"HISTORY\t{drift.family}\t"
            f"{('#' + str(drift.issue)) if drift.issue else '-'}\t{drift.message}"
            for drift in audit.closed_history
        )
    return "\n".join(rows) + "\n"


def _write_synthetic_tree(root: pathlib.Path) -> None:
    (root / "ids.rs").write_text(
        "pub enum PowerId { Live = 0, Waiting = 1, }\n"
    )
    engine = root / "engine"
    engine.mkdir(parents=True)
    (engine / "admission.rs").write_text(
        "pub const IMPLEMENTED_POWERS: &[PowerId] = &[PowerId::Live];\n"
        "pub const IMPLEMENTED_PLAYER_POWERS: &[PowerId] = &[PowerId::Live];\n"
    )
    for group, kind_type in (("steps", "StepKind"), ("moves", "MoveKind")):
        directory = root / group
        directory.mkdir(parents=True)
        variants = ("Done", "Fresh", "Blocked") if group == "steps" else ("Done",)
        family = "sample" if group == "steps" else "quiet"
        entries = ", ".join(
            f'({kind_type}::{variant}, "{family}")' for variant in variants
        )
        (directory / "mod.rs").write_text(
            f"pub const FAMILY_OF: &[({kind_type}, &str)] = &[{entries}];\n"
        )
        if group == "steps":
            body = """
pub const IMPLEMENTED: &[StepKind] = &[
    StepKind::Done, // comments and line breaks are intentional
];

/// `fresh` -- not modeled.
/// Python: dispatch; the kind is read in a branch.
pub(crate) fn fresh(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let _ = ctx;
    Err(EngineRefusal::StepKindNotModeled(StepKind::Fresh))
}

/// `blocked` -- not modeled. Named primitive required.
/// ESCALATED-ON: power-variant-absent(PowerId::Blocked)
pub(crate) fn blocked(ctx: &mut StepCtx<'_>) -> Result<(), EngineRefusal> {
    let _ = ctx;
    Err(EngineRefusal::StepKindNotModeled(
        StepKind::Blocked,
    ))
}
"""
        else:
            body = "pub const IMPLEMENTED: &[MoveKind] = &[MoveKind::Done];\n"
        (directory / f"{family}.rs").write_text(body)


def self_test() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        _write_synthetic_tree(root)
        families = scan_tree(root, frozenset({("steps/sample", "Blocked")}))
        states = {family.name: family.state for family in families}
        if states != {"moves/quiet": "COMPLETE", "steps/sample": "POOL"}:
            raise TriageError(f"synthetic state mismatch: {states}")
        issues = (
            Issue(
                1,
                "Rust port wave: steps/sample",
                "OPEN",
                frozenset({"rust-port-wave", "unclaimed"}),
                "",
            ),
            Issue(
                2,
                "Rust port wave: moves/quiet",
                "CLOSED",
                frozenset({"rust-port-wave"}),
                "",
            ),
        )
        if audit_labels(families, issues).drifts:
            raise TriageError("synthetic clean labels unexpectedly drift")
        bad = dataclasses.replace(
            issues[0], labels=frozenset({"rust-port-wave", "engine-blocked"})
        )
        drifts = label_drifts(families, (bad, issues[1]))
        if len(drifts) != 1 or drifts[0].family != "steps/sample":
            raise TriageError("synthetic label mutation was not detected")


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--src-root", type=pathlib.Path, default=DEFAULT_SRC)
    parser.add_argument("--json", action="store_true", help="emit machine-readable JSON")
    labels = parser.add_mutually_exclusive_group()
    labels.add_argument(
        "--github-labels",
        action="store_true",
        help="check labels using authenticated gh issue data",
    )
    labels.add_argument(
        "--github-fixture",
        type=pathlib.Path,
        help="check labels using an explicit offline gh-format JSON fixture",
    )
    parser.add_argument("--repo", help="OWNER/REPO for --github-labels")
    parser.add_argument("--self-test", action="store_true")
    return parser.parse_args(argv)


def main(
    argv: Sequence[str] | None = None,
    *,
    required_markers: frozenset[tuple[str, str]] = REQUIRED_ESCALATION_MARKERS,
) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    try:
        if args.self_test:
            self_test()
        families = scan_tree(args.src_root, required_markers)
        issues: tuple[Issue, ...] | None = None
        if args.github_labels:
            issues = load_github_issues(args.repo)
        elif args.github_fixture:
            issues = load_issue_fixture(args.github_fixture)
        audit = audit_labels(families, issues) if issues is not None else None
        if args.json:
            payload: dict[str, object] = {
                "totals": _totals(families),
                "families": [_family_json(family) for family in families],
            }
            if audit is not None:
                payload["label_drifts"] = [
                    dataclasses.asdict(drift) for drift in audit.drifts
                ]
                payload["closed_label_history"] = [
                    dataclasses.asdict(drift) for drift in audit.closed_history
                ]
            print(json.dumps(payload, indent=2, sort_keys=True))
        else:
            print(render_text(families, audit), end="")
        return 1 if audit and audit.drifts else 0
    except TriageError as error:
        print(f"family_triage: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())

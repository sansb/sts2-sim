#!/usr/bin/env python3
"""Mutation controls for the tree-derived Rust family triage instrument."""

from __future__ import annotations

import dataclasses
import importlib.util
import io
import json
import pathlib
import re
import shutil
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout


HERE = pathlib.Path(__file__).resolve()
RUST_DIR = HERE.parents[1]


def _load_tool():
    spec = importlib.util.spec_from_file_location(
        "family_triage", RUST_DIR / "tools" / "family_triage.py"
    )
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


TRIAGE = _load_tool()
SYNTHETIC_REQUIRED = frozenset({("steps/sample", "Blocked")})

POWER_BACKFILL_EXPECTED = {
    ("steps/ironclad_rare", "TankExact"): (
        "power-variant-absent(PowerId::Tank)",
        "power-variant-absent(PowerId::Guarded)",
    ),
    ("steps/necrobinder_uncommon", "SoulboundExact"): (
        "power-variant-absent(PowerId::Soulbound)",
    ),
    ("steps/neutral", "TagTeamExact"): (
        "power-variant-absent(PowerId::TagTeam)",
    ),
    ("steps/silent_rare", "FlankingExact"): (
        "power-variant-absent(PowerId::Flanking)",
    ),
    ("steps/silent_uncommon", "ConcoctExact"): (
        "power-variant-absent(PowerId::Concoct)",
    ),
}


class SyntheticTree(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)
        TRIAGE._write_synthetic_tree(self.root)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def scan(self):
        return {
            family.name: family
            for family in TRIAGE.scan_tree(self.root, SYNTHETIC_REQUIRED)
        }


class ExactSourceClassification(SyntheticTree):
    def test_known_nonzero_case_distinguishes_all_three_dispositions(self) -> None:
        families = self.scan()
        sample = families["steps/sample"]
        self.assertEqual(sample.implemented, ("Done",))
        self.assertEqual([stub.kind for stub in sample.untriaged], ["Fresh"])
        self.assertEqual([stub.kind for stub in sample.escalated], ["Blocked"])
        self.assertEqual(sample.state, "POOL")
        self.assertEqual(families["moves/quiet"].state, "COMPLETE")

    def test_only_an_exact_stub_body_counts(self) -> None:
        path = self.root / "steps" / "sample.rs"
        source = path.read_text()
        source = source.replace(
            "    let _ = ctx;\n    Err(EngineRefusal::StepKindNotModeled(StepKind::Fresh))",
            '    let _ = "EngineRefusal::StepKindNotModeled(StepKind::Fresh)";\n'
            "    Ok(())",
        )
        path.write_text(source)
        with self.assertRaisesRegex(TRIAGE.TriageError, r"unclassified=\['Fresh'\]"):
            self.scan()

    def test_the_full_balanced_body_is_searched_past_forty_lines(self) -> None:
        path = self.root / "steps" / "sample.rs"
        source = path.read_text().replace(
            "    let _ = ctx;\n    Err(EngineRefusal::StepKindNotModeled(StepKind::Fresh))",
            "    let _ = ctx;\n"
            + "".join(f"    // provenance line {index}\n" for index in range(60))
            + "    Err(EngineRefusal::StepKindNotModeled(StepKind::Fresh))",
        )
        path.write_text(source)
        self.assertEqual(
            [stub.kind for stub in self.scan()["steps/sample"].untriaged], ["Fresh"]
        )

    def test_a_statement_before_the_refusal_is_not_a_generated_stub(self) -> None:
        path = self.root / "steps" / "sample.rs"
        source = path.read_text().replace(
            "    let _ = ctx;\n    Err(EngineRefusal::StepKindNotModeled(StepKind::Fresh))",
            "    inspect(ctx);\n    Err(EngineRefusal::StepKindNotModeled(StepKind::Fresh))",
        )
        path.write_text(source)
        with self.assertRaisesRegex(TRIAGE.TriageError, r"unclassified=\['Fresh'\]"):
            self.scan()

    def test_a_private_refusal_helper_is_not_a_generated_stub(self) -> None:
        path = self.root / "steps" / "sample.rs"
        path.write_text(path.read_text().replace("pub(crate) fn fresh", "fn fresh"))
        with self.assertRaisesRegex(TRIAGE.TriageError, r"unclassified=\['Fresh'\]"):
            self.scan()

    def test_comments_and_decoy_constants_do_not_change_the_manifest(self) -> None:
        path = self.root / "steps" / "sample.rs"
        source = path.read_text()
        source = (
            "// const IMPLEMENTED: &[StepKind] = &[StepKind::Fake];\n"
            'const NOTE: &str = r#"StepKindNotModeled(StepKind::Fake)"#;\n'
            "/* nested /* const IMPLEMENTED: &[StepKind] = &[]; */ comment */\n"
            + source
        )
        path.write_text(source)
        self.assertEqual(self.scan()["steps/sample"].implemented, ("Done",))

    def test_attributes_nested_constants_and_brace_characters_are_ignored(self) -> None:
        path = self.root / "steps" / "sample.rs"
        source = path.read_text().replace(
            "pub const IMPLEMENTED",
            "#[rustfmt::skip]\npub const IMPLEMENTED",
        )
        source += """
mod decoys {
    const IMPLEMENTED: &[StepKind] = &[StepKind::Fake];
    fn braces() { let _ = ('{', '}', b'{', r#"}"#); }
}
"""
        path.write_text(source)
        self.assertEqual(self.scan()["steps/sample"].implemented, ("Done",))

    def test_manifest_shape_and_duplicates_fail_closed(self) -> None:
        path = self.root / "steps" / "sample.rs"
        original = path.read_text()
        for replacement, message in (
            ("StepKind::Done, StepKind::Done,", "duplicate"),
            ("other::Done,", "unexpected IMPLEMENTED"),
        ):
            with self.subTest(replacement=replacement):
                path.write_text(original.replace("StepKind::Done,", replacement))
                with self.assertRaisesRegex(TRIAGE.TriageError, message):
                    self.scan()
        path.write_text(original)

    def test_family_of_is_the_independent_completeness_authority(self) -> None:
        path = self.root / "steps" / "mod.rs"
        path.write_text(
            path.read_text().replace(
                '(StepKind::Blocked, "sample")',
                '(StepKind::Blocked, "sample"), (StepKind::NewKind, "sample")',
            )
        )
        with self.assertRaisesRegex(TRIAGE.TriageError, r"unclassified=\['NewKind'\]"):
            self.scan()


class ContiguousDocBlockIsLoadBearing(SyntheticTree):
    def test_removing_the_marker_promotes_the_stub_to_escalated(self) -> None:
        path = self.root / "steps" / "sample.rs"
        path.write_text(path.read_text().replace(TRIAGE.BOILERPLATE, "dispatch branch"))
        sample = self.scan()["steps/sample"]
        self.assertEqual(sample.state, "engine-blocked")
        self.assertEqual(len(sample.untriaged), 0)
        self.assertEqual({stub.kind for stub in sample.escalated}, {"Fresh", "Blocked"})

    def test_a_blank_or_attribute_breaks_the_preceding_doc_block(self) -> None:
        path = self.root / "steps" / "sample.rs"
        original = path.read_text()
        needle = "/// Python: dispatch; the kind is read in a branch.\n"
        for separator in ("\n", "#[allow(dead_code)]\n"):
            with self.subTest(separator=separator):
                path.write_text(original.replace(needle, needle + separator))
                sample = self.scan()["steps/sample"]
                self.assertEqual(sample.state, "engine-blocked")
        path.write_text(original)

    def test_four_slashes_are_not_a_rust_doc_comment(self) -> None:
        path = self.root / "steps" / "sample.rs"
        path.write_text(
            path.read_text().replace(
                "/// Python: dispatch; the kind is read in a branch.",
                "//// Python: dispatch; the kind is read in a branch.",
            )
        )
        self.assertEqual(self.scan()["steps/sample"].state, "engine-blocked")


class EscalationFreshnessControls(SyntheticTree):
    marker = (
        "/// ESCALATED-ON: "
        "power-variant-absent(PowerId::Blocked)"
    )

    @property
    def family_path(self) -> pathlib.Path:
        return self.root / "steps" / "sample.rs"

    @property
    def ids_path(self) -> pathlib.Path:
        return self.root / "ids.rs"

    @property
    def admission_path(self) -> pathlib.Path:
        return self.root / "engine" / "admission.rs"

    def test_live_marker_is_parsed_and_reported(self) -> None:
        sample = self.scan()["steps/sample"]
        blocked = next(stub for stub in sample.escalated if stub.kind == "Blocked")
        self.assertEqual(
            [predicate.render() for predicate in blocked.escalated_on],
            ["power-variant-absent(PowerId::Blocked)"],
        )
        row = TRIAGE._family_json(sample)
        self.assertEqual(
            row["escalated"][0]["escalated_on"],
            ["power-variant-absent(PowerId::Blocked)"],
        )
        self.assertEqual(TRIAGE._totals(self.scan().values())["freshness_marked"], 1)

    def test_marker_presence_is_required_and_contiguous(self) -> None:
        original = self.family_path.read_text()
        for replacement in ("", self.marker + "\n\n", self.marker + "\n#[allow(dead_code)]"):
            with self.subTest(replacement=replacement):
                self.family_path.write_text(original.replace(self.marker, replacement))
                with self.assertRaisesRegex(TRIAGE.TriageError, "registry.*missing"):
                    self.scan()
        self.family_path.write_text(original)

    def test_marker_grammar_and_registry_are_fail_closed(self) -> None:
        original = self.family_path.read_text()
        mutations = (
            (self.marker.replace("power-variant-absent", "symbol-absent"), "unsupported"),
            (self.marker + "\n" + self.marker, "repeats"),
        )
        for replacement, message in mutations:
            with self.subTest(replacement=replacement):
                self.family_path.write_text(original.replace(self.marker, replacement))
                with self.assertRaisesRegex(TRIAGE.TriageError, message):
                    self.scan()

        self.family_path.write_text(
            original.replace(
                "/// Python: dispatch; the kind is read in a branch.",
                "/// Python: dispatch; the kind is read in a branch.\n"
                "/// ESCALATED-ON: power-variant-absent(PowerId::Other)",
            )
        )
        with self.assertRaisesRegex(TRIAGE.TriageError, "unregistered"):
            self.scan()

        implemented = original.replace(
            "StepKind::Done, // comments and line breaks are intentional",
            "StepKind::Done, StepKind::Blocked, // comments and line breaks are intentional",
        ).replace(
            "    let _ = ctx;\n    Err(EngineRefusal::StepKindNotModeled(\n"
            "        StepKind::Blocked,\n    ))",
            "    let _ = ctx;\n    Ok(())",
        )
        self.family_path.write_text(implemented)
        with self.assertRaisesRegex(TRIAGE.TriageError, "not an exact StepKind stub"):
            self.scan()

        self.family_path.write_text(original)
        with self.assertRaisesRegex(TRIAGE.TriageError, "registry.*Missing"):
            TRIAGE.scan_tree(
                self.root,
                SYNTHETIC_REQUIRED | {("steps/sample", "Missing")},
            )

    def test_absent_variant_uses_real_tokens_not_comments_or_strings(self) -> None:
        original = self.ids_path.read_text()
        self.ids_path.write_text(
            "// PowerId::Blocked\n"
            'const NOTE: &str = "PowerId::Blocked";\n'
            + original
        )
        self.scan()

        self.ids_path.write_text(original.replace("Waiting = 1", "Waiting = 1, Other = 2"))
        self.scan()

        self.ids_path.write_text(original.replace("Waiting = 1", "Waiting = 1, Blocked = 2"))
        with self.assertRaisesRegex(TRIAGE.TriageError, "freshness expired"):
            self.scan()

    def test_allowlist_predicate_is_side_specific_and_requires_a_variant(self) -> None:
        original_family = self.family_path.read_text()
        class_a = (
            "/// ESCALATED-ON: "
            "power-not-admitted(IMPLEMENTED_PLAYER_POWERS, PowerId::Blocked)"
        )
        self.family_path.write_text(original_family.replace(self.marker, class_a))
        with self.assertRaisesRegex(TRIAGE.TriageError, "predicate for absent"):
            self.scan()

        self.ids_path.write_text(
            self.ids_path.read_text().replace("Waiting = 1", "Waiting = 1, Blocked = 2")
        )
        self.scan()

        original_admission = self.admission_path.read_text()
        self.admission_path.write_text(
            original_admission.replace(
                "IMPLEMENTED_POWERS: &[PowerId] = &[PowerId::Live]",
                "IMPLEMENTED_POWERS: &[PowerId] = &[PowerId::Live, PowerId::Blocked]",
            )
        )
        self.scan()

        self.admission_path.write_text(
            original_admission.replace(
                "IMPLEMENTED_PLAYER_POWERS: &[PowerId] = &[PowerId::Live]",
                "IMPLEMENTED_PLAYER_POWERS: &[PowerId] = "
                "&[PowerId::Live, PowerId::Blocked]",
            )
        )
        with self.assertRaisesRegex(TRIAGE.TriageError, "freshness expired"):
            self.scan()

    def test_repeated_predicates_are_conjoined(self) -> None:
        second = (
            "/// ESCALATED-ON: "
            "power-not-admitted(IMPLEMENTED_PLAYER_POWERS, PowerId::Waiting)"
        )
        self.family_path.write_text(
            self.family_path.read_text().replace(self.marker, self.marker + "\n" + second)
        )
        self.scan()

        original_ids = self.ids_path.read_text()
        self.ids_path.write_text(
            original_ids.replace("Waiting = 1", "Waiting = 1, Blocked = 2")
        )
        with self.assertRaisesRegex(TRIAGE.TriageError, "freshness expired"):
            self.scan()

        self.ids_path.write_text(original_ids)
        self.admission_path.write_text(
            self.admission_path.read_text().replace(
                "IMPLEMENTED_PLAYER_POWERS: &[PowerId] = &[PowerId::Live]",
                "IMPLEMENTED_PLAYER_POWERS: &[PowerId] = "
                "&[PowerId::Live, PowerId::Waiting]",
            )
        )
        with self.assertRaisesRegex(TRIAGE.TriageError, "freshness expired"):
            self.scan()


class LabelDriftControls(SyntheticTree):
    def issues(self):
        return (
            TRIAGE.Issue(
                10,
                "Rust port wave: steps/sample (three kinds)",
                "OPEN",
                frozenset({"rust-port-wave", "unclaimed"}),
                "https://example.test/10",
            ),
            TRIAGE.Issue(
                11,
                "Rust port wave: moves/quiet",
                "CLOSED",
                frozenset({"rust-port-wave"}),
                "https://example.test/11",
            ),
        )

    def test_clean_zero_and_each_label_mutation(self) -> None:
        families = tuple(self.scan().values())
        issues = self.issues()
        self.assertEqual(TRIAGE.label_drifts(families, issues), ())

        blocked_pool = dataclasses.replace(
            issues[0], labels=frozenset({"rust-port-wave", "engine-blocked"})
        )
        stale_complete = dataclasses.replace(
            issues[1],
            state="OPEN",
            labels=frozenset({"rust-port-wave", "engine-blocked"}),
        )
        for mutated, expected in (
            ((blocked_pool, issues[1]), "steps/sample"),
            ((issues[0], stale_complete), "moves/quiet"),
            ((issues[1],), "steps/sample"),
        ):
            with self.subTest(expected=expected):
                drifts = TRIAGE.label_drifts(families, mutated)
                self.assertEqual([drift.family for drift in drifts], [expected])

    def test_claimed_is_the_atomic_pool_ownership_exception(self) -> None:
        families = tuple(self.scan().values())
        claimed = dataclasses.replace(
            self.issues()[0], labels=frozenset({"rust-port-wave", "claimed"})
        )
        self.assertEqual(TRIAGE.label_drifts(families, (claimed, self.issues()[1])), ())

    def test_pool_labels_are_exactly_one_of_unclaimed_or_claimed(self) -> None:
        families = tuple(self.scan().values())
        quiet = self.issues()[1]
        for labels in (
            frozenset({"rust-port-wave"}),
            frozenset({"rust-port-wave", "unclaimed", "claimed"}),
        ):
            with self.subTest(labels=labels):
                pool = dataclasses.replace(self.issues()[0], labels=labels)
                self.assertEqual(
                    [drift.family for drift in TRIAGE.label_drifts(families, (pool, quiet))],
                    ["steps/sample"],
                )

    def test_closed_label_archaeology_is_reported_but_not_actionable(self) -> None:
        families = tuple(self.scan().values())
        stale = dataclasses.replace(
            self.issues()[1], labels=frozenset({"rust-port-wave", "engine-blocked"})
        )
        audit = TRIAGE.audit_labels(families, (self.issues()[0], stale))
        self.assertEqual(audit.drifts, ())
        self.assertEqual([row.family for row in audit.closed_history], ["moves/quiet"])

    def test_open_complete_is_lifecycle_drift_even_without_routing_labels(self) -> None:
        families = tuple(self.scan().values())
        open_complete = dataclasses.replace(self.issues()[1], state="OPEN")
        audit = TRIAGE.audit_labels(families, (self.issues()[0], open_complete))
        self.assertEqual([row.family for row in audit.drifts], ["moves/quiet"])
        self.assertEqual(audit.closed_history, ())

    def test_closed_pool_is_lifecycle_drift_even_with_unclaimed(self) -> None:
        families = tuple(self.scan().values())
        closed_pool = dataclasses.replace(self.issues()[0], state="CLOSED")
        audit = TRIAGE.audit_labels(families, (closed_pool, self.issues()[1]))
        self.assertEqual([row.family for row in audit.drifts], ["steps/sample"])
        self.assertEqual(audit.closed_history, ())

    def test_closed_engine_blocked_is_lifecycle_drift_with_matching_label(self) -> None:
        path = self.root / "steps" / "sample.rs"
        path.write_text(path.read_text().replace(TRIAGE.BOILERPLATE, "named blocker"))
        families = tuple(self.scan().values())
        closed_blocked = dataclasses.replace(
            self.issues()[0],
            state="CLOSED",
            labels=frozenset({"rust-port-wave", "engine-blocked"}),
        )
        audit = TRIAGE.audit_labels(families, (closed_blocked, self.issues()[1]))
        self.assertEqual([row.family for row in audit.drifts], ["steps/sample"])
        self.assertEqual(audit.closed_history, ())

    def test_duplicate_title_mapping_fails_closed(self) -> None:
        families = tuple(self.scan().values())
        duplicate = dataclasses.replace(self.issues()[0], number=99)
        with self.assertRaisesRegex(TRIAGE.TriageError, "multiple.*steps/sample"):
            TRIAGE.label_drifts(families, self.issues() + (duplicate,))

    def test_explicit_offline_fixture_accepts_gh_and_string_labels(self) -> None:
        fixture = self.root / "issues.json"
        fixture.write_text(
            json.dumps(
                [
                    {
                        "number": 10,
                        "title": "Rust port wave: steps/sample",
                        "state": "OPEN",
                        "labels": [{"name": "rust-port-wave"}, {"name": "unclaimed"}],
                    },
                    {
                        "number": 11,
                        "title": "Rust port wave: moves/quiet",
                        "state": "CLOSED",
                        "labels": ["rust-port-wave"],
                    },
                ]
            )
        )
        issues = TRIAGE.load_issue_fixture(fixture)
        self.assertEqual(TRIAGE.label_drifts(tuple(self.scan().values()), issues), ())

    def test_cli_clean_zero_and_known_drift_have_distinct_exit_codes(self) -> None:
        fixture = self.root / "issues.json"
        rows = [
            {
                "number": issue.number,
                "title": issue.title,
                "state": issue.state,
                "labels": [{"name": label} for label in sorted(issue.labels)],
                "url": issue.url,
            }
            for issue in self.issues()
        ]
        fixture.write_text(json.dumps(rows))
        argv = [
            "--src-root",
            str(self.root),
            "--github-fixture",
            str(fixture),
            "--json",
        ]
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            self.assertEqual(
                TRIAGE.main(argv, required_markers=SYNTHETIC_REQUIRED), 0
            )
        rows[0]["labels"] = [
            {"name": "rust-port-wave"},
            {"name": "engine-blocked"},
        ]
        fixture.write_text(json.dumps(rows))
        with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
            self.assertEqual(
                TRIAGE.main(argv, required_markers=SYNTHETIC_REQUIRED), 1
            )


class RealTreeContract(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name) / "src"
        shutil.copytree(RUST_DIR / "src", self.root)

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def replace_body(self, relative: str, function: str, body: str) -> None:
        path = self.root / relative
        source = path.read_text()
        tokens = TRIAGE.lex_rust(source)
        for cursor, token in enumerate(tokens):
            if token.text != "fn" or tokens[cursor + 1].text != function:
                continue
            opening = cursor + 2
            while tokens[opening].text != "{":
                opening += 1
            closing = TRIAGE._matching_brace(tokens, opening)
            path.write_text(
                source[: tokens[opening].start + 1]
                + "\n"
                + body
                + "\n"
                + source[tokens[closing].start :]
            )
            return
        self.fail(f"missing real family body: {relative}::{function}")

    def inject_const_array_entry(self, source: str, name: str, entry: str) -> str:
        """Insert into one named top-level const array without pinning its shape."""
        tokens = TRIAGE.lex_rust(source)
        openings: list[int] = []
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
            self.assertLess(cursor, len(tokens), f"{name} has no initializer")
            cursor += 1
            if cursor < len(tokens) and tokens[cursor].text == "&":
                cursor += 1
            self.assertLess(cursor, len(tokens), f"{name} has no array initializer")
            self.assertEqual(tokens[cursor].text, "[", f"{name} is not an array")
            openings.append(tokens[cursor].start + 1)
        self.assertEqual(len(openings), 1, f"expected one top-level const {name}")
        opening = openings[0]
        return source[:opening] + f"\n    {entry}," + source[opening:]

    def inject_enum_variant(self, source: str, name: str, variant: str) -> str:
        """Insert into one named enum without pinning its count or members."""
        tokens = TRIAGE.lex_rust(source)
        openings: list[int] = []
        for index, token in enumerate(tokens[:-1]):
            if token.text != "enum" or tokens[index + 1].text != name:
                continue
            cursor = index + 2
            while cursor < len(tokens) and tokens[cursor].text != "{":
                cursor += 1
            self.assertLess(cursor, len(tokens), f"enum {name} has no body")
            openings.append(tokens[cursor].start + 1)
        self.assertEqual(len(openings), 1, f"expected one enum {name}")
        opening = openings[0]
        return source[:opening] + f"\n    {variant} = 999999," + source[opening:]

    def test_live_tree_partitions_every_generated_kind(self) -> None:
        families = TRIAGE.scan_tree(self.root)
        self.assertGreater(len(families), 0)
        self.assertEqual(len({family.name for family in families}), len(families))
        self.assertGreater(sum(len(family.implemented) for family in families), 0)
        self.assertGreater(
            sum(len(family.untriaged) + len(family.escalated) for family in families), 0
        )

    def test_live_call_of_the_void_is_implemented_and_its_marker_is_retired(self) -> None:
        families = {family.name: family for family in TRIAGE.scan_tree(self.root)}
        family = families["steps/necrobinder_rare"]
        self.assertIn("CallOfTheVoid", family.implemented)
        self.assertNotIn(
            ("steps/necrobinder_rare", "CallOfTheVoid"),
            TRIAGE.REQUIRED_ESCALATION_MARKERS,
        )

        family_path = self.root / "steps" / "necrobinder_rare.rs"
        source = family_path.read_text()
        marker = "/// ESCALATED-ON: power-variant-absent(PowerId::CallOfTheVoid)\n"
        self.assertNotIn(marker, source)
        mutated = source.replace(
            "pub(crate) fn call_of_the_void(ctx:",
            marker + "pub(crate) fn call_of_the_void(ctx:",
            1,
        )
        self.assertNotEqual(mutated, source)
        family_path.write_text(mutated)
        with self.assertRaisesRegex(
            TRIAGE.TriageError,
            r"call_of_the_void has ESCALATED-ON but is not an exact StepKind stub",
        ):
            TRIAGE.scan_tree(self.root)

    def test_live_power_backfill_is_exact_and_required(self) -> None:
        families = {family.name: family for family in TRIAGE.scan_tree(self.root)}
        actual = {}
        for family_name, family in families.items():
            for stub in family.escalated:
                key = (family_name, stub.kind)
                if key in POWER_BACKFILL_EXPECTED:
                    actual[key] = tuple(
                        predicate.render() for predicate in stub.escalated_on
                    )
        self.assertEqual(actual, POWER_BACKFILL_EXPECTED)

        for (family_name, kind), predicates in POWER_BACKFILL_EXPECTED.items():
            for predicate in predicates:
                path = self.root / f"{family_name}.rs"
                source = path.read_text()
                marker = f"/// ESCALATED-ON: {predicate}\n"
                self.assertEqual(source.count(marker), 1)
                path.write_text(source.replace(marker, "", 1))
        with self.assertRaisesRegex(TRIAGE.TriageError, "registry.*missing") as error:
            TRIAGE.scan_tree(self.root)
        for family_name, kind in POWER_BACKFILL_EXPECTED:
            self.assertIn(repr((family_name, kind)), str(error.exception))

    def test_each_absent_power_landing_expires_its_exact_marker(self) -> None:
        ids_path = self.root / "ids.rs"
        source = ids_path.read_text()
        families = TRIAGE.scan_tree(self.root)
        absent = {
            predicate.removeprefix("power-variant-absent(PowerId::").removesuffix(")"):
            (family_name, kind)
            for (family_name, kind), predicates in POWER_BACKFILL_EXPECTED.items()
            for predicate in predicates
            if predicate.startswith("power-variant-absent")
        }
        for power, (family_name, kind) in absent.items():
            with self.subTest(landed=(power, family_name, kind)):
                ids_path.write_text(
                    self.inject_enum_variant(source, "PowerId", power)
                )
                with self.assertRaisesRegex(
                    TRIAGE.TriageError,
                    rf"{family_name.split('/')[-1]}.*{kind}.*freshness expired.*{power}",
                ):
                    TRIAGE._validate_escalation_freshness(
                        self.root,
                        families,
                        TRIAGE.REQUIRED_ESCALATION_MARKERS,
                    )
                ids_path.write_text(source)

    def test_absent_power_decoys_do_not_expire_backfill_markers(self) -> None:
        ids_path = self.root / "ids.rs"
        source = ids_path.read_text()
        absent = sorted(
            predicate.removeprefix("power-variant-absent(PowerId::").removesuffix(")")
            for predicates in POWER_BACKFILL_EXPECTED.values()
            for predicate in predicates
            if predicate.startswith("power-variant-absent")
        )
        decoys = "".join(
            f"// PowerId::{power}\n"
            f'const {power.upper()}_DECOY: &str = "PowerId::{power}";\n'
            for power in absent
        )
        ids_path.write_text(decoys + source)
        TRIAGE.scan_tree(self.root)

    def test_each_unadmitted_power_landing_is_exactly_side_specific(self) -> None:
        admission_path = self.root / "engine" / "admission.rs"
        source = admission_path.read_text()
        families = TRIAGE.scan_tree(self.root)
        rows = []
        for (family_name, kind), predicates in POWER_BACKFILL_EXPECTED.items():
            for predicate in predicates:
                match = TRIAGE._POWER_NOT_ADMITTED.fullmatch(predicate)
                if match:
                    rows.append(
                        (family_name, kind, match.group("allowlist"), match.group("variant"))
                    )

        for family_name, kind, allowlist, power in rows:
            with self.subTest(admitted=(family_name, kind, allowlist, power)):
                admission_path.write_text(
                    self.inject_const_array_entry(
                        source, allowlist, f"PowerId::{power}"
                    )
                )
                with self.assertRaisesRegex(
                    TRIAGE.TriageError,
                    rf"{family_name.split('/')[-1]}.*{kind}.*freshness expired.*{allowlist}.*{power}",
                ):
                    TRIAGE._validate_escalation_freshness(
                        self.root,
                        families,
                        TRIAGE.REQUIRED_ESCALATION_MARKERS,
                    )
                admission_path.write_text(source)

        wrong_side = source
        for _, _, allowlist, power in rows:
            opposite = (
                "IMPLEMENTED_POWERS"
                if allowlist == "IMPLEMENTED_PLAYER_POWERS"
                else "IMPLEMENTED_PLAYER_POWERS"
            )
            wrong_side = self.inject_const_array_entry(
                wrong_side, opposite, f"PowerId::{power}"
            )
        admission_path.write_text(wrong_side)
        TRIAGE._validate_escalation_freshness(
            self.root,
            families,
            TRIAGE.REQUIRED_ESCALATION_MARKERS,
        )

    def test_enum_mutation_helper_ignores_count_and_existing_members(self) -> None:
        source = """
pub enum PowerId {
    OddFirst = 41,
    UnexpectedSecond = 7,
}
impl PowerId { pub const COUNT: usize = 12345; }
"""
        mutated = self.inject_enum_variant(source, "PowerId", "Calamity")
        variants = TRIAGE._parse_enum_variants(
            TRIAGE.lex_rust(mutated), "PowerId"
        )
        self.assertEqual(
            variants, {"Calamity", "OddFirst", "UnexpectedSecond"}
        )

    def test_live_template_markers_are_required_and_source_derived(self) -> None:
        expected = {
            "Underworld": (
                "IMPLEMENTED_PLAYER_POWERS",
                "PowerId::Underworld",
            ),
        }
        families = {family.name: family for family in TRIAGE.scan_tree(self.root)}
        templates = {stub.kind: stub for stub in families["steps/templates"].escalated}
        self.assertEqual(set(templates), set(expected))
        for kind, (allowlist, power) in expected.items():
            with self.subTest(kind=kind):
                self.assertEqual(
                    [predicate.render() for predicate in templates[kind].escalated_on],
                    [f"power-not-admitted({allowlist}, {power})"],
                )

        family_path = self.root / "steps" / "templates.rs"
        source = family_path.read_text()
        for kind, (allowlist, power) in expected.items():
            with self.subTest(deleted=kind):
                marker = f"/// ESCALATED-ON: power-not-admitted({allowlist}, {power})\n"
                self.assertIn(marker, source)
                family_path.write_text(source.replace(marker, "", 1))
                with self.assertRaisesRegex(TRIAGE.TriageError, "registry.*missing"):
                    TRIAGE.scan_tree(self.root)
                family_path.write_text(source)

    def test_live_oblivion_is_implemented_and_its_marker_is_retired(self) -> None:
        families = {family.name: family for family in TRIAGE.scan_tree(self.root)}
        family = families["steps/templates"]
        self.assertIn("Oblivion", family.implemented)
        self.assertNotIn(
            ("steps/templates", "Oblivion"),
            TRIAGE.REQUIRED_ESCALATION_MARKERS,
        )

        family_path = self.root / "steps" / "templates.rs"
        source = family_path.read_text()
        marker = (
            "/// ESCALATED-ON: "
            "power-not-admitted(IMPLEMENTED_POWERS, PowerId::Oblivion)\n"
        )
        self.assertNotIn(marker, source)
        mutated = source.replace(
            "pub(crate) fn oblivion",
            marker + "pub(crate) fn oblivion",
            1,
        )
        self.assertNotEqual(mutated, source)
        family_path.write_text(mutated)
        with self.assertRaisesRegex(
            TRIAGE.TriageError,
            r"oblivion has ESCALATED-ON but is not an exact StepKind stub",
        ):
            TRIAGE.scan_tree(self.root)

    def test_live_entropy_is_implemented_and_its_marker_is_retired(self) -> None:
        families = {family.name: family for family in TRIAGE.scan_tree(self.root)}
        family = families["steps/templates"]
        self.assertIn("Entropy", family.implemented)
        self.assertNotIn(
            ("steps/templates", "Entropy"),
            TRIAGE.REQUIRED_ESCALATION_MARKERS,
        )

        source = (self.root / "steps" / "templates.rs").read_text()
        self.assertNotIn(
            "power-not-admitted(IMPLEMENTED_PLAYER_POWERS, PowerId::Entropy)",
            source,
        )

    def test_live_template_allowlist_admission_expires_each_marker(self) -> None:
        expected = {
            "Underworld": ("IMPLEMENTED_PLAYER_POWERS", "Underworld"),
        }
        admission_path = self.root / "engine" / "admission.rs"
        source = admission_path.read_text()
        for kind, (allowlist, power) in expected.items():
            with self.subTest(admitted=kind, allowlist=allowlist):
                admission_path.write_text(
                    self.inject_const_array_entry(
                        source,
                        allowlist,
                        f"PowerId::{power}",
                    )
                )
                with self.assertRaisesRegex(
                    TRIAGE.TriageError,
                    rf"templates.*{kind}.*freshness expired.*{allowlist}.*{power}",
                ):
                    TRIAGE.scan_tree(self.root)
                admission_path.write_text(source)

        wrong_side = source
        for _, (allowlist, power) in expected.items():
            opposite = (
                "IMPLEMENTED_POWERS"
                if allowlist == "IMPLEMENTED_PLAYER_POWERS"
                else "IMPLEMENTED_PLAYER_POWERS"
            )
            wrong_side = self.inject_const_array_entry(
                wrong_side,
                opposite,
                f"PowerId::{power}",
            )
        admission_path.write_text(wrong_side)
        TRIAGE.scan_tree(self.root)

    def test_allowlist_mutation_helper_ignores_count_and_existing_members(self) -> None:
        source = """
pub const IMPLEMENTED_POWERS: [PowerId; 777] = [
    PowerId::Weak,
    PowerId::Artifact,
];
"""
        mutated = self.inject_const_array_entry(
            source,
            "IMPLEMENTED_POWERS",
            "PowerId::Knockdown",
        )
        entries = TRIAGE._parse_kind_list(
            TRIAGE._const_initializer(TRIAGE.lex_rust(mutated), "IMPLEMENTED_POWERS"),
            "PowerId",
        )
        self.assertEqual(entries, ("Knockdown", "Weak", "Artifact"))

    def test_cli_self_test_exercises_clean_and_mutated_results(self) -> None:
        TRIAGE.self_test()

    def test_implemented_glimmer_cannot_hide_its_own_refusal_after_a_prologue(self) -> None:
        self.replace_body(
            "steps/single_pile_selection.rs",
            "glimmer_exact",
            """    if ctx.target.is_some() {
        return Err(EngineRefusal::MalformedArgs("glimmer_exact"));
    }
    Err(EngineRefusal::StepKindNotModeled(
        StepKind::GlimmerExact,
    ))""",
        )
        with self.assertRaisesRegex(
            TRIAGE.TriageError,
            r"steps/single_pile_selection.*implemented_self_refusal=\['GlimmerExact'\]",
        ):
            TRIAGE.scan_tree(self.root)

    def test_implemented_move_cannot_hide_its_own_refusal_in_a_branch(self) -> None:
        self.replace_body(
            "moves/shared.rs",
            "attack",
            """    if ctx.state.history.over {
        return Err(EngineRefusal::MoveKindNotModeled(MoveKind::Attack));
    }
    Ok(())""",
        )
        with self.assertRaisesRegex(
            TRIAGE.TriageError,
            r"moves/shared.*implemented_self_refusal=\['Attack'\]",
        ):
            TRIAGE.scan_tree(self.root)

    def test_an_unimplemented_kind_refusal_is_not_attributed_to_an_implemented_kind(self) -> None:
        self.replace_body(
            "steps/single_pile_selection.rs",
            "glimmer_exact",
            """    if ctx.target.is_some() {
        return Err(EngineRefusal::StepKindNotModeled(StepKind::TutorExact));
    }
    Ok(())""",
        )
        families = TRIAGE.scan_tree(self.root)
        self.assertTrue(
            any(
                family.name == "steps/single_pile_selection"
                and family.implemented == ("GlimmerExact",)
                for family in families
            )
        )


if __name__ == "__main__":
    unittest.main()

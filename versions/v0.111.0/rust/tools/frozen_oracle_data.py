#!/usr/bin/env python3
"""Integrity pins for the Python-oracle outputs the crate keeps as frozen data.

#2827 item D (preparing item F's hard delete of the Python simulator,
``solve_fight.py`` and the Python ``content/`` package, which item F landed). Several crate inputs
were *regenerated from the Python simulator* on every ``rust port`` run and
diffed against the committed bytes: the canonical projection fixture, the
R0.5 slice line, the opening relic-hook manifest and the Mawler admission
replay. Three cargo tests shelled out to ``python3`` for their documents, and
the codegen and encounter census read the Python registry live.

``combat_sim`` has been frozen at v0.111.0 since the #1282 authority flip, so
each of those outputs is already a constant, and item F removes the only way
to regenerate them. Sean's 2026-09-24 decision on #2827: *oracle-derived pins
become frozen data with no Python regeneration path*. This tool is that
decision's check. ``--check`` (standard library, no simulator) verifies the
sha256 of every frozen file below, so a frozen oracle output can change only
by an edit that also changes this table — a reviewable, deliberate event —
never by drift.

#2999 deleted the producers themselves (every generator named below except
``registry_snapshot.py`` and ``encounter_coverage_census.py``, whose Python
recording modes item F removed) and extended this table to the other outputs of the Python
parity and search tooling it retired: the exact-solve and search-stress
corpora, the search experiment's prepared Insatiable root, the opening,
refusal and projection parity reports, the must-cover audit's report, and
the run-counter registries the cargo tests read out of ``solve_fight.py``.
A producer named here as deleted is in git history before #2999.
#2827 item F then froze ``fixtures/run_counters_v1.json`` and deleted its
generator, ``gen_run_counters_pins.py`` (the run-history counter oracle).

Item F deleted the simulator, so nothing can re-derive any file below: the
``--verify-producers`` mode that re-ran the last live producer
(``registry_snapshot.py --verify-replay``) went with it. Every producer named
in the table is now history.

Refreshing a pin (``--write``) is legitimate only for a deliberate,
reviewed hand edit of a frozen file; say so in the PR.
"""

from __future__ import annotations

import argparse
import hashlib
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve()
RUST_DIR = HERE.parents[1]

#: crate-relative path -> (sha256, producer). The producer names what wrote
#: the bytes; for the three former inline cargo-test scripts it is the test
#: whose script, now in git history, produced them.
FROZEN: dict[str, tuple[str, str]] = {
    "fixtures/canonical_state_v2_ironclad_toadpoles.json": (
        "ad2447f1b5f353640db18ecb3c955416c0efa1fd790ddc29c060184882bba973",
        "tools/project_state.py --emit (CLI deleted #2999)",
    ),
    "fixtures/mawler_admission_v1.json": (
        "5dd7e9908ea1564c75d843a06cbd6de0bb7f2f6e9a71e40d13c18a7b448a0873",
        "tools/generate_mawler_admission.py (deleted #2999)",
    ),
    "fixtures/slice_line_v1.json": (
        "be5391e724e7cfe7109cb210b57a479d4f1ba94d87382b40d2c2b98a0c52eda8",
        "tools/gen_slice_pins.py --write (deleted #2999)",
    ),
    "fixtures/opening_relic_hooks_v1.json": (
        "21f0699ae67ff9cf6b69113a6d1f12308be0dc5704c4dd37e384d138d1d22bda",
        "tools/gen_opening_relic_pins.py --write (deleted #2999)",
    ),
    "fixtures/exact_solve_corpus_v1.json": (
        "923169f5c40402aaa8a0a0f1e1750325ca9eaaf4319f244f7f4f4ad4dc408909",
        "the retired Python exact-solve generator; its root builder "
        "tools/exact_solve_corpus.py was deleted #2999",
    ),
    "fixtures/run_counters_v1.json": (
        "257e2f859f62dcb9664876643c211a68383e71f07ea2f998ea19bbf20b60a6c6",
        "tools/gen_run_counters_pins.py (deleted #2827 item F; --check was "
        "fresh against the Python oracle immediately before)",
    ),
    "fixtures/frozen_python_run_counter_registries_v1.json": (
        "42f73944a029f9c2b8fe88c1b455a73b118ed98873135179279cc68f73caaa55",
        "src/run_counters/tests.rs's solve_fight.py registry parser, "
        "frozen #2999",
    ),
    "data/cards_census.json": (
        "a41b5aea92a4ad67197d96c590d2c5acc61f89584f89d4d0a5418cf39ef921ce",
        "a byte copy of solver/cards_census.json, verified by the cargo "
        "test until #2999",
    ),
    "data/card_max_upgrade.json": (
        "234ffb707dae61893c12e97e4cf01671ada91875a8ec9f4c8638762e883d315c",
        "solver/card_templates.json's max_upgrade table, verified by the "
        "cargo test until #2999",
    ),
    "../eval/search/stress-v1.json": (
        "42189645b79fdb86075ddb49198dce7749d6153e4b78d820c56e73be71dee49e",
        "tools/generate_search_stress.py (deleted #2999)",
    ),
    "../eval/search/stress-known-witnesses.json": (
        "94782ffe2c776c1a89737c303b3f211c6cc418f624fb87d626f821624981373d",
        "tools/generate_search_stress.py (deleted #2999)",
    ),
    "../eval/search/generated/insatiable-hp33.json": (
        "fb7aebf0770c1b70e58504156f3f59c8b46945d3f0bba5eb351bd508184c119b",
        "tools/generate_search_stress.py (deleted #2999)",
    ),
    "../eval/search/generated/insatiable-hp16.json": (
        "37b110a34b4f75891a9cac0103ba09287dd28903b6050353b6a27b01f5f1f407",
        "tools/generate_search_stress.py (deleted #2999)",
    ),
    "../eval/search/generated/insatiable-hp8.json": (
        "76a8ba35aab1fc47f0c06c112e75cb627af1ae8ea0957d7ee7e52068e964cb59",
        "tools/generate_search_stress.py (deleted #2999)",
    ),
    "../eval/search/throughput-insatiable.json": (
        "eb95bfde9e85fee3fb937e72e9d5ef65a5539f9ef6f87b600b7437708cf16bb1",
        "tools/replay_throughput.py (deleted #2999)",
    ),
    "benchmarks/2026-09-17-insatiable-search/entry.json": (
        "ee57129024d25fabd7e64dda8ade844e3b56eff9977e0867bf830973a3b3e1f6",
        "tools/search_experiment.py prepare (deleted #2999)",
    ),
    "benchmarks/2026-09-17-insatiable-search/human-replay.json": (
        "91bcf4b51fa875a2b32ce53b4affc08351432885c439f00943c853ca6dfaea36",
        "tools/search_experiment.py prepare (deleted #2999)",
    ),
    "benchmarks/2026-09-17-insatiable-search/native-validation.json": (
        "170bfb0e59e953b99ac87cb085a395d263ec19d113b5924bc26b8daf2bccc956",
        "tools/search_experiment.py prepare (deleted #2999)",
    ),
    "OPENING_CENSUS.md": (
        "f36a67ef3e477147c595d0f962d2d656dbaf88ffc64f4a0c82dbb157319a44cd",
        "tools/opening_census.py (deleted #2999)",
    ),
    "PROJECTION_PARITY_CENSUS.md": (
        "b07082c92bcb00bb3b873ff13a66ffe8bdd2a731009c80702ce6702570567215",
        "tools/projection_parity_census.py --write (deleted #2999)",
    ),
    "REFUSAL_PARITY_CENSUS.md": (
        "01dfc154ac107e0c47d8f2ac7faffd41989668c6af9c2bb8638df2ab3ea5fbad",
        "tools/refusal_parity_census.py --write (deleted #2999)",
    ),
    "REFUSAL_PARITY_CENSUS.json": (
        "37c5425574d583a0801c60beaa285c55161c39588883604cceb5a3770b956be9",
        "tools/refusal_parity_census.py --write (deleted #2999)",
    ),
    "MUST_COVER_DECLARATIONS.md": (
        "03958fa123dcf72707670b190fdc9b5e38a4074efc94d5e86e8b0f684fa55f60",
        "tools/must_cover_declaration_audit.py (deleted #2999)",
    ),
    "must_cover_declaration_debt.json": (
        "3878fd51b5029a5e9f4e081eb23b3c7b24e6fb6e0279a4a61cbaeaf682461dca",
        "tools/must_cover_declaration_audit.py (deleted #2999)",
    ),
    "fixtures/frozen_python_relic_review_roots_v1.json": (
        "f3a596123fffb888a1998f17834161a161743779fa04972cb70b4a852fae93f2",
        "src/engine/admission.rs::"
        "python_projected_relic_states_load_and_admit_through_boundary "
        "(inline script, before #2827 item D)",
    ),
    "fixtures/frozen_python_restlessness_draws_v1.json": (
        "b232bd6946828a4b5d82c3a089f64a93bdb167ea30016723e80eb57a19f68790",
        "src/engine/mod.rs::"
        "python_projected_restlessness_draws_load_and_resume_in_rust "
        "(inline script, before #2827 item D)",
    ),
    "fixtures/frozen_python_tutor_replay_v1.json": (
        "b18018ce44a75919acb8dea6ccd5c500eec577032ba8530d06cef7e683a404f7",
        "src/engine/mod.rs::"
        "python_projected_replayed_tutor_card_uid_loads_and_resumes_in_rust "
        "(inline script, before #2827 item D)",
    ),
    "data/python_registry.v0.111.0.json": (
        "8fe651e87300dcf605b140acf95bd7b2e3334b27b2cf5894685763373d3eb9fa",
        "tools/registry_snapshot.py --write (mode deleted #2827 item F)",
    ),
    "data/encounter_provenance.v0.111.0.json": (
        "9976b7c501c82e783ee1a0a7dca1cf62aafaa9e98d93f1625dd6c1de650c210d",
        "tools/encounter_coverage_census.py --write-provenance "
        "(mode deleted #2827 item F)",
    ),
}

def sha256(path: pathlib.Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def check() -> list[str]:
    findings = []
    for relative, (expected, producer) in sorted(FROZEN.items()):
        path = RUST_DIR / relative
        if not path.is_file():
            findings.append(f"{relative}: missing (producer: {producer})")
            continue
        actual = sha256(path)
        if actual != expected:
            findings.append(
                f"{relative}: sha256 {actual} is not the frozen {expected} "
                f"(producer: {producer}). Frozen Python-oracle data changes "
                "only with a deliberate pin refresh (`--write`) in the same "
                "PR, justified as a v0.111.0 parity-verdict bug fix")
    return findings


def write() -> int:
    source = HERE.read_text()
    changed = 0
    for relative, (expected, _producer) in sorted(FROZEN.items()):
        actual = sha256(RUST_DIR / relative)
        if actual != expected:
            if source.count(expected) != 1:
                raise SystemExit(f"cannot locate the pin for {relative}")
            source = source.replace(expected, actual)
            print(f"refreshed {relative}: {expected[:12]} -> {actual[:12]}")
            changed += 1
    if changed:
        HERE.write_text(source)
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    group = parser.add_mutually_exclusive_group(required=True)
    group.add_argument("--check", action="store_true",
                       help="verify every frozen file's sha256 (no Python "
                            "simulator needed; what CI runs)")
    group.add_argument("--write", action="store_true",
                       help="refresh the sha256 table in this file")
    args = parser.parse_args(argv)
    if args.write:
        return write()
    findings = check()
    for finding in findings:
        print(f"STALE: {finding}", file=sys.stderr)
    if findings:
        return 1
    print(f"{len(FROZEN)} frozen Python-oracle files intact")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

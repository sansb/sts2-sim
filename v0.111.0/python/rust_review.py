"""Snapshot-backed bounded Rust search and Rust-replayed review documents.

Both roots are built by the Rust opening (`sts-sim entry --opening`, #2827),
not by the frozen Python simulator: the floor-snapshot root from a start save
(`--save`), and the replay-capture root from the capture's embedded run
(`--capture-run`, #2972). On the v0.111.0 capture corpus the Rust opening is a
strict admission superset of the Python roots this module used to build, and
every document difference between them is an IL-documented Python error
(#2847, #2954) or a registered Rust-only slot (see the invariant walks
`2026-09-24-issue2827-review-rust-opening.md` and
`2026-09-24-issue2972-capture-run-entry.md`). An opening the Rust port cannot
build exactly is a named refusal. Opening checks and Rust
admission/transactional transition refusals still bind.
All search transitions
and all published action checkpoints come from Rust; no Python action replay
or search is required. Admission and transition refusals remain refusals.
"""
import copy
import json
import subprocess
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "tools"))

import live_coach  # noqa: E402  (the save adapter, `build_entry`)
import relay_parser  # noqa: E402
from rust_replay import (  # noqa: E402
    AFTER_ENERGY_RESET_NATIVE_IDS, RustReplay, check_native_snapshot,
    native_after_energy_reset_powers, recorded_witness, selection_details)
import review_summary as review  # noqa: E402
from rust_exact_solve import canonical_document, default_binary  # noqa: E402


def card_name(card):
    label = card["id"] + ("+" if card.get("upgrade") else "")
    if card.get("enchantment"):
        label += f" [{card['enchantment'][0]} {card['enchantment'][1]}]"
    return label


def snapshot(doc, intents=None):
    # `intent` is the move the monster will act with, which is what the game
    # shows as its intent during the player's turn. It comes from the Rust
    # `intents` query (#2806), which names pattern-driven monsters such as The
    # Insatiable as well as random-table ones, and adds `intent_damage` (the
    # per-hit number the game displays, after Strength, Weak, Vulnerable and
    # the other modifiers) and `intent_hits` only where the engine proves the
    # number exact. Where the query names nothing, `intent` falls back to the
    # projected `next_move` (random-table monsters only), else None, so an
    # older binary still snapshots (Sean, 2026-09-17: "we don't have anything
    # that shows cards drawn and enemy intents"). The frontend humanizes ids.
    monsters = doc.get("monsters", [])
    if intents is not None and (
            len(intents) != len(monsters)
            or any("uid" in m and m["uid"] != i.get("uid") for m, i in zip(monsters, intents))):
        raise ValueError("Rust intents do not match the projected monster roster")
    enemies = []
    for index, m in enumerate(monsters):
        enemy = {"name": m["kind"], "hp": m.get("hp", 0),
                 "maxSeen": m.get("max_hp", m.get("hp", 0)),
                 "intent": m.get("next_move") or None}
        described = intents[index] if intents is not None else {}
        if described.get("intent"):
            enemy["intent"] = described["intent"]
        if described.get("intent_damage") is not None and described.get("intent_hits"):
            enemy["intent_damage"] = described["intent_damage"]
            enemy["intent_hits"] = described["intent_hits"]
        enemies.append(enemy)
    return {"hp": doc["player"].get("hp", 0), "block": doc["player"].get("block", 0),
            "energy": doc["player"].get("energy", 3),
            "hand": [card_name(c) for c in doc["piles"].get("hand", [])],
            "enemies": enemies}


def _intents(ask):
    """The Rust intents for the loaded state, or None from a pre-#2806 binary."""
    try:
        return ask({"cmd": "intents"})["intents"]
    except ValueError as exc:
        if "unknown_command" in str(exc):
            return None
        raise


def _mark_snapshot(state, intents_from=None):
    """A `presentation_marks` visible state (`engine::presentation`) in the
    review snapshot's shape. Intents are read off ``intents_from`` (the
    action's final snapshot) where the roster lines up; a mark carries none."""
    known = (intents_from or {}).get("enemies") or []
    enemies = []
    for index, monster in enumerate(state["monsters"]):
        enemy = {"name": monster["kind"], "hp": monster["hp"],
                 "maxSeen": monster.get("max_hp", monster["hp"]), "intent": None}
        if index < len(known) and known[index]["name"] == monster["kind"]:
            enemy["intent"] = known[index].get("intent")
        enemies.append(enemy)
    return {"hp": state["hp"], "block": state["block"], "energy": state["energy"],
            "hand": [card_name(c) for c in state["hand"]], "enemies": enemies}


def _visible(snapshot):
    return (snapshot["hp"], snapshot["block"],
            [(e["name"], e["hp"]) for e in snapshot["enemies"]])


def split_autoplays(marks, final):
    """The rows an apply's automatic plays add, from its presentation marks.

    Returns ``(parent_state, rows, turn)``, or None when the apply finished no
    automatic play (then nothing is split). ``parent_state`` is the state the
    action itself left: the snapshot at the first automatic play's start.
    ``rows`` is one ``{"kind": "auto", ...}`` per finished play, in the order
    it finished, each carrying the state it left; a last ``{"kind": "auto",
    "source": None}`` row carries whatever the apply did after the last one,
    when that is visible (a turn-start relic, say), so no change is folded
    into a row that did not make it. ``turn`` is the turn the first play ran
    on: past the action's own turn when End turn's next-turn draw fed them.
    """
    begins = [m for m in marks if m.get("kind") == "autoplay_begin" and m.get("state")]
    ends = [m for m in marks if m.get("kind") == "autoplay_end" and m.get("state")]
    if not begins or not ends:
        return None
    rows = [{"kind": "auto", "source": m["source"], "card": card_name(m["card"]),
             "state_after": _mark_snapshot(m["state"], final)} for m in ends]
    if _visible(rows[-1]["state_after"]) != _visible(final):
        rows.append({"kind": "auto", "source": None, "state_after": final})
    else:
        # Nothing visible happened after the last play but the rest of a
        # draw: the last row ends on the hand the apply ended on.
        rows[-1]["state_after"]["hand"] = final["hand"]
    return _mark_snapshot(begins[0]["state"], final), rows, begins[0]["state"]["turn"]


def replay_line(binary, entry, best, final_check=None):
    """Replay a Rust witness in Rust into the turn-grouped play log.

    ``final_check``, when given, is called with the replayed final state
    before the claimed outcome is checked; it raises to withhold the line
    (the legacy path's unknown-stream check, `rust_legacy_review`).
    """
    with RustReplay(binary, entry) as session:
        ask = session.ask
        before, groups = entry, []
        intents = _intents(ask)
        for wire in best["actions"]:
            visible = snapshot(before, intents)
            turn = before["player"].get("turn", 1)
            if not groups or groups[-1]["turn"] != turn:
                # `enemies` is the roster as the turn opened, before its first
                # action: the intents the player saw when choosing the line.
                groups.append({"turn": turn, "hp": visible["hp"], "block": visible["block"],
                               "hand": visible["hand"], "enemies": visible["enemies"],
                               "actions": []})
            applied = ask({"cmd": "apply", "action": wire, "describe_selection": wire["kind"] == "select",
                           "presentation_marks": True})
            after = ask({"cmd": "project"})["state"]
            after_intents = _intents(ask) if intents is not None else None
            final = snapshot(after, after_intents)
            action = {"kind": wire["kind"], "wire": wire, "digest_after": applied["digest"],
                      "state_after": final}
            # Cards the game played on its own inside this apply (Hellraiser's
            # drawn Strikes) are rows of their own, off the `actions` list so
            # a decision's (turn, k) address never moves. Plays fed by End
            # turn's next-turn draw open the NEXT turn's group; any others
            # follow the action that caused them.
            split = split_autoplays(applied.get("presentation_marks") or [], final)
            next_turn_rows = None
            if split is not None:
                action["state_after"], rows, auto_turn = split
                if auto_turn != turn:
                    next_turn_rows = (auto_turn, rows)
                else:
                    action["autoplays"] = rows
            if wire["kind"] == "play":
                card = next(c for c in before["piles"]["hand"] if c["uid"] == wire["uid"])
                action["card"] = card_name(card)
                if wire.get("selection") is not None:
                    selected = next(c for c in before["piles"]["hand"] if c["uid"] == wire["selection"])
                    action["exhaust"] = card_name(selected)
                if wire.get("target") is not None:
                    target = before["monsters"][wire["target"]]
                    action["target"] = f"{target.get('hp', 0)}hp {target['kind']}"
            elif wire["kind"] == "potion":
                action["potion"] = before["player"]["potion_slots"][wire["slot"]]
                if wire.get("target") is not None:
                    target = before["monsters"][wire["target"]]
                    action["target"] = f"{target.get('hp', 0)}hp {target['kind']}"
            elif wire["kind"] == "select":
                details = selection_details(before, wire, after, applied.get("selected_uids"))
                if details:
                    action.update({k: v for k, v in details.items() if k != "uids"})
                else:
                    answer = wire.get("answer", {})
                    action["choice"] = (f"Option {answer['index'] + 1}" if "index" in answer
                                        else "Recorded selection")
            groups[-1]["actions"].append(action)
            if next_turn_rows is not None:
                auto_turn, rows = next_turn_rows
                groups.append({"turn": auto_turn, "hp": final["hp"], "block": final["block"],
                               "hand": final["hand"], "enemies": final["enemies"],
                               "autoplays": rows, "actions": []})
            before, intents = after, after_intents
        if final_check is not None:
            final_check(before)
        digest = canonical_document.differential_digest(before)
        won = before["player"].get("over", False) and before["player"].get("hp", 0) > 0 and all(
            m.get("hp", 0) <= 0 for m in before.get("monsters", []))
        if (digest != best["final_digest"] or bool(won) != best["won"] or
                before["player"].get("hp", 0) != best["combat_hp"]):
            raise ValueError("Rust retained witness did not replay to its claimed outcome")
        return groups


# Moved to `rust_replay` so the eval census reads the same native order
# without importing this module's save adapter (#3020).
_AFTER_ENERGY_RESET_NATIVE_IDS = AFTER_ENERGY_RESET_NATIVE_IDS
_native_after_energy_reset_powers = native_after_energy_reset_powers


def _rust_opening(binary, save, fight, build, *, source="--save",
                  native_checkpoints=False):
    """`sts-sim entry --opening` on one run: the whole combat-entry root.

    `source` names the input schema: `--save` for a schema-20 start save,
    `--capture-run` for a replay capture's decoded embedded run (#2972).
    The document it returns is `sts-sim-canonical-v2`, the same bytes Rust
    search loads. An entry or opening refusal is a normal answer on stdout and
    is raised here by name; a non-zero exit is an argv contract failure.

    `native_checkpoints` adds `--native-checkpoints` (#3392) and returns
    `(document, recorded)`, where `recorded` is the opening's own native
    checkpoint list (`None` when the opening reported none).
    """
    if source not in ("--save", "--capture-run"):
        raise ValueError(f"unknown Rust entry input {source!r}")
    with tempfile.TemporaryDirectory(prefix="sts-rust-opening-") as directory:
        path = Path(directory) / "entry.json"
        path.write_text(json.dumps(save))
        completed = subprocess.run(
            [str(binary), "entry", "--build", build, source, str(path),
             "--encounter", fight.encounter_id, "--node-type", fight.node_type,
             "--opening", *(("--native-checkpoints",) if native_checkpoints else ())],
            text=True, capture_output=True, timeout=60, check=False)
    if completed.returncode:
        raise ValueError("Rust opening CLI failed: " + completed.stderr[-500:])
    document = json.loads(completed.stdout)
    recorded = None
    if native_checkpoints and document.get("schema") == _OPENING_CHECKPOINTS_SCHEMA:
        document, recorded = document.get("state"), document.get("native_checkpoints")
        if not isinstance(document, dict):
            raise ValueError("Rust opening checkpoints carry no root")
    if document.get("schema") != "sts-sim-canonical-v2":
        opening = document.get("opening") or {}
        refusal = document.get("refusal") or {}
        kind = (opening.get("refusal_class") or document.get("refusal_class")
                or "entry_refused")
        detail = opening.get("detail") or refusal.get("detail") or ""
        raise ValueError(f"Rust opening refused: {kind}: {detail[:300]}")
    return (document, recorded) if native_checkpoints else document


def _load_refusal(binary, document):
    """The refusal `sts-sim diff-serve` gives this document, or None."""
    completed = subprocess.run(
        [str(binary), "diff-serve"],
        input=json.dumps({"cmd": "load", "entry": document}) + "\n"
        + json.dumps({"cmd": "quit"}) + "\n",
        text=True, capture_output=True, timeout=60, check=False)
    lines = [line for line in completed.stdout.splitlines() if line.strip()]
    if len(lines) < 2:
        raise ValueError("diff-serve answered no load line: "
                         + completed.stderr[-300:])
    answer = json.loads(lines[1])
    if "ok" in answer:
        return None
    return json.dumps(answer.get("refusal", answer))


def _with_entry_reset_witness(binary, document):
    """Carry the entry's empty AfterEnergyReset listener order as a fact.

    Without `player.after_energy_reset_order` the engine treats the order as
    legacy-unknown and later refuses a mid-fight listener whose order it
    cannot place. The Python adapter wrote `[]` exactly when no listener was
    live at entry. The boundary is the authority on that claim: it rejects an
    order that does not exactly match the live listeners (`boundary.rs`,
    "must exactly match live reset listeners"), so `[]` is proposed and kept
    unless that specific rejection comes back, in which case the root stays
    legacy-unknown, as the Python adapter left it. Any other load refusal is
    search's to report on the witnessed root.
    """
    candidate = copy.deepcopy(document)
    candidate["player"]["after_energy_reset_order"] = []
    refusal = _load_refusal(binary, candidate)
    if refusal is not None and "after_energy_reset_order" in refusal:
        return document
    return candidate


def build_root(run_path, fight_index, saves, binary=None):
    raw = json.loads(Path(run_path).read_text())
    run = relay_parser.parse_run(str(run_path))
    review.require_admitted(run.build_id)
    if run.build_id != "v0.111.0" or not 0 <= fight_index < len(run.fights):
        raise ValueError("snapshot review build or fight unsupported")
    fight = run.fights[fight_index]
    binary = binary or default_binary()
    if binary is None:
        raise ValueError("Rust solver executable is unavailable")
    roots = []
    if not isinstance(saves, list) or not 1 <= len(saves) <= 8:
        raise ValueError("expected one to eight captured start snapshots")
    for item in saves:
        save = item["save"]
        if (item["game_build"] != run.build_id or save["rng"]["seed"] != run.seed or
                save["start_time"] != raw["start_time"]):
            raise ValueError("snapshot belongs to another run or build")
        facts = live_coach.build_entry(save, fight.encounter_id, fight.node_type)
        if facts["node_index"] != fight.node_index:
            raise ValueError("snapshot belongs to another floor")
        roots.append(_with_entry_reset_witness(
            binary, _rust_opening(binary, save, fight, run.build_id)))
    if len({canonical_document.differential_digest(root) for root in roots}) != 1:
        raise ValueError("start snapshots disagree; an exact root cannot be selected")
    return run, fight, roots[0]


_OPENING_CHECKPOINT = "After player turn start"
_OPENING_CHECKPOINTS_SCHEMA = "sts-sim-opening-checkpoints-v1"
_AFTER_PLAYER_TURN_START = "after_player_turn_start"


def _opening_checkpoint_state(document, recorded):
    """The Rust state the capture's opening checkpoint is compared against.

    Native writes "After player turn start" BEFORE turn one's
    `RunAutoPrePlayPhase` (`CombatManager/<StartTurn>d__100::MoveNext` RVA
    0x3f781c IL_096f/IL_0975, then IL_0b1e), while the root is post-AutoPre:
    an Imbued AutoPlay (#3381) or Whispering Earring's loop (#3414) has run
    in it. So the checkpoint is compared against the opening's recorded
    `after_player_turn_start` state, exactly as the census does
    (`eval_suite.opening_checkpoint_state`, #3392). With nothing recorded (a
    SetupPlayerTurn that paused on a choice, where native took the checkpoint
    at that pause) the root is the state compared. A malformed report raises.
    """
    if recorded is None:
        return document
    if not isinstance(recorded, list):
        raise ValueError("the opening did not report its native checkpoints")
    states = {}
    for entry in recorded:
        kind = entry.get("kind") if isinstance(entry, dict) else None
        if kind != _AFTER_PLAYER_TURN_START or kind in states:
            raise ValueError(f"unexpected opening native checkpoint {kind!r}")
        if not isinstance(entry.get("state"), dict):
            raise ValueError(f"opening native checkpoint {kind} does not project")
        states[kind] = entry["state"]
    return states.get(_AFTER_PLAYER_TURN_START, document)


def _check_native_enchantments(document, native_player):
    """Each pile's enchantments, card by card, against the native checkpoint.

    `rust_replay.check_native_snapshot` compares card ids and upgrade levels;
    the Python-rooted opening check this replaces also compared enchantments
    (`mcr_validate._native_card_projection`), so that comparison stays.
    """
    for pile in native_player["piles"]:
        modeled = [tuple(card["enchantment"]) if card.get("enchantment") else None
                   for card in document["piles"].get(pile["pile_type"].lower(), [])]
        native = [(state["card"]["enchantment"]["id"], state["card"]["enchantment"]["level"])
                  if state["card"].get("enchantment") else None
                  for state in pile["cards"]]
        if modeled != native:
            raise ValueError("Rust opening enchantments differ from native checkpoint")


def _replay_opening(binary, capture_run, fight, build, checkpoint):
    """The Rust opening from a capture's embedded run, checked and witnessed.

    `sts-sim entry --capture-run` maps the decoded run onto the save surface
    exactly (#2972) and runs the same opening `--save` does. The opening is
    then compared with the capture's first native checkpoint, which the game
    wrote after the first turn started and before any player input: player
    resources, the live monster roster, every pile's card ids, upgrades and
    enchantments, and all nine combat RNG streams.

    That checkpoint's hero power list is also the native `AfterEnergyReset`
    order (`_native_after_energy_reset_powers`). The opening must hold exactly
    those listeners at exactly those amounts, and the root then carries the
    order as `player.after_energy_reset_order`. The boundary checks that order against
    the live listeners ("must exactly match live reset listeners"). On this
    path the order is observed rather than proposed, so a boundary rejection
    of it is a refusal, not a reason to fall back to legacy-unknown.
    """
    document, recorded = _rust_opening(
        binary, capture_run, fight, build, source="--capture-run",
        native_checkpoints=True)
    compared = _opening_checkpoint_state(document, recorded)
    check_native_snapshot(compared, checkpoint)
    _check_native_enchantments(compared, checkpoint["players"][0])
    hero = next(c for c in checkpoint["creatures"] if c.get("player_id") is not None)
    native = _native_after_energy_reset_powers(hero)
    # The six listeners' amounts, as the Python-rooted check compared them.
    # Each is a canonical player slot under its own name (`boundary.rs`).
    modeled = {name: compared["player"].get(name, 0)
               for name in _AFTER_ENERGY_RESET_NATIVE_IDS.values()}
    if {name: amount for name, amount in modeled.items() if amount} != dict(native):
        raise ValueError("Rust opening reset listeners differ from native checkpoint")
    order = [name for name, _ in native]
    if compared is not document:
        # Turn one's AutoPre ran after the checkpoint (#3414): a listener it
        # acquired joins `Creature.Powers` after every checkpoint power, in
        # acquisition order, which is the order the root already carries for
        # listeners acquired after the opening.
        order += [name for name in document["player"].get("after_energy_reset_order") or ()
                  if name not in order]
    document = copy.deepcopy(document)
    document["player"]["after_energy_reset_order"] = order
    refusal = _load_refusal(binary, document)
    if refusal is not None and "after_energy_reset_order" in refusal:
        raise ValueError("Rust boundary rejected the native AfterEnergyReset order: "
                         + refusal[:300])
    return document


def replay_capture_run(run_path, fight_index, replay, projection, lineage=()):
    """The capture run `sts-sim entry --capture-run` roots, as `(run, fight, capture)`.

    The capture's embedded run is taken as-is, except that the separately
    resolved save projection supplies unlock provenance only. The capture
    supplies actual HP, deck order, potion capacity and all nine RNG streams;
    no run-history counter predictions enter this path. The eval suite's
    upload import (#2915) writes this document beside the capture, so prod
    reviews and the certification root an upload from the same input.

    ``lineage`` is a branch run's ancestry (#3373): a pre-branch fight's
    capture carries the SOURCE run's ``start_time``, admitted by exactly the
    rule ``review_provenance.analyze_bundle`` associated it under.
    """
    import review_provenance
    import review_summary_v2 as adapter

    raw = json.loads(Path(run_path).read_text())
    run = relay_parser.parse_run(str(run_path))
    review.require_admitted(run.build_id)
    if run.build_id != "v0.111.0" or replay.get("version") != run.build_id:
        raise ValueError("replay belongs to another build")
    if not 0 <= fight_index < len(run.fights):
        raise ValueError("replay fight index out of range")
    fight = run.fights[fight_index]
    saved = replay["run"]
    if (saved["rng"]["seed"] != run.seed
            or not review_provenance._encounter_matches(replay, fight)
            or review_provenance._history_depth(saved) != fight.node_index
            or not review_provenance.capture_identity_admitted(
                saved["start_time"], raw["start_time"], fight.node_index,
                lineage)):
        raise ValueError("replay belongs to another run or fight")
    if review_provenance.candidate_counter_map({"entry": projection}) != review_provenance._replay_counter_map(saved):
        raise ValueError("entry projection does not match replay RNG counters")
    # Existing complete-envelope, character, deck/relic and persistent-field
    # validation remains binding. It also resolves captured upgrade copies.
    adapter.provenance_entry_for_fight(
        fight, projection, expected_character=run.character, replay=replay)
    player = adapter._complete_projected_player(projection)
    capture = copy.deepcopy(saved)
    if len(capture["players"]) != 1:
        raise ValueError("replay requires one player")
    capture["players"][0]["unlock_state"] = copy.deepcopy(player["unlock_state"])
    # MCR uses PascalCase stream names; the save adapter uses snake_case.
    # CombatOrbs is the native capture spelling (not CombatOrbGeneration).
    names = {**live_coach.STREAMS, "combat_orbs": "CombatOrbs"}
    rng = saved["rng"]
    streams = {}
    for name, native in names.items():
        words, counter = rng["states"][native], rng["counters"][native]
        if (not isinstance(words, list) or len(words) != 4
                or any(type(w) is not int or not 0 <= w < 2**64 for w in words)
                or type(counter) is not int or counter < 0):
            raise ValueError("malformed captured RNG stream: " + native)
        streams[name] = {"counter": counter, **dict(zip(("s0", "s1", "s2", "s3"), words))}
    if live_coach.verify_stream_seeding({"rng": {"seed": run.seed, "rngs": streams}}, run.build_id):
        raise ValueError("captured RNG states disagree with seed/counters")
    return run, fight, capture


def build_replay_root(run_path, fight_index, replay, projection, binary=None,
                      lineage=()):
    """Use the MCR's entry snapshot even when its action suffix is missing.

    The root is the Rust opening built from the capture's embedded run
    (`sts-sim entry --capture-run --opening`, #2972) and checked against the
    capture's first native checkpoint (`_replay_opening`).
    """
    run, fight, capture = replay_capture_run(
        run_path, fight_index, replay, projection, lineage)
    checksums = replay.get("checksums") or []
    if (not checksums or checksums[0].get("context") != _OPENING_CHECKPOINT
            or not checksums[0].get("full_state")):
        raise ValueError("capture carries no opening checkpoint")
    binary = binary or default_binary()
    if binary is None:
        raise ValueError("Rust solver executable is unavailable")
    return run, fight, _replay_opening(
        binary, capture, fight, run.build_id, checksums[0]["full_state"])


def capped_heal_band(fight):
    """Combat-end HPs consistent with a win whose ordinary post-combat relic
    heal ended at the max-HP cap (#1166), or None when that is not the case.

    `review_summary.actual_outcome` refuses this composition from `.run`
    alone. After removing the max-HP gains it removes first, the recorded
    endpoint is `min(c + heal, max_hp) == max_hp`, so the combat-end HP `c`
    lies in `[max_hp - heal, max_hp]`. Only an exact recorded replay may pick
    the value inside that band.
    """
    if fight.hp_after <= 0:
        return None
    gain = sum(amount for relic, amount in review.POST_COMBAT_MAX_HP_GAINS.items()
               if relic in fight.relics_entering)
    heal = sum(amount for relic, amount in review.POST_COMBAT_RELIC_HEALS.items()
               if relic in fight.relics_entering)
    if not heal or fight.hp_after - gain != fight.max_hp_entering:
        return None
    return (max(1, fight.max_hp_entering - heal), fight.max_hp_entering)


def generate(run_path, fight_index, saves, config, *, provenance_entry=None,
             branch_lineage=()):
    started = time.monotonic()
    identity = {"fight_index": fight_index}
    try:
        binary = config.rust_exact_solver_binary or default_binary()
        if binary is None:
            raise ValueError("Rust solver executable is unavailable")
        if saves is not None:
            run, fight, entry = build_root(run_path, fight_index, saves, binary)
        else:
            run, fight, entry = build_replay_root(
                run_path, fight_index, config.recorded_replay, provenance_entry, binary,
                branch_lineage)
        identity.update(floor=fight.node_index + 1, encounter_id=fight.encounter_id,
                        node_type=fight.node_type, run_seed=run.seed)
        outcomes, diagnostics = search_outcomes(binary, entry, config)
        if not outcomes:
            refusal = next((d["first_refusal"] for d in diagnostics if d.get("first_refusal")), None)
            if refusal is not None:
                # Every playout that reached a terminal crossed a refusal.
                raise ValueError("Rust search refused: " + json.dumps(refusal))
            raise ValueError("search budget reached without a terminal witness")
        try:
            actual = review.actual_outcome(fight)
            ambiguous = None
        except review.ReviewRefusal as refusal:
            # A capped heal is settled only by an exact recorded replay.
            band = capped_heal_band(fight)
            if band is None or config.recorded_replay is None:
                raise
            ambiguous = refusal
            actual = {"won": True, "entry_hp": fight.hp_entering, "final_hp": None,
                      "final_hp_range": band,
                      "potions_used": {"count": len(fight.potions_used),
                                       "names": list(fight.potions_used)},
                      "turns": fight.turns_taken}
        witness = None
        if ambiguous is not None:
            try:
                witness = recorded_witness(binary, entry, config.recorded_replay, actual)
            except (ValueError, KeyError, IndexError, TypeError, NotImplementedError, OSError,
                    subprocess.SubprocessError):
                raise ambiguous from None
            band = actual.pop("final_hp_range")
            actual["final_hp"] = witness["combat_hp"]
        actual["hp_lost"] = actual["entry_hp"] - actual["final_hp"]
        document = {"schema_version": 8, "status": "ok", "fight": identity,
            "actual": actual, "best_actual_seed": outcomes[0], "alternative_actual_seeds": outcomes[1:],
            "metadata": {"game_build": run.build_id, "simulator": review.simulator_document(run.build_id, binary),
                "trust_tier": "modeled", "entry_state_source": "captured_floor_start" if saves else "captured_replay_start",
                "opening_validation": "not_checked" if saves else "native_resources_pile_identities_and_rng",
                "opening_adapter": "rust_entry_opening", "search_engine": "rust", "replay_engine": "rust",
                "entry_digest": canonical_document.differential_digest(entry),
                "snapshots": [{k: item[k] for k in ("id", "sha256")} for item in (saves or [])],
                "objective": "win, then combat HP; potion usage is reported separately",
                "searches": diagnostics, "elapsed_seconds": time.monotonic() - started,
                "native_full_checksum": "not_checked", "in_game_verification": "pending"}}
        record_potion_belt(document, entry, config)
        if ambiguous is not None:
            document["metadata"]["actual_final_hp_source"] = {
                "source": "recorded_replay", "reason": "capped_postcombat_heal",
                "band": list(band)}
        if config.recorded_replay is not None:
            try:
                witness = witness or recorded_witness(binary, entry, config.recorded_replay, actual)
                document["recorded_line"] = replay_line(binary, entry, witness)
                document["metadata"]["recorded_replay"] = {
                    "status": "complete", "engine": "rust",
                    "final_digest": witness["final_digest"],
                    "validation": "all_inputs_and_recorded_outcome",
                    "native_checkpoints": witness["native_checkpoints"]}
            except (ValueError, KeyError, IndexError, TypeError, NotImplementedError, OSError, subprocess.SubprocessError) as exc:
                document["metadata"]["recorded_replay"] = {"status": "unavailable", "reason": str(exc)[:500]}
        return document
    except (ValueError, KeyError, TypeError, NotImplementedError, OSError, subprocess.SubprocessError) as exc:
        return {"schema_version": 8, "status": "refused", "fight": identity,
                "refusal": {"reason": "rust_snapshot_review_refused", "message": str(exc)},
                "metadata": {"simulator": review.simulator_document("v0.111.0", config.rust_exact_solver_binary),
                             "search_engine": "rust", "elapsed_seconds": time.monotonic() - started}}


# The one passive potion: the engine never offers it as a drink (only its own
# ShouldDie hook consumes it), so it is never counted as drinkable.
PASSIVE_POTIONS = frozenset({"FAIRY_IN_A_BOTTLE"})


def potion_holdback(entry, config):
    """The search's potion restriction for ``config``, or None for none.

    ``hold_count`` keeps at least that many of the entry belt's drinkable
    potions: the search may drink at most ``drinkable - hold_count`` times,
    and a potion gained mid-fight spends that budget when drunk. Each held
    slot must name a drinkable entry potion. A holdback the belt cannot honour
    (more held than it has) is a ValueError, i.e. a refused review.
    """
    count = getattr(config, "potion_hold_count", 0)
    slots = tuple(getattr(config, "potion_hold_slots", ()))
    if not count and not slots:
        return None
    belt = entry["player"].get("potion_slots") or []
    drinkable = [i for i, name in enumerate(belt)
                 if name is not None and name not in PASSIVE_POTIONS]
    for slot in slots:
        if slot not in drinkable:
            raise ValueError(f"potion holdback: slot {slot} holds no drinkable potion "
                             f"(belt {belt!r})")
    if count > len(drinkable):
        raise ValueError(f"potion holdback: cannot keep {count} of "
                         f"{len(drinkable)} drinkable potions")
    return {"hold_count": count, "hold_slots": sorted(slots),
            "held_potions": [belt[s] for s in sorted(slots)],
            "max_drinks": len(drinkable) - count}


def record_potion_belt(document, entry, config):
    """Record the entry belt, and any holdback, in the document's metadata.

    The site offers a holdback re-solve from ``entry_potion_slots``: slot
    indices are what ``--hold-slot`` names, and an empty or absent belt is
    what tells it there is nothing to hold.
    """
    document["metadata"]["entry_potion_slots"] = list(entry["player"].get("potion_slots") or [])
    holdback = potion_holdback(entry, config)
    if holdback is not None:
        document["metadata"]["potion_holdback"] = holdback


def potion_holdback_argv(holdback):
    """The ``sts-sim search`` flags for a :func:`potion_holdback` result."""
    if holdback is None:
        return []
    # A slots-only holdback needs no budget: held slots are never drunk.
    argv = ["--max-potions", str(holdback["max_drinks"])] if holdback["hold_count"] else []
    for slot in holdback["hold_slots"]:
        argv += ["--hold-slot", str(slot)]
    return argv


def search_outcomes(binary, entry, config, *, final_check=None, withheld=None):
    """Bounded Rust search (UCT, then random) and each witness's Rust replay.

    Returns ``(outcomes, diagnostics)``: ``outcomes`` ranked best first, each
    an achieved lower bound with its replayed line. A witness whose replay
    ``final_check`` rejects is not an outcome; the rejection is appended to
    ``withheld`` (``{"line_id", "reason"}``) when a list is given.
    """
    budget = config.actual_deadline if config.actual_deadline is not None else 30.0
    outcomes = []
    # Whether each outcome's line is a Battleworn Dummy timeout (#3369): the
    # search reports it beside `won`. It ranks the outcomes and stays out of
    # the review document, whose schema is unchanged.
    forfeits = {}
    diagnostics = []
    holdback = potion_holdback(entry, config)
    with tempfile.TemporaryDirectory(prefix="sts-rust-review-") as directory:
        path = Path(directory) / "entry.json"
        path.write_text(json.dumps(entry))
        for method in ("uct", "random"):
            completed = subprocess.run([str(binary), "search", str(path), method, "2", str(budget / 2), "1.414", "0.05", str(2**64 - 1), str(config.horizon), *potion_holdback_argv(holdback)],
                text=True, capture_output=True, timeout=budget / 2 + 15)
            if completed.returncode:
                raise ValueError("Rust search refused: " + completed.stderr[-2000:])
            result = json.loads(completed.stdout)
            if result["entry_digest"] != canonical_document.differential_digest(entry):
                raise ValueError("Rust search root mismatch")
            best = result.get("best")
            diagnostics.append({key: value for key, value in result.items() if key not in ("best", "improvements")})
            if best is None:
                continue
            if final_check is None:
                line = replay_line(binary, entry, best)
            else:
                try:
                    line = replay_line(binary, entry, best, final_check)
                except final_check.withholds as exc:
                    if withheld is not None:
                        withheld.append({"line_id": method, "reason": str(exc)[:300],
                                         "stream": getattr(exc, "stream", None)})
                    continue
            forfeits[method] = bool(best.get("battleworn_timeout", False))
            outcomes.append({"line_id": "uct" if method == "uct" else "random",
                    "display_name": f"{best['combat_hp']} HP · {best['turn']} turns · {method.upper()}",
                    "line": line, "result": "win" if best["won"] else "loss", "won": best["won"],
                    "final_hp": best["combat_hp"], "hp_lost": entry["player"]["hp"] - best["combat_hp"],
                    "potions_used": sum(a["kind"] == "potion" for a in best["actions"]), "turns": best["turn"],
                    "exact": False, "deadline_hit": True, "bound": "achieved_lower_bound",
                    "claim": {"kind": "achieved", "display": "achieved (lower bound)"},
                    "verification_note": "Replayed in the Rust simulator; not yet verified in game.",
                    "verification": {"rust_replay": True, "in_game": "pending", "entry_digest": result["entry_digest"],
                                     "final_digest": best["final_digest"]}})
    outcomes.sort(key=lambda o: outcome_rank(o, forfeits[o["line_id"]]), reverse=True)
    return outcomes, diagnostics


def outcome_rank(outcome, battleworn_timeout):
    """Sort key for achieved outcomes, best highest.

    A win first. Then (#3369, Sean 2026-09-27) a win that keeps its event
    reward over a Battleworn Dummy timeout at any HP: the timer's escape is a
    won combat, but the event pays only on a kill. Then HP, fewer potions,
    fewer turns. `battleworn_timeout` is false outside that fight, so every
    other fight ranks exactly as before.
    """
    return (outcome["won"], not battleworn_timeout, outcome["final_hp"],
            -outcome["potions_used"], -outcome["turns"])

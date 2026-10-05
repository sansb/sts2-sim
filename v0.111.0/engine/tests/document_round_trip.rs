//! #3674 — a document the engine projects reloads as the state that
//! projected it.
//!
//! The census certifies play: every recorded action against the game's own
//! checkpoints. It never reloads a document, so a state that projects,
//! loads, and projects back *differently* passes it. Two such shapes sat in
//! 143 census projections, in fights that all certify:
//!
//! * `player.next_creature_uid` came back one higher whenever the document
//!   also held `osty_corpse`. The loader decoded the corpse through the
//!   engine's pet-creation edge, which advances a tracked creature counter.
//!   Native creates the Osty creature once
//!   (`OstyCmd/<Summon>d__0::MoveNext` `0x3ee040` `IL_01e9`), and a dead
//!   Osty is still that creature, so the session's counter was right and the
//!   reloaded one was not: the next spawned enemy would have taken a uid one
//!   above native's.
//! * a Parafright that had died and revived held Strength with no
//!   `power_attachments` row in the session, and an `unknown` row after a
//!   reload. Native keeps the illusion's `StrengthPower` instance through
//!   its death (`IllusionPower::ShouldPowerBeRemovedOnDeath` `0xa3a80`), so
//!   here the session was the wrong side: it now keeps the row.
//!
//! The first four tests are the guard: every stored fixture, every step.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sts_sim::boundary::HotBoundary;
use sts_sim::canonical::CanonicalStateV2;
use sts_sim::catalog::Catalog;
use sts_sim::engine::{self, Action};
use sts_sim::exact_solve_v1::ExactSolveActionV1;
use sts_sim::hot::HotState;

/// The only reason a document the fixtures project may fail to load: a
/// receipt parked on a predecessor `engine::admit` refuses (#3644's second
/// row, tracked there).
const PARKED_ON_AN_UNADMITTED_PREDECESSOR: &str = "ActionReplay predecessor is not admitted";

fn fights() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../eval/fights")
}

fn hydrate(document: &CanonicalStateV2) -> Result<(Catalog, HotState), String> {
    let catalog = HotBoundary::catalog_from_canonical(document).map_err(|e| e.to_string())?;
    let state = HotBoundary::from_canonical(document, &catalog).map_err(|e| e.to_string())?;
    Ok((catalog, state))
}

fn action(wire: &Value) -> Action {
    let wire: ExactSolveActionV1 = serde_json::from_value(wire.clone()).unwrap();
    Action::try_from(wire).unwrap()
}

/// A stored fixture's entry and recorded line, or `None` for a fixture that
/// stores no line (a refusal fixture).
fn fixture(id: &str) -> Option<(CanonicalStateV2, Value)> {
    let dir = fights().join(id);
    let line = std::fs::read(dir.join("human_line.json")).ok()?;
    let entry = std::fs::read(dir.join("entry.canonical.json")).unwrap();
    Some((
        serde_json::from_slice(&entry).unwrap(),
        serde_json::from_slice(&line).unwrap(),
    ))
}

/// Replay a fixture's recorded line. At every step the projected document
/// must carry the stored digest, and must either reload and project back to
/// itself or refuse to load under the one named reason.
///
/// `visit` sees each step's document and, where it loaded, the reloaded
/// state. Returns how many steps loaded and how many refused.
fn replay(
    id: &str,
    entry: &CanonicalStateV2,
    line: &Value,
    mut visit: impl FnMut(usize, &CanonicalStateV2, Option<(&Catalog, &HotState)>),
) -> (usize, usize) {
    let (catalog, mut state) = hydrate(entry).unwrap_or_else(|error| panic!("{id}: {error}"));
    let actions = line["actions"].as_array().unwrap();
    let digests = line["step_digests"].as_array().unwrap();
    assert_eq!(actions.len(), digests.len(), "{id}");
    let mut events = Vec::new();
    let (mut loaded, mut refused) = (0, 0);
    for (index, wire) in actions.iter().enumerate() {
        events.clear();
        state = engine::apply_action_into(&state, &catalog, &action(wire), &mut events)
            .unwrap_or_else(|error| panic!("{id} step {index}: {error}"));
        let document = HotBoundary::try_to_canonical(&state, &catalog)
            .unwrap_or_else(|error| panic!("{id} step {index}: {error}"));
        assert_eq!(
            Value::String(document.differential_digest()),
            digests[index],
            "{id} step {index}: the session's document moved",
        );
        match hydrate(&document) {
            Ok((reload_catalog, reloaded)) => {
                let again = HotBoundary::try_to_canonical(&reloaded, &reload_catalog)
                    .unwrap_or_else(|error| panic!("{id} step {index}: {error}"));
                assert!(
                    again == document,
                    "{id} step {index} ({wire}): the document loads and projects back \
                     differently\nsession:  {}\nreloaded: {}",
                    serde_json::to_string(&document).unwrap(),
                    serde_json::to_string(&again).unwrap(),
                );
                visit(index, &document, Some((&reload_catalog, &reloaded)));
                loaded += 1;
            }
            Err(error) => {
                assert!(
                    error.contains(PARKED_ON_AN_UNADMITTED_PREDECESSOR),
                    "{id} step {index} ({wire}): {error}",
                );
                visit(index, &document, None);
                refused += 1;
            }
        }
    }
    (loaded, refused)
}

/// The guard, one quarter of the stored fixtures: every step of each
/// recorded line reloads and projects back identically, or refuses to load
/// by name. Four tests rather than one so the walk uses four cores (about
/// 75 s of work in the dev profile).
fn every_document_reloads_as_itself(shard: usize) {
    let mut ids: Vec<String> = std::fs::read_dir(fights())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    ids.sort();
    let (mut lines, mut loaded, mut refused) = (0, 0, 0);
    let mut refusing: BTreeMap<&str, usize> = BTreeMap::new();
    for (_, id) in ids
        .iter()
        .enumerate()
        .filter(|(index, _)| index % 4 == shard)
    {
        let Some((entry, line)) = fixture(id) else {
            continue;
        };
        let (ok, no) = replay(id, &entry, &line, |_, _, _| {});
        lines += 1;
        loaded += ok;
        refused += no;
        if no != 0 {
            refusing.insert(id, no);
        }
    }
    // Not vacuous: the tree holds about 600 lines and 14,000 steps, and
    // nearly every step reloads. The exact totals move with the fixture set,
    // so only a floor is pinned.
    assert!(lines >= 100, "shard {shard}: {lines} fixture lines");
    assert!(
        loaded >= 2_500,
        "shard {shard}: {loaded} documents reloaded"
    );
    assert!(
        refused * 50 < loaded,
        "shard {shard}: {refused} of {} documents refuse to load: {refusing:?}",
        loaded + refused,
    );
}

#[test]
fn every_document_a_stored_fixture_projects_reloads_as_itself_0() {
    every_document_reloads_as_itself(0);
}

#[test]
fn every_document_a_stored_fixture_projects_reloads_as_itself_1() {
    every_document_reloads_as_itself(1);
}

#[test]
fn every_document_a_stored_fixture_projects_reloads_as_itself_2() {
    every_document_reloads_as_itself(2);
}

#[test]
fn every_document_a_stored_fixture_projects_reloads_as_itself_3() {
    every_document_reloads_as_itself(3);
}

fn steps_holding(id: &str, holds: impl Fn(&CanonicalStateV2) -> bool) -> Vec<usize> {
    let (entry, line) = fixture(id).unwrap();
    let mut steps = Vec::new();
    replay(id, &entry, &line, |index, document, reloaded| {
        if holds(document) {
            assert!(reloaded.is_some(), "{id} step {index} loads");
            steps.push(index);
        }
    });
    steps
}

/// The first shape, on the documents the census projected it from: a dead
/// Osty beside a tracked creature counter, with every enemy still standing
/// (the Knights, the Ruby Raiders beside a Byrdpip, the Ceremonial Beast).
/// `replay` asserts each reloads as itself; this pins that the fixtures
/// still reach the shape.
#[test]
fn a_retained_osty_corpse_reloads_with_the_session_creature_counter() {
    let corpse_beside_a_tracked_counter = |document: &CanonicalStateV2| {
        document.player.get("osty_corpse") == Some(&json!(true))
            && document.player.contains_key("next_creature_uid")
    };
    for (id, counter, steps) in [
        ("f29e8ebdad08c5ee", 4, &[13][..]),
        ("f57e223853b3668c", 5, &[12][..]),
        ("f066148b82434548", 2, &[26][..]),
    ] {
        assert_eq!(
            steps_holding(id, corpse_beside_a_tracked_counter),
            steps,
            "{id}"
        );
        let (entry, _) = fixture(id).unwrap();
        assert_eq!(entry.player["next_creature_uid"], json!(counter), "{id}");
    }
}

/// Whether the first shape could change play: it could. A reloaded state's
/// next spawn took the uid after native's.
///
/// `f25a5fda04e82415` is a Phrog Parasite fight with an Osty and a Byrdpip:
/// the Phrog is creature uid 0, the pets hold 1 and 2, and the four
/// Wrigglers the Phrog's death spawns take 3..=6 (step 13 of the stored
/// line, certified against the capture, whose targets address monsters by
/// creation order). Here the Osty has died before the Phrog does. The
/// document says `next_creature_uid: 3`; before #3674 its reload said 4 and
/// spawned 4..=7.
#[test]
fn a_spawn_after_reloading_beside_an_osty_corpse_takes_the_native_uid() {
    let id = "f25a5fda04e82415";
    let (entry, line) = fixture(id).unwrap();
    let (catalog, mut state) = hydrate(&entry).unwrap();
    let actions = line["actions"].as_array().unwrap();
    let kill = 13;
    for wire in &actions[..kill] {
        state = engine::apply_action(&state, &catalog, &action(wire))
            .unwrap()
            .state;
    }
    let mut document = HotBoundary::try_to_canonical(&state, &catalog).unwrap();
    assert_eq!(document.player["next_creature_uid"], json!(3));
    assert_eq!(document.player["ally"], json!(["OSTY", 2, 2]));
    document.player.remove("ally");
    document
        .player
        .insert("osty_corpse".to_owned(), json!(true));

    let (catalog, reloaded) = hydrate(&document).unwrap();
    assert_eq!(
        HotBoundary::try_to_canonical(&reloaded, &catalog).unwrap(),
        document,
    );
    let after = engine::apply_action(&reloaded, &catalog, &action(&actions[kill]))
        .unwrap()
        .state;
    let after = HotBoundary::try_to_canonical(&after, &catalog).unwrap();
    let roster: Vec<(&str, i64)> = after
        .monsters
        .iter()
        .map(|monster| {
            (
                monster["kind"].as_str().unwrap(),
                monster.get("uid").and_then(Value::as_i64).unwrap_or(0),
            )
        })
        .collect();
    assert_eq!(
        roster,
        [
            ("PHROG_PARASITE", 0),
            ("WRIGGLER", 3),
            ("WRIGGLER", 4),
            ("WRIGGLER", 5),
            ("WRIGGLER", 6),
        ],
    );
    assert_eq!(after.player["next_creature_uid"], json!(7));
}

/// The second shape, on the fight the census projected it from:
/// `f6706a4e38f307ca`, The Obscura and its Parafright. The Obscura buffs both
/// at step 7 (Strength 3, applied by the Obscura, uid 0). The Parafright is
/// killed at steps 11, 16, 36 and 39 and revives each time; the Obscura
/// buffs again at step 18, onto a Parafright that has died twice.
///
/// At every step from 7 on the Parafright's row is the Obscura's one
/// instance, mirroring its Strength: while it stands, while it is down
/// (`revive_stage` 1), and after it revives. Before #3674 the row vanished
/// at step 11 and never came back, and each of those documents reloaded with
/// an `unknown` row instead. Every one of them now reloads as itself
/// (`replay`).
///
/// That `unknown` row was not inert. This fight can reach `Misery` and
/// Sleight of Flesh, where admission refuses an unobserved Strength applier
/// ("Misery Strength clone unknown applier"), so a reloaded document was
/// refused for a reason its session state did not have. Here the standing,
/// downed and revived documents of steps 7 to 14 are admitted, and no later
/// one is refused for a Strength or Misery reason: from step 15 the only
/// reason is the Debris row #3644 tracks, which also parks step 32.
#[test]
fn a_revived_parafright_keeps_the_obscuras_strength_row() {
    let id = "f6706a4e38f307ca";
    let (entry, line) = fixture(id).unwrap();
    let mut downed = Vec::new();
    let mut rows = BTreeMap::new();
    let mut admitted = Vec::new();
    let mut parked = Vec::new();
    replay(id, &entry, &line, |index, document, reloaded| {
        let Some(parafright) = document
            .monsters
            .iter()
            .find(|monster| monster["kind"] == json!("PARAFRIGHT"))
        else {
            return;
        };
        let Some(strength) = parafright.get("strength") else {
            assert!(
                !parafright.contains_key("power_attachments"),
                "step {index}"
            );
            return;
        };
        assert_eq!(
            parafright.get("power_attachments"),
            Some(&json!([["strength", "monster", 0, strength, 0]])),
            "step {index}",
        );
        *rows.entry(strength.as_i64().unwrap()).or_insert(0) += 1;
        if parafright["hp"].as_i64().unwrap() <= 0 {
            assert_eq!(parafright["revive_stage"], json!(1), "step {index}");
            downed.push(index);
        }
        let Some((catalog, state)) = reloaded else {
            parked.push(index);
            return;
        };
        match engine::admit(document, state, catalog) {
            Ok(()) => admitted.push(index),
            Err(refusal) => {
                let missing = format!("{:?}", refusal.missing().collect::<Vec<_>>());
                assert!(
                    missing.contains("Debris generation provenance")
                        && !missing.contains("Strength")
                        && !missing.contains("Misery"),
                    "step {index}: {missing}",
                );
            }
        }
    });
    assert_eq!(downed, [11, 16, 17, 36, 39]);
    assert_eq!(rows, BTreeMap::from([(3, 11), (6, 23)]));
    assert_eq!(admitted, [7, 8, 9, 10, 11, 12, 13, 14]);
    assert_eq!(parked, [32]);
}

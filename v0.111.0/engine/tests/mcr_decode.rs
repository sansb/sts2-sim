//! The `.mcr` decoder (#3578 slice A), without Python.
//!
//! `tools/mcr_parity.py` is the parity check: it decodes every replay with the
//! Python decoder and with this one and requires the same document. It wrote
//! `fixtures/mcr_decode_pins_v1.json` only after they agreed, so each pin is
//! the digest of a document the Python decoder also produced. These tests
//! hold the decoder to those digests, and to its named refusals.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use sts_sim::mcr;

fn testdata() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../python/testdata")
}

fn pins() -> serde_json::Value {
    serde_json::from_str(include_str!("../fixtures/mcr_decode_pins_v1.json")).unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn every_pinned_fixture_decodes_to_its_pinned_document() {
    let pins = pins();
    assert_eq!(pins["game_version"], mcr::tables_game_version());
    let fixtures = pins["fixtures"].as_object().unwrap();
    assert!(fixtures.len() >= 11, "the v0.111.0 fixtures are pinned");
    for (name, digest) in fixtures {
        let bytes = std::fs::read(testdata().join(name)).unwrap();
        let document = mcr::decode(&bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            sha256_hex(document.to_string().as_bytes()),
            digest.as_str().unwrap(),
            "{name} decodes to a different document"
        );
        assert_eq!(document["version"], "v0.111.0", "{name}");
    }
}

#[test]
fn a_decoded_replay_has_the_fields_the_replay_reads() {
    // Any pinned fixture will do: take the first.
    let pins = pins();
    let name = pins["fixtures"].as_object().unwrap().keys().next().unwrap();
    let bytes = std::fs::read(testdata().join(name)).unwrap();
    let document = mcr::decode(&bytes).unwrap();
    let events = document["events"].as_array().unwrap();
    assert!(!events.is_empty());
    assert!(events.iter().all(|event| event["event_type"].is_string()));
    assert!(document["checksums"].is_array());
    let player = &document["run"]["players"][0];
    assert!(
        player["deck"]
            .as_array()
            .is_some_and(|deck| !deck.is_empty())
    );
    // 64-bit values stay exact: a seed is a JSON integer, not a float.
    assert!(player["player_rng"]["seed"].is_u64());
}

#[test]
fn every_other_build_is_refused_by_name_never_guessed_at() {
    let pinned: Vec<String> = pins()["fixtures"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    let mut refused = 0;
    for entry in std::fs::read_dir(testdata()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if path.extension().is_none_or(|ext| ext != "mcr") || pinned.contains(&name) {
            continue;
        }
        let error = mcr::decode(&std::fs::read(&path).unwrap()).expect_err(&name);
        assert_eq!(error.code, "unsupported_replay_build", "{name}: {error}");
        assert!(
            error.detail.contains("modelIdHash mismatch"),
            "{name}: {error}"
        );
        refused += 1;
    }
    assert!(
        refused >= 26,
        "the older builds' fixtures are still there: {refused}"
    );
}

#[test]
fn damaged_input_is_refused_and_never_panics() {
    let pins = pins();
    let name = pins["fixtures"].as_object().unwrap().keys().next().unwrap();
    let bytes = std::fs::read(testdata().join(name)).unwrap();
    // Empty, and every truncation near the start: the header itself is short.
    assert_eq!(mcr::decode(&[]).unwrap_err().code, "truncated_replay");
    for cut in 0..64.min(bytes.len()) {
        let error = mcr::decode(&bytes[..cut]).unwrap_err();
        assert!(
            [
                "truncated_replay",
                "malformed_replay",
                "unsupported_replay_build"
            ]
            .contains(&error.code),
            "cut {cut}: {error}"
        );
    }
    // Truncations across the body, and one trailing byte too many.
    for cut in (64..bytes.len()).step_by((bytes.len() / 97).max(1)) {
        assert!(mcr::decode(&bytes[..cut]).is_err(), "cut {cut} decoded");
    }
    let mut padded = bytes.clone();
    padded.push(0);
    assert_eq!(mcr::decode(&padded).unwrap_err().code, "malformed_replay");
    // Every single-bit flip in the first kilobyte either decodes or refuses.
    for index in 0..1024.min(bytes.len()) {
        let mut flipped = bytes.clone();
        flipped[index] ^= 0x10;
        let _ = mcr::decode(&flipped);
    }
}

#[test]
fn the_tables_are_a_byte_copy_of_the_python_tables() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ours = std::fs::read(crate_dir.join("data/mcr_tables.v0.111.0.json")).unwrap();
    let theirs = std::fs::read(crate_dir.join("../python/mcr_tables.json")).unwrap();
    assert_eq!(sha256_hex(&ours), sha256_hex(&theirs));
}

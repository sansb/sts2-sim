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
    // This fixture's stream ends inside its last byte, so a byte after it is
    // a ninth unread bit. (One that ends ON a byte boundary may carry one
    // zero byte: `a_byte_aligned_capture_may_carry_one_zero_byte`.)
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

/// #3756: a capture whose stream ends exactly on a byte boundary. The game's
/// writer stops there; some uploaded files carry one more zero byte. Both
/// decode to the same document, and nothing else after the stream does.
#[test]
fn a_byte_aligned_capture_may_carry_one_zero_byte() {
    let bytes = std::fs::read(testdata().join("WDRG3QZP2Y7G_n02_byte_aligned.mcr")).unwrap();
    let document = mcr::decode(&bytes).unwrap();
    assert_eq!(document["events"].as_array().unwrap().len(), 1);
    assert_eq!(document["checksums"].as_array().unwrap().len(), 2);
    // The stream ends on the boundary: dropping the last byte truncates it,
    // and that byte is not padding the decoder skipped.
    assert_eq!(
        mcr::decode(&bytes[..bytes.len() - 1]).unwrap_err().code,
        "truncated_replay"
    );

    let with = |tail: &[u8]| {
        let mut longer = bytes.clone();
        longer.extend_from_slice(tail);
        mcr::decode(&longer)
    };
    assert_eq!(with(&[0]).unwrap(), document);
    for tail in [&[1u8][..], &[0x80], &[0, 0], &[0, 1]] {
        let error = with(tail).unwrap_err();
        assert_eq!(error.code, "malformed_replay", "{tail:?}");
        assert!(
            error.detail.contains("unconsumed bits after decode"),
            "{tail:?}: {error}"
        );
    }
}

/// #2103: a snapshot that carries player map drawings decodes, and so does
/// everything after it. The strokes are the ones the site's map-ink fixture for
/// this run records from the floor save: an erased pen dot still in the list,
/// then eraser strokes of 126 and 133 points over it.
#[test]
fn a_capture_with_map_drawings_decodes_to_its_last_bit() {
    let bytes = std::fs::read(testdata().join("39D29WG6HMEK_d21_map_drawings.mcr")).unwrap();
    let document = mcr::decode(&bytes).unwrap();
    let run = &document["run"];
    assert_eq!(run["rng"]["seed"], "39D29WG6HMEK");
    // A drift of one bit in the snapshot would not leave 17 events and 25
    // checksums that end on the final byte.
    assert_eq!(document["events"].as_array().unwrap().len(), 17);
    assert_eq!(document["checksums"].as_array().unwrap().len(), 25);

    let drawings = run["map_drawings"].as_array().unwrap();
    assert_eq!(drawings.len(), 1);
    assert_eq!(drawings[0]["player_id"], run["players"][0]["net_id"]);
    let lines = drawings[0]["lines"].as_array().unwrap();
    assert_eq!(lines.len(), 33);
    let shape: Vec<(bool, usize)> = lines
        .iter()
        .map(|line| {
            (
                line["is_eraser"].as_bool().unwrap(),
                line["points"].as_array().unwrap().len(),
            )
        })
        .collect();
    assert_eq!(shape.iter().map(|(_, points)| points).sum::<usize>(), 346);
    assert_eq!(
        shape[27..31],
        [(true, 126), (true, 133), (true, 2), (true, 3)]
    );
    assert!(
        shape
            .iter()
            .enumerate()
            .all(|(index, (eraser, _))| *eraser == (27..31).contains(&index))
    );
    assert_eq!(
        lines[0]["points"][0],
        serde_json::json!([-0.595458984375, 0.42401123046875])
    );
    for point in lines
        .iter()
        .flat_map(|line| line["points"].as_array().unwrap())
    {
        let (x, y) = (point[0].as_f64().unwrap(), point[1].as_f64().unwrap());
        assert!((-3.0..3.0).contains(&x) && (-2.0..2.0).contains(&y));
        // On the grid: 13 bits over a span of 6, 16 bits over 4.
        assert_eq!(((x + 3.0) * 8192.0 / 6.0).fract(), 0.0);
        assert_eq!(((y + 2.0) * 65536.0 / 4.0).fract(), 0.0);
    }
}

#[test]
fn the_tables_are_a_byte_copy_of_the_python_tables() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ours = std::fs::read(crate_dir.join("data/mcr_tables.v0.111.0.json")).unwrap();
    let theirs = std::fs::read(crate_dir.join("../python/mcr_tables.json")).unwrap();
    assert_eq!(sha256_hex(&ours), sha256_hex(&theirs));
}

//! Checked-in corpus and typed-boundary contract for the Rust solo authority.

use serde_json::Value;

use sts_sim::canonical::CanonicalStateV2;
use sts_sim::solo_v1::{SOLO_V1_POLICY, SoloV1Admission, admit_exact_solve};

const CORPUS: &str = include_str!("../fixtures/exact_solve_corpus_v1.json");

#[test]
fn checked_in_historical_oracle_rows_are_canonical_solo_candidates() {
    let corpus: Value = serde_json::from_str(CORPUS).expect("corpus JSON parses");
    assert_eq!(
        corpus["schema"], "sts-sim-exact-solve-corpus-v1",
        "never reinterpret a corpus schema"
    );
    assert_eq!(corpus["policy"], SOLO_V1_POLICY);
    assert_eq!(
        corpus["authority"]["exact_search"], "rust-solo-v1",
        "the retired Python search must not remain exact-search authority"
    );

    let rows = corpus["rows"].as_array().expect("rows array");
    assert_eq!(
        rows.len(),
        2,
        "keep the first authority corpus small and legible"
    );
    assert!(
        rows.iter()
            .any(|row| row["source"]["kind"] == "mcr_capture")
    );
    assert!(rows.iter().any(|row| row["source"]["kind"] == "synthetic"));
    for row in rows {
        let entry: CanonicalStateV2 =
            serde_json::from_value(row["entry"].clone()).expect("canonical entry parses");
        assert_eq!(
            entry.differential_digest(),
            row["entry_digest"].as_str().expect("entry digest"),
            "{} entry digest",
            row["id"].as_str().unwrap_or("unnamed")
        );
        assert_eq!(admit_exact_solve(&entry), Ok(SoloV1Admission::Admitted));
        assert!(matches!(
            row["python"]["status"].as_str(),
            Some("exact" | "deadline")
        ));
        for action in row["python"]["action_line"]
            .as_array()
            .expect("action line")
        {
            match action["kind"].as_str().expect("action kind") {
                "end" => assert_eq!(action, &serde_json::json!({"kind": "end"})),
                "play" => {
                    assert!(
                        action.get("card").is_none(),
                        "payload identity is never a replay key"
                    );
                    assert!(
                        action["uid"].as_u64().is_some(),
                        "Rust play protocol requires UID"
                    );
                    assert!(action.get("target").is_none() || action["target"].as_u64().is_some());
                    let diagnostic = action["diagnostic_card"]
                        .as_object()
                        .expect("diagnostic card");
                    assert!(diagnostic["id"].as_str().is_some());
                    assert!(diagnostic["upgrade"].as_i64().is_some());
                }
                other => panic!("unknown corpus action kind {other:?}"),
            }
        }
    }
}

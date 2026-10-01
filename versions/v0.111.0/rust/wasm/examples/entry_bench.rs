//! #3471: time `entry` natively, per request, for comparison with
//! `js/entry_bench.mjs` running the same requests through the wasm module.
//! stdin: JSONL `{"request": <entry request>}`; stdout: JSONL `{"ms": .., "bytes": ..}`.
use std::io::{BufRead, Write};

fn main() {
    let reps: usize = std::env::args()
        .nth(1)
        .and_then(|n| n.parse().ok())
        .unwrap_or(3);
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    for line in std::io::stdin().lock().lines() {
        let line: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        let request = line["request"].to_string();
        let mut best = f64::MAX;
        let mut bytes = 0;
        for _ in 0..reps {
            let start = std::time::Instant::now();
            let answer = sts_sim_wasm::entry(&request);
            best = best.min(start.elapsed().as_secs_f64() * 1000.0);
            bytes = answer.len();
        }
        writeln!(out, "{}", serde_json::json!({ "ms": best, "bytes": bytes })).unwrap();
    }
}

//! Semantic retrieval regression suite (runs in CI, no Ollama needed).
//!
//! Replays embeddings recorded from the real model used in physical acceptance
//! testing (qwen3-embedding:0.6b) through the production pipeline: retrieval
//! profiles, model-aware query formatting, hubness/scale calibration, lexical
//! fusion and the Best Match decision. Any change that degrades intent
//! retrieval — or labels a wrong Spark as a confident Best Match — fails here.
//!
//! If the profile recipe, query formatting, or calibration goals change, the
//! recorded texts no longer match: re-record on a machine with the model via
//!   cargo test --test semantic_live record -- --ignored --nocapture

mod support;

use std::collections::HashMap;

use support::*;

const RECORDED_MODEL: &str = "qwen3-embedding:0.6b";

fn recorded() -> HashMap<String, Vec<f32>> {
    let path = fixture_path(RECORDED_MODEL);
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing {}; record it with the live harness",
            path.display()
        )
    });
    let rec: RecordedVectors = serde_json::from_str(&raw).expect("fixture json");
    assert_eq!(rec.model, RECORDED_MODEL);
    rec.vectors
        .into_iter()
        .map(|(k, v)| (k, decode_vector(&v)))
        .collect()
}

#[test]
fn acceptance_goals_find_the_right_spark() {
    let vectors = recorded();
    let dir = tempfile::tempdir().unwrap();
    let lib = acceptance_library(dir.path());
    let texts = document_texts(&lib, RECORDED_MODEL);

    let missing: Vec<String> = every_text(&texts, RECORDED_MODEL)
        .into_iter()
        .filter(|t| !vectors.contains_key(&text_key(t)))
        .map(|t| t.chars().take(80).collect())
        .collect();
    assert!(
        missing.is_empty(),
        "The retrieval texts changed ({} not recorded, e.g. {:?}). Re-record the fixture with the live harness.",
        missing.len(),
        missing.first()
    );

    let embed = |t: &str| vectors[&text_key(t)].clone();
    let index = build_index(RECORDED_MODEL, &texts, &embed);
    let report = evaluate(&lib, &index, RECORDED_MODEL, &embed);
    report.print();

    let acc = &report.acceptance;
    // Round 1 baseline: 6 correct / 3 acceptable / 3 incorrect (1 confidently wrong).
    assert_eq!(
        Report::count(acc, Grade::ConfidentlyWrong),
        0,
        "a wrong Spark was shown as the Best Match"
    );
    assert_eq!(
        Report::count(acc, Grade::Missed),
        0,
        "an acceptance goal's Spark wasn't shown at all"
    );
    assert!(
        Report::count(acc, Grade::Correct) >= 9,
        "only {} of 12 acceptance goals got the expected Best Match",
        Report::count(acc, Grade::Correct)
    );
    let dev = &report.dev;
    assert_eq!(
        Report::count(dev, Grade::ConfidentlyWrong),
        0,
        "a wrong Spark was shown as the Best Match"
    );
    assert!(
        Report::count(dev, Grade::Correct) >= 26,
        "only {} of {} paraphrased goals got the expected Best Match",
        Report::count(dev, Grade::Correct),
        dev.len()
    );
    assert!(
        report.false_confident.is_empty(),
        "unrelated goals shown as a confident match"
    );
}

#[test]
fn f16_round_trip_is_precise_enough() {
    for x in [0.0f32, 1.0, -1.0, 0.123_456, -0.031_25, 1e-5, 0.999] {
        let y = f16_to_f32(f32_to_f16(x));
        assert!((x - y).abs() <= x.abs() * 1e-3 + 1e-6, "{x} -> {y}");
    }
}

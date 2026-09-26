//! Semantic retrieval regression suite (runs in CI, no Ollama needed).
//!
//! Replays embeddings recorded from Sparkwell's canonical model
//! (qwen3-embedding:8b-q8_0) through the production pipeline: labelled
//! retrieval profiles, the Qwen3 query instruction, hubness/scale
//! calibration, lexical fusion and the Best Match decision. Any change that
//! degrades intent retrieval — or labels a wrong Spark as a confident Best
//! Match — fails here.
//!
//! If the profile recipe, query formatting, or calibration goals change, the
//! recorded texts no longer match: re-record on a machine with the model via
//!   cargo test --test semantic_live record -- --ignored --nocapture

mod support;

use std::collections::HashMap;

use sparkwell_lib::ai::models::CANONICAL_EMBED_MODEL;
use support::*;

const RECORDED_MODEL: &str = CANONICAL_EMBED_MODEL;

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
fn goals_find_the_right_spark() {
    let vectors = recorded();
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let lib = acceptance_library(a.path());
    let texts = document_texts(&lib, RECORDED_MODEL);
    let (user_lib, user_ids) = user_library(b.path());
    let user_texts = document_texts(&user_lib, RECORDED_MODEL);

    let missing: Vec<String> = union_texts(&[&texts, &user_texts], RECORDED_MODEL)
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
    let mut report = evaluate(&lib, &index, RECORDED_MODEL, &embed);
    let user_index = build_index(RECORDED_MODEL, &user_texts, &embed);
    evaluate_user(
        &mut report,
        &user_lib,
        &user_ids,
        &user_index,
        RECORDED_MODEL,
        &embed,
    );
    report.print();

    // Zero confidently wrong Best Matches anywhere comes first.
    for (label, list) in [
        ("acceptance", &report.acceptance),
        ("dev", &report.dev),
        ("held-out", &report.heldout),
        ("test", &report.test),
        ("user-added", &report.premium),
        ("acceptance+user", &report.acceptance_with_user),
    ] {
        assert_eq!(
            Report::count(list, Grade::ConfidentlyWrong),
            0,
            "a wrong Spark was shown as the Best Match ({label})"
        );
    }
    assert!(
        report.false_confident.is_empty(),
        "unrelated goals shown as a confident match: {:?}",
        report.false_confident
    );

    // Then how often the right Spark is the confident answer, and that it is
    // always offered. Floors are the results recorded for this calibration.
    let floors = [
        ("acceptance", &report.acceptance, 8, 0),
        ("dev", &report.dev, 26, 0),
        ("held-out", &report.heldout, 10, 1),
        ("test", &report.test, 10, 0),
        ("user-added", &report.premium, 10, 0),
        ("acceptance+user", &report.acceptance_with_user, 7, 0),
    ];
    for (label, list, correct, missed) in floors {
        assert!(
            Report::count(list, Grade::Correct) >= correct,
            "{label}: only {} of {} goals got the expected Best Match",
            Report::count(list, Grade::Correct),
            list.len()
        );
        assert!(
            Report::count(list, Grade::Missed) <= missed,
            "{label}: {} goals didn't offer the expected Spark at all",
            Report::count(list, Grade::Missed)
        );
    }
}

#[test]
fn f16_round_trip_is_precise_enough() {
    for x in [0.0f32, 1.0, -1.0, 0.123_456, -0.031_25, 1e-5, 0.999] {
        let y = f16_to_f32(f32_to_f16(x));
        assert!((x - y).abs() <= x.abs() * 1e-3 + 1e-6, "{x} -> {y}");
    }
}

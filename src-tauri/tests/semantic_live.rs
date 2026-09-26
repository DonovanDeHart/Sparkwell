//! Live retrieval-quality harness against a real local Ollama.
//!
//! Ignored by default (CI has no Ollama). With Ollama running:
//!
//!   cargo test --test semantic_live -- --ignored --nocapture
//!
//! `semantic_retrieval_quality` embeds the acceptance library with the
//! installed embedding model (or SPARKWELL_EMBED_MODEL) through the same code
//! the app uses and grades every goal (see tests/support).
//!
//! `record_semantic_fixture` writes the vectors used by the offline regression
//! suite (tests/semantic_regression.rs). Re-record whenever the retrieval
//! profile recipe, query formatting, or calibration goals change:
//!
//!   cargo test --test semantic_live record -- --ignored --nocapture

mod support;

use std::collections::HashMap;
use std::time::Duration;

use sparkwell_lib::ai::models::pick_embedding_model;
use sparkwell_lib::ai::ollama::OllamaClient;
use sparkwell_lib::search::profile::PROFILE_VERSION;
use support::*;

async fn live_vectors(model: &str, texts: &[String]) -> HashMap<String, Vec<f32>> {
    let client = OllamaClient::default();
    let mut out = HashMap::new();
    for chunk in texts.chunks(16) {
        let v = client
            .embed(model, chunk, Duration::from_secs(300))
            .await
            .expect("embedding request");
        for (t, v) in chunk.iter().zip(v) {
            out.insert(t.clone(), v);
        }
    }
    out
}

async fn installed_model() -> String {
    let models = OllamaClient::default()
        .list_models()
        .await
        .expect("Ollama must be running on 127.0.0.1:11434");
    let override_name = std::env::var("SPARKWELL_EMBED_MODEL").ok();
    pick_embedding_model(&models, override_name.as_deref())
        .expect("an embedding model must be installed")
}

#[tokio::test]
#[ignore]
async fn semantic_retrieval_quality() {
    let model = installed_model().await;
    println!("embedding model: {model}");
    let dir = tempfile::tempdir().unwrap();
    let lib = acceptance_library(dir.path());
    let texts = document_texts(&lib, &model);
    let vectors = live_vectors(&model, &every_text(&texts, &model)).await;
    let embed = |t: &str| vectors[t].clone();
    let index = build_index(&model, &texts, &embed);
    let report = evaluate(&lib, &index, &model, &embed);
    report.print();

    assert_eq!(
        Report::count(&report.acceptance, Grade::ConfidentlyWrong),
        0,
        "confidently wrong acceptance answers"
    );
    assert_eq!(
        Report::count(&report.dev, Grade::ConfidentlyWrong),
        0,
        "confidently wrong dev answers"
    );
    assert!(
        report.false_confident.is_empty(),
        "unrelated goals shown as confident"
    );
}

#[tokio::test]
#[ignore]
async fn record_semantic_fixture() {
    let model = installed_model().await;
    let dir = tempfile::tempdir().unwrap();
    let lib = acceptance_library(dir.path());
    let texts = document_texts(&lib, &model);
    let all = every_text(&texts, &model);
    let vectors = live_vectors(&model, &all).await;
    let recorded = RecordedVectors {
        model: model.clone(),
        profile_version: PROFILE_VERSION.to_string(),
        note: "Recorded by `cargo test --test semantic_live record -- --ignored`. Keys are content hashes of the exact embedded text.".into(),
        vectors: all
            .iter()
            .map(|t| (text_key(t), encode_vector(&vectors[t])))
            .collect(),
    };
    let path = fixture_path(&model);
    std::fs::write(&path, serde_json::to_string(&recorded).unwrap()).unwrap();
    println!(
        "wrote {} vectors to {}",
        recorded.vectors.len(),
        path.display()
    );
}

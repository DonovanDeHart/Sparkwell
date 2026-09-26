//! Live retrieval-quality harness against a real local Ollama.
//!
//! Ignored by default (CI has no Ollama). With Ollama running and the
//! canonical model installed (`ollama pull qwen3-embedding:8b-q8_0`):
//!
//!   cargo test --test semantic_live semantic_retrieval_quality -- --ignored --nocapture
//!
//! `semantic_retrieval_quality` embeds the acceptance library (and the
//! user-added Sparks) through the same code the app uses, grades every goal
//! (see tests/support) and reports indexing and query timings.
//!
//! `record_semantic_fixture` writes the vectors used by the offline regression
//! suite (tests/semantic_regression.rs). Re-record whenever the retrieval
//! profile recipe, query formatting, or calibration goals change:
//!
//!   cargo test --test semantic_live record -- --ignored --nocapture

mod support;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use sparkwell_lib::ai::models::{self, pick_embedding_model};
use sparkwell_lib::ai::ollama::OllamaClient;
use sparkwell_lib::search::profile::PROFILE_VERSION;
use support::*;

/// Texts per request, as the app's indexer sends them.
const BATCH: usize = 16;

async fn live_vectors(model: &str, texts: &[String]) -> HashMap<String, Vec<f32>> {
    let client = OllamaClient::default();
    let mut out = HashMap::new();
    for chunk in texts.chunks(BATCH) {
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
    pick_embedding_model(&models, override_name.as_deref()).unwrap_or_else(|| {
        panic!(
            "install the canonical model first: ollama pull {}",
            models::CANONICAL_EMBED_MODEL
        )
    })
}

struct Corpus {
    _dirs: (tempfile::TempDir, tempfile::TempDir),
    lib: sparkwell_lib::storage::Library,
    texts: Texts,
    user_lib: sparkwell_lib::storage::Library,
    user_ids: Vec<i64>,
    user_texts: Texts,
}

fn corpus(model: &str) -> Corpus {
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let lib = acceptance_library(a.path());
    let texts = document_texts(&lib, model);
    let (user_lib, user_ids) = user_library(b.path());
    let user_texts = document_texts(&user_lib, model);
    Corpus {
        _dirs: (a, b),
        lib,
        texts,
        user_lib,
        user_ids,
        user_texts,
    }
}

#[tokio::test]
#[ignore]
async fn semantic_retrieval_quality() {
    let model = installed_model().await;
    println!("embedding model: {model}");
    let c = corpus(&model);
    let client = OllamaClient::default();

    // Indexing: the acceptance library's views, in the app's batch size.
    let doc_texts: Vec<String> = c.texts.docs.iter().flat_map(|(_, v)| v.clone()).collect();
    let started = Instant::now();
    let _ = live_vectors(&model, &doc_texts).await;
    println!(
        "indexing: {} Sparks, {} views in {:.2}s",
        c.texts.docs.len(),
        doc_texts.len(),
        started.elapsed().as_secs_f64()
    );

    let all = union_texts(&[&c.texts, &c.user_texts], &model);
    let vectors = live_vectors(&model, &all).await;

    // Warm query latency, one goal at a time as the app sends them.
    let mut times = Vec::new();
    for (q, _, _) in ACCEPTANCE {
        let t = Instant::now();
        client
            .embed(
                &model,
                &[models::query_text(&model, q)],
                Duration::from_secs(60),
            )
            .await
            .expect("query");
        times.push(t.elapsed().as_millis());
    }
    times.sort_unstable();
    println!(
        "warm query latency: median {} ms, max {} ms",
        times[times.len() / 2],
        times[times.len() - 1]
    );

    let embed = |t: &str| vectors[t].clone();
    let index = build_index(&model, &c.texts, &embed);
    let mut report = evaluate(&c.lib, &index, &model, &embed);
    let user_index = build_index(&model, &c.user_texts, &embed);
    evaluate_user(
        &mut report,
        &c.user_lib,
        &c.user_ids,
        &user_index,
        &model,
        &embed,
    );
    report.print();

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
            "confidently wrong {label} answers"
        );
    }
    assert!(
        report.false_confident.is_empty(),
        "unrelated goals shown as confident"
    );
}

#[tokio::test]
#[ignore]
async fn record_semantic_fixture() {
    let model = installed_model().await;
    let c = corpus(&model);
    let all = union_texts(&[&c.texts, &c.user_texts], &model);
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

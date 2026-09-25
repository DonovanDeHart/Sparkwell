//! Live retrieval-quality harness against a real local Ollama.
//!
//! Ignored by default (CI has no Ollama). Run with:
//!   cargo test --test semantic_live -- --ignored --nocapture
//!
//! Seeds the starter library, embeds it with the installed embedding model, and
//! checks intent phrasings that share few or no words with the Spark they
//! should find. It also prints the similarity statistics used to calibrate
//! `search::SemanticScores`.
//!
//! Acceptance bar (Best Match is a ranking, not a certainty):
//! - never a confidently wrong answer;
//! - the expected Spark is always visible (Best Match, or among the closest
//!   Sparks when two candidates are too close to call);
//! - at least 6/9 are confident Best Matches;
//! - clearly unrelated goals are never presented as a confident match.

use std::time::Duration;

use sparkwell_lib::ai::embed_text;
use sparkwell_lib::ai::models::{self, pick_embedding_model};
use sparkwell_lib::ai::ollama::OllamaClient;
use sparkwell_lib::search::vectors::{normalized, VectorIndex};
use sparkwell_lib::search::{self, Confidence, SearchOutcome, SemanticScores};
use sparkwell_lib::sparks::{self, seed::seed_starter_sparks};
use sparkwell_lib::storage::{Library, OpenMode};

const RELATED: &[(&str, &str)] = &[
    (
        "my app keeps crashing in production and I can't figure out why",
        "Root Cause Detective",
    ),
    (
        "grow my channel with videos people watch to the end",
        "YouTube Script Architect",
    ),
    (
        "I need AI to help me build an MCP server",
        "MCP Server Architect",
    ),
    (
        "give Claude tools to talk to my database",
        "MCP Server Architect",
    ),
    (
        "find out what the evidence really says about intermittent fasting",
        "Deep Research Framework",
    ),
    (
        "my instructions to the model keep getting ignored",
        "Prompt Engineering Master",
    ),
    (
        "turn my rough startup idea into a plan",
        "Project Launch Planner",
    ),
    (
        "have an autonomous coder refactor my repository safely",
        "Codex Architecture Expert",
    ),
    (
        "orchestrate several assistants that hand work to each other",
        "AI Agent System Designer",
    ),
];

const UNRELATED: &[&str] = &[
    "bake sourdough bread at home",
    "plan the seating chart for my wedding",
];

fn leader(o: &SearchOutcome) -> Option<&str> {
    o.best
        .as_ref()
        .or_else(|| o.alternatives.first())
        .map(|s| s.title.as_str())
}

#[tokio::test]
#[ignore]
async fn semantic_retrieval_quality() {
    let client = OllamaClient::default();
    let installed = client
        .list_models()
        .await
        .expect("Ollama must be running on 127.0.0.1:11434");
    let model =
        pick_embedding_model(&installed, None).expect("an embedding model must be installed");
    println!("embedding model: {model}");

    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::open(dir.path(), OpenMode::CreateIfMissing).unwrap();
    seed_starter_sparks(&mut lib).unwrap();
    let docs = sparks::search_docs(&lib, &sparks::all_ids(&lib).unwrap()).unwrap();
    let inputs: Vec<String> = docs
        .iter()
        .map(|d| {
            format!(
                "{}{}",
                models::document_prefix(&model),
                embed_text(&d.title, &d.summary, &d.tags, &d.body)
            )
        })
        .collect();
    let vectors = client
        .embed(&model, &inputs, Duration::from_secs(300))
        .await
        .unwrap();
    let mut index = VectorIndex::empty(Some(model.clone()));
    for (d, v) in docs.iter().zip(vectors) {
        index.insert(d.id, normalized(v));
    }

    let run = |q: Vec<f32>, query: &str| {
        let sem = SemanticScores::from_index(&index, &q).expect("enough vectors");
        search::search(&lib, query, Some(&sem), false, 0).unwrap()
    };
    let embed_query = |query: &str| {
        let client = client.clone();
        let model = model.clone();
        let text = format!("{}{}", models::query_prefix(&model), query);
        async move {
            client
                .embed(&model, &[text], Duration::from_secs(60))
                .await
                .unwrap()
                .remove(0)
        }
    };

    let (mut visible, mut strong_correct, mut strong_wrong) = (0, 0, Vec::new());
    for (query, expected) in RELATED {
        let q = embed_query(query).await;
        let mut sims = index.similarities(&q);
        sims.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let outcome = run(q, query);
        let standard = search::search(&lib, query, None, false, 0).unwrap();
        println!(
            "\n{:?}: {query}\n   expected {expected}\n   semantic {:?} ({:?})   standard {:?} ({:?})",
            outcome.confidence,
            leader(&outcome),
            outcome.mode,
            leader(&standard),
            standard.confidence
        );
        for (id, cos) in sims.iter().take(3) {
            let title = &docs.iter().find(|d| d.id == *id).unwrap().title;
            println!("      cos {cos:.3}  {title}");
        }
        let shown = outcome
            .best
            .iter()
            .chain(outcome.alternatives.iter())
            .any(|s| s.title == *expected);
        if shown {
            visible += 1;
        }
        match (&outcome.best, outcome.confidence) {
            (Some(best), Confidence::Strong) if best.title == *expected => strong_correct += 1,
            (Some(best), Confidence::Strong) => strong_wrong.push((*query, best.title.clone())),
            _ => {}
        }
    }

    let mut false_confident = Vec::new();
    for query in UNRELATED {
        let q = embed_query(query).await;
        let outcome = run(q, query);
        println!(
            "\nunrelated {:?}: {query} -> {:?}",
            outcome.confidence,
            leader(&outcome)
        );
        if outcome.confidence == Confidence::Strong {
            false_confident.push(*query);
        }
    }

    println!(
        "\nvisible {visible}/{} · strong-correct {strong_correct} · strong-wrong {} · false-confident {}",
        RELATED.len(),
        strong_wrong.len(),
        false_confident.len()
    );
    assert!(
        strong_wrong.is_empty(),
        "confidently wrong: {strong_wrong:?}"
    );
    assert!(
        false_confident.is_empty(),
        "unrelated goals shown as confident: {false_confident:?}"
    );
    assert_eq!(
        visible,
        RELATED.len(),
        "expected Spark not shown for every goal"
    );
    assert!(
        strong_correct >= 6,
        "only {strong_correct}/9 confident Best Matches"
    );
}

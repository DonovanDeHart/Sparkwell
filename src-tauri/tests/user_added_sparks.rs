//! The capture model: prompts made elsewhere (ChatGPT, Claude, Gemini…) are
//! pasted into Sparkwell and must come back from Copy Spark exactly as saved.
//! Retrieval uses separate, derived profiles; nothing in Sparkwell rewrites
//! the original.
//!
//! How well these Sparks are found by intent is measured with real vectors
//! in semantic_regression.rs / semantic_live.rs (the PREMIUM goals).

mod support;

use sparkwell_lib::ai::metadata;
use sparkwell_lib::ai::models::{document_text, CANONICAL_EMBED_MODEL};
use sparkwell_lib::ai::{spark_views, views_hash};
use sparkwell_lib::sparks::{self, SparkInput};
use support::*;

#[test]
fn pasted_prompts_are_stored_and_copied_byte_for_byte() {
    let dir = tempfile::tempdir().unwrap();
    let (mut lib, ids) = user_library(dir.path());
    let originals = user_sparks();
    assert!(
        originals.iter().any(|s| s.body.contains("\r\n")),
        "fixture covers CRLF"
    );
    assert!(
        originals.iter().any(|s| s.body.starts_with('\n')),
        "fixture covers leading blank lines"
    );
    assert!(
        originals.iter().any(|s| s.body.ends_with(' ')),
        "fixture covers trailing spaces"
    );
    for (id, original) in ids.iter().zip(&originals) {
        let stored = sparks::get_detail(&lib, *id).unwrap();
        assert_eq!(stored.body.as_bytes(), original.body.as_bytes());
        let (_, copied) = sparks::record_copy(&mut lib, *id).unwrap();
        assert_eq!(copied.as_bytes(), original.body.as_bytes());
    }
}

#[test]
fn retrieval_profiles_are_separate_from_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let (lib, ids) = user_library(dir.path());
    for doc in sparks::search_docs(&lib, &ids).unwrap() {
        let views = spark_views(&doc);
        // Profile first (labelled, compact), then a few passages.
        assert!(views[0].starts_with("Title: "), "{}", views[0]);
        assert!(views.len() <= 5);
        assert!(views.iter().all(|v| v != &doc.body));
        assert!(views[0].chars().count() < doc.body.chars().count());
        // What is embedded comes from the views, in the model's document format
        // (no instruction on documents for Qwen3 Embedding).
        for v in &views {
            assert_eq!(&document_text(CANONICAL_EMBED_MODEL, v), v);
        }
        // Blank titles are derived for display and retrieval; the body isn't touched.
        assert!(!doc.title.is_empty());
    }
}

#[test]
fn editing_details_and_drafting_metadata_never_touch_the_body() {
    let dir = tempfile::tempdir().unwrap();
    let (mut lib, ids) = user_library(dir.path());
    let original = user_sparks().remove(1);
    let id = ids[1];
    let before_hash = {
        let doc = &sparks::search_docs(&lib, &[id]).unwrap()[0];
        views_hash(CANONICAL_EMBED_MODEL, &spark_views(doc))
    };

    // A drafted suggestion only ever has title, summary and tags.
    let suggestion = metadata::parse(
        r#"{"title":"TypeScript Strict Migration","summary":"Moves a codebase to strict mode step by step.","tags":["typescript","migration"],"body":"REWRITTEN"}"#,
    )
    .unwrap();
    let detail = sparks::get_detail(&lib, id).unwrap();
    sparks::update(
        &mut lib,
        id,
        SparkInput {
            title: suggestion.title,
            summary: suggestion.summary,
            body: detail.body.clone(),
            tags: suggestion.tags,
            favorite: detail.summary.favorite,
            source_note: None,
            allow_duplicate: false,
        },
    )
    .unwrap();

    let after = sparks::get_detail(&lib, id).unwrap();
    assert_eq!(after.summary.title, "TypeScript Strict Migration");
    assert_eq!(after.body.as_bytes(), original.body.as_bytes());
    // New details mean a new retrieval profile (re-indexed), same original.
    let doc = &sparks::search_docs(&lib, &[id]).unwrap()[0];
    assert_ne!(
        views_hash(CANONICAL_EMBED_MODEL, &spark_views(doc)),
        before_hash
    );
}

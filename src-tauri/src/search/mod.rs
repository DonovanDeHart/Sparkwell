//! Retrieval: lexical (always available) + semantic (when local embeddings are
//! ready), fused into one decisive Best Match.
//!
//! Scores are internal ranking signals only. The UI shows qualitative states
//! (Best Match / no strong match), never a fabricated percentage.

pub mod text;
pub mod vectors;

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::sparks::{self, SearchDoc, SparkSummary};
use crate::storage::Library;
use text::{fts_query, is_stopword, query_terms, stem, tokenize, tokens_match};
use vectors::VectorIndex;

/// Minimum fused score for a confident Best Match.
pub const STRONG_MATCH: f64 = 0.42;
/// Minimum score for a Spark to be offered as a "closest" candidate.
pub const CANDIDATE_MIN: f64 = 0.10;
const FTS_CANDIDATES: usize = 50;
const SEMANTIC_CANDIDATES: usize = 20;
const MAX_ALTERNATIVES: usize = 3;
/// Only the start of very long bodies is scanned for lexical coverage; the
/// full body is still indexed by FTS for recall.
const BODY_SCAN_CHARS: usize = 20_000;
/// Cosine distance above the library baseline that counts as a full semantic match.
const SEMANTIC_SPAN: f32 = 0.22;
/// Baseline used when the library is too small for a meaningful median.
const SMALL_LIBRARY_BASELINE: f32 = 0.35;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SearchMode {
    /// Semantic + lexical fusion.
    Semantic,
    /// Lexical/title/tag/full-text only.
    Standard,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    Strong,
    Weak,
    None,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOutcome {
    pub query: String,
    pub mode: SearchMode,
    pub confidence: Confidence,
    /// The single recommendation. Present only for a strong match.
    pub best: Option<SparkSummary>,
    /// Closest candidates, offered when there is no strong match.
    pub alternatives: Vec<SparkSummary>,
    /// Some Sparks have not been embedded yet (semantic mode only).
    pub partially_indexed: bool,
}

#[derive(Debug, Clone)]
pub struct Scored {
    pub id: i64,
    pub score: f64,
    pub base: f64,
    favorite: bool,
    usage: i64,
}

/// Semantic evidence for a query: cosine per Spark plus the library baseline.
pub struct SemanticScores {
    cosines: HashMap<i64, f32>,
    baseline: f32,
}

impl SemanticScores {
    pub fn from_index(index: &VectorIndex, query_vector: &[f32]) -> Option<SemanticScores> {
        let sims = index.similarities(query_vector);
        if sims.is_empty() {
            return None;
        }
        let mut values: Vec<f32> = sims.iter().map(|(_, c)| *c).collect();
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        // The median Spark is, by definition, not what the user asked for; it
        // anchors "unrelated" for whichever embedding model is installed.
        let baseline = if values.len() >= 5 {
            values[values.len() / 2]
        } else {
            SMALL_LIBRARY_BASELINE
        };
        Some(SemanticScores { cosines: sims.into_iter().collect(), baseline })
    }

    fn normalized(&self, id: i64) -> Option<f64> {
        self.cosines
            .get(&id)
            .map(|c| (((c - self.baseline) / SEMANTIC_SPAN).clamp(0.0, 1.0)) as f64)
    }

    fn top_ids(&self, n: usize) -> Vec<i64> {
        let mut all: Vec<(i64, f32)> = self.cosines.iter().map(|(k, v)| (*k, *v)).collect();
        all.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
        all.into_iter().take(n).map(|(id, _)| id).collect()
    }
}

fn stemmed_tokens(text: &str) -> Vec<String> {
    tokenize(text).iter().map(|t| stem(t)).collect()
}

fn any_match(term: &str, tokens: &[String]) -> bool {
    tokens.iter().any(|t| tokens_match(term, t))
}

/// Field-weighted coverage of the query terms, blended with how much of the
/// title the query covers. Range 0..=1.
pub fn lexical_score(terms: &[String], doc: &SearchDoc) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let title = stemmed_tokens(&doc.title);
    let tags: Vec<String> = doc.tags.iter().flat_map(|t| stemmed_tokens(t)).collect();
    let summary = stemmed_tokens(&doc.summary);
    let body_head: String = doc.body.chars().take(BODY_SCAN_CHARS).collect();
    let body = stemmed_tokens(&body_head);

    let covered: f64 = terms
        .iter()
        .map(|term| {
            if any_match(term, &title) {
                1.0
            } else if any_match(term, &tags) {
                0.85
            } else if any_match(term, &summary) {
                0.55
            } else if any_match(term, &body) {
                0.25
            } else {
                0.0
            }
        })
        .sum();
    let coverage = covered / terms.len() as f64;

    let title_content: Vec<String> = tokenize(&doc.title)
        .into_iter()
        .filter(|t| !is_stopword(t))
        .map(|t| stem(&t))
        .collect();
    let title_recall = if title_content.is_empty() {
        0.0
    } else {
        title_content.iter().filter(|t| any_match(t, terms)).count() as f64 / title_content.len() as f64
    };

    0.8 * coverage + 0.2 * title_recall
}

/// Light, bounded preference for Sparks the user relies on. Never large enough
/// to override intent (max 0.045 on a 0..1 scale).
fn history_bonus(doc: &SearchDoc, now_ms: i64) -> f64 {
    let favorite = if doc.favorite { 0.015 } else { 0.0 };
    let usage = ((1.0 + doc.usage_count.max(0) as f64).ln() / 51f64.ln()).min(1.0) * 0.02;
    let recency = doc
        .last_copied_at
        .map(|t| {
            let days = (now_ms - t).max(0) as f64 / 86_400_000.0;
            (1.0 - days / 30.0).max(0.0) * 0.01
        })
        .unwrap_or(0.0);
    favorite + usage + recency
}

/// Scores candidate documents. Pure function: deterministic for equal inputs.
pub fn rank(query: &str, docs: &[SearchDoc], semantic: Option<&SemanticScores>, now_ms: i64) -> Vec<Scored> {
    let terms = query_terms(query);
    let query_tokens = tokenize(query);
    let mut scored: Vec<Scored> = docs
        .iter()
        .map(|doc| {
            let lexical = lexical_score(&terms, doc);
            let mut base = match semantic.and_then(|s| s.normalized(doc.id)) {
                Some(sem) => (0.6 * sem + 0.4 * lexical).max(0.85 * lexical),
                None => lexical,
            };
            if !query_tokens.is_empty() && tokenize(&doc.title) == query_tokens {
                base = base.max(0.95);
            }
            Scored {
                id: doc.id,
                score: base + history_bonus(doc, now_ms),
                base,
                favorite: doc.favorite,
                usage: doc.usage_count,
            }
        })
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.favorite.cmp(&a.favorite))
            .then(b.usage.cmp(&a.usage))
            .then(a.id.cmp(&b.id))
    });
    scored
}

/// Runs retrieval against the library. `semantic` is `None` in standard mode.
pub fn search(
    lib: &Library,
    query: &str,
    semantic: Option<&SemanticScores>,
    partially_indexed: bool,
    now_ms: i64,
) -> AppResult<SearchOutcome> {
    let query = query.trim();
    if query_terms(query).is_empty() {
        return Err(AppError::Validation("Describe what you're trying to accomplish.".into()));
    }

    let mut ids: Vec<i64> = sparks::fts_candidates(lib, &fts_query(query), FTS_CANDIDATES)?
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    if let Some(sem) = semantic {
        let mut seen: HashSet<i64> = ids.iter().copied().collect();
        for id in sem.top_ids(SEMANTIC_CANDIDATES) {
            if seen.insert(id) {
                ids.push(id);
            }
        }
    }

    let docs = sparks::search_docs(lib, &ids)?;
    let ranked = rank(query, &docs, semantic, now_ms);
    let by_id: HashMap<i64, &SearchDoc> = docs.iter().map(|d| (d.id, d)).collect();

    let summary_of = |s: &Scored| -> SparkSummary {
        let d = by_id[&s.id];
        SparkSummary {
            id: d.id,
            title: d.title.clone(),
            summary: d.summary.clone(),
            tags: d.tags.clone(),
            favorite: d.favorite,
            usage_count: d.usage_count,
        }
    };

    let top = ranked.first();
    let confidence = match top {
        Some(s) if s.base >= STRONG_MATCH => Confidence::Strong,
        Some(s) if s.base >= CANDIDATE_MIN => Confidence::Weak,
        _ => Confidence::None,
    };
    let (best, alternatives) = match confidence {
        Confidence::Strong => (top.map(summary_of), Vec::new()),
        _ => (
            None,
            ranked
                .iter()
                .filter(|s| s.base >= CANDIDATE_MIN)
                .take(MAX_ALTERNATIVES)
                .map(summary_of)
                .collect(),
        ),
    };

    Ok(SearchOutcome {
        query: query.to_string(),
        mode: if semantic.is_some() { SearchMode::Semantic } else { SearchMode::Standard },
        confidence,
        best,
        alternatives,
        partially_indexed: semantic.is_some() && partially_indexed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sparks::seed::seed_starter_sparks;
    use crate::sparks::{create, SparkInput};

    fn seeded() -> Library {
        let mut lib = Library::open_in_memory();
        seed_starter_sparks(&mut lib).unwrap();
        lib
    }

    fn best_title(lib: &Library, q: &str) -> Option<String> {
        search(lib, q, None, false, 0).unwrap().best.map(|b| b.title)
    }

    #[test]
    fn intent_queries_find_the_right_spark_without_ai() {
        let lib = seeded();
        let cases = [
            ("I need AI to help me build an MCP server.", "MCP Server Architect"),
            ("help me write a youtube video script", "YouTube Script Architect"),
            ("research a topic deeply with sources", "Deep Research Framework"),
            ("improve my prompt", "Prompt Engineering Master"),
            ("design a multi-agent system", "AI Agent System Designer"),
            ("debugging a production failure, find the root cause", "Root Cause Detective"),
            ("plan the launch of my new project", "Project Launch Planner"),
            ("codex architecture", "Codex Architecture Expert"),
        ];
        for (query, expected) in cases {
            assert_eq!(best_title(&lib, query).as_deref(), Some(expected), "query: {query}");
        }
    }

    #[test]
    fn exact_title_wins() {
        let lib = seeded();
        assert_eq!(best_title(&lib, "root cause detective").as_deref(), Some("Root Cause Detective"));
    }

    #[test]
    fn unrelated_query_is_not_a_confident_match() {
        let lib = seeded();
        let out = search(&lib, "bake sourdough bread at home", None, false, 0).unwrap();
        assert_ne!(out.confidence, Confidence::Strong);
        assert!(out.best.is_none());
    }

    #[test]
    fn empty_library_returns_no_match() {
        let lib = Library::open_in_memory();
        let out = search(&lib, "build an mcp server", None, false, 0).unwrap();
        assert_eq!(out.confidence, Confidence::None);
        assert!(out.best.is_none());
        assert!(out.alternatives.is_empty());
    }

    #[test]
    fn empty_query_is_rejected() {
        let lib = seeded();
        assert!(matches!(search(&lib, "   ", None, false, 0), Err(AppError::Validation(_))));
    }

    #[test]
    fn weak_match_offers_closest_candidates() {
        let mut lib = Library::open_in_memory();
        create(
            &mut lib,
            SparkInput {
                title: "Email Drafter".into(),
                summary: "Writes polite emails.".into(),
                body: "Draft an email. Mention the budget when relevant.".into(),
                ..Default::default()
            },
        )
        .unwrap();
        // Partial evidence (summary + body hits, one term unmatched): weak.
        let out = search(&lib, "polite budget forecast", None, false, 0).unwrap();
        assert_eq!(out.confidence, Confidence::Weak);
        assert!(out.best.is_none());
        assert_eq!(out.alternatives.len(), 1);
        assert_eq!(out.alternatives[0].title, "Email Drafter");

        // A single body-level hit among many terms is not worth offering.
        let out = search(&lib, "quarterly budget forecast spreadsheet", None, false, 0).unwrap();
        assert_eq!(out.confidence, Confidence::None);
        assert!(out.alternatives.is_empty());
    }

    #[test]
    fn ranking_is_deterministic_for_duplicates() {
        let mut lib = Library::open_in_memory();
        for (i, title) in ["Twin Spark", "Twin Spark"].into_iter().enumerate() {
            create(
                &mut lib,
                SparkInput {
                    title: title.into(),
                    body: format!("{title} body {i}"),
                    ..Default::default()
                },
            )
            .unwrap();
        }
        let a = search(&lib, "twin spark", None, false, 0).unwrap().best.unwrap().id;
        for _ in 0..5 {
            assert_eq!(search(&lib, "twin spark", None, false, 0).unwrap().best.unwrap().id, a);
        }
        assert_eq!(a, 1, "ties break toward the older Spark");
    }

    #[test]
    fn history_is_only_a_tie_breaker() {
        let docs = vec![
            SearchDoc {
                id: 1,
                title: "MCP Server Architect".into(),
                summary: String::new(),
                tags: vec![],
                body: String::new(),
                favorite: false,
                usage_count: 0,
                last_copied_at: None,
            },
            SearchDoc {
                id: 2,
                title: "Recipe Helper".into(),
                summary: "server".into(),
                tags: vec![],
                body: String::new(),
                favorite: true,
                usage_count: 10_000,
                last_copied_at: Some(0),
            },
        ];
        let ranked = rank("mcp server", &docs, None, 0);
        assert_eq!(ranked[0].id, 1);
    }

    #[test]
    fn semantic_signal_finds_matches_without_shared_words() {
        // Doc 1 has no lexical overlap with the query but a strongly aligned
        // vector; the other docs sit at the baseline.
        let docs: Vec<SearchDoc> = (1..=6)
            .map(|id| SearchDoc {
                id,
                title: format!("Spark {id}"),
                summary: String::new(),
                tags: vec![],
                body: String::new(),
                favorite: false,
                usage_count: 0,
                last_copied_at: None,
            })
            .collect();
        let mut index = VectorIndex::empty(Some("m".into()));
        index.insert(1, vectors::normalized(vec![1.0, 0.1, 0.0]));
        for id in 2..=6 {
            index.insert(id, vectors::normalized(vec![0.3, 1.0, 0.2 * id as f32]));
        }
        let sem = SemanticScores::from_index(&index, &[1.0, 0.0, 0.0]).unwrap();
        let ranked = rank("grow my channel audience", &docs, Some(&sem), 0);
        assert_eq!(ranked[0].id, 1);
        assert!(ranked[0].base >= STRONG_MATCH);
        assert!(ranked[1].base < STRONG_MATCH);
    }

    #[test]
    fn semantic_and_lexical_fuse() {
        let lib = seeded();
        let mut index = VectorIndex::empty(Some("m".into()));
        let ids = sparks::all_ids(&lib).unwrap();
        // All vectors identical: semantic carries no preference, lexical decides.
        for id in &ids {
            index.insert(*id, vec![1.0, 0.0]);
        }
        let sem = SemanticScores::from_index(&index, &[1.0, 0.0]).unwrap();
        let out = search(&lib, "build an mcp server", Some(&sem), false, 0).unwrap();
        assert_eq!(out.mode, SearchMode::Semantic);
        assert_eq!(out.best.unwrap().title, "MCP Server Architect");
    }
}

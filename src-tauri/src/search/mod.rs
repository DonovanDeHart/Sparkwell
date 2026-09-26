//! Retrieval: lexical (always available) + semantic (when local embeddings are
//! ready), fused into one decisive Best Match.
//!
//! Scores are internal ranking signals only. The UI shows qualitative states
//! (Best Match / no strong match) and which retrieval mode produced them, never
//! a fabricated percentage.

pub mod pool;
pub mod profile;
pub mod text;
pub mod vectors;

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::sparks::{self, SearchDoc, SparkSummary};
use crate::storage::Library;
use text::{fts_query, is_stopword, query_terms, stem, tokenize, tokens_match};
use vectors::VectorIndex;

/// Standard mode: minimum score for a confident Best Match.
pub const STRONG_MATCH: f64 = 0.42;
/// Standard mode: minimum score for a Spark to be offered as a "closest" candidate.
pub const CANDIDATE_MIN: f64 = 0.10;
const FTS_CANDIDATES: usize = 50;
const SEMANTIC_CANDIDATES: usize = 20;
const MAX_ALTERNATIVES: usize = 3;
/// Only the start of very long bodies is scanned for lexical coverage; the
/// full body is still indexed by FTS for recall.
const BODY_SCAN_CHARS: usize = 20_000;
/// Standard mode: two different Sparks this close together are an ambiguous
/// result, so both are offered instead of pretending to know which was meant.
const AMBIGUITY_MARGIN: f64 = 0.10;
/// Standard mode: above this the top result is decisive even if another Spark
/// is close (e.g. an exact title typed as the goal).
const DECISIVE_MATCH: f64 = 0.8;

/// Semantic mode: fused = SEM_WEIGHT · semantic + LEX_WEIGHT · lexical, where
/// lexical terms are weighted by how rare they are in the library so a shared
/// common word ("research", "sources") can't outvote meaning.
const SEM_WEIGHT: f64 = 0.6;
const LEX_WEIGHT: f64 = 0.4;
/// Sparks not embedded yet (brief, while indexing) compete on words alone.
const UNINDEXED_FACTOR: f64 = 0.85;
/// Semantic mode confidence. Calibrated on the physical acceptance library
/// (tests/semantic_regression.rs) in the middle of the range where no known
/// goal gets a confidently wrong answer (T 0.22–0.38, M 0.12–0.14 all hold).
const SEM_STRONG: f64 = 0.30;
const SEM_MARGIN: f64 = 0.12;
const SEM_CANDIDATE_MIN: f64 = 0.10;
/// Two leading Sparks whose purposes are this alike are interchangeable for
/// the goal, so a narrow lead between them isn't treated as ambiguity.
const SAME_PURPOSE: f32 = 0.85;
/// Purpose similarity is precomputed for this many leading semantic candidates.
const PURPOSE_CANDIDATES: usize = 12;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SearchMode {
    /// Semantic + lexical fusion.
    Semantic,
    /// Lexical/title/tag/full-text only.
    Standard,
}

/// Why a search used standard retrieval instead of local intelligence. The UI
/// always labels standard results so the mode never changes silently.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Fallback {
    /// Ollama isn't running (or hasn't been detected yet).
    Offline,
    /// Ollama is running but no local embedding model is installed.
    NoEmbeddingModel,
    /// The library is still being indexed for local intelligence.
    Indexing,
    /// The embedding model didn't answer within the time budget.
    TimedOut,
    /// Local intelligence answered with an error.
    Failed,
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
    /// Present in standard mode: why local intelligence wasn't used.
    pub fallback: Option<Fallback>,
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

/// Semantic evidence for one query, ready for fusion.
pub struct SemanticScores {
    scores: HashMap<i64, f64>,
    purpose: HashMap<(i64, i64), f32>,
}

fn pair_key(a: i64, b: i64) -> (i64, i64) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

impl SemanticScores {
    /// Returns `None` when the index can't provide trustworthy evidence
    /// (not enough indexed Sparks, no generic pool, wrong dimensions).
    pub fn from_index(index: &VectorIndex, query_vector: &[f32]) -> Option<SemanticScores> {
        let mut evidence = index.evidence(query_vector);
        if evidence.is_empty() {
            return None;
        }
        evidence.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.id.cmp(&b.id))
        });
        let lead: Vec<i64> = evidence
            .iter()
            .take(PURPOSE_CANDIDATES)
            .map(|e| e.id)
            .collect();
        let mut purpose = HashMap::new();
        for (i, a) in lead.iter().enumerate() {
            for b in &lead[i + 1..] {
                if let Some(s) = index.purpose_similarity(*a, *b) {
                    purpose.insert(pair_key(*a, *b), s);
                }
            }
        }
        Some(SemanticScores {
            scores: evidence.into_iter().map(|e| (e.id, e.score)).collect(),
            purpose,
        })
    }

    /// Test/support constructor from explicit scores.
    pub fn from_scores(scores: HashMap<i64, f64>, purpose: HashMap<(i64, i64), f32>) -> Self {
        let purpose = purpose
            .into_iter()
            .map(|((a, b), s)| (pair_key(a, b), s))
            .collect();
        SemanticScores { scores, purpose }
    }

    fn normalized(&self, id: i64) -> Option<f64> {
        self.scores.get(&id).copied()
    }

    fn same_purpose(&self, a: i64, b: i64) -> bool {
        self.purpose
            .get(&pair_key(a, b))
            .is_some_and(|s| *s >= SAME_PURPOSE)
    }

    fn top_ids(&self, n: usize) -> Vec<i64> {
        let mut all: Vec<(i64, f64)> = self.scores.iter().map(|(k, v)| (*k, *v)).collect();
        all.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
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
/// title the query covers. Range 0..=1. With `weights`, each term counts in
/// proportion to its rarity in the library.
pub fn lexical_score(
    terms: &[String],
    doc: &SearchDoc,
    weights: Option<&HashMap<String, f64>>,
) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let title = stemmed_tokens(&doc.title);
    let tags: Vec<String> = doc.tags.iter().flat_map(|t| stemmed_tokens(t)).collect();
    let summary = stemmed_tokens(&doc.summary);
    let body_head: String = doc.body.chars().take(BODY_SCAN_CHARS).collect();
    let body = stemmed_tokens(&body_head);

    let (mut covered, mut total) = (0.0, 0.0);
    for term in terms {
        let w = weights.and_then(|w| w.get(term)).copied().unwrap_or(1.0);
        total += w;
        covered += w * if any_match(term, &title) {
            1.0
        } else if any_match(term, &tags) {
            0.85
        } else if any_match(term, &summary) {
            0.55
        } else if any_match(term, &body) {
            0.25
        } else {
            0.0
        };
    }
    let coverage = if total > 0.0 { covered / total } else { 0.0 };

    let title_content: Vec<String> = tokenize(&doc.title)
        .into_iter()
        .filter(|t| !is_stopword(t))
        .map(|t| stem(&t))
        .collect();
    let title_recall = if title_content.is_empty() {
        0.0
    } else {
        title_content.iter().filter(|t| any_match(t, terms)).count() as f64
            / title_content.len() as f64
    };

    0.8 * coverage + 0.2 * title_recall
}

/// Rarity weight per query term: ln(1 + N / (1 + documents containing it)).
pub fn term_weights(lib: &Library, terms: &[String]) -> AppResult<HashMap<String, f64>> {
    let n = lib.spark_count()?.max(1) as f64;
    let mut out = HashMap::new();
    for term in terms {
        let df = sparks::document_frequency(lib, term)? as f64;
        out.insert(term.clone(), (1.0 + n / (1.0 + df)).ln());
    }
    Ok(out)
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
pub fn rank(
    query: &str,
    docs: &[SearchDoc],
    semantic: Option<&SemanticScores>,
    weights: Option<&HashMap<String, f64>>,
    now_ms: i64,
) -> Vec<Scored> {
    let terms = query_terms(query);
    let query_tokens = tokenize(query);
    let mut scored: Vec<Scored> = docs
        .iter()
        .map(|doc| {
            let mut base = match semantic {
                Some(s) => {
                    let lexical = lexical_score(&terms, doc, weights);
                    match s.normalized(doc.id) {
                        Some(sem) => SEM_WEIGHT * sem + LEX_WEIGHT * lexical,
                        None => UNINDEXED_FACTOR * lexical,
                    }
                }
                None => lexical_score(&terms, doc, None),
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

/// How confident the ranking is, and the minimum score for a candidate.
fn judge(ranked: &[Scored], semantic: Option<&SemanticScores>) -> (Confidence, f64) {
    let (Some(top), second) = (ranked.first(), ranked.get(1)) else {
        return (Confidence::None, CANDIDATE_MIN);
    };
    let runner_up = second.map(|s| s.base).unwrap_or(0.0);
    match semantic {
        Some(sem) => {
            let interchangeable = second.is_some_and(|s| sem.same_purpose(top.id, s.id));
            let confidence = if top.base >= SEM_STRONG
                && (top.base - runner_up >= SEM_MARGIN || interchangeable)
            {
                Confidence::Strong
            } else if top.base >= SEM_CANDIDATE_MIN {
                Confidence::Weak
            } else {
                Confidence::None
            };
            (confidence, SEM_CANDIDATE_MIN)
        }
        None => {
            let confidence = if top.base >= STRONG_MATCH
                && (top.base - runner_up >= AMBIGUITY_MARGIN || top.base >= DECISIVE_MATCH)
            {
                Confidence::Strong
            } else if top.base >= CANDIDATE_MIN {
                Confidence::Weak
            } else {
                Confidence::None
            };
            (confidence, CANDIDATE_MIN)
        }
    }
}

/// Runs retrieval against the library. `semantic` is `None` in standard mode,
/// in which case `fallback` says why.
pub fn search(
    lib: &Library,
    query: &str,
    semantic: Option<&SemanticScores>,
    fallback: Option<Fallback>,
    partially_indexed: bool,
    now_ms: i64,
) -> AppResult<SearchOutcome> {
    let query = query.trim();
    let terms = query_terms(query);
    if terms.is_empty() {
        return Err(AppError::Validation(
            "Describe what you're trying to accomplish.".into(),
        ));
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
    let weights = match semantic {
        Some(_) => Some(term_weights(lib, &terms)?),
        None => None,
    };
    let ranked = rank(query, &docs, semantic, weights.as_ref(), now_ms);
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

    let (confidence, candidate_min) = judge(&ranked, semantic);
    let (best, alternatives) = match confidence {
        Confidence::Strong => (ranked.first().map(summary_of), Vec::new()),
        _ => (
            None,
            ranked
                .iter()
                .filter(|s| s.base >= candidate_min)
                .take(MAX_ALTERNATIVES)
                .map(summary_of)
                .collect(),
        ),
    };

    let mode = if semantic.is_some() {
        SearchMode::Semantic
    } else {
        SearchMode::Standard
    };
    Ok(SearchOutcome {
        query: query.to_string(),
        mode,
        fallback: if semantic.is_some() {
            None
        } else {
            Some(fallback.unwrap_or(Fallback::Offline))
        },
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

    fn standard(lib: &Library, q: &str) -> SearchOutcome {
        search(lib, q, None, Some(Fallback::Offline), false, 0).unwrap()
    }

    fn best_title(lib: &Library, q: &str) -> Option<String> {
        standard(lib, q).best.map(|b| b.title)
    }

    #[test]
    fn intent_queries_find_the_right_spark_without_ai() {
        let lib = seeded();
        let cases = [
            (
                "I need AI to help me build an MCP server.",
                "MCP Server Architect",
            ),
            (
                "help me write a youtube video script",
                "YouTube Script Architect",
            ),
            (
                "research a topic deeply with sources",
                "Deep Research Framework",
            ),
            ("improve my prompt", "Prompt Engineering Master"),
            ("design a multi-agent system", "AI Agent System Designer"),
            (
                "debugging a production failure, find the root cause",
                "Root Cause Detective",
            ),
            (
                "plan the launch of my new project",
                "Project Launch Planner",
            ),
            ("codex architecture", "Codex Architecture Expert"),
        ];
        for (query, expected) in cases {
            assert_eq!(
                best_title(&lib, query).as_deref(),
                Some(expected),
                "query: {query}"
            );
        }
    }

    #[test]
    fn standard_results_always_say_why() {
        let lib = seeded();
        let out = search(&lib, "mcp server", None, Some(Fallback::TimedOut), false, 0).unwrap();
        assert_eq!(out.mode, SearchMode::Standard);
        assert_eq!(out.fallback, Some(Fallback::TimedOut));
        // A missing reason is never reported as semantic.
        let out = search(&lib, "mcp server", None, None, false, 0).unwrap();
        assert_eq!(out.fallback, Some(Fallback::Offline));
    }

    #[test]
    fn exact_title_wins() {
        let lib = seeded();
        assert_eq!(
            best_title(&lib, "root cause detective").as_deref(),
            Some("Root Cause Detective")
        );
    }

    #[test]
    fn unrelated_query_is_not_a_confident_match() {
        let lib = seeded();
        let out = standard(&lib, "bake sourdough bread at home");
        assert_ne!(out.confidence, Confidence::Strong);
        assert!(out.best.is_none());
    }

    #[test]
    fn empty_library_returns_no_match() {
        let lib = Library::open_in_memory();
        let out = standard(&lib, "build an mcp server");
        assert_eq!(out.confidence, Confidence::None);
        assert!(out.best.is_none());
        assert!(out.alternatives.is_empty());
    }

    #[test]
    fn empty_query_is_rejected() {
        let lib = seeded();
        assert!(matches!(
            search(&lib, "   ", None, None, false, 0),
            Err(AppError::Validation(_))
        ));
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
        let out = standard(&lib, "polite budget forecast");
        assert_eq!(out.confidence, Confidence::Weak);
        assert!(out.best.is_none());
        assert_eq!(out.alternatives.len(), 1);
        assert_eq!(out.alternatives[0].title, "Email Drafter");

        // A single body-level hit among many terms is not worth offering.
        let out = standard(&lib, "quarterly budget forecast spreadsheet");
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
        let a = standard(&lib, "twin spark").best.unwrap().id;
        for _ in 0..5 {
            assert_eq!(standard(&lib, "twin spark").best.unwrap().id, a);
        }
        assert_eq!(a, 1, "ties break toward the older Spark");
    }

    fn doc(id: i64, title: &str) -> SearchDoc {
        SearchDoc {
            id,
            title: title.into(),
            summary: String::new(),
            tags: vec![],
            body: String::new(),
            favorite: false,
            usage_count: 0,
            last_copied_at: None,
        }
    }

    #[test]
    fn history_is_only_a_tie_breaker() {
        let mut recipe = doc(2, "Recipe Helper");
        recipe.summary = "server".into();
        recipe.favorite = true;
        recipe.usage_count = 10_000;
        recipe.last_copied_at = Some(0);
        let docs = vec![doc(1, "MCP Server Architect"), recipe];
        let ranked = rank("mcp server", &docs, None, None, 0);
        assert_eq!(ranked[0].id, 1);
    }

    fn semantic(scores: &[(i64, f64)], purpose: &[((i64, i64), f32)]) -> SemanticScores {
        SemanticScores::from_scores(
            scores.iter().copied().collect(),
            purpose.iter().copied().collect(),
        )
    }

    #[test]
    fn semantic_signal_finds_matches_without_shared_words() {
        let docs: Vec<SearchDoc> = (1..=6).map(|id| doc(id, &format!("Spark {id}"))).collect();
        let sem = semantic(
            &[(1, 0.9), (2, 0.1), (3, 0.05), (4, 0.0), (5, 0.0), (6, 0.0)],
            &[],
        );
        let ranked = rank("grow my channel audience", &docs, Some(&sem), None, 0);
        assert_eq!(ranked[0].id, 1);
        let (confidence, _) = judge(&ranked, Some(&sem));
        assert_eq!(confidence, Confidence::Strong);
    }

    #[test]
    fn near_ties_between_different_sparks_are_not_confident() {
        let docs: Vec<SearchDoc> = (1..=3).map(|id| doc(id, &format!("Spark {id}"))).collect();
        let sem = semantic(&[(1, 0.80), (2, 0.74), (3, 0.1)], &[((1, 2), 0.40)]);
        let ranked = rank("goal", &docs, Some(&sem), None, 0);
        assert_eq!(judge(&ranked, Some(&sem)).0, Confidence::Weak);
        // The same lead between two Sparks with the same purpose is decisive:
        // either one serves the goal.
        let sem = semantic(&[(1, 0.80), (2, 0.74), (3, 0.1)], &[((1, 2), 0.90)]);
        let ranked = rank("goal", &docs, Some(&sem), None, 0);
        assert_eq!(judge(&ranked, Some(&sem)).0, Confidence::Strong);
    }

    #[test]
    fn weak_semantic_evidence_is_never_a_best_match() {
        let docs: Vec<SearchDoc> = (1..=3).map(|id| doc(id, &format!("Spark {id}"))).collect();
        let sem = semantic(&[(1, 0.40), (2, 0.0), (3, 0.0)], &[]);
        let ranked = rank("goal", &docs, Some(&sem), None, 0);
        // 0.6 * 0.40 = 0.24 < SEM_STRONG.
        assert_eq!(judge(&ranked, Some(&sem)).0, Confidence::Weak);
        let sem = semantic(&[(1, 0.05), (2, 0.0), (3, 0.0)], &[]);
        let ranked = rank("goal", &docs, Some(&sem), None, 0);
        assert_eq!(judge(&ranked, Some(&sem)).0, Confidence::None);
    }

    #[test]
    fn rare_terms_outweigh_common_ones() {
        let mut a = doc(1, "Alpha");
        a.summary = "common words everywhere".into();
        let mut b = doc(2, "Beta");
        b.summary = "zebra".into();
        let terms = query_terms("common zebra");
        let weights: HashMap<String, f64> =
            [("common".to_string(), 0.1), ("zebra".to_string(), 2.0)]
                .into_iter()
                .collect();
        assert!(
            lexical_score(&terms, &b, Some(&weights)) > lexical_score(&terms, &a, Some(&weights))
        );
        // Unweighted, they tie.
        assert!((lexical_score(&terms, &a, None) - lexical_score(&terms, &b, None)).abs() < 1e-9);
    }

    #[test]
    fn semantic_mode_reports_itself_and_uses_weights() {
        let lib = seeded();
        let ids = sparks::all_ids(&lib).unwrap();
        let scores: HashMap<i64, f64> = ids.iter().map(|id| (*id, 0.0)).collect();
        let sem = SemanticScores::from_scores(scores, HashMap::new());
        let out = search(&lib, "build an mcp server", Some(&sem), None, false, 0).unwrap();
        assert_eq!(out.mode, SearchMode::Semantic);
        assert_eq!(out.fallback, None);
        // No semantic preference: words decide, but alone they stay below the
        // semantic confidence bar, so the Spark is offered, not asserted.
        let leader = out
            .best
            .or_else(|| out.alternatives.first().cloned())
            .unwrap();
        assert_eq!(leader.title, "MCP Server Architect");
    }
}

//! Shared semantic-retrieval evaluation for `semantic_live` (real Ollama) and
//! `semantic_regression` (recorded vectors, runs in CI).
//!
//! The library is the one used in physical acceptance test round 1
//! (fixtures/acceptance_library.json). Queries:
//! - ACCEPTANCE: the 12 goals from that test, with the expected Spark and the
//!   alternatives that are semantically justified;
//! - DEV: 36 further paraphrases used while tuning, so calibration isn't
//!   fitted to the 12 alone;
//! - UNRELATED: goals no Spark serves, which must never get a Best Match.

#![allow(dead_code)]

use std::collections::HashMap;

use sparkwell_lib::ai::{models, spark_views};
use sparkwell_lib::search::pool::GENERIC_GOALS;
use sparkwell_lib::search::vectors::VectorIndex;
use sparkwell_lib::search::{self, Confidence, SearchOutcome, SemanticScores};
use sparkwell_lib::sparks::{self, SparkInput};
use sparkwell_lib::storage::{Library, OpenMode};

pub const SQL: &str = "You are a database performance engineer. Diagnose and speed…";

pub const ACCEPTANCE: &[(&str, &str, &[&str])] = &[
    ("I need AI to teach me how to create tools and resources that other AI applications can call", "MCP Server Architect", &[]),
    ("Help me teach an autonomous agent a new reusable capability", "AI Agent Skill Builder", &[]),
    ("I need AI to investigate a topic comprehensively and cross-check multiple sources", "Deep Research Framework", &[]),
    ("I want an AI to inspect the structure of my application and identify design weaknesses", "Software Architecture Reviewer", &["Codex Architecture Expert"]),
    ("grow my channel with videos people watch to the end", "YouTube Script Architect", &[]),
    ("my database is crawling, figure out why this query takes forever", SQL, &[]),
    ("turn what we discussed in today's call into tasks with owners", "Meeting Notes to Action Plan", &[]),
    ("get a fresh coding session up to speed on this project", "Project Handoff Brief", &["Codex Architecture Expert"]),
    ("check whether my system prompt has contradictions", "Custom Instructions Analyst", &["Prompt Engineering Master"]),
    ("should we ship this version? run the final checks", "Release Readiness Audit (Long-Form)", &[]),
    ("set up a study plan from the PDFs I uploaded to Google's notebook tool", "NotebookLM Research Brief", &[]),
    ("my app keeps crashing and I can't tell why", "Root Cause Detective", &[]),
];

pub const DEV: &[(&str, &str, &[&str])] = &[
    (
        "have an autonomous coder refactor my repository safely",
        "Codex Architecture Expert",
        &[],
    ),
    (
        "set up my coding agent to plan changes in phases and verify them before committing",
        "Codex Architecture Expert",
        &[],
    ),
    (
        "orchestrate several assistants that hand work to each other",
        "AI Agent System Designer",
        &[],
    ),
    (
        "design the tools, memory and guardrails for an agent I'm building",
        "AI Agent System Designer",
        &["AI Agent Skill Builder"],
    ),
    (
        "find out what the evidence really says about intermittent fasting",
        "Deep Research Framework",
        &[],
    ),
    (
        "I want a rigorous literature review with graded evidence and citations",
        "Deep Research Framework",
        &[],
    ),
    (
        "my instructions to the model keep getting ignored",
        "Prompt Engineering Master",
        &["Custom Instructions Analyst"],
    ),
    (
        "rewrite this prompt so it's clearer and easier to test",
        "Prompt Engineering Master",
        &[],
    ),
    (
        "give Claude tools to talk to my database",
        "MCP Server Architect",
        &[],
    ),
    (
        "expose my company's API so ChatGPT and Claude can call it as tools",
        "MCP Server Architect",
        &[],
    ),
    (
        "write a hook and script for my next video",
        "YouTube Script Architect",
        &[],
    ),
    (
        "plan a video that keeps viewers watching until the end",
        "YouTube Script Architect",
        &[],
    ),
    (
        "our service goes down every night and nobody knows why",
        "Root Cause Detective",
        &[],
    ),
    (
        "help me debug an intermittent failure step by step",
        "Root Cause Detective",
        &[],
    ),
    (
        "turn my rough startup idea into a plan",
        "Project Launch Planner",
        &[],
    ),
    (
        "break my side project into milestones with the first steps I can take today",
        "Project Launch Planner",
        &[],
    ),
    (
        "package a repeatable workflow so my Claude Code agent can use it as a skill",
        "AI Agent Skill Builder",
        &[],
    ),
    (
        "teach my agent how to do a new task it can reuse later",
        "AI Agent Skill Builder",
        &[],
    ),
    (
        "audit my system design for scalability and reliability risks",
        "Software Architecture Reviewer",
        &[],
    ),
    (
        "critique the architecture of my web app before we scale it",
        "Software Architecture Reviewer",
        &["Codex Architecture Expert"],
    ),
    (
        "make my python module cleaner without changing what it does",
        "Python Refactoring Coach",
        &[],
    ),
    (
        "teach me how to refactor this messy code step by step",
        "Python Refactoring Coach",
        &["Codex Architecture Expert"],
    ),
    (
        "are we ready to launch this version? do a final checklist",
        "Release Readiness Audit (Long-Form)",
        &[],
    ),
    (
        "go/no-go review before shipping the release candidate",
        "Release Readiness Audit (Long-Form)",
        &[],
    ),
    (
        "review my ChatGPT custom instructions and tighten them",
        "Custom Instructions Analyst",
        &[],
    ),
    (
        "why does my system prompt behave inconsistently",
        "Custom Instructions Analyst",
        &["Prompt Engineering Master"],
    ),
    (
        "summarize this project so a new developer can pick it up tomorrow",
        "Project Handoff Brief",
        &[],
    ),
    (
        "write context for a new AI chat so it can continue where we left off",
        "Project Handoff Brief",
        &[],
    ),
    (
        "I uploaded papers to NotebookLM, what should I ask it?",
        "NotebookLM Research Brief",
        &[],
    ),
    (
        "make a study plan from my notebook sources",
        "NotebookLM Research Brief",
        &[],
    ),
    (
        "this postgres query is slow, which index do I need?",
        SQL,
        &[],
    ),
    ("read my EXPLAIN plan and tell me what's wrong", SQL, &[]),
    (
        "turn this transcript into action items with owners",
        "Meeting Notes to Action Plan",
        &[],
    ),
    (
        "summarize decisions and to-dos from our standup notes",
        "Meeting Notes to Action Plan",
        &[],
    ),
    (
        "I need AI to help me build an MCP server",
        "MCP Server Architect",
        &[],
    ),
    ("grow my youtube channel", "YouTube Script Architect", &[]),
];

pub const UNRELATED: &[&str] = &[
    "bake sourdough bread at home",
    "plan the seating chart for my wedding",
    "what's the weather in Denver tomorrow",
    "recommend a good sci-fi novel to read",
    "fix my bike's squeaky brakes",
    "write a birthday poem for my mom",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grade {
    /// Confident Best Match of the expected Spark.
    Correct,
    /// A justified alternative as Best Match, or no strong match with the
    /// expected (or a justified) Spark offered among the closest.
    Acceptable,
    /// Expected Spark not shown at all (no strong match, not offered).
    Missed,
    /// Best Match is a Spark that doesn't serve the goal.
    ConfidentlyWrong,
}

pub fn grade(outcome: &SearchOutcome, expected: &str, alternatives: &[&str]) -> Grade {
    let ok = |t: &str| t == expected || alternatives.contains(&t);
    match (&outcome.best, outcome.confidence) {
        (Some(best), Confidence::Strong) if best.title == expected => Grade::Correct,
        (Some(best), Confidence::Strong) if ok(&best.title) => Grade::Acceptable,
        (Some(_), Confidence::Strong) => Grade::ConfidentlyWrong,
        _ if outcome.alternatives.iter().any(|a| ok(&a.title)) => Grade::Acceptable,
        _ => Grade::Missed,
    }
}

pub fn leader(o: &SearchOutcome) -> String {
    o.best
        .as_ref()
        .or_else(|| o.alternatives.first())
        .map(|s| s.title.clone())
        .unwrap_or_else(|| "—".into())
}

#[derive(serde::Deserialize)]
struct FixtureSpark {
    title: String,
    summary: String,
    tags: Vec<String>,
    favorite: bool,
    body: String,
}

#[derive(serde::Deserialize)]
struct Fixture {
    sparks: Vec<FixtureSpark>,
}

/// The acceptance library, created through the normal repository code.
pub fn acceptance_library(dir: &std::path::Path) -> Library {
    let raw = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/acceptance_library.json"),
    )
    .expect("fixture");
    let fixture: Fixture = serde_json::from_str(&raw).expect("fixture json");
    let mut lib = Library::open(dir, OpenMode::CreateIfMissing).expect("library");
    for s in fixture.sparks {
        sparks::create(
            &mut lib,
            SparkInput {
                title: s.title,
                summary: s.summary,
                body: s.body,
                tags: s.tags,
                favorite: s.favorite,
                source_note: None,
                allow_duplicate: true,
            },
        )
        .expect("create");
    }
    lib
}

/// Every text the pipeline embeds for this library and query set.
pub struct Texts {
    pub docs: Vec<(i64, Vec<String>)>,
    pub pool: Vec<String>,
}

pub fn document_texts(lib: &Library, model: &str) -> Texts {
    let docs = sparks::search_docs(lib, &sparks::all_ids(lib).unwrap()).unwrap();
    let mut docs: Vec<(i64, Vec<String>)> = docs
        .iter()
        .map(|d| {
            (
                d.id,
                spark_views(d)
                    .iter()
                    .map(|v| models::document_text(model, v))
                    .collect(),
            )
        })
        .collect();
    docs.sort_by_key(|(id, _)| *id);
    Texts {
        docs,
        pool: GENERIC_GOALS
            .iter()
            .map(|g| models::pool_text(model, g))
            .collect(),
    }
}

pub fn all_queries() -> Vec<String> {
    ACCEPTANCE
        .iter()
        .map(|(q, _, _)| q.to_string())
        .chain(DEV.iter().map(|(q, _, _)| q.to_string()))
        .chain(UNRELATED.iter().map(|q| q.to_string()))
        .collect()
}

pub fn build_index(model: &str, texts: &Texts, embed: &dyn Fn(&str) -> Vec<f32>) -> VectorIndex {
    let mut index = VectorIndex::empty(Some(model.to_string()));
    for (id, views) in &texts.docs {
        index.insert(*id, views.iter().map(|t| embed(t)).collect());
    }
    index.set_pool(texts.pool.iter().map(|t| embed(t)).collect());
    index
}

#[derive(Default)]
pub struct Report {
    pub acceptance: Vec<(String, String, Grade)>,
    pub dev: Vec<(String, String, Grade)>,
    pub false_confident: Vec<String>,
}

impl Report {
    pub fn count(list: &[(String, String, Grade)], g: Grade) -> usize {
        list.iter().filter(|(_, _, x)| *x == g).count()
    }

    pub fn print(&self) {
        println!("\nAcceptance goals:");
        for (q, leader, g) in &self.acceptance {
            println!("  {g:<16?} {q}\n                   -> {leader}");
        }
        for (label, list) in [("acceptance", &self.acceptance), ("dev", &self.dev)] {
            println!(
                "{label}: correct {} · acceptable {} · missed {} · confidently wrong {} (of {})",
                Self::count(list, Grade::Correct),
                Self::count(list, Grade::Acceptable),
                Self::count(list, Grade::Missed),
                Self::count(list, Grade::ConfidentlyWrong),
                list.len()
            );
        }
        println!(
            "unrelated goals shown as a confident match: {:?}",
            self.false_confident
        );
    }
}

pub fn evaluate(
    lib: &Library,
    index: &VectorIndex,
    model: &str,
    embed: &dyn Fn(&str) -> Vec<f32>,
) -> Report {
    let run = |q: &str| {
        let v = embed(&models::query_text(model, q));
        let sem = SemanticScores::from_index(index, &v).expect("index ready");
        search::search(lib, q, Some(&sem), None, false, 0).unwrap()
    };
    let mut report = Report::default();
    for (q, expected, alts) in ACCEPTANCE {
        let o = run(q);
        report
            .acceptance
            .push((q.to_string(), leader(&o), grade(&o, expected, alts)));
    }
    for (q, expected, alts) in DEV {
        let o = run(q);
        report
            .dev
            .push((q.to_string(), leader(&o), grade(&o, expected, alts)));
    }
    for q in UNRELATED {
        if run(q).confidence == Confidence::Strong {
            report.false_confident.push(q.to_string());
        }
    }
    report
}

/// Every text embedded by `evaluate` + `build_index`.
pub fn every_text(texts: &Texts, model: &str) -> Vec<String> {
    let mut out: Vec<String> = texts
        .docs
        .iter()
        .flat_map(|(_, v)| v.iter().cloned())
        .collect();
    out.extend(texts.pool.iter().cloned());
    out.extend(all_queries().iter().map(|q| models::query_text(model, q)));
    out
}

pub fn fixture_path(model: &str) -> std::path::PathBuf {
    let safe: String = model
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("semantic_{safe}.json"))
}

pub fn text_key(text: &str) -> String {
    sparkwell_lib::storage::content_hash(&[text])
}

// f16 storage keeps the recorded fixture small while preserving ranking.
pub fn f32_to_f16(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    let mant = bits & 0x7f_ffff;
    if exp <= 0 {
        if exp < -10 {
            return sign;
        }
        let m = (mant | 0x80_0000) >> (1 - exp);
        return sign | ((m + 0x1000) >> 13) as u16;
    }
    if exp >= 31 {
        return sign | 0x7c00;
    }
    let half = sign | ((exp as u16) << 10) | ((mant >> 13) as u16);
    // round to nearest
    if mant & 0x1000 != 0 {
        half + 1
    } else {
        half
    }
}

pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let exp = ((h >> 10) & 0x1f) as u32;
    let mant = (h & 0x3ff) as u32;
    let bits = if exp == 0 {
        if mant == 0 {
            sign
        } else {
            let mut e = 127 - 15 + 1;
            let mut m = mant;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            sign | (e << 23) | ((m & 0x3ff) << 13)
        }
    } else if exp == 31 {
        sign | 0x7f80_0000 | (mant << 13)
    } else {
        sign | ((exp + 127 - 15) << 23) | (mant << 13)
    };
    f32::from_bits(bits)
}

pub fn encode_vector(v: &[f32]) -> String {
    use base64::Engine;
    let bytes: Vec<u8> = v
        .iter()
        .flat_map(|x| f32_to_f16(*x).to_le_bytes())
        .collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub fn decode_vector(s: &str) -> Vec<f32> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s)
        .expect("base64");
    bytes
        .chunks_exact(2)
        .map(|c| f16_to_f32(u16::from_le_bytes([c[0], c[1]])))
        .collect()
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedVectors {
    pub model: String,
    pub profile_version: String,
    pub note: String,
    /// content_hash(text) -> base64 little-endian f16 vector
    pub vectors: HashMap<String, String>,
}

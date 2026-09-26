//! Shared semantic-retrieval evaluation for `semantic_live` (real Ollama) and
//! `semantic_regression` (recorded vectors, runs in CI).
//!
//! The library is the one used in physical acceptance test round 1
//! (fixtures/acceptance_library.json). Queries:
//! - ACCEPTANCE: the 12 goals from that test, with the expected Spark and the
//!   alternatives that are semantically justified;
//! - DEV: 36 further paraphrases used while tuning, so calibration isn't
//!   fitted to the 12 alone;
//! - UNRELATED: goals no Spark serves, which must never get a Best Match;
//! - HELDOUT / HELDOUT_UNRELATED: goals written before the 8B calibration
//!   (indirect, conversational, vague, technical, long and short phrasings),
//!   used as the validation set when choosing between calibrations;
//! - TEST / TEST_UNRELATED: written after the first calibration attempt and
//!   never used to tune or choose anything — the honest generalisation check;
//! - PREMIUM: goals for Sparks pasted the way users capture prompts made with
//!   ChatGPT, Claude or Gemini (fixtures/user_added_sparks.json), searched in
//!   the acceptance library plus those Sparks.

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

/// Written before calibrating for qwen3-embedding:8b-q8_0; used as validation.
pub const HELDOUT: &[(&str, &str, &[&str])] = &[
    ("agent tools protocol server", "MCP Server Architect", &["AI Agent System Designer"]),
    (
        "I want ChatGPT to be able to look things up in our company wiki by itself",
        "MCP Server Architect",
        &["AI Agent System Designer"],
    ),
    (
        "So we've been at this app for weeks and I keep losing track. Tomorrow a contractor joins and I need them to hit the ground running without asking me a hundred questions.",
        "Project Handoff Brief",
        &[],
    ),
    ("make my prompts better", "Prompt Engineering Master", &["Custom Instructions Analyst"]),
    ("locate the underlying reason our checkout keeps failing", "Root Cause Detective", &[]),
    ("viewers drop off after thirty seconds", "YouTube Script Architect", &[]),
    ("full table scan on a 40M row table, should I add a composite index?", SQL, &[]),
    ("ugh, the meeting ran long, can you pull out who's doing what", "Meeting Notes to Action Plan", &[]),
    ("pre-launch QA sign-off", "Release Readiness Audit (Long-Form)", &[]),
    ("check if our new version is safe to put in front of customers", "Release Readiness Audit (Long-Form)", &[]),
    (
        "I keep telling the AI in my settings to be concise and it rambles anyway",
        "Custom Instructions Analyst",
        &["Prompt Engineering Master"],
    ),
    ("multi-agent planner and executor with shared memory and tool guardrails", "AI Agent System Designer", &[]),
    ("write a SKILL.md so Claude can reuse my deployment steps", "AI Agent Skill Builder", &[]),
    ("I have an idea for an app but no clue where to start", "Project Launch Planner", &[]),
    ("is splitting our monolith into microservices a mistake?", "Software Architecture Reviewer", &["Codex Architecture Expert"]),
    ("clean up nested ifs and giant functions in my .py files", "Python Refactoring Coach", &[]),
    (
        "let the coding assistant change things across the repo, but carefully and one phase at a time",
        "Codex Architecture Expert",
        &[],
    ),
    ("compare what several studies found and tell me how confident we can be", "Deep Research Framework", &[]),
    ("I dumped my lecture PDFs into NotebookLM, set me up to study them", "NotebookLM Research Brief", &[]),
    (
        "Our Node API's p99 latency spiked after the last deploy, the logs are noisy and I can't reproduce it locally. Walk me through narrowing it down.",
        "Root Cause Detective",
        &[],
    ),
    ("brainstorm a video concept with a strong opening", "YouTube Script Architect", &[]),
    ("what should I build first for my SaaS MVP", "Project Launch Planner", &[]),
    ("why is my ORM generating such slow SQL", SQL, &["Root Cause Detective"]),
];

/// Final test goals, written after the first 8B calibration attempt and
/// never used to tune or choose settings (HELDOUT served as validation).
pub const TEST: &[(&str, &str, &[&str])] = &[
    (
        "hook up our Postgres so Claude can query it through a proper protocol server",
        "MCP Server Architect",
        &["AI Agent System Designer"],
    ),
    (
        "I want my AI assistant to take new abilities it can reuse without me re-explaining",
        "AI Agent Skill Builder",
        &["AI Agent System Designer"],
    ),
    (
        "dig into whether remote work actually lowers productivity, with sources I can trust",
        "Deep Research Framework",
        &[],
    ),
    (
        "poke holes in how our backend services are structured",
        "Software Architecture Reviewer",
        &["Codex Architecture Expert"],
    ),
    (
        "script for a 10 minute explainer video",
        "YouTube Script Architect",
        &[],
    ),
    ("this report query takes 40 seconds, speed it up", SQL, &[]),
    (
        "I have messy notes from a client call, turn them into next steps and who owns them",
        "Meeting Notes to Action Plan",
        &[],
    ),
    (
        "brief the next AI session on everything we did today",
        "Project Handoff Brief",
        &[],
    ),
    (
        "audit my custom GPT's instructions for conflicts",
        "Custom Instructions Analyst",
        &["Prompt Engineering Master"],
    ),
    (
        "final release gate before we tag v2.0",
        "Release Readiness Audit (Long-Form)",
        &[],
    ),
    (
        "I want NotebookLM to teach me organic chemistry from my textbook chapters",
        "NotebookLM Research Brief",
        &[],
    ),
    (
        "production keeps throwing 502s at random",
        "Root Cause Detective",
        &[],
    ),
    (
        "make the AI coding agent work like a careful senior engineer in my codebase",
        "Codex Architecture Expert",
        &[],
    ),
    (
        "design a crew of AI agents with a manager that delegates",
        "AI Agent System Designer",
        &[],
    ),
    (
        "my prompt gives different answers every time, tighten it up",
        "Prompt Engineering Master",
        &["Custom Instructions Analyst"],
    ),
    (
        "turn my weekend project idea into a roadmap",
        "Project Launch Planner",
        &[],
    ),
    (
        "this python script works but it's ugly, help me clean it up safely",
        "Python Refactoring Coach",
        &[],
    ),
    (
        "thumbnail and title ideas plus an outline for my channel",
        "YouTube Script Architect",
        &[],
    ),
    (
        "figure out why our nightly batch job silently skips records",
        "Root Cause Detective",
        &[],
    ),
    (
        "slow joins after the table grew, what indexes am I missing",
        SQL,
        &[],
    ),
];

/// Everyday goals no Spark serves, for the final test.
pub const TEST_UNRELATED: &[&str] = &[
    "suggest a name for my new puppy",
    "how long should I boil an egg",
    "write a limerick about a cat",
    "find cheap flights to Tokyo in March",
];

/// Everyday goals no Spark serves (none of them is a calibration goal).
pub const HELDOUT_UNRELATED: &[&str] = &[
    "what stretches help with lower back pain",
    "plan a kid's birthday party on a budget",
    "convert 5 miles to kilometers",
    "write a haiku about autumn leaves",
    "tips for repotting a fiddle leaf fig",
    "how do I get red wine out of a carpet",
];

/// Goals for the user-added Sparks, by their index in
/// fixtures/user_added_sparks.json, phrased unlike the Sparks' own wording.
pub const PREMIUM: &[(&str, usize, &[&str])] = &[
    ("help me decide whether we should expand into Europe next year and write it up for the board", 0, &[]),
    ("I have to recommend one of three options to leadership, lay out the tradeoffs", 0, &[]),
    ("our javascript codebase has no types, plan a gradual conversion", 1, &[]),
    ("turn on strict type checking without breaking the build", 1, &[]),
    ("who else sells something like my product and how do they price it", 2, &["Deep Research Framework"]),
    ("size up the competition before we pitch investors", 2, &["Deep Research Framework"]),
    (
        "set up an assistant that sorts incoming customer tickets by urgency and drafts replies",
        3,
        &["AI Agent System Designer"],
    ),
    ("automate first-line responses to our help desk", 3, &["AI Agent System Designer"]),
    ("turn my rough thoughts into a post for my professional network", 4, &[]),
    ("I want to build my personal brand with weekly posts", 4, &[]),
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

#[derive(serde::Deserialize)]
pub struct UserFixture {
    pub sparks: Vec<UserSpark>,
}

#[derive(serde::Deserialize, Clone)]
pub struct UserSpark {
    pub title: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub body: String,
}

pub fn user_sparks() -> Vec<UserSpark> {
    let raw = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/user_added_sparks.json"),
    )
    .expect("user fixture");
    serde_json::from_str::<UserFixture>(&raw)
        .expect("user fixture json")
        .sparks
}

/// The acceptance library plus the user-added Sparks, saved through the
/// normal Add New Spark path. Returns their ids in fixture order.
pub fn user_library(dir: &std::path::Path) -> (Library, Vec<i64>) {
    let mut lib = acceptance_library(dir);
    let ids = user_sparks()
        .into_iter()
        .map(|s| {
            sparks::create(
                &mut lib,
                SparkInput {
                    title: s.title,
                    summary: s.summary,
                    body: s.body,
                    tags: s.tags,
                    favorite: s.favorite,
                    source_note: None,
                    allow_duplicate: false,
                },
            )
            .expect("create user spark")
            .id
        })
        .collect();
    (lib, ids)
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
        .chain(HELDOUT.iter().map(|(q, _, _)| q.to_string()))
        .chain(HELDOUT_UNRELATED.iter().map(|q| q.to_string()))
        .chain(PREMIUM.iter().map(|(q, _, _)| q.to_string()))
        .chain(TEST.iter().map(|(q, _, _)| q.to_string()))
        .chain(TEST_UNRELATED.iter().map(|q| q.to_string()))
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
    pub heldout: Vec<(String, String, Grade)>,
    pub test: Vec<(String, String, Grade)>,
    /// User-added Spark goals, in the acceptance library plus those Sparks.
    pub premium: Vec<(String, String, Grade)>,
    /// The 12 acceptance goals again, with the user-added Sparks present.
    pub acceptance_with_user: Vec<(String, String, Grade)>,
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
        for (label, list) in [
            ("dev", &self.dev),
            ("held-out", &self.heldout),
            ("test", &self.test),
            ("user-added", &self.premium),
            ("acceptance+user", &self.acceptance_with_user),
        ] {
            for (q, leader, g) in list.iter().filter(|(_, _, g)| *g != Grade::Correct) {
                println!("  [{label}] {g:<16?} {q}\n                   -> {leader}");
            }
        }
        for (label, list) in [
            ("acceptance", &self.acceptance),
            ("dev", &self.dev),
            ("held-out", &self.heldout),
            ("test", &self.test),
            ("user-added", &self.premium),
            ("acceptance+user", &self.acceptance_with_user),
        ] {
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
    for (q, expected, alts) in HELDOUT {
        let o = run(q);
        report
            .heldout
            .push((q.to_string(), leader(&o), grade(&o, expected, alts)));
    }
    for (q, expected, alts) in TEST {
        let o = run(q);
        report
            .test
            .push((q.to_string(), leader(&o), grade(&o, expected, alts)));
    }
    for q in UNRELATED
        .iter()
        .chain(HELDOUT_UNRELATED)
        .chain(TEST_UNRELATED)
    {
        if run(q).confidence == Confidence::Strong {
            report.false_confident.push(q.to_string());
        }
    }
    report
}

/// Adds the user-added Spark goals (and the acceptance goals again) run
/// against the acceptance library plus the user-added Sparks.
pub fn evaluate_user(
    report: &mut Report,
    lib: &Library,
    ids: &[i64],
    index: &VectorIndex,
    model: &str,
    embed: &dyn Fn(&str) -> Vec<f32>,
) {
    let run = |q: &str| {
        let v = embed(&models::query_text(model, q));
        let sem = SemanticScores::from_index(index, &v).expect("index ready");
        search::search(lib, q, Some(&sem), None, false, 0).unwrap()
    };
    let titles = sparks::search_docs(lib, ids).unwrap();
    let title_of = |id: i64| titles.iter().find(|d| d.id == id).unwrap().title.clone();
    for (q, i, alts) in PREMIUM {
        let o = run(q);
        let expected = title_of(ids[*i]);
        report
            .premium
            .push((q.to_string(), leader(&o), grade(&o, &expected, alts)));
    }
    for (q, expected, alts) in ACCEPTANCE {
        let o = run(q);
        report
            .acceptance_with_user
            .push((q.to_string(), leader(&o), grade(&o, expected, alts)));
    }
    for q in UNRELATED.iter().chain(HELDOUT_UNRELATED) {
        if run(q).confidence == Confidence::Strong {
            report
                .false_confident
                .push(format!("{q} (with user-added Sparks)"));
        }
    }
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

/// Every text embedded for several libraries (no duplicates, stable order).
pub fn union_texts(all: &[&Texts], model: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    all.iter()
        .flat_map(|t| every_text(t, model))
        .filter(|t| seen.insert(t.clone()))
        .collect()
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

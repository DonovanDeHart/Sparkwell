//! Calibration sweep for the canonical embedding model (opt-in; needs Ollama
//! the first time; vectors are cached under target/semantic-cache/).
//!
//!   cargo test --test semantic_tune -- --ignored --nocapture
//!
//! Sweeps `search::Calibration` over the ACCEPTANCE and DEV goals only (plus
//! the unrelated goals, which must never be confident). HELDOUT and the
//! user-added goals are deliberately not used here: they are the honest
//! out-of-sample check in `semantic_live` / `semantic_regression`.

mod support;

use std::collections::HashMap;
use std::time::Duration;

use sparkwell_lib::ai::models::{self, pick_embedding_model};
use sparkwell_lib::ai::ollama::OllamaClient;
use sparkwell_lib::search::vectors::VectorIndex;
use sparkwell_lib::search::{ranked_candidates, Calibration, SemanticScores, CALIBRATION};
use sparkwell_lib::sparks;
use support::*;

async fn cached_vectors(model: &str, texts: &[String]) -> HashMap<String, Vec<f32>> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/semantic-cache");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}.json", model.replace([':', '/'], "_")));
    let mut cache: HashMap<String, String> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let missing: Vec<String> = texts
        .iter()
        .filter(|t| !cache.contains_key(&text_key(t)))
        .cloned()
        .collect();
    if !missing.is_empty() {
        let client = OllamaClient::default();
        for chunk in missing.chunks(16) {
            let v = client
                .embed(model, chunk, Duration::from_secs(300))
                .await
                .expect("embedding request");
            for (t, v) in chunk.iter().zip(v) {
                cache.insert(text_key(t), encode_vector(&v));
            }
        }
        std::fs::write(&path, serde_json::to_string(&cache).unwrap()).unwrap();
        println!("embedded {} new texts", missing.len());
    }
    texts
        .iter()
        .map(|t| (t.clone(), decode_vector(&cache[&text_key(t)])))
        .collect()
}

/// (set: 0 acceptance, 1 dev, 2 validation; goal; expected and alternatives)
type Goal<'a> = (u8, &'a str, Option<(String, Vec<String>)>);

/// What the judge needs for one goal: ranked (title, base) and the purpose
/// similarity of the top two.
struct Case {
    expected: Option<(String, Vec<String>)>,
    ranked: Vec<(String, f64)>,
    purpose12: f32,
}

#[derive(Default, Clone, Copy)]
struct Tally {
    correct_acc: usize,
    correct_dev: usize,
    correct_val: usize,
    acceptable: usize,
    missed: usize,
    wrong: usize,
}

fn judge(cases: &[(u8, Case)], strong: f64, margin: f64, same: f32) -> Tally {
    let mut t = Tally::default();
    for (set, c) in cases {
        let (top, top_base) = &c.ranked[0];
        let second = c.ranked.get(1).map(|r| r.1).unwrap_or(0.0);
        let confident = *top_base >= strong && (top_base - second >= margin || c.purpose12 >= same);
        match &c.expected {
            None => {
                if confident {
                    t.wrong += 1;
                }
            }
            Some((exp, alts)) => {
                let ok = |x: &String| x == exp || alts.contains(x);
                if confident {
                    if top == exp {
                        match set {
                            0 => t.correct_acc += 1,
                            1 => t.correct_dev += 1,
                            _ => t.correct_val += 1,
                        }
                    } else if ok(top) {
                        t.acceptable += 1;
                    } else {
                        t.wrong += 1;
                    }
                } else if c
                    .ranked
                    .iter()
                    .filter(|r| r.1 >= CALIBRATION.candidate_min)
                    .take(3)
                    .any(|r| ok(&r.0))
                {
                    t.acceptable += 1;
                } else {
                    t.missed += 1;
                }
            }
        }
    }
    t
}

#[tokio::test]
#[ignore]
async fn sweep_calibration() {
    let models = OllamaClient::default().list_models().await.expect("Ollama");
    let model = pick_embedding_model(
        &models,
        std::env::var("SPARKWELL_EMBED_MODEL").ok().as_deref(),
    )
    .expect("canonical model installed");
    let dir = tempfile::tempdir().unwrap();
    let lib = acceptance_library(dir.path());
    let texts = document_texts(&lib, &model);
    let vectors = cached_vectors(&model, &every_text(&texts, &model)).await;
    let embed = |t: &str| vectors[t].clone();
    let titles: HashMap<i64, String> = sparks::search_docs(&lib, &sparks::all_ids(&lib).unwrap())
        .unwrap()
        .into_iter()
        .map(|d| (d.id, d.title))
        .collect();

    let exp =
        |e: &str, a: &[&str]| Some((e.to_string(), a.iter().map(|s| s.to_string()).collect()));
    let goals: Vec<Goal> = ACCEPTANCE
        .iter()
        .map(|(q, e, a)| (0u8, *q, exp(e, a)))
        .chain(DEV.iter().map(|(q, e, a)| (1u8, *q, exp(e, a))))
        .chain(HELDOUT.iter().map(|(q, e, a)| (2u8, *q, exp(e, a))))
        .chain(
            UNRELATED
                .iter()
                .chain(HELDOUT_UNRELATED)
                .map(|q| (1u8, *q, None)),
        )
        .collect();

    let mut rows = Vec::new();
    for sem_weight in [0.6, 0.7, 0.8] {
        for profile_weight in [1.0f32, 1.05] {
            for best_view in [0.75f32] {
                for hub_neighbours in [5usize, 10, 20, 40] {
                    for scale_spreads in [6.0f32, 8.0, 10.0, 13.0] {
                        let cal = Calibration {
                            sem_weight,
                            lex_weight: 1.0 - sem_weight,
                            profile_weight,
                            best_view,
                            hub_neighbours,
                            scale_spreads,
                            scale_max: 1.0,
                            ..CALIBRATION
                        };
                        let mut index = VectorIndex::with_calibration(Some(model.clone()), cal);
                        for (id, views) in &texts.docs {
                            index.insert(*id, views.iter().map(|t| embed(t)).collect());
                        }
                        index.set_pool(texts.pool.iter().map(|t| embed(t)).collect());
                        let scale = index.scale();
                        let cases: Vec<(u8, Case)> = goals
                            .iter()
                            .map(|(set, q, exp)| {
                                let v = embed(&models::query_text(&model, q));
                                let sem = SemanticScores::from_index(&index, &v).unwrap();
                                let ranked = ranked_candidates(&lib, q, Some(&sem)).unwrap();
                                let purpose12 = match (ranked.first(), ranked.get(1)) {
                                    (Some(a), Some(b)) => sem.purpose(a.id, b.id).unwrap_or(0.0),
                                    _ => 0.0,
                                };
                                (
                                    *set,
                                    Case {
                                        expected: exp.clone(),
                                        ranked: ranked
                                            .iter()
                                            .map(|s| (titles[&s.id].clone(), s.base))
                                            .collect(),
                                        purpose12,
                                    },
                                )
                            })
                            .collect();
                        if let Ok(g) = std::env::var("TUNE_GRID") {
                            let f: Vec<f64> = g.split(',').map(|x| x.parse().unwrap()).collect();
                            let this = [
                                sem_weight,
                                profile_weight as f64,
                                best_view as f64,
                                hub_neighbours as f64,
                                scale_spreads as f64,
                            ];
                            if f.iter().zip(this).all(|(a, b)| (a - b).abs() < 1e-6) {
                                println!("grid for {g} (scale {scale:.3}): acc/dev/wrong  rows strong 0.20..0.50, cols margin 0.04..0.20");
                                for si in 0..16 {
                                    let strong = 0.20 + si as f64 * 0.02;
                                    let cells: Vec<String> = (0..9)
                                        .map(|mi| {
                                            let t =
                                                judge(&cases, strong, 0.04 + mi as f64 * 0.02, 0.9);
                                            format!(
                                                "{:2}/{:2}/{:2}/{}",
                                                t.correct_acc,
                                                t.correct_dev,
                                                t.correct_val,
                                                t.wrong
                                            )
                                        })
                                        .collect();
                                    println!("grid {strong:.2} | {}", cells.join("  "));
                                }
                            }
                        }
                        for same in [0.9f32, 2.0] {
                            // Plateau: how many (strong, margin) cells have no wrong answer.
                            let mut grid = Vec::new();
                            for si in 0..16 {
                                for mi in 0..9 {
                                    let strong = 0.20 + si as f64 * 0.02;
                                    let margin = 0.04 + mi as f64 * 0.02;
                                    grid.push((
                                        strong,
                                        margin,
                                        judge(&cases, strong, margin, same),
                                    ));
                                }
                            }
                            let safe = grid.iter().filter(|g| g.2.wrong == 0).count();
                            for (strong, margin, t) in grid.iter().filter(|g| g.2.wrong == 0) {
                                // Neighbours (±1 step each way) must be safe too.
                                let robust = grid
                                    .iter()
                                    .filter(|g| {
                                        (g.0 - strong).abs() < 0.021 && (g.1 - margin).abs() < 0.021
                                    })
                                    .all(|g| g.2.wrong == 0);
                                if robust || std::env::var("TUNE_ANY").is_ok() {
                                    rows.push((cal, scale, same, *strong, *margin, *t, safe));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    rows.sort_by(|a, b| {
        let key = |r: &(Calibration, f32, f32, f64, f64, Tally, usize)| {
            (
                r.5.correct_acc + r.5.correct_dev + r.5.correct_val,
                usize::MAX - r.5.missed,
                r.5.correct_acc,
                r.6,
            )
        };
        key(b).cmp(&key(a))
    });
    println!("zero-wrong settings (acc correct /12, dev /36, validation /23, acceptable, missed, safe cells):");
    for (cal, scale, same, strong, margin, t, safe) in rows.iter().take(40) {
        println!(
            "acc {:2} dev {:2} val {:2} acpt {:2} miss {} safe {:3} | sem {:.2} pw {:.2} bv {:.2} hub {:2} ss {:.1} (scale {:.3}) same {:.2} strong {:.2} margin {:.2}",
            t.correct_acc, t.correct_dev, t.correct_val, t.acceptable, t.missed, safe,
            cal.sem_weight, cal.profile_weight, cal.best_view, cal.hub_neighbours, cal.scale_spreads, scale, same, strong, margin
        );
    }
}

/// Prints, for every tuning goal, the top candidates with their fused,
/// semantic and lexical scores under the current CALIBRATION.
#[tokio::test]
#[ignore]
async fn diagnose() {
    use sparkwell_lib::search::{lexical_score, term_weights, text::query_terms};
    let models = OllamaClient::default().list_models().await.expect("Ollama");
    let model = pick_embedding_model(
        &models,
        std::env::var("SPARKWELL_EMBED_MODEL").ok().as_deref(),
    )
    .expect("canonical model installed");
    let dir = tempfile::tempdir().unwrap();
    let lib = acceptance_library(dir.path());
    let texts = document_texts(&lib, &model);
    let vectors = cached_vectors(&model, &every_text(&texts, &model)).await;
    let embed = |t: &str| vectors[t].clone();
    // TUNE_CAL="sem_weight,scale_spreads,hub_neighbours" overrides CALIBRATION.
    let cal = std::env::var("TUNE_CAL")
        .ok()
        .map(|v| {
            let f: Vec<f64> = v.split(',').map(|x| x.trim().parse().unwrap()).collect();
            Calibration {
                sem_weight: f[0],
                lex_weight: 1.0 - f[0],
                scale_spreads: f[1] as f32,
                hub_neighbours: f[2] as usize,
                profile_weight: f.get(3).copied().unwrap_or(1.05) as f32,
                scale_max: 1.0,
                ..CALIBRATION
            }
        })
        .unwrap_or(CALIBRATION);
    let mut index = VectorIndex::with_calibration(Some(model.clone()), cal);
    for (id, views) in &texts.docs {
        index.insert(*id, views.iter().map(|t| embed(t)).collect());
    }
    index.set_pool(texts.pool.iter().map(|t| embed(t)).collect());
    println!("scale {:.3}", index.scale());
    let docs = sparks::search_docs(&lib, &sparks::all_ids(&lib).unwrap()).unwrap();
    let only = std::env::var("TUNE_ONLY").ok();
    let goals = ACCEPTANCE
        .iter()
        .chain(DEV)
        .chain(
            HELDOUT
                .iter()
                .filter(|_| std::env::var("TUNE_HELDOUT").is_ok()),
        )
        .map(|(q, e, _)| (*q, *e))
        .chain(UNRELATED.iter().map(|q| (*q, "-")));
    for (q, expected) in goals {
        if only.as_deref().is_some_and(|o| !q.contains(o)) {
            continue;
        }
        let v = embed(&models::query_text(&model, q));
        let raw: HashMap<i64, f64> = index
            .evidence(&v)
            .into_iter()
            .map(|e| (e.id, e.score))
            .collect();
        let terms = query_terms(q);
        let weights = term_weights(&lib, &terms).unwrap();
        let sem = SemanticScores::from_index(&index, &v).unwrap();
        let ranked = ranked_candidates(&lib, q, Some(&sem)).unwrap();
        println!("\n{q}  [expected: {expected}]");
        for s in ranked.iter().take(4) {
            let d = docs.iter().find(|d| d.id == s.id).unwrap();
            println!(
                "   {:.3}  sem {:.3}  lex {:.3}  {}",
                s.base,
                raw.get(&s.id).copied().unwrap_or(-1.0),
                lexical_score(&terms, d, Some(&weights)),
                d.title.chars().take(48).collect::<String>()
            );
        }
        if let Some(pos) = ranked
            .iter()
            .position(|s| docs.iter().any(|d| d.id == s.id && d.title == expected))
        {
            if pos >= 4 {
                println!("   expected at rank {}", pos + 1);
            }
        }
    }
}

/// Writes the exact retrieval texts (views per Spark) and goals to
/// target/semantic-cache/texts.json for offline experiments.
#[test]
#[ignore]
fn dump_texts() {
    let dir = tempfile::tempdir().unwrap();
    let lib = acceptance_library(dir.path());
    let docs = sparks::search_docs(&lib, &sparks::all_ids(&lib).unwrap()).unwrap();
    let sparks_json: Vec<serde_json::Value> = docs
        .iter()
        .map(|d| {
            serde_json::json!({
                "title": d.title, "summary": d.summary, "tags": d.tags, "body": d.body,
                "views": sparkwell_lib::ai::spark_views(d),
            })
        })
        .collect();
    let goals = |list: &[(&str, &str, &[&str])]| -> Vec<serde_json::Value> {
        list.iter()
            .map(|(q, e, a)| serde_json::json!({"q": q, "e": e, "a": a}))
            .collect()
    };
    let out = serde_json::json!({
        "sparks": sparks_json,
        "acceptance": goals(ACCEPTANCE), "dev": goals(DEV), "heldout": goals(HELDOUT),
        "unrelated": UNRELATED, "pool": sparkwell_lib::search::pool::GENERIC_GOALS,
    });
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/semantic-cache/texts.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, serde_json::to_string_pretty(&out).unwrap()).unwrap();
}

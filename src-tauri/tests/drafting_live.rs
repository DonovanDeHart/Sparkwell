//! Opt-in: Auto-fill against a running Ollama with the local chat model
//! Sparkwell would choose. Drafts details for varied Sparks (video, coding,
//! AI agent, research, writing, long structured), saves each with its drafted
//! details and checks the stored and copied body is the original, byte for
//! byte. Reports cold and warm timings.
//!
//! cargo test --test drafting_live -- --ignored --nocapture

mod support;

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sparkwell_lib::ai::ollama::{OllamaClient, OLLAMA_BASE_URL};
use sparkwell_lib::ai::{metadata, models};
use sparkwell_lib::sparks::{self, SparkInput};
use sparkwell_lib::storage::{Library, OpenMode};
use support::*;

const VIDEO_SPARK: &str = "A cinematic, realistic, and seamlessly loopable video of Coulterville, California in 1899, during the twilight of the Gold Rush era. The camera slowly glides down Main Street at golden hour, past the Hotel Jeffery's weathered balcony, the Wells Fargo office and the stone Sun Sun Wo store. Dust hangs in warm, low-angle sunlight; a horse-drawn freight wagon creaks by, a dog trots across the street, and a shopkeeper sweeps the wooden boardwalk.\r\n\r\nPeriod-accurate details: kerosene lamps being lit, hand-painted signage, miners in canvas trousers and suspenders, women in high-collared dresses, hitching posts and water troughs.\r\nAtmosphere: nostalgic, quiet, lived-in, with distant hammering from the blacksmith and wind moving through oak trees on the surrounding Sierra foothills.\r\nLighting: warm amber key light, long shadows, soft haze, subtle film grain, 35mm anamorphic look, shallow depth of field.\r\nMotion: slow, steady dolly movement; natural ambient motion in flags, smoke from chimneys and swaying lanterns. The final frame must match the first frame exactly so the clip loops seamlessly.\r\n\r\n8 seconds, 24 fps, 16:9, no text, no modern objects, no people looking at the camera.  ";

async fn get(path: &str) -> Value {
    reqwest::Client::new()
        .get(format!("{OLLAMA_BASE_URL}{path}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

async fn resident() -> Vec<String> {
    get("/api/ps").await["models"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|m| {
            format!(
                "{} ({:.1} GB VRAM, context {})",
                m["name"].as_str().unwrap_or("?"),
                m["size_vram"].as_f64().unwrap_or(0.0) / 1e9,
                m["context_length"]
            )
        })
        .collect()
}

async fn unload(model: &str) {
    reqwest::Client::new()
        .post(format!("{OLLAMA_BASE_URL}/api/generate"))
        .json(&json!({ "model": model, "keep_alive": 0 }))
        .send()
        .await
        .unwrap();
    for _ in 0..50 {
        if !resident().await.iter().any(|r| r.starts_with(model)) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "needs a running Ollama with a local chat model"]
async fn auto_fill_drafts_useful_details_and_never_touches_the_spark() {
    let client = OllamaClient::default();
    let installed = client.list_models().await.expect("Ollama is running");
    let plan = models::plan_chat_models(
        &installed,
        std::env::var("SPARKWELL_CHAT_MODEL").ok().as_deref(),
    );
    let mut chosen = None;
    for candidate in &plan.candidates {
        let caps = client.capabilities(candidate).await.unwrap();
        if models::chat_capable(caps.as_deref()) {
            let thinking = caps.is_some_and(|c| c.iter().any(|x| x == "thinking"));
            chosen = Some((candidate.clone(), thinking));
            break;
        }
    }
    let (model, thinking) = chosen.expect("a local chat model within the Auto-fill limit");
    assert!(!models::is_cloud(
        installed.iter().find(|m| m.name == model).unwrap()
    ));
    println!("Auto-fill model: {model} (thinking model: {thinking})");

    let mut spark_bodies: Vec<(&str, String)> = vec![("image/video", VIDEO_SPARK.to_string())];
    let kinds = [
        "long structured",
        "coding",
        "research",
        "AI agent",
        "writing",
    ];
    for (kind, s) in kinds.iter().zip(user_sparks()) {
        spark_bodies.push((kind, s.body));
    }

    // Cold: the model is not loaded, as after a restart.
    unload(&model).await;
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::open(dir.path(), OpenMode::CreateIfMissing).unwrap();
    let mut warm_times = Vec::new();
    for (i, (kind, body)) in spark_bodies.iter().enumerate() {
        let started = Instant::now();
        let was_loaded = client.chat_model_loaded(&model).await.unwrap();
        if !was_loaded {
            client
                .load_chat_model(&model, Duration::from_secs(120))
                .await
                .expect("model loads");
        }
        let loaded_in = started.elapsed();
        let content = client
            .chat_json(
                &model,
                metadata::messages(body),
                metadata::schema(),
                thinking,
                Duration::from_secs(60),
            )
            .await
            .expect("drafts");
        let total = started.elapsed();
        let s = metadata::parse(&content).expect("usable details");
        if i == 0 {
            assert!(!was_loaded, "first draft starts cold");
            println!(
                "\ncold: {:.1}s total ({:.1}s loading, {:.1}s drafting)",
                total.as_secs_f64(),
                loaded_in.as_secs_f64(),
                (total - loaded_in).as_secs_f64()
            );
            println!("resident after loading: {:?}", resident().await);
        } else {
            assert!(was_loaded, "later drafts reuse the loaded model");
            warm_times.push(total.as_secs_f64());
        }
        println!(
            "\n[{kind}] {:.1}s\n  title:   {}\n  summary: {}\n  tags:    {}",
            total.as_secs_f64(),
            s.title,
            s.summary,
            s.tags.join(" · ")
        );
        let words = s.title.split_whitespace().count();
        assert!((2..=8).contains(&words), "functional title: {}", s.title);
        assert!(!s.summary.is_empty() && s.summary.chars().count() <= 241);
        assert!((3..=6).contains(&s.tags.len()), "3-6 tags: {:?}", s.tags);

        // Saved with its drafted details, the Spark itself is unchanged.
        let saved = sparks::create(
            &mut lib,
            SparkInput {
                title: s.title,
                summary: s.summary,
                body: body.clone(),
                tags: s.tags,
                ..Default::default()
            },
        )
        .unwrap();
        let stored = sparks::get_detail(&lib, saved.id).unwrap();
        assert_eq!(stored.body.as_bytes(), body.as_bytes());
        let (_, copied) = sparks::record_copy(&mut lib, saved.id).unwrap();
        assert_eq!(copied.as_bytes(), body.as_bytes());
    }
    warm_times.sort_by(f64::total_cmp);
    println!(
        "\nwarm: median {:.2}s, max {:.2}s over {} Sparks",
        warm_times[warm_times.len() / 2],
        warm_times.last().unwrap(),
        warm_times.len()
    );
    println!("resident at the end: {:?}", resident().await);
}

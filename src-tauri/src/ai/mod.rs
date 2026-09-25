//! Optional local intelligence (Ollama).
//!
//! Three levels, per the technical spec:
//! - ready: hybrid semantic + lexical retrieval, Smart Add metadata;
//! - offline: lexical retrieval, every library action still available;
//! - no search needed: favorites, add, copy, settings never touch AI.
//!
//! A background service probes Ollama (never delaying startup), keeps the
//! embedding index current, and reports status changes to the UI.

pub mod metadata;
pub mod models;
pub mod ollama;

use std::time::{Duration, Instant};

use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::error::{AppError, AppResult};
use crate::search::vectors::{encode, normalized, VectorIndex};
use crate::state::AppState;
use crate::storage::{content_hash, now_ms, Library};
use metadata::MetadataSuggestion;
use ollama::OllamaError;

pub const EVENT_AI_STATUS: &str = "sparkwell://ai-status";

const BATCH_SIZE: usize = 8;
const QUERY_TIMEOUT: Duration = Duration::from_millis(3000);
const FIRST_BATCH_TIMEOUT: Duration = Duration::from_secs(120);
const BATCH_TIMEOUT: Duration = Duration::from_secs(60);
const METADATA_TIMEOUT: Duration = Duration::from_secs(60);
const EMBED_BODY_CHARS: usize = 3_000;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum AiState {
    /// First probe has not completed yet.
    #[default]
    Checking,
    /// Ollama is reachable.
    Online,
    /// Ollama is not running / not installed.
    Offline,
}

#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AiStatus {
    pub state: AiState,
    pub embed_model: Option<String>,
    pub chat_model: Option<String>,
    /// Sparks with an embedding for `embed_model`.
    pub indexed: usize,
    pub total: usize,
    pub indexing: bool,
}

impl AiStatus {
    pub fn semantic_ready(&self) -> bool {
        self.state == AiState::Online && self.embed_model.is_some()
    }
    pub fn smart_add_ready(&self) -> bool {
        self.state == AiState::Online && self.chat_model.is_some()
    }
}

fn set_status<R: Runtime>(app: &AppHandle<R>, change: impl FnOnce(&mut AiStatus)) {
    let state = app.state::<AppState>();
    let snapshot = {
        let Ok(mut status) = state.ai.lock() else {
            return;
        };
        let before = status.clone();
        change(&mut status);
        if *status == before {
            return;
        }
        status.clone()
    };
    let _ = app.emit(EVENT_AI_STATUS, snapshot);
}

fn env_override(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Starts the background service. Returns immediately.
pub fn spawn_service<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            refresh(&app).await;
            if app.state::<AppState>().ai_status().semantic_ready() {
                index_pending(&app).await;
            }
            let online = app.state::<AppState>().ai_status().state == AiState::Online;
            let delay = if online {
                Duration::from_secs(60)
            } else {
                Duration::from_secs(15)
            };
            let state = app.state::<AppState>();
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = state.ai_wake.notified() => {}
            }
        }
    });
}

/// Asks the service to re-probe and re-index soon (e.g. after edits or when
/// the panel is shown, so a newly started Ollama is noticed quickly).
pub fn wake<R: Runtime>(app: &AppHandle<R>) {
    app.state::<AppState>().ai_wake.notify_one();
}

/// Probes Ollama and updates the status, reloading the vector index when the
/// embedding model changes.
pub async fn refresh<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState>();
    let probe = state.ollama.list_models().await;
    let total = state
        .with_library(|lib| lib.spark_count())
        .unwrap_or(0)
        .max(0) as usize;

    let (online, embed_model, chat_model) = match probe {
        Ok(models) => (
            true,
            models::pick_embedding_model(&models, env_override("SPARKWELL_EMBED_MODEL").as_deref()),
            models::pick_chat_model(&models, env_override("SPARKWELL_CHAT_MODEL").as_deref()),
        ),
        Err(_) => (false, None, None),
    };

    // Keep vectors for the current model even while offline so a brief outage
    // doesn't require reloading; they're simply unused until Ollama returns.
    if let Some(model) = &embed_model {
        let needs_reload = state
            .vectors
            .read()
            .map(|v| v.model.as_deref() != Some(model.as_str()))
            .unwrap_or(true);
        if needs_reload {
            reload_vectors(app, model);
        }
    }
    let indexed = state.vectors.read().map(|v| v.len()).unwrap_or(0);

    set_status(app, |s| {
        s.state = if online {
            AiState::Online
        } else {
            AiState::Offline
        };
        if online {
            s.embed_model = embed_model;
            s.chat_model = chat_model;
        }
        s.total = total;
        s.indexed = indexed.min(total);
        if !online {
            s.indexing = false;
        }
    });
}

/// Loads stored vectors for `model` from the current library.
pub fn reload_vectors<R: Runtime>(app: &AppHandle<R>, model: &str) {
    let state = app.state::<AppState>();
    let index = state
        .with_library(|lib| VectorIndex::load(lib, model))
        .unwrap_or_else(|_| VectorIndex::empty(Some(model.to_string())));
    if let Ok(mut v) = state.vectors.write() {
        *v = index;
    };
}

/// Text that represents a Spark for embedding.
pub fn embed_text(title: &str, summary: &str, tags: &[String], body: &str) -> String {
    let head: String = body.chars().take(EMBED_BODY_CHARS).collect();
    format!("{title}\n{summary}\nTags: {}\n\n{head}", tags.join(", "))
}

struct Pending {
    id: i64,
    text: String,
    hash: String,
}

fn load_pending(lib: &Library, model: &str) -> AppResult<Vec<Pending>> {
    let ids: Vec<i64> = {
        let mut stmt = lib.conn.prepare(
            "SELECT id FROM sparks WHERE id NOT IN (SELECT spark_id FROM embeddings WHERE model = ?1)
             ORDER BY favorite DESC, usage_count DESC, id LIMIT 5000",
        )?;
        let rows = stmt.query_map([model], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let docs = crate::sparks::search_docs(lib, &ids)?;
    Ok(docs
        .into_iter()
        .map(|d| {
            let text = embed_text(&d.title, &d.summary, &d.tags, &d.body);
            Pending {
                id: d.id,
                hash: content_hash(&[model, &text]),
                text,
            }
        })
        .collect())
}

/// Stores vectors, skipping Sparks that were edited or deleted meanwhile.
fn store_vectors(
    lib: &mut Library,
    model: &str,
    batch: &[Pending],
    vectors: Vec<Vec<f32>>,
) -> AppResult<Vec<(i64, Vec<f32>)>> {
    let ids: Vec<i64> = batch.iter().map(|p| p.id).collect();
    let current: std::collections::HashMap<i64, String> = crate::sparks::search_docs(lib, &ids)?
        .into_iter()
        .map(|d| {
            (
                d.id,
                content_hash(&[model, &embed_text(&d.title, &d.summary, &d.tags, &d.body)]),
            )
        })
        .collect();
    let tx = lib.conn.transaction()?;
    let mut stored = Vec::new();
    for (pending, vector) in batch.iter().zip(vectors) {
        if current.get(&pending.id) != Some(&pending.hash) {
            continue;
        }
        let v = normalized(vector);
        tx.execute(
            "INSERT OR REPLACE INTO embeddings (spark_id, model, dimensions, vector, content_hash, generated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![pending.id, model, v.len() as i64, encode(&v), pending.hash, now_ms()],
        )?;
        stored.push((pending.id, v));
    }
    tx.commit()?;
    Ok(stored)
}

/// Embeds every Spark that lacks a vector for the current model.
async fn index_pending<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState>();
    let Some(model) = state.ai_status().embed_model else {
        return;
    };
    let generation = state.generation();
    let pending = match state.with_library(|lib| load_pending(lib, &model)) {
        Ok(p) if !p.is_empty() => p,
        _ => return,
    };
    let prefix = models::document_prefix(&model);
    set_status(app, |s| s.indexing = true);

    for (i, batch) in pending.chunks(BATCH_SIZE).enumerate() {
        let inputs: Vec<String> = batch
            .iter()
            .map(|p| format!("{prefix}{}", p.text))
            .collect();
        let timeout = if i == 0 {
            FIRST_BATCH_TIMEOUT
        } else {
            BATCH_TIMEOUT
        };
        match state.ollama.embed(&model, &inputs, timeout).await {
            Ok(vectors) => {
                if state.generation() != generation {
                    break; // library switched; this work belongs to the old one
                }
                match state.with_library(|lib| store_vectors(lib, &model, batch, vectors)) {
                    Ok(stored) => {
                        if let Ok(mut index) = state.vectors.write() {
                            if index.model.as_deref() == Some(model.as_str()) {
                                for (id, v) in stored {
                                    index.insert(id, v);
                                }
                            }
                        }
                        let indexed = state.vectors.read().map(|v| v.len()).unwrap_or(0);
                        set_status(app, |s| {
                            s.indexed = indexed;
                            s.total = s.total.max(indexed);
                        });
                    }
                    Err(e) => {
                        log::warn!("failed to store embeddings: {e}");
                        break;
                    }
                }
            }
            Err(OllamaError::Unreachable) => {
                set_status(app, |s| s.state = AiState::Offline);
                break;
            }
            Err(e) => {
                log::warn!("embedding batch failed: {e}");
                break;
            }
        }
    }
    set_status(app, |s| s.indexing = false);
}

/// Embeds a search query. `None` means "use standard search".
pub async fn embed_query<R: Runtime>(app: &AppHandle<R>, query: &str) -> Option<Vec<f32>> {
    let state = app.state::<AppState>();
    let status = state.ai_status();
    if !status.semantic_ready() {
        return None;
    }
    let model = status.embed_model?;
    let usable = state
        .vectors
        .read()
        .map(|v| v.model.as_deref() == Some(model.as_str()) && !v.is_empty())
        .unwrap_or(false);
    if !usable {
        return None;
    }
    let input = vec![format!("{}{}", models::query_prefix(&model), query)];
    match state.ollama.embed(&model, &input, QUERY_TIMEOUT).await {
        Ok(mut v) => v.pop(),
        Err(e) => {
            log::info!("query embedding unavailable, using standard search: {e}");
            if e == OllamaError::Unreachable {
                set_status(app, |s| s.state = AiState::Offline);
            }
            wake(app);
            None
        }
    }
}

/// Proposes title/summary/tags for a Spark body. Never saves anything.
pub async fn suggest_metadata<R: Runtime>(
    app: &AppHandle<R>,
    body: &str,
) -> AppResult<MetadataSuggestion> {
    let state = app.state::<AppState>();
    let status = state.ai_status();
    let model = match (status.smart_add_ready(), status.chat_model) {
        (true, Some(m)) => m,
        _ => {
            return Err(AppError::Ai(
                "Local intelligence is offline. Add the details yourself.".into(),
            ))
        }
    };
    if body.trim().is_empty() {
        return Err(AppError::Validation("Paste the Spark first.".into()));
    }
    let content = state
        .ollama
        .chat_json(
            &model,
            metadata::messages(body),
            metadata::schema(),
            METADATA_TIMEOUT,
        )
        .await
        .map_err(|e| {
            if e == OllamaError::Unreachable {
                set_status(app, |s| s.state = AiState::Offline);
            }
            AppError::Ai(format!(
                "Couldn't draft details ({e}). You can fill them in yourself."
            ))
        })?;
    metadata::parse(&content).ok_or_else(|| {
        AppError::Ai(
            "The local model returned something unusable. Fill in the details yourself.".into(),
        )
    })
}

/// Called when the panel is shown: preloads the embedding model so the first
/// search is fast, and nudges the service to notice a newly started Ollama.
pub fn on_panel_shown<R: Runtime>(app: &AppHandle<R>) {
    static LAST_WARMUP: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let state = app.state::<AppState>();
    let status = state.ai_status();
    if status.state != AiState::Online {
        wake(app);
        return;
    }
    let Some(model) = status.embed_model else {
        return;
    };
    let due = LAST_WARMUP
        .lock()
        .map(|mut last| {
            let due = last
                .map(|t| t.elapsed() > Duration::from_secs(120))
                .unwrap_or(true);
            if due {
                *last = Some(Instant::now());
            }
            due
        })
        .unwrap_or(false);
    if due {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let state = app.state::<AppState>();
            let _ = state
                .ollama
                .embed(&model, &["warm up".to_string()], FIRST_BATCH_TIMEOUT)
                .await;
        });
    }
}

/// Drops the in-memory vector for a changed/deleted Spark and schedules indexing.
pub fn on_spark_changed<R: Runtime>(app: &AppHandle<R>, id: i64) {
    let state = app.state::<AppState>();
    if let Ok(mut v) = state.vectors.write() {
        v.remove(id);
    }
    let total = state
        .with_library(|lib| lib.spark_count())
        .unwrap_or(0)
        .max(0) as usize;
    let indexed = state.vectors.read().map(|v| v.len()).unwrap_or(0);
    set_status(app, |s| {
        s.total = total;
        s.indexed = indexed.min(total);
    });
    wake(app);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sparks::{self, SparkInput};

    #[test]
    fn pending_and_store_skip_stale_content() {
        let mut lib = Library::open_in_memory();
        let a = sparks::create(
            &mut lib,
            SparkInput {
                title: "A".into(),
                body: "alpha".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let b = sparks::create(
            &mut lib,
            SparkInput {
                title: "B".into(),
                body: "beta".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let pending = load_pending(&lib, "m").unwrap();
        assert_eq!(pending.len(), 2);

        // B is edited after its text was captured for embedding.
        sparks::update(
            &mut lib,
            b.id,
            SparkInput {
                title: "B2".into(),
                body: "beta two".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let stored = store_vectors(
            &mut lib,
            "m",
            &pending,
            vec![vec![1.0, 0.0], vec![0.0, 1.0]],
        )
        .unwrap();
        let stored_ids: Vec<i64> = stored.iter().map(|(id, _)| *id).collect();
        assert_eq!(stored_ids, vec![a.id]);

        // B is still pending; A is not.
        let still: Vec<i64> = load_pending(&lib, "m")
            .unwrap()
            .iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(still, vec![b.id]);
        // Another model needs its own vectors.
        assert_eq!(load_pending(&lib, "other").unwrap().len(), 2);

        let index = VectorIndex::load(&lib, "m").unwrap();
        assert_eq!(index.len(), 1);
        assert!(index.contains(a.id));
    }

    #[test]
    fn deleted_spark_vectors_are_not_stored() {
        let mut lib = Library::open_in_memory();
        let a = sparks::create(
            &mut lib,
            SparkInput {
                title: "A".into(),
                body: "alpha".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let pending = load_pending(&lib, "m").unwrap();
        sparks::delete(&mut lib, a.id).unwrap();
        let stored = store_vectors(&mut lib, "m", &pending, vec![vec![1.0]]).unwrap();
        assert!(stored.is_empty());
    }

    #[test]
    fn status_capabilities() {
        let mut s = AiStatus::default();
        assert!(!s.semantic_ready());
        s.state = AiState::Online;
        assert!(!s.semantic_ready());
        s.embed_model = Some("e".into());
        assert!(s.semantic_ready());
        assert!(!s.smart_add_ready());
        s.chat_model = Some("c".into());
        assert!(s.smart_add_ready());
        s.state = AiState::Offline;
        assert!(!s.semantic_ready() && !s.smart_add_ready());
    }
}

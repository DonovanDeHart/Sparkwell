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

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use rusqlite::params;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::error::{AppError, AppResult};
use crate::search::pool::GENERIC_GOALS;
use crate::search::profile::{document_views, ProfileSource, PROFILE_VERSION};
use crate::search::vectors::{encode_views, normalized, VectorIndex};
use crate::search::Fallback;
use crate::sparks::{self, SearchDoc};
use crate::state::AppState;
use crate::storage::{content_hash, now_ms, Library};
use metadata::MetadataSuggestion;
use ollama::OllamaError;

pub const EVENT_AI_STATUS: &str = "sparkwell://ai-status";

/// Texts per `/api/embed` request while indexing.
const BATCH_TEXTS: usize = 16;
/// A query embedding from a model that is already loaded answers in well under
/// a second; this budget only absorbs a busy moment.
const QUERY_TIMEOUT_WARM: Duration = Duration::from_secs(5);
/// A cold model must first be loaded into memory. Waiting (with a visible
/// "waking up" state) gives a consistent semantic result instead of silently
/// switching to standard search.
const QUERY_TIMEOUT_COLD: Duration = Duration::from_secs(25);
/// Ollama keeps the embedding model loaded for 30 minutes after use.
const WARM_WINDOW: Duration = Duration::from_secs(25 * 60);
const POOL_TIMEOUT: Duration = Duration::from_secs(90);
const FIRST_BATCH_TIMEOUT: Duration = Duration::from_secs(120);
const BATCH_TIMEOUT: Duration = Duration::from_secs(60);
const METADATA_TIMEOUT: Duration = Duration::from_secs(60);

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
    /// Small local chat model used by Smart Add (None: Auto-fill unavailable).
    pub chat_model: Option<String>,
    /// Local chat models exist but are all too large for Smart Add.
    pub chat_models_too_large: bool,
    /// Sparks with current vectors for `embed_model`.
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

fn mark_warm(state: &AppState) {
    if let Ok(mut w) = state.embed_warm_until.lock() {
        *w = Some(Instant::now() + WARM_WINDOW);
    }
}

fn is_warm(state: &AppState) -> bool {
    state
        .embed_warm_until
        .lock()
        .map(|w| w.is_some_and(|t| Instant::now() < t))
        .unwrap_or(false)
}

/// Starts the background service. Returns immediately.
pub fn spawn_service<R: Runtime>(app: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        loop {
            refresh(&app).await;
            let status = app.state::<AppState>().ai_status();
            if status.semantic_ready() {
                if let Some(model) = status.embed_model {
                    if ensure_pool(&app, &model).await {
                        index_pending(&app).await;
                    }
                }
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

    let (online, embed_model, chat) = match probe {
        Ok(models) => (
            true,
            models::pick_embedding_model(&models, env_override("SPARKWELL_EMBED_MODEL").as_deref()),
            models::pick_chat_model(&models, env_override("SPARKWELL_CHAT_MODEL").as_deref()),
        ),
        Err(_) => (false, None, models::ChatChoice::default()),
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
            s.chat_models_too_large = chat.model.is_none() && chat.skipped_large;
            s.chat_model = chat.model;
        }
        s.total = total;
        s.indexed = indexed.min(total);
        if !online {
            s.indexing = false;
        }
    });
}

/// The views embedded for a Spark (see `search::profile`).
pub fn spark_views(doc: &SearchDoc) -> Vec<String> {
    document_views(&ProfileSource {
        title: &doc.title,
        summary: &doc.summary,
        tags: &doc.tags,
        body: &doc.body,
    })
}

/// Identifies exactly what was embedded for a Spark with `model`.
pub fn views_hash(model: &str, views: &[String]) -> String {
    content_hash(&[PROFILE_VERSION, model, &views.join("\u{1f}")])
}

fn docs_in_batches(lib: &Library, ids: &[i64], mut f: impl FnMut(SearchDoc)) -> AppResult<()> {
    for chunk in ids.chunks(200) {
        for d in sparks::search_docs(lib, chunk)? {
            f(d);
        }
    }
    Ok(())
}

fn expected_hashes(lib: &Library, model: &str) -> AppResult<HashMap<i64, String>> {
    let mut out = HashMap::new();
    docs_in_batches(lib, &sparks::all_ids(lib)?, |d| {
        out.insert(d.id, views_hash(model, &spark_views(&d)));
    })?;
    Ok(out)
}

/// Loads stored vectors for `model` from the current library. Vectors from an
/// older profile recipe or edited Sparks are left out and re-embedded.
pub fn reload_vectors<R: Runtime>(app: &AppHandle<R>, model: &str) {
    let state = app.state::<AppState>();
    let mut index = state
        .with_library(|lib| {
            let expected = expected_hashes(lib, model)?;
            VectorIndex::load(lib, model, &expected)
        })
        .unwrap_or_else(|_| VectorIndex::empty(Some(model.to_string())));
    if let Ok(pool) = state.pool.lock() {
        if let Some((pool_model, vectors)) = pool.as_ref() {
            if pool_model == model {
                index.set_pool(vectors.clone());
            }
        }
    }
    if let Ok(mut v) = state.vectors.write() {
        *v = index;
    };
}

/// Makes sure the generic calibration goals are embedded for `model` and
/// attached to the index. Returns false while they can't be (Ollama busy).
async fn ensure_pool<R: Runtime>(app: &AppHandle<R>, model: &str) -> bool {
    let state = app.state::<AppState>();
    let cached = state.pool.lock().ok().and_then(|p| {
        p.as_ref()
            .filter(|(m, _)| m == model)
            .map(|(_, v)| v.clone())
    });
    let vectors = match cached {
        Some(v) => v,
        None => {
            let texts: Vec<String> = GENERIC_GOALS
                .iter()
                .map(|g| models::pool_text(model, g))
                .collect();
            match state.ollama.embed(model, &texts, POOL_TIMEOUT).await {
                Ok(v) => {
                    mark_warm(&state);
                    let v: Vec<Vec<f32>> = v.into_iter().map(normalized).collect();
                    if let Ok(mut p) = state.pool.lock() {
                        *p = Some((model.to_string(), v.clone()));
                    }
                    v
                }
                Err(e) => {
                    log::info!("calibration goals not embedded yet: {e}");
                    if e == OllamaError::Unreachable {
                        set_status(app, |s| s.state = AiState::Offline);
                    }
                    return false;
                }
            }
        }
    };
    if let Ok(mut index) = state.vectors.write() {
        if index.model.as_deref() == Some(model) && !index.has_pool() {
            index.set_pool(vectors);
        }
    }
    true
}

struct Pending {
    id: i64,
    /// Views, already formatted for the model.
    texts: Vec<String>,
    hash: String,
}

fn load_pending(lib: &Library, model: &str, indexed: &HashSet<i64>) -> AppResult<Vec<Pending>> {
    let ids: Vec<i64> = {
        let mut stmt = lib
            .conn
            .prepare("SELECT id FROM sparks ORDER BY favorite DESC, usage_count DESC, id")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        rows.collect::<rusqlite::Result<Vec<i64>>>()?
            .into_iter()
            .filter(|id| !indexed.contains(id))
            .take(5000)
            .collect()
    };
    let mut order: HashMap<i64, usize> = HashMap::new();
    for (i, id) in ids.iter().enumerate() {
        order.insert(*id, i);
    }
    let mut out = Vec::new();
    docs_in_batches(lib, &ids, |d| {
        let views = spark_views(&d);
        out.push(Pending {
            id: d.id,
            hash: views_hash(model, &views),
            texts: views
                .iter()
                .map(|v| models::document_text(model, v))
                .collect(),
        });
    })?;
    out.sort_by_key(|p| order.get(&p.id).copied().unwrap_or(usize::MAX));
    Ok(out)
}

/// Stores vectors, skipping Sparks that were edited or deleted meanwhile.
fn store_vectors(
    lib: &mut Library,
    model: &str,
    batch: &[&Pending],
    vectors: Vec<Vec<Vec<f32>>>,
) -> AppResult<Vec<(i64, Vec<Vec<f32>>)>> {
    let ids: Vec<i64> = batch.iter().map(|p| p.id).collect();
    let current: HashMap<i64, String> = sparks::search_docs(lib, &ids)?
        .into_iter()
        .map(|d| (d.id, views_hash(model, &spark_views(&d))))
        .collect();
    let tx = lib.conn.transaction()?;
    let mut stored = Vec::new();
    for (pending, views) in batch.iter().zip(vectors) {
        if current.get(&pending.id) != Some(&pending.hash) || views.is_empty() {
            continue;
        }
        let views: Vec<Vec<f32>> = views.into_iter().map(normalized).collect();
        let dims = views[0].len();
        if dims == 0 || views.iter().any(|v| v.len() != dims) {
            continue;
        }
        tx.execute(
            "INSERT OR REPLACE INTO embeddings (spark_id, model, dimensions, vector, content_hash, generated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![pending.id, model, dims as i64, encode_views(&views), pending.hash, now_ms()],
        )?;
        stored.push((pending.id, views));
    }
    tx.commit()?;
    Ok(stored)
}

/// Groups Sparks into requests of about [`BATCH_TEXTS`] texts, never splitting
/// one Spark's views across requests.
fn batches(pending: &[Pending]) -> Vec<Vec<&Pending>> {
    let mut out: Vec<Vec<&Pending>> = Vec::new();
    let mut current: Vec<&Pending> = Vec::new();
    let mut texts = 0;
    for p in pending {
        if !current.is_empty() && texts + p.texts.len() > BATCH_TEXTS {
            out.push(std::mem::take(&mut current));
            texts = 0;
        }
        texts += p.texts.len();
        current.push(p);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// Embeds every Spark that lacks current vectors for the embedding model.
async fn index_pending<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState>();
    let Some(model) = state.ai_status().embed_model else {
        return;
    };
    let generation = state.generation();
    let indexed: HashSet<i64> = state
        .vectors
        .read()
        .map(|v| {
            if v.model.as_deref() == Some(model.as_str()) {
                v.ids().collect()
            } else {
                HashSet::new()
            }
        })
        .unwrap_or_default();
    let pending = match state.with_library(|lib| load_pending(lib, &model, &indexed)) {
        Ok(p) if !p.is_empty() => p,
        _ => return,
    };
    set_status(app, |s| s.indexing = true);

    for (i, batch) in batches(&pending).into_iter().enumerate() {
        let inputs: Vec<String> = batch.iter().flat_map(|p| p.texts.iter().cloned()).collect();
        let timeout = if i == 0 {
            FIRST_BATCH_TIMEOUT
        } else {
            BATCH_TIMEOUT
        };
        match state.ollama.embed(&model, &inputs, timeout).await {
            Ok(flat) => {
                mark_warm(&state);
                if state.generation() != generation {
                    break; // library switched; this work belongs to the old one
                }
                let mut it = flat.into_iter();
                let per_spark: Vec<Vec<Vec<f32>>> = batch
                    .iter()
                    .map(|p| it.by_ref().take(p.texts.len()).collect())
                    .collect();
                match state.write_library(|lib| store_vectors(lib, &model, &batch, per_spark)) {
                    Ok(stored) => {
                        if let Ok(mut index) = state.vectors.write() {
                            if index.model.as_deref() == Some(model.as_str()) {
                                for (id, views) in stored {
                                    index.insert(id, views);
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

/// Embeds a goal for semantic search. `Err` says why standard search must be
/// used instead; the UI shows it, so the mode never changes silently.
pub async fn embed_query<R: Runtime>(app: &AppHandle<R>, goal: &str) -> Result<Vec<f32>, Fallback> {
    let state = app.state::<AppState>();
    let status = state.ai_status();
    if status.state != AiState::Online {
        return Err(Fallback::Offline);
    }
    let model = status.embed_model.ok_or(Fallback::NoEmbeddingModel)?;
    let ready = state
        .vectors
        .read()
        .map(|v| v.model.as_deref() == Some(model.as_str()) && v.ready())
        .unwrap_or(false);
    if !ready {
        wake(app);
        return Err(Fallback::Indexing);
    }
    let timeout = if is_warm(&state) {
        QUERY_TIMEOUT_WARM
    } else {
        QUERY_TIMEOUT_COLD
    };
    let input = vec![models::query_text(&model, goal)];
    match state.ollama.embed(&model, &input, timeout).await {
        Ok(mut v) => {
            mark_warm(&state);
            v.pop().ok_or(Fallback::Failed)
        }
        Err(e) => {
            log::info!("query embedding unavailable, using standard search: {e}");
            wake(app);
            Err(match e {
                OllamaError::Unreachable => {
                    set_status(app, |s| s.state = AiState::Offline);
                    Fallback::Offline
                }
                OllamaError::Timeout => Fallback::TimedOut,
                OllamaError::ModelMissing => Fallback::NoEmbeddingModel,
                OllamaError::BadResponse(_) => Fallback::Failed,
            })
        }
    }
}

/// Proposes title/summary/tags for a Spark body. Never saves anything, and
/// only runs when the user asks for it.
pub async fn suggest_metadata<R: Runtime>(
    app: &AppHandle<R>,
    body: &str,
) -> AppResult<MetadataSuggestion> {
    let state = app.state::<AppState>();
    let status = state.ai_status();
    let model = match (status.smart_add_ready(), status.chat_model) {
        (true, Some(m)) => m,
        _ if status.state != AiState::Online => {
            return Err(AppError::Ai(
                "Local intelligence is offline. Add the details yourself.".into(),
            ))
        }
        _ => {
            return Err(AppError::Ai(
                "Auto-fill needs a small local chat model. Add the details yourself.".into(),
            ))
        }
    };
    if body.trim().is_empty() {
        return Err(AppError::Validation("Paste the Spark first.".into()));
    }
    let result = state
        .ollama
        .chat_json(
            &model,
            metadata::messages(body),
            metadata::schema(),
            METADATA_TIMEOUT,
        )
        .await;
    // Drafting may have pushed the embedding model out of memory; reload it
    // quietly so the next search isn't cold.
    rewarm(app);
    let content = result.map_err(|e| {
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

fn rewarm<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState>();
    let Some(model) = state.ai_status().embed_model else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let text = models::pool_text(&model, "warm up");
        if state
            .ollama
            .embed(&model, &[text], FIRST_BATCH_TIMEOUT)
            .await
            .is_ok()
        {
            mark_warm(&state);
        }
    });
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
    if status.embed_model.is_none() {
        return;
    }
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
        rewarm(app);
    }
}

/// Drops the in-memory vectors for a changed/deleted Spark and schedules indexing.
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
    use crate::sparks::SparkInput;

    fn indexed_ids(lib: &Library, model: &str) -> HashSet<i64> {
        let expected = expected_hashes(lib, model).unwrap();
        VectorIndex::load(lib, model, &expected)
            .unwrap()
            .ids()
            .collect()
    }

    fn fake_vectors(batch: &[&Pending], dims: usize) -> Vec<Vec<Vec<f32>>> {
        batch
            .iter()
            .map(|p| {
                (0..p.texts.len())
                    .map(|i| {
                        let mut v = vec![0.0; dims];
                        v[i % dims] = 1.0;
                        v
                    })
                    .collect()
            })
            .collect()
    }

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
        let pending = load_pending(&lib, "m", &HashSet::new()).unwrap();
        assert_eq!(pending.len(), 2);
        assert!(
            pending.iter().all(|p| p.texts.len() >= 2),
            "profile + at least one passage"
        );

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
        let batch: Vec<&Pending> = pending.iter().collect();
        let stored = store_vectors(&mut lib, "m", &batch, fake_vectors(&batch, 3)).unwrap();
        let stored_ids: Vec<i64> = stored.iter().map(|(id, _)| *id).collect();
        assert_eq!(stored_ids, vec![a.id]);

        // B is still pending; A is not. Another model needs its own vectors.
        let done = indexed_ids(&lib, "m");
        assert_eq!(done, HashSet::from([a.id]));
        let still: Vec<i64> = load_pending(&lib, "m", &done)
            .unwrap()
            .iter()
            .map(|p| p.id)
            .collect();
        assert_eq!(still, vec![b.id]);
        assert!(indexed_ids(&lib, "other").is_empty());
    }

    #[test]
    fn vectors_from_an_older_recipe_are_not_loaded() {
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
        // A row written by an earlier build (single vector, old hash).
        lib.conn
            .execute(
                "INSERT INTO embeddings VALUES (?1, 'm', 2, x'0000803f00000000', 'old-hash', 0)",
                [a.id],
            )
            .unwrap();
        assert!(
            indexed_ids(&lib, "m").is_empty(),
            "stale vectors are re-embedded, not used"
        );
        assert_eq!(load_pending(&lib, "m", &HashSet::new()).unwrap().len(), 1);
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
        let pending = load_pending(&lib, "m", &HashSet::new()).unwrap();
        sparks::delete(&mut lib, a.id).unwrap();
        let batch: Vec<&Pending> = pending.iter().collect();
        let stored = store_vectors(&mut lib, "m", &batch, fake_vectors(&batch, 3)).unwrap();
        assert!(stored.is_empty());
    }

    #[test]
    fn batches_never_split_a_spark() {
        let pending: Vec<Pending> = (0..7)
            .map(|id| Pending {
                id,
                texts: vec![String::new(); 5],
                hash: String::new(),
            })
            .collect();
        let groups = batches(&pending);
        assert!(groups
            .iter()
            .all(|g| g.iter().map(|p| p.texts.len()).sum::<usize>() <= BATCH_TEXTS));
        assert_eq!(groups.iter().map(Vec::len).sum::<usize>(), 7);
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

//! Internal model policy. Model selection is intentionally not exposed in the
//! UI; Sparkwell picks sensible installed models and can be overridden with
//! `SPARKWELL_EMBED_MODEL` / `SPARKWELL_CHAT_MODEL` for power users.

use crate::search::profile::clean_goal;

/// A model reported by Ollama's `/api/tags`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InstalledModel {
    pub name: String,
    /// Ollama "cloud" models proxy requests to a remote host. Sparkwell never
    /// uses them: Spark content must stay on this device.
    pub remote: bool,
    /// Download size in bytes, when reported.
    pub size: Option<u64>,
    /// Parameter count in billions, parsed from `details.parameter_size`.
    pub parameters_b: Option<f64>,
}

/// Preferred embedding models, best first.
const EMBED_PREFERENCE: &[&str] = &[
    "nomic-embed-text",
    "mxbai-embed-large",
    "snowflake-arctic-embed2",
    "bge-m3",
    "embeddinggemma",
    "qwen3-embedding",
    "snowflake-arctic-embed",
    "granite-embedding",
    "bge-large",
    "all-minilm",
    "paraphrase-multilingual",
];

/// Preferred small instruction models for metadata drafting, best first.
const CHAT_PREFERENCE: &[&str] = &[
    "qwen2.5",
    "llama3.2",
    "qwen3",
    "gemma3",
    "phi4-mini",
    "llama3.1",
    "mistral",
    "gemma2",
    "phi3",
    "qwen2",
    "llama3",
];

/// Smart Add drafts a title, summary and a few tags: a small model does this
/// well in seconds. Larger models are never loaded for it automatically (they
/// take long to load and can push the embedding model out of memory).
pub const SMART_ADD_MAX_PARAMETERS_B: f64 = 8.5;
/// Fallback when the parameter count is unknown.
pub const SMART_ADD_MAX_BYTES: u64 = 6 * 1024 * 1024 * 1024;

/// Model family without registry namespace or tag: `library/qwen2.5:3b` -> `qwen2.5`.
pub fn family(name: &str) -> &str {
    let without_tag = name.split(':').next().unwrap_or(name);
    without_tag.rsplit('/').next().unwrap_or(without_tag)
}

pub fn is_cloud(model: &InstalledModel) -> bool {
    let lower = model.name.to_ascii_lowercase();
    model.remote || lower.ends_with("-cloud") || lower.ends_with(":cloud")
}

pub fn is_embedding_model(name: &str) -> bool {
    let fam = family(name).to_ascii_lowercase();
    fam.contains("embed")
        || EMBED_PREFERENCE.iter().any(|p| fam == *p)
        || fam.starts_with("bge")
        || fam.starts_with("all-minilm")
        || fam.starts_with("paraphrase-multilingual")
}

/// Parses Ollama's `parameter_size` ("3.2B", "567.75M", "8B") into billions.
pub fn parse_parameters_b(s: &str) -> Option<f64> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic())?);
    let n: f64 = num.trim().parse().ok()?;
    match unit.trim().to_ascii_uppercase().as_str() {
        "B" => Some(n),
        "M" => Some(n / 1000.0),
        "K" => Some(n / 1_000_000.0),
        "T" => Some(n * 1000.0),
        _ => None,
    }
}

fn local(models: &[InstalledModel]) -> impl Iterator<Item = &InstalledModel> {
    models.iter().filter(|m| !is_cloud(m))
}

fn matches_override(m: &InstalledModel, wanted: &str) -> bool {
    m.name == wanted || family(&m.name) == wanted || m.name == format!("{wanted}:latest")
}

pub fn pick_embedding_model(
    models: &[InstalledModel],
    override_name: Option<&str>,
) -> Option<String> {
    let candidates: Vec<&InstalledModel> = local(models)
        .filter(|m| is_embedding_model(&m.name))
        .collect();
    if let Some(wanted) = override_name.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(m) = candidates.iter().find(|m| matches_override(m, wanted)) {
            return Some(m.name.clone());
        }
    }
    for pref in EMBED_PREFERENCE {
        if let Some(m) = candidates
            .iter()
            .find(|m| family(&m.name).eq_ignore_ascii_case(pref))
        {
            return Some(m.name.clone());
        }
    }
    candidates.first().map(|m| m.name.clone())
}

/// Whether a chat model is small enough for Smart Add.
pub fn suitable_for_smart_add(m: &InstalledModel) -> bool {
    match (m.parameters_b, m.size) {
        (Some(p), _) => p <= SMART_ADD_MAX_PARAMETERS_B,
        (None, Some(bytes)) => bytes <= SMART_ADD_MAX_BYTES,
        (None, None) => false,
    }
}

/// The chat model Smart Add would use, and whether larger local chat models
/// were passed over (so the UI can explain why Auto-fill is unavailable).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChatChoice {
    pub model: Option<String>,
    pub skipped_large: bool,
}

/// Prefers known small instruction families, then the smallest suitable
/// local model. Never cloud models; never models above the Smart Add size
/// limit unless explicitly named in `SPARKWELL_CHAT_MODEL`.
pub fn pick_chat_model(models: &[InstalledModel], override_name: Option<&str>) -> ChatChoice {
    let chat: Vec<&InstalledModel> = local(models)
        .filter(|m| !is_embedding_model(&m.name))
        .collect();
    if let Some(wanted) = override_name.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(m) = chat.iter().find(|m| matches_override(m, wanted)) {
            return ChatChoice {
                model: Some(m.name.clone()),
                skipped_large: false,
            };
        }
    }
    let mut suitable: Vec<&InstalledModel> = chat
        .iter()
        .copied()
        .filter(|m| suitable_for_smart_add(m))
        .collect();
    let skipped_large = suitable.len() < chat.len();
    let rank = |m: &InstalledModel| {
        CHAT_PREFERENCE
            .iter()
            .position(|p| family(&m.name).eq_ignore_ascii_case(p))
            .unwrap_or(CHAT_PREFERENCE.len())
    };
    suitable.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then(a.size.unwrap_or(u64::MAX).cmp(&b.size.unwrap_or(u64::MAX)))
            .then(a.name.cmp(&b.name))
    });
    ChatChoice {
        model: suitable.first().map(|m| m.name.clone()),
        skipped_large,
    }
}

/// How a model family expects queries and documents to be phrased. Models
/// trained with task instructions or prefixes retrieve measurably better when
/// they are used.
struct EmbeddingFormat {
    query_prefix: &'static str,
    document_prefix: &'static str,
}

fn embedding_format(model: &str) -> EmbeddingFormat {
    let fam = family(model).to_ascii_lowercase();
    let (query_prefix, document_prefix) = match fam.as_str() {
        "nomic-embed-text" => ("search_query: ", "search_document: "),
        "mxbai-embed-large" | "snowflake-arctic-embed" => {
            ("Represent this sentence for searching relevant passages: ", "")
        }
        "snowflake-arctic-embed2" => ("query: ", ""),
        "embeddinggemma" => ("task: search result | query: ", "title: none | text: "),
        f if f.starts_with("qwen3-embedding") => (
            "Instruct: Given a user's goal, retrieve the saved AI prompt that best helps accomplish it\nQuery: ",
            "",
        ),
        f if f.contains("e5") => ("query: ", "passage: "),
        _ => ("", ""),
    };
    EmbeddingFormat {
        query_prefix,
        document_prefix,
    }
}

/// The text embedded for a user's goal (conversational openers removed).
pub fn query_text(model: &str, goal: &str) -> String {
    format!(
        "{}{}",
        embedding_format(model).query_prefix,
        clean_goal(goal)
    )
}

/// The text embedded for one of the generic calibration goals.
pub fn pool_text(model: &str, goal: &str) -> String {
    format!("{}{goal}", embedding_format(model).query_prefix)
}

/// The text embedded for one view of a Spark.
pub fn document_text(model: &str, view: &str) -> String {
    format!("{}{view}", embedding_format(model).document_prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str) -> InstalledModel {
        InstalledModel {
            name: name.into(),
            ..Default::default()
        }
    }

    fn sized(name: &str, params: f64, gb: f64) -> InstalledModel {
        InstalledModel {
            name: name.into(),
            remote: false,
            size: Some((gb * 1024.0 * 1024.0 * 1024.0) as u64),
            parameters_b: Some(params),
        }
    }

    #[test]
    fn picks_preferred_embedding_model() {
        let models = vec![
            m("llama3.2:3b"),
            m("all-minilm:latest"),
            m("nomic-embed-text:latest"),
        ];
        assert_eq!(
            pick_embedding_model(&models, None).as_deref(),
            Some("nomic-embed-text:latest")
        );
    }

    #[test]
    fn unknown_models_still_usable_for_embeddings() {
        let models = vec![m("my-custom-embedder:1"), m("someone/fancy-chat:7b")];
        assert_eq!(
            pick_embedding_model(&models, None).as_deref(),
            Some("my-custom-embedder:1")
        );
    }

    #[test]
    fn nothing_installed() {
        assert_eq!(pick_embedding_model(&[], None), None);
        assert_eq!(
            pick_chat_model(&[m("nomic-embed-text")], None),
            ChatChoice::default()
        );
    }

    #[test]
    fn cloud_models_are_never_selected() {
        let models = vec![
            m("gpt-oss:120b-cloud"),
            InstalledModel {
                name: "qwen3-coder:480b".into(),
                remote: true,
                ..Default::default()
            },
            m("deepseek-v3.1:cloud"),
        ];
        assert_eq!(pick_chat_model(&models, None).model, None);
        assert_eq!(
            pick_chat_model(&models, Some("gpt-oss:120b-cloud")).model,
            None
        );
    }

    #[test]
    fn smart_add_never_loads_large_chat_models_by_itself() {
        // The acceptance workstation: only large local chat models + cloud ones.
        let models = vec![
            sized("nemotron-3.5-lightning:30b", 31.6, 23.6),
            sized("glm-4.7-flash:latest", 29.9, 17.7),
            sized("gemma4:12b", 12.2, 7.0),
            sized("qwen3-embedding:0.6b", 0.6, 0.6),
            m("minimax-m3:cloud"),
        ];
        let choice = pick_chat_model(&models, None);
        assert_eq!(choice.model, None);
        assert!(
            choice.skipped_large,
            "the UI explains that installed models are too large"
        );
        // An explicit override is respected (power users accept the cost).
        assert_eq!(
            pick_chat_model(&models, Some("gemma4:12b"))
                .model
                .as_deref(),
            Some("gemma4:12b")
        );
    }

    #[test]
    fn smart_add_prefers_small_known_families_then_size() {
        let models = vec![
            sized("llama3.1:8b", 8.0, 4.9),
            sized("mystery-chat:3b", 3.0, 1.9),
            sized("qwen2.5:7b", 7.6, 4.7),
            sized("qwen2.5:3b", 3.1, 1.9),
        ];
        assert_eq!(
            pick_chat_model(&models, None).model.as_deref(),
            Some("qwen2.5:3b")
        );
        let unknown_size = vec![m("tiny-llm"), sized("gemma3:4b", 4.3, 3.3)];
        assert_eq!(
            pick_chat_model(&unknown_size, None).model.as_deref(),
            Some("gemma3:4b")
        );
    }

    #[test]
    fn override_is_respected_when_installed() {
        let models = vec![m("nomic-embed-text:latest"), m("mxbai-embed-large:latest")];
        assert_eq!(
            pick_embedding_model(&models, Some("mxbai-embed-large")).as_deref(),
            Some("mxbai-embed-large:latest")
        );
        // Unknown override falls back to policy.
        assert_eq!(
            pick_embedding_model(&models, Some("missing")).as_deref(),
            Some("nomic-embed-text:latest")
        );
    }

    #[test]
    fn parameter_sizes_parse() {
        assert_eq!(parse_parameters_b("3.2B"), Some(3.2));
        assert_eq!(parse_parameters_b("567.75M"), Some(0.56775));
        assert_eq!(parse_parameters_b("8B"), Some(8.0));
        assert_eq!(parse_parameters_b(""), None);
        assert_eq!(parse_parameters_b("big"), None);
    }

    #[test]
    fn model_aware_query_and_document_text() {
        assert_eq!(family("registry.ollama.ai/library/qwen2.5:3b"), "qwen2.5");
        assert_eq!(
            query_text(
                "nomic-embed-text:latest",
                "I need AI to help me build an MCP server"
            ),
            "search_query: Build an MCP server"
        );
        assert_eq!(document_text("nomic-embed-text", "x"), "search_document: x");
        let q = query_text("qwen3-embedding:0.6b", "my app keeps crashing");
        assert!(q.starts_with("Instruct: "));
        assert!(q.ends_with("\nQuery: My app keeps crashing"));
        assert_eq!(document_text("qwen3-embedding:0.6b", "x"), "x");
        assert_eq!(query_text("all-minilm", "plan a launch"), "Plan a launch");
    }
}

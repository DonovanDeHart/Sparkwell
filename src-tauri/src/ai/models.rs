//! Internal model policy. Model selection is intentionally not exposed in the
//! UI.
//!
//! Semantic retrieval uses one canonical embedding model,
//! [`CANONICAL_EMBED_MODEL`]: the retrieval pipeline (query instruction,
//! thresholds, calibration) is tuned and tested for it, so Sparkwell never
//! substitutes whichever other embedding model happens to be installed. If it
//! is missing, standard search is used and the UI says how to install it.
//! `SPARKWELL_EMBED_MODEL` overrides this for development only (uncalibrated).
//!
//! Smart Add picks the smallest capable installed local chat model up to the
//! 14B class (`SPARKWELL_CHAT_MODEL` overrides); embedding and cloud models
//! are never used as chat models.

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

/// Sparkwell's supported semantic-retrieval model (Qwen3 Embedding 8B, Q8_0).
pub const CANONICAL_EMBED_MODEL: &str = "qwen3-embedding:8b-q8_0";

/// Known embedding-model families. Only used to keep embedding models out of
/// the chat-model choice; semantic search itself uses the canonical model.
const EMBEDDING_FAMILIES: &[&str] = &[
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

/// Smart Add drafts a title, summary and a few tags. Local instruction models
/// up to the 14B class do this well: gemma4:12b answers in under a second
/// once loaded on an RTX 5090 and fits beside the embedding model. Larger
/// (27B+) models are never loaded for it automatically: they take much longer
/// to load and compete with other work for GPU memory.
pub const SMART_ADD_MAX_PARAMETERS_B: f64 = 15.0;
/// Fallback when the parameter count is unknown (a 14B model at Q4 is ~9 GB).
pub const SMART_ADD_MAX_BYTES: u64 = 10 * 1024 * 1024 * 1024;
/// Below this, drafted details get noticeably worse; such models are only
/// used when nothing larger (within the limit) is installed.
const SMART_ADD_QUALITY_MIN_B: f64 = 2.5;

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
        || EMBEDDING_FAMILIES.iter().any(|p| fam == *p)
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

/// The embedding model semantic search uses: the canonical model when it is
/// installed locally, otherwise none (standard search). A development
/// override is honoured only if that model is installed.
pub fn pick_embedding_model(
    models: &[InstalledModel],
    override_name: Option<&str>,
) -> Option<String> {
    let candidates: Vec<&InstalledModel> = local(models)
        .filter(|m| is_embedding_model(&m.name))
        .collect();
    let wanted = override_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(CANONICAL_EMBED_MODEL);
    candidates
        .iter()
        .find(|m| m.name == wanted || m.name == format!("{wanted}:latest"))
        .map(|m| m.name.clone())
}

/// Whether a chat model is within the size Smart Add loads by itself.
pub fn suitable_for_smart_add(m: &InstalledModel) -> bool {
    match (m.parameters_b, m.size) {
        (Some(p), _) => p <= SMART_ADD_MAX_PARAMETERS_B,
        (None, Some(bytes)) => bytes <= SMART_ADD_MAX_BYTES,
        (None, None) => false,
    }
}

/// Approximate size in billions of parameters (Q4 weights are ~0.6 bytes each).
fn approx_parameters_b(m: &InstalledModel) -> f64 {
    m.parameters_b
        .or_else(|| m.size.map(|bytes| bytes as f64 / 0.6e9))
        .unwrap_or(f64::MAX)
}

/// The local chat models Smart Add may use, best first. Each is still checked
/// with Ollama ([`chat_capable`]) before it is used.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChatPlan {
    pub candidates: Vec<String>,
    /// Local chat models exist, but all are above the automatic size limit.
    pub skipped_large: bool,
    /// The installed model `SPARKWELL_CHAT_MODEL` names (first candidate).
    pub override_model: Option<String>,
    /// Why `SPARKWELL_CHAT_MODEL` can't be used (automatic choice applies).
    pub override_issue: Option<String>,
}

/// Prefers the smallest capable local model within the size limit (tiny
/// models only as a last resort). Never cloud or embedding models; a larger
/// local model only when explicitly named in `SPARKWELL_CHAT_MODEL`.
pub fn plan_chat_models(models: &[InstalledModel], override_name: Option<&str>) -> ChatPlan {
    let chat: Vec<&InstalledModel> = local(models)
        .filter(|m| !is_embedding_model(&m.name))
        .collect();
    let mut suitable: Vec<&InstalledModel> = chat
        .iter()
        .copied()
        .filter(|m| suitable_for_smart_add(m))
        .collect();
    let tiny = |m: &InstalledModel| approx_parameters_b(m) < SMART_ADD_QUALITY_MIN_B;
    suitable.sort_by(|a, b| {
        tiny(a)
            .cmp(&tiny(b))
            .then(approx_parameters_b(a).total_cmp(&approx_parameters_b(b)))
            .then(a.name.cmp(&b.name))
    });
    let mut plan = ChatPlan {
        candidates: suitable.iter().map(|m| m.name.clone()).collect(),
        skipped_large: !chat.is_empty() && suitable.is_empty(),
        ..Default::default()
    };
    if let Some(wanted) = override_name.map(str::trim).filter(|s| !s.is_empty()) {
        match chat.iter().find(|m| matches_override(m, wanted)) {
            Some(m) => {
                plan.candidates.retain(|c| c != &m.name);
                plan.candidates.insert(0, m.name.clone());
                plan.override_model = Some(m.name.clone());
            }
            None => {
                let issue = if models
                    .iter()
                    .any(|m| matches_override(m, wanted) && is_cloud(m))
                {
                    "is a cloud model; Sparkwell only uses local models"
                } else if models
                    .iter()
                    .any(|m| matches_override(m, wanted) && is_embedding_model(&m.name))
                {
                    "is an embedding model, not a chat model"
                } else {
                    "isn't installed in Ollama"
                };
                plan.override_issue = Some(format!("{wanted} {issue}"));
            }
        }
    }
    plan
}

/// Whether Ollama reports a model as a text-generation model. Older Ollama
/// versions don't report capabilities; the name checks above then apply.
pub fn chat_capable(capabilities: Option<&[String]>) -> bool {
    capabilities.map_or(true, |caps| {
        caps.iter().any(|c| c == "completion") && !caps.iter().any(|c| c == "embedding")
    })
}

/// Sparkwell's retrieval task for Qwen3 Embedding, in the model card's format.
const QWEN3_QUERY_PREFIX: &str = "Instruct: Given a user's goal, retrieve the reusable AI prompt whose intended purpose best helps accomplish it, matching intention and desired outcome rather than exact wording\nQuery: ";

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
        "mxbai-embed-large" | "snowflake-arctic-embed" => (
            "Represent this sentence for searching relevant passages: ",
            "",
        ),
        "snowflake-arctic-embed2" => ("query: ", ""),
        "embeddinggemma" => ("task: search result | query: ", "title: none | text: "),
        // Qwen3 Embedding is instruction-aware: queries carry a one-sentence
        // task ("Instruct: {task}\nQuery: {query}"), documents are embedded
        // as they are (per the model card), so the two are asymmetric.
        f if f.starts_with("qwen3-embedding") => (QWEN3_QUERY_PREFIX, ""),
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
    fn semantic_search_uses_the_canonical_model() {
        let models = vec![
            m("llama3.2:3b"),
            m("nomic-embed-text:latest"),
            m("qwen3-embedding:0.6b"),
            m(CANONICAL_EMBED_MODEL),
        ];
        assert_eq!(
            pick_embedding_model(&models, None).as_deref(),
            Some(CANONICAL_EMBED_MODEL)
        );
    }

    #[test]
    fn other_embedding_models_are_never_substituted() {
        // Uncalibrated models would give unpredictable quality: standard
        // search (with install guidance) is used instead.
        let models = vec![
            m("nomic-embed-text:latest"),
            m("qwen3-embedding:0.6b"),
            m("qwen3-embedding:8b"),
            m("my-custom-embedder:1"),
        ];
        assert_eq!(pick_embedding_model(&models, None), None);
    }

    #[test]
    fn embedding_models_are_never_chat_models() {
        let models = vec![sized(CANONICAL_EMBED_MODEL, 7.6, 8.1)];
        assert_eq!(plan_chat_models(&models, None), ChatPlan::default());
        let plan = plan_chat_models(&models, Some(CANONICAL_EMBED_MODEL));
        assert!(plan.candidates.is_empty());
        assert_eq!(
            plan.override_issue.as_deref(),
            Some("qwen3-embedding:8b-q8_0 is an embedding model, not a chat model")
        );
    }

    #[test]
    fn qwen3_queries_carry_the_retrieval_instruction_and_documents_do_not() {
        let q = query_text(
            CANONICAL_EMBED_MODEL,
            "I need AI to help me build an MCP server",
        );
        assert!(q.starts_with("Instruct: "), "{q}");
        assert!(q.ends_with("\nQuery: Build an MCP server"), "{q}");
        assert_eq!(q.matches('\n').count(), 1);
        let doc = "Title: MCP Server Architect";
        assert_eq!(document_text(CANONICAL_EMBED_MODEL, doc), doc);
    }

    #[test]
    fn nothing_installed() {
        assert_eq!(pick_embedding_model(&[], None), None);
        assert_eq!(
            plan_chat_models(&[m("nomic-embed-text")], None),
            ChatPlan::default()
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
        assert_eq!(plan_chat_models(&models, None), ChatPlan::default());
        let plan = plan_chat_models(&models, Some("gpt-oss:120b-cloud"));
        assert!(plan.candidates.is_empty());
        assert_eq!(
            plan.override_issue.as_deref(),
            Some("gpt-oss:120b-cloud is a cloud model; Sparkwell only uses local models")
        );
    }

    /// The models installed on the acceptance workstation (RTX 5090).
    fn acceptance_workstation() -> Vec<InstalledModel> {
        vec![
            sized(CANONICAL_EMBED_MODEL, 7.6, 7.5),
            m("minimax-m3:cloud"),
            InstalledModel {
                name: "nemotron-3-ultra:cloud".into(),
                remote: true,
                parameters_b: Some(550.0),
                ..Default::default()
            },
            m("gpt-oss:120b-cloud"),
            sized("nemotron-3.5-lightning:30b", 32.9, 23.6),
            sized("glm-4.7-flash:latest", 29.9, 17.7),
            sized("gemma4:31b", 31.3, 18.5),
            sized("qwen3-embedding:0.6b", 0.6, 0.6),
            sized("gemma4:12b", 11.9, 7.1),
            sized("qwen3.8:27b", 27.3, 16.5),
        ]
    }

    #[test]
    fn the_acceptance_workstation_drafts_with_its_12b_model() {
        let plan = plan_chat_models(&acceptance_workstation(), None);
        // Only the 12B model qualifies: never 27B+, cloud or embedding models.
        assert_eq!(plan.candidates, vec!["gemma4:12b"]);
        assert!(!plan.skipped_large);
        assert_eq!(plan.override_issue, None);
    }

    #[test]
    fn larger_models_are_never_loaded_automatically() {
        let models: Vec<InstalledModel> = acceptance_workstation()
            .into_iter()
            .filter(|m| m.name != "gemma4:12b")
            .collect();
        let plan = plan_chat_models(&models, None);
        assert!(plan.candidates.is_empty());
        assert!(
            plan.skipped_large,
            "the UI explains that installed models are too large"
        );
        // An explicit override is respected (power users accept the cost).
        assert_eq!(
            plan_chat_models(&models, Some("gemma4:31b")).candidates,
            vec!["gemma4:31b"]
        );
    }

    #[test]
    fn smart_add_prefers_the_smallest_capable_model() {
        let models = vec![
            sized("gemma3:12b", 12.2, 8.1),
            sized("qwen2.5:0.5b", 0.5, 0.4),
            sized("mistral-nemo:12b", 12.2, 7.1),
            sized("llama3.2:3b", 3.2, 2.0),
            sized("phi4:14b", 14.7, 9.1),
        ];
        assert_eq!(
            plan_chat_models(&models, None).candidates,
            vec![
                "llama3.2:3b",
                "gemma3:12b",
                "mistral-nemo:12b",
                "phi4:14b",
                // Tiny models draft poorly: last resort only.
                "qwen2.5:0.5b",
            ]
        );
        // Without a parameter count, the download size decides.
        let unknown = vec![
            InstalledModel {
                name: "mystery:latest".into(),
                size: Some(4_700_000_000),
                ..Default::default()
            },
            m("no-size-info"),
        ];
        assert_eq!(
            plan_chat_models(&unknown, None).candidates,
            vec!["mystery:latest"]
        );
    }

    #[test]
    fn a_chat_model_override_is_checked_and_otherwise_ignored() {
        let models = acceptance_workstation();
        let plan = plan_chat_models(&models, Some("qwen3.8:27b"));
        assert_eq!(plan.candidates, vec!["qwen3.8:27b", "gemma4:12b"]);
        assert_eq!(plan.override_model.as_deref(), Some("qwen3.8:27b"));
        assert_eq!(plan.override_issue, None);

        let plan = plan_chat_models(&models, Some("llama3.2:3b"));
        assert_eq!(
            plan.candidates,
            vec!["gemma4:12b"],
            "automatic choice applies"
        );
        assert_eq!(
            plan.override_issue.as_deref(),
            Some("llama3.2:3b isn't installed in Ollama")
        );
        assert_eq!(plan_chat_models(&models, Some("  ")).override_issue, None);
    }

    #[test]
    fn only_text_generation_models_can_draft() {
        let caps = |c: &[&str]| c.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(chat_capable(Some(&caps(&[
            "completion",
            "vision",
            "tools",
            "thinking"
        ]))));
        assert!(!chat_capable(Some(&caps(&["tools", "embedding"]))));
        assert!(!chat_capable(Some(&caps(&["vision"]))));
        assert!(chat_capable(None), "older Ollama: name checks apply");
    }

    #[test]
    fn development_override_is_respected_only_when_installed() {
        let models = vec![m("nomic-embed-text:latest"), m(CANONICAL_EMBED_MODEL)];
        assert_eq!(
            pick_embedding_model(&models, Some("nomic-embed-text")).as_deref(),
            Some("nomic-embed-text:latest")
        );
        assert_eq!(pick_embedding_model(&models, Some("missing")), None);
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

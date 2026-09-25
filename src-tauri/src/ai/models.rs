//! Internal model policy. Model selection is intentionally not exposed in the
//! UI; Sparkwell picks sensible installed models and can be overridden with
//! `SPARKWELL_EMBED_MODEL` / `SPARKWELL_CHAT_MODEL` for power users.

/// A model reported by Ollama's `/api/tags`.
#[derive(Debug, Clone, PartialEq)]
pub struct InstalledModel {
    pub name: String,
    /// Ollama "cloud" models proxy requests to a remote host. Sparkwell never
    /// uses them: Spark content must stay on this device.
    pub remote: bool,
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
    "qwen2.5", "llama3.2", "qwen3", "gemma3", "phi4-mini", "llama3.1", "mistral", "gemma2",
    "phi3", "qwen2", "llama3",
];

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

fn local(models: &[InstalledModel]) -> impl Iterator<Item = &InstalledModel> {
    models.iter().filter(|m| !is_cloud(m))
}

fn pick(models: &[InstalledModel], preference: &[&str], override_name: Option<&str>, want_embedding: bool) -> Option<String> {
    let candidates: Vec<&InstalledModel> = local(models)
        .filter(|m| is_embedding_model(&m.name) == want_embedding)
        .collect();
    if let Some(wanted) = override_name.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some(m) = candidates
            .iter()
            .find(|m| m.name == wanted || family(&m.name) == wanted || m.name == format!("{wanted}:latest"))
        {
            return Some(m.name.clone());
        }
    }
    for pref in preference {
        if let Some(m) = candidates.iter().find(|m| family(&m.name).eq_ignore_ascii_case(pref)) {
            return Some(m.name.clone());
        }
    }
    candidates.first().map(|m| m.name.clone())
}

pub fn pick_embedding_model(models: &[InstalledModel], override_name: Option<&str>) -> Option<String> {
    pick(models, EMBED_PREFERENCE, override_name, true)
}

pub fn pick_chat_model(models: &[InstalledModel], override_name: Option<&str>) -> Option<String> {
    pick(models, CHAT_PREFERENCE, override_name, false)
}

/// Some embedding models are trained with task prefixes; using them measurably
/// improves retrieval quality.
pub fn document_prefix(model: &str) -> &'static str {
    match family(model) {
        "nomic-embed-text" => "search_document: ",
        _ => "",
    }
}

pub fn query_prefix(model: &str) -> &'static str {
    match family(model) {
        "nomic-embed-text" => "search_query: ",
        "mxbai-embed-large" => "Represent this sentence for searching relevant passages: ",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str) -> InstalledModel {
        InstalledModel { name: name.into(), remote: false }
    }

    #[test]
    fn picks_preferred_embedding_model() {
        let models = vec![m("llama3.2:3b"), m("all-minilm:latest"), m("nomic-embed-text:latest")];
        assert_eq!(pick_embedding_model(&models, None).as_deref(), Some("nomic-embed-text:latest"));
        assert_eq!(pick_chat_model(&models, None).as_deref(), Some("llama3.2:3b"));
    }

    #[test]
    fn unknown_models_still_usable() {
        let models = vec![m("my-custom-embedder:1"), m("someone/fancy-chat:7b")];
        assert_eq!(pick_embedding_model(&models, None).as_deref(), Some("my-custom-embedder:1"));
        assert_eq!(pick_chat_model(&models, None).as_deref(), Some("someone/fancy-chat:7b"));
    }

    #[test]
    fn nothing_installed() {
        assert_eq!(pick_embedding_model(&[], None), None);
        assert_eq!(pick_chat_model(&[m("nomic-embed-text")], None), None);
    }

    #[test]
    fn cloud_models_are_never_selected() {
        let models = vec![
            m("gpt-oss:120b-cloud"),
            InstalledModel { name: "qwen3-coder:480b".into(), remote: true },
            m("deepseek-v3.1:cloud"),
        ];
        assert_eq!(pick_chat_model(&models, None), None);
        assert_eq!(pick_chat_model(&models, Some("gpt-oss:120b-cloud")), None);
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
    fn family_parsing() {
        assert_eq!(family("registry.ollama.ai/library/qwen2.5:3b"), "qwen2.5");
        assert_eq!(family("nomic-embed-text"), "nomic-embed-text");
        assert_eq!(query_prefix("nomic-embed-text:latest"), "search_query: ");
    }
}

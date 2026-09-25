//! Text normalisation for intent-style queries.
//!
//! Users describe outcomes ("I need AI to help me build an MCP server"), so
//! the conversational scaffolding is removed before matching and a light
//! stemmer lets "servers"/"server" and "debugging"/"debug" meet.

/// Conversational filler that carries no retrieval signal in a goal statement.
const STOPWORDS: &[&str] = &[
    "a", "able", "about", "accomplish", "ai", "also", "am", "an", "and", "any", "anything", "are",
    "as", "assist", "at", "be", "been", "being", "but", "by", "can", "chatgpt", "claude", "could",
    "d", "did", "do", "does", "doing", "done", "for", "from", "gemini", "get", "got", "help",
    "helping", "helps", "how", "i", "if", "im", "in", "into", "is", "it", "its", "just", "like",
    "ll", "llm", "m", "make", "making", "may", "me", "might", "mine", "must", "my", "need",
    "needed", "needs", "of", "on", "or", "our", "please", "re", "really", "s", "shall", "should",
    "so", "some", "something", "spark", "sparks", "t", "than", "that", "the", "then", "these",
    "thing", "things", "this", "those", "to", "try", "trying", "us", "use", "using", "ve", "very",
    "want", "wanted", "wants", "was", "way", "we", "were", "what", "when", "where", "which", "who",
    "why", "will", "with", "would", "you", "your",
];

fn fold_char(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        other => other,
    }
}

/// Lowercases, folds common accents, and splits on anything that is not a
/// letter or digit.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            current.push(fold_char(c));
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

pub fn is_stopword(token: &str) -> bool {
    STOPWORDS.binary_search(&token).is_ok()
}

/// Deliberately light suffix stripping. It only needs to be applied
/// consistently to both sides of a comparison.
pub fn stem(token: &str) -> String {
    if token.chars().count() <= 3 || !token.is_ascii() {
        return token.to_string();
    }
    const RULES: &[(&str, &str)] = &[
        ("ies", "y"),
        ("ied", "y"),
        ("ing", ""),
        ("ers", ""),
        ("ed", ""),
        ("es", ""),
        ("er", ""),
        ("s", ""),
    ];
    for (suffix, replacement) in RULES {
        if let Some(base) = token.strip_suffix(suffix) {
            if base.len() >= 3 && !(suffix == &"s" && base.ends_with('s')) {
                return format!("{base}{replacement}");
            }
        }
    }
    token.to_string()
}

/// Whether two stemmed tokens should be considered the same concept.
pub fn tokens_match(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    short.len() >= 4 && long.starts_with(short)
}

/// Meaningful, stemmed, de-duplicated query terms. Falls back to all terms if
/// the query consisted only of filler words (e.g. "AI").
pub fn query_terms(query: &str) -> Vec<String> {
    let raw = tokenize(query);
    let mut picked: Vec<String> = raw
        .iter()
        .filter(|t| !is_stopword(t))
        .map(|t| stem(t))
        .collect();
    if picked.is_empty() {
        picked = raw.iter().filter(|t| t.chars().count() > 1).map(|t| stem(t)).collect();
    }
    let mut out: Vec<String> = Vec::new();
    for t in picked {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

/// Builds an FTS5 MATCH expression: any meaningful term, prefix-matched.
/// Tokens are alphanumeric only, so quoting them is injection-safe.
pub fn fts_query(query: &str) -> String {
    let raw = tokenize(query);
    let mut terms: Vec<&String> = raw.iter().filter(|t| !is_stopword(t)).collect();
    if terms.is_empty() {
        terms = raw.iter().filter(|t| t.chars().count() > 1).collect();
    }
    let mut seen: Vec<&str> = Vec::new();
    let mut parts = Vec::new();
    for t in terms {
        if seen.contains(&t.as_str()) {
            continue;
        }
        seen.push(t);
        if t.chars().count() >= 3 {
            parts.push(format!("\"{t}\"*"));
        } else {
            parts.push(format!("\"{t}\""));
        }
    }
    parts.join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopwords_are_sorted_for_binary_search() {
        let mut sorted = STOPWORDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, STOPWORDS, "STOPWORDS must stay sorted");
    }

    #[test]
    fn intent_phrasing_is_stripped() {
        assert_eq!(query_terms("I need AI to help me build an MCP server."), vec!["build", "mcp", "serv"]);
        assert_eq!(
            query_terms("Help me write a YouTube video script"),
            vec!["write", "youtube", "video", "script"]
        );
        // A query made only of filler keeps its words rather than matching nothing.
        assert_eq!(
            query_terms("What are you trying to accomplish?"),
            vec!["what", "are", "you", "try", "to", "accomplish"]
        );
    }

    #[test]
    fn filler_only_query_falls_back() {
        assert_eq!(query_terms("AI"), vec!["ai"]);
        assert!(query_terms("   ").is_empty());
    }

    #[test]
    fn stemming_aligns_word_forms() {
        assert!(tokens_match(&stem("servers"), &stem("server")));
        assert!(tokens_match(&stem("debugging"), &stem("debug")));
        assert!(tokens_match(&stem("architecture"), &stem("architect")));
        assert!(tokens_match(&stem("scripts"), &stem("script")));
        assert!(tokens_match(&stem("planning"), &stem("planner")));
        assert!(tokens_match(&stem("classes"), &stem("class")));
        assert!(!tokens_match("mcp", "mcpx"));
        assert_eq!(stem("api"), "api");
    }

    #[test]
    fn tokenizer_folds_case_accents_and_punctuation() {
        assert_eq!(tokenize("Résumé, C++ & Node.js!"), vec!["resume", "c", "node", "js"]);
        assert_eq!(tokenize("日本語 test"), vec!["日本語", "test"]);
    }

    #[test]
    fn fts_query_is_safe_and_prefixed() {
        assert_eq!(fts_query("build an MCP server"), "\"build\"* OR \"mcp\"* OR \"server\"*");
        assert_eq!(fts_query("\"quote\" OR NEAR(x)"), "\"quote\"* OR \"near\"* OR \"x\"");
        assert_eq!(fts_query(""), "");
    }
}

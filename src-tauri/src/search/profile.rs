//! Retrieval profiles: what a Spark "means" for intent search.
//!
//! Embedding the raw body favors whatever happens to be at the top of a long
//! Spark and dilutes its purpose. Instead each Spark is represented by several
//! short *views*:
//!
//! 1. a compact profile — title, summary, purpose (opening instructions), what
//!    it needs from the user (the trailing input placeholder), tags, and the
//!    topics its sections cover (sampled across the *whole* body);
//! 2. up to [`MAX_PASSAGES`] passages spread evenly across the body, each
//!    prefixed with the Spark's identity so it can't be mistaken for another.
//!
//! A 50,000-character Spark and a 1,000-character Spark therefore get the same
//! number and size of views. The full body is untouched: Copy Spark always
//! reads it from the library.
//!
//! Everything here is deterministic; [`PROFILE_VERSION`] is part of every
//! stored embedding's content hash, so changing the recipe re-indexes.

/// Bump when the view recipe changes so stored vectors are re-embedded.
pub const PROFILE_VERSION: &str = "profile-v2";

/// Passages sampled from the body (in addition to the profile view).
pub const MAX_PASSAGES: usize = 4;
const PASSAGE_CHARS: usize = 1_200;
const MAX_TOPICS: usize = 12;
const PURPOSE_CHARS: usize = 320;

/// The fields a profile is built from.
pub struct ProfileSource<'a> {
    pub title: &'a str,
    pub summary: &'a str,
    pub tags: &'a [String],
    pub body: &'a str,
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn char_prefix(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

/// Removes a leading list marker (`- `, `* `, `+ `, `1. `, `12) `).
fn strip_list_marker(s: &str) -> &str {
    if let Some(rest) = s.strip_prefix(['-', '*', '+']) {
        return if rest.starts_with(char::is_whitespace) {
            rest.trim_start()
        } else {
            s
        };
    }
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=3).contains(&digits) {
        if let Some(rest) = s[digits..].strip_prefix(['.', ')']) {
            if rest.starts_with(char::is_whitespace) {
                return rest.trim_start();
            }
        }
    }
    s
}

/// A body line without Markdown decoration.
fn clean_line(line: &str) -> String {
    let mut s = line.trim();
    let hashes = s.chars().take_while(|c| *c == '#').count().min(6);
    if hashes > 0 {
        s = s[hashes..].trim_start();
    }
    if let Some(rest) = s.strip_prefix('>') {
        s = rest.trim_start();
    }
    s = strip_list_marker(s);
    collapse(&s.replace("**", "").replace("__", "").replace('`', ""))
}

fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        if matches!(c, '.' | '!' | '?') && chars.peek().is_some_and(|n| n.is_whitespace()) {
            let s = current.trim().to_string();
            if !s.is_empty() {
                out.push(s);
            }
            current.clear();
        }
    }
    let s = current.trim().to_string();
    if !s.is_empty() {
        out.push(s);
    }
    out
}

/// The short topic at the start of a list item or heading
/// ("Security - authentication..." -> "Security").
fn topic_of(line: &str) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    for p in 3..=60.min(chars.len()) {
        let rest = &chars[p..];
        let dash = rest.len() >= 3
            && rest[0].is_whitespace()
            && matches!(rest[1], '-' | '–' | '—')
            && rest[2].is_whitespace();
        let colon = rest.len() >= 2 && rest[0] == ':' && rest[1].is_whitespace();
        if dash || colon {
            return Some(chars[..p].iter().collect::<String>().trim().to_string());
        }
    }
    if line.split_whitespace().count() <= 7 {
        return Some(line.trim_end_matches([':', '.']).to_string());
    }
    None
}

/// Evenly samples `n` items across `items`, keeping their order.
fn spread<T: Clone>(items: &[T], n: usize) -> Vec<T> {
    if items.len() <= n {
        return items.to_vec();
    }
    let step = items.len() as f64 / n as f64;
    (0..n)
        .map(|i| items[(i as f64 * step) as usize].clone())
        .collect()
}

fn starts_like_list_item(s: &str) -> bool {
    if s.starts_with(['-', '*', '+']) {
        return true;
    }
    let digits = s.bytes().take_while(u8::is_ascii_digit).count();
    if (1..=3).contains(&digits) && s[digits..].starts_with(['.', ')']) {
        return true;
    }
    if let Some(rest) = s.strip_prefix("Phase") {
        let rest_trim = rest.trim_start();
        return rest.len() != rest_trim.len()
            && rest_trim.starts_with(|c: char| c.is_ascii_digit());
    }
    false
}

/// Drops outline numbering such as "01.02 [014] ".
fn strip_outline_number(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut i = 0;
    let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    if !(1..=3).contains(&digits) {
        return s;
    }
    i += digits;
    while i < bytes.len() && bytes[i] == b'.' {
        let more = bytes[i + 1..]
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .count();
        if !(1..=3).contains(&more) {
            break;
        }
        i += 1 + more;
    }
    let mut rest = s[i..].trim_start();
    if let Some(inner) = rest.strip_prefix('[') {
        let d = inner.bytes().take_while(u8::is_ascii_digit).count();
        if d > 0 && inner[d..].starts_with(']') {
            rest = inner[d + 1..].trim_start();
        }
    }
    rest
}

/// Headings and list-item topics across the whole body.
pub fn section_topics(body: &str) -> Vec<String> {
    let mut topics: Vec<String> = Vec::new();
    for raw in body.lines() {
        let s = raw.trim();
        if s.is_empty() || !(s.starts_with('#') || starts_like_list_item(s)) {
            continue;
        }
        let cleaned = clean_line(s);
        let Some(topic) = topic_of(strip_outline_number(&cleaned)) else {
            continue;
        };
        if topic.chars().count() >= 3
            && !topics
                .iter()
                .any(|t| t.to_lowercase() == topic.to_lowercase())
        {
            topics.push(topic);
        }
    }
    spread(&topics, MAX_TOPICS)
}

/// The opening instructions (purpose) and the trailing input the Spark asks for.
pub fn purpose_and_needs(body: &str) -> (String, Vec<String>) {
    let lines: Vec<String> = body
        .lines()
        .map(clean_line)
        .filter(|l| !l.is_empty())
        .collect();
    let mut opening: Vec<String> = Vec::new();
    for l in &lines {
        let label_only = l.ends_with(':') && l.split_whitespace().count() <= 6;
        if l.starts_with('[') || label_only {
            continue;
        }
        opening.extend(sentences(l));
        if opening.len() >= 2 {
            break;
        }
    }
    opening.truncate(2);
    let purpose = char_prefix(&opening.join(" "), PURPOSE_CHARS).to_string();

    let mut needs = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if l.starts_with('[') && l.ends_with(']') {
            let label = match i.checked_sub(1).map(|j| &lines[j]) {
                Some(prev) if prev.ends_with(':') => prev.as_str(),
                _ => "",
            };
            needs.push(
                format!("{label} {}", l.trim_matches(['[', ']']))
                    .trim()
                    .to_string(),
            );
        }
    }
    let keep = needs.len().saturating_sub(2);
    (purpose, needs.split_off(keep))
}

/// The compact profile view.
pub fn profile_text(src: &ProfileSource) -> String {
    let (purpose, needs) = purpose_and_needs(src.body);
    let topics = section_topics(src.body);
    let mut parts = vec![src.title.to_string()];
    if !src.summary.is_empty() {
        parts.push(src.summary.to_string());
    }
    parts.push(format!("Purpose: {purpose}"));
    if !needs.is_empty() {
        parts.push(format!("Needs: {}", needs.join("; ")));
    }
    if !src.tags.is_empty() {
        parts.push(format!("Tags: {}", src.tags.join(", ")));
    }
    if !topics.is_empty() {
        parts.push(format!("Covers: {}", topics.join("; ")));
    }
    parts.join("\n")
}

/// Splits the body into paragraph-aligned passages of about
/// [`PASSAGE_CHARS`] and samples at most [`MAX_PASSAGES`] across its length.
pub fn passages(body: &str) -> Vec<String> {
    let mut paragraphs: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in body.split('\n') {
        if line.trim().is_empty() {
            if !current.trim().is_empty() {
                paragraphs.push(current.trim().to_string());
            }
            current.clear();
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }
    }
    if !current.trim().is_empty() {
        paragraphs.push(current.trim().to_string());
    }

    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for p in paragraphs {
        if !cur.is_empty() && cur.chars().count() + p.chars().count() + 2 > PASSAGE_CHARS {
            out.push(std::mem::take(&mut cur));
        }
        cur = if cur.is_empty() {
            p
        } else {
            format!("{cur}\n\n{p}").trim().to_string()
        };
        while cur.chars().count() > PASSAGE_CHARS * 3 / 2 {
            let head = char_prefix(&cur, PASSAGE_CHARS).to_string();
            cur = cur[head.len()..].to_string();
            out.push(head);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    spread(&out, MAX_PASSAGES)
}

/// All views embedded for one Spark: the profile first, then passages.
pub fn document_views(src: &ProfileSource) -> Vec<String> {
    let identity = format!(
        "{}\n{}\nTags: {}",
        src.title,
        src.summary,
        src.tags.join(", ")
    );
    let mut views = vec![profile_text(src)];
    views.extend(
        passages(src.body)
            .into_iter()
            .map(|p| format!("{identity}\n\n{p}")),
    );
    views
}

/// Conversational openers that carry no intent ("I need AI to", "Help me").
/// Stripped before embedding the goal; the rest of the sentence is kept verbatim.
pub fn clean_goal(goal: &str) -> String {
    let original = goal.trim();
    let mut rest = original;
    loop {
        let before = rest;
        rest = strip_word(rest, "please").unwrap_or(rest);
        if let Some(r) = strip_i_need(rest) {
            rest = r;
        } else if let Some(r) = strip_phrase(rest, &["help", "me"]) {
            rest = strip_word(r, "to").unwrap_or(r);
        } else if let Some(r) = strip_phrase(rest, &["can", "you"]) {
            rest = r;
        } else if let Some(r) = strip_phrase(rest, &["i'm", "trying", "to"])
            .or_else(|| strip_phrase(rest, &["im", "trying", "to"]))
        {
            rest = r;
        }
        if rest == before {
            break;
        }
    }
    if rest.is_empty() {
        return original.to_string();
    }
    let mut chars = rest.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => original.to_string(),
    }
}

/// Strips `word` plus following whitespace, case-insensitively.
fn strip_word<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    let head = s.get(..word.len())?;
    if !head.eq_ignore_ascii_case(word) {
        return None;
    }
    let rest = &s[word.len()..];
    let trimmed = rest.trim_start();
    (trimmed.len() < rest.len()).then_some(trimmed)
}

fn strip_phrase<'a>(s: &'a str, words: &[&str]) -> Option<&'a str> {
    words.iter().try_fold(s, |acc, w| strip_word(acc, w))
}

/// "i (really)? (need|want|would like) (a|an)? (ai)? (to)?"
fn strip_i_need(s: &str) -> Option<&str> {
    let mut rest = strip_word(s, "i")?;
    rest = strip_word(rest, "really").unwrap_or(rest);
    rest = strip_word(rest, "need")
        .or_else(|| strip_word(rest, "want"))
        .or_else(|| strip_phrase(rest, &["would", "like"]))?;
    rest = strip_word(rest, "an")
        .or_else(|| strip_word(rest, "a"))
        .unwrap_or(rest);
    rest = strip_word(rest, "ai").unwrap_or(rest);
    rest = strip_word(rest, "to").unwrap_or(rest);
    Some(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MCP_BODY: &str = "You are an expert in the Model Context Protocol (MCP) and production backend engineering. Help me design and build an MCP server.\n\nFirst, ask me up to 5 clarifying questions if the purpose is unclear. Then deliver:\n\n1. Purpose & capabilities - what the server exposes and to which MCP clients.\n2. Primitive design\n   - Tools: name, description written for the model.\n   - Resources: URIs, content types.\n3. Transport - stdio vs. streamable HTTP, with the reasoning.\n\nWhat the server should do:\n[Describe the server's purpose and data sources]";

    fn src<'a>(title: &'a str, tags: &'a [String], body: &'a str) -> ProfileSource<'a> {
        ProfileSource {
            title,
            summary: "Design and build MCP servers.",
            tags,
            body,
        }
    }

    #[test]
    fn profile_captures_purpose_needs_and_topics() {
        let tags = vec!["MCP".to_string(), "Architecture".to_string()];
        let p = profile_text(&src("MCP Server Architect", &tags, MCP_BODY));
        assert!(p.starts_with("MCP Server Architect\nDesign and build MCP servers."));
        assert!(p.contains("Purpose: You are an expert in the Model Context Protocol (MCP) and production backend engineering. Help me design and build an MCP server."));
        assert!(p.contains(
            "Needs: What the server should do: Describe the server's purpose and data sources"
        ));
        assert!(p.contains("Tags: MCP, Architecture"));
        assert!(p.contains(
            "Covers: Purpose & capabilities; Primitive design; Tools; Resources; Transport"
        ));
    }

    #[test]
    fn long_and_short_sparks_get_comparable_views() {
        let tags: Vec<String> = vec![];
        let short = document_views(&src("Short", &tags, "Do one thing well.\n\n[Paste input]"));
        let long_body: String = (0..400)
            .map(|i| format!("{:02}.{:02} [{i:03}] Check item {i} - verify item number {i} carefully and cite evidence.\n", i / 12, i % 12))
            .collect::<Vec<_>>()
            .join("\n")
            + "\nEND-MARKER";
        let long = document_views(&src("Long", &tags, &long_body));
        assert_eq!(short.len(), 2);
        assert_eq!(
            long.len(),
            1 + MAX_PASSAGES,
            "long Sparks are sampled, not truncated"
        );
        for v in &long {
            assert!(v.chars().count() < 2_400, "every view stays compact");
        }
        // Passages span the body: the last sampled passage comes from the end half.
        let last = long.last().unwrap();
        let n: usize = last
            .split("item number ")
            .nth(1)
            .and_then(|s| s.split_whitespace().next())
            .and_then(|s| s.parse().ok())
            .unwrap();
        assert!(
            n > 200,
            "last passage should come from late in the body (got item {n})"
        );
        // Outline numbering is dropped and topics are sampled across the body.
        let profile = &long[0];
        assert!(profile.contains("Covers: Check item 0; "), "{profile}");
        assert!(
            profile.contains("Check item 366"),
            "late sections are represented: {profile}"
        );
    }

    #[test]
    fn clean_goal_strips_conversational_openers_only() {
        assert_eq!(
            clean_goal("I need AI to teach me how to create tools and resources"),
            "Teach me how to create tools and resources"
        );
        assert_eq!(
            clean_goal("Help me teach an autonomous agent a new reusable capability"),
            "Teach an autonomous agent a new reusable capability"
        );
        assert_eq!(
            clean_goal("I want an AI to inspect the structure of my application"),
            "Inspect the structure of my application"
        );
        assert_eq!(
            clean_goal("please help me to plan a launch"),
            "Plan a launch"
        );
        assert_eq!(clean_goal("my app keeps crashing"), "My app keeps crashing");
        assert_eq!(clean_goal("I need AI"), "AI");
        assert_eq!(
            clean_goal("help me"),
            "Help me",
            "an opener with nothing after it is kept"
        );
        assert_eq!(clean_goal("  "), "");
    }

    #[test]
    fn topics_parse_dashes_and_colons() {
        assert_eq!(
            topic_of("Security - auth and secrets").as_deref(),
            Some("Security")
        );
        assert_eq!(topic_of("Tools: name, schema").as_deref(), Some("Tools"));
        assert_eq!(topic_of("Short heading:").as_deref(), Some("Short heading"));
        assert_eq!(
            topic_of("this is a long sentence without any separator at all here"),
            None
        );
    }
}

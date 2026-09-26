//! Spark repository: validation, CRUD, favorites and copy usage metadata.
//!
//! Every write keeps the full-text index in the same transaction and drops
//! stale embeddings so retrieval never serves outdated content.

pub mod seed;

use rusqlite::{params, params_from_iter, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::storage::{content_hash, now_ms, Library};

pub const MAX_TITLE_CHARS: usize = 120;
pub const MAX_SUMMARY_CHARS: usize = 600;
pub const MAX_BODY_CHARS: usize = 1_000_000;
pub const MAX_TAGS: usize = 8;
pub const MAX_TAG_CHARS: usize = 32;
pub const MAX_NOTE_CHARS: usize = 500;

/// Data submitted by Add New Spark / Edit Spark.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SparkInput {
    pub title: String,
    pub summary: String,
    pub body: String,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub source_note: Option<String>,
    /// Save even when an identical Spark body already exists.
    pub allow_duplicate: bool,
}

/// What the sidebar needs to render a Spark. Never includes the body.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SparkSummary {
    pub id: i64,
    pub title: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub favorite: bool,
    pub usage_count: i64,
}

/// Full Spark, used only by the editor.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SparkDetail {
    #[serde(flatten)]
    pub summary: SparkSummary,
    pub body: String,
    pub source_note: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_copied_at: Option<i64>,
}

/// Internal row used by the search service.
#[derive(Debug, Clone)]
pub struct SearchDoc {
    pub id: i64,
    pub title: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub body: String,
    pub favorite: bool,
    pub usage_count: i64,
    pub last_copied_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
struct CleanInput {
    title: String,
    summary: String,
    body: String,
    body_hash: String,
    tags: Vec<String>,
    favorite: bool,
    source_note: Option<String>,
}

fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_words(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let cut: String = s.chars().take(max_chars).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > max_chars / 2 => cut[..i].to_string(),
        _ => cut,
    };
    format!(
        "{}…",
        cut.trim_end_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace())
    )
}

const MAX_DERIVED_TITLE: usize = 60;

/// Markdown headings are structure (a title), not summary prose.
fn is_heading(line: &str) -> bool {
    line.trim_start().starts_with('#')
}

fn strip_markers(line: &str) -> &str {
    line.trim()
        .trim_start_matches(['#', '*', '>', '-', '=', '_', '`'])
        .trim()
}

/// The first sentence of a line, including its closing punctuation.
fn first_sentence(line: &str) -> &str {
    for (i, c) in line.char_indices() {
        if matches!(c, '.' | '!' | '?') {
            let end = i + c.len_utf8();
            if line[end..].chars().next().map_or(true, char::is_whitespace) {
                return &line[..end];
            }
        }
    }
    line
}

/// Capitalises each word (keeping any capitals already there, e.g.
/// "YouTube", "MCP"); short joining words stay lowercase.
fn title_case(words: &[&str]) -> String {
    const SMALL: &[&str] = &[
        "a", "an", "and", "as", "at", "by", "for", "in", "of", "on", "or", "the", "to", "with",
    ];
    words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if i > 0 && SMALL.contains(w) {
                return (*w).to_string();
            }
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Titles a role-style opening: "You are a database performance engineer.
/// Diagnose…" -> "Database Performance Engineer".
fn role_title(sentence: &str) -> Option<String> {
    const PREAMBLES: &[&str] = &[
        "you are ",
        "you're ",
        "act as ",
        "acting as ",
        "pretend you are ",
        "imagine you are ",
        "behave as ",
        "serve as ",
    ];
    const ARTICLES: &[&str] = &["a ", "an ", "the ", "my "];
    const CLAUSE_BREAKS: &[&str] = &[
        " who ",
        " that ",
        " which ",
        " working ",
        " with ",
        " specializing ",
        " specialising ",
        " helping ",
        " focused ",
        " known ",
        " and ",
        " to ",
        " for ",
    ];
    // ASCII lowercasing keeps byte offsets aligned with `sentence`.
    let lower = sentence.to_ascii_lowercase();
    let preamble = PREAMBLES.iter().find(|p| lower.starts_with(**p))?;
    let mut start = preamble.len();
    if let Some(article) = ARTICLES.iter().find(|a| lower[start..].starts_with(**a)) {
        start += article.len();
    }
    let rest = &lower[start..];
    let mut end = rest
        .find(['.', ',', ';', ':', '(', '!', '?', '—', '–'])
        .unwrap_or(rest.len());
    for clause in CLAUSE_BREAKS {
        if let Some(i) = rest.find(clause) {
            end = end.min(i);
        }
    }
    let words: Vec<&str> = sentence[start..start + end].split_whitespace().collect();
    // One word ("an assistant") says too little; many words aren't a title.
    (2..=6).contains(&words.len()).then(|| title_case(&words))
}

fn fit_title(text: &str) -> String {
    let text = text.trim_end_matches(['.', ':', '*', '#', '`']).trim();
    let title = truncate_words(&collapse_whitespace(text), MAX_DERIVED_TITLE);
    if title.is_empty() {
        "Untitled Spark".into()
    } else {
        title
    }
}

/// Derives a title from the start of a Spark body, and returns the text a
/// derived summary should draw from (whatever the title didn't use).
fn derive_lead(body: &str) -> (String, String) {
    let mut lines = body.lines();
    let Some(first) = lines.by_ref().find(|l| !strip_markers(l).is_empty()) else {
        return ("Untitled Spark".into(), String::new());
    };
    let rest = lines.collect::<Vec<_>>().join("\n");
    if is_heading(first) {
        return (fit_title(strip_markers(first)), rest);
    }
    let line = strip_markers(first);
    let sentence = first_sentence(line);
    let after = format!("{}\n{rest}", &line[sentence.len()..]);
    if let Some(title) = role_title(sentence) {
        return (title, after);
    }
    if sentence.chars().count() <= MAX_DERIVED_TITLE + 1 {
        return (fit_title(sentence), after);
    }
    // Too long to be a title: shortened, and the summary keeps all of it.
    (fit_title(sentence), format!("{line}\n{rest}"))
}

/// Derives a readable title from the start of a Spark body.
pub fn derive_title(body: &str) -> String {
    derive_lead(body).0
}

/// Derives a short summary from the body when the user provided none. When
/// the title was derived too, the summary starts after what the title used.
pub fn derive_summary(body: &str, title_was_derived: bool) -> String {
    let text = if title_was_derived {
        derive_lead(body).1
    } else {
        body.to_string()
    };
    let prose: Vec<&str> = text.lines().filter(|l| !is_heading(l)).collect();
    let cleaned: String = collapse_whitespace(&prose.join(" ").replace(['#', '*', '`', '>'], " "));
    if cleaned.is_empty() {
        return String::new();
    }
    // Prefer ending on a sentence boundary within the budget.
    let budget = 180;
    if cleaned.chars().count() <= budget {
        return cleaned;
    }
    let head: String = cleaned.chars().take(budget).collect();
    if let Some(end) = head.rfind(['.', '!', '?']) {
        if end > 60 {
            return head[..=end].to_string();
        }
    }
    truncate_words(&cleaned, budget)
}

/// Normalises tags: trims, strips `#`, collapses whitespace, de-duplicates
/// case-insensitively and caps count/length.
pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in tags {
        for piece in raw.split(',') {
            let tag = collapse_whitespace(piece.trim().trim_start_matches('#'));
            if tag.is_empty() {
                continue;
            }
            let tag: String = tag.chars().take(MAX_TAG_CHARS).collect();
            if out.iter().any(|t| t.eq_ignore_ascii_case(&tag)) {
                continue;
            }
            out.push(tag);
            if out.len() == MAX_TAGS {
                return out;
            }
        }
    }
    out
}

fn body_hash(body: &str) -> String {
    content_hash(&[&collapse_whitespace(body)])
}

fn clean(input: SparkInput) -> AppResult<CleanInput> {
    let body = input.body.trim().to_string();
    if body.is_empty() {
        return Err(AppError::Validation(
            "Paste or type the Spark itself before saving.".into(),
        ));
    }
    if body.chars().count() > MAX_BODY_CHARS {
        return Err(AppError::Validation(format!(
            "This Spark is longer than {} characters. Split it into smaller Sparks.",
            MAX_BODY_CHARS
        )));
    }

    let typed_title = collapse_whitespace(&input.title);
    if typed_title.chars().count() > MAX_TITLE_CHARS {
        return Err(AppError::Validation(format!(
            "Keep the title under {MAX_TITLE_CHARS} characters."
        )));
    }
    let title_was_derived = typed_title.is_empty();
    let title = if title_was_derived {
        derive_title(&body)
    } else {
        typed_title
    };

    let typed_summary = collapse_whitespace(&input.summary);
    if typed_summary.chars().count() > MAX_SUMMARY_CHARS {
        return Err(AppError::Validation(format!(
            "Keep the summary under {MAX_SUMMARY_CHARS} characters."
        )));
    }
    let summary = if typed_summary.is_empty() {
        derive_summary(&body, title_was_derived)
    } else {
        typed_summary
    };

    let source_note = input
        .source_note
        .map(|n| n.trim().chars().take(MAX_NOTE_CHARS).collect::<String>())
        .filter(|n| !n.is_empty());

    Ok(CleanInput {
        body_hash: body_hash(&body),
        title,
        summary,
        body,
        tags: normalize_tags(&input.tags),
        favorite: input.favorite,
        source_note,
    })
}

fn find_duplicate(
    lib: &Library,
    hash: &str,
    exclude_id: Option<i64>,
) -> AppResult<Option<(i64, String)>> {
    Ok(lib
        .conn
        .query_row(
            "SELECT id, title FROM sparks WHERE body_hash = ?1 AND id != ?2 ORDER BY id LIMIT 1",
            params![hash, exclude_id.unwrap_or(-1)],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

fn write_tags(tx: &Transaction, spark_id: i64, tags: &[String]) -> AppResult<()> {
    tx.execute("DELETE FROM spark_tags WHERE spark_id = ?1", [spark_id])?;
    for (position, tag) in tags.iter().enumerate() {
        tx.execute(
            "INSERT INTO tags (name) VALUES (?1) ON CONFLICT(name) DO NOTHING",
            [tag],
        )?;
        let tag_id: i64 =
            tx.query_row("SELECT id FROM tags WHERE name = ?1", [tag], |r| r.get(0))?;
        tx.execute(
            "INSERT INTO spark_tags (spark_id, tag_id, position) VALUES (?1, ?2, ?3)",
            params![spark_id, tag_id, position as i64],
        )?;
    }
    tx.execute(
        "DELETE FROM tags WHERE id NOT IN (SELECT DISTINCT tag_id FROM spark_tags)",
        [],
    )?;
    Ok(())
}

fn write_fts(tx: &Transaction, spark_id: i64, c: &CleanInput) -> AppResult<()> {
    tx.execute("DELETE FROM sparks_fts WHERE rowid = ?1", [spark_id])?;
    tx.execute(
        "INSERT INTO sparks_fts (rowid, title, summary, tags, body) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![spark_id, c.title, c.summary, c.tags.join(" "), c.body],
    )?;
    Ok(())
}

pub fn create(lib: &mut Library, input: SparkInput) -> AppResult<SparkSummary> {
    let allow_duplicate = input.allow_duplicate;
    let c = clean(input)?;
    if !allow_duplicate {
        if let Some((existing_id, existing_title)) = find_duplicate(lib, &c.body_hash, None)? {
            return Err(AppError::Duplicate {
                existing_id,
                existing_title,
            });
        }
    }
    let now = now_ms();
    let tx = lib.conn.transaction()?;
    tx.execute(
        "INSERT INTO sparks (title, summary, body, body_hash, favorite, favorited_at,
                             created_at, updated_at, source_note)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8)",
        params![
            c.title,
            c.summary,
            c.body,
            c.body_hash,
            c.favorite,
            c.favorite.then_some(now),
            now,
            c.source_note
        ],
    )?;
    let id = tx.last_insert_rowid();
    write_tags(&tx, id, &c.tags)?;
    write_fts(&tx, id, &c)?;
    tx.commit()?;
    get_summary(lib, id)
}

pub fn update(lib: &mut Library, id: i64, input: SparkInput) -> AppResult<SparkSummary> {
    let allow_duplicate = input.allow_duplicate;
    let c = clean(input)?;
    let existing: Option<bool> = lib
        .conn
        .query_row("SELECT favorite FROM sparks WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    let was_favorite = existing.ok_or(AppError::NotFound)?;
    if !allow_duplicate {
        if let Some((existing_id, existing_title)) = find_duplicate(lib, &c.body_hash, Some(id))? {
            return Err(AppError::Duplicate {
                existing_id,
                existing_title,
            });
        }
    }
    let now = now_ms();
    let tx = lib.conn.transaction()?;
    tx.execute(
        "UPDATE sparks SET title = ?2, summary = ?3, body = ?4, body_hash = ?5,
                favorite = ?6,
                favorited_at = CASE WHEN ?6 = 0 THEN NULL
                                    WHEN ?7 = 1 THEN favorited_at ELSE ?8 END,
                updated_at = ?8, source_note = ?9
         WHERE id = ?1",
        params![
            id,
            c.title,
            c.summary,
            c.body,
            c.body_hash,
            c.favorite,
            was_favorite,
            now,
            c.source_note
        ],
    )?;
    write_tags(&tx, id, &c.tags)?;
    write_fts(&tx, id, &c)?;
    // Content changed: embeddings for every model are now stale.
    tx.execute("DELETE FROM embeddings WHERE spark_id = ?1", [id])?;
    tx.commit()?;
    get_summary(lib, id)
}

pub fn delete(lib: &mut Library, id: i64) -> AppResult<()> {
    let tx = lib.conn.transaction()?;
    let n = tx.execute("DELETE FROM sparks WHERE id = ?1", [id])?;
    if n == 0 {
        return Err(AppError::NotFound);
    }
    tx.execute("DELETE FROM sparks_fts WHERE rowid = ?1", [id])?;
    tx.execute(
        "DELETE FROM tags WHERE id NOT IN (SELECT DISTINCT tag_id FROM spark_tags)",
        [],
    )?;
    tx.commit()?;
    Ok(())
}

pub fn set_favorite(lib: &mut Library, id: i64, favorite: bool) -> AppResult<SparkSummary> {
    let n = lib.conn.execute(
        "UPDATE sparks SET favorite = ?2,
                favorited_at = CASE WHEN ?2 = 0 THEN NULL
                                    ELSE COALESCE(favorited_at, ?3) END
         WHERE id = ?1",
        params![id, favorite, now_ms()],
    )?;
    if n == 0 {
        return Err(AppError::NotFound);
    }
    get_summary(lib, id)
}

/// Returns the complete body for copying and records the usage.
pub fn record_copy(lib: &mut Library, id: i64) -> AppResult<(String, String)> {
    let tx = lib.conn.transaction()?;
    let row: Option<(String, String)> = tx
        .query_row("SELECT title, body FROM sparks WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?;
    let (title, body) = row.ok_or(AppError::NotFound)?;
    tx.execute(
        "UPDATE sparks SET usage_count = usage_count + 1, last_copied_at = ?2 WHERE id = ?1",
        params![id, now_ms()],
    )?;
    tx.commit()?;
    Ok((title, body))
}

/// Reads the body without recording usage.
pub fn body(lib: &Library, id: i64) -> AppResult<(String, String)> {
    lib.conn
        .query_row("SELECT title, body FROM sparks WHERE id = ?1", [id], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .optional()?
        .ok_or(AppError::NotFound)
}

fn tags_for(lib: &Library, id: i64) -> AppResult<Vec<String>> {
    let mut stmt = lib.conn.prepare_cached(
        "SELECT t.name FROM spark_tags st JOIN tags t ON t.id = st.tag_id
         WHERE st.spark_id = ?1 ORDER BY st.position",
    )?;
    let tags = stmt
        .query_map([id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<String>>>()?;
    Ok(tags)
}

pub fn get_summary(lib: &Library, id: i64) -> AppResult<SparkSummary> {
    let row = lib
        .conn
        .query_row(
            "SELECT id, title, summary, favorite, usage_count FROM sparks WHERE id = ?1",
            [id],
            |r| {
                Ok(SparkSummary {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    summary: r.get(2)?,
                    tags: Vec::new(),
                    favorite: r.get(3)?,
                    usage_count: r.get(4)?,
                })
            },
        )
        .optional()?;
    let mut s = row.ok_or(AppError::NotFound)?;
    s.tags = tags_for(lib, id)?;
    Ok(s)
}

pub fn get_detail(lib: &Library, id: i64) -> AppResult<SparkDetail> {
    let summary = get_summary(lib, id)?;
    lib.conn
        .query_row(
            "SELECT body, source_note, created_at, updated_at, last_copied_at FROM sparks WHERE id = ?1",
            [id],
            |r| {
                Ok(SparkDetail {
                    summary: summary.clone(),
                    body: r.get(0)?,
                    source_note: r.get(1)?,
                    created_at: r.get(2)?,
                    updated_at: r.get(3)?,
                    last_copied_at: r.get(4)?,
                })
            },
        )
        .map_err(Into::into)
}

/// Favorites in the order they were favorited, so rows never jump around.
pub fn list_favorites(lib: &Library) -> AppResult<Vec<SparkSummary>> {
    let ids: Vec<i64> = {
        let mut stmt = lib
            .conn
            .prepare_cached("SELECT id FROM sparks WHERE favorite = 1 ORDER BY favorited_at, id")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    ids.into_iter().map(|id| get_summary(lib, id)).collect()
}

/// Loads search documents for the given ids (order not guaranteed).
pub fn search_docs(lib: &Library, ids: &[i64]) -> AppResult<Vec<SearchDoc>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!(
        "SELECT id, title, summary, body, favorite, usage_count, last_copied_at
         FROM sparks WHERE id IN ({placeholders})"
    );
    let mut stmt = lib.conn.prepare(&sql)?;
    let rows = stmt
        .query_map(params_from_iter(ids.iter()), |r| {
            Ok(SearchDoc {
                id: r.get(0)?,
                title: r.get(1)?,
                summary: r.get(2)?,
                tags: Vec::new(),
                body: r.get(3)?,
                favorite: r.get(4)?,
                usage_count: r.get(5)?,
                last_copied_at: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    rows.into_iter()
        .map(|mut d| {
            d.tags = tags_for(lib, d.id)?;
            Ok(d)
        })
        .collect()
}

pub fn all_ids(lib: &Library) -> AppResult<Vec<i64>> {
    let mut stmt = lib
        .conn
        .prepare_cached("SELECT id FROM sparks ORDER BY id")?;
    let ids = stmt
        .query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

/// Number of Sparks whose indexed text contains `term` (prefix match). Used to
/// weight query words by how rare they are in this library.
pub fn document_frequency(lib: &Library, term: &str) -> AppResult<i64> {
    let token: String = term.chars().filter(|c| c.is_alphanumeric()).collect();
    if token.is_empty() {
        return Ok(0);
    }
    // Alphanumeric only, so quoting is injection-safe.
    let expr = if token.chars().count() >= 3 {
        format!("\"{token}\"*")
    } else {
        format!("\"{token}\"")
    };
    let mut stmt = lib
        .conn
        .prepare_cached("SELECT COUNT(*) FROM sparks_fts WHERE sparks_fts MATCH ?1")?;
    Ok(stmt.query_row([expr], |r| r.get(0))?)
}

/// Full-text candidates as (id, bm25) where lower bm25 is better.
pub fn fts_candidates(lib: &Library, fts_query: &str, limit: usize) -> AppResult<Vec<(i64, f64)>> {
    if fts_query.is_empty() {
        return Ok(Vec::new());
    }
    let mut stmt = lib.conn.prepare_cached(
        "SELECT rowid, bm25(sparks_fts, 10.0, 4.0, 6.0, 1.0) AS rank
         FROM sparks_fts WHERE sparks_fts MATCH ?1 ORDER BY rank LIMIT ?2",
    )?;
    let rows = stmt
        .query_map(params![fts_query, limit as i64], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(title: &str, body: &str) -> SparkInput {
        SparkInput {
            title: title.into(),
            body: body.into(),
            ..Default::default()
        }
    }

    #[test]
    fn create_and_read_back() {
        let mut lib = Library::open_in_memory();
        let s = create(
            &mut lib,
            SparkInput {
                title: "  MCP   Server Architect ".into(),
                summary: "Design MCP servers.".into(),
                body: "\n\nYou are an MCP expert.\n".into(),
                tags: vec!["MCP".into(), "#architecture, mcp".into(), " ".into()],
                favorite: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(s.title, "MCP Server Architect");
        assert_eq!(s.tags, vec!["MCP", "architecture"]);
        assert!(s.favorite);
        let d = get_detail(&lib, s.id).unwrap();
        assert_eq!(d.body, "You are an MCP expert.");
        assert_eq!(list_favorites(&lib).unwrap().len(), 1);
    }

    #[test]
    fn empty_body_is_rejected() {
        let mut lib = Library::open_in_memory();
        let err = create(&mut lib, input("Title", "   \n ")).err().unwrap();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn overlong_title_is_rejected() {
        let mut lib = Library::open_in_memory();
        let err = create(&mut lib, input(&"x".repeat(MAX_TITLE_CHARS + 1), "body"))
            .err()
            .unwrap();
        assert!(matches!(err, AppError::Validation(_)));
    }

    #[test]
    fn missing_title_and_summary_are_derived() {
        let mut lib = Library::open_in_memory();
        let s = create(
            &mut lib,
            input(
                "",
                "## Deep Research Protocol:\nInvestigate the topic thoroughly. Cite sources.",
            ),
        )
        .unwrap();
        assert_eq!(s.title, "Deep Research Protocol");
        assert_eq!(s.summary, "Investigate the topic thoroughly. Cite sources.");
    }

    #[test]
    fn very_long_body_round_trips_exactly() {
        let mut lib = Library::open_in_memory();
        let body: String = (0..20_000)
            .map(|i| format!("Line {i}: do the thing carefully.\n"))
            .collect();
        let s = create(&mut lib, input("Long", &body)).unwrap();
        let (_, copied) = record_copy(&mut lib, s.id).unwrap();
        assert_eq!(copied, body.trim());
        assert!(get_summary(&lib, s.id).unwrap().summary.chars().count() <= 181);
    }

    #[test]
    fn duplicates_are_detected_unless_allowed() {
        let mut lib = Library::open_in_memory();
        let a = create(&mut lib, input("A", "Same   body\ntext")).unwrap();
        let err = create(&mut lib, input("B", "Same body text"))
            .err()
            .unwrap();
        match err {
            AppError::Duplicate { existing_id, .. } => assert_eq!(existing_id, a.id),
            other => panic!("expected duplicate, got {other:?}"),
        }
        let mut dup = input("B", "Same body text");
        dup.allow_duplicate = true;
        assert!(create(&mut lib, dup).is_ok());
    }

    #[test]
    fn update_changes_content_and_invalidates_embeddings() {
        let mut lib = Library::open_in_memory();
        let s = create(&mut lib, input("Old", "old body")).unwrap();
        lib.conn
            .execute(
                "INSERT INTO embeddings VALUES (?1, 'm', 1, x'0000803f', 'h', 0)",
                [s.id],
            )
            .unwrap();
        let mut next = input("New", "new body");
        next.tags = vec!["fresh".into()];
        let u = update(&mut lib, s.id, next).unwrap();
        assert_eq!(u.title, "New");
        assert_eq!(u.tags, vec!["fresh"]);
        let n: i64 = lib
            .conn
            .query_row("SELECT COUNT(*) FROM embeddings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        let hits = fts_candidates(&lib, "\"new\"*", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert!(fts_candidates(&lib, "\"old\"", 10).unwrap().is_empty());
    }

    #[test]
    fn update_missing_spark_is_not_found() {
        let mut lib = Library::open_in_memory();
        assert!(matches!(
            update(&mut lib, 99, input("x", "y")),
            Err(AppError::NotFound)
        ));
    }

    #[test]
    fn update_to_same_body_is_not_a_duplicate_of_itself() {
        let mut lib = Library::open_in_memory();
        let s = create(&mut lib, input("A", "body")).unwrap();
        assert!(update(&mut lib, s.id, input("A2", "body")).is_ok());
    }

    #[test]
    fn delete_removes_everything() {
        let mut lib = Library::open_in_memory();
        let mut i = input("T", "b");
        i.tags = vec!["only".into()];
        let s = create(&mut lib, i).unwrap();
        delete(&mut lib, s.id).unwrap();
        assert!(matches!(get_summary(&lib, s.id), Err(AppError::NotFound)));
        let tags: i64 = lib
            .conn
            .query_row("SELECT COUNT(*) FROM tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tags, 0);
        let fts: i64 = lib
            .conn
            .query_row("SELECT COUNT(*) FROM sparks_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts, 0);
        assert!(matches!(delete(&mut lib, s.id), Err(AppError::NotFound)));
    }

    #[test]
    fn favorites_keep_their_order() {
        let mut lib = Library::open_in_memory();
        let a = create(&mut lib, input("A", "a")).unwrap();
        let b = create(&mut lib, input("B", "b")).unwrap();
        set_favorite(&mut lib, b.id, true).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(3));
        set_favorite(&mut lib, a.id, true).unwrap();
        let favs: Vec<i64> = list_favorites(&lib).unwrap().iter().map(|s| s.id).collect();
        assert_eq!(favs, vec![b.id, a.id]);
        // Re-favoriting an existing favorite keeps its position.
        set_favorite(&mut lib, b.id, true).unwrap();
        let favs: Vec<i64> = list_favorites(&lib).unwrap().iter().map(|s| s.id).collect();
        assert_eq!(favs, vec![b.id, a.id]);
        set_favorite(&mut lib, b.id, false).unwrap();
        assert_eq!(list_favorites(&lib).unwrap().len(), 1);
    }

    #[test]
    fn copy_records_usage() {
        let mut lib = Library::open_in_memory();
        let s = create(&mut lib, input("A", "full body")).unwrap();
        let (_, body) = record_copy(&mut lib, s.id).unwrap();
        assert_eq!(body, "full body");
        record_copy(&mut lib, s.id).unwrap();
        assert_eq!(get_summary(&lib, s.id).unwrap().usage_count, 2);
        assert!(get_detail(&lib, s.id).unwrap().last_copied_at.is_some());
        assert!(matches!(
            record_copy(&mut lib, 999),
            Err(AppError::NotFound)
        ));
    }

    #[test]
    fn tag_normalisation() {
        let tags = normalize_tags(&[
            "#Rust".into(),
            "rust".into(),
            "a,b".into(),
            "  spaced   tag ".into(),
            "x".repeat(50),
        ]);
        assert_eq!(tags[0], "Rust");
        assert_eq!(tags[1], "a");
        assert_eq!(tags[2], "b");
        assert_eq!(tags[3], "spaced tag");
        assert_eq!(tags[4].chars().count(), MAX_TAG_CHARS);
        let many: Vec<String> = (0..20).map(|i| format!("t{i}")).collect();
        assert_eq!(normalize_tags(&many).len(), MAX_TAGS);
    }

    #[test]
    fn derive_title_handles_markdown_and_length() {
        assert_eq!(
            derive_title("# Role: Senior Engineer\nbody"),
            "Role: Senior Engineer"
        );
        assert_eq!(derive_title("\n\n   \n"), "Untitled Spark");
        let long = derive_title(&"word ".repeat(40));
        assert!(long.ends_with('…'));
        assert!(long.chars().count() <= 61);
    }

    #[test]
    fn derived_titles_name_the_role_instead_of_cutting_a_sentence() {
        let cases = [
            (
                "You are a database performance engineer. Diagnose and speed up the slow query below.",
                "Database Performance Engineer",
            ),
            (
                "You are a senior software architect working inside my repository as an autonomous coding agent.",
                "Senior Software Architect",
            ),
            (
                "You are an expert in the Model Context Protocol (MCP) and production backend engineering.",
                "Expert in the Model Context Protocol",
            ),
            (
                "You are a YouTube strategist and scriptwriter known for high-retention videos.",
                "YouTube Strategist",
            ),
            (
                "Act as a senior troubleshooting engineer. Help me find the root cause.",
                "Senior Troubleshooting Engineer",
            ),
            // Not a role: the first sentence, without a trailing period.
            (
                "Summarize these meeting notes into action items. Keep it short.",
                "Summarize these meeting notes into action items",
            ),
            // One vague word isn't a title; the sentence is.
            (
                "You are an assistant. Reply to my emails politely.",
                "You are an assistant",
            ),
        ];
        for (body, title) in cases {
            assert_eq!(derive_title(body), title, "{body}");
        }
    }

    #[test]
    fn derived_summaries_skip_headings_and_what_the_title_used() {
        let md = "# Python Refactoring Coach 🔥❄️\n\nYou are a senior Python engineer. Refactor my code for clarity.";
        // A typed title: the heading still isn't summary text.
        assert_eq!(
            derive_summary(md, false),
            "You are a senior Python engineer. Refactor my code for clarity."
        );
        assert_eq!(
            derive_summary(md, true),
            "You are a senior Python engineer. Refactor my code for clarity."
        );
        let plain =
            "You are a database performance engineer. Diagnose and speed up the slow query below.";
        assert_eq!(
            derive_summary(plain, true),
            "Diagnose and speed up the slow query below."
        );
        // A single sentence that became the whole title leaves no summary.
        assert_eq!(derive_summary("Plan a product launch.", true), "");
    }

    #[test]
    fn unicode_content_is_preserved() {
        let mut lib = Library::open_in_memory();
        let body = "Écris un résumé 📝 — 日本語もOK";
        let s = create(&mut lib, input("Résumé", body)).unwrap();
        assert_eq!(record_copy(&mut lib, s.id).unwrap().1, body);
        assert_eq!(fts_candidates(&lib, "\"resume\"*", 5).unwrap().len(), 1);
    }
}

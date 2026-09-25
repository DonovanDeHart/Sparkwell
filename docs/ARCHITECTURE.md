# Sparkwell architecture

This document records how the MVP is built and why, as a companion to the three canonical specifications (Product Vision & MVP Contract, UI/UX & Visual Design, Technical Architecture & Build Acceptance). Where this build made a choice the specs left open, the reasoning is here.

## System shape

```
┌──────────────────────────── Sparkwell.exe (Tauri 2) ────────────────────────────┐
│                                                                                  │
│  WebView2 (React + TypeScript)            Rust core                              │
│  ┌──────────────────────────┐   typed    ┌─────────────────────────────────────┐ │
│  │ app/  shell + state      │  commands  │ commands.rs  (the only IPC surface) │ │
│  │ features/ search, fav,   │ ─────────▶ │ window / platform   dock, show/hide │ │
│  │   add-spark, settings    │ ◀───────── │ hotkey              register+rollback│ │
│  │ services/api.ts          │   events   │ sparks              repository      │ │
│  └──────────────────────────┘            │ search              lexical+semantic│ │
│                                          │ ai                  Ollama adapter  │ │
│                                          │ library, storage    SQLite lifecycle│ │
│                                          └──────────┬───────────────┬──────────┘ │
└─────────────────────────────────────────────────────┼───────────────┼────────────┘
                                                      │               │ HTTP, localhost only
                                     %LOCALAPPDATA%\Sparkwell\       127.0.0.1:11434 (optional Ollama)
                                     Library\sparkwell.db
```

**The Rust core owns the library.** All persistence, retrieval, clipboard, hotkey, dialog, and AI work happens in Rust behind a small set of typed commands (`src-tauri/src/commands.rs`). The webview has no filesystem, SQL, shell, or HTTP capability; its Tauri permissions are limited to listening for events and starting a window drag (`capabilities/main-window.json`). The technical spec points to the Tauri SQL plugin as a reference; it was not used because relocation, embedding storage, full-text maintenance and search ranking are simpler and safer to keep transactional in one process, and it avoids exposing raw SQL to the webview.

## Modules (spec §2 "Recommended internal modules")

| Module | File(s) | Responsibility |
| --- | --- | --- |
| window | `window.rs`, `platform.rs` | Dock to the right edge of the target monitor's work area; show / hide / toggle; pin (always on top); safe click-away auto-hide; tray toggle debounce |
| hotkey | `hotkey.rs`, `src/services/hotkeys.ts` | Accelerator validation, register/unregister lifecycle with rollback, suspend while recording |
| sparks | `sparks/` | Validation, CRUD, favorites order, copy usage metadata, duplicate detection, starter Sparks |
| search | `search/` | Intent-aware lexical scoring, FTS5 candidates, in-process vector index, fusion and confidence |
| ai | `ai/` | Ollama health/model policy, background indexer, query embeddings, Smart Add metadata |
| settings | `settings.rs` | Typed preferences file (hotkey, pin, library location) |
| storage | `storage/`, `library.rs` | SQLite open/identify/migrate, integrity checks, safe relocation, recovery |

## Window behaviour

- **Form factor.** One frameless, transparent, non-resizable window (`tauri.conf.json`). The panel is drawn by CSS inside an 8 px transparent gutter so it has rounded corners and a soft shadow on any Windows version. Native acrylic was not used: it fills the whole rectangle (square corners under a rounded panel) and is known to lag while the window moves.
- **Docking.** On show, `platform::target_work_area` picks the monitor of the *foreground window* (where the user was working when they pressed the hotkey), falling back to the cursor's monitor, and uses `GetMonitorInfoW().rcWork` so the taskbar is respected. Width is 436 logical px (clamped to 392 px on small logical screens, never more than 30% of the work area); height is the full work area. Size is applied, the window moved, and the size re-applied so moving between monitors with different DPI lands correctly.
- **Toggle semantics.** Hidden → show and focus the goal input (text selected, last query kept). Visible but unfocused → focus. Visible and focused → hide. The tray icon toggles too, ignoring the click that caused a click-away hide.
- **Pin.** Pinned: always on top and never auto-hides. Unpinned: collapses on Esc, click-away (after a 400 ms show grace period and a 140 ms re-check, and never while a native folder dialog is open) and 650 ms after a successful copy, which hands focus back to the previous app for an immediate paste.
- **Lifecycle.** Single instance (a second launch reveals the first), close = hide, tray has Show/Hide and Quit, launch-at-startup uses the official autostart plugin with `--hidden` so login starts quietly.

## Activation hotkey

`hotkey::validate` is the authority (the UI only converts key events to accelerator strings):

- exactly one non-modifier key, plus modifiers;
- letters, digits, punctuation, Space and Enter need **two** modifiers, because a single-modifier shortcut such as `Ctrl+K` would hijack that key in every app;
- F1–F12, navigation and numpad keys need one modifier; F13–F24 may be used alone;
- Escape, Tab, Delete, Backspace, lock and PrintScreen keys are refused, as are shortcuts Windows reserves (`Alt+F4`, `Win+Shift+S`, `Win+arrows`, …).

`hotkey::change` follows the spec's order: unregister the old shortcut, register the new one, and persist only after the OS accepts it. If registration fails (owned by another app), the previous shortcut is re-registered and the UI shows a conflict message. While recording, the current shortcut is temporarily released so pressing it can be captured. Hiding the window always re-registers it.

## Data model

`migrations/0001_initial.sql` (applied transactionally, tracked with `PRAGMA user_version`; the file is guarded by `PRAGMA application_id = 'SPKW'` so Sparkwell never adopts or migrates an unrelated database):

- `sparks`: `id, title, summary, body, body_hash, favorite, favorited_at, created_at, updated_at, usage_count, last_copied_at, source_note, category`
- `tags`, `spark_tags(spark_id, tag_id, position)`
- `embeddings(spark_id, model, dimensions, vector BLOB, content_hash, generated_at)`, keyed by `(spark_id, model)` so switching embedding models does not discard vectors
- `settings(key, value)`: library-scoped metadata
- `sparks_fts`: an FTS5 table (`porter unicode61 remove_diacritics 2`) maintained in the same transaction as every Spark write

Rules: **Copy always reads the full `body` from the database**; the summary is never a substitute. Editing a Spark deletes its embeddings (for every model) in the same transaction, and the indexer re-embeds it. The indexer also re-checks the content hash before storing, so a vector computed from stale text is never saved.

Preferences that must be known *before* the library opens (library location, activation hotkey, pin) live in `%LOCALAPPDATA%\Sparkwell\config.json`, written atomically (temp file and rename). A corrupt file is backed up and replaced by defaults, so preferences can never stop Sparkwell from launching. Launch-at-startup is read from the OS registration rather than stored.

## Library location and safety

- The default is `%LOCALAPPDATA%\Sparkwell\Library\sparkwell.db`. It sits outside the WebView2 cache folder (`%LOCALAPPDATA%\com.sparkwell.app`, which the uninstaller's optional "delete app data" removes). The NSIS uninstaller only removes the install directory when it is empty, so uninstalling never touches the library; CI verifies this.
- **Copy & switch**: `VACUUM INTO` a temporary file in the target folder, run `PRAGMA integrity_check`, compare the Spark count and application id, rename into place, open, persist the new location, then switch. Any failure removes only the partial copy, and the original library stays active and untouched.
- **Use existing library**: opens and migrates a Sparkwell library already in the folder, after an explicit confirmation.
- If a custom location is missing at launch (for example an unplugged drive), Sparkwell does **not** silently create an empty library elsewhere. It shows a calm "library isn't available" state with Try again / Choose location / Use default location.

## Retrieval

1. **Normalise the goal.** Tokenise, fold accents, strip conversational filler ("I need AI to help me…"), and lightly stem so "servers" meets "server" and "debugging" meets "debug".
2. **Candidates.** Take the top 50 FTS5 hits (bm25 with title > tags > summary > body weights), plus the top 20 semantic neighbours when semantic evidence exists.
3. **Lexical score** (0–1): field-weighted coverage of the goal's terms (title 1.0, tags 0.85, summary 0.55, body 0.25), blended 80/20 with how much of the Spark's title the goal covers.
4. **Semantic score** (0–1): cosine similarity measured as a **z-score against the library's own similarity distribution for this query**, mapped from z = 0.5 → 0 to z = 2.0 → 1. This makes the signal independent of each embedding model's absolute cosine range. It requires at least 5 indexed Sparks; below that, standard retrieval is used.
5. **Fusion:** `max(0.6·semantic + 0.4·lexical, 0.85·lexical)`, so strong word evidence is never penalised. An exact title match is floored at 0.95. Favorite, usage and recency add at most 0.045 and serve only as tie-breakers. Ties are broken deterministically.
6. **Confidence:**
   - A **Best Match** needs a fused score ≥ 0.42 *and* a lead of at least 0.10 over the runner-up, unless the score is decisive (≥ 0.8).
   - A near-tie between two different Sparks is shown honestly as "No strong match" with the closest Sparks listed.
   - Scores are never shown as percentages; the UI uses qualitative states only (spec §4 ranking guidance).

The thresholds were calibrated against live `nomic-embed-text` runs (`src-tauri/tests/semantic_live.rs`). The expected Spark was visible for 9/9 intent phrasings with little or no word overlap, with 7/9 confident Best Matches, no confidently wrong answer, and no false confidence on unrelated goals.

The vector index is an in-memory map of normalised `f32` vectors. A linear scan over a few thousand Sparks takes well under a millisecond, so no vector database is needed (spec §4.4).

## Local intelligence (Ollama)

- `ai::spawn_service` runs a background loop that never delays startup. It probes `GET /api/tags` with a 1.5 s timeout (every 15 s while offline, 60 s while online, and immediately when the panel opens or Sparks change), picks models, loads stored vectors, and embeds any Spark lacking a vector for the current model (`POST /api/embed`, batches of 8). Progress appears in the footer.
- **Model policy:** a preference list per role (`nomic-embed-text`, `mxbai-embed-large`, … for embeddings; `qwen2.5`, `llama3.2`, … for chat), falling back to any installed local model. Models reported as remote/cloud (`remote_host` / `remote_model`, or a `-cloud` / `:cloud` tag) are never selected. `SPARKWELL_EMBED_MODEL` and `SPARKWELL_CHAT_MODEL` override the choice. Task prefixes are applied for models trained with them (for example `search_query:` / `search_document:` for nomic).
- **Query embeddings** have a 3 s budget. Showing the panel pre-warms the embedding model so the first search is fast. Any failure (timeout, missing model, stopped server, malformed response) silently falls back to standard search and schedules a re-probe.
- **Smart Add** uses `POST /api/chat` with a JSON schema. The Spark body is wrapped as data with explicit instructions not to follow it. Output is parsed defensively (clipped, tags normalised) and only fills the editor; nothing is saved without the user pressing Save.
- **Privacy:** the client is hard-wired to `http://127.0.0.1:11434`, uses no proxy, and makes no other network calls. There is no telemetry anywhere in the app.

## UI

- React 19 with hand-written CSS on design tokens (`src/styles/tokens.css`). There is no component library, so the Fire & Ice identity stays deliberate: ice (cyan) for focus, retrieval and trust; fire (amber) for Sparks, copy, creation and favorites.
- One stable shell with no page navigation: header → goal input → result region → Favorites → sticky Add New Spark → Local Only footer. Add/Edit and Settings are in-panel drawers.
- Retrieval runs on Enter (never per keystroke). A pending skeleton appears only if retrieval takes more than 140 ms, so instant local searches never flash a loading state. Stale responses are discarded by request id.
- Accessibility:
  - Every control is keyboard reachable with visible focus rings.
  - Icon buttons have accessible names and tooltips.
  - State is never conveyed by colour alone ("Copied" text and check icon, pressed states, labels).
  - `prefers-reduced-motion` is honoured.
  - Type sizes are in rem, so Windows text scaling applies.
- `src/services/api.ts` is the only bridge to the core. In a plain browser (`npm run dev`) and in tests it falls back to `mockBackend.ts`. That fallback sits behind `import.meta.env.DEV`, so it is compiled out of production builds (verified: no mock code in `dist/`).

## Explicitly out of scope (per the MVP contract)

Collections, View Details, Fork, model selectors, Run/Send-to-AI, accounts, sync, marketplace, analytics, version history, and prompt testing are all absent by design.

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

- **Form factor.** One frameless, non-resizable window that *is* the panel (`tauri.conf.json`). Windows 11 draws its rounded corners, 1 px border and drop shadow (`shadow: true`). On Windows 11 22H2 and later, with Windows transparency effects on, the panel sits on a DWM acrylic backdrop behind a dark tint (frosted glass); otherwise it is painted opaque, so desktop content never shows through sharply (`window::apply_material`, `AppSnapshot::glass`). On Windows 10 the native shadow is turned off (it would draw a white border).
- **Docking.** On show, `platform::target_work_area` picks the monitor of the *foreground window* (where the user was working when they pressed the hotkey), falling back to the cursor's monitor, and uses `GetMonitorInfoW().rcWork` so the taskbar is respected. `platform::dock_rect` places the panel top-right with an 8 px gap, like a Windows flyout: 436 logical px wide (392 px on small logical screens, never more than 30% of the work area) and as tall as its content within 700–900 logical px, never beyond the work area. The UI measures its content and reports it (`set_panel_height`); the window resizes in place, keeping its top edge, and overlays get the full height. The window rectangle includes invisible resize borders, so placement targets the client area. Size is applied, the window moved, and both re-applied so moving between monitors with different DPI lands correctly.
- **Docked, not draggable.** The header is not a drag region and the webview has no `start-dragging` permission.
- **Toggle semantics.** Hidden → show and focus the goal input (text selected, last query kept), or the open overlay if there is one. Visible but unfocused → focus. Visible and focused → hide. The tray icon toggles too, ignoring the click that caused a click-away hide.
- **Pin.** Pinned: always on top and never auto-hides. Unpinned: collapses on Esc, click-away (after a 400 ms show grace period and a 140 ms re-check, and never while a native folder dialog is open) and 650 ms after a successful copy, which hands focus back to the previous app for an immediate paste.
- **Lifecycle.** Single instance (a second launch reveals the first; `--quit` asks it to quit cleanly), close = hide, tray has Show/Hide and Quit. Launch-at-startup uses the official autostart plugin with `--hidden`. The window is created unfocused (`focus: false`), so a `--hidden` start never takes keyboard focus; only an explicit show (hotkey, tray, relaunch) focuses it.

## Activation hotkey

`hotkey::validate` is the authority (the UI only converts key events to accelerator strings):

- exactly one non-modifier key, plus modifiers;
- letters, digits, punctuation, Space and Enter need **two** modifiers, because a single-modifier shortcut such as `Ctrl+K` would hijack that key in every app;
- F1–F12, navigation and numpad keys need one modifier; F13–F24 may be used alone;
- Escape, Tab, Delete, Backspace, lock and PrintScreen keys are refused, as are shortcuts Windows reserves (`Alt+F4`, `Win+Shift+S`, `Win+arrows`, …).

`hotkey::change` follows the spec's order: unregister the old shortcut, register the new one, and persist only after the OS accepts it. If registration fails (owned by another app), the previous shortcut is re-registered and the UI shows a conflict message. While recording, the current shortcut is temporarily released so pressing it can be captured. Hiding the window always re-registers it.

There is **no default shortcut**: no single combination is free on every machine. On first run (`config.json` has no `onboarded` flag yet) a welcome asks the user to press one, using the same validation and conflict check; "Skip for now" leaves Sparkwell usable from the tray icon and Settings shows "Not set". Configs written before this existed keep their shortcut and skip the welcome. At startup a saved shortcut that fails to register is never swapped for a guess: `hotkey::initial_status` records the error, the panel shows a notice with "Choose a shortcut" (on a `--hidden` start it is shown without taking focus), and the tray tooltip says the shortcut is unavailable.

## Data model

`migrations/0001_initial.sql` (applied transactionally, tracked with `PRAGMA user_version`; the file is guarded by `PRAGMA application_id = 'SPKW'` so Sparkwell never adopts or migrates an unrelated database):

- `sparks`: `id, title, summary, body, body_hash, favorite, favorited_at, created_at, updated_at, usage_count, last_copied_at, source_note, category`
- `tags`, `spark_tags(spark_id, tag_id, position)`
- `embeddings(spark_id, model, dimensions, vector BLOB, content_hash, generated_at)`, keyed by `(spark_id, model)` so switching embedding models does not discard vectors
- `settings(key, value)`: library-scoped metadata
- `sparks_fts`: an FTS5 table (`porter unicode61 remove_diacritics 2`) maintained in the same transaction as every Spark write

Rules: **Copy always reads the full `body` from the database**; the summary is never a substitute. Editing a Spark deletes its embeddings (for every model) in the same transaction, and the indexer re-embeds it. The indexer also re-checks the content hash before storing, so a vector computed from stale text is never saved.

Preferences that must be known *before* the library opens (library location, activation hotkey, pin, whether the first-run welcome is done) live in `%LOCALAPPDATA%\Sparkwell\config.json`, written atomically (temp file and rename). A corrupt file is backed up and replaced by defaults, so preferences can never stop Sparkwell from launching. Launch-at-startup is read from the OS registration rather than stored.

The library runs in WAL mode. Every write is followed by a passive checkpoint, and quitting (tray, Settings, `--quit`) runs a `TRUNCATE` checkpoint and closes the connection, so `sparkwell.db` is complete on its own: copying just that file copies the library.

## Library location and safety

- The default is `%LOCALAPPDATA%\Sparkwell\Library\sparkwell.db`. It sits outside the WebView2 cache folder (`%LOCALAPPDATA%\com.sparkwell.app`, which the uninstaller's optional "delete app data" removes). The NSIS uninstaller only removes the install directory when it is empty, so uninstalling never touches the library; CI verifies this.
- **Copy & switch**: `VACUUM INTO` a temporary file in the target folder, run `PRAGMA integrity_check`, compare the Spark count and application id, rename into place, open, persist the new location, then switch. Any failure removes only the partial copy, and the original library stays active and untouched.
- **Use existing library**: opens and migrates a Sparkwell library already in the folder, after an explicit confirmation.
- If a custom location is missing at launch (for example an unplugged drive), Sparkwell does **not** silently create an empty library elsewhere. It shows a calm "library isn't available" state with Try again / Choose location / Use default location.

## Retrieval

1. **Normalise the goal.** Tokenise, fold accents, strip conversational filler ("I need AI to help me…"), and lightly stem so "servers" meets "server" and "debugging" meets "debug".
2. **Candidates.** Take the top 50 FTS5 hits (bm25 with title > tags > summary > body weights), plus the top 20 semantic neighbours when semantic evidence exists.
3. **Lexical score** (0–1): field-weighted coverage of the goal's terms (title 1.0, tags 0.85, summary 0.55, body 0.25), each term weighted by its rarity in the library (IDF), blended 80/20 with how much of the Spark's title the goal covers.
4. **Retrieval profiles** (`search::profile`). Each Spark is embedded as several views: a compact *profile* (title, summary, the Purpose/Needs it states, tags, and the topics its sections cover) and up to four passages spread across the body, each prefixed with the Spark's identity. A long, detailed Spark is therefore compared on what it is for, not diluted by its length, and a short one is not favoured just for being short.
5. **Semantic score** (`search::vectors`). Similarity to a Spark is a soft maximum over its views (0.75 × best + 0.25 × second, profile weighted 1.05). Two model-independent corrections come from a fixed pool of 40 generic, unrelated goals embedded with the same model: a Spark's *hub level* (its mean similarity to the 10 generic goals it is closest to) is subtracted, so Sparks written in generic "AI assistant" language don't match everything; and the result is scaled by the typical spread of similarities for that model (clamped 0.15–0.6), so thresholds carry across embedding models. Queries and documents get the prefixes each model family was trained with (`ai::models`: qwen3-embedding instructions, nomic `search_query:` / `search_document:`, e5 `query:` / `passage:`, …). At least 5 indexed Sparks are needed; below that, standard retrieval is used.
6. **Fusion:** 0.6 × semantic + 0.4 × lexical (Sparks not yet indexed use 0.85 × lexical). An exact title match is floored at 0.95. Favorite, usage and recency only break ties.
7. **Confidence.** "No strong match" is always preferred to a wrong answer:
   - With semantic evidence, a **Best Match** needs a fused score ≥ 0.30 and a lead of at least 0.12 over the runner-up, unless the two leaders state essentially the same purpose (profile similarity ≥ 0.85), which is a tie between equivalent Sparks rather than ambiguity. Candidates below 0.10 are not shown.
   - Standard retrieval keeps its calibrated rule (≥ 0.42 with a 0.10 lead, or ≥ 0.8).
   - Scores are never shown as percentages; the UI uses qualitative states only (spec §4 ranking guidance).
8. **Never silent.** Every standard result says so and why (offline, no embedding model, still indexing, didn't answer in time, unavailable). Semantic results say "Matched by local intelligence".

Changing the profile format (`PROFILE_VERSION`) or model changes each Spark's content hash, so the indexer re-embeds in the background.

**Evaluation.** `src-tauri/tests/support` holds the 12 goals from the physical acceptance test (each with its expected Spark and acceptable alternatives), 36 further paraphrases, and 6 unrelated goals, run against the 17-Spark acceptance library (`tests/fixtures/acceptance_library.json`). Results are graded Correct / Acceptable / Missed / Confidently wrong. `semantic_regression.rs` replays recorded `qwen3-embedding:0.6b` vectors in CI; `semantic_live.rs` (opt-in) runs the same suite against a live Ollama and can re-record the fixture.

The vector index is an in-memory map of normalised `f32` vectors. A linear scan over a few thousand Sparks takes well under a millisecond, so no vector database is needed (spec §4.4).

## Local intelligence (Ollama)

- `ai::spawn_service` runs a background loop that never delays startup. It probes `GET /api/tags` with a 1.5 s timeout (every 15 s while offline, 60 s while online, and immediately when the panel opens or Sparks change), picks models, loads stored vectors, and embeds any Spark whose views changed (`POST /api/embed`, batches of up to 16 texts, never splitting a Spark). Progress appears in the footer.
- **Embedding model policy:** a preference list (`nomic-embed-text`, `mxbai-embed-large`, `snowflake-arctic-embed2`, `bge-m3`, `embeddinggemma`, `qwen3-embedding`, …), falling back to any installed local embedding model. Models reported as remote/cloud are never selected. `SPARKWELL_EMBED_MODEL` overrides the choice.
- **Query embeddings** get 5 s while the model is known to be loaded and 25 s when it may be cold; showing the panel pre-warms it (kept loaded for 30 min). After 1.2 s the pending state reads "Waking up local intelligence…" and the rest of the panel stays usable. A timeout or failure falls back to standard search **with a label saying why**, and schedules a re-probe.
- **Smart Add (Auto-fill)** runs only when the user presses "Auto-fill details"; pasting never starts it. It uses `POST /api/chat` with a JSON schema, and only a *small* local model (≤ 8.5 B parameters or ≤ 6 GiB, preferring `qwen2.5`, `llama3.2`, `qwen3`, `gemma3`, `phi4-mini`, …), so it answers in seconds and doesn't push the embedding model out of memory; the chat model is kept loaded for only 2 min and the embedding model is re-warmed afterwards. `SPARKWELL_CHAT_MODEL` overrides the choice. When it can't run, the editor says why (offline, or no small model, with a one-line `ollama pull` suggestion). The Spark body is wrapped as data with explicit instructions not to follow it. Output is parsed defensively (clipped, tags normalised to the library's Title Case) and only fills the editor; nothing is saved without the user pressing Save.
- **Privacy:** the client is hard-wired to `http://127.0.0.1:11434`, uses no proxy, and makes no other network calls. There is no telemetry anywhere in the app.

## UI

- React 19 with hand-written CSS on design tokens (`src/styles/tokens.css`). There is no component library, so the Fire & Ice identity stays deliberate: ice (cyan) for focus, retrieval and trust; fire (amber) for Sparks, copy, creation and favorites.
- One stable shell with no page navigation: header → goal input → result region → Favorites → sticky Add New Spark → Local Only footer. Add/Edit and Settings are in-panel drawers.
- Retrieval runs on Enter (never per keystroke). Enter again on the same goal copies the Best Match; `Ctrl+Enter` also copies, but another app can own it globally, so the hint names Enter. A pending skeleton appears only if retrieval takes more than 140 ms, so instant local searches never flash a loading state. Stale responses are discarded by request id.
- Text fields never offer browser autofill: WebView2 general autofill is off for the window (`generalAutofillEnabled: false`) and every field sets `autocomplete="off"`.
- Accessibility:
  - Every control is keyboard reachable with visible focus rings.
  - Icon buttons have accessible names and tooltips.
  - State is never conveyed by colour alone ("Copied" text and check icon, pressed states, labels).
  - `prefers-reduced-motion` is honoured.
  - Type sizes are in rem, so Windows text scaling applies.
- `src/services/api.ts` is the only bridge to the core. In a plain browser (`npm run dev`) and in tests it falls back to `mockBackend.ts`. That fallback sits behind `import.meta.env.DEV`, so it is compiled out of production builds (verified: no mock code in `dist/`).

## Explicitly out of scope (per the MVP contract)

Collections, View Details, Fork, model selectors, Run/Send-to-AI, accounts, sync, marketplace, analytics, version history, and prompt testing are all absent by design.

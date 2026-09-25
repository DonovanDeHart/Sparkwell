# Sparkwell testing record

What was verified for the MVP, and how. The technical spec's rule is that the shipping gate is a *verified desktop workflow*, not source completion; this file maps each acceptance item to its evidence.

## Automated

| Suite | Command | Covers |
| --- | --- | --- |
| Rust unit tests (79) | `cd src-tauri && cargo test` | Migrations (idempotent, rejects newer schema); library identity (refuses foreign or garbage files); Spark validation, CRUD, derived titles and summaries, very long bodies (20k lines round-trip exactly), Unicode, duplicates, favorites order, copy usage; tag normalisation; FTS maintenance; relocation (verified copy, never overwrites, no partial file on failure, bad paths); config (defaults, round trip, corrupt file backed up); lexical retrieval for intent phrasings; empty library; empty query; weak / no match; deterministic ties; history as tie-breaker only; semantic-only matches; fusion; vector encoding; hotkey validation (canonical form, hijacking combos, reserved Windows shortcuts, malformed input); Ollama model policy (never cloud models, overrides); Smart Add output parsing; indexer never stores stale or deleted content; unreachable Ollama fails fast; right-edge dock geometry at 100/125/150/200% scaling, on a secondary monitor with negative offsets, and with the taskbar on top |
| UI behaviour tests (29) | `npm test` | Launch state; intent search → Best Match; Copy Spark copies the full body; unpinned collapse after copy; pinned stays; one-click Favorite copy; Ctrl+Enter copy; honest no-match; closest candidates; semantic label; clearing returns to idle; clipboard failure reported without claiming success; Esc collapse; unfavorite with Undo; manual Add without AI; required body; duplicate warning with Save anyway; discard confirmation; Smart Add fills fields but never saves; edit from Best Match; empty library; library unavailable; hotkey record (invalid → conflict → saved); hotkey cancel restores; launch at startup; library move requires confirmation; Settings limited to MVP sections |
| Keyboard helpers | `npm test` | Key-code → accelerator mapping, modifier ordering, keycap labels |
| Static checks | `npm run typecheck`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` | Types, formatting, lints |
| Live retrieval harness (opt-in) | `cargo test --test semantic_live -- --ignored --nocapture` | Real Ollama embeddings over the starter library: 9/9 expected Sparks shown, 7/9 confident Best Matches, 0 confidently wrong, 0 false-confident for unrelated goals (nomic-embed-text, Ollama 0.13.5) |

CI (`.github/workflows/ci.yml`) runs the static checks and both suites on Linux, runs the UI tests and the Rust tests on Windows, builds the NSIS + MSI packages on `windows-latest`, and runs `scripts/windows-smoke.ps1` against the installed app.

## Real desktop app runs

### Linux (Xvfb + Openbox), debug build of the real app

These exercise the real Rust core, real SQLite file, real global shortcut (X11), and real system clipboard. Only the operating system differs from the target.

| Check | Result |
| --- | --- |
| First launch creates `~/.local/share/Sparkwell/Library/sparkwell.db`, seeds 8 starter Sparks and 5 Favorites | ✅ |
| Window docks to the right edge of the monitor work area (x = width − 436, full height) | ✅ |
| Goal "I need AI to help me build an MCP server" → Best Match *MCP Server Architect* (standard search) | ✅ |
| Ctrl+Enter / Copy Spark → complete 19-line body on the system clipboard (checked with `xclip`) | ✅ |
| Unpinned panel collapses after copy; global hotkey shows it again | ✅ |
| Hotkey changed in Settings to Ctrl+Shift+K → old combination released, new one works, persisted to `config.json`, still active after restart | ✅ |
| Launch with `--hidden` (autostart) starts without showing the panel | ✅ |
| Esc collapses (unpinned); click-away collapses (unpinned); pinned stays visible and on top; pin state persisted | ✅ |
| Add New Spark through the UI (body, title, tags, favorite) → row written to SQLite with tags and favorite; appears in Favorites | ✅ |
| Kill and relaunch → added Spark still present and retrievable by intent | ✅ |
| Second launch → still exactly one process (single instance) | ✅ |
| Ollama started while the app was running → detected without restart; library embedded in the background (768-d vectors) | ✅ |
| Semantic goal "grow my channel with videos people watch to the end" → *YouTube Script Architect*, "Matched by local intelligence" | ✅ |
| Smart Add with `qwen2.5:0.5b`: paste into an empty editor → title, summary and tags drafted and editable; nothing saved until Save | ✅ |
| Ollama stopped mid-session → next search falls back to standard retrieval with "Local intelligence offline · standard search active"; correct result; no error | ✅ |
| Ollama restarted → semantic retrieval resumes automatically | ✅ |
| Library Location → native folder dialog (panel stays open behind it) → explicit confirmation → verified copy (integrity ok, all Sparks and embeddings) → switched; original untouched; relaunch opens the new location | ✅ |
| Custom library folder missing at launch ("unplugged drive") → calm unavailable state; **no empty library created elsewhere**; after reconnecting, *Try again* recovers | ✅ |
| Two monitors (1920×1080 + 1280×1024 offset at +56) → docks to the monitor under the cursor, respecting its offset top and width clamp; re-docks to the other monitor when summoned there | ✅ |

### Windows (CI, `windows-latest`), release build

`scripts/windows-smoke.ps1` installs the produced NSIS package silently and checks the installed app. Results are in the **Windows build, package & smoke test** job log, with a screenshot in the `smoke-artifacts` artifact:

- installs per-user without admin;
- launches, creates and seeds `%LOCALAPPDATA%\Sparkwell\Library\sparkwell.db`;
- docks to the right edge of the monitor work area at full work-area height;
- a second launch exits (single instance);
- injected `Ctrl+Alt+Space` hides and re-shows the panel via the real Win32 `RegisterHotKey` registration, and the shown panel owns the foreground;
- relaunch keeps the library;
- uninstall removes the app but **keeps the library**.

## Visual verification

The UI was compared against the canonical references (UI/UX spec, Reference C) with Chromium screenshots at the real window size, covering: idle, Best Match, copied, no strong match, closest Sparks, semantic label with indexing footer, Add New Spark, Settings, hotkey recording / conflict / saved, and 125% text scaling (no clipping; tags that don't fit drop out whole rather than clipping).

## Not yet verified on physical Windows hardware

These need a person at a Windows 11 machine; the logic behind each is covered by the tests above:

- Physical multi-monitor setups with mixed DPI (dock geometry is unit-tested for mixed scale factors and was verified on a two-monitor X11 layout).
- 125 / 150 / 200% Windows display scaling in the running app (dock sizing is unit-tested per scale; UI type is rem-based).
- Pasting a copied Spark into a Windows text editor (Copy uses the Tauri clipboard plugin, backed by the same `arboard` library verified end to end on Linux; the Windows smoke test does not click Copy).
- A hotkey conflict with a real third-party app (the conflict path is covered by the rollback logic and UI tests).

Suggested 5-minute manual pass on Windows 11: install → press Ctrl+Alt+Space in another app → type a goal → Enter → Ctrl+Enter → paste into Notepad → Settings → record a new hotkey → restart → confirm it works → with two monitors, click into an app on the other monitor and summon Sparkwell there.

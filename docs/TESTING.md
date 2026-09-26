# Sparkwell testing record

What was verified for the MVP, and how. The technical spec's rule is that the shipping gate is a *verified desktop workflow*, not source completion; this file maps each acceptance item to its evidence.

## Automated

| Suite | Command | Covers |
| --- | --- | --- |
| Rust unit tests (111) | `cd src-tauri && cargo test` | Everything listed in previous rounds, plus: canonical embedding-model policy (never substitutes another model, never a chat model), Qwen3 query instruction and plain documents, labelled profiles, switching models invalidates only the vector cache (and records model, recipe and dimensions), Spark bodies stored exactly as given |
| Window configuration (2) | `cargo test --test window_config` | Window created unfocused; WebView2 general autofill off |
| User-added Sparks (3) | `cargo test --test user_added_sparks` | Five pasted "premium" prompts (CRLF, tabs, emoji, leading/trailing whitespace) stored and copied byte for byte; retrieval profiles separate from the body; editing details or drafting metadata never touches the body |
| Semantic regression (2) | `cargo test --test semantic_regression` | Recorded `qwen3-embedding:8b-q8_0` vectors (215 texts) replayed in CI: zero confidently wrong Best Matches across all 101 goals (and the 12 acceptance goals again with the user-added Sparks present) and 16 unrelated goals; floors at the recorded results (acceptance 8 correct, dev 26, validation 10, test 10, user-added 10/10); f16 round trip |
| UI behaviour tests (48) | `npm test` | As before, plus: semantic model not installed (label, Settings guidance, library still usable), Ollama offline, indexing progress, a pasted Spark saved and copied exactly as written |
| Keyboard helpers | `npm test` | Key-code → accelerator mapping, modifier ordering, keycap labels |
| Static checks | `npm run typecheck`, `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` | Types, formatting, lints |
| Live semantic suite (opt-in) | `cargo test --test semantic_live semantic_retrieval_quality -- --ignored --nocapture` | The same suites against a live Ollama with `qwen3-embedding:8b-q8_0` (RTX 5090, Ollama 0.34.4): acceptance 8 correct · 4 acceptable · 0 missed · 0 confidently wrong; dev 26 · 10 · 0 · 0; validation 10 · 12 · 1 · 0; test 10 · 10 · 0 · 0; user-added 10 · 0 · 0 · 0; no unrelated goal confident. Indexing 17 Sparks (45 views) 1.8 s, warm query 22 ms median |

`cargo test --test semantic_tune -- --ignored --nocapture` sweeps the calibration (tuning + validation goals only).

CI (`.github/workflows/ci.yml`) runs the static checks and all suites on Linux, runs the UI tests and the Rust tests on Windows, builds the NSIS + MSI packages on `windows-latest`, and runs `scripts/windows-smoke.ps1` against the installed app.

## Real desktop app runs

### Windows 11 (the acceptance machine), release build of the remediation branch

Run on 2026-09-25 with `target/release/sparkwell.exe` (not installed), against the tester's library after backing it up; the library was restored byte-for-byte afterwards and the tester's own build relaunched. Geometry came from `GetWindowRect`, `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)` and the client rectangle; screenshots were taken with GDI. **No keyboard or mouse input was injected**, so typing, Esc, dragging and clicks were not exercised on the physical machine.

| Check | Result |
| --- | --- |
| Docks top-right inside the work area with an 8 px gap: client 436×732 at (2360, −1072) on a 1920×1080 monitor whose work area is (884, −1080)–(2804, −48) | ✅ |
| Height fits content: 732 px at idle with 7 Favorites (was the full 1392 px work area); grew in place to 874 px when a notice appeared | ✅ |
| Frosted glass: DWM acrylic backdrop behind the tinted panel (backdrop colour blends into the panel, nothing sharp shows through); Windows draws the rounded corners, border and shadow | ✅ |
| `--hidden` start: panel stays hidden and the foreground window is unchanged | ✅ |
| Saved shortcut already registered by another process at a `--hidden` start: panel shown with the "Choose a shortcut" notice, foreground unchanged, conflict logged | ✅ |
| `--quit` closes the running instance; no `-wal` or `-shm` file is left beside `sparkwell.db` | ✅ |
| Existing 0.1.0 config (custom shortcut, pinned) kept as is; no first-run welcome | ✅ |

### Linux (Xvfb + Openbox), debug build of the real app

Recorded for the MVP build, before the acceptance remediation (the dock was full height then). These exercise the real Rust core, real SQLite file, real global shortcut (X11), and real system clipboard. Only the operating system differs from the target.

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

`scripts/windows-smoke.ps1` installs the produced NSIS package silently and drives the installed app with injected input. It checks:

- the installer places the app in `%LOCALAPPDATA%\Sparkwell` (per-user, no admin);
- the first run shows the panel and registers no shortcut until the user chooses one (then the user's choice, `Ctrl+Alt+Space` on the runner, is seeded);
- the library is created and seeded; the panel docks top-right inside the work area, compact, with the right width (visible bounds, not the window rect);
- a second launch exits (single instance);
- the activation shortcut hides and re-shows the panel, and the shown panel owns the foreground;
- the core loop by keyboard puts the complete Best Match body on the Windows clipboard, and the unpinned panel collapses;
- a relaunch retrieves and copies again (library persisted);
- `--quit` closes it cleanly and `sparkwell.db` is complete on its own;
- a `--hidden` start stays hidden and keeps focus; with the saved shortcut taken by another process, it shows the notice without taking focus;
- uninstalling removes the app and keeps the library.

Results are in each CI run (artifact `smoke-artifacts` has screenshots and the app log). The last fully recorded run before the remediation passed all 19 checks of that version (run 36116217949, commit `3d6787e`).

## Performance (release build)

- Release executable: 7.1 MB (Linux); NSIS installer and MSI are produced by CI.
- Warm activation: hotkey → visible panel in 66–86 ms (Linux/Xvfb, measured with polling overhead).
- Standard retrieval runs in-process in milliseconds; the pending skeleton only appears if retrieval exceeds 140 ms (semantic queries with a cold model).

## Visual verification

The UI was compared against the canonical references (UI/UX spec, Reference C) with Chromium screenshots at the real window size, covering: idle, Best Match, copied, no strong match, closest Sparks, semantic label with indexing footer, Add New Spark, Settings, hotkey recording / conflict / saved, and 125% text scaling (no clipping; tags that don't fit drop out whole rather than clipping).

## Not yet verified on physical Windows hardware

These still need a person at the machine (the logic behind each is covered by the tests above):

- Typing, Esc, clicks and dragging in the new build (no input was injected in the local run): Esc closing Settings and the editor, the header not dragging, Enter again copying while OpenWhispr owns `Ctrl+Enter`.
- The first-run welcome and the installed NSIS package on this machine (CI covers both on a clean runner).
- Auto-fill with a small local model: this machine only has chat models above the size limit, so Auto-fill is disabled with an explanation there.
- Mixed-DPI multi-monitor setups and 125/150/200% scaling in the running app (dock geometry is unit-tested per scale).
- How the frosted glass reads over bright and busy backgrounds, and with Windows transparency effects turned off (the panel is then opaque).

Suggested manual pass (round 2): install → choose a shortcut in the welcome (try one that's taken first) → press it in another app → type a goal → Enter → Enter again → paste into Notepad → open Settings and the editor and close each with Esc → try dragging the header → restart → confirm the shortcut still works → turn on Launch at Startup and sign out/in → the panel stays in the tray and focus stays where it was.

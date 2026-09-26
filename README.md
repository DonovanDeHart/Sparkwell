# Sparkwell

**Your Sparks, always within reach.**

Sparkwell is a compact Windows desktop companion that turns your personal library of reusable AI prompts — *Sparks* — into an always-available asset. Press a hotkey, describe what you're trying to accomplish, get the one best Spark, copy it, and get back to work.

> Need → describe the goal → Best Match → **Copy Spark** → paste anywhere → continue working

A **Spark** is a reusable expression of intent: a prompt, workflow, role definition, research protocol, project launcher, or other structure that gives AI trajectory, context, and shape. Sparks are model-agnostic — one Spark pastes cleanly into ChatGPT, Claude, Gemini, Codex, Cursor, or a local model.

## What it does

- **Goal-first retrieval.** Type an outcome in plain language ("I need AI to help me build an MCP server"); Sparkwell returns one decisive **Best Match**, or honestly says there's no strong match and shows the closest Sparks.
- **One-click copy.** *Copy Spark* puts the complete stored Spark on the clipboard. Unpinned, the panel then collapses so focus returns to the app you were using — ready for Ctrl+V.
- **Favorites** for direct, one-click copying of the Sparks you use most.
- **+ Add New Spark** — paste a Spark and save it. Title and summary are optional (derived from the text if left blank). With a small local model available, *Auto-fill details* drafts a title, summary and tags when you ask; you edit them before saving.
- **Compact panel, docked top-right** on the monitor you're working on: as tall as its content, respecting the taskbar and per-monitor scaling, on frosted glass where Windows 11 supports it. It stays docked. Pin it to keep it on top; unpinned it collapses on Esc, click-away, or after copying.
- **Your own global shortcut.** On first run Sparkwell asks you to press the shortcut you want (no single combination is free on every machine), validates it, and checks it isn't taken by another app. You can skip and set it later in Settings; the tray icon always opens Sparkwell. If a saved shortcut stops working (another app took it), Sparkwell tells you at startup instead of failing silently.
- **Local-first.** The library is a single SQLite file on your device. No account, no cloud, no telemetry.
- **Optional local intelligence.** If [Ollama](https://ollama.com) is running, retrieval becomes semantic (matching meaning, not just words) and Auto-fill becomes available. Without it, everything still works with standard search, and results always say which kind of search found them.

## Keyboard

| Keys | Action |
| --- | --- |
| Your activation shortcut (chosen on first run) | Show / focus / hide Sparkwell from any app |
| `Enter` | Find the best Spark for the goal |
| `Enter` again | Copy the Best Match shown for that goal |
| `Shift+Enter` | New line in the goal |
| `Ctrl+Enter` | Copy the Best Match (if another app hasn't taken it globally) |
| `Esc` | Close the open panel; otherwise collapse Sparkwell (unpinned) |
| `Ctrl+N` | Add New Spark |
| `Ctrl+,` | Settings |
| `Ctrl+F` | Focus the goal input |
| `Ctrl+Enter` / `Ctrl+S` in the editor | Save the Spark |

## Install (Windows 11)

Download the installer from the latest CI run (artifact **sparkwell-windows**) or build it yourself (below), then run `Sparkwell_0.1.0_x64-setup.exe`. It installs per-user (no administrator rights) and starts Sparkwell docked to the right edge. The tray icon offers Show/Hide and Quit.

WebView2 is part of Windows 11; the installer fetches it automatically on systems that lack it.

## Local intelligence with Ollama (optional)

1. Install Ollama from <https://ollama.com> and make sure it is running.
2. Pull Sparkwell's semantic-search model, Qwen3 Embedding 8B (about 8 GB; Sparkwell never downloads it for you):
   ```powershell
   ollama pull qwen3-embedding:8b-q8_0
   ```
   It is the one embedding model Sparkwell is calibrated for; other embedding models are not used. It needs about 9 GB of GPU memory while loaded (it stays loaded for 30 minutes after use). Until it is installed, Settings says "Local semantic search is not ready" and searches use standard search.
3. Optionally pull a small chat model for Auto-fill in Add New Spark. Only small models (up to about 8 B parameters) are used, so drafting takes seconds and doesn't push the embedding model out of memory:
   ```powershell
   ollama pull qwen2.5:3b     # or: ollama pull llama3.2
   ```

Sparkwell detects Ollama automatically (it checks in the background and whenever the panel opens), indexes your library in the background (the footer shows "Indexing Sparks…"), and keeps the index current as you add or edit Sparks. Retrieval uses compact retrieval profiles derived from each Spark; the Spark you saved is never changed. There is no model configuration UI by design. `SPARKWELL_CHAT_MODEL` picks the Auto-fill model; `SPARKWELL_EMBED_MODEL` exists for development only (other embedding models are uncalibrated).

| Ollama | What you get |
| --- | --- |
| Running with `qwen3-embedding:8b-q8_0` | Semantic + lexical hybrid retrieval ("Matched by local intelligence") |
| Running without it | Standard search, labelled "semantic model not installed" |
| Running with a small chat model | Auto-fill drafts a title, summary and tags on request |
| Stopped, missing, or busy | Standard search (title, tags, summary, full text); Favorites, Add, Copy, Settings all work |

Sparkwell only talks to `127.0.0.1:11434`, bypasses any proxy, and never uses Ollama "cloud" models, so Spark content does not leave your device.

## Your library

- Default location: `%LOCALAPPDATA%\Sparkwell\Library\sparkwell.db` (a single SQLite file).
- Preferences: `%LOCALAPPDATA%\Sparkwell\config.json` (shortcut, pin state, custom library location).
- `sparkwell.db` is complete on its own after Sparkwell quits (the write-ahead log is folded back in), so copying that one file copies the library.
- Logs (no Spark content): `%LOCALAPPDATA%\com.sparkwell.app\logs\`.
- **Change location** in Settings → Library Location. Sparkwell copies the library to the new folder, verifies the copy (integrity check and Spark count), and only then switches. The original file is left untouched as a backup. You can also point Sparkwell at a folder that already contains a Sparkwell library.
- Uninstalling Sparkwell never deletes your library.
- A new library starts with eight starter Sparks so the app is useful immediately; edit or delete them freely.

## Development

Requirements: Node.js 22+, Rust (stable), and the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS (on Windows: Microsoft C++ Build Tools and WebView2).

```powershell
npm ci
npm run app:dev        # desktop app with hot reload (Vite + Tauri)
```

UI-only iteration in a browser (uses an in-memory mock backend that is never included in production builds):

```powershell
npm run dev            # http://localhost:1420
```

### Checks

```powershell
npm run typecheck
npm test                                   # UI behaviour tests (Vitest + Testing Library)
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test                                 # core unit tests + recorded semantic regression suite
cargo test --test semantic_live semantic_retrieval_quality -- --ignored --nocapture   # optional: same suite on a live Ollama
```

`cargo test` needs the frontend built once (`npm run build`) because the app embeds `dist/`.

### Production build

```powershell
npm ci
npm run app:build
```

Artifacts:

- Installer: `src-tauri\target\release\bundle\nsis\Sparkwell_0.1.0_x64-setup.exe`
- MSI: `src-tauri\target\release\bundle\msi\Sparkwell_0.1.0_x64_en-US.msi`
- Executable: `src-tauri\target\release\sparkwell.exe`

`scripts/windows-smoke.ps1` installs the built NSIS package and verifies the real app end to end (first run, docking, single instance, global shortcut, restart persistence, clean quit, quiet `--hidden` start, uninstall keeps the library). CI runs it on `windows-latest` for every pull request. It installs over and uninstalls any existing Sparkwell, so run it on a clean machine.

## Repository layout

```
src/                     React + TypeScript UI
  app/                   shell (header, footer, state hooks, orchestration)
  components/            icons, buttons, toggle, toast, menu, logo
  features/              search (goal input, Best Match), favorites, add-spark, settings, sparks
  services/              typed IPC client, hotkey helpers, dev-only mock backend
  styles/                Fire & Ice design tokens and base styles
src-tauri/               Rust core (Tauri 2)
  src/window.rs          dock, show/hide/toggle, pin, safe auto-hide
  src/platform.rs        target monitor + work area (Win32 foreground window)
  src/hotkey.rs          accelerator validation and register/rollback lifecycle
  src/sparks/            Spark repository and starter Sparks
  src/search/            lexical scoring, vector index, fusion
  src/ai/                Ollama client, model policy, indexer, Smart Add
  src/storage/           SQLite lifecycle, migrations, relocation
  src/library.rs         open / switch / recover the library
  src/commands.rs        the typed command surface exposed to the UI
  migrations/            SQL migrations
  capabilities/          minimal Tauri permissions
  tests/                 window config, semantic regression (recorded vectors) and live suites
tests/                   UI tests
scripts/                 Windows smoke test
docs/                    architecture and testing notes
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for design decisions and [docs/TESTING.md](docs/TESTING.md) for what has been verified and how.

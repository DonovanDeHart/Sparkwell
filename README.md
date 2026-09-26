# Sparkwell

**Your Sparks, always within reach.**

Sparkwell is a compact Windows desktop companion that turns your personal library of reusable AI prompts — *Sparks* — into an always-available asset. Press a hotkey, describe what you're trying to accomplish, get the one best Spark, copy it, and get back to work.

> Need → describe the goal → Best Match → **Copy Spark** → paste anywhere → continue working

A **Spark** is a reusable expression of intent: a prompt, workflow, role definition, research protocol, project launcher, or other structure that gives AI trajectory, context, and shape. Sparks are model-agnostic — one Spark pastes cleanly into ChatGPT, Claude, Gemini, Codex, Cursor, or a local model.

## What it does

- **Goal-first retrieval.** Type an outcome in plain language ("I need AI to help me build an MCP server"); Sparkwell returns one decisive **Best Match**, or honestly says there's no strong match and shows the closest Sparks.
- **One-click copy.** *Copy Spark* puts the complete stored Spark on the clipboard. Unpinned, the panel then collapses so focus returns to the app you were using — ready for Ctrl+V.
- **Favorites** for direct, one-click copying of the Sparks you use most.
- **+ Add New Spark** — paste a Spark and save it. Title and summary are optional (derived from the text if left blank). With local AI available, *Smart Add* drafts a title, summary and tags you can edit before saving.
- **Right-edge sidebar** on the monitor you're working on, respecting the taskbar and per-monitor scaling. Pin it to keep it on top; unpinned it collapses on Esc, click-away, or after copying.
- **Your own global shortcut.** On first run Sparkwell asks you to press the shortcut you want (no single combination is free on every machine), validates it, and checks it isn't taken by another app. You can skip and set it later in Settings; the tray icon always opens Sparkwell. If a saved shortcut stops working (another app took it), Sparkwell tells you at startup instead of failing silently.
- **Local-first.** The library is a single SQLite file on your device. No account, no cloud, no telemetry.
- **Optional local intelligence.** If [Ollama](https://ollama.com) is running, retrieval becomes semantic (matching meaning, not just words) and Smart Add becomes available. Without it, everything still works with standard search.

## Keyboard

| Keys | Action |
| --- | --- |
| Your activation shortcut (chosen on first run) | Show / focus / hide Sparkwell from any app |
| `Enter` | Find the best Spark for the goal |
| `Shift+Enter` | New line in the goal |
| `Ctrl+Enter` | Copy the Best Match |
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
2. Pull an embedding model for semantic search:
   ```powershell
   ollama pull nomic-embed-text
   ```
3. Optionally pull a small chat model for Smart Add (any installed chat model works; these are preferred):
   ```powershell
   ollama pull qwen2.5:3b     # or: ollama pull llama3.2
   ```

Sparkwell detects Ollama automatically (it checks in the background and whenever the panel opens), embeds your library in the background, and keeps the index current as you add or edit Sparks. There is no model configuration UI by design. Power users can override the automatic choice with the `SPARKWELL_EMBED_MODEL` / `SPARKWELL_CHAT_MODEL` environment variables.

| Ollama | What you get |
| --- | --- |
| Running with an embedding model | Semantic + lexical hybrid retrieval ("Matched by local intelligence") |
| Running with a chat model | Smart Add metadata drafting |
| Stopped, missing, or busy | Standard search (title, tags, summary, full text); Favorites, Add, Copy, Settings all work |

Sparkwell only talks to `127.0.0.1:11434`, bypasses any proxy, and never uses Ollama "cloud" models, so Spark content does not leave your device.

## Your library

- Default location: `%LOCALAPPDATA%\Sparkwell\Library\sparkwell.db` (a single SQLite file).
- Preferences: `%LOCALAPPDATA%\Sparkwell\config.json` (hotkey, pin state, custom library location).
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
cargo test                                 # core unit tests (storage, search, hotkey, AI parsing, ...)
cargo test --test semantic_live -- --ignored --nocapture   # optional: live retrieval-quality harness (needs Ollama)
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

`scripts/windows-smoke.ps1` installs the built NSIS package and verifies the real app end to end (docking, single instance, global hotkey, restart persistence, uninstall keeps the library). CI runs it on `windows-latest` for every pull request.

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
  tests/                 live semantic retrieval harness (opt-in)
tests/                   UI tests
scripts/                 Windows smoke test
docs/                    architecture and testing notes
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for design decisions and [docs/TESTING.md](docs/TESTING.md) for what has been verified and how.

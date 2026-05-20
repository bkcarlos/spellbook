<h1>
  <img src="assets/icon-128.png" alt="" width="48" align="left" style="margin-right:12px"/>
  Spellbook
</h1>

> Your personal spellbook of shell incantations. Paste a command, fuzzy-search later, copy with one click.

A small, fast desktop app (Rust + egui) for anyone who lives in a terminal — devs, ops, data folks. Stores every shell command worth keeping — your `docker prune`, `ssh tunnel`, `ffmpeg one-liner` — and gets them back in 200ms.

- **Local-first** — SQLite on your disk, no account, no cloud.
- **Paste-and-go** — `Cmd+N`, paste, hit Enter. Title/category/tags inferred automatically.
- **Fuzzy search** — `dkps` finds `docker ps`. Weighted across title/tag/command/category.
- **One-click copy** — `Cmd+Shift+C` from anywhere; Enter on the list.
- **Parameter templates** — write `ssh ${USER}@${HOST}`, get a fill-in dialog before copy.
- **Inline editor** — select a command, edit on the right side. Auto-saves 500ms after you stop typing.
- **Recycle bin** — soft delete with 7-day undo window.
- **AI (optional)** — paste-to-enhance, AI explain, natural-language → command, etc. Off by default; configured in Settings.

---

## Install

### macOS (Homebrew)

Two ways, pick one:

```bash
# CLI binary — adds `spellbook` to PATH, run from a terminal
brew tap bkcarlos/spellbook
brew install spellbook
spellbook

# .app bundle — installs Spellbook.app to /Applications, Dock-friendly,
# Spotlight-searchable, Launchpad icon
brew tap bkcarlos/spellbook
brew install --cask spellbook
open -a Spellbook
```

Then run `spellbook` from a terminal.

### Windows

Download `spellbook-x86_64-pc-windows-msvc.zip` from the
[latest release](https://github.com/bkcarlos/spellbook/releases/latest),
unzip, and run `spellbook.exe`. No installer — the .exe is self-contained.

Optional: move the .exe to a folder on your `PATH` so you can launch it from
the Run dialog (`Win + R`), or pin it to the Start Menu / Taskbar by right-
clicking the .exe in Explorer.

### macOS .app bundle (drag-to-Applications, Dock-friendly)

```bash
git clone https://github.com/bkcarlos/spellbook
cd spellbook
./scripts/build_app.sh           # → target/Spellbook.app
mv target/Spellbook.app /Applications/
open /Applications/Spellbook.app
```

First launch shows a Gatekeeper warning (the app is unsigned). Right-click
→ Open → Open the first time to bypass. After that, just double-click.

To build a universal binary that runs on both arm64 and Intel Macs:

```bash
./scripts/build_app.sh universal
```

### From source (CLI)

Requires Rust 1.75+ (`rustup`).

```bash
git clone https://github.com/bkcarlos/spellbook
cd spellbook
cargo build --release
./target/release/spellbook
```

Pre-built binaries for Linux x86_64 / macOS arm64 / macOS x86_64 are attached
to each [GitHub release](https://github.com/bkcarlos/spellbook/releases).

---

## Keyboard shortcuts

Press `?` (or `F1`) inside the app to see them all.

| Key | Action |
|---|---|
| `Cmd/Ctrl + N` | New command (auto-pastes clipboard) |
| `Cmd/Ctrl + K` | Focus search |
| `Cmd/Ctrl + Shift + C` | Copy selected command |
| `Cmd/Ctrl + D` | Toggle favorite |
| `Cmd/Ctrl + E` | Focus title for renaming |
| `Cmd/Ctrl + I` | AI: generate a command from natural language |
| `↑` / `↓` | Move selection |
| `Enter` | Copy selected (in trash: restore) |
| `Esc` | Close dialog / clear search |
| `?` or `F1` | Show this list |

---

## AI features (opt-in)

All AI features are off by default. To enable:

1. Open **设置** (top bar).
2. Pick a provider: OpenAI-compatible (covers OpenAI, DeepSeek, Ollama-with-/v1) or Anthropic.
3. Supply your API key one of two ways:
   - Set the env var named in Settings (e.g. `export OPENAI_API_KEY=sk-...`), **or**
   - Paste it into Settings and click **Save to Keychain** — stored in the OS
     keychain (macOS Keychain / Windows Credential Manager / libsecret on Linux).
4. Toggle the features you want.

The "paste-to-enhance" feature shows a one-time consent dialog explaining what's
sent before the first call. The app never overwrites a field you've manually
edited. API keys are read from the env var first, then the OS keychain — never
written to the config file.

Data lives in:

- **macOS** `~/Library/Application Support/Spellbook/`
- **Linux** `~/.local/share/Spellbook/`
- **Windows** `%APPDATA%\Spellbook\`

---

## Updates

Spellbook checks GitHub for new releases on startup (result cached 24h).

- **macOS `.app` install** — when a newer version is available, the About dialog
  shows a one-click **Update now** button. It downloads the signed `.dmg`,
  swaps `/Applications/Spellbook.app`, strips quarantine, and offers a restart.
  No `brew`/`curl` needed.
- **All other install paths** (Homebrew CLI, raw `cargo install`, Windows `.exe`,
  Linux tarball) — the dialog links to the GitHub release page; upgrades go
  through your usual channel (`brew upgrade spellbook`, re-download, etc).

---

## Development

```bash
cargo test                    # unit tests (83+)
cargo run                     # debug build
cargo build --release         # 12MB native binary
```

Project structure:

```
src/
├── main.rs        # entry, font bundling
├── app.rs         # UI (egui), inline editor, modals
├── db.rs          # SQLite schema + CRUD
├── inference.rs   # rule-based title/category/tag inference
├── search.rs      # fuzzy-matcher integration
├── llm.rs         # LLM client + manager (OpenAI / Anthropic)
├── installer.rs   # macOS .app one-click self-update (DMG)
├── update.rs      # GitHub release version check
└── models.rs      # domain types
docs/PRD.md        # full product requirements
assets/            # bundled fonts (NotoEmoji, OFL license)
```

---

## License

MIT (see `LICENSE`).

Bundled fonts:

- **NotoEmoji-Regular** — SIL Open Font License 1.1 (see `assets/OFL.txt`).

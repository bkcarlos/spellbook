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

```bash
brew tap bkcarlos/spellbook
brew install spellbook
```

Then run `spellbook` from a terminal.

### From source

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
3. Set the env var holding your API key (e.g. `export OPENAI_API_KEY=sk-...`).
4. Toggle the features you want.

The "paste-to-enhance" feature shows a one-time consent dialog explaining what's
sent before the first call. The app never overwrites a field you've manually
edited. API keys are read from environment variables only — never written to
the config file.

Data lives in:

- **macOS** `~/Library/Application Support/Spellbook/`
- **Linux** `~/.local/share/Spellbook/`
- **Windows** `%APPDATA%\Spellbook\`

---

## Development

```bash
cargo test                    # unit tests (78+)
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
└── models.rs      # domain types
docs/PRD.md        # full product requirements
assets/            # bundled fonts (NotoEmoji, OFL license)
```

---

## License

MIT (see `LICENSE`).

Bundled fonts:

- **NotoEmoji-Regular** — SIL Open Font License 1.1 (see `assets/OFL.txt`).

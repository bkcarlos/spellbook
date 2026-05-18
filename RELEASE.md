# Release process

## Cutting a release

1. Bump version in `Cargo.toml` (e.g. `0.1.0` → `0.2.0`).
2. Commit: `git commit -am "Release v0.2.0"`.
3. Tag: `git tag v0.2.0`.
4. Push: `git push origin main --tags`.

The push of a `v*` tag triggers `.github/workflows/release.yml`, which:

- Cross-compiles for `aarch64-apple-darwin`, `x86_64-apple-darwin`, and
  `x86_64-unknown-linux-gnu`.
- Packages each as a `.tar.gz`.
- Computes SHA-256 for each archive.
- Creates a **draft** GitHub Release with the artifacts attached.

Edit the draft release to add notes, then publish.

## Updating the Homebrew tap

Users install via:

```bash
brew tap bkcarlos/spellbook
brew install spellbook
```

That requires a separate repo named `homebrew-spellbook` containing
`Formula/spellbook.rb`. To bump it after a release:

1. From the workflow output, grab the three SHA-256 values printed by the
   "Print Homebrew formula stanza" step.
2. In the `homebrew-spellbook` repo, edit `Formula/spellbook.rb`:
   - Update `version "X.Y.Z"`.
   - Replace each `sha256 "..."` with the matching value.
3. Commit and push.

A copy of the formula template lives at `Formula/spellbook.rb` in **this**
repo. Keep them in sync.

## First-time tap setup

To create the tap repo:

```bash
# in a new directory
mkdir homebrew-spellbook && cd homebrew-spellbook
git init -b main
mkdir Formula
cp /path/to/spellbook/Formula/spellbook.rb Formula/
git add . && git commit -m "Initial formula"
gh repo create bkcarlos/homebrew-spellbook --public --source=. --push
```

After that, the install command above just works for everyone.

## Local install (no Homebrew)

```bash
cargo install --path .
# or
cargo build --release && cp target/release/spellbook /usr/local/bin/
```

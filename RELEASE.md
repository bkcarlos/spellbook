# Release process

## TL;DR — cutting the next release

Replace `OLD` / `NEW` with the actual versions (e.g. `0.1.8` → `0.1.9`):

```bash
# 1. bump version
sed -i '' 's/^version = "OLD"/version = "NEW"/' Cargo.toml
git commit -am "Release vNEW"

# 2. tag + push — CI does everything else
git tag vNEW
git push origin main --tags
```

Within ~10 minutes:

- ✅ macOS arm64, macOS x86_64, Linux x86_64 CLI binaries built
- ✅ Universal `Spellbook.app` assembled + wrapped in `Spellbook-X.Y.Z.dmg`
- ✅ GitHub Release published with all archives (3 .tar.gz + 1 .dmg)
- ✅ Homebrew **formula** auto-bumped (CLI binary install)
- ✅ Homebrew **cask** auto-bumped (drag-to-Applications .app install)
- ✅ Users can `brew upgrade spellbook` or `brew upgrade --cask spellbook`

## Install paths users can take

```bash
brew install spellbook              # CLI: binary in /opt/homebrew/bin/
brew install --cask spellbook       # GUI: Spellbook.app in /Applications/
```

Both reach the same Spellbook binary internally; the cask just wraps it in a
proper .app bundle so it shows in Dock / Spotlight / Launchpad.

## One-time setup (already done if you're reading this)

### 1. Create the tap repo

GitHub → New repository → `homebrew-spellbook` → Public → empty.

### 2. Create a Personal Access Token (PAT) for CI to push to the tap

GitHub → Settings → Developer settings → Personal access tokens → Fine-grained tokens → Generate new token:

- **Token name**: `spellbook-tap-push`
- **Repository access**: Only select repositories → `bkcarlos/homebrew-spellbook`
- **Permissions** → Repository permissions → **Contents**: Read and write
- **Expiration**: 1 year (set a reminder to rotate)

Copy the token (starts with `github_pat_...`).

### 3. Add it as a secret on the spellbook repo

GitHub → `bkcarlos/spellbook` → Settings → Secrets and variables → Actions → New repository secret:

- **Name**: `TAP_TOKEN`
- **Value**: paste the PAT

If `TAP_TOKEN` is missing, the publish-tap job logs a friendly skip
and does nothing — the build still succeeds, you just need to bump
the formula manually with `scripts/patch_formula.sh`.

## Manual fallback

If CI's tap push fails (PAT expired, etc), fix the formula locally:

```bash
./scripts/patch_formula.sh                 # patches Formula/spellbook.rb in place
cp Formula/spellbook.rb /path/to/homebrew-spellbook/Formula/
( cd /path/to/homebrew-spellbook && git add -A && git commit -m "Bump" && git push )
```

## Install variants

```bash
# Homebrew (recommended)
brew tap bkcarlos/spellbook
brew install spellbook

# Direct binary download
curl -L https://github.com/bkcarlos/spellbook/releases/latest/download/spellbook-aarch64-apple-darwin.tar.gz | tar xz
./spellbook

# From source
cargo install --path .
```

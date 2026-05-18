#!/usr/bin/env bash
# Download release archives, compute SHA-256, patch Formula/spellbook.rb in place.
# Manual fallback for when CI's publish-tap step is skipped (missing TAP_TOKEN
# secret) or you need to re-render the formula locally.
#
# Usage:  ./scripts/patch_formula.sh [VERSION]
# Default version: read from Cargo.toml.
#
# Compatible with macOS default bash 3.2 (no associative arrays).
set -eu
IFS=$'\n\t'

cd "$(dirname "$0")/.."

VERSION="${1:-$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)}"
echo "patching formula for v${VERSION}"

REPO="bkcarlos/spellbook"
BASE="https://github.com/${REPO}/releases/download/v${VERSION}"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

ARM_FILE="spellbook-aarch64-apple-darwin.tar.gz"
INTEL_FILE="spellbook-x86_64-apple-darwin.tar.gz"
LINUX_FILE="spellbook-x86_64-unknown-linux-gnu.tar.gz"

fetch_sha() {
  local file="$1"
  echo "  downloading $file ..." >&2
  if ! curl -sfL "$BASE/$file" -o "$TMP/$file"; then
    echo "  ✗ failed to download $file" >&2
    echo "    (has the v${VERSION} release been published? https://github.com/${REPO}/releases)" >&2
    exit 1
  fi
  shasum -a 256 "$TMP/$file" | awk '{print $1}'
}

ARM_SHA=$(fetch_sha "$ARM_FILE")
INTEL_SHA=$(fetch_sha "$INTEL_FILE")
LINUX_SHA=$(fetch_sha "$LINUX_FILE")

echo "  arm:   $ARM_SHA"
echo "  intel: $INTEL_SHA"
echo "  linux: $LINUX_SHA"

VERSION="$VERSION" ARM_SHA="$ARM_SHA" INTEL_SHA="$INTEL_SHA" LINUX_SHA="$LINUX_SHA" python3 <<'PY'
import os, re
path = "Formula/spellbook.rb"
src = open(path).read()
shas = {
    "aarch64-apple-darwin":      os.environ["ARM_SHA"],
    "x86_64-apple-darwin":       os.environ["INTEL_SHA"],
    "x86_64-unknown-linux-gnu":  os.environ["LINUX_SHA"],
}
# Bump version line
src = re.sub(r'^  version\s+"[^"]+"',
             f'  version "{os.environ["VERSION"]}"',
             src, count=1, flags=re.MULTILINE)
# First pass: replace any leftover REPLACE_WITH_* placeholders
src = src.replace("REPLACE_WITH_ARM64_MAC_SHA256",     shas["aarch64-apple-darwin"])
src = src.replace("REPLACE_WITH_INTEL_MAC_SHA256",     shas["x86_64-apple-darwin"])
src = src.replace("REPLACE_WITH_LINUX_X86_64_SHA256",  shas["x86_64-unknown-linux-gnu"])
# Second pass: replace already-real SHAs (handles re-patching)
for target, sha in shas.items():
    pat = re.compile(
        r'(url\s+"[^"]*' + re.escape(target) + r'[^"]*"\s*\n\s*sha256\s+)"[^"]*"'
    )
    src = pat.sub(lambda m: f'{m.group(1)}"{sha}"', src)
open(path, "w").write(src)
print(f"  patched {path}")
PY

echo
echo "Formula ready. Next steps:"
echo "  1. clone tap:  git clone git@github.com:bkcarlos/homebrew-spellbook.git"
echo "  2. cp Formula/spellbook.rb /path/to/tap/Formula/"
echo "  3. cd tap && git add -A && git commit -m \"Bump spellbook to ${VERSION}\" && git push"

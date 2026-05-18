#!/usr/bin/env bash
# Download release archives, compute SHA-256, patch Formula/spellbook.rb.
# Usage:  ./scripts/patch_formula.sh [VERSION]
# Default version: read from Cargo.toml.
set -euo pipefail

cd "$(dirname "$0")/.."

VERSION="${1:-$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)}"
echo "patching formula for v${VERSION}"

REPO="bkcarlos/spellbook"
BASE="https://github.com/${REPO}/releases/download/v${VERSION}"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

declare -A FILES=(
  [arm64_mac]="spellbook-aarch64-apple-darwin.tar.gz"
  [intel_mac]="spellbook-x86_64-apple-darwin.tar.gz"
  [linux]="spellbook-x86_64-unknown-linux-gnu.tar.gz"
)
declare -A SHAS

for key in arm64_mac intel_mac linux; do
  f="${FILES[$key]}"
  echo "  downloading $f ..."
  if ! curl -sfL "$BASE/$f" -o "$TMP/$f"; then
    echo "  ✗ failed to download $f — has the draft release been published?"
    echo "    https://github.com/${REPO}/releases"
    exit 1
  fi
  sha=$(shasum -a 256 "$TMP/$f" | awk '{print $1}')
  SHAS[$key]=$sha
  echo "  $key: $sha"
done

# Patch the formula in place using a Python rewrite (handles bumping version + 3 SHAs)
python3 - <<PY
import re
path = "Formula/spellbook.rb"
with open(path) as f:
    s = f.read()
s = re.sub(r'version\s+"[^"]+"', 'version "${VERSION}"', s, count=1)
s = s.replace("REPLACE_WITH_ARM64_MAC_SHA256",   "${SHAS[arm64_mac]}")
s = s.replace("REPLACE_WITH_INTEL_MAC_SHA256",   "${SHAS[intel_mac]}")
s = s.replace("REPLACE_WITH_LINUX_X86_64_SHA256","${SHAS[linux]}")
# Also handle re-patching: replace any existing arm/intel/linux sha256 line with the new value
# (one block at a time; safe because each URL is unique within its on_* branch)
def repatch(text, marker_url, new_sha):
    # find the line containing this URL, then replace the next sha256 line
    lines = text.splitlines()
    for i, line in enumerate(lines):
        if marker_url in line:
            for j in range(i+1, min(i+4, len(lines))):
                if 'sha256' in lines[j]:
                    lines[j] = re.sub(r'sha256\s+"[^"]+"', f'sha256 "{new_sha}"', lines[j])
                    break
            break
    return "\n".join(lines) + ("\n" if text.endswith("\n") else "")
s = repatch(s, "aarch64-apple-darwin",         "${SHAS[arm64_mac]}")
s = repatch(s, "x86_64-apple-darwin",          "${SHAS[intel_mac]}")
s = repatch(s, "x86_64-unknown-linux-gnu",     "${SHAS[linux]}")
with open(path, "w") as f:
    f.write(s)
print(f"  patched {path}")
PY

echo
echo "Formula ready. Next steps:"
echo "  1. Create repo bkcarlos/homebrew-spellbook (empty)"
echo "  2. cp Formula/spellbook.rb to that repo as Formula/spellbook.rb"
echo "  3. Commit & push"
echo "  4. Verify:  brew tap bkcarlos/spellbook && brew install spellbook"

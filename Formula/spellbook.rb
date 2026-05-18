class Spellbook < Formula
  desc "Local-first command vault — your spellbook of shell incantations"
  homepage "https://github.com/bkcarlos/spellbook"
  version "0.1.0"
  license "MIT"

  # ---------------------------------------------------------------------------
  # After each release, run `.github/workflows/release.yml` (push a vX.Y.Z tag),
  # then copy the SHA-256 values from the workflow output into the three
  # `sha256` lines below. Bump `version` to match.
  # ---------------------------------------------------------------------------

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/bkcarlos/spellbook/releases/download/v#{version}/spellbook-aarch64-apple-darwin.tar.gz"
      sha256 "03f60d97336393d0c06ac87871bd5518b4bc2be7f3907b56666abb10fa82aac1"
    else
      url "https://github.com/bkcarlos/spellbook/releases/download/v#{version}/spellbook-x86_64-apple-darwin.tar.gz"
      sha256 "4a5c5bd3c3b3c0a75c2fb00666e0a19d4be877f2ea72fcc70c79b237f3df9615"
    end
  end

  on_linux do
    url "https://github.com/bkcarlos/spellbook/releases/download/v#{version}/spellbook-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "a44a6848e92bfeed47135a9239d9f459a9167c5f8bb6b67fa09bfc79cd421324"
  end

  def install
    bin.install "spellbook"
  end

  def caveats
    <<~EOS
      Spellbook stores its data in:
        macOS: ~/Library/Application Support/Spellbook/
        Linux: ~/.local/share/Spellbook/

      Launch with:
        spellbook

      Press `?` inside the app to see all keyboard shortcuts.
    EOS
  end

  test do
    assert_predicate bin/"spellbook", :executable?
  end
end

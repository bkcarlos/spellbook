cask "spellbook" do
  version "0.1.0"
  sha256 "REPLACE_WITH_DMG_SHA256"

  url "https://github.com/bkcarlos/spellbook/releases/download/v#{version}/Spellbook-#{version}.dmg"
  name "Spellbook"
  desc "Local-first command vault — your spellbook of shell incantations"
  homepage "https://github.com/bkcarlos/spellbook"

  # ---------------------------------------------------------------------------
  # After each release, scripts/patch_cask.sh (or CI's publish-tap job)
  # downloads Spellbook-X.Y.Z.dmg, computes SHA-256, and replaces the
  # placeholder above + bumps the version string.
  # ---------------------------------------------------------------------------

  livecheck do
    url :url
    strategy :github_latest
  end

  app "Spellbook.app"

  # Spellbook isn't notarized with an Apple Developer ID (would cost
  # $99/yr), so Homebrew's default quarantine attribute would trigger
  # Gatekeeper's "cannot verify developer" warning on first launch.
  # The binary IS ad-hoc signed (rust linker does that for arm64 macOS),
  # so once quarantine is removed it launches fine.
  postflight do
    system_command "/usr/bin/xattr",
                   args:         ["-dr", "com.apple.quarantine", "#{appdir}/Spellbook.app"],
                   must_succeed: false
  end

  zap trash: [
    "~/Library/Application Support/Spellbook",
    "~/Library/Caches/Spellbook",
    "~/.config/Spellbook",
  ]
end

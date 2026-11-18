# Homebrew cask. tools/release.ps1 fills in the SHA-256 of each .dmg attached
# to the GitHub release; the publisher submits it to a tap (that needs the
# user's account). Unsigned builds: Gatekeeper asks once (right-click > Open).
cask "uni-term" do
  arch arm: "arm64", intel: "x86_64"

  version "0.1.0"
  sha256 arm:   "SHA256-OF-THE-ARM64-DMG",
         intel: "SHA256-OF-THE-X86_64-DMG"

  url "https://github.com/OWNER/uni-term/releases/download/v#{version}/uni-term-#{version}-macos-#{arch}.dmg"
  name "uni-term"
  desc "One terminal for Windows, macOS and Linux"
  homepage "https://github.com/OWNER/uni-term"

  app "uni-term.app"
  binary "#{appdir}/uni-term.app/Contents/MacOS/ut"

  zap trash: [
    "~/Library/Application Support/uni-term",
  ]
end
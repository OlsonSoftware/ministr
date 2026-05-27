# Formula for the OlsonSoftware/tap homebrew tap.
# To set up the tap:
#   1. Create a new GitHub repo: OlsonSoftware/homebrew-tap
#   2. Copy this file to Formula/ministr.rb in that repo
#   3. Update SHA256 hashes after each release (from .sha256 files in GitHub Release assets)
#   4. Users install with: brew install OlsonSoftware/tap/ministr
#
# To update after a release:
#   1. Download the new .sha256 files from the GitHub Release
#   2. Update the version and sha256 values below
#   3. Push to the homebrew-tap repo

class Ministr < Formula
  desc "Code intelligence MCP server for AI coding agents — semantic code search, symbol navigation, and cross-language bridge detection"
  homepage "https://ministr.ai"
  version "0.1.0"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/OlsonSoftware/ministr/releases/download/v#{version}/ministr-aarch64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_ARM64_SHA256"
    else
      url "https://github.com/OlsonSoftware/ministr/releases/download/v#{version}/ministr-x86_64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_X86_64_SHA256"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/OlsonSoftware/ministr/releases/download/v#{version}/ministr-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER_LINUX_ARM64_SHA256"
    else
      url "https://github.com/OlsonSoftware/ministr/releases/download/v#{version}/ministr-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER_LINUX_X86_64_SHA256"
    end
  end

  def install
    bin.install "ministr"
  end

  test do
    assert_match "ministr", shell_output("#{bin}/ministr --version")
  end
end

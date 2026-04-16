# Formula for the AlrikOlson/tap homebrew tap.
# To set up the tap:
#   1. Create a new GitHub repo: AlrikOlson/homebrew-iris
#   2. Copy this file to Formula/iris.rb in that repo
#   3. Update SHA256 hashes after each release (from .sha256 files in GitHub Release assets)
#   4. Users install with: brew install AlrikOlson/tap/iris
#
# To update after a release:
#   1. Download the new .sha256 files from the GitHub Release
#   2. Update the version and sha256 values below
#   3. Push to the homebrew-iris repo

class Iris < Formula
  desc "Context cache controller for LLM agents — MCP server with session tracking, prefetch, and budget management"
  homepage "https://github.com/AlrikOlson/iris-rs"
  version "0.1.0"
  license any_of: ["MIT", "Apache-2.0"]

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/AlrikOlson/iris-rs/releases/download/v#{version}/iris-aarch64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_ARM64_SHA256"
    else
      url "https://github.com/AlrikOlson/iris-rs/releases/download/v#{version}/iris-x86_64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER_X86_64_SHA256"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/AlrikOlson/iris-rs/releases/download/v#{version}/iris-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER_LINUX_ARM64_SHA256"
    else
      url "https://github.com/AlrikOlson/iris-rs/releases/download/v#{version}/iris-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER_LINUX_X86_64_SHA256"
    end
  end

  def install
    bin.install "iris"
  end

  test do
    assert_match "iris", shell_output("#{bin}/iris --version")
  end
end

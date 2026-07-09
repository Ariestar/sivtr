# Homebrew formula for sivtr.
#
# This is a TEMPLATE consumed by the `homebrew` job in
# .github/workflows/release.yml: on each release it substitutes @VERSION@,
# @SOURCE_URL@ and @SOURCE_SHA256@ and pushes the result to the
# Ariestar/homebrew-sivtr tap, so `brew install ariestar/sivtr/sivtr` works.
#
# It builds from the release source archive (the workspace includes
# crates/sivtr-core as a path dependency, and Cargo.lock is committed, so
# `cargo install --locked` works offline-then-fetch for deps).
class Sivtr < Formula
  desc "Local workspace memory for terminal output and AI coding sessions"
  homepage "https://github.com/Ariestar/sivtr"
  url "@SOURCE_URL@"
  sha256 "@SOURCE_SHA256@"
  version "@VERSION@"
  license "Apache-2.0"
  head "https://github.com/Ariestar/sivtr.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/sivtr --version")
  end
end

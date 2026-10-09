# Homebrew formula for the `runway` CLI. Publish to a tap repo
# (Dvorinka/homebrew-tap) at release time; replace the sha256
# placeholders with the digests of the v{version} release tarballs:
#   curl -sL <url> | shasum -a 256
class Runway < Formula
  desc "Runway — self-hosted deployment platform CLI"
  homepage "https://github.com/Dvorinka/runway"
  version "0.2.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Dvorinka/runway/releases/download/v#{version}/runway-darwin-arm64.tar.gz"
      sha256 "b96c36c7a08c7ccdae46bcb0731f59949cdbd171cdb6b88949a909728f126cd2"
    end
    on_intel do
      url "https://github.com/Dvorinka/runway/releases/download/v#{version}/runway-darwin-x64.tar.gz"
      sha256 "0834120d965ea9d12c85e2401fbea04d25d04f447eb2bacf23440f26dcd51265"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Dvorinka/runway/releases/download/v#{version}/runway-linux-arm64.tar.gz"
      sha256 "e5f0dafacb09f0b222b66abfbc1ffdf7ff452efecbb008810002e2a085de0b5e"
    end
    on_intel do
      url "https://github.com/Dvorinka/runway/releases/download/v#{version}/runway-linux-x64.tar.gz"
      sha256 "73827e1265d12521f463062a6af7b8a838e5fa439a1d71d88da48fa9ee70069a"
    end
  end

  def install
    bin.install "runway"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/runway --version")
  end
end

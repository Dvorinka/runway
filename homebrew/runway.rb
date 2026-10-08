# Homebrew formula for the `runway` CLI. Publish to a tap repo
# (Dvorinka/homebrew-tap) at release time; replace the sha256
# placeholders with the digests of the v{version} release tarballs:
#   curl -sL <url> | shasum -a 256
class Runway < Formula
  desc "Runway — self-hosted deployment platform CLI"
  homepage "https://github.com/Dvorinka/runway"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Dvorinka/runway/releases/download/v#{version}/runway-darwin-arm64.tar.gz"
      sha256 "UPDATE_ON_RELEASE"
    end
    on_intel do
      url "https://github.com/Dvorinka/runway/releases/download/v#{version}/runway-darwin-x64.tar.gz"
      sha256 "UPDATE_ON_RELEASE"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Dvorinka/runway/releases/download/v#{version}/runway-linux-arm64.tar.gz"
      sha256 "UPDATE_ON_RELEASE"
    end
    on_intel do
      url "https://github.com/Dvorinka/runway/releases/download/v#{version}/runway-linux-x64.tar.gz"
      sha256 "UPDATE_ON_RELEASE"
    end
  end

  def install
    bin.install "runway"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/runway --version")
  end
end

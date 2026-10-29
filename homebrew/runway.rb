# Homebrew formula for the `runway` CLI. Publish to a tap repo
# (Dvorinka/homebrew-tap) at release time; replace the sha256
# placeholders with the digests of the v{version} release tarballs:
#   curl -sL <url> | shasum -a 256
class Runway < Formula
  desc "Runway — self-hosted deployment platform CLI"
  homepage "https://github.com/Dvorinka/runway"
  version "0.3.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Dvorinka/runway/releases/download/v#{version}/runway-darwin-arm64.tar.gz"
      sha256 "6032a1d61cb28e440ace9247aab568151bd60ff65c1095c4d1fb7d716618b3ff"
    end
    on_intel do
      url "https://github.com/Dvorinka/runway/releases/download/v#{version}/runway-darwin-x64.tar.gz"
      sha256 "15eb05e7085c5a875517ee2501ba4e361b7c20dbb00f07f578c9a9f6adcda906"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Dvorinka/runway/releases/download/v#{version}/runway-linux-arm64.tar.gz"
      sha256 "0147c35a879662971bc7fb95eb21a9aac938350a32db6b341fdbf864c4605e22"
    end
    on_intel do
      url "https://github.com/Dvorinka/runway/releases/download/v#{version}/runway-linux-x64.tar.gz"
      sha256 "0c6c5fa2bb7c839b084490e0bb9ea3e49857e2367686bfb9e41c9b93c3f0c895"
    end
  end

  def install
    bin.install "runway"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/runway --version")
  end
end

# 此文件仅为初始模板,真源在 dengmengmian/homebrew-tap/Formula/reviewgate.rb,
# 由 release.yml 的 homebrew job 在发版后自动更新;这里的 version/sha256 可能过期。
class Reviewgate < Formula
  desc "Pre-merge quality gate for AI-generated code"
  homepage "https://dengmengmian.github.io/ReviewGate/"
  version "0.7.0"
  license "MIT"

  livecheck do
    url :stable
    strategy :github_latest
  end

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/dengmengmian/ReviewGate/releases/download/v#{version}/reviewgate-darwin-arm64"
      sha256 "b33eae89818584297b8369da3de0f0289cb59d74d2b0b22d21d4e1bd34d58a07"
    else
      url "https://github.com/dengmengmian/ReviewGate/releases/download/v#{version}/reviewgate-darwin-x64"
      sha256 "9d267f88a0582a4b8c67dd9959082c0c64fba947a03cd1a1e653dc1df552c5b4"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/dengmengmian/ReviewGate/releases/download/v#{version}/reviewgate-linux-arm64"
      sha256 "e06f6a0d81b33d2db0b50f667e8524f5827a5eb77e770a60e74ffc34f0e73205"
    else
      url "https://github.com/dengmengmian/ReviewGate/releases/download/v#{version}/reviewgate-linux-x64"
      sha256 "579d36f4f976cec6c8baa61177f51a27ed38d2bd50b8bcfd501adb42a2ec3d1a"
    end
  end

  def install
    bin.install Dir["reviewgate-*"].first => "reviewgate"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/reviewgate --version")
  end
end

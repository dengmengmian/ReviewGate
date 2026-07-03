class Reviewgate < Formula
  desc "Pre-merge quality gate for AI-generated code"
  homepage "https://dengmengmian.github.io/ReviewGate/"
  version "0.5.0"
  license "MIT"

  livecheck do
    url :stable
    strategy :github_latest
  end

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/dengmengmian/ReviewGate/releases/download/v#{version}/reviewgate-darwin-arm64"
      sha256 "2a32ad3eb346570cb45444a5dc2f8c64f98b8fbed67f515514e287b6dcefdbb2"
    else
      url "https://github.com/dengmengmian/ReviewGate/releases/download/v#{version}/reviewgate-darwin-x64"
      sha256 "246a42e18b8353118fd1579a8460504e32210ad3f33a612ee11f6a267e149ad8"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/dengmengmian/ReviewGate/releases/download/v#{version}/reviewgate-linux-arm64"
      sha256 "6e2a89acf267b8ea16e5dda8170fbd0d1c833ff7d005689eaf5743dbcbc394e1"
    else
      url "https://github.com/dengmengmian/ReviewGate/releases/download/v#{version}/reviewgate-linux-x64"
      sha256 "f99e71af3455cb9857ee505c8068204142b9bf66eaf19e83fd3215e095275e13"
    end
  end

  def install
    bin.install Dir["reviewgate-*"].first => "reviewgate"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/reviewgate --version")
  end
end

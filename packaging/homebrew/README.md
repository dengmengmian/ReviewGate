# Homebrew Formula 发布

`reviewgate.rb` 是 ReviewGate CLI 的 Homebrew formula，发布在 `dengmengmian/homebrew-tap`（与 AgentGate cask 同一个 tap）。

用户安装：

```bash
brew install dengmengmian/tap/reviewgate
```

## 每次发版更新

新版本发布后，`version` 和四个 `sha256` 要同步更新，校验和直接取 release 附带的 `sha256sum.txt`：

```bash
VERSION=0.6.0
curl -sL "https://github.com/dengmengmian/ReviewGate/releases/download/v${VERSION}/sha256sum.txt"
```

改完复制到 tap 仓库 `Formula/reviewgate.rb` 并推送。后续可在 release workflow 里自动化（用 PAT 推 tap 仓库）。

## 本地验证

```bash
brew style dengmengmian/tap
brew install dengmengmian/tap/reviewgate && brew test reviewgate
```

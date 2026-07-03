# Homebrew Formula 发布

`reviewgate.rb` 是 ReviewGate CLI 的 Homebrew formula，发布在 `dengmengmian/homebrew-tap`（与 AgentGate cask 同一个 tap）。

用户安装：

```bash
brew install dengmengmian/tap/reviewgate
```

## 每次发版更新

release.yml 的 `homebrew` job 会在发版后自动更新 tap 仓库的 formula（version + 四个平台 sha256，
取自 release 的 sha256sum.txt），需要仓库 Secret `HOMEBREW_TAP_TOKEN`（对 dengmengmian/homebrew-tap
有 contents:write 的 fine-grained PAT）。crates.io 发布也由 `crates` job 自动完成（需要 Secret
`CARGO_REGISTRY_TOKEN`）。本目录的 `reviewgate.rb` 是模板/参考副本，结构改动时需与 tap 仓库同步。

## 本地验证

```bash
brew style dengmengmian/tap
brew install dengmengmian/tap/reviewgate && brew test reviewgate
```

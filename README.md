<p align="center">
  <img src="docs/assets/logo.svg" alt="ReviewGate" width="420">
</p>

<p align="center">
  合并前先让 AI 审一遍 AI 写的代码：<b>优先拦住高风险问题，少看低价值 review 噪音</b>
</p>

<p align="center">
  <a href="README.en.md">English</a> · 简体中文
</p>

<p align="center">
  <a href="https://github.com/dengmengmian/ReviewGate/actions/workflows/ci.yml"><img src="https://github.com/dengmengmian/ReviewGate/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/dengmengmian/ReviewGate/releases/latest"><img src="https://img.shields.io/github/v/release/dengmengmian/ReviewGate" alt="Release"></a>
  <img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT">
</p>

ReviewGate 用在 PR 合并前，给 AI 生成或 AI 大量参与的代码做二次审查。它不替代人工 review，而是先帮 reviewer 过滤一遍：**高风险问题推到前面，低置信反馈默认折叠**。

## 30 秒跑起来

你只需要三样东西：一个 git 仓库、一个 LLM API key、`reviewgate` 命令。

```bash
# 1) 安装
curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/install.sh | sh

# 2) 写一份全局配置，之后所有仓库都能用
mkdir -p ~/.reviewgate
cat > ~/.reviewgate/config.toml <<'EOF'
provider = "deepseek"

[providers.deepseek]
protocol = "openai"
base_url = "https://api.deepseek.com/v1"
model = "deepseek-v4-pro"
# api_key = "sk-..."   # 可写在这里；更推荐用下方环境变量，避免把密钥提交进仓库
EOF

# 3) 用环境变量放 key（优先级高于配置文件里的 api_key），不写进配置文件
export REVIEWGATE_API_KEY="你的 key"

# 4) 确认模型能连上
reviewgate llm test

# 5) 进入任意有改动的 git 仓库，开始审查
cd /path/to/your/repo
reviewgate review
```

看到 `BLOCK` 表示有高置信问题建议先处理；看到 `WARN` 表示有风险或审查未完整完成；`PASS` 表示没有发现达到闸口阈值的问题。

Windows 用户可以用 PowerShell 安装：

```powershell
irm https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/install.ps1 | iex
```

> **升级**：重新运行上面的安装命令即可——`install.sh` / `install.ps1` 总是拉取最新 release 并覆盖旧版本；或直接 `reviewgate upgrade` 自更新到最新版。

## 适合解决什么

| 场景 | ReviewGate 帮你做什么 |
|---|---|
| AI 一次改了很多文件 | 先按安全、性能、逻辑等维度扫一遍，减少人工漏看 |
| review 评论太多太散 | 合并重复发现，默认折叠低置信反馈 |
| 担心 AI 写出“看起来对、其实错”的代码 | 专门检查幻觉 API、假设漂移、复制后未适配 |
| 团队有业务规则 | 把权限、金额、状态机等规则写进配置，每次审查自动带上 |
| 想确认实现是否真的符合需求/设计 | 传入本次意图（`--intent`），独立 Agent 跨文件审「实现 vs 意图」，输出验收清单 |
| 想在 CI 里加一道闸口 | 高置信问题可阻断合并，未审完不会被当成干净通过 |

<details>
<summary><b>它是怎么审的？</b></summary>

启动多个并行 Agent，分维度审查你的改动：

| 维度 | 关注 |
|---|---|
| 🔒 security | 注入、越权、密钥泄露、不安全反序列化 |
| ⚡ perf | N+1、无谓拷贝、热路径复杂度、阻塞调用 |
| 🧠 logic | 边界条件、空值、错误处理、并发竞态 |
| 📐 style | 命名、可读性、重复代码 |
| 🤖 ai_smell | 幻觉 API、看似合理实则错误、假设漂移、复制未适配 |
| 📋 business | 项目业务规则、权限边界、状态机、金额/订单/库存（配置 `[business].rules` 后自动启用） |

然后：

1. **行号直报 + 校验** —— LLM 直接抄标注行号，引擎用代码片段锚点校验/兜底，降低行号漂移。
2. **跨维度去重 + 一致性加分** —— 同一处被多个维度标记时合并，并提升置信度。
3. **证伪 Judge** —— 每条发现都被独立验证，带证据单次裁决，证不掉才保留。
4. **置信度闸口** —— 高置信问题阻断合并，低置信"废话"默认折叠（仍可展开查看，透明）。

只读安全边界、prompt 缓存复用、确定性重复函数检测、墙钟超时兜底见下文。

</details>

## 安装方式

```bash
# Linux / macOS
curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/install.sh | sh
```

```powershell
# Windows (PowerShell)
irm https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/install.ps1 | iex
```

如果你不想直接执行远程脚本，可以先下载 `install.sh` / `install.ps1` 审阅，或从 GitHub Releases 手动下载对应平台的二进制。

或从源码：`cargo install --path crates/cli`（Windows 需 VS Build Tools 以编译 tree-sitter）

## 配置

ReviewGate 自带 0 个模型。你需要提供一个 OpenAI 兼容或 Anthropic 的 LLM 端点。

建议把通用配置放到 `~/.reviewgate/config.toml`，这样所有仓库都能直接用；只有项目需要覆盖规则时，再在项目根目录放 `reviewgate.toml`。

**最小配置**只要一个 provider：

```toml
provider = "deepseek"

[providers.deepseek]
protocol = "openai"          # OpenAI 兼容（DeepSeek/Kimi/GLM/通义…）；用 Anthropic 则填 "anthropic"
base_url = "https://api.deepseek.com/v1"
model    = "deepseek-v4-pro"
api_key  = "sk-..."          # 可写在这里；CI/共享环境更推荐用环境变量（见下，优先级更高）
```

API key 推荐用环境变量（覆盖配置文件里的 `api_key`）：

```bash
export REVIEWGATE_API_KEY="sk-..."
```

<details>
<summary><b>可选：闸口阈值 · 业务规则 · 组织 skill · 配置位置</b>（点开）</summary>

```toml
[gate]
block_threshold = 0.8        # 置信度 ≥ 0.8 阻断合并
warn_threshold  = 0.5        # ≥ 0.5 警告，更低默认折叠

# 项目业务规则：配置后自动启用 business 维度，规则编号 [B1].. 可追溯
[business]
rules = [
  "金额字段必须使用整数分，禁止 float",
  "用户级资源访问必须校验 owner_id",
]
# rules_dir  = ".reviewgate/rules"  # <语言>.md 按改动语言注入；business.md 等始终注入
# skills_dir = ".claude/skills"     # 复用组织已写成 skill 的 review 规则（自动剥 frontmatter）
```

- **配置发现顺序**（找到即用）：`REVIEWGATE_CONFIG` 指定路径 → 当前目录 `./reviewgate.toml`（项目级覆盖）→ `~/.reviewgate/config.toml`（全局默认）。
- **CI 注入密钥**：用 `REVIEWGATE_API_KEY` 避免提交明文（同样支持 `REVIEWGATE_BASE_URL` / `REVIEWGATE_MODEL`）。
- **组织 skill 复用**：`skills_dir` 支持 `<子目录>/SKILL.md` 与扁平 `*.md`；与 `rules_dir`（纯规则 md）可同时用。

</details>

## 接入方式

ReviewGate 一个引擎，三种形态——**CLI 为主，Skill / Action 都是调 CLI 的薄壳**。

### 1. CLI（主形态）

```bash
reviewgate review                       # 审查当前改动，默认 5 维度；配置业务规则后自动加 business
reviewgate review --dimensions security,logic
reviewgate review --format json         # 机器可读
reviewgate review --no-judge            # 更快，误报略多
reviewgate review --show-filtered       # 展开被过滤的低置信项
reviewgate review --fail-on block       # BLOCK → 退出码 1（CI 用）
reviewgate review --timeout 120         # 单维度墙钟上限（秒），超时跳过该维度保留其余
reviewgate review --samples 3           # 每维度多采样取并集，提升 flaky 漏报（如 SSRF）的召回稳定性
reviewgate review --fix                 # 逐条 y/N 确认后把建议代码应用到工作区（锚点校验防改错）
reviewgate review --judge-concurrency 4 # 限制 Judge 并发，避免候选多时触发限流
reviewgate review --verbose             # 打印每维度轮数 + token/缓存命中率
reviewgate review --commit <sha>        # 审查单个 commit；或 --from <base> --to <head>
reviewgate review --intent spec.md      # 额外做「实现 vs 意图」技术评审（传入需求/设计/验收标准）
reviewgate review --commit <sha> --intent-from-commit  # 用该 commit 的提交信息作为意图
```

### 意图 / 技术评审（`--intent`）

缺陷评审不需要知道「本该做什么」；**技术评审需要**。传入本次改动的意图（需求/设计/验收标准，文件或 `-` 读 stdin），ReviewGate 会**额外**起一个**独立的整体性 Agent**——从 diff 出发主动跨文件追调用方、契约、测试，判断实现是否完整、正确地满足意图，输出一份**验收清单**（每条标准 ✓满足 / ✗缺失 / ✗破坏 / ⚠不符 / •建议）。意图会被**拆成 N 条验收标准（C1..CN）逐条核对**；没被逐条裁决的标准兜底标 `? 未核对`（不留空清单），只要存在未核对标准就**降级 WARN**，绝不伪装 PASS。它与常驻的 `business.rules` 正交：规则是不变量，`--intent` 是每次不同的「这次该做什么」。不传 `--intent` 时零开销。

```bash
reviewgate review --from main --to HEAD --intent docs/requirement.md
```

`--exec-verify` 会让模型生成的自包含 JS/Python 片段在本机运行以验证边界用例。它默认关闭，且当前只是临时目录 + 清空环境 + 超时的**弱隔离**，不是 OS 级沙箱；只建议在可信或隔离的 CI 环境使用。

ReviewGate 会按 `REVIEWGATE_OUTPUT_LANGUAGE` 或终端 locale（`LC_ALL` / `LC_MESSAGES` / `LANG`）要求模型输出 finding 文案；例如 `REVIEWGATE_OUTPUT_LANGUAGE="Chinese (Simplified)" reviewgate review`。

输出示例：

```
ReviewGate: BLOCK

1 files changed · 1 must fix · 0 warnings · 3 filtered
LLM: 输入 120k tok（缓存命中 88%）· 输出 2.1k tok

Must Fix

1. handler.rs:3
   security / high / confidence 1.00

   SQL 注入：用户输入 req.user_id 通过 format! 直接拼接到 DELETE 语句…

   Current:
     -   let q = format!("DELETE FROM users WHERE id = {}", req.user_id);
   Suggested patch:
     +   let q = "DELETE FROM users WHERE id = $1";

Not Shown
  3 low-confidence findings hidden. Run with --show-filtered to inspect them.
```

**退出码（CI 闸口用）**：`BLOCK → 1`，否则 `0`；用 `--fail-on block|warn|never` 调整。

```bash
# CI 里（慢端点加超时兜底）
REVIEWGATE_API_KEY=$SECRET reviewgate review --timeout 300 --fail-on block
```

**调试子命令**（开发用）：

```bash
reviewgate diff                                  # 看解析后的 diff 摘要
reviewgate tool find_callers '{"symbol":"foo"}'  # 单独试某个工具（tree-sitter）
reviewgate agent --dimension logic               # 只跑单维度看原始输出
```

### 2. Claude Code Skill

**个人用**：把 `integrations/claude-skill/SKILL.md` 拷到 `~/.claude/skills/reviewgate/`，在 Claude Code 里说"审查我的改动"即可触发。

**团队/组织接入（推荐）**：在你的项目根目录一键装入并提交，全队共享、且用你们自己的规则：

```bash
curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/integrations/claude-skill/install-into-project.sh | sh
```

它会在你的仓库里生成（幂等，不覆盖已存在文件）：
- `.claude/skills/reviewgate/SKILL.md` —— **团队共享 skill**（提交后全队 Claude Code 自动可用）。
- `.reviewgate/rules/business.md` —— **组织业务规则**（每次审查都注入，改成你们的真实约定，用 `[B1]/[B2]` 编号便于追溯）。
- `.reviewgate/rules/<语言>.md` —— 语言起步规则按改动语言注入。ReviewGate 内置 **45 种语言**规则（Python/Go/JS/TS/Rust/Java/C/C++/C#/Ruby/PHP/Swift/Kotlin/Scala/Dart/Objective-C/Lua/Perl/Haskell/Elixir/Erlang/Clojure/Groovy/Julia/R/OCaml/F#/Zig/Nim/Crystal/仓颉/Shell/PowerShell/HTML/CSS/Vue/Svelte/SQL/GraphQL/Solidity/Fortran/COBOL/Pascal/Dockerfile/Terraform），无需拷贝。
  自建 `<语言>.md` 可覆盖或追加；也可用 `[business] builtin_language_rules=false` 整体关闭内置语言规则。
- `reviewgate.toml` —— 配置模板（端点 + `[business].rules_dir`；密钥用 `REVIEWGATE_API_KEY` 环境变量注入）。

> **skill 对所有组织相同（薄壳）；差异化在你们提交的 `.reviewgate/rules/`**——`business.md` 写组织专属规则，`<语言>.md` 可直接用起步库，也可替换成你们自己的语言约定。提交 `.claude/` + `.reviewgate/` 即完成团队接入。

### 3. GitHub Action（PR 闸口）

把 `integrations/github-action/example-workflow.yml` 放到 `.github/workflows/`，在仓库 Secrets 配置 `REVIEWGATE_API_KEY`。PR 上自动审查、发摘要评论、按置信度阻断合并。

## 为什么可信

- 自研 Agent 编排与 LLM 客户端，**零 SDK 依赖**（reqwest 直连，OpenAI/Anthropic 双协议）。
- 工具集刻意**只读 + 结构化上报**（不照搬通用编码 Agent 的写/任意 shell）；
  路径限制（`confine_path`）确保不读仓库外文件。
- 代码上下文检索：tree-sitter 精确符号检索 + 函数体提取（重复检测）。
- prompt 缓存复用（system 通用化 + diff 大块缓存断点，跨维度/跨轮）。

### 可插拔 / 可扩展

每一层都是可替换或可叠加的，按需启用、缺了优雅降级：

- **LLM 提供方**：`LlmClient` trait + 双协议（OpenAI 兼容 / Anthropic）。配置即切换 DeepSeek / Kimi / GLM / 通义 / Claude 等，无需改代码。
- **代码检索后端**：`CodeIndex` trait —— `GrepIndex`（默认、即用）/ `TreeSitterIndex`（AST 精确）。`find_definition/callers/references` 的**工具签名不变**，换后端 Agent 那层不动；未支持语言自动回退按行匹配。
- **规则分层可叠加**：45 种**内置语言规则**（`builtin_language_rules` 可整体关）＋ `rules_dir/<语言>.md`（覆盖/追加，优先级更高）＋ `skills_dir`（直接吃组织已有的 review skill）＋ `[business].rules` 内联业务规则。
- **外部程序可选可插拔**：`git` 是唯一硬依赖；ripgrep / linter / 类型检查器等**检测到才用，缺了降级为纯 LLM**——绝不强制用户装一堆。
- **执行验证 opt-in**：`--exec-verify` 的沙箱 `run_check` 默认关闭，保留只读信任边界。
- **三形态薄壳**：CLI / Claude Skill / GitHub Action 都是同一 core 引擎的薄包装。

变更记录见 [`CHANGELOG.md`](CHANGELOG.md)，贡献指南见 [`CONTRIBUTING.md`](CONTRIBUTING.md)。

## 公开评测

以下结果来自 `docs/evals/` 中留痕的公开样本，不是通用准确率承诺。当前样本主要用 `deepseek-v4-pro` 真实跑通（[`docs/evals/`](docs/evals/) · [总览](docs/evals/README.md)）：

- **精度**：在已记录的真实 PR、45 语言干净样例和真实合并 commit 样本中，未观察到误 BLOCK；疑似误报会在 eval 记录中保留核查过程。
- **召回**：真实 CVE（revert 法）+ ~18 漏洞类型 + 真实用户 issue + 合成强触发；**真实 PR revert 金标准 4/4**（axios 原型污染 SSRF / requests Content-Type 解析 / gin ClientIP XFF / ripgrep gitignore 缓存）全部命中，发现精确还原原修复所针对的回归。
- **语言**：**45 种内置默认开**（常见+不常见：含仓颉/Zig/Nim/Crystal/OCaml/F#/Solidity/COBOL/Fortran/Dockerfile/Terraform…），按改动语言注入、可关、可覆盖。
- **大 PR / 未审完不静默放行**：diff 超上下文窗口、请求失败、上下文超限、超时、超大文件跳过等情况会降级 WARN，并可让 CI 非 0 退出，避免被当成干净 PASS。机制与真实大 PR 实测（最大 55 文件/5000 行）见 [`docs/BIG_PR_HANDLING.md`](docs/BIG_PR_HANDLING.md)。
- **意图评审（`--intent`）**：真实代码 + 真实意图实测。结构化强制把意图拆成 N 条验收标准逐条核对；受控 A/B 能区分完整 vs 不完整实现（axios、cobra Go 切片别名 bug：故意制造缺口→精确命中、完整版→0 误报），10 个真实正确修复 commit 跨 5 语言 **10/10 met 且 0 误报**；未逐条核对的标准兜底标「未核对」并降级 WARN，绝不空清单或伪 PASS。见 [`docs/evals/`](docs/evals/) 第六节（[结构化强制](docs/evals/2026-06-27__intent-structured-enforcement.md) · [受控 A/B](docs/evals/2026-06-27__intent-mvp-ab.md) · [10 commit 批量](docs/evals/2026-06-27__intent-batch10.md) · [Go A/B](docs/evals/2026-06-27__intent-cobra-pr2356.md)）。
- **诚实局限**：细微多步算术/进位 off-by-one 是静态审查硬尾，见 [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md)，建议测试互补。

## 当前状态

Beta：核心链路完整（多维并行 + 证伪 Judge + 置信度闸口 + 业务规则 + 意图/技术评审 + 45 语言内置规则 + 重复检测 + 多采样 + `--fix` 锚点校验 + reachability 分级 + 大 diff 自适应单元/未审完不静默放行 + CLI/Skill/Action），
含 CI（fmt/clippy -D warnings/test，Win+Ubuntu）、只读安全边界、缓存与超时兜底。
变更记录见 [`CHANGELOG.md`](CHANGELOG.md)。

## License

[MIT](LICENSE)

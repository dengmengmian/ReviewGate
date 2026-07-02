<p align="center">
  <img src="docs/assets/logo.svg" alt="ReviewGate" width="420">
</p>

<p align="center">
  AI 代码的合并前质量闸口：<b>优先拦截高风险问题，减少低价值 review 噪音</b>
</p>

<p align="center">
  <a href="README.en.md">English</a> · 简体中文
</p>

<p align="center">
  <a href="https://github.com/dengmengmian/ReviewGate/actions/workflows/ci.yml"><img src="https://github.com/dengmengmian/ReviewGate/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/dengmengmian/ReviewGate/releases/latest"><img src="https://img.shields.io/github/v/release/dengmengmian/ReviewGate" alt="Release"></a>
  <img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="License: MIT">
</p>

ReviewGate 是给 AI 生成或 AI 大量参与代码准备的合并前质量闸口。核心链路已可用于真实 PR 和 CI；它不替代测试和人工 review，而是在合并前先做一轮过滤：高风险问题推到前面，低置信反馈默认折叠。

| 核心价值 | 对团队的意义 |
|---|---|
| 拦高危 | 按安全、逻辑、性能、业务规则等维度并行审查，把 must-fix 放到前面 |
| 降噪音 | 去重、证伪、按置信度过滤，默认隐藏低价值反馈 |
| 不假通过 | 超时、上下文过大、未审完都会降级 WARN，不把不完整审查伪装成 PASS |

## 快速开始

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
EOF

# 3) 用环境变量放 key，不写进配置文件
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

## 输出长这样

```text
━━ ReviewGate ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  ✖ BLOCK    1 files · 1 must-fix · 0 warn · 3 hidden
  LLM 120k in (cache 88%) · 2.1k out
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

▌ MUST FIX

  1  handler.rs:3                       security · high · 100%

     SQL 注入：用户输入 req.user_id 通过 format! 直接拼接到 DELETE 语句…

     Patch
       - let q = format!("DELETE FROM users WHERE id = {}", req.user_id);
       + let q = "DELETE FROM users WHERE id = $1";

▌ NOT SHOWN

  3 low-confidence findings hidden. Run with --show-filtered to inspect them.
```

## 什么时候适合 / 不适合

| 适合 | 不适合 |
|---|---|
| AI 一次改很多文件，reviewer 想先知道哪里最危险 | 替代单元测试、集成测试或人工 review |
| 团队有权限、金额、状态机等业务规则需要反复检查 | 让模型自动改代码并无人确认地合并 |
| 想给 PR/CI 加一道高置信风险闸口 | 对误报零容忍、无法接受保守 WARN |
| 想用 `--intent` 检查实现是否符合需求/设计 | 完全没有 LLM API key 或不允许代码片段发给模型 |

## 为什么可以信

| 证据 | 说明 |
|---|---|
| 公开评测留痕 | 真实 PR、revert 金标准、45 语言样例、大 PR、意图评审结果都记录在 [`docs/evals/`](docs/evals/) |
| 默认只读 | 除显式 `--fix` 且逐条确认外，审查链路不写工作区，不执行任意 shell |
| 保守闸口 | 低置信默认折叠；未审完、超时、上下文超限会降级 WARN |

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

ReviewGate 不绑定模型。你可以接 OpenAI 兼容或 Anthropic 端点，按团队成本、速度和上下文窗口选择 provider。

建议把通用配置放到 `~/.reviewgate/config.toml`，这样所有仓库都能直接用；只有项目需要覆盖规则时，再在项目根目录放 `reviewgate.toml`。

**最小配置**只要一个 provider：

```toml
provider = "deepseek"

[providers.deepseek]
protocol = "openai"          # OpenAI 兼容（DeepSeek/Kimi/GLM/通义…）；用 Anthropic 则填 "anthropic"
base_url = "https://api.deepseek.com/v1"
model    = "deepseek-v4-pro"
# api_key = ""               # 可省略；推荐用 REVIEWGATE_API_KEY 注入
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

ReviewGate 一个引擎、多种形态，都只是调同一个 `reviewgate` CLI。**CLI 为主、GitHub Action 用于 PR/CI**，二者经真实使用打磨；**Claude Code Skill、Codex 与 AtomCode 是更薄的 agent 指令壳（experimental）**——已按当前 JSON schema 校准，但成熟度与覆盖面不及前两者。

### 1. CLI（主形态）

```bash
reviewgate review                       # 审查当前工作区改动
reviewgate review --from main --to HEAD # 审查当前分支相对 main 的改动
reviewgate review --intent spec.md      # 检查实现是否符合需求/设计
reviewgate review --format json         # 输出机器可读 JSON
reviewgate review --fail-on block       # BLOCK 时退出码 1，适合 CI
```

<details>
<summary><b>更多 CLI 参数</b></summary>

```bash
reviewgate review --dimensions security,logic
reviewgate review --no-judge             # 更快，误报略多
reviewgate review --show-filtered        # 展开被过滤的低置信项
reviewgate review --timeout 120          # 单维度墙钟上限（秒）
reviewgate review --samples 3            # 每维度多采样取并集
reviewgate review --fix                  # 逐条 y/N 确认后应用建议代码（作用于本次 review 覆盖的改动）
reviewgate review --fix-all              # 不逐条确认，直接全部应用（可非交互，供 CI/脚本）
reviewgate review --fix-all --fix-branch # 可叠加 --fix-branch（对 --fix / --fix-all 都适用）：先新建分支再改，保持原分支干净，可选跟分支名
reviewgate review --commit HEAD --fix    # 审查已提交的改动并应用修复（见下方注意）
reviewgate review --judge-concurrency 4  # 限制 Judge 并发
reviewgate review --fanout-concurrency 6 # 限制 fan-out 并发
reviewgate review --verbose              # 打印 token/缓存/轮数
reviewgate review --commit <sha>         # 审查单个 commit
reviewgate review --commit <sha> --intent-from-commit
```

> **注意：`--fix` / `--fix-all` 只作用于本次 review 覆盖的那份 diff。** 不带范围时，review 默认审**工作区未提交的改动**（`git diff HEAD`）——如果改动已经 commit、工作区是干净的，`--fix` 会「未检测到变更、无可应用修复」。要修**已提交**的改动，请带上范围，例如 `reviewgate review --commit HEAD --fix` 或 `reviewgate review --from main --to HEAD --fix`。

</details>

### 意图 / 技术评审（`--intent`）

缺陷评审不需要知道「本该做什么」；**技术评审需要**。传入本次改动的意图（需求/设计/验收标准，文件或 `-` 读 stdin），ReviewGate 会**额外**起一个**独立的整体性 Agent**——从 diff 出发主动跨文件追调用方、契约、测试，判断实现是否完整、正确地满足意图，输出一份**验收清单**（每条标准 ✓满足 / ✗缺失 / ✗破坏 / ⚠不符 / •建议）。意图会被**拆成 N 条验收标准（C1..CN）逐条核对**；没被逐条裁决的标准兜底标 `? 未核对`（不留空清单），只要存在未核对标准就**降级 WARN**，绝不伪装 PASS。它与常驻的 `business.rules` 正交：规则是不变量，`--intent` 是每次不同的「这次该做什么」。不传 `--intent` 时零开销。

```bash
reviewgate review --from main --to HEAD --intent docs/requirement.md
```

`--exec-verify` 会让模型生成的自包含 JS/Python 片段在本机运行以验证边界用例。它默认关闭，且当前只是临时目录 + 清空环境 + 超时的**弱隔离**，不是 OS 级沙箱；只建议在可信或隔离的 CI 环境使用。

**输出语言**：影响 **finding 文案**（问题描述/修复建议）与**整份报告骨架**（章节标题如 `MUST FIX`/`NEXT STEPS`、状态词 `PASS`/`WARN`/`BLOCK`、计数行、验收清单、实时进度行）——中文 locale 下均显示中文，其它语言回退英文。命令名（`reviewgate review …`）、维度/严重度标识、token 计量行保持英文。语言按以下优先级决定：

1. **`REVIEWGATE_OUTPUT_LANGUAGE`** —— 显式指定，原样使用（如 `"Chinese (Simplified)"`、`"日本語"`）。
2. **终端 locale**，按 `LC_ALL` > `LC_MESSAGES` > `LANG` 取第一个非空值映射（`zh_CN`→简中、`zh_TW`/`zh_HK`/`zh_MO`→繁中、`ja`→日语、`ko`、`fr`、`de`、`es`、`pt_BR`、`ru`、`it`…）。
3. **兜底英文** —— 上述都没有，或 locale 为 `C` / `POSIX` 时。

仅读环境变量（不读 git 配置或仓库内容），所以未设 locale 的 CI 默认英文。强制某语言：`REVIEWGATE_OUTPUT_LANGUAGE="Chinese (Simplified)" reviewgate review`。

**退出码（CI 闸口用）**：`0` 放行 · `1` 被闸口拦截（按 `--fail-on block|warn|never` 判定）· `2` 工具自身出错（配置/网络/密钥等，不是代码问题，CI 应重试或告警而非当成 must-fix）。非法的 `--fail-on` / `--format` 取值在解析期就报错（退出码 2），不会被静默当成默认值。

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

**个人用**：把 `integrations/claude-skill/SKILL.md` 拷到 `~/.claude/skills/reviewgate/`（然后重载 Claude Code）。**显式触发用 `/reviewgate` 最可靠**——只说"审查我的改动"可能被 Claude Code 内置的通用 code-review 抢走。

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

```yaml
name: ReviewGate
on:
  pull_request:

permissions:
  contents: read
  pull-requests: write

jobs:
  review:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0

      - uses: dengmengmian/ReviewGate/integrations/github-action@v0
        env:
          REVIEWGATE_API_KEY: ${{ secrets.REVIEWGATE_API_KEY }}
        with:
          dimensions: all
          fail-on: block
          comment: "true"
```

> **版本策略**：推荐用 `@v0` 跟随 0.x 兼容更新。Action 默认下载最新 CLI，CLI 发版通常不用改 workflow；需要可复现 CI 时，用 `with: { version: "v0.2.0" }` 钉死 CLI 引擎版本。

> **意图评审（可选）**：加 `with: { intent: "auto" }` 后，Action 会自动把 **PR 标题+描述**作为 `--intent` 做「实现 vs 意图」评审并输出验收清单——正好覆盖「每个 hunk 都自洽、但整体没做到 PR 声称的事」这类缺陷向审查抓不到的问题。PR 描述写得越像验收标准效果越好；标题含糊会产生「未核对」项并降级 WARN，故默认关闭。也可传路径指向固定意图文档。

### 4. Codex（AGENTS.md，experimental）

OpenAI Codex CLI 读项目根的 `AGENTS.md`。一键把 ReviewGate 用法幂等并入（不覆盖已有内容）：

```bash
curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/integrations/codex/install-into-project.sh | sh
```

它把一段 ReviewGate 指令并入 `./AGENTS.md`，并生成 `reviewgate.toml` + `.reviewgate/rules/` 模板。之后在 Codex 里说"用 ReviewGate 审查我的改动"即可。与 Claude Skill 同源、同 JSON schema。

### 5. AtomCode（experimental）

[AtomCode](https://github.com/dengmengmian/AtomCode) 用与 Claude Code 相同的 `SKILL.md` 格式，会自动发现 `.atomcode/skills/`、`.claude/skills/`（项目与全局）。一键装入项目级 skill（与 claude-skill 同一份 SKILL.md）：

```bash
curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/integrations/atomcode/install-into-project.sh | sh
```

它生成 `.atomcode/skills/reviewgate/SKILL.md` + `reviewgate.toml` + `.reviewgate/rules/` 模板。若你已装过 claude-skill，AtomCode 会自动发现 `.claude/skills/`，无需重复安装。

## 设计细节

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

| 指标 | 当前记录 |
|---|---|
| 误 BLOCK | 已记录真实 PR、45 语言干净样例、真实合并 commit 样本中未观察到误 BLOCK |
| Revert 金标准 | 真实 PR revert gold set **4/4** 命中：axios、requests、gin、ripgrep |
| 语言覆盖 | **45 种内置语言规则** 默认开启，可关闭、覆盖或追加 |
| 大 PR | diff 超上下文、请求失败、超时、超大文件跳过会降级 WARN，不静默 PASS |
| 意图评审 | 10 个真实正确修复 commit 跨 5 语言 **10/10 met 且 0 误报** |

详细评测见 [`docs/evals/`](docs/evals/)；大 PR 机制见 [`docs/BIG_PR_HANDLING.md`](docs/BIG_PR_HANDLING.md)；已知局限见 [`docs/LIMITATIONS.md`](docs/LIMITATIONS.md)。

## 当前状态

ReviewGate 核心链路已可用于真实 PR 和 CI。团队接入时建议先以 `WARN` / 评论模式观察一段时间，再把 `BLOCK` 接入强制合并闸口。

| 状态 | 说明 |
|---|---|
| 已可用 | CLI、Claude Code Skill、GitHub Action、业务规则、意图评审、大 PR 降级处理 |
| 默认边界 | 审查链路只读；`--fix` 需要逐条确认；未审完不会静默 PASS |
| 仍需配合 | 不能替代测试和人工 review；细微多步计算、强运行时语义仍建议靠测试覆盖 |
| 质量保障 | CI 覆盖 fmt、clippy `-D warnings`、测试，运行于 Ubuntu 和 Windows |

变更记录见 [`CHANGELOG.md`](CHANGELOG.md)。

## License

[MIT](LICENSE)

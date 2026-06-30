#!/bin/sh
# 把 ReviewGate 用法装进**当前项目**给 OpenAI Codex CLI 用（团队共享、可提交）。
# Codex 读项目根的 AGENTS.md；本脚本把 ReviewGate 段落幂等地并入其中。
# 在你项目根目录运行：
#   curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/integrations/codex/install-into-project.sh | sh
set -e

RAW="https://raw.githubusercontent.com/dengmengmian/ReviewGate/main"

if [ ! -d .git ]; then
  echo "⚠ 当前目录不是 git 仓库根。请在你的项目根目录运行。" >&2
fi

# 1) 把 ReviewGate 段落并入 ./AGENTS.md（幂等：已存在 BEGIN 标记则跳过，不重复追加）
if [ -f AGENTS.md ] && grep -q "<!-- BEGIN ReviewGate -->" AGENTS.md; then
  echo "· AGENTS.md 已含 ReviewGate 段落，跳过（如需更新请先删掉 BEGIN..END 之间再重跑）"
else
  block="$(curl -fsSL "$RAW/integrations/codex/AGENTS.md")"
  { [ -f AGENTS.md ] && printf '\n'; printf '%s\n' "$block"; } >> AGENTS.md
  echo "✓ 已把 ReviewGate 段落写入 ./AGENTS.md（Codex 会自动读取）"
fi

# 2) 业务规则模板（始终注入审查 prompt；改成你们自己的规则）
mkdir -p .reviewgate/rules
if [ -f .reviewgate/rules/business.md ]; then
  echo "· 已存在 .reviewgate/rules/business.md，跳过"
else
  cat > .reviewgate/rules/business.md <<'EOF'
# 业务规则（本组织专属——改成你们的真实约定）

这些规则会注入到每次审查，所有维度可见。用 `[B1]/[B2]…` 编号便于追溯。

- [B1] 金额一律用整数（分），禁止 float / 浮点运算。
- [B2] 用户级资源（订单/账户/文件）访问必须校验归属（owner_id == 当前用户）。
- [B3] 对外接口的分页 size 必须有上限，禁止无界查询。
- [B4] 写操作必须幂等或带去重键。
EOF
  echo "✓ .reviewgate/rules/business.md（业务规则模板，请改成你们的）"
fi

# 3) 项目级配置模板（密钥用环境变量注入，勿提交明文）
if [ -f reviewgate.toml ]; then
  echo "· 已存在 reviewgate.toml，跳过"
else
  cat > reviewgate.toml <<'EOF'
# ReviewGate 项目配置。密钥请用环境变量 REVIEWGATE_API_KEY 注入，勿在此写明文。
provider = "default"

[providers.default]
protocol = "openai"          # OpenAI 兼容端点
base_url = "https://your-endpoint/v1"
api_key  = "set-via-REVIEWGATE_API_KEY"
model    = "your-model"

[gate]
block_threshold = 0.8
warn_threshold  = 0.5

# 启用组织业务/语言规则（上面 .reviewgate/rules/）
[business]
rules_dir = ".reviewgate/rules"
EOF
  echo "✓ reviewgate.toml（配置模板——填好端点；密钥用 REVIEWGATE_API_KEY 环境变量）"
fi

cat <<'EOF'

下一步：
  1) 装 CLI：curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/install.sh | sh
            （Windows：irm .../install.ps1 | iex）
  2) 编辑 reviewgate.toml 填端点；export REVIEWGATE_API_KEY=你的key；reviewgate llm test 验证
  3) 改 .reviewgate/rules/business.md 成你们组织的真实规则
  4) 提交 AGENTS.md 和 .reviewgate/ 到仓库 → 全队的 Codex 共享
  5) 在 Codex 里说"用 ReviewGate 审查我的改动"即可触发；或直接 reviewgate review
EOF

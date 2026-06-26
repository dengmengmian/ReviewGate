#!/bin/sh
# 把 ReviewGate skill + 规则模板装进**当前项目**（团队共享、可提交）。
# 在你项目根目录运行：
#   curl -fsSL https://raw.githubusercontent.com/dengmengmian/ReviewGate/main/integrations/claude-skill/install-into-project.sh | sh
# 幂等：已存在的文件不覆盖。
set -e

RAW="https://raw.githubusercontent.com/dengmengmian/ReviewGate/main"

if [ ! -d .git ]; then
  echo "⚠ 当前目录不是 git 仓库根。请在你的项目根目录运行。" >&2
fi

mkdir -p .claude/skills/reviewgate .reviewgate/rules

# 1) 团队共享 skill（提交后，全队在 Claude Code 里都能用）
if [ -f .claude/skills/reviewgate/SKILL.md ]; then
  echo "· 已存在 .claude/skills/reviewgate/SKILL.md，跳过"
else
  curl -fsSL "$RAW/integrations/claude-skill/SKILL.md" -o .claude/skills/reviewgate/SKILL.md
  echo "✓ .claude/skills/reviewgate/SKILL.md（团队共享 skill）"
fi

# 2) 组织业务规则模板（始终注入到审查 prompt；改成你们自己的规则）
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

# 3) 语言起步规则：全部 45 种语言已**内置默认注入**（按改动语言自动启用），无需拷贝。
#    如需覆盖/扩充某语言规则，自行创建 .reviewgate/rules/<语言>.md 即会叠加（优先于内置）。
echo "· 45 种语言起步规则已内置，无需拷贝；如需定制创建 .reviewgate/rules/<语言>.md 即可叠加"

# 4) 规则目录说明
if [ ! -f .reviewgate/rules/README.md ]; then
  cat > .reviewgate/rules/README.md <<'EOF'
# .reviewgate/rules

- `business.md`：业务规则，**每次审查都注入**。
- `<语言>.md`：语言专属规则，**仅当该语言文件被改动时注入**。
  支持：rust / typescript / javascript / python / go / java / ruby / c / cpp / kotlin / swift / php / csharp。
  安装脚本已放入一套起步语言陷阱规则；你可以直接修改、删除，或替换成组织自己的约定。
EOF
fi

# 5) 项目级配置模板（密钥用环境变量注入，勿提交明文）
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
  2) 编辑 reviewgate.toml 填端点；export REVIEWGATE_API_KEY=你的key
  3) 改 .reviewgate/rules/business.md 成你们组织的真实规则
  4) 按需调整 .reviewgate/rules/<语言>.md 起步规则
  5) 提交 .claude/ 和 .reviewgate/ 到仓库 → 全队共享
  6) 在 Claude Code 里说"审查我的改动"即可触发；或直接 reviewgate review
EOF

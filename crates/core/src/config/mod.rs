//! 配置加载。
//!
//! 发现顺序：`REVIEWGATE_CONFIG` 环境变量 → 当前目录 `./reviewgate.toml`
//! → `~/.reviewgate/config.toml`。后续里程碑再加环境变量字段级覆盖。

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// LLM 线上协议。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    /// OpenAI 兼容 `/chat/completions`（DeepSeek/Kimi/GLM/通义…）。
    #[default]
    Openai,
    /// Anthropic `/v1/messages`。
    Anthropic,
}

/// 单个提供方配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub protocol: Protocol,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    /// 模型输入上下文窗口（token）预算。用于把大 diff 按预算切成多个审查单元，
    /// 并在发送前预检避免撞 provider 的 context-length 上限。缺省按 [`DEFAULT_MAX_INPUT_TOKENS`]。
    /// 小窗口 provider（如部分 64k/128k 端点）应显式调小。
    #[serde(default)]
    pub max_input_tokens: Option<u32>,
}

/// 默认输入预算：对主流 200k/1M 上下文模型不会误切；小窗 provider 需在配置里显式调小。
/// 即便此值高于真实窗口，发送前预检 + "未审完不放行"不变量也会兜底（不会静默 PASS）。
pub const DEFAULT_MAX_INPUT_TOKENS: u32 = 200_000;

impl ProviderConfig {
    /// 解析后的输入 token 预算（缺省取 [`DEFAULT_MAX_INPUT_TOKENS`]）。
    pub fn max_input_tokens(&self) -> u32 {
        self.max_input_tokens.unwrap_or(DEFAULT_MAX_INPUT_TOKENS)
    }
}

/// 闸口阈值配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateConfig {
    /// 置信度 ≥ 此值 → 阻断。
    #[serde(default = "default_block")]
    pub block_threshold: f32,
    /// 置信度 ≥ 此值 → 警告；低于则默认折叠过滤。
    #[serde(default = "default_warn")]
    pub warn_threshold: f32,
    /// 审查未完成（某单元请求失败/超上下文/被跳过）时是否阻止"通过"。默认 true：
    /// 未审完则**永不 PASS**（至少 WARN，且 CI 非 0 退出），杜绝"漏审却放行"。
    #[serde(default = "default_true")]
    pub fail_on_incomplete: bool,
}

fn default_block() -> f32 {
    0.8
}
fn default_warn() -> f32 {
    0.5
}

impl Default for GateConfig {
    fn default() -> Self {
        GateConfig {
            block_threshold: default_block(),
            warn_threshold: default_warn(),
            fail_on_incomplete: true,
        }
    }
}

fn default_true() -> bool {
    true
}

/// 业务/项目规则配置。注入到共享 prompt 块，供各维度参考。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusinessConfig {
    /// 内联业务规则列表（最直接的形式）。
    #[serde(default)]
    pub rules: Vec<String>,
    /// 规则目录（相对仓库根）。其中 `<语言>.md`（如 `rust.md`）仅在该语言被改动时注入；
    /// 其它（如 `business.md` / `security.md`）始终注入。
    #[serde(default)]
    pub rules_dir: Option<String>,
    /// skill 目录（相对仓库根）：读取组织把 review 规则写成的 **skill 文件**（SKILL.md 格式，
    /// 自动剥离 YAML frontmatter），把正文注入审查。支持 `<子目录>/SKILL.md` 与扁平 `*.md` 两种布局。
    /// 与 `rules_dir` 互补——后者是纯规则 md，这里专吃组织已有的 skill。
    #[serde(default)]
    pub skills_dir: Option<String>,
    /// 是否默认注入**内置语言起步规则**（按本次改动语言自动注入已验证的公认陷阱清单）。
    /// 默认 true；置 false 可完全关闭。用户 `rules_dir/<lang>.md` 会**补充**而非替换。
    #[serde(default = "default_true")]
    pub builtin_language_rules: bool,
}

impl Default for BusinessConfig {
    fn default() -> Self {
        Self {
            rules: Vec::new(),
            rules_dir: None,
            skills_dir: None,
            builtin_language_rules: true,
        }
    }
}

/// 顶层配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// 默认提供方名（对应 `providers` 的 key）。
    pub provider: String,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    /// 闸口阈值（缺省用默认值）。
    #[serde(default)]
    pub gate: GateConfig,
    /// 业务/项目规则（缺省为空）。
    #[serde(default)]
    pub business: BusinessConfig,
}

impl Config {
    /// 按发现顺序加载配置。
    pub fn load() -> Result<Config> {
        let path = Self::discover()
            .ok_or_else(|| anyhow!("未找到 reviewgate.toml（可设 REVIEWGATE_CONFIG 指定路径）"))?;
        Self::from_path(&path)
    }

    /// 从指定路径加载。
    pub fn from_path(path: &Path) -> Result<Config> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("读取配置失败：{}", path.display()))?;
        let cfg: Config =
            toml::from_str(&text).with_context(|| format!("解析配置失败：{}", path.display()))?;
        Ok(cfg)
    }

    /// 取默认提供方的配置。
    pub fn active_provider(&self) -> Result<&ProviderConfig> {
        self.providers
            .get(&self.provider)
            .ok_or_else(|| anyhow!("配置里没有名为 `{}` 的提供方", self.provider))
    }

    /// 取默认提供方并应用环境变量覆盖（用于 CI 注入密钥等）。
    ///
    /// `REVIEWGATE_API_KEY` / `REVIEWGATE_BASE_URL` / `REVIEWGATE_MODEL` 覆盖对应字段。
    pub fn active_provider_resolved(&self) -> Result<ProviderConfig> {
        let mut p = self.active_provider()?.clone();
        if let Ok(k) = std::env::var("REVIEWGATE_API_KEY") {
            if !k.is_empty() {
                p.api_key = k;
            }
        }
        if let Ok(u) = std::env::var("REVIEWGATE_BASE_URL") {
            if !u.is_empty() {
                p.base_url = u;
            }
        }
        if let Ok(m) = std::env::var("REVIEWGATE_MODEL") {
            if !m.is_empty() {
                p.model = m;
            }
        }
        Ok(p)
    }

    fn discover() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("REVIEWGATE_CONFIG") {
            let p = PathBuf::from(p);
            if p.is_file() {
                return Some(p);
            }
        }
        let cwd = PathBuf::from("reviewgate.toml");
        if cwd.is_file() {
            return Some(cwd);
        }
        if let Some(home) = home_dir() {
            let p = home.join(".reviewgate").join("config.toml");
            if p.is_file() {
                return Some(p);
            }
        }
        None
    }
}

/// 跨平台用户主目录：Unix 用 `HOME`，Windows 默认不设 HOME，回退 `USERPROFILE`。
/// 零依赖，覆盖 `~/.reviewgate/config.toml` 在 Windows 上找不到的问题。
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|s| !s.is_empty()))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOML: &str = r#"
provider = "qwen"
[providers.qwen]
base_url = "https://x/v1"
api_key = "k"
model = "m"
[providers.anthropic]
protocol = "anthropic"
base_url = "https://api.anthropic.com"
api_key = "a"
model = "claude"
[business]
skills_dir = ".claude/skills"
"#;

    #[test]
    fn parses_and_defaults() {
        let cfg: Config = toml::from_str(TOML).unwrap();
        assert_eq!(cfg.provider, "qwen");
        // 未写 protocol → 默认 openai。
        assert_eq!(cfg.providers["qwen"].protocol, Protocol::Openai);
        // 显式 anthropic。
        assert_eq!(cfg.providers["anthropic"].protocol, Protocol::Anthropic);
        // gate 缺省。
        assert_eq!(cfg.gate.block_threshold, 0.8);
        assert_eq!(cfg.gate.warn_threshold, 0.5);
        // business.skills_dir 解析到。
        assert_eq!(cfg.business.skills_dir.as_deref(), Some(".claude/skills"));
    }

    #[test]
    fn active_provider_lookup() {
        let cfg: Config = toml::from_str(TOML).unwrap();
        assert_eq!(cfg.active_provider().unwrap().model, "m");
        // 不存在的 provider → 报错。
        let bad: Config = toml::from_str("provider = \"nope\"").unwrap();
        assert!(bad.active_provider().is_err());
    }
}

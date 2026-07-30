//! `reviewgate init` —— 写出全局配置，降低冷启动摩擦。

use anyhow::{bail, Context, Result};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// 内置 provider 预设。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderPreset {
    pub name: &'static str,
    pub protocol: &'static str,
    pub base_url: &'static str,
    pub model: &'static str,
    pub note: &'static str,
}

pub const PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        name: "deepseek",
        protocol: "openai",
        base_url: "https://api.deepseek.com/v1",
        model: "deepseek-v4-pro",
        note: "OpenAI-compatible · good default cost/quality",
    },
    ProviderPreset {
        name: "openai",
        protocol: "openai",
        base_url: "https://api.openai.com/v1",
        model: "gpt-4.1",
        note: "OpenAI official endpoint",
    },
    ProviderPreset {
        name: "anthropic",
        protocol: "anthropic",
        base_url: "https://api.anthropic.com",
        model: "claude-sonnet-4-5",
        note: "Anthropic Messages API",
    },
];

pub fn find_preset(name: &str) -> Option<&'static ProviderPreset> {
    PRESETS
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name.trim()))
}

/// 渲染 `~/.reviewgate/config.toml` 正文（不含 api_key，推荐环境变量注入）。
pub fn render_config(provider: &str, protocol: &str, base_url: &str, model: &str) -> String {
    format!(
        r#"# ReviewGate global config (written by `reviewgate init`)
# API key: set REVIEWGATE_API_KEY in the environment — do not commit secrets.
# Docs: https://github.com/dengmengmian/ReviewGate

provider = "{provider}"

[providers.{provider}]
protocol = "{protocol}"
base_url = "{base_url}"
model = "{model}"
# api_key = ""   # optional; prefer REVIEWGATE_API_KEY
"#
    )
}

/// 默认配置目录：`~/.reviewgate`。
pub fn default_config_dir() -> Result<PathBuf> {
    let home = reviewgate_core::config::home_dir().ok_or_else(|| {
        anyhow::anyhow!("cannot resolve home directory (set HOME or USERPROFILE)")
    })?;
    Ok(home.join(".reviewgate"))
}

/// 将配置写入 `config_dir/config.toml`。
pub fn write_config(config_dir: &Path, content: &str, force: bool) -> Result<PathBuf> {
    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("failed to create {}", config_dir.display()))?;
    let path = config_dir.join("config.toml");
    if path.is_file() && !force {
        bail!(
            "config already exists: {}\n  re-run with --force to overwrite, or edit the file in place",
            path.display()
        );
    }
    std::fs::write(&path, content)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

#[derive(Debug, Clone)]
pub struct InitChoice {
    pub provider: String,
    pub protocol: String,
    pub base_url: String,
    pub model: String,
}

/// 非交互：用 flag / 预设解析最终配置。
pub fn resolve_noninteractive(
    provider: &str,
    protocol: Option<&str>,
    base_url: Option<&str>,
    model: Option<&str>,
) -> Result<InitChoice> {
    let provider = provider.trim();
    if provider.is_empty() {
        bail!("--provider must not be empty");
    }
    if provider.eq_ignore_ascii_case("custom") {
        let base_url = base_url
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("custom provider requires --base-url"))?;
        let model = model
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("custom provider requires --model"))?;
        let protocol = protocol
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("openai");
        if protocol != "openai" && protocol != "anthropic" {
            bail!("--protocol must be openai or anthropic, got `{protocol}`");
        }
        return Ok(InitChoice {
            provider: "custom".into(),
            protocol: protocol.into(),
            base_url: base_url.into(),
            model: model.into(),
        });
    }
    let preset = find_preset(provider).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown provider `{provider}`; use one of: deepseek, openai, anthropic, custom"
        )
    })?;
    let protocol = protocol.unwrap_or(preset.protocol);
    if protocol != "openai" && protocol != "anthropic" {
        bail!("--protocol must be openai or anthropic, got `{protocol}`");
    }
    Ok(InitChoice {
        provider: preset.name.into(),
        protocol: protocol.into(),
        base_url: base_url.unwrap_or(preset.base_url).into(),
        model: model.unwrap_or(preset.model).into(),
    })
}

fn read_line(prompt: &str, default: &str) -> Result<String> {
    let mut out = io::stdout();
    if default.is_empty() {
        write!(out, "{prompt}: ")?;
    } else {
        write!(out, "{prompt} [{default}]: ")?;
    }
    out.flush()?;
    let mut buf = String::new();
    io::stdin()
        .read_line(&mut buf)
        .context("failed to read stdin")?;
    let t = buf.trim();
    if t.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(t.to_string())
    }
}

/// 交互：列出预设并收集选择。
pub fn resolve_interactive() -> Result<InitChoice> {
    eprintln!("ReviewGate init — write a global config (API key stays in the environment).\n");
    eprintln!("Provider presets:");
    for (i, p) in PRESETS.iter().enumerate() {
        eprintln!("  {}) {:<10}  {}  ({})", i + 1, p.name, p.model, p.note);
    }
    eprintln!("  4) custom      your own base_url + model");
    eprintln!();

    let pick = read_line("Choose provider (1-4 or name)", "1")?;
    let choice = match pick.as_str() {
        "1" | "deepseek" => resolve_noninteractive("deepseek", None, None, None)?,
        "2" | "openai" => resolve_noninteractive("openai", None, None, None)?,
        "3" | "anthropic" => resolve_noninteractive("anthropic", None, None, None)?,
        "4" | "custom" => {
            let protocol = read_line("protocol (openai|anthropic)", "openai")?;
            let base_url = read_line("base_url", "")?;
            let model = read_line("model", "")?;
            resolve_noninteractive("custom", Some(&protocol), Some(&base_url), Some(&model))?
        }
        other => {
            if find_preset(other).is_some() {
                resolve_noninteractive(other, None, None, None)?
            } else {
                bail!("invalid choice `{other}`");
            }
        }
    };

    // Allow overriding model/base_url after preset pick.
    let base_url = read_line("base_url", &choice.base_url)?;
    let model = read_line("model", &choice.model)?;
    Ok(InitChoice {
        provider: choice.provider,
        protocol: choice.protocol,
        base_url,
        model,
    })
}

/// CLI 入口：写出配置并打印下一步。
pub fn run_init(
    provider: &str,
    protocol: Option<&str>,
    base_url: Option<&str>,
    model: Option<&str>,
    yes: bool,
    force: bool,
    config_dir: Option<&Path>,
    run_llm_test: bool,
) -> Result<i32> {
    let interactive = !yes && io::stdin().is_terminal() && io::stdout().is_terminal();
    let choice = if interactive {
        resolve_interactive()?
    } else {
        resolve_noninteractive(provider, protocol, base_url, model)?
    };

    let dir = match config_dir {
        Some(p) => p.to_path_buf(),
        None => default_config_dir()?,
    };
    let content = render_config(
        &choice.provider,
        &choice.protocol,
        &choice.base_url,
        &choice.model,
    );
    let path = write_config(&dir, &content, force)?;

    eprintln!("Wrote {}", path.display());
    eprintln!();
    eprintln!("Next:");
    eprintln!("  1) export REVIEWGATE_API_KEY=\"your-key\"");
    eprintln!("  2) reviewgate llm test");
    eprintln!("  3) reviewgate demo          # poisoned fixture should BLOCK");
    eprintln!("  4) cd your-repo && reviewgate review");

    if run_llm_test {
        eprintln!();
        eprintln!("Running `reviewgate llm test` ...");
        // Async `llm test` is wired by the CLI wrapper after this returns.
    }
    Ok(0)
}

use std::io::IsTerminal;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn render_config_contains_provider_and_no_hardcoded_secret() {
        let s = render_config(
            "deepseek",
            "openai",
            "https://api.deepseek.com/v1",
            "deepseek-v4-pro",
        );
        assert!(s.contains("provider = \"deepseek\""));
        assert!(s.contains("protocol = \"openai\""));
        assert!(s.contains("base_url = \"https://api.deepseek.com/v1\""));
        assert!(s.contains("model = \"deepseek-v4-pro\""));
        assert!(s.contains("REVIEWGATE_API_KEY"));
        assert!(!s.contains("sk-"));
        // No active (uncommented) api_key assignment — key stays in the environment.
        for line in s.lines() {
            let t = line.trim();
            if t.starts_with('#') {
                continue;
            }
            assert!(
                !t.starts_with("api_key"),
                "config must not set api_key in file: {t}"
            );
        }
    }

    #[test]
    fn resolve_preset_deepseek() {
        let c = resolve_noninteractive("deepseek", None, None, None).unwrap();
        assert_eq!(c.provider, "deepseek");
        assert_eq!(c.protocol, "openai");
        assert!(c.base_url.contains("deepseek"));
    }

    #[test]
    fn resolve_custom_requires_url_and_model() {
        assert!(resolve_noninteractive("custom", None, None, None).is_err());
        let c = resolve_noninteractive(
            "custom",
            Some("openai"),
            Some("http://localhost:8080/v1"),
            Some("my-model"),
        )
        .unwrap();
        assert_eq!(c.provider, "custom");
        assert_eq!(c.model, "my-model");
    }

    #[test]
    fn write_config_refuses_overwrite_without_force() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("rg-init-{nanos}"));
        let content = render_config("deepseek", "openai", "https://x", "m");
        write_config(&dir, &content, false).unwrap();
        let err = write_config(&dir, &content, false).unwrap_err();
        assert!(err.to_string().contains("already exists"), "err={err}");
        write_config(&dir, &content, true).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_provider_errors() {
        let err = resolve_noninteractive("not-a-vendor", None, None, None).unwrap_err();
        assert!(err.to_string().contains("unknown provider"));
    }
}

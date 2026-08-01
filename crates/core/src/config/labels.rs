//! 严重度标签自定义：让团队用自己的词和自己的定义来分级。
//!
//! 只改**显示名、颜色、定义**，不动 `Severity` 这三档本身——闸口阈值、退出码、
//! 跨维加分全都建立在这三档上，动它等于动闸口语义。团队真正想改的也不是档位数量，
//! 而是"什么算 high"：这里的 `definition` 会注入 review prompt，直接影响模型怎么分级。

use crate::model::Severity;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

/// 一档严重度的团队定制（配置里的 `[[severity_labels]]`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeverityLabel {
    /// 目标档位：`high` | `med` | `low`。
    pub id: String,
    /// 报告里显示的名字（缺省用 id 本身）。
    #[serde(default)]
    pub label: Option<String>,
    /// 报告配色：red | yellow | green | blue | magenta | cyan | gray。
    #[serde(default)]
    pub color: Option<String>,
    /// 该档位的团队定义。非空则注入 review prompt，让模型按此分级。
    #[serde(default)]
    pub definition: Option<String>,
}

/// 支持的颜色名 → ANSI SGR 参数。
const COLORS: &[(&str, &str)] = &[
    ("red", "1;31"),
    ("yellow", "33"),
    ("green", "32"),
    ("blue", "34"),
    ("magenta", "35"),
    ("cyan", "36"),
    ("gray", "2"),
];

fn color_code(name: &str) -> Option<&'static str> {
    COLORS
        .iter()
        .find(|(n, _)| *n == name.trim().to_ascii_lowercase())
        .map(|(_, c)| *c)
}

fn parse_severity(id: &str) -> Option<Severity> {
    match id.trim().to_ascii_lowercase().as_str() {
        "high" => Some(Severity::High),
        "med" | "medium" => Some(Severity::Med),
        "low" => Some(Severity::Low),
        _ => None,
    }
}

/// 解析后的标签表。三档各一条，缺省即内置行为。
#[derive(Debug, Clone)]
pub struct SeverityLabels {
    high: Entry,
    med: Entry,
    low: Entry,
}

#[derive(Debug, Clone)]
struct Entry {
    label: String,
    color: &'static str,
    definition: Option<String>,
}

impl Default for SeverityLabels {
    fn default() -> Self {
        Self {
            high: Entry {
                label: "high".into(),
                color: "1;31",
                definition: None,
            },
            med: Entry {
                label: "med".into(),
                color: "33",
                definition: None,
            },
            low: Entry {
                label: "low".into(),
                color: "2",
                definition: None,
            },
        }
    }
}

impl SeverityLabels {
    /// 应用配置。未知 id / 未知颜色**报错**——写错了却当没写，团队会以为定制生效了。
    pub fn resolve(labels: &[SeverityLabel]) -> Result<Self> {
        let mut out = Self::default();
        for l in labels {
            let Some(sev) = parse_severity(&l.id) else {
                bail!(
                    "unknown severity_labels id `{}` (use high | med | low)",
                    l.id
                );
            };
            let entry = out.entry_mut(sev);
            if let Some(label) = &l.label {
                if label.trim().is_empty() {
                    bail!("severity_labels `{}` has an empty label", l.id);
                }
                entry.label = label.trim().to_string();
            }
            if let Some(color) = &l.color {
                let Some(code) = color_code(color) else {
                    bail!(
                        "unknown severity_labels color `{color}` for `{}` (use {})",
                        l.id,
                        COLORS
                            .iter()
                            .map(|(n, _)| *n)
                            .collect::<Vec<_>>()
                            .join(" | ")
                    );
                };
                entry.color = code;
            }
            if let Some(def) = &l.definition {
                let def = def.trim();
                if !def.is_empty() {
                    entry.definition = Some(def.to_string());
                }
            }
        }
        Ok(out)
    }

    fn entry(&self, sev: Severity) -> &Entry {
        match sev {
            Severity::High => &self.high,
            Severity::Med => &self.med,
            Severity::Low => &self.low,
        }
    }

    fn entry_mut(&mut self, sev: Severity) -> &mut Entry {
        match sev {
            Severity::High => &mut self.high,
            Severity::Med => &mut self.med,
            Severity::Low => &mut self.low,
        }
    }

    /// 报告里显示的名字。
    pub fn label(&self, sev: Severity) -> &str {
        &self.entry(sev).label
    }

    /// 报告配色的 ANSI SGR 参数。
    pub fn color(&self, sev: Severity) -> &'static str {
        self.entry(sev).color
    }

    /// 注入 prompt 的分级定义块。没有任何团队定义时返回 `None`（零开销、prompt 不变）。
    pub fn prompt_block(&self) -> Option<String> {
        let mut lines = Vec::new();
        for sev in [Severity::High, Severity::Med, Severity::Low] {
            if let Some(def) = &self.entry(sev).definition {
                lines.push(format!("- `{}`: {def}", sev.as_str()));
            }
        }
        if lines.is_empty() {
            return None;
        }
        Some(format!(
            "## Severity definitions (project-specific)\n\
             Classify severity by these definitions rather than your defaults:\n{}",
            lines.join("\n")
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_builtin_behaviour() {
        let l = SeverityLabels::default();
        assert_eq!(l.label(Severity::High), "high");
        assert_eq!(l.color(Severity::High), "1;31");
        assert_eq!(l.color(Severity::Med), "33");
        assert_eq!(l.color(Severity::Low), "2");
        assert!(l.prompt_block().is_none(), "无定制时不应改动 prompt");
    }

    #[test]
    fn custom_label_color_and_definition_apply() {
        let cfg = vec![
            SeverityLabel {
                id: "high".into(),
                label: Some("Blocker".into()),
                color: Some("magenta".into()),
                definition: Some("必须修复才能合并".into()),
            },
            SeverityLabel {
                id: "medium".into(),
                label: None,
                color: None,
                definition: Some("下个迭代修".into()),
            },
        ];
        let l = SeverityLabels::resolve(&cfg).unwrap();
        assert_eq!(l.label(Severity::High), "Blocker");
        assert_eq!(l.color(Severity::High), "35");
        // 未指定的字段保持默认。
        assert_eq!(l.label(Severity::Med), "med");
        assert_eq!(l.color(Severity::Med), "33");

        let block = l.prompt_block().expect("有定义就应注入");
        assert!(block.contains("必须修复才能合并"));
        assert!(block.contains("下个迭代修"));
        assert!(!block.contains("`low`"), "没定义的档位不进 prompt: {block}");
    }

    #[test]
    fn unknown_id_or_color_errors() {
        let bad_id = vec![SeverityLabel {
            id: "critical".into(),
            label: None,
            color: None,
            definition: None,
        }];
        let err = SeverityLabels::resolve(&bad_id).unwrap_err().to_string();
        assert!(err.contains("high | med | low"), "{err}");

        let bad_color = vec![SeverityLabel {
            id: "high".into(),
            label: None,
            color: Some("chartreuse".into()),
            definition: None,
        }];
        assert!(SeverityLabels::resolve(&bad_color).is_err());
    }

    #[test]
    fn empty_label_is_rejected() {
        let empty = vec![SeverityLabel {
            id: "low".into(),
            label: Some("   ".into()),
            color: None,
            definition: None,
        }];
        assert!(SeverityLabels::resolve(&empty).is_err());
    }
}

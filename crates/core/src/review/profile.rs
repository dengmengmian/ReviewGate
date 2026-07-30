//! 运行 profile：`gate`（默认严闸口）vs `audit`（更宽审计）。

use crate::model::Dimension;
use serde::{Deserialize, Serialize};

/// CLI / 配置层审查姿态（与 `ReviewProfile::Deep` 安全深审正交）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunProfile {
    /// 合并闸口：默认四维、samples=1、精度优先。
    #[default]
    Gate,
    /// 审计摸底：更高采样、可选 style、略宽，接受更多 WARN。
    Audit,
}

impl RunProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            RunProfile::Gate => "gate",
            RunProfile::Audit => "audit",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "gate" | "default" => Some(RunProfile::Gate),
            "audit" | "wide" => Some(RunProfile::Audit),
            _ => None,
        }
    }

    /// 在用户未显式改 samples 时的建议采样。
    pub fn default_samples(self) -> usize {
        match self {
            RunProfile::Gate => 1,
            RunProfile::Audit => 2,
        }
    }

    /// 默认维度集。`user_all` 表示用户传了 `all` 或未指定。
    pub fn dimensions(self, user_all: bool, user_dims: Option<Vec<Dimension>>) -> Vec<Dimension> {
        if let Some(d) = user_dims {
            return d;
        }
        if !user_all {
            return Dimension::ALL.to_vec();
        }
        match self {
            RunProfile::Gate => Dimension::ALL.to_vec(),
            // audit：默认四维 + style（更宽，接受噪声）。
            RunProfile::Audit => {
                let mut d = Dimension::ALL.to_vec();
                if !d.contains(&Dimension::Style) {
                    d.push(Dimension::Style);
                }
                d
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_aliases() {
        assert_eq!(RunProfile::parse("gate"), Some(RunProfile::Gate));
        assert_eq!(RunProfile::parse("AUDIT"), Some(RunProfile::Audit));
        assert_eq!(RunProfile::parse("nope"), None);
    }

    #[test]
    fn audit_adds_style_on_all() {
        let d = RunProfile::Audit.dimensions(true, None);
        assert!(d.contains(&Dimension::Style));
        assert!(d.contains(&Dimension::Security));
    }

    #[test]
    fn gate_default_no_style() {
        let d = RunProfile::Gate.dimensions(true, None);
        assert!(!d.contains(&Dimension::Style));
        assert_eq!(d.len(), Dimension::ALL.len());
    }

    #[test]
    fn explicit_dims_win() {
        let d = RunProfile::Audit.dimensions(true, Some(vec![Dimension::Logic]));
        assert_eq!(d, vec![Dimension::Logic]);
    }
}

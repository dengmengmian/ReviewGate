//! 质量闸口：按置信度阈值把审查结果判定为 pass / warn / block，
//! 并把低于 warn 阈值的发现标记为已过滤（透明保留，供展开查看）。

use crate::config::GateConfig;
use crate::model::{Finding, Reachability, Severity};
use serde::Serialize;

/// 闸口判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum GateDecision {
    /// 无可信问题，放行。
    Pass,
    /// 有需关注的问题，但未达阻断线。
    Warn,
    /// 有高置信问题，阻断合并。
    Block,
}

impl GateDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            GateDecision::Pass => "PASS",
            GateDecision::Warn => "WARN",
            GateDecision::Block => "BLOCK",
        }
    }
}

/// 应用闸口：标记过滤项并返回判定（severity + confidence 双因子）。
///
/// - confidence < warn_threshold              → 标记 filtered（默认折叠）
/// - confidence ≥ block_threshold 且 severity 非 Low 且非 Latent → 计入阻断
/// - 其余达到 warn 阈值的                      → 计入警告
///
/// **Low 严重度永不阻断**：高置信的琐碎项（多为 style）最多 Warn，不该 Block 合并。
///
/// **Latent（潜伏雷）永不阻断**：代码本身成立但当前控制流打不到的真问题，作为提示
/// 保留并可见，但最多 Warn——避免把"理论存在、现在触发不了"的问题判成阻断级。
pub fn apply_gate(findings: &mut [Finding], gate: &GateConfig) -> GateDecision {
    let mut decision = GateDecision::Pass;
    for f in findings.iter_mut() {
        if f.confidence < gate.warn_threshold {
            f.filtered = true;
            continue;
        }
        f.filtered = false;
        let blocks = f.confidence >= gate.block_threshold
            && f.severity != Severity::Low
            && f.reachability != Reachability::Latent;
        if blocks {
            decision = GateDecision::Block;
        } else if decision == GateDecision::Pass {
            decision = GateDecision::Warn;
        }
    }
    decision
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Dimension, Reachability, Severity};

    fn finding(conf: f32) -> Finding {
        finding_sev(conf, Severity::Med)
    }

    fn finding_sev(conf: f32, sev: Severity) -> Finding {
        Finding {
            dimension: Dimension::Logic,
            confidence: conf,
            severity: sev,
            path: "a.rs".into(),
            start_line: 1,
            end_line: 1,
            message: "m".into(),
            existing_code: "x".into(),
            evidence: String::new(),
            suggestion: None,
            suggestion_code: String::new(),
            reachability: Reachability::default(),
            filtered: false,
            agreed_dimensions: 1,
        }
    }

    #[test]
    fn gate_decisions() {
        let g = GateConfig::default(); // block 0.8, warn 0.5
        let mut a = [finding(0.9)];
        assert_eq!(apply_gate(&mut a, &g), GateDecision::Block);

        let mut b = [finding(0.6)];
        assert_eq!(apply_gate(&mut b, &g), GateDecision::Warn);

        let mut c = [finding(0.3)];
        assert_eq!(apply_gate(&mut c, &g), GateDecision::Pass);
        assert!(c[0].filtered);
    }

    #[test]
    fn low_severity_never_blocks() {
        let g = GateConfig::default();
        // 高置信(0.95)但 Low 严重度 → 不该 Block，最多 Warn。
        let mut a = [finding_sev(0.95, Severity::Low)];
        assert_eq!(apply_gate(&mut a, &g), GateDecision::Warn);
        assert!(!a[0].filtered); // 仍展示，只是不阻断
                                 // High 严重度高置信 → 仍 Block。
        let mut b = [finding_sev(0.95, Severity::High)];
        assert_eq!(apply_gate(&mut b, &g), GateDecision::Block);
    }

    #[test]
    fn latent_never_blocks() {
        let g = GateConfig::default();
        // 高置信(0.95) + High 严重度，但可达性 = Latent（潜伏雷）→ 不该 Block，最多 Warn。
        let mut a = [finding_sev(0.95, Severity::High)];
        a[0].reachability = Reachability::Latent;
        assert_eq!(apply_gate(&mut a, &g), GateDecision::Warn);
        assert!(!a[0].filtered); // 仍展示，只是不阻断
                                 // 同样高置信 High 但 Reachable → 仍 Block。
        let mut b = [finding_sev(0.95, Severity::High)];
        b[0].reachability = Reachability::Reachable;
        assert_eq!(apply_gate(&mut b, &g), GateDecision::Block);
    }
}

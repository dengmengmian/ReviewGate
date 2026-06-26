//! 审查发现的后处理：跨维度一致性加分 + 复合排序。

use crate::model::Finding;

/// 复合排序：未过滤优先 → 严重度降（High→Low）→ 置信度降。
pub(super) fn sort_findings(findings: &mut [Finding]) {
    findings.sort_by(|a, b| {
        a.filtered
            .cmp(&b.filtered)
            .then(b.severity.cmp(&a.severity))
            .then(
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
}

/// 跨维度一致性加分：被 N(≥2) 个不同维度独立标记的发现，置信度按
/// 每多一个维度 +0.05、最多 +0.15 上调，并封顶 0.99（不冒充确定）。
pub(super) fn boost_cross_dimension_agreement(findings: &mut [Finding]) {
    for f in findings.iter_mut() {
        if f.agreed_dimensions >= 2 {
            let bonus = (0.05 * (f.agreed_dimensions - 1) as f32).min(0.15);
            f.confidence = (f.confidence + bonus).min(0.99);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Dimension, Reachability, Severity};

    fn finding(conf: f32, agreed: u8) -> Finding {
        Finding {
            dimension: Dimension::Logic,
            confidence: conf,
            severity: Severity::High,
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
            agreed_dimensions: agreed,
        }
    }

    #[test]
    fn sort_orders_unfiltered_severity_confidence() {
        let mut fs = vec![
            finding(0.95, 1), // 默认 High，未过滤
            {
                let mut f = finding(0.99, 1);
                f.filtered = true; // 高置信但被过滤 → 应排最后
                f
            },
            {
                let mut f = finding(0.70, 1);
                f.severity = Severity::Low; // 低危未过滤
                f
            },
        ];
        sort_findings(&mut fs);
        // 未过滤的 High(0.95) 第一，未过滤的 Low(0.70) 第二，被过滤的(0.99) 垫底。
        assert_eq!(fs[0].confidence, 0.95);
        assert_eq!(fs[1].confidence, 0.70);
        assert!(fs[2].filtered);
    }

    #[test]
    fn agreement_boost_scales_and_caps() {
        let mut fs = vec![
            finding(0.6, 1),  // 单维度：不加分
            finding(0.6, 2),  // +0.05
            finding(0.6, 4),  // +0.15（封顶）
            finding(0.95, 5), // 加分后封顶 0.99
        ];
        boost_cross_dimension_agreement(&mut fs);
        assert!((fs[0].confidence - 0.6).abs() < 1e-6);
        assert!((fs[1].confidence - 0.65).abs() < 1e-6);
        assert!((fs[2].confidence - 0.75).abs() < 1e-6);
        assert!((fs[3].confidence - 0.99).abs() < 1e-6);
    }
}

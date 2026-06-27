//! 跨维度去重：同一处问题被多个维度同时上报时合并，保留最可信的一条。
//!
//! 两条路径：
//! - **已定位**（start_line>0）：按 (path, start_line) 分组合并。
//! - **未定位**（start_line==0，重定位失败）：按内容聚类——同 path 且
//!   `existing_code` 有共同的「显著行」即视为同一处问题，跨维度合并。
//!   这能兜住"非连续片段 → 重定位失败 → 逃过去重"导致的同一 bug 多次上报。

use crate::model::Finding;
use std::collections::{HashMap, HashSet};

/// 规范化一行：折叠空白。
fn normalize(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 片段里的「显著行」：规范化后长度 ≥ 8 的行，用作内容指纹。
fn significant_lines(code: &str) -> HashSet<String> {
    code.lines()
        .map(normalize)
        .filter(|l| l.len() >= 8)
        .collect()
}

/// 从分组里选最佳并合并其它维度标注。
fn merge_group(mut group: Vec<Finding>) -> Finding {
    group.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.severity.cmp(&a.severity))
    });
    let mut best = group.remove(0);
    // 统计组内**不同维度**数（含 best 自身），作为跨维度交叉印证信号。
    let mut all_dims: Vec<&str> = std::iter::once(best.dimension.as_str())
        .chain(group.iter().map(|f| f.dimension.as_str()))
        .collect();
    all_dims.sort_unstable();
    all_dims.dedup();
    best.agreed_dimensions = all_dims.len().min(u8::MAX as usize) as u8;

    let others: Vec<&str> = all_dims
        .into_iter()
        .filter(|d| *d != best.dimension.as_str())
        .collect();
    if !others.is_empty() {
        best.message
            .push_str(&format!("（另由 {} 维度同时标记）", others.join("/")));
    }
    // 归属修正：若该发现引用了业务规则（[B1]/[B2]…），其语义归属应是 business 维度，
    // 而非"恰好置信度最高"的那个维度（去重前常被 security/logic 同时报）。
    if cites_business_rule(&best.message) {
        best.dimension = crate::model::Dimension::Business;
    }
    best
}

/// 消息是否引用了业务规则编号 `[B<数字>]`。
fn cites_business_rule(msg: &str) -> bool {
    let bytes = msg.as_bytes();
    msg.match_indices("[B").any(|(i, _)| {
        bytes
            .get(i + 2)
            .map(|b| b.is_ascii_digit())
            .unwrap_or(false)
    })
}

pub fn dedupe(findings: Vec<Finding>) -> Vec<Finding> {
    // 1) 已定位：按 (path, start_line) 分组。
    let mut located_order: Vec<(String, u32)> = Vec::new();
    let mut located: HashMap<(String, u32), Vec<Finding>> = HashMap::new();
    // 2) 未定位：内容聚类。每个簇 = (path, 显著行并集, 成员)。
    struct Cluster {
        path: String,
        sig: HashSet<String>,
        items: Vec<Finding>,
    }
    let mut clusters: Vec<Cluster> = Vec::new();

    for f in findings {
        if f.start_line > 0 {
            let key = (f.path.clone(), f.start_line);
            if !located.contains_key(&key) {
                located_order.push(key.clone());
            }
            located.entry(key).or_default().push(f);
            continue;
        }

        // 未定位：找内容相交的同 path 簇。
        let mut sig = significant_lines(&f.existing_code);
        if sig.is_empty() {
            // 没有显著代码行时退而用 message 作指纹。
            let m = normalize(&f.message);
            sig.insert(m.chars().take(60).collect());
        }
        let hit = clusters
            .iter_mut()
            .find(|c| c.path == f.path && c.sig.intersection(&sig).next().is_some());
        match hit {
            Some(c) => {
                c.sig.extend(sig);
                c.items.push(f);
            }
            None => clusters.push(Cluster {
                path: f.path.clone(),
                sig,
                items: vec![f],
            }),
        }
    }

    let mut out = Vec::new();
    for key in located_order {
        out.push(merge_group(located.remove(&key).unwrap()));
    }
    for c in clusters {
        out.push(merge_group(c.items));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Dimension, Severity};

    fn f(dim: Dimension, conf: f32, line: u32) -> Finding {
        Finding {
            dimension: dim,
            confidence: conf,
            severity: Severity::High,
            path: "x.rs".into(),
            start_line: line,
            end_line: line,
            message: "msg".into(),
            existing_code: "code".into(),
            evidence: String::new(),
            suggestion: None,
            suggestion_code: String::new(),
            reachability: crate::model::Reachability::default(),
            filtered: false,
            agreed_dimensions: 1,
            criterion: None,
            intent_status: None,
        }
    }

    fn fc(dim: Dimension, conf: f32, code: &str) -> Finding {
        Finding {
            dimension: dim,
            confidence: conf,
            severity: Severity::High,
            path: "x.h".into(),
            start_line: 0,
            end_line: 0,
            message: format!("{dim} 的描述"),
            existing_code: code.into(),
            evidence: String::new(),
            suggestion: None,
            suggestion_code: String::new(),
            reachability: crate::model::Reachability::default(),
            filtered: false,
            agreed_dimensions: 1,
            criterion: None,
            intent_status: None,
        }
    }

    #[test]
    fn merges_same_line_keeps_best() {
        let input = vec![
            f(Dimension::Security, 1.0, 3),
            f(Dimension::AiSmell, 0.9, 3),
            f(Dimension::Perf, 0.8, 7),
        ];
        let out = dedupe(input);
        assert_eq!(out.len(), 2);
        let line3 = out.iter().find(|x| x.start_line == 3).unwrap();
        assert_eq!(line3.dimension, Dimension::Security);
        assert!(line3.message.contains("ai_smell"));
        // 两个不同维度交叉印证。
        assert_eq!(line3.agreed_dimensions, 2);
        // 单独一条不应被记为多维度。
        let line7 = out.iter().find(|x| x.start_line == 7).unwrap();
        assert_eq!(line7.agreed_dimensions, 1);
    }

    #[test]
    fn unlocated_merged_by_shared_significant_line() {
        // 三条 line-0，existing_code 片段不同但共享关键行 → 应聚成 1 条。
        let key = "MemPool(MemPool&&) = default;";
        let input = vec![
            fc(
                Dimension::Security,
                0.95,
                &format!("{key}\n    ~MemPool();"),
            ),
            fc(Dimension::Logic, 0.9, &format!("{key}\n    other line;")),
            fc(
                Dimension::AiSmell,
                0.92,
                &format!("MemPool& operator=(const MemPool&) = delete;\n{key}\n~MemPool();"),
            ),
        ];
        let out = dedupe(input);
        assert_eq!(out.len(), 1, "三条应合并成 1 条");
        assert_eq!(out[0].dimension, Dimension::Security); // 置信度最高
        assert!(out[0].message.contains("logic") || out[0].message.contains("ai_smell"));
    }

    #[test]
    fn business_rule_citation_relabels_to_business() {
        // 一条 security 维度但引用了 [B2] 的发现，去重后应归到 business 维度。
        let mut sec = f(Dimension::Security, 0.99, 5);
        sec.message = "[B2] 越权访问：删除了 owner_id 校验".into();
        let out = dedupe(vec![sec]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].dimension, Dimension::Business);
    }

    #[test]
    fn non_rule_finding_keeps_dimension() {
        // 普通 security 发现（不引用规则）维持 security 维度。
        let mut sec = f(Dimension::Security, 0.99, 5);
        sec.message = "SQL 注入".into();
        let out = dedupe(vec![sec]);
        assert_eq!(out[0].dimension, Dimension::Security);
    }

    #[test]
    fn unlocated_distinct_issues_not_merged() {
        // 两条 line-0，无共享显著行 → 不合并。
        let input = vec![
            fc(Dimension::Logic, 0.9, "let a = foo_bar_baz();"),
            fc(Dimension::Perf, 0.9, "for x in huge_collection_iter {}"),
        ];
        let out = dedupe(input);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn sets_agreed_dimensions_count() {
        // 同一行被 3 个不同维度标记 → agreed_dimensions == 3；单独的那条 == 1。
        let input = vec![
            f(Dimension::Security, 1.0, 3),
            f(Dimension::AiSmell, 0.9, 3),
            f(Dimension::Logic, 0.85, 3),
            f(Dimension::Perf, 0.8, 7),
        ];
        let out = dedupe(input);
        let line3 = out.iter().find(|x| x.start_line == 3).unwrap();
        assert_eq!(line3.agreed_dimensions, 3);
        let line7 = out.iter().find(|x| x.start_line == 7).unwrap();
        assert_eq!(line7.agreed_dimensions, 1);
    }
}

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
            .push_str(&format!(" (also flagged by {})", others.join("/")));
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
        sig: HashSet<String>,
        items: Vec<Finding>,
    }
    let mut clusters: Vec<Cluster> = Vec::new();
    // path → 该 path 下各簇在 `clusters` 中的下标。未定位聚类只在**同 path** 的簇里找相交，
    // 避免对全体簇做 O(N×K) 线性扫（大量未定位发现时会退化）。
    let mut clusters_by_path: HashMap<String, Vec<usize>> = HashMap::new();

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
        let idxs = clusters_by_path.entry(f.path.clone()).or_default();
        let hit = idxs
            .iter()
            .copied()
            .find(|&i| clusters[i].sig.intersection(&sig).next().is_some());
        match hit {
            Some(i) => {
                clusters[i].sig.extend(sig);
                clusters[i].items.push(f);
            }
            None => {
                idxs.push(clusters.len());
                clusters.push(Cluster {
                    sig,
                    items: vec![f],
                });
            }
        }
    }

    // located 分组按精确 start_line；再合并「区间重叠 **且** 内容指纹相交」的组——
    // 同一问题被不同维度锚在略不同行时（如 logic@423-429 + ai_smell@426-429）精确分组会漏合。
    // 双重条件（行重叠 + 共享显著代码行）防误合：相邻但不同的问题不会共享 existing_code 内容。
    let located_groups: Vec<Vec<Finding>> = located_order
        .into_iter()
        .map(|key| located.remove(&key).unwrap())
        .collect();

    let mut out = Vec::new();
    for g in merge_overlapping_located(located_groups) {
        out.push(merge_group(g));
    }
    for c in clusters {
        out.push(merge_group(c.items));
    }
    out
}

/// 合并「同 path、行区间重叠、且 existing_code 显著行相交」的已定位组。
fn merge_overlapping_located(groups: Vec<Vec<Finding>>) -> Vec<Vec<Finding>> {
    struct G {
        path: String,
        start: u32,
        end: u32,
        sig: HashSet<String>,
        items: Vec<Finding>,
    }
    let mut gs: Vec<G> = groups
        .into_iter()
        .map(|items| {
            let path = items[0].path.clone();
            let start = items.iter().map(|f| f.start_line).min().unwrap_or(0);
            let end = items
                .iter()
                .map(|f| f.end_line.max(f.start_line))
                .max()
                .unwrap_or(0);
            let mut sig = HashSet::new();
            for f in &items {
                sig.extend(significant_lines(&f.existing_code));
            }
            G {
                path,
                start,
                end,
                sig,
                items,
            }
        })
        .collect();

    // 贪心合并：反复找一对可合并的组并入，直到不动点。每 PR 发现数少，足够。
    let mut merged = true;
    while merged {
        merged = false;
        'scan: for i in 0..gs.len() {
            for j in (i + 1)..gs.len() {
                let overlap = gs[i].path == gs[j].path
                    && gs[i].start <= gs[j].end
                    && gs[j].start <= gs[i].end;
                let shared = gs[i].sig.intersection(&gs[j].sig).next().is_some();
                if overlap && shared {
                    let b = gs.remove(j);
                    gs[i].start = gs[i].start.min(b.start);
                    gs[i].end = gs[i].end.max(b.end);
                    gs[i].sig.extend(b.sig);
                    gs[i].items.extend(b.items);
                    merged = true;
                    break 'scan;
                }
            }
        }
    }
    gs.into_iter().map(|g| g.items).collect()
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
    fn cites_business_rule_detects_multiple_citations() {
        assert!(cites_business_rule("[B1] rule"));
        assert!(cites_business_rule("[B12] rule"));
        assert!(!cites_business_rule("[BX] rule"));
        assert!(!cites_business_rule("no citation"));
    }

    #[test]
    fn unlocated_fallback_to_message_fingerprint() {
        // existing_code 无显著行，但 message 不同 → 不应合并。
        let input = vec![fc(Dimension::Logic, 0.9, "short"), {
            let mut f = fc(Dimension::Perf, 0.9, "short");
            f.message = "different message fingerprint".into();
            f
        }];
        let out = dedupe(input);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn normalize_collapses_whitespace() {
        assert_eq!(super::normalize("  a\t b  c "), "a b c");
    }

    /// 带行范围 + existing_code 的构造。
    fn frc(dim: Dimension, conf: f32, start: u32, end: u32, code: &str) -> Finding {
        Finding {
            start_line: start,
            end_line: end,
            existing_code: code.into(),
            message: format!("{dim} 说明"),
            ..f(dim, conf, start)
        }
    }

    #[test]
    fn overlapping_ranges_with_shared_code_merge() {
        // 同一问题被两维度锚在略不同的行（logic@10-16 + ai_smell@13-16），
        // 区间重叠 + 共享显著代码行 → 合并为一条（真实复现自 dbeaver/cline/ComfyUI）。
        let shared = "log.error(\"failed to get content\");";
        let input = vec![
            frc(Dimension::Logic, 0.72, 10, 16, shared),
            frc(Dimension::AiSmell, 0.6, 13, 16, shared),
        ];
        let out = dedupe(input);
        assert_eq!(out.len(), 1, "重叠+同内容应合并: {out:#?}");
        assert_eq!(out[0].agreed_dimensions, 2);
    }

    #[test]
    fn overlapping_ranges_but_different_code_do_not_merge() {
        // 防误合：区间重叠但 existing_code 无共同显著行（不同问题）→ 保留两条。
        let input = vec![
            frc(
                Dimension::Logic,
                0.9,
                10,
                20,
                "let sql = format!(\"select ...\");",
            ),
            frc(
                Dimension::Perf,
                0.9,
                15,
                16,
                "for item in huge_list.clone() {}",
            ),
        ];
        let out = dedupe(input);
        assert_eq!(out.len(), 2, "重叠但内容不同不应合并: {out:#?}");
    }

    #[test]
    fn non_overlapping_same_pattern_stays_separate() {
        // 同 bug 模式但在不同位置（不重叠）→ 每处都要修，分开报（真实来自 lvgl 两个函数）。
        let code = "span->txt == NULL && span->static_flag";
        let input = vec![
            frc(Dimension::AiSmell, 0.83, 216, 221, code),
            frc(Dimension::AiSmell, 0.83, 274, 279, code),
        ];
        let out = dedupe(input);
        assert_eq!(out.len(), 2, "不重叠的两处即使同模式也不合并: {out:#?}");
    }

    #[test]
    fn merge_group_single_finding_keeps_dimension_and_no_suffix() {
        let f = f(Dimension::Security, 0.9, 5);
        let out = dedupe(vec![f]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].agreed_dimensions, 1);
        assert!(!out[0].message.contains("also flagged by"));
    }

    #[test]
    fn significant_lines_filters_short_and_blank() {
        let sig = significant_lines("  \n  short  \nthis_is_long_enough\n");
        assert!(!sig.contains("short"));
        assert!(sig.contains("this_is_long_enough"));
    }

    #[test]
    fn merge_overlapping_chain_transitively_merges() {
        // A 与 B 重叠且共享代码，B 与 C 重叠且共享代码，但 A 与 C 不直接重叠。
        // 合并算法应通过传递闭包把三者合并为 1 组。
        let shared = "log.error(\"failed\");";
        let a = frc(Dimension::Security, 0.9, 10, 15, shared);
        let b = frc(Dimension::Logic, 0.85, 14, 20, shared);
        let c = frc(Dimension::AiSmell, 0.8, 19, 25, shared);
        let out = dedupe(vec![a, b, c]);
        assert_eq!(out.len(), 1, "传递重叠应合并为 1 组: {out:#?}");
        assert_eq!(out[0].agreed_dimensions, 3);
    }

    #[test]
    fn dedupe_preserves_empty_input() {
        assert!(dedupe(vec![]).is_empty());
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

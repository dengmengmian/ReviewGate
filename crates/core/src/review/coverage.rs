//! 大 PR / 多单元覆盖快照：已覆盖 vs 未完成路径，供报告与闸口叙事。

use crate::diff::Diff;
use crate::review::critical::{incomplete_advice, unfinished_paths};
use crate::review::units::{UnitJobSummary, UnitPlanSummary};
use crate::review::ReviewWarning;
use serde::Serialize;

/// 一次审查的路径覆盖快照（合成报告用）。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CoverageSnapshot {
    /// 本次 diff 改动的全部路径。
    pub changed_paths: Vec<String>,
    /// 被至少一个**可审**（非 oversized）单元覆盖的路径。
    pub planned_paths: Vec<String>,
    /// 因 oversized 在计划阶段就被跳过的路径。
    pub skipped_oversized_paths: Vec<String>,
    /// 审查过程中未完成的路径（warnings.paths / unit: 维度）。
    pub unfinished_paths: Vec<String>,
    /// 计划内且未出现在 unfinished/oversized 的路径（best-effort「已尝试覆盖」）。
    pub covered_paths: Vec<String>,
    /// 可操作建议（超时 / 拆 PR / 调预算）。
    pub advice: Vec<String>,
    /// 是否多单元。
    pub multi_unit: bool,
    /// 是否 incomplete。
    pub incomplete: bool,
}

impl CoverageSnapshot {
    /// 是否应在报告中展示（多单元 **或** incomplete；干净单单元不刷屏）。
    pub fn should_surface(&self) -> bool {
        self.multi_unit || self.incomplete || !self.skipped_oversized_paths.is_empty()
    }
}

/// 从 unit 计划 + warnings 组装覆盖快照。
pub fn build_coverage(
    diff: &Diff,
    unit_plan: &UnitPlanSummary,
    warnings: &[ReviewWarning],
    incomplete: bool,
) -> CoverageSnapshot {
    let changed_paths: Vec<String> = diff.files.iter().map(|f| f.path().to_string()).collect();

    let mut planned: Vec<String> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for job in &unit_plan.units {
        if job.oversized || job.status == "skipped_oversized" {
            for p in &job.paths {
                if !skipped.contains(p) {
                    skipped.push(p.clone());
                }
            }
        } else {
            for p in &job.paths {
                if !planned.contains(p) {
                    planned.push(p.clone());
                }
            }
        }
    }

    let unfinished = unfinished_paths(warnings);
    // Also treat oversized skips as unfinished for coverage narrative.
    let mut unfinished_all = unfinished.clone();
    for p in &skipped {
        if !unfinished_all.contains(p) {
            unfinished_all.push(p.clone());
        }
    }
    // Dimension-level incomplete (e.g. logic timed out) with no path anchors: do not
    // claim full path coverage — mark all planned paths unfinished for honesty.
    if incomplete && unfinished_all.is_empty() && !planned.is_empty() {
        unfinished_all = planned.clone();
    }

    let covered: Vec<String> = planned
        .iter()
        .filter(|p| !unfinished_all.contains(p))
        .cloned()
        .collect();

    let mut advice = incomplete_advice(warnings);
    if unit_plan.unit_count > 1 && !advice.iter().any(|a| a.contains("Split") || a.contains("拆")) {
        advice.push(
            "Large PR was split into directory-packed units; re-run unfinished units with higher --timeout or smaller diffs"
                .into(),
        );
    }
    if !skipped.is_empty() && !advice.iter().any(|a| a.contains("max_input_tokens")) {
        advice.push(
            "Oversized file diffs were skipped; split those files or raise provider max_input_tokens"
                .into(),
        );
    }

    CoverageSnapshot {
        changed_paths,
        planned_paths: planned,
        skipped_oversized_paths: skipped,
        unfinished_paths: unfinished_all,
        covered_paths: covered,
        advice,
        multi_unit: unit_plan.unit_count > 1,
        incomplete,
    }
}

/// 从 unit 列表刷新 status（oversized 跳过 vs 审查中 unfinished）。
pub fn refresh_unit_statuses(plan: &mut UnitPlanSummary, warnings: &[ReviewWarning]) {
    let unfinished = unfinished_paths(warnings);
    for job in &mut plan.units {
        if job.oversized {
            job.status = "skipped_oversized".into();
            continue;
        }
        let hit = job.paths.iter().any(|p| unfinished.contains(p))
            || warnings.iter().any(|w| {
                // Dimension-level incomplete (security/logic timed out) marks all planned paths unfinished for narrative.
                matches!(
                    w.kind,
                    "timed_out" | "failed" | "incomplete" | "auth_failed"
                ) && !w.dimension.starts_with("unit:")
                    && !w.dimension.starts_with("business")
            });
        // Dimension-level incomplete: mark reviewable units as incomplete (honest).
        let dim_incomplete = warnings.iter().any(|w| {
            matches!(
                w.kind,
                "timed_out" | "failed" | "incomplete" | "auth_failed"
            ) && !w.dimension.starts_with("unit:")
        });
        if hit || dim_incomplete {
            // Prefer path-specific unfinished when present
            if job.paths.iter().any(|p| unfinished.contains(p)) {
                job.status = "incomplete".into();
            } else if dim_incomplete {
                job.status = "incomplete".into();
            } else {
                job.status = "reviewed".into();
            }
        } else {
            job.status = "reviewed".into();
        }
    }
}

/// 干净单单元完整审查：不应捏造 incomplete 覆盖。
pub fn clean_single_unit_coverage(diff: &Diff) -> CoverageSnapshot {
    let paths: Vec<String> = diff.files.iter().map(|f| f.path().to_string()).collect();
    let plan = UnitPlanSummary {
        unit_count: 1,
        reviewable_units: 1,
        oversized_units: 0,
        units: vec![UnitJobSummary {
            id: 0,
            paths: paths.clone(),
            est_tokens: 0,
            oversized: false,
            status: "reviewed".into(),
        }],
    };
    build_coverage(diff, &plan, &[], false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileDiff, FileStatus, Hunk, Line, LineKind};
    use crate::llm::estimate_tokens;
    use crate::review::units::{plan_units, summarize_units};
    use crate::review::ReviewWarning;

    fn line(content: &str, no: u32) -> Line {
        Line {
            kind: LineKind::Added,
            content: content.into(),
            old_lineno: None,
            new_lineno: Some(no),
        }
    }

    fn file(path: &str, lines: u32, width: usize) -> FileDiff {
        let hunk = Hunk {
            old_start: 0,
            old_count: 0,
            new_start: 1,
            new_count: lines,
            section: String::new(),
            lines: (1..=lines).map(|n| line(&"x".repeat(width), n)).collect(),
        };
        FileDiff {
            old_path: None,
            new_path: Some(path.into()),
            status: FileStatus::Added,
            binary: false,
            hunks: vec![hunk],
        }
    }

    #[test]
    fn multi_unit_plan_groups_dirs_and_lists_paths() {
        let diff = Diff {
            files: vec![
                file("a/1.rs", 60, 40),
                file("a/2.rs", 60, 40),
                file("b/3.rs", 60, 40),
                file("b/4.rs", 60, 40),
            ],
        };
        let one = estimate_tokens(&diff.files[0].render_for_prompt());
        let budget = (one * 2) * 5 / 4 + 1;
        let units = plan_units(&diff, budget);
        assert!(units.len() > 1);
        let plan = summarize_units(&diff, &units);
        assert_eq!(plan.unit_count, units.len());
        // every path exactly once
        let mut all: Vec<String> = plan.units.iter().flat_map(|u| u.paths.clone()).collect();
        all.sort();
        let mut expected: Vec<String> = diff.files.iter().map(|f| f.path().to_string()).collect();
        expected.sort();
        assert_eq!(all, expected);
        // directory packing: a/* together
        let a_unit = plan
            .units
            .iter()
            .find(|u| u.paths.iter().any(|p| p.starts_with("a/")))
            .unwrap();
        assert!(a_unit.paths.iter().all(|p| p.starts_with("a/")));
    }

    #[test]
    fn incomplete_coverage_lists_unfinished_and_advice() {
        let diff = Diff {
            files: vec![file("src/auth/a.rs", 5, 10), file("src/ui/b.rs", 5, 10)],
        };
        let units = plan_units(&diff, 100_000);
        let plan = summarize_units(&diff, &units);
        let warnings = vec![ReviewWarning::new(
            "unit:src/auth/a.rs",
            "oversized",
            "too big",
        )
        .with_paths(vec!["src/auth/a.rs".into()])
        .with_advice("split")];
        let cov = build_coverage(&diff, &plan, &warnings, true);
        assert!(cov.should_surface());
        assert!(cov.unfinished_paths.iter().any(|p| p.contains("auth")));
        assert!(!cov.advice.is_empty());
        assert!(cov.incomplete);
    }

    #[test]
    fn clean_single_unit_does_not_invent_incomplete() {
        let diff = Diff {
            files: vec![file("a.rs", 5, 10)],
        };
        let cov = clean_single_unit_coverage(&diff);
        assert!(!cov.should_surface() || (!cov.incomplete && cov.unfinished_paths.is_empty()));
        assert!(cov.unfinished_paths.is_empty());
        assert!(!cov.incomplete);
        assert_eq!(cov.covered_paths, vec!["a.rs".to_string()]);
    }

    #[test]
    fn oversized_unit_marked_skipped_in_summary() {
        let diff = Diff {
            files: vec![file("small.rs", 3, 10), file("huge.rs", 5000, 80)],
        };
        let units = plan_units(&diff, 2000);
        let plan = summarize_units(&diff, &units);
        assert!(plan.oversized_units >= 1);
        assert!(plan
            .units
            .iter()
            .any(|u| u.oversized && u.status == "skipped_oversized"));
    }
}

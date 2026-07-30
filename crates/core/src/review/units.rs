//! 审查单元规划：把改动文件按 token 预算切成若干"审查单元"。
//!
//! **N 默认 = 1**：放得下就整包一个单元（正常 PR 零退化、缓存照旧）。放不下才按**目录就近**
//! 把相关文件聚在一起装箱，让相互调用的文件尽量同箱以保住跨文件推理；跨单元依赖仍可由
//! `read_file`/`find_callers` 工具按需够到。单文件 diff 自身就超预算的，独占一个 `oversized` 单元。

use crate::diff::Diff;
use crate::llm::estimate_tokens;
use serde::Serialize;
use std::path::Path;

/// 一个审查单元：一组 [`Diff::files`] 下标。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewUnit {
    /// 本单元包含的文件在 `diff.files` 中的下标。
    pub files: Vec<usize>,
    /// 单文件 diff 自身就超预算——无法再切小，需特殊处理（diff-only 重试或跳过）。
    pub oversized: bool,
    /// 本单元 diff 的估算 token（仅供日志/诊断）。
    pub est_tokens: usize,
}

/// 报告用：单个 unit/job 的路径清单。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UnitJobSummary {
    /// 0-based unit id。
    pub id: usize,
    /// 本单元覆盖的路径。
    pub paths: Vec<String>,
    /// 估算 token。
    pub est_tokens: usize,
    /// 是否 oversized（计划阶段即跳过审查内容）。
    pub oversized: bool,
    /// `planned` | `reviewed` | `skipped_oversized` | `incomplete`
    pub status: String,
}

/// 整次审查的 unit 计划摘要（多单元时合成报告的核心）。
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct UnitPlanSummary {
    pub unit_count: usize,
    pub reviewable_units: usize,
    pub oversized_units: usize,
    pub units: Vec<UnitJobSummary>,
}

/// 从 `plan_units` 结果生成面向用户的 job 摘要。
pub fn summarize_units(diff: &Diff, units: &[ReviewUnit]) -> UnitPlanSummary {
    let mut jobs = Vec::with_capacity(units.len());
    let mut oversized_units = 0usize;
    let mut reviewable = 0usize;
    for (id, u) in units.iter().enumerate() {
        let paths: Vec<String> = u
            .files
            .iter()
            .filter_map(|&i| diff.files.get(i).map(|f| f.path().to_string()))
            .collect();
        let status = if u.oversized {
            oversized_units += 1;
            "skipped_oversized"
        } else {
            reviewable += 1;
            "planned"
        };
        jobs.push(UnitJobSummary {
            id,
            paths,
            est_tokens: u.est_tokens,
            oversized: u.oversized,
            status: status.into(),
        });
    }
    UnitPlanSummary {
        unit_count: units.len(),
        reviewable_units: reviewable,
        oversized_units,
        units: jobs,
    }
}

/// 预留给单元上下文/工具轮次/输出的头寸：单元 diff 只占预算的 80%。
const UNIT_FILL_RATIO_NUM: usize = 4;
const UNIT_FILL_RATIO_DEN: usize = 5;

/// 取文件的目录 key（用于就近分组）。无父目录则为空串（仓库根）。
fn dir_key(diff: &Diff, idx: usize) -> String {
    let p = diff.files[idx].path();
    Path::new(p)
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 把 diff 切成审查单元。`budget` 为输入 token 预算（通常取 provider 的 `max_input_tokens`）。
pub fn plan_units(diff: &Diff, budget: usize) -> Vec<ReviewUnit> {
    let n = diff.files.len();
    if n == 0 {
        return Vec::new();
    }
    let usable = (budget * UNIT_FILL_RATIO_NUM / UNIT_FILL_RATIO_DEN).max(1);

    let est: Vec<usize> = diff
        .files
        .iter()
        .map(|f| estimate_tokens(&f.render_for_prompt()))
        .collect();
    let total: usize = est.iter().sum();

    // 正常 PR：整包一个单元。
    if total <= usable {
        return vec![ReviewUnit {
            files: (0..n).collect(),
            oversized: false,
            est_tokens: total,
        }];
    }

    // 超预算：按目录就近排序后贪心装箱。
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| {
        dir_key(diff, a)
            .cmp(&dir_key(diff, b))
            .then_with(|| diff.files[a].path().cmp(diff.files[b].path()))
    });

    let mut units: Vec<ReviewUnit> = Vec::new();
    let mut cur: Vec<usize> = Vec::new();
    let mut cur_est = 0usize;
    let flush = |cur: &mut Vec<usize>, cur_est: &mut usize, units: &mut Vec<ReviewUnit>| {
        if !cur.is_empty() {
            units.push(ReviewUnit {
                files: std::mem::take(cur),
                oversized: false,
                est_tokens: *cur_est,
            });
            *cur_est = 0;
        }
    };

    for &i in &idx {
        if est[i] > usable {
            // 单文件就超预算：先收尾当前箱，再独占一个 oversized 单元。
            flush(&mut cur, &mut cur_est, &mut units);
            units.push(ReviewUnit {
                files: vec![i],
                oversized: true,
                est_tokens: est[i],
            });
            continue;
        }
        if !cur.is_empty() && cur_est + est[i] > usable {
            flush(&mut cur, &mut cur_est, &mut units);
        }
        cur.push(i);
        cur_est += est[i];
    }
    flush(&mut cur, &mut cur_est, &mut units);
    units
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileDiff, FileStatus, Hunk, Line, LineKind};

    fn line(content: &str, no: u32) -> Line {
        Line {
            kind: LineKind::Added,
            content: content.into(),
            old_lineno: None,
            new_lineno: Some(no),
        }
    }

    /// 造一个新增文件，hunk 里塞 `lines` 行、每行 `width` 字符，用于撑出可控的 token 量。
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
    fn empty_diff_no_units() {
        assert!(plan_units(&Diff { files: vec![] }, 1000).is_empty());
    }

    #[test]
    fn small_diff_single_unit() {
        let diff = Diff {
            files: vec![
                file("a/x.rs", 5, 10),
                file("a/y.rs", 5, 10),
                file("b/z.rs", 5, 10),
            ],
        };
        let units = plan_units(&diff, 100_000);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].files, vec![0, 1, 2]);
        assert!(!units[0].oversized);
    }

    #[test]
    fn large_diff_splits_and_groups_by_directory() {
        // 每个文件估算约 (width+~10)*lines/3 token；给个很小的预算逼它切。
        let diff = Diff {
            files: vec![
                file("a/1.rs", 60, 40),
                file("a/2.rs", 60, 40),
                file("b/3.rs", 60, 40),
                file("b/4.rs", 60, 40),
            ],
        };
        // usable = budget*0.8。给约能装 ~2 个文件的预算。
        let one = estimate_tokens(&diff.files[0].render_for_prompt());
        let budget = (one * 2) * 5 / 4 + 1; // usable ≈ 2 个文件
        let units = plan_units(&diff, budget);
        assert!(units.len() >= 2, "应切成多个单元");
        // 同目录文件应落在同一单元（就近分组）：a/ 的两个下标 0,1 不应被拆到不同单元。
        let unit_of = |i: usize| units.iter().position(|u| u.files.contains(&i)).unwrap();
        assert_eq!(unit_of(0), unit_of(1), "a/ 目录两文件应同箱");
        assert_eq!(unit_of(2), unit_of(3), "b/ 目录两文件应同箱");
        // 所有文件都被覆盖且不重复。
        let mut all: Vec<usize> = units.iter().flat_map(|u| u.files.clone()).collect();
        all.sort_unstable();
        assert_eq!(all, vec![0, 1, 2, 3]);
    }

    #[test]
    fn dir_key_groups_by_parent() {
        let diff = Diff {
            files: vec![
                file("a/x.rs", 1, 1),
                file("a/b/y.rs", 1, 1),
                file("z.rs", 1, 1),
            ],
        };
        assert_eq!(dir_key(&diff, 0), "a");
        assert_eq!(dir_key(&diff, 1), "a/b");
        assert_eq!(dir_key(&diff, 2), "");
    }

    #[test]
    fn multiple_oversized_files_each_get_own_unit() {
        let diff = Diff {
            files: vec![
                file("small.rs", 3, 10),
                file("huge1.rs", 5000, 80),
                file("huge2.rs", 5000, 80),
            ],
        };
        let units = plan_units(&diff, 2000);
        let oversized: Vec<_> = units.iter().filter(|u| u.oversized).collect();
        assert_eq!(oversized.len(), 2);
    }

    #[test]
    fn zero_budget_still_produces_oversized_units() {
        // budget=0 时 usable=max(1)，空 diff 应返回空，但 oversized 文件仍独占单元。
        assert!(plan_units(&Diff { files: vec![] }, 0).is_empty());

        let diff = Diff {
            files: vec![file("small.rs", 1, 1), file("huge.rs", 5000, 80)],
        };
        let units = plan_units(&diff, 0);
        assert!(
            units.iter().any(|u| u.oversized && u.files == vec![1]),
            "超大文件应在 budget=0 时独占 oversized 单元: {units:?}"
        );
    }

    #[test]
    fn mixed_oversized_and_normal_files_are_not_combined() {
        // 超大文件不应被合并进普通单元。
        let diff = Diff {
            files: vec![
                file("a/normal.rs", 10, 10),
                file("a/huge.rs", 5000, 80),
                file("b/normal.rs", 10, 10),
            ],
        };
        let units = plan_units(&diff, 2000);
        let huge_unit = units
            .iter()
            .find(|u| u.files.len() == 1 && u.files[0] == 1)
            .expect("huge 应独占单元");
        assert!(huge_unit.oversized);
        // 普通文件应被覆盖且不重复。
        let mut all: Vec<usize> = units.iter().flat_map(|u| u.files.clone()).collect();
        all.sort_unstable();
        assert_eq!(all, vec![0, 1, 2]);
    }

    #[test]
    fn single_oversized_file_gets_its_own_unit() {
        let diff = Diff {
            files: vec![file("small.rs", 3, 10), file("huge.rs", 5000, 80)],
        };
        // 预算让 huge.rs 单文件就超 usable。
        let units = plan_units(&diff, 2000);
        let huge_unit = units
            .iter()
            .find(|u| u.files == vec![1])
            .expect("huge 独占单元");
        assert!(huge_unit.oversized);
    }

    #[test]
    fn plan_units_excludes_binary_files_like_empty_render() {
        let mut f = file("img.png", 1, 1);
        f.binary = true;
        let diff = Diff {
            files: vec![f, file("a.rs", 5, 10)],
        };
        let units = plan_units(&diff, 100_000);
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].files, vec![0, 1]);
    }

    #[test]
    fn plan_units_respects_directory_grouping_on_split() {
        // 每个文件估算约 (width+~10)*lines/3 token；给个很小的预算逼它切。
        let diff = Diff {
            files: vec![
                file("a/1.rs", 60, 40),
                file("a/2.rs", 60, 40),
                file("b/3.rs", 60, 40),
                file("b/4.rs", 60, 40),
            ],
        };
        // usable = budget*0.8。给约能装 ~2 个文件的预算。
        let one = estimate_tokens(&diff.files[0].render_for_prompt());
        let budget = (one * 2) * 5 / 4 + 1; // usable ≈ 2 个文件
        let units = plan_units(&diff, budget);
        // 同目录文件应落在同一单元（就近分组）：a/ 的两个下标 0,1 不应被拆到不同单元。
        let unit_of = |i: usize| units.iter().position(|u| u.files.contains(&i)).unwrap();
        assert_eq!(unit_of(0), unit_of(1), "a/ 目录两文件应同箱");
        assert_eq!(unit_of(2), unit_of(3), "b/ 目录两文件应同箱");
    }

    #[test]
    fn tokens_estimated_positive() {
        let diff = Diff {
            files: vec![file("x.rs", 10, 10)],
        };
        let units = plan_units(&diff, 100_000);
        assert!(units[0].est_tokens > 0);
    }

    #[test]
    fn summarize_units_lists_every_path_exactly_once() {
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
        assert!(units.len() > 1, "fixture must force multi-unit");
        let plan = summarize_units(&diff, &units);
        assert_eq!(plan.unit_count, units.len());
        let mut all: Vec<String> = plan.units.iter().flat_map(|u| u.paths.clone()).collect();
        all.sort();
        let mut expected: Vec<String> = diff.files.iter().map(|f| f.path().to_string()).collect();
        expected.sort();
        assert_eq!(all, expected, "every changed file in exactly one unit");
        // no path appears twice
        let mut seen = std::collections::HashSet::new();
        for p in &all {
            assert!(seen.insert(p.clone()), "duplicate path {p}");
        }
    }
}

//! 审查单元规划：把改动文件按 token 预算切成若干"审查单元"。
//!
//! **N 默认 = 1**：放得下就整包一个单元（正常 PR 零退化、缓存照旧）。放不下才按**目录就近**
//! 把相关文件聚在一起装箱，让相互调用的文件尽量同箱以保住跨文件推理；跨单元依赖仍可由
//! `read_file`/`find_callers` 工具按需够到。单文件 diff 自身就超预算的，独占一个 `oversized` 单元。

use crate::diff::Diff;
use crate::llm::estimate_tokens;
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
}

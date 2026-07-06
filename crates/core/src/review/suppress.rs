//! 误报抑制：给每条 finding 一个**稳定指纹**，团队把确认过的误报指纹写进仓库根的
//! `.reviewgate/ignore`，下次命中同一指纹的 finding 标记为已过滤（不进闸口判定，
//! 但 `--show-filtered` 仍可展开）——复用 `filtered` 机制，透明、可审计、全队共享。
//!
//! 指纹刻意**不含行号**：同一段代码的同一个误报，即使后续改动导致行号漂移，也要仍能命中。

use crate::model::Finding;
use std::collections::HashSet;
use std::path::Path;

/// 计算一条 finding 的稳定指纹（12 位 hex）。
///
/// 输入 = `path` + `dimension` + 归一化后的 `existing_code`（折叠每行空白）。
/// **不含行号、不含 message**——行号会漂移，message 是 LLM 生成、每次措辞不同。
/// 用 FNV-1a：跨 Rust 版本/平台稳定，可安全提交进仓库的 ignore 文件
/// （`std::hash::DefaultHasher` 不保证稳定，不能用于此）。
pub fn fingerprint(f: &Finding) -> String {
    // 归一化代码：折叠所有空白（缩进/换行/多空格），吸收格式抖动。
    let code = f
        .existing_code
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let input = format!("{}\0{}\0{}", f.path, f.dimension.as_str(), code);
    format!("{:016x}", fnv1a_64(input.as_bytes()))[..12].to_string()
}

/// 加载仓库根的 `.reviewgate/ignore` 抑制指纹集。
///
/// 格式：每行一个指纹（首个空白分隔 token），`#` 开始为注释、空行忽略。
/// 文件不存在或读失败 → 返回空集（优雅降级，绝不因此中断审查）。
pub fn load_ignore(repo_root: &Path) -> HashSet<String> {
    let path = repo_root.join(".reviewgate").join("ignore");
    let Ok(content) = std::fs::read_to_string(&path) else {
        return HashSet::new();
    };
    content
        .lines()
        .filter_map(|line| {
            // 剥行尾注释，取首个空白分隔 token 作为指纹。
            let code = line.split('#').next().unwrap_or("");
            code.split_whitespace().next().map(str::to_string)
        })
        .collect()
}

/// 抑制命中 ignore 指纹的 finding：标记 `filtered`，从主集拆出。
///
/// 返回 `(保留, 被抑制)`。被抑制项应在**闸口之后**并回（供 `--show-filtered` 展示），
/// 不参与闸口判定——这样一条被确认为误报的高危发现不会再 BLOCK 合并。
/// `ignore` 为空时零开销，原样返回全部保留。
pub fn apply_suppression(
    findings: Vec<Finding>,
    ignore: &HashSet<String>,
) -> (Vec<Finding>, Vec<Finding>) {
    if ignore.is_empty() {
        return (findings, Vec::new());
    }
    let mut kept = Vec::new();
    let mut suppressed = Vec::new();
    for mut f in findings {
        if ignore.contains(&fingerprint(&f)) {
            // 只标 filtered（不改文案）：抑制项在闸口后并回、渲染归入 filtered 组，
            // 带指纹供辨识；报告文案跟 output_language，core 不注入硬编码文本。
            f.filtered = true;
            suppressed.push(f);
        } else {
            kept.push(f);
        }
    }
    (kept, suppressed)
}

/// FNV-1a 64 位哈希。跨 Rust 版本/平台稳定，故可用于提交进仓库的 ignore 指纹。
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Dimension, Reachability, Severity};

    fn finding(path: &str, dim: Dimension, start: u32, existing: &str) -> Finding {
        Finding {
            dimension: dim,
            confidence: 0.9,
            severity: Severity::High,
            path: path.into(),
            start_line: start,
            end_line: start,
            message: "some message".into(),
            existing_code: existing.into(),
            evidence: String::new(),
            suggestion: None,
            suggestion_code: String::new(),
            reachability: Reachability::Unknown,
            filtered: false,
            agreed_dimensions: 1,
            criterion: None,
            intent_status: None,
        }
    }

    #[test]
    fn fingerprint_is_stable_and_short_hex() {
        let f = finding(
            "src/handler.rs",
            Dimension::Security,
            10,
            "let q = format!(\"...\");",
        );
        let fp = fingerprint(&f);
        assert_eq!(fp.len(), 12, "fingerprint should be 12 hex chars");
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
        // 确定性：同输入两次一致。
        assert_eq!(fp, fingerprint(&f));
    }

    #[test]
    fn fingerprint_ignores_line_numbers_and_whitespace() {
        // 同 path+dimension+代码内容，但行号不同、缩进/空白不同 → 必须同指纹。
        let a = finding(
            "src/handler.rs",
            Dimension::Security,
            10,
            "let q = format!(\"x\");",
        );
        let b = finding(
            "src/handler.rs",
            Dimension::Security,
            42,
            "  let   q =    format!(\"x\");  ",
        );
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn load_ignore_missing_file_is_empty() {
        // 不存在的目录/文件 → 空集，不 panic。
        let set = load_ignore(Path::new("/nonexistent/reviewgate/repo"));
        assert!(set.is_empty());
    }

    #[test]
    fn load_ignore_parses_fingerprints_skipping_comments() {
        let dir = std::env::temp_dir().join(format!("rg-ignore-test-{}", std::process::id()));
        let rg = dir.join(".reviewgate");
        std::fs::create_dir_all(&rg).unwrap();
        std::fs::write(
            rg.join("ignore"),
            "# 这是注释\n\na3f2c1b09d4e\nbbbbbbbbbbbb  # 行尾人类说明\n   \n   cccccccccccc  维度 path 备注\n",
        )
        .unwrap();

        let set = load_ignore(&dir);
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(set.len(), 3);
        assert!(set.contains("a3f2c1b09d4e"));
        assert!(set.contains("bbbbbbbbbbbb")); // 行尾注释被剥掉
        assert!(set.contains("cccccccccccc")); // 只取首个 token
    }

    #[test]
    fn apply_suppression_partitions_and_marks_matched() {
        let hit = finding("src/a.rs", Dimension::Security, 10, "let q = 1;");
        let miss = finding("src/b.rs", Dimension::Logic, 5, "let z = 2;");
        let fp = fingerprint(&hit);
        let ignore: HashSet<String> = std::iter::once(fp.clone()).collect();

        let (kept, suppressed) = apply_suppression(vec![hit, miss], &ignore);

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].path, "src/b.rs"); // 未命中的保留、不被标记
        assert!(!kept[0].filtered);

        assert_eq!(suppressed.len(), 1);
        assert_eq!(suppressed[0].path, "src/a.rs");
        assert!(suppressed[0].filtered, "被抑制项应标 filtered");
        // 不改写 message：报告文案跟 output_language，core 不注入硬编码英文（本地化交给渲染层）。
        assert_eq!(
            suppressed[0].message, "some message",
            "抑制不应改动 finding 文案"
        );
    }

    #[test]
    fn apply_suppression_empty_ignore_keeps_all() {
        let a = finding("src/a.rs", Dimension::Security, 10, "let q = 1;");
        let (kept, suppressed) = apply_suppression(vec![a], &HashSet::new());
        assert_eq!(kept.len(), 1);
        assert!(suppressed.is_empty());
        assert!(!kept[0].filtered);
    }

    #[test]
    fn fingerprint_differs_on_code_dimension_or_path() {
        let base = finding("src/handler.rs", Dimension::Security, 10, "let q = 1;");
        // 代码不同。
        let diff_code = finding("src/handler.rs", Dimension::Security, 10, "let q = 2;");
        assert_ne!(fingerprint(&base), fingerprint(&diff_code));
        // 维度不同。
        let diff_dim = finding("src/handler.rs", Dimension::Logic, 10, "let q = 1;");
        assert_ne!(fingerprint(&base), fingerprint(&diff_dim));
        // 路径不同。
        let diff_path = finding("src/other.rs", Dimension::Security, 10, "let q = 1;");
        assert_ne!(fingerprint(&base), fingerprint(&diff_path));
    }
}

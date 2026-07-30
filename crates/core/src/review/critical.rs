//! 大 PR / 关键路径 incomplete 策略。

use crate::review::ReviewWarning;
use globset::{Glob, GlobSetBuilder};

/// 内置「安全相关路径」glob（`force_fail_incomplete_paths = null` 时使用）。
pub fn builtin_critical_globs() -> Vec<&'static str> {
    vec![
        "**/auth/**",
        "**/authentication/**",
        "**/payment/**",
        "**/billing/**",
        "**/security/**",
        "**/*secret*",
        "**/crypto/**",
        "**/oauth/**",
        "**/password*",
    ]
}

/// 解析配置：`None` → 内置；`Some([])` → 关闭；`Some(list)` → 自定义。
pub fn resolve_critical_globs(config: &Option<Vec<String>>) -> Vec<String> {
    match config {
        None => builtin_critical_globs()
            .into_iter()
            .map(|s| s.to_string())
            .collect(),
        Some(v) => v.clone(),
    }
}

/// 路径是否命中任一 glob。
pub fn path_matches_any(path: &str, globs: &[String]) -> bool {
    if globs.is_empty() {
        return false;
    }
    let mut b = GlobSetBuilder::new();
    for g in globs {
        if let Ok(glob) = Glob::new(g) {
            b.add(glob);
        }
    }
    let Ok(set) = b.build() else {
        return false;
    };
    // Also try path without leading ./
    let p = path.trim_start_matches("./");
    set.is_match(p) || set.is_match(path)
}

fn is_hard_incomplete_kind(kind: &str) -> bool {
    matches!(
        kind,
        "oversized" | "timed_out" | "failed" | "incomplete" | "auth_failed" | "context_overflow"
    )
}

/// 告警是否表明 `path` 未审完（显式 paths、unit: 维度、oversized 标签、或 security 维失败）。
fn warning_marks_path_unfinished(w: &ReviewWarning, path: &str) -> bool {
    if w.paths.iter().any(|up| up == path) {
        return true;
    }
    if w.dimension == "security" {
        return true;
    }
    if w.dimension == format!("unit:{path}") || w.dimension.starts_with(&format!("unit:{path}")) {
        return true;
    }
    if w.kind == "oversized" && (w.paths.iter().any(|x| x == path) || w.dimension.contains(path))
    {
        return true;
    }
    false
}

/// incomplete 时：未审完路径或 security 维失败 + 改动触及关键路径 → 应强制失败。
pub fn critical_incomplete_forces_fail(
    incomplete: bool,
    warnings: &[ReviewWarning],
    changed_paths: &[String],
    critical_globs: &[String],
) -> bool {
    if !incomplete || critical_globs.is_empty() {
        return false;
    }

    // 1) 告警上显式携带的路径 / unit: 维度路径命中关键 glob。
    for w in warnings {
        for p in &w.paths {
            if path_matches_any(p, critical_globs) {
                return true;
            }
        }
        if let Some(rest) = w.dimension.strip_prefix("unit:") {
            if path_matches_any(rest, critical_globs) {
                return true;
            }
        }
    }

    // 2) security 维未审完 → 任一改动文件落在关键路径上即强制失败。
    let security_incomplete = warnings.iter().any(|w| w.dimension == "security");
    if security_incomplete {
        for p in changed_paths {
            if path_matches_any(p, critical_globs) {
                return true;
            }
        }
    }

    // 3) 有 hard incomplete 类告警时：关键路径若被标为未审完（含 oversized unit）→ 强制失败。
    let hard = warnings.iter().any(|w| is_hard_incomplete_kind(w.kind));
    if hard {
        for p in changed_paths {
            if !path_matches_any(p, critical_globs) {
                continue;
            }
            if warnings.iter().any(|w| warning_marks_path_unfinished(w, p)) {
                return true;
            }
        }
    }

    false
}

/// 从 warnings 汇总未覆盖路径与建议。
pub fn incomplete_advice(warnings: &[ReviewWarning]) -> Vec<String> {
    let mut advice = Vec::new();
    let kinds: Vec<&str> = warnings.iter().map(|w| w.kind).collect();
    if kinds.iter().any(|k| *k == "timed_out") {
        advice.push("Raise --timeout (e.g. --timeout 300) and re-run".into());
    }
    if kinds.iter().any(|k| *k == "oversized") {
        advice.push("Split oversized files into smaller PRs or raise provider max_input_tokens".into());
    }
    if kinds.iter().any(|k| *k == "auth_failed") {
        advice.push("Fix API key (REVIEWGATE_API_KEY or config) and re-run".into());
    }
    if kinds.iter().any(|k| *k == "incomplete" || *k == "failed") {
        advice.push("Re-run with -v; consider --samples 2 for flaky dimensions".into());
    }
    if advice.is_empty() && !warnings.is_empty() {
        advice.push("Re-run: reviewgate review --timeout 300 -v".into());
    }
    advice
}

pub fn unfinished_paths(warnings: &[ReviewWarning]) -> Vec<String> {
    let mut out = Vec::new();
    for w in warnings {
        for p in &w.paths {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
        if let Some(rest) = w.dimension.strip_prefix("unit:") {
            let p = rest.to_string();
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::review::ReviewWarning;

    #[test]
    fn builtin_auth_path_matches() {
        let g = resolve_critical_globs(&None);
        assert!(path_matches_any("src/auth/login.rs", &g));
        assert!(path_matches_any("payment/charge.go", &g));
        assert!(!path_matches_any("src/ui/button.tsx", &g));
    }

    #[test]
    fn empty_config_disables() {
        let g = resolve_critical_globs(&Some(vec![]));
        assert!(g.is_empty());
        assert!(!critical_incomplete_forces_fail(
            true,
            &[],
            &["src/auth/a.rs".into()],
            &g
        ));
    }

    #[test]
    fn security_dim_incomplete_on_auth_forces() {
        let g = resolve_critical_globs(&None);
        let w = vec![ReviewWarning::new("security", "timed_out", "timeout")];
        assert!(critical_incomplete_forces_fail(
            true,
            &w,
            &["src/auth/login.rs".into()],
            &g
        ));
    }

    #[test]
    fn oversized_critical_path_forces() {
        let g = resolve_critical_globs(&None);
        let w = vec![ReviewWarning::new("unit:src/payment/pay.rs", "oversized", "big")
            .with_paths(vec!["src/payment/pay.rs".into()])];
        assert!(critical_incomplete_forces_fail(
            true,
            &w,
            &["src/payment/pay.rs".into()],
            &g
        ));
    }

    #[test]
    fn complete_review_never_forces() {
        let g = resolve_critical_globs(&None);
        assert!(!critical_incomplete_forces_fail(
            false,
            &[],
            &["src/auth/x.rs".into()],
            &g
        ));
    }

    #[test]
    fn non_critical_path_with_timeout_does_not_force() {
        let g = resolve_critical_globs(&None);
        let w = vec![ReviewWarning::new("logic", "timed_out", "timeout")];
        assert!(!critical_incomplete_forces_fail(
            true,
            &w,
            &["src/ui/button.tsx".into()],
            &g
        ));
    }
}

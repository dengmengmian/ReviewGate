//! 增量复审缓存（opt-in，`--incremental`）：按**文件**缓存发现。
//!
//! 键 = 该文件的 diff 内容 + **评审签名**。文件 hunk 逐字节不变且签名不变 →
//! 复用该文件上轮的发现，跳过最贵的 LLM fan-out；只重审内容变了的文件。
//!
//! 之所以正确：ReviewGate 只报**改动行上**的发现（见 LIMITATIONS #5），发现天然锚定
//! 在有 diff 的文件上；文件 diff 逐字节相同 → 发现不变。跨文件上下文由 Agent 按需拉取，
//! 不影响缓存正确性。任何会改变"同一文件产出什么发现"的输入（维度/模型/规则/采样/
//! exec_verify）都进签名，一变则整体失效——绝不复用过期结果。
//!
//! 缓存是纯本地性能优化，存 `.reviewgate/cache/incremental.json`（应 gitignore）。

use crate::diff::Diff;
use crate::model::{Dimension, Finding};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::suppress::short_hash;

/// 评审签名：任何影响"同一文件会产出什么发现"的输入都进签名，变了则缓存整体失效。
///
/// `prompt_version` 让 prompt/编排升级后旧缓存自动作废（发版时手动 bump）。
pub fn review_signature(
    dimensions: &[Dimension],
    model: &str,
    rules_body: &str,
    judge: bool,
    samples: usize,
    exec_verify: bool,
) -> String {
    let mut dims: Vec<&str> = dimensions.iter().map(|d| d.as_str()).collect();
    dims.sort_unstable();
    dims.dedup();
    // prompt_version：编排/prompt 有语义变更时 bump，让旧缓存自动作废。
    let input = format!(
        "v1\0{}\0{}\0{}\0{}\0{}\0{}",
        dims.join(","),
        model,
        rules_body,
        judge,
        samples,
        exec_verify
    );
    short_hash(input.as_bytes())
}

/// 单文件缓存键 = 评审签名 + 该文件 diff 内容。
pub fn file_key(file_diff_content: &str, signature: &str) -> String {
    short_hash(format!("{signature}\0{file_diff_content}").as_bytes())
}

/// 按缓存分区：返回 (需重审的文件下标, 复用的缓存发现)。
/// 命中（文件 diff 内容 + 签名一致）→ 复用其发现；未命中 → 进 todo 重审。
pub fn partition(
    diff: &Diff,
    signature: &str,
    cache: &IncrementalCache,
) -> (Vec<usize>, Vec<Finding>) {
    let mut todo = Vec::new();
    let mut reused = Vec::new();
    for (idx, file) in diff.files.iter().enumerate() {
        let key = file_key(&file.render_for_prompt(), signature);
        match cache.get(&key) {
            Some(hits) => reused.extend(hits.iter().cloned()),
            None => todo.push(idx),
        }
    }
    (todo, reused)
}

/// 把本轮重审(todo)文件的新发现写回缓存：按 path 归组，每个 todo 文件写一条键
/// （**零发现也写空 vec**——否则干净文件每轮都重审，增量白做）。
///
/// 排除 `intent` 维度：意图评审是整体性的、依赖每次不同的意图文档，不可按文件缓存，
/// 每轮照常全跑。
pub fn store(
    cache: &mut IncrementalCache,
    diff: &Diff,
    todo: &[usize],
    fresh: &[Finding],
    signature: &str,
) {
    let mut by_path: HashMap<&str, Vec<Finding>> = HashMap::new();
    for f in fresh.iter().filter(|f| f.dimension != Dimension::Intent) {
        by_path.entry(f.path.as_str()).or_default().push(f.clone());
    }
    for &idx in todo {
        let file = &diff.files[idx];
        let key = file_key(&file.render_for_prompt(), signature);
        cache.insert(key, by_path.get(file.path()).cloned().unwrap_or_default());
    }
}

/// 磁盘缓存：键 → 该文件的发现列表（已是终态：含 judge 后置信度）。
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct IncrementalCache {
    entries: HashMap<String, Vec<Finding>>,
}

fn cache_path(repo_root: &Path) -> PathBuf {
    repo_root
        .join(".reviewgate")
        .join("cache")
        .join("incremental.json")
}

impl IncrementalCache {
    /// 从仓库根加载。文件缺失/损坏 → 空缓存（优雅降级，绝不因缓存问题中断审查）。
    pub fn load(repo_root: &Path) -> Self {
        std::fs::read_to_string(cache_path(repo_root))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// 命中则返回该文件缓存的发现。
    pub fn get(&self, key: &str) -> Option<&Vec<Finding>> {
        self.entries.get(key)
    }

    /// 写入/覆盖某文件键的发现。
    pub fn insert(&mut self, key: String, findings: Vec<Finding>) {
        self.entries.insert(key, findings);
    }

    /// 持久化到 `.reviewgate/cache/incremental.json`（自动建目录）。
    ///
    /// 同时在缓存目录写一个 `.gitignore`（`*`）让缓存**自忽略**——否则未跟踪的缓存文件
    /// 会被 `git ls-files --others` 收进 diff、当成新增代码审查（自食其果）。有了它，
    /// 即使用户没在仓库 gitignore 里排除，缓存也永不进 review。
    pub fn save(&self, repo_root: &Path) -> std::io::Result<()> {
        let path = cache_path(repo_root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            let gi = parent.join(".gitignore");
            if !gi.exists() {
                std::fs::write(gi, "*\n")?;
            }
        }
        std::fs::write(path, serde_json::to_string(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Reachability, Severity};

    fn sig() -> String {
        review_signature(
            &[Dimension::Security, Dimension::Logic],
            "deepseek-v4-pro",
            "rules body",
            true,
            1,
            false,
        )
    }

    #[test]
    fn signature_stable_and_order_independent() {
        let a = review_signature(
            &[Dimension::Security, Dimension::Logic],
            "m",
            "r",
            true,
            1,
            false,
        );
        // 维度顺序不同 → 同签名（集合语义）。
        let b = review_signature(
            &[Dimension::Logic, Dimension::Security],
            "m",
            "r",
            true,
            1,
            false,
        );
        assert_eq!(a, b);
        assert_eq!(a.len(), 12);
    }

    #[test]
    fn signature_changes_on_any_input() {
        let base = sig();
        assert_ne!(
            base,
            review_signature(
                &[Dimension::Security],
                "deepseek-v4-pro",
                "rules body",
                true,
                1,
                false
            )
        );
        assert_ne!(
            base,
            review_signature(
                &[Dimension::Security, Dimension::Logic],
                "other-model",
                "rules body",
                true,
                1,
                false
            )
        );
        assert_ne!(
            base,
            review_signature(
                &[Dimension::Security, Dimension::Logic],
                "deepseek-v4-pro",
                "CHANGED rules",
                true,
                1,
                false
            )
        );
        // judge / samples / exec_verify 各自都影响签名。
        assert_ne!(
            base,
            review_signature(
                &[Dimension::Security, Dimension::Logic],
                "deepseek-v4-pro",
                "rules body",
                false,
                1,
                false
            )
        );
        assert_ne!(
            base,
            review_signature(
                &[Dimension::Security, Dimension::Logic],
                "deepseek-v4-pro",
                "rules body",
                true,
                3,
                false
            )
        );
        assert_ne!(
            base,
            review_signature(
                &[Dimension::Security, Dimension::Logic],
                "deepseek-v4-pro",
                "rules body",
                true,
                1,
                true
            )
        );
    }

    #[test]
    fn file_key_depends_on_content_and_signature() {
        let s = sig();
        let k1 = file_key("diff content A", &s);
        assert_eq!(k1.len(), 12);
        assert_ne!(k1, file_key("diff content B", &s)); // 内容变
        assert_ne!(k1, file_key("diff content A", "othersig")); // 签名变
        assert_eq!(k1, file_key("diff content A", &s)); // 确定性
    }

    fn finding(path: &str) -> Finding {
        Finding {
            dimension: Dimension::Security,
            confidence: 0.9,
            severity: Severity::High,
            path: path.into(),
            start_line: 3,
            end_line: 3,
            message: "m".into(),
            existing_code: "code".into(),
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
    fn cache_missing_dir_loads_empty_and_get_is_none() {
        let c = IncrementalCache::load(Path::new("/nonexistent/rg/repo"));
        assert!(c.get("anykey").is_none());
    }

    fn file_diff(path: &str, content: &str) -> crate::diff::FileDiff {
        use crate::diff::{FileStatus, Hunk, Line, LineKind};
        crate::diff::FileDiff {
            old_path: None,
            new_path: Some(path.into()),
            status: FileStatus::Added,
            binary: false,
            hunks: vec![Hunk {
                old_start: 0,
                old_count: 0,
                new_start: 1,
                new_count: 1,
                section: String::new(),
                lines: vec![Line {
                    kind: LineKind::Added,
                    content: content.into(),
                    old_lineno: None,
                    new_lineno: Some(1),
                }],
            }],
        }
    }

    #[test]
    fn partition_reuses_hits_and_lists_misses() {
        let s = sig();
        let diff = Diff {
            files: vec![file_diff("a.rs", "code A"), file_diff("b.rs", "code B")],
        };
        let mut cache = IncrementalCache::default();
        // 预置 a.rs 的缓存（键 = a.rs 的 diff 内容 + 签名）。
        let mut f = finding("a.rs");
        f.dimension = Dimension::Logic;
        let key_a = file_key(&diff.files[0].render_for_prompt(), &s);
        cache.insert(key_a, vec![f]);

        let (todo, reused) = partition(&diff, &s, &cache);
        assert_eq!(todo, vec![1], "只有未命中的 b.rs 进 todo");
        assert_eq!(reused.len(), 1);
        assert_eq!(reused[0].path, "a.rs");
    }

    #[test]
    fn store_writes_per_file_including_empty() {
        let s = sig();
        let diff = Diff {
            files: vec![file_diff("a.rs", "code A"), file_diff("clean.rs", "code C")],
        };
        // a.rs 有一条发现；clean.rs 无发现；一条 intent 发现应被排除。
        let mut intent = finding("a.rs");
        intent.dimension = Dimension::Intent;
        let fresh = vec![finding("a.rs"), intent];

        let mut cache = IncrementalCache::default();
        store(&mut cache, &diff, &[0, 1], &fresh, &s);

        let key_a = file_key(&diff.files[0].render_for_prompt(), &s);
        let key_clean = file_key(&diff.files[1].render_for_prompt(), &s);
        assert_eq!(
            cache.get(&key_a).unwrap().len(),
            1,
            "a.rs 缓存 1 条(不含 intent)"
        );
        assert_eq!(
            cache.get(&key_clean).map(Vec::len),
            Some(0),
            "干净文件也写空 vec，命中后跳过重审"
        );
    }

    #[test]
    fn cache_insert_save_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!("rg-inc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut c = IncrementalCache::default();
        c.insert("k1".into(), vec![finding("a.rs")]);
        c.save(&dir).unwrap();

        let loaded = IncrementalCache::load(&dir);
        std::fs::remove_dir_all(&dir).ok();

        let hit = loaded.get("k1").expect("k1 应命中");
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].path, "a.rs");
        assert!(loaded.get("missing").is_none());
    }
}

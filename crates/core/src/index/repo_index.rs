//! 持久全仓定义索引（opt-in）：`reviewgate index build` 预扫整库、tree-sitter 提取所有
//! 符号定义，持久化为 `.reviewgate/cache/symbols.json`。
//!
//! 收益：`find_definition` 从"每次 git grep + 解析候选文件"变成 **O(1) 全仓完整查表**——
//! Agent 追跨文件定义时更快更全（不再受候选文件截断影响）。纯本地、无外部依赖、无嵌入；
//! 建了就用、没建则回退现有按需 [`super::TreeSitterIndex`]（优雅降级，绝不硬依赖）。
//!
//! 边界：只索引**定义**。`find_callers` / `find_references` 仍走按需后端（调用点扫描本就要读正文）。

use super::{list_definitions, CodeIndex, Lang, SymbolLoc, TreeSitterIndex};
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 符号名 → 定义位置（全仓、完整）。
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct RepoIndex {
    /// 符号名 → 定义位置列表。
    defs: HashMap<String, Vec<SymbolLoc>>,
    /// 建库时的 `HEAD` sha（陈旧提示用）。旧版索引/无 git 时为 None。
    #[serde(default)]
    built_at_head: Option<String>,
}

fn index_path(repo_root: &Path) -> PathBuf {
    repo_root
        .join(".reviewgate")
        .join("cache")
        .join("symbols.json")
}

impl RepoIndex {
    /// 从 (路径, 源码) 列表建库（纯函数，便于测试）。不支持的语言文件自动跳过。
    pub fn build_from_files<S: AsRef<str>>(files: &[(S, S)]) -> Self {
        let mut defs: HashMap<String, Vec<SymbolLoc>> = HashMap::new();
        for (path, source) in files {
            for (name, loc) in list_definitions(path.as_ref(), source.as_ref()) {
                defs.entry(name).or_default().push(loc);
            }
        }
        RepoIndex {
            defs,
            built_at_head: None,
        }
    }

    /// 索引到的**不同符号名**数。
    pub fn symbol_count(&self) -> usize {
        self.defs.len()
    }

    /// 索引到的**定义总数**（同名多定义分别计）。
    pub fn definition_count(&self) -> usize {
        self.defs.values().map(Vec::len).sum()
    }

    /// 查符号定义（全仓、完整）。未命中返回空切片。
    pub fn definitions(&self, symbol: &str) -> &[SymbolLoc] {
        self.defs.get(symbol).map(Vec::as_slice).unwrap_or(&[])
    }

    /// 建库时的 `HEAD` sha（用于陈旧检测）。
    pub fn built_at_head(&self) -> Option<&str> {
        self.built_at_head.as_deref()
    }

    /// 持久化到 `.reviewgate/cache/symbols.json`（自动建目录并写自忽略 `.gitignore`）。
    pub fn save(&self, repo_root: &Path) -> std::io::Result<()> {
        let path = index_path(repo_root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
            let gi = parent.join(".gitignore");
            if !gi.exists() {
                std::fs::write(gi, "*\n")?;
            }
        }
        std::fs::write(path, serde_json::to_string(self)?)
    }

    /// 从仓库根加载。文件缺失/损坏 → `None`（未建索引，调用方回退按需检索）。
    pub fn load(repo_root: &Path) -> Option<Self> {
        let s = std::fs::read_to_string(index_path(repo_root)).ok()?;
        serde_json::from_str(&s).ok()
    }

    /// 扫描整库（`git ls-files` 的跟踪文件）建库。仅读受支持语言的文件；读失败的单个文件跳过。
    pub async fn build(repo_root: &Path) -> Result<Self> {
        let listing = crate::diff::git::git(&["ls-files"]).await?;
        let mut pairs: Vec<(String, String)> = Vec::new();
        for path in listing.lines() {
            if Lang::from_path(path) == Lang::Other {
                continue; // 无 tree-sitter grammar 的语言不建定义索引
            }
            if let Ok(src) = std::fs::read_to_string(repo_root.join(path)) {
                pairs.push((path.to_string(), src));
            }
        }
        let mut idx = Self::build_from_files(&pairs);
        idx.built_at_head = crate::diff::git::git(&["rev-parse", "HEAD"])
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        Ok(idx)
    }
}

/// 校验索引里的定义位置是否仍成立：读该文件该行，trim 后与建库时的 `snippet` 一致才算有效。
/// 文件读不到 / 行不存在 / 内容变了（定义被移动或删除）→ 无效（陈旧），调用方按 miss 回退按需。
/// 这把"移动/删除的定义命中旧位置"变成安全的 miss，陈旧索引不会给出过时位置。
fn location_still_valid(repo_root: &Path, loc: &SymbolLoc) -> bool {
    let Ok(src) = std::fs::read_to_string(repo_root.join(&loc.path)) else {
        return false;
    };
    src.lines()
        .nth((loc.line as usize).saturating_sub(1))
        .map(|line| line.trim() == loc.snippet)
        .unwrap_or(false)
}

/// 把持久索引接成 [`CodeIndex`]：`find_definition` 命中查表（并校验位置仍成立）、
/// 未命中或全部陈旧则回退按需；`find_callers` / `find_references` 直接走按需后端。
pub struct PersistentIndex {
    repo: RepoIndex,
    repo_root: PathBuf,
    fallback: TreeSitterIndex,
}

impl PersistentIndex {
    pub fn new(repo: RepoIndex, repo_root: impl Into<PathBuf>) -> Self {
        PersistentIndex {
            repo,
            repo_root: repo_root.into(),
            fallback: TreeSitterIndex::new(),
        }
    }
}

#[async_trait]
impl CodeIndex for PersistentIndex {
    async fn find_definition(&self, symbol: &str, lang: Option<Lang>) -> Result<Vec<SymbolLoc>> {
        // 命中项逐条校验位置仍成立，滤掉陈旧的（定义已移动/删除）。
        let valid: Vec<SymbolLoc> = self
            .repo
            .definitions(symbol)
            .iter()
            .filter(|loc| location_still_valid(&self.repo_root, loc))
            .cloned()
            .collect();
        if !valid.is_empty() {
            return Ok(valid);
        }
        // 未命中或全部陈旧 → 回退按需，绝不因索引空/旧而漏。
        self.fallback.find_definition(symbol, lang).await
    }
    async fn find_callers(&self, symbol: &str, lang: Option<Lang>) -> Result<Vec<SymbolLoc>> {
        self.fallback.find_callers(symbol, lang).await
    }
    async fn find_references(&self, symbol: &str, lang: Option<Lang>) -> Result<Vec<SymbolLoc>> {
        self.fallback.find_references(symbol, lang).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::SymbolKind;

    const RUST_A: &str = "fn login(id: u32) {}\nstruct User {}\n";
    const RUST_B: &str = "fn login(x: u8) {}\nfn logout() {}\n";

    #[test]
    fn build_indexes_all_defs_across_files() {
        let idx = RepoIndex::build_from_files(&[("a.rs", RUST_A), ("b.rs", RUST_B)]);
        // login 定义在两个文件 → 2 处。
        let login = idx.definitions("login");
        assert_eq!(login.len(), 2, "login 应有两处定义");
        assert!(login.iter().all(|l| l.kind == SymbolKind::Function));
        // User / logout 各一处。
        assert_eq!(idx.definitions("User").len(), 1);
        assert_eq!(idx.definitions("logout").len(), 1);
        // 未知符号 → 空。
        assert!(idx.definitions("nope").is_empty());
        assert_eq!(idx.symbol_count(), 3); // login, User, logout
        assert_eq!(idx.definition_count(), 4); // login×2 + User + logout
    }

    #[tokio::test]
    async fn persistent_index_serves_valid_definition_from_map() {
        // 文件仍与建库时一致 → 命中查表返回，位置校验通过。
        let dir = std::env::temp_dir().join(format!("rg-pi-valid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), RUST_A).unwrap();

        let idx = RepoIndex::build_from_files(&[("a.rs", RUST_A)]);
        let pi = PersistentIndex::new(idx, &dir);
        let defs = pi.find_definition("User", None).await.unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].kind, SymbolKind::Type);
        assert_eq!(defs[0].path, "a.rs");
    }

    #[test]
    fn location_valid_only_when_line_matches() {
        let dir = std::env::temp_dir().join(format!("rg-loc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.rs"), "fn login() {}\n").unwrap();

        let good = SymbolLoc {
            path: "a.rs".into(),
            line: 1,
            col: 1,
            kind: SymbolKind::Function,
            snippet: "fn login() {}".into(),
        };
        assert!(location_still_valid(&dir, &good), "行内容一致应有效");

        // 定义被移动/改写 → 该行内容不再匹配。
        let mut moved = good.clone();
        moved.snippet = "fn logout() {}".into();
        assert!(!location_still_valid(&dir, &moved), "内容变了应无效");

        // 行超出文件 / 文件不存在 → 无效。
        let mut oob = good.clone();
        oob.line = 999;
        assert!(!location_still_valid(&dir, &oob));
        let mut nofile = good.clone();
        nofile.path = "gone.rs".into();
        assert!(!location_still_valid(&dir, &nofile));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unsupported_files_skipped() {
        let idx = RepoIndex::build_from_files(&[("readme.txt", "not code")]);
        assert_eq!(idx.symbol_count(), 0);
    }

    #[test]
    fn save_load_roundtrip_and_missing_is_none() {
        let dir = std::env::temp_dir().join(format!("rg-repoidx-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        assert!(RepoIndex::load(&dir).is_none(), "未建索引 → None");

        let idx = RepoIndex::build_from_files(&[("a.rs", RUST_A)]);
        idx.save(&dir).unwrap();
        let loaded = RepoIndex::load(&dir).expect("应能加载");
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(loaded.definitions("login").len(), 1);
        assert_eq!(loaded.definitions("User")[0].kind, SymbolKind::Type);
    }
}

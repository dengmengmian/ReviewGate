//! 审查范围排除：把不该花 token 审的文件挡在 LLM 之前。
//!
//! 三个来源，优先级由低到高（后者可用 `!` 反选覆盖前者）：
//! 1. 内置默认（lock 文件 / vendored 依赖 / 生成代码 / 压缩产物），可整体关闭；
//! 2. 仓库根的 `.reviewgateignore`（gitignore 语法）；
//! 3. 配置里的 `[exclude] patterns`。
//!
//! 二进制文件永远排除（LLM 读不了）。
//!
//! **排除即公开**：被排除的文件会带原因回传给调用方并出现在报告里。闸口不允许
//! 悄悄少审文件——那正是"假通过"。

use crate::diff::Diff;
use anyhow::{Context, Result};
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use serde::Serialize;
use std::path::Path;

/// `.reviewgateignore` 文件名（放在仓库根）。
pub const IGNORE_FILE: &str = ".reviewgateignore";

/// 内置默认排除项（gitignore 语法）。只收"提交进仓库但审了没意义"的东西：
/// 依赖锁、vendored 源码、protobuf/ORM 生成物、压缩打包产物。
/// 刻意保守——漏审真实代码的代价远大于多审几个文件。
pub const BUILTIN_PATTERNS: &[&str] = &[
    // 依赖锁文件
    "Cargo.lock",
    "package-lock.json",
    "pnpm-lock.yaml",
    "yarn.lock",
    "bun.lockb",
    "poetry.lock",
    "Pipfile.lock",
    "uv.lock",
    "Gemfile.lock",
    "composer.lock",
    "go.sum",
    "flake.lock",
    // vendored 依赖
    "vendor/",
    "node_modules/",
    "third_party/",
    // 生成代码
    "*.pb.go",
    "*.pb.cc",
    "*.pb.h",
    "*_pb2.py",
    "*_pb2_grpc.py",
    "*.g.dart",
    "*.freezed.dart",
    "*_generated.go",
    // 压缩 / 打包产物
    "*.min.js",
    "*.min.css",
    "*.js.map",
    "*.css.map",
];

/// 一个文件被排除的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExcludeReason {
    /// 二进制文件，LLM 无法审查。
    Binary,
    /// 命中内置默认列表。
    Builtin,
    /// 命中仓库根的 `.reviewgateignore`。
    IgnoreFile,
    /// 命中配置里的 `[exclude] patterns`。
    Config,
}

impl ExcludeReason {
    pub fn as_str(self) -> &'static str {
        match self {
            ExcludeReason::Binary => "binary",
            ExcludeReason::Builtin => "builtin",
            ExcludeReason::IgnoreFile => IGNORE_FILE,
            ExcludeReason::Config => "config",
        }
    }
}

/// 一条被排除的记录（进报告，供用户核对是否误排）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExcludedFile {
    pub path: String,
    pub reason: ExcludeReason,
}

/// 路径排除匹配器。
#[derive(Debug, Default)]
pub struct Excluder {
    builtin: Option<Gitignore>,
    ignore_file: Option<Gitignore>,
    config: Option<Gitignore>,
}

impl Excluder {
    /// 构造匹配器。
    ///
    /// - `patterns`：配置里的 `[exclude] patterns`（gitignore 语法，相对仓库根）。
    /// - `builtin`：是否启用 [`BUILTIN_PATTERNS`]。
    /// - `repo_root`：仓库根，用于读取 `.reviewgateignore`。传 `None` 则不读。
    ///
    /// 注意：gitignore 语法几乎不存在"非法模式"（`ignore` crate 对未闭合字符类等一律
    /// 按字面量处理），所以写错的 pattern 表现为**不匹配**而非报错。被排除的文件清单
    /// 会打进报告，用来核对是否与预期一致。
    pub fn new(patterns: &[String], builtin: bool, repo_root: Option<&Path>) -> Result<Self> {
        let builtin = if builtin {
            Some(build_matcher(
                BUILTIN_PATTERNS.iter().map(|s| s.to_string()),
                "builtin exclude patterns",
            )?)
        } else {
            None
        };

        let ignore_file = match repo_root {
            Some(root) => {
                let path = root.join(IGNORE_FILE);
                match std::fs::read_to_string(&path) {
                    Ok(text) => Some(build_matcher(
                        text.lines().map(|l| l.to_string()),
                        &path.display().to_string(),
                    )?),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                    Err(e) => {
                        return Err(e).with_context(|| format!("failed to read {}", path.display()))
                    }
                }
            }
            None => None,
        };

        let config = if patterns.is_empty() {
            None
        } else {
            Some(build_matcher(
                patterns.iter().cloned(),
                "[exclude] patterns",
            )?)
        };

        Ok(Self {
            builtin,
            ignore_file,
            config,
        })
    }

    /// 判断一个路径是否被排除，返回原因。`binary` 为该文件是否是二进制 diff。
    ///
    /// 高优先级来源的 `!` 反选可以救回低优先级排掉的文件；二进制不可救。
    pub fn reason(&self, path: &str, binary: bool) -> Option<ExcludeReason> {
        if binary {
            return Some(ExcludeReason::Binary);
        }
        let mut hit: Option<ExcludeReason> = None;
        for (matcher, reason) in [
            (&self.builtin, ExcludeReason::Builtin),
            (&self.ignore_file, ExcludeReason::IgnoreFile),
            (&self.config, ExcludeReason::Config),
        ] {
            let Some(m) = matcher else { continue };
            match m.matched_path_or_any_parents(path, false) {
                ignore::Match::Ignore(_) => hit = Some(reason),
                ignore::Match::Whitelist(_) => hit = None,
                ignore::Match::None => {}
            }
        }
        hit
    }

    /// 就地从 diff 里摘掉被排除的文件，返回被摘掉的清单（保持原顺序）。
    pub fn apply(&self, diff: &mut Diff) -> Vec<ExcludedFile> {
        let mut removed = Vec::new();
        diff.files
            .retain(|f| match self.reason(f.path(), f.binary) {
                Some(reason) => {
                    removed.push(ExcludedFile {
                        path: f.path().to_string(),
                        reason,
                    });
                    false
                }
                None => true,
            });
        removed
    }
}

fn build_matcher(lines: impl Iterator<Item = String>, source: &str) -> Result<Gitignore> {
    // root 用空串：diff 里的路径本就相对仓库根，不做前缀剥离。
    let mut b = GitignoreBuilder::new("");
    for line in lines {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        b.add_line(None, line)
            .with_context(|| format!("invalid exclude pattern `{line}` in {source}"))?;
    }
    b.build()
        .with_context(|| format!("failed to build exclude matcher from {source}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileDiff, FileStatus};

    fn fd(path: &str, binary: bool) -> FileDiff {
        FileDiff {
            old_path: None,
            new_path: Some(path.to_string()),
            status: FileStatus::Added,
            binary,
            hunks: Vec::new(),
        }
    }

    #[test]
    fn builtin_excludes_lock_vendor_and_generated() {
        let ex = Excluder::new(&[], true, None).unwrap();
        assert_eq!(ex.reason("Cargo.lock", false), Some(ExcludeReason::Builtin));
        assert_eq!(ex.reason("go.sum", false), Some(ExcludeReason::Builtin));
        assert_eq!(
            ex.reason("web/node_modules/left-pad/index.js", false),
            Some(ExcludeReason::Builtin)
        );
        assert_eq!(
            ex.reason("api/service.pb.go", false),
            Some(ExcludeReason::Builtin)
        );
        assert_eq!(
            ex.reason("static/app.min.js", false),
            Some(ExcludeReason::Builtin)
        );
        // 真实源码不能被误排。
        assert_eq!(ex.reason("crates/core/src/lib.rs", false), None);
        assert_eq!(ex.reason("src/vendored_note.rs", false), None);
    }

    #[test]
    fn builtin_can_be_disabled() {
        let ex = Excluder::new(&[], false, None).unwrap();
        assert_eq!(ex.reason("Cargo.lock", false), None);
    }

    #[test]
    fn binary_is_always_excluded() {
        let ex = Excluder::new(&[], false, None).unwrap();
        assert_eq!(
            ex.reason("assets/logo.png", true),
            Some(ExcludeReason::Binary)
        );
    }

    #[test]
    fn config_patterns_apply_and_can_whitelist_builtin() {
        let pats = vec!["docs/**".to_string(), "!Cargo.lock".to_string()];
        let ex = Excluder::new(&pats, true, None).unwrap();
        assert_eq!(
            ex.reason("docs/guide.md", false),
            Some(ExcludeReason::Config)
        );
        // 配置里的 `!` 能救回内置排掉的文件。
        assert_eq!(ex.reason("Cargo.lock", false), None);
    }

    #[test]
    fn unmatched_bracket_is_literal_not_an_error() {
        // gitignore 语义下未闭合字符类按字面量处理；记录这一行为，避免误以为会报错。
        let pats = vec!["a[.rs".to_string()];
        let ex = Excluder::new(&pats, false, None).unwrap();
        assert_eq!(ex.reason("a[.rs", false), Some(ExcludeReason::Config));
        assert_eq!(ex.reason("ax.rs", false), None);
    }

    #[test]
    fn ignore_file_is_read_from_repo_root() {
        let dir = std::env::temp_dir().join(format!("rg_excl_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(IGNORE_FILE), "# comment\ntestdata/\n").unwrap();

        let ex = Excluder::new(&[], false, Some(&dir)).unwrap();
        assert_eq!(
            ex.reason("testdata/big.json", false),
            Some(ExcludeReason::IgnoreFile)
        );
        assert_eq!(ex.reason("src/main.rs", false), None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_ignore_file_is_not_an_error() {
        let dir = std::env::temp_dir().join(format!("rg_excl_none_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(Excluder::new(&[], false, Some(&dir)).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_removes_files_and_reports_them() {
        let ex = Excluder::new(&[], true, None).unwrap();
        let mut diff = Diff {
            files: vec![
                fd("src/main.rs", false),
                fd("Cargo.lock", false),
                fd("assets/logo.png", true),
            ],
        };
        let removed = ex.apply(&mut diff);
        assert_eq!(diff.files.len(), 1);
        assert_eq!(diff.files[0].path(), "src/main.rs");
        assert_eq!(
            removed,
            vec![
                ExcludedFile {
                    path: "Cargo.lock".into(),
                    reason: ExcludeReason::Builtin
                },
                ExcludedFile {
                    path: "assets/logo.png".into(),
                    reason: ExcludeReason::Binary
                },
            ]
        );
    }
}

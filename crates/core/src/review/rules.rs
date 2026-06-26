//! 项目规则注入。
//!
//! 把配置里的业务规则与 `rules_dir/*.md` 组装成一段 markdown，注入到**共享 prompt 块**
//! （所有维度可见、可被缓存）。`rules_dir` 里以语言命名的文件（`rust.md`/`typescript.md`…）
//! 只在该语言本次被改动时注入，避免给所有语言塞一坨无关规则。

use crate::config::BusinessConfig;
use crate::diff::Diff;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Debug, Clone, Default)]
pub struct RulesSection {
    pub body: String,
    pub warnings: Vec<String>,
}

/// 扩展名 → 语言名（用于 `rules_dir` 里 `<lang>.md` 的按需注入）。
fn ext_to_lang(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "py" | "pyi" => "python",
        "go" => "go",
        "java" => "java",
        "rb" | "rake" => "ruby",
        "c" | "h" => "c",
        "cc" | "cpp" | "hpp" | "cxx" | "hh" => "cpp",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "php" => "php",
        "cs" => "csharp",
        "scala" | "sc" => "scala",
        "dart" => "dart",
        "m" | "mm" => "objc",
        "lua" => "lua",
        "pl" | "pm" => "perl",
        "hs" => "haskell",
        "ex" | "exs" => "elixir",
        "erl" | "hrl" => "erlang",
        "clj" | "cljs" | "cljc" => "clojure",
        "groovy" | "gradle" => "groovy",
        "jl" => "julia",
        "r" => "r",
        "ml" | "mli" => "ocaml",
        "fs" | "fsx" | "fsi" => "fsharp",
        "zig" => "zig",
        "nim" | "nims" => "nim",
        "cr" => "crystal",
        "cj" => "cangjie",
        "sh" | "bash" | "zsh" => "shell",
        "ps1" | "psm1" => "powershell",
        "html" | "htm" => "html",
        "css" | "scss" | "sass" | "less" => "css",
        "vue" => "vue",
        "svelte" => "svelte",
        "sql" => "sql",
        "graphql" | "gql" => "graphql",
        "sol" => "solidity",
        "f" | "f90" | "f95" | "f03" | "for" => "fortran",
        "cob" | "cbl" => "cobol",
        "pas" | "pp" => "pascal",
        "dockerfile" => "dockerfile",
        "tf" | "tfvars" => "terraform",
        _ => return None,
    })
}

/// 全部已识别的语言名（45 种：用于 `rules_dir`/内置规则文件 stem 校验）。
const KNOWN_LANGUAGES: &[&str] = &[
    "rust",
    "typescript",
    "javascript",
    "python",
    "go",
    "java",
    "ruby",
    "c",
    "cpp",
    "kotlin",
    "swift",
    "php",
    "csharp",
    "scala",
    "dart",
    "objc",
    "lua",
    "perl",
    "haskell",
    "elixir",
    "erlang",
    "clojure",
    "groovy",
    "julia",
    "r",
    "ocaml",
    "fsharp",
    "zig",
    "nim",
    "crystal",
    "cangjie",
    "shell",
    "powershell",
    "html",
    "css",
    "vue",
    "svelte",
    "sql",
    "graphql",
    "solidity",
    "fortran",
    "cobol",
    "pascal",
    "dockerfile",
    "terraform",
];

/// 文件名 stem 是否是一个我们识别的语言名。
fn is_language_name(s: &str) -> bool {
    KNOWN_LANGUAGES.contains(&s)
}

/// 内置语言起步规则（编进二进制；源为 `crates/core/prompts/languages/<lang>.md`，单一来源）。
/// **只默认带已逐一验证的核心语言**；长尾语言（ruby/kotlin/swift/php/csharp）的规则文件同在该目录，
/// 但默认不注入——作为 opt-in 模板（install 脚本拷给用户），待验证后再加入此处默认开。
fn builtin_language_rule(lang: &str) -> Option<&'static str> {
    Some(match lang {
        "python" => include_str!("../../prompts/languages/python.md"),
        "go" => include_str!("../../prompts/languages/go.md"),
        "javascript" => include_str!("../../prompts/languages/javascript.md"),
        "typescript" => include_str!("../../prompts/languages/typescript.md"),
        "rust" => include_str!("../../prompts/languages/rust.md"),
        "java" => include_str!("../../prompts/languages/java.md"),
        "c" => include_str!("../../prompts/languages/c.md"),
        "cpp" => include_str!("../../prompts/languages/cpp.md"),
        // 第二批验证通过（clean 不误 BLOCK）：
        "csharp" => include_str!("../../prompts/languages/csharp.md"),
        "php" => include_str!("../../prompts/languages/php.md"),
        "swift" => include_str!("../../prompts/languages/swift.md"),
        "kotlin" => include_str!("../../prompts/languages/kotlin.md"),
        // 第三批验证通过：
        "scala" => include_str!("../../prompts/languages/scala.md"),
        "dart" => include_str!("../../prompts/languages/dart.md"),
        "objc" => include_str!("../../prompts/languages/objc.md"),
        "lua" => include_str!("../../prompts/languages/lua.md"),
        "perl" => include_str!("../../prompts/languages/perl.md"),
        "haskell" => include_str!("../../prompts/languages/haskell.md"),
        "elixir" => include_str!("../../prompts/languages/elixir.md"),
        // 第四批验证通过：
        "clojure" => include_str!("../../prompts/languages/clojure.md"),
        "groovy" => include_str!("../../prompts/languages/groovy.md"),
        "julia" => include_str!("../../prompts/languages/julia.md"),
        "ocaml" => include_str!("../../prompts/languages/ocaml.md"),
        "fsharp" => include_str!("../../prompts/languages/fsharp.md"),
        "zig" => include_str!("../../prompts/languages/zig.md"),
        "nim" => include_str!("../../prompts/languages/nim.md"),
        "erlang" => include_str!("../../prompts/languages/erlang.md"),
        // 第五批验证通过（含 r/ruby trivial 重测）：
        "ruby" => include_str!("../../prompts/languages/ruby.md"),
        "r" => include_str!("../../prompts/languages/r.md"),
        "crystal" => include_str!("../../prompts/languages/crystal.md"),
        "cangjie" => include_str!("../../prompts/languages/cangjie.md"),
        "html" => include_str!("../../prompts/languages/html.md"),
        "css" => include_str!("../../prompts/languages/css.md"),
        "svelte" => include_str!("../../prompts/languages/svelte.md"),
        "sql" => include_str!("../../prompts/languages/sql.md"),
        "solidity" => include_str!("../../prompts/languages/solidity.md"),
        "fortran" => include_str!("../../prompts/languages/fortran.md"),
        "cobol" => include_str!("../../prompts/languages/cobol.md"),
        "pascal" => include_str!("../../prompts/languages/pascal.md"),
        "dockerfile" => include_str!("../../prompts/languages/dockerfile.md"),
        "terraform" => include_str!("../../prompts/languages/terraform.md"),
        // 收尾批验证通过（45/45 全部默认开）：
        "shell" => include_str!("../../prompts/languages/shell.md"),
        "powershell" => include_str!("../../prompts/languages/powershell.md"),
        "vue" => include_str!("../../prompts/languages/vue.md"),
        "graphql" => include_str!("../../prompts/languages/graphql.md"),
        _ => return None,
    })
}

/// 本次改动涉及的语言集合。
fn changed_languages(diff: &Diff) -> BTreeSet<&'static str> {
    diff.files
        .iter()
        .filter_map(|f| f.new_path.as_deref().or(f.old_path.as_deref()))
        .filter_map(|p| Path::new(p).extension().and_then(|e| e.to_str()))
        .filter_map(ext_to_lang)
        .collect()
}

/// 构造注入到共享 prompt 块的"项目规则"段；无规则时返回空串。
pub fn build_rules_section(business: &BusinessConfig, diff: &Diff, repo_root: &Path) -> String {
    build_rules_section_with_warnings(business, diff, repo_root).body
}

/// 同 [`build_rules_section`]，额外返回配置了但不可读取的规则来源，供 CLI/JSON 告警。
pub fn build_rules_section_with_warnings(
    business: &BusinessConfig,
    diff: &Diff,
    repo_root: &Path,
) -> RulesSection {
    let mut out = String::new();
    let mut warnings = Vec::new();

    if !business.rules.is_empty() {
        out.push_str(
            "## Project business rules\n\nAlso check the following project rules. **Report only when the change clearly violates a rule or when a new/modified path bypasses a rule**. \
            When a rule is hit, prefix the finding message with its id, for example `[B2] ...`, so it can be traced:\n",
        );
        // 编号 B1/B2…，供 finding 引用（轻量可追溯，无需结构化字段）。
        let mut n = 0;
        for r in &business.rules {
            let r = r.trim();
            if !r.is_empty() {
                n += 1;
                out.push_str(&format!("- [B{n}] {r}\n"));
            }
        }
        out.push('\n');
    }

    // 内置语言起步规则：按本次改动语言自动注入（默认开，已验证的核心语言）。
    if business.builtin_language_rules {
        for lang in changed_languages(diff) {
            if let Some(body) = builtin_language_rule(lang) {
                let body = body.trim();
                if !body.is_empty() {
                    out.push_str(&format!("## Built-in language rules: {lang}\n\n{body}\n\n"));
                }
            }
        }
    }

    if let Some(dir) = &business.rules_dir {
        let path = repo_root.join(dir);
        match render_rules_dir(&path, diff) {
            Some(s) => out.push_str(&s),
            None => warnings.push(format!(
                "rules_dir does not exist or is not readable: {dir}"
            )),
        }
    }

    if let Some(dir) = &business.skills_dir {
        let path = repo_root.join(dir);
        match render_skills_dir(&path) {
            Some(s) => out.push_str(&s),
            None => warnings.push(format!(
                "skills_dir does not exist or is not readable: {dir}"
            )),
        }
    }

    RulesSection {
        body: out,
        warnings,
    }
}

/// 读取 skill 目录里组织自定义的 review 规则 skill（SKILL.md 格式），剥离 frontmatter 后注入。
/// 支持 `<子目录>/SKILL.md`（Claude Code skill 布局）与扁平 `*.md` 两种。
fn render_skills_dir(dir: &Path) -> Option<String> {
    let mut files: Vec<std::path::PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                if p.is_dir() {
                    let s = p.join("SKILL.md");
                    s.is_file().then_some(s)
                } else if p.extension().and_then(|x| x.to_str()) == Some("md") {
                    Some(p)
                } else {
                    None
                }
            })
            .collect(),
        Err(_) => return None,
    };
    files.sort();

    let mut out = String::new();
    for path in files {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let (name, desc, body) = parse_skill(&content);
        let body = body.trim();
        if body.is_empty() {
            continue;
        }
        let title = name.unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("skill")
                .to_string()
        });
        out.push_str(&format!("## Rule skill: {title}\n\n"));
        if let Some(d) = desc {
            if !d.trim().is_empty() {
                out.push_str(&format!("{}\n\n", d.trim()));
            }
        }
        out.push_str(&format!("{body}\n\n"));
    }
    Some(out)
}

/// 解析 skill 文件：剥离 YAML frontmatter，返回 `(name, description, body)`。
/// 无 frontmatter 时返回 `(None, None, 全文)`。
fn parse_skill(content: &str) -> (Option<String>, Option<String>, &str) {
    let trimmed = content.trim_start_matches('\u{feff}'); // 去 BOM
    let Some(rest) = trimmed.strip_prefix("---") else {
        return (None, None, content);
    };
    let Some(end) = rest.find("\n---") else {
        return (None, None, content);
    };
    let front = &rest[..end];
    let body = rest[end + 4..].trim_start_matches(['\n', '\r']);
    let mut name = None;
    let mut desc = None;
    for line in front.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("name:") {
            name = Some(v.trim().trim_matches('"').to_string());
        } else if let Some(v) = line.strip_prefix("description:") {
            desc = Some(v.trim().trim_matches('"').to_string());
        }
    }
    (name, desc, body)
}

/// 读取 `rules_dir` 下的 `*.md`，按语言路由后拼接。
fn render_rules_dir(dir: &Path, diff: &Diff) -> Option<String> {
    let langs = changed_languages(diff);
    let mut entries: Vec<_> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
            .collect(),
        Err(_) => return None,
    };
    entries.sort_by_key(|e| e.file_name());

    let mut out = String::new();
    for e in entries {
        let path = e.path();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        // 语言命名的规则：仅在该语言本次被改动时注入。
        if is_language_name(&stem) && !langs.contains(stem.as_str()) {
            continue;
        }
        if let Ok(body) = std::fs::read_to_string(&path) {
            let body = body.trim();
            if !body.is_empty() {
                out.push_str(&format!("## Rule: {stem}\n\n{body}\n\n"));
            }
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{Diff, FileDiff, FileStatus};

    fn diff_with(paths: &[&str]) -> Diff {
        Diff {
            files: paths
                .iter()
                .map(|p| FileDiff {
                    old_path: None,
                    new_path: Some(p.to_string()),
                    status: FileStatus::Modified,
                    binary: false,
                    hunks: vec![],
                })
                .collect(),
        }
    }

    #[test]
    fn inline_rules_rendered() {
        let b = BusinessConfig {
            rules: vec!["金额用整数分".into(), "  ".into(), "校验 owner_id".into()],
            rules_dir: None,
            skills_dir: None,
            builtin_language_rules: false,
        };
        let s = build_rules_section(&b, &diff_with(&["a.rs"]), Path::new("/tmp"));
        assert!(s.contains("## Project business rules"));
        // 规则被编号 B1/B2，空白项跳过且不占编号。
        assert!(s.contains("- [B1] 金额用整数分"));
        assert!(s.contains("- [B2] 校验 owner_id"));
        assert!(!s.contains("B3"));
    }

    #[test]
    fn empty_config_yields_empty() {
        // 无业务规则 + 改的是非内置语言文件 → 空段。
        let b = BusinessConfig::default();
        assert!(build_rules_section(&b, &diff_with(&["notes.txt"]), Path::new("/tmp")).is_empty());
    }

    #[test]
    fn builtin_language_rules_injected_by_default() {
        // 默认配置（builtin on）+ 改了 Python 文件 → 注入内置 Python 起步规则。
        let b = BusinessConfig::default();
        let s = build_rules_section(&b, &diff_with(&["app.py"]), Path::new("/tmp"));
        assert!(
            s.contains("## Built-in language rules: python"),
            "should inject built-in Python rules: {s}"
        );
        // 非编程语言文件（无对应规则）不注入 → 空。
        let s_json = build_rules_section(&b, &diff_with(&["config.json"]), Path::new("/tmp"));
        assert!(
            s_json.is_empty(),
            "non-language file should inject nothing: {s_json}"
        );
        // 显式关闭 → 即使改 Python 也不注入。
        let off = BusinessConfig {
            builtin_language_rules: false,
            ..BusinessConfig::default()
        };
        assert!(build_rules_section(&off, &diff_with(&["app.py"]), Path::new("/tmp")).is_empty());
    }

    #[test]
    fn changed_languages_maps_extensions() {
        let langs = changed_languages(&diff_with(&["src/a.rs", "web/b.tsx", "x.md"]));
        assert!(langs.contains("rust"));
        assert!(langs.contains("typescript"));
        assert!(!langs.contains("go"));
    }

    #[test]
    fn language_rules_routed_by_changed_files() {
        let dir = std::env::temp_dir().join(format!("rg_rules_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rust.md"), "Rust 专项规则").unwrap();
        std::fs::write(dir.join("go.md"), "Go 专项规则").unwrap();
        std::fs::write(dir.join("business.md"), "通用业务规则").unwrap();

        // 只改了 Rust：rust.md + business.md 注入，go.md 不注入。
        let out = render_rules_dir(&dir, &diff_with(&["src/a.rs"])).unwrap();
        assert!(out.contains("Rust 专项规则"));
        assert!(out.contains("通用业务规则"));
        assert!(!out.contains("Go 专项规则"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn parse_skill_strips_frontmatter() {
        let content = "---\nname: pr-conventions\ndescription: 团队 PR 约定\n---\n\n## 规则\n禁止裸 unwrap。\n";
        let (name, desc, body) = parse_skill(content);
        assert_eq!(name.as_deref(), Some("pr-conventions"));
        assert_eq!(desc.as_deref(), Some("团队 PR 约定"));
        assert!(body.starts_with("## 规则"));
        assert!(!body.contains("---"));
        // 无 frontmatter：原样返回。
        let (n2, _, b2) = parse_skill("没有 frontmatter 的正文");
        assert!(n2.is_none());
        assert_eq!(b2, "没有 frontmatter 的正文");
    }

    #[test]
    fn skills_dir_injects_skill_bodies() {
        let dir = std::env::temp_dir().join(format!("rg_skills_test_{}", std::process::id()));
        // 子目录 SKILL.md 布局 + 扁平 .md 各一。
        std::fs::create_dir_all(dir.join("naming")).unwrap();
        std::fs::write(
            dir.join("naming/SKILL.md"),
            "---\nname: 命名规范\ndescription: d\n---\n变量用 snake_case。\n",
        )
        .unwrap();
        std::fs::write(dir.join("flat.md"), "扁平规则正文。").unwrap();

        let out = render_skills_dir(&dir).unwrap();
        assert!(out.contains("Rule skill: 命名规范"));
        assert!(out.contains("变量用 snake_case"));
        assert!(out.contains("扁平规则正文"));
        assert!(!out.contains("snake_case。\n---")); // frontmatter 已剥离

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_rule_sources_are_reported() {
        let root = std::env::temp_dir().join(format!("rg_missing_rules_{}", std::process::id()));
        let b = BusinessConfig {
            rules: Vec::new(),
            rules_dir: Some("rules".into()),
            skills_dir: Some("skills".into()),
            builtin_language_rules: false,
        };

        let section = build_rules_section_with_warnings(&b, &diff_with(&["src/a.rs"]), &root);

        assert!(section.body.is_empty());
        assert_eq!(section.warnings.len(), 2);
        assert!(section.warnings[0].contains("rules_dir"));
        assert!(section.warnings[1].contains("skills_dir"));
    }

    #[test]
    fn bundled_language_rule_templates_are_routable() {
        let rules_library = Path::new(env!("CARGO_MANIFEST_DIR")).join("prompts/languages");
        let entries: Vec<_> = std::fs::read_dir(&rules_library)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md"))
            .collect();

        assert!(!entries.is_empty());
        let mut language_template_count = 0;
        for entry in entries {
            let path = entry.path();
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap();
            if stem == "README" {
                continue;
            }
            language_template_count += 1;
            assert!(is_language_name(stem), "unroutable rule template: {stem}");
            assert!(
                !std::fs::read_to_string(&path).unwrap().trim().is_empty(),
                "empty rule template: {stem}"
            );
        }
        assert!(language_template_count > 0);
    }
}

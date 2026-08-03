//! 错投仓库识别：判断 Issue 讲的是不是**别的**仓库的事。
//!
//! 设计取向是宁可漏判不可误判——「你提错地方了」如果说错，对反馈者的伤害
//! 远大于漏掉一条错投带来的收益。因此只认显式信号：
//!
//! * 强信号（0.8）：出现指向其他仓库的链接 **且** 有「该去别处提」这类元表述；
//! * 弱信号（0.45）：其他仓库被反复提到，而当前仓库一次都没出现。
//!
//! 弱信号故意压在默认置信度闸门（0.5）之下——只留标签给人筛，不对外发言。
//!
//! **能力边界**：纯规则只能覆盖「反馈者自己提到了别的仓库」这一种情形。
//! 真正常见的错投（内容属于另一个项目，但通篇没提仓库名）需要理解仓库主题，
//! 规则做不到，别指望这个模块能兜住。

use serde::{Deserialize, Serialize};

/// 已知代码托管站点，用于从正文里认出仓库链接。
const FORGE_HOSTS: &[&str] = &[
    "github.com",
    "gitee.com",
    "gitlab.com",
    "atomgit.com",
    "gitcode.com",
    "bitbucket.org",
];

/// 「这事该去别处说」的元表述。只有反馈者自己点破时才算强信号。
const MISROUTE_PHRASES: &[&str] = &[
    "提错",
    "发错",
    "走错",
    "不是这个仓库",
    "不是这个repo",
    "应该在",
    "应该去",
    "转到",
    "移到",
    "属于",
    "wrong repo",
    "wrong repository",
    "wrong project",
    "should be filed",
    "should be reported",
    "belongs in",
    "belongs to",
    "moved to",
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MisroutedHint {
    pub detected: bool,
    pub confidence: f32,
    /// 正文里提到的、不同于当前仓库的仓库（`owner/repo`，已去重）。
    pub target_repos: Vec<String>,
    pub reasons: Vec<String>,
}

/// 检测 Issue 是否讲的是别的仓库。`current_repo` 形如 `owner/repo`。
pub fn detect_misrouted(title: &str, body: &str, current_repo: &str) -> MisroutedHint {
    let text = format!("{title}\n{body}");
    let lower = text.to_lowercase();
    let current = current_repo.trim().to_lowercase();

    let mentions = collect_repo_mentions(&lower);
    let mut foreign: Vec<String> = Vec::new();
    let mut foreign_hits = 0usize;
    let mut self_hits = 0usize;
    for m in &mentions {
        if *m == current {
            self_hits += 1;
        } else {
            foreign_hits += 1;
            if !foreign.contains(m) {
                foreign.push(m.clone());
            }
        }
    }
    if foreign.is_empty() {
        return MisroutedHint::default();
    }
    // 反馈者提到了当前仓库 = 他知道自己在哪，剩下的引用是上下文而非错投。
    if self_hits > 0 || current_repo_named(&lower, &current) {
        return MisroutedHint::default();
    }

    if MISROUTE_PHRASES.iter().any(|p| lower.contains(p)) {
        return MisroutedHint {
            detected: true,
            confidence: 0.8,
            target_repos: foreign,
            reasons: vec!["misroute_phrase_with_foreign_repo".into()],
        };
    }
    // 只被提一次的外部仓库，绝大多数是依赖或参考资料，不算信号。
    if foreign_hits >= 2 {
        return MisroutedHint {
            detected: true,
            confidence: 0.45,
            target_repos: foreign,
            reasons: vec!["repeated_foreign_repo".into()],
        };
    }
    MisroutedHint::default()
}

/// 当前仓库名（含裸 `owner/repo` 与单独的 repo 名）是否在正文里出现过。
fn current_repo_named(lower: &str, current: &str) -> bool {
    if current.is_empty() {
        return false;
    }
    if lower.contains(current) {
        return true;
    }
    match current.split_once('/') {
        Some((_, name)) if name.len() >= 3 => lower.contains(name),
        _ => false,
    }
}

/// 扫出所有 `<forge-host>/<owner>/<repo>` 引用，按出现顺序返回（含重复）。
fn collect_repo_mentions(lower: &str) -> Vec<String> {
    let mut out = Vec::new();
    for host in FORGE_HOSTS {
        let mut from = 0usize;
        while let Some(pos) = lower[from..].find(host) {
            let start = from + pos + host.len();
            from = start;
            let rest = match lower[start..].strip_prefix('/') {
                Some(r) => r,
                None => continue,
            };
            if let Some(slug) = take_owner_repo(rest) {
                out.push(slug);
            }
        }
    }
    out
}

/// 从 `owner/repo/...` 取出 `owner/repo`；取不全就放弃（宁可漏判）。
fn take_owner_repo(rest: &str) -> Option<String> {
    let mut it = rest.split('/');
    let owner = clean_segment(it.next()?)?;
    let repo = clean_segment(it.next()?)?;
    Some(format!("{owner}/{repo}"))
}

fn clean_segment(seg: &str) -> Option<String> {
    let s: String = seg
        .chars()
        .take_while(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    let s = s.trim_matches('.').to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_misroute_is_detected() {
        let h = detect_misrouted(
            "编译报错",
            "这个问题应该在 https://github.com/other/engine 提，我提错仓库了",
            "acme/app",
        );
        assert!(h.detected);
        assert!(h.confidence >= 0.8, "got {}", h.confidence);
        assert_eq!(h.target_repos, vec!["other/engine".to_string()]);
    }

    /// 最大误判源：把「我依赖了某个库」当成「你提错仓库了」。
    #[test]
    fn dependency_mention_is_not_misroute() {
        let h = detect_misrouted(
            "保存时崩溃",
            "我用了 github.com/serde-rs/serde 做序列化，调用 save 时崩溃，版本 5.0.3",
            "acme/app",
        );
        assert!(!h.detected, "a dependency link must not trigger: {h:?}");
    }

    #[test]
    fn link_to_current_repo_is_not_misroute() {
        let h = detect_misrouted(
            "文档链接失效",
            "https://github.com/acme/app/blob/main/README.md 这个链接 404 了，应该在文档里修一下",
            "acme/app",
        );
        assert!(!h.detected, "self-reference must not trigger: {h:?}");
    }

    #[test]
    fn no_repo_reference_is_not_misroute() {
        let h = detect_misrouted("崩溃", "点保存就闪退，应该在启动后就有问题", "acme/app");
        assert!(!h.detected);
    }

    #[test]
    fn repeated_foreign_repo_is_a_weak_signal_only() {
        let h = detect_misrouted(
            "构建失败",
            "github.com/other/engine 构建失败，github.com/other/engine 的 CI 也是红的",
            "acme/app",
        );
        assert!(h.detected);
        assert!(
            h.confidence < 0.5,
            "weak signal must stay under the action gate, got {}",
            h.confidence
        );
        assert_eq!(h.target_repos, vec!["other/engine".to_string()]);
    }

    #[test]
    fn current_repo_mentioned_cancels_weak_signal() {
        let h = detect_misrouted(
            "构建失败",
            "github.com/other/engine 构建失败，acme/app 这边也受影响",
            "acme/app",
        );
        assert!(!h.detected, "current repo is in scope: {h:?}");
    }
}

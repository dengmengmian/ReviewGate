//! Issue 类型分类：启发式（默认/离线）+ 可扩展 LLM。

use super::model::IssueType;
use super::safety::SafetyScores;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Classification {
    pub primary_type: IssueType,
    pub confidence: f32,
    pub reasons: Vec<String>,
}

/// 改进类动作动词。出现在标题开头 = 「请做某事」，是需求而非故障描述。
const ACTION_VERBS: &[&str] = &[
    "add",
    "append",
    "support",
    "allow",
    "introduce",
    "migrate",
    "provide",
    "expose",
    "enable",
    "improve",
    "implement",
    "extend",
    "make",
];

/// 标题前两个词里是否有改进动作动词。
fn leading_action_verb(title: &str) -> bool {
    let head = title
        .trim()
        .trim_start_matches(|c: char| !c.is_alphanumeric());
    head.split_whitespace().take(2).any(|w| {
        let w = w
            .trim_matches(|c: char| !c.is_ascii_alphabetic())
            .to_ascii_lowercase();
        ACTION_VERBS.contains(&w.as_str())
    })
}

/// 标题命中比正文顺带一提强得多：标题是作者自己对问题的概括。
const TITLE_BONUS: f32 = 0.25;

/// `docs:` / `fix:` 这类前缀是作者对类型的显式声明，比散落的关键词更可信。
const PREFIX_BONUS: f32 = 0.3;

/// 会出现在文件名里、但不代表主题的扩展名。`SECURITY.md` 不是安全问题。
const NOISE_EXTS: &[&str] = &[
    "md", "txt", "json", "toml", "yaml", "yml", "lock", "rs", "ts", "tsx", "js", "jsx", "py", "go",
    "java", "sh", "png", "jpg", "jpeg", "gif", "svg", "log", "html", "css",
];

/// 剥掉代码块、行内 code、URL、文件名——这些地方的字面量是标识符，不是内容。
/// 线上踩到三次：文档链接里的 `docs`、清单里的 `SECURITY.md`、
/// 以及一条图片链接把中文 Issue 的回复语言翻成了英文。
pub(crate) fn strip_topic_noise(s: &str) -> String {
    let mut kept = String::with_capacity(s.len());
    let mut in_fence = false;
    for line in s.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    let kept = strip_inline_code(&kept);
    let kept = strip_urls(&kept);
    strip_filenames(&kept)
}

/// 去掉成对反引号之间的内容（奇数个反引号时保留剩余文本，不吞正文）。
fn strip_inline_code(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending = String::new();
    let mut inside = false;
    for c in s.chars() {
        if c == '`' {
            if inside {
                pending.clear();
            } else {
                out.push_str(&pending);
                pending.clear();
            }
            inside = !inside;
            continue;
        }
        if inside {
            pending.push(c);
        } else {
            out.push(c);
        }
    }
    // 未闭合的反引号：后面那段是正文，不能丢
    out.push_str(&pending);
    out
}

fn strip_urls(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = find_scheme(rest) {
        out.push_str(&rest[..pos]);
        let tail = &rest[pos..];
        let end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | ']' | '"' | '\'' | '，' | '。'))
            .unwrap_or(tail.len());
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

fn find_scheme(s: &str) -> Option<usize> {
    let http = s.find("http://");
    let https = s.find("https://");
    match (http, https) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// 删掉 `SECURITY.md` / `image.png` 这类文件名 token，保留周围文本。
fn strip_filenames(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut token = String::new();
    let flush = |out: &mut String, token: &mut String| {
        if !is_noise_filename(token) {
            out.push_str(token);
        }
        token.clear();
    };
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/') {
            token.push(c);
        } else {
            flush(&mut out, &mut token);
            out.push(c);
        }
    }
    flush(&mut out, &mut token);
    out
}

fn is_noise_filename(token: &str) -> bool {
    let last = token.rsplit('/').next().unwrap_or(token);
    match last.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty() && NOISE_EXTS.contains(&ext.to_ascii_lowercase().as_str())
        }
        None => false,
    }
}

/// 作者对类型的显式声明：`docs: xxx` 前缀，或标题开头的 `[Bug]` / `[Feature]` 标签。
/// 标签可能有多个（`[共创大赛]-[Feature] …`），任一命中即算。
fn conventional_prefix(title: &str) -> Option<IssueType> {
    for tag in leading_tags(title) {
        if let Some(t) = tag_to_type(&tag) {
            return Some(t);
        }
    }
    let head = title.split([':', '：']).next()?.trim();
    tag_to_type(head)
}

/// 标题开头连续的 `[...]` 标签内容。
fn leading_tags(title: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut rest = title.trim_start();
    while let Some(inner) = rest.strip_prefix('[') {
        match inner.find(']') {
            Some(end) => {
                tags.push(inner[..end].trim().to_string());
                rest = inner[end + 1..].trim_start_matches([' ', '-', '－', '_']);
            }
            None => break,
        }
    }
    tags
}

fn tag_to_type(tag: &str) -> Option<IssueType> {
    let tag = tag.trim();
    if tag.is_empty() || tag.len() > 14 || !tag.is_ascii() {
        return None;
    }
    match tag.to_ascii_lowercase().as_str() {
        "docs" | "doc" | "documentation" => Some(IssueType::Documentation),
        "fix" | "bug" | "bugfix" => Some(IssueType::Bug),
        "feat" | "feature" | "enhancement" => Some(IssueType::FeatureRequest),
        "perf" | "performance" => Some(IssueType::Performance),
        "security" | "sec" => Some(IssueType::Security),
        _ => None,
    }
}

/// 一组同类关键词的命中强度。命中位置（标题 / 正文）和命中数量都计入，
/// 否则「标题写 docs」和「正文里随口提一句文档」拿到同一个分数。
fn keyword_weight(text: &str, title_lc: &str, base: f32, keys: &[&str]) -> Option<f32> {
    let n = keys.iter().filter(|k| text.contains(**k)).count();
    if n == 0 {
        return None;
    }
    let title_hit = keys.iter().any(|k| title_lc.contains(k));
    let extra = 0.08 * (n - 1).min(2) as f32;
    Some(base + if title_hit { TITLE_BONUS } else { 0.0 } + extra)
}

/// 基于规则的分类器（不依赖网络；LLM 可在外层覆盖）。
pub fn classify_heuristic(title: &str, body: &str, safety: &SafetyScores) -> Classification {
    if safety.advertisement_score >= 0.75 {
        return Classification {
            primary_type: IssueType::Advertisement,
            confidence: safety.advertisement_score,
            reasons: safety.reasons.clone(),
        };
    }
    if safety.spam_score >= 0.75 {
        return Classification {
            primary_type: IssueType::Spam,
            confidence: safety.spam_score,
            reasons: safety.reasons.clone(),
        };
    }
    if safety.abuse_score >= 0.75 {
        return Classification {
            primary_type: IssueType::Abuse,
            confidence: safety.abuse_score,
            reasons: safety.reasons.clone(),
        };
    }

    let title_lc = strip_topic_noise(title).to_lowercase();
    let text = strip_topic_noise(&format!("{title}\n{body}")).to_lowercase();
    let mut scores: Vec<(IssueType, f32, &str)> = Vec::new();

    let bump =
        |scores: &mut Vec<(IssueType, f32, &str)>, t: IssueType, w: f32, why: &'static str| {
            if let Some(e) = scores.iter_mut().find(|x| x.0 == t) {
                e.1 += w;
            } else {
                scores.push((t, w, why));
            }
        };

    let weight = |base: f32, keys: &[&str]| keyword_weight(&text, &title_lc, base, keys);

    if let Some(w) = weight(
        0.45,
        &[
            "crash",
            "bug",
            "error",
            "panic",
            "exception",
            "segfault",
            "deadlock",
            "oom",
            "does not",
            "doesn't",
            "not working",
            "broken",
            "fails",
            "failing",
            "incorrect",
            "unexpected",
            "regression",
            "hangs",
            "freeze",
            "失败",
            "崩溃",
            "报错",
            "错误",
            "故障",
            "异常",
            "闪退",
            "卡死",
            "死锁",
            "阻塞",
            "超时",
            "无法",
            "不能用",
            "用不了",
            "不生效",
            "没反应",
            "失效",
            "丢失",
            "清空",
            "截断",
            "溢出",
            "耗尽",
            "暴涨",
            "未终止",
            "无限递归",
            "泄漏",
        ],
    ) {
        bump(&mut scores, IssueType::Bug, w, "error_language");
    }
    if let Some(w) = weight(
        0.4,
        &[
            "feature",
            "enhancement",
            "wishlist",
            "would be nice",
            "please add",
            "add support",
            "support for",
            "proposal",
            "migrate",
            "introduce",
            "支持",
            "希望",
            "建议",
            "需求",
            "增加",
            "新增",
            "优化",
            "能否",
            "期望",
            "改进",
        ],
    ) {
        bump(
            &mut scores,
            IssueType::FeatureRequest,
            w,
            "feature_language",
        );
    }
    if let Some(w) = weight(
        0.35,
        &["how to", "how do i", "question", "?", "请问", "怎么"],
    ) {
        bump(&mut scores, IssueType::Question, w, "question_language");
    }
    if let Some(w) = weight(0.4, &["docs", "documentation", "readme", "typo", "文档"]) {
        bump(&mut scores, IssueType::Documentation, w, "docs_language");
    }
    if let Some(w) = weight(0.3, &["config", "configuration", "settings", "配置"]) {
        bump(&mut scores, IssueType::Configuration, w, "config_language");
    }
    if let Some(w) = weight(
        0.5,
        &[
            "security",
            "vulnerability",
            "cve",
            "xss",
            "csrf",
            "ssrf",
            "rce",
            "injection",
            "traversal",
            "安全",
            "漏洞",
            "注入",
            "穿越",
            "越权",
            "提权",
            "泄露",
            "绕过",
            "凭据",
            "权限过宽",
        ],
    ) {
        bump(&mut scores, IssueType::Security, w, "security_language");
    }
    if let Some(w) = weight(
        0.4,
        &["slow", "performance", "latency", "memory leak", "性能"],
    ) {
        bump(&mut scores, IssueType::Performance, w, "perf_language");
    }
    if ["windows", "macos", "linux", "android", "ios", "compat"]
        .iter()
        .any(|k| text.contains(k))
        && scores.iter().any(|(t, _, _)| *t == IssueType::Bug)
    {
        bump(&mut scores, IssueType::Compatibility, 0.15, "platform_hint");
    }

    // 英文祈使句标题（`Add …` / `Support …` / `Automatically append …`）是需求的常见写法。
    // 只看前两个词，避免正文里的动词把缺陷报告也拽成需求。
    if leading_action_verb(title) {
        // 强度等同于「标题命中 feature 关键词」：祈使句标题本身就是明确的诉求声明。
        bump(
            &mut scores,
            IssueType::FeatureRequest,
            0.4 + TITLE_BONUS,
            "imperative_title",
        );
    }

    // 作者自己写的 `docs:` / `fix:` 前缀压过散落关键词——那是显式声明。
    if let Some(t) = conventional_prefix(title) {
        bump(&mut scores, t, PREFIX_BONUS, "conventional_prefix");
    }

    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    if let Some((t, conf, why)) = scores.first() {
        Classification {
            primary_type: *t,
            confidence: conf.min(0.95),
            reasons: vec![(*why).to_string()],
        }
    } else {
        Classification {
            primary_type: IssueType::Unknown,
            confidence: 0.3,
            reasons: vec!["no_strong_signal".into()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::safety::score_safety;

    #[test]
    fn classifies_bug() {
        let safety = score_safety("app crash", "panic on save");
        let c = classify_heuristic("app crash", "panic on save", &safety);
        assert_eq!(c.primary_type, IssueType::Bug);
    }

    /// 线上回归：atomgit#2 标题直接写 `docs`，正文也是文档诉求，却只给 40%。
    /// 单次固定权重让证据量和命中位置完全不影响分数。
    #[test]
    fn title_hit_scores_higher_than_passing_body_mention() {
        let t = "docs：希望添加最佳实践";
        let b = "如题，希望官方提供一些最佳实践";
        let strong = classify_heuristic(t, b, &score_safety(t, b));
        assert_eq!(strong.primary_type, IssueType::Documentation);
        assert!(
            strong.confidence >= 0.6,
            "explicit docs ask should be confident, got {}",
            strong.confidence
        );

        let t2 = "保存时崩溃";
        let b2 = "panic on save，另外文档里也没写这个字段";
        let weak = classify_heuristic(t2, b2, &score_safety(t2, b2));
        assert_eq!(
            weak.primary_type,
            IssueType::Bug,
            "a passing docs mention must not outweigh a crash report"
        );
    }

    /// 线上回归（atomcode 123 条 open issue 全量 triage）：
    /// URL 和文件名里的字面量被当成了主题词。
    #[test]
    fn identifiers_are_not_topic_signals() {
        // #1258：正文只有两张图和一个文档链接，主题是「rewind 用不了」
        let t = "rewind  不能用";
        let b = "![image.png](https://raw.atomgit.com/user-images/a/image.png 'image.png')\n\
                 https://atomcode.atomgit.com/docs/zh/slash-commands.html";
        let c = classify_heuristic(t, b, &score_safety(t, b));
        assert_eq!(
            c.primary_type,
            IssueType::Bug,
            "a docs URL must not make it a docs request: {c:?}"
        );

        // #49：SECURITY.md 是文件名，不是安全主题
        let t2 = "【社区】缺少 CONTRIBUTING.md / SECURITY.md / CHANGELOG.md";
        let b2 = "建议补齐社区文档，方便新人参与";
        let c2 = classify_heuristic(t2, b2, &score_safety(t2, b2));
        assert_ne!(
            c2.primary_type,
            IssueType::Security,
            "a filename must not make it a security issue: {c2:?}"
        );
    }

    /// 线上回归（GitHub pallets/click#3571）：`rce` 命中了 "pe**rce**ntage"，
    /// 一条进度条显示 Bug 被判成安全问题。短 ASCII 缩写必须按词边界匹配，
    /// 否则 `oom`⊂"room"、`bug`⊂"debug" 都会误命中。
    #[test]
    fn ascii_keywords_need_word_boundaries() {
        let t = "`click.progressbar` doesn't show full completion";
        let b = "This would be consistent with the default percentage formatting, as can be seen \
                 by commenting out the line.";
        let c = classify_heuristic(t, b, &score_safety(t, b));
        assert_ne!(c.primary_type, IssueType::Security, "{c:?}");

        // 但真正独立出现的缩写仍要命中
        let t2 = "Possible RCE via template injection";
        let c2 = classify_heuristic(t2, "user input reaches eval", &score_safety(t2, ""));
        assert_eq!(c2.primary_type, IssueType::Security, "{c2:?}");
    }

    /// 线上回归（GitHub pallets/click#3652）：标题「Automatically append ellipsis…」
    /// 是功能建议，却因为正文用 "does not visually signal" 描述现状痛点而被判成缺陷。
    /// 英文 Issue 的祈使句标题（动词开头）是需求的常见形态。
    #[test]
    fn imperative_title_reads_as_a_request() {
        let t = "Automatically append ellipsis (`...`) to metavars when `multiple=True` in options";
        let b = "the auto-generated usage string does not visually signal that the option \
                 can be repeated. Expected behavior if option foo has multiple=True";
        let c = classify_heuristic(t, b, &score_safety(t, b));
        assert_eq!(c.primary_type, IssueType::FeatureRequest, "{c:?}");
    }

    /// 但「X does not work」这种主谓句仍是缺陷，不能被祈使句规则带偏。
    #[test]
    fn declarative_failure_title_stays_a_bug() {
        let t = "Bash completion does not clear required flags when supplied";
        let c = classify_heuristic(t, "still listed as required", &score_safety(t, ""));
        assert_eq!(c.primary_type, IssueType::Bug, "{c:?}");
    }

    /// 线上回归（GitHub spf13/cobra#2465、clap-rs/clap#6421 等）：英文缺陷/需求
    /// 的常用表述几乎没覆盖，三条真实 Issue 全判成 unknown 30%。
    #[test]
    fn english_defect_and_request_wording_is_classified() {
        for (t, b, want) in [
            (
                "Bash completion does not clear required flags when supplied",
                "After supplying the flag once, completion still lists it as required.",
                IssueType::Bug,
            ),
            (
                "bash: completion broken when the bin name contains a hyphen",
                "The generated script fails to source.",
                IssueType::Bug,
            ),
            (
                "Migrate sentinels to use Python 3.15's PEP 661 sentinel",
                "It would be nice to drop the custom sentinel class.",
                IssueType::FeatureRequest,
            ),
        ] {
            let c = classify_heuristic(t, b, &score_safety(t, b));
            assert_eq!(c.primary_type, want, "{t} -> {c:?}");
        }
    }

    /// 线上回归（AtomGit new_review/go-redis #3）：「Watch 回调返回错误时被反复重试」
    /// 判成 feature_request 40%——「错误」这个最常用的故障词根本不在表里，
    /// 反被正文的「期望」抢走，连带 verify 也没跑。
    #[test]
    fn plain_error_wording_is_a_bug() {
        let t = "ClusterClient 的 Watch 回调返回错误时被反复重试";
        let b = "回调自身返回业务错误时仍会重试整个事务。期望直接上抛。";
        let c = classify_heuristic(t, b, &score_safety(t, b));
        assert_eq!(c.primary_type, IssueType::Bug, "{c:?}");
    }

    /// 线上回归：18 条明确的缺陷被判成 unknown 30%，中文故障词几乎没覆盖。
    #[test]
    fn chinese_failure_vocabulary_is_classified_as_bug() {
        for (t, b) in [
            (
                "[共创大赛] hook脚本执行器stdout/stderr管道死锁",
                "并发写入时双方互相等待",
            ),
            (
                "[共创大赛] search_replace空搜索串导致整文件被清空",
                "传入空串后文件内容全部丢失",
            ),
            (
                "[共创大赛] write_file覆盖时read_to_string全量读取导致大文件OOM",
                "大文件时内存耗尽",
            ),
            ("[共创大赛] bridge runtime超时后子进程未终止", "子进程残留"),
            (
                "[共创大赛] MCP registry call_tool跨await持锁导致阻塞",
                "整个注册表被阻塞",
            ),
        ] {
            let c = classify_heuristic(t, b, &score_safety(t, b));
            assert_eq!(c.primary_type, IssueType::Bug, "{t} -> {c:?}");
        }
    }

    /// 线上回归：中文需求表述（建议/需求/希望/增加）全部落到 unknown。
    #[test]
    fn chinese_request_vocabulary_is_classified_as_feature() {
        for (t, b) in [
            (
                "建议：为 memory 系统引入三层记忆模型",
                "目前是扁平结构，几个明显短板",
            ),
            ("功能建议-增加桌面端-增加轮训功能", "桌面端需要定时刷新"),
            ("增加skill分类，方便查找使用", "skill 变多以后不好找"),
            (
                "需求：AtomCode for VSCode插件的历史对话策略优化",
                "希望能保留更多历史",
            ),
        ] {
            let c = classify_heuristic(t, b, &score_safety(t, b));
            assert_eq!(c.primary_type, IssueType::FeatureRequest, "{t} -> {c:?}");
        }
    }

    /// 加了大量故障词之后，真正的安全问题不能被 Bug 抢走。
    #[test]
    fn real_security_issues_stay_security() {
        for (t, b) in [
            (
                "[共创大赛] hook脚本路径无限制导致目录穿越",
                "可以写到仓库外的路径",
            ),
            (
                "[共创大赛] skill模板shell注入执行顺序导致命令注入",
                "模板参数直接拼进 shell",
            ),
            (
                "[共创大赛] web_search DuckDuckGo重定向无协议限制导致SSRF",
                "可请求内网地址",
            ),
        ] {
            let c = classify_heuristic(t, b, &score_safety(t, b));
            assert_eq!(c.primary_type, IssueType::Security, "{t} -> {c:?}");
        }
    }

    /// 曾经试过「命中安全词就优先」，被真实数据否掉：`明文`、`内存安全` 这种
    /// 顺带一提的词会把 20 条明确标了 [Feature] 的需求拽成安全问题，
    /// 代价远大于它想保住的那几条资源耗尽缺陷。正文里的弱安全词不得压过显式声明。
    #[test]
    fn stray_security_word_never_overrides_explicit_tag() {
        let t = "[共创大赛]-[Feature] /model -s — 仅切换当前会话，不写全局默认";
        let b = "目前 /model 会把选择明文写入全局配置，希望增加仅本次会话生效的开关";
        let c = classify_heuristic(t, b, &score_safety(t, b));
        assert_eq!(c.primary_type, IssueType::FeatureRequest, "{c:?}");

        let t2 = "[共创大赛] memory文件超限时读取整个文件导致OOM";
        let b2 = "## 影响范围\n- 影响类型：内存安全\n";
        let c2 = classify_heuristic(t2, b2, &score_safety(t2, b2));
        assert_eq!(
            c2.primary_type,
            IssueType::Bug,
            "resource exhaustion reads as a defect unless the title says otherwise: {c2:?}"
        );
    }

    /// 回归：`[Feature]` 方括号标签没被当成显式声明，被正文的「数据丢失」抢成 Bug。
    #[test]
    fn bracket_tag_declares_the_type() {
        let t = "[共创大赛]-[Feature] 修改项目源码 / 配置前执行备份,避免数据丢失 / 回滚风险";
        let b = "### 存在风险\n修改出错后无法一键回滚，容易导致项目启动失败";
        let c = classify_heuristic(t, b, &score_safety(t, b));
        assert_eq!(c.primary_type, IssueType::FeatureRequest, "{c:?}");
    }

    /// 回归：「没办法…希望…」是先讲痛点再提诉求，不是故障报告。
    #[test]
    fn pain_point_plus_wish_is_a_feature_request() {
        let t = "在命令行开发的时候没办法看完实时的对话，希望将对话、执行、审核分开 这样方便清楚";
        let b = "cli看到的文字有限，跑测试时会把之前的会话覆盖掉，希望能把会话执行分开";
        let c = classify_heuristic(t, b, &score_safety(t, b));
        assert_eq!(c.primary_type, IssueType::FeatureRequest, "{c:?}");
    }

    #[test]
    fn confidence_stays_capped() {
        let t = "docs documentation readme typo 文档";
        let b = "docs documentation readme typo 文档";
        let c = classify_heuristic(t, b, &score_safety(t, b));
        assert!(c.confidence <= 0.95, "got {}", c.confidence);
    }
}

//! 面向用户的 Issue 说明：优先用 LLM 基于**已验证证据**写人话；
//! 无 LLM 或失败时用确定性模板（仍只引用真实命中，不编路径）。
//! 回复语言：跟随 Issue 正文语言（中/英），可被环境变量覆盖。

use super::model::{
    IssueReviewDecision, IssueType, IssueVerdict, NormalizedIssue, BOT_COMMENT_MARKER,
};
use super::verify::TechnicalVerification;
use crate::llm::LlmClient;
use crate::model::Message;
use anyhow::{Context, Result};

/// 评论回复语言（MVP：中/英）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyLang {
    Zh,
    En,
}

/// 探测回复语言。
/// 优先级：`REVIEWGATE_ISSUE_REPLY_LANGUAGE` → Issue 标题/正文 CJK 占比 → `output_language()`。
pub fn detect_reply_lang(title: &str, body: &str) -> ReplyLang {
    if let Ok(explicit) = std::env::var("REVIEWGATE_ISSUE_REPLY_LANGUAGE") {
        let e = explicit.trim().to_ascii_lowercase();
        if !e.is_empty() {
            return parse_reply_lang(&e);
        }
    }
    // 链接/代码里的拉丁字符不代表 Issue 是英文写的，先剥掉再数。
    let sample = super::classify::strip_topic_noise(&format!("{title}\n{body}"));
    let (cjk, latin) = count_scripts(&sample);
    // 有足够汉字则中文；明显拉丁文则英文；否则跟 CLI 输出语言
    if cjk >= 12 && cjk * 2 >= latin {
        return ReplyLang::Zh;
    }
    if latin >= 40 && cjk * 3 < latin {
        return ReplyLang::En;
    }
    let out = crate::language::output_language().to_ascii_lowercase();
    if out.contains("chinese") || out.starts_with("zh") || out.contains("中文") {
        ReplyLang::Zh
    } else {
        ReplyLang::En
    }
}

fn parse_reply_lang(s: &str) -> ReplyLang {
    match s {
        "zh" | "zh-cn" | "zh_cn" | "chinese" | "中文" | "cn" => ReplyLang::Zh,
        _ => ReplyLang::En,
    }
}

fn count_scripts(s: &str) -> (usize, usize) {
    let mut cjk = 0usize;
    let mut latin = 0usize;
    for ch in s.chars() {
        if ('\u{4e00}'..='\u{9fff}').contains(&ch)
            || ('\u{3400}'..='\u{4dbf}').contains(&ch)
            || ('\u{f900}'..='\u{faff}').contains(&ch)
        {
            cjk += 1;
        } else if ch.is_ascii_alphabetic() {
            latin += 1;
        }
    }
    (cjk, latin)
}

/// 按结论选择回复形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyShape {
    /// 疑似/确认缺陷：定位 + 机制 + 建议
    FullDebug,
    /// 疑似回归：定位 + 对照历史修复 + 版本对比建议
    Regression,
    /// 需要补充信息
    NeedsInfo,
    /// 可能重复
    Duplicate,
    /// 可能已修复
    AlreadyFixed,
    /// 非缺陷
    NotABug,
    /// 垃圾/广告
    SpamShort,
}

pub fn reply_shape(decision: &IssueReviewDecision) -> ReplyShape {
    match decision.verdict {
        IssueVerdict::Spam | IssueVerdict::Advertisement => ReplyShape::SpamShort,
        IssueVerdict::NeedsInfo => ReplyShape::NeedsInfo,
        IssueVerdict::Duplicate => ReplyShape::Duplicate,
        IssueVerdict::AlreadyFixed => ReplyShape::AlreadyFixed,
        IssueVerdict::NotABug => ReplyShape::NotABug,
        IssueVerdict::Regression => ReplyShape::Regression,
        IssueVerdict::LikelyBug | IssueVerdict::ConfirmedBug => ReplyShape::FullDebug,
        IssueVerdict::Unverified => {
            // 文档/功能/提问未跑代码验证时，不要用「缺报错」的 NeedsInfo 话术
            if matches!(
                decision.primary_type,
                IssueType::Documentation
                    | IssueType::FeatureRequest
                    | IssueType::Question
                    | IssueType::Support
            ) {
                return ReplyShape::NotABug;
            }
            if decision.verification_ran && !decision.code_hits.is_empty() {
                ReplyShape::FullDebug
            } else {
                ReplyShape::NeedsInfo
            }
        }
    }
}

/// 同步生成（无 LLM）：确定性人话 + marker。
pub fn generate_user_comment_sync(
    decision: &IssueReviewDecision,
    normalized: &NormalizedIssue,
    body_raw: &str,
    technical: Option<&TechnicalVerification>,
    mentions_line: Option<&str>,
) -> String {
    let lang = detect_reply_lang(&decision.issue_title, body_raw);
    let narrative = compose_narrative(lang, decision, normalized, body_raw, technical, None);
    assemble_comment(decision, &narrative, mentions_line, technical)
}

/// 生成完整用户向评论（含 marker）。
/// 缺陷类：结构化「已核实/未证实」；LLM **仅润色「可先做」**，机制不交给模型编。
pub async fn generate_user_comment(
    llm: Option<&dyn LlmClient>,
    decision: &IssueReviewDecision,
    normalized: &NormalizedIssue,
    body_raw: &str,
    technical: Option<&TechnicalVerification>,
    mentions_line: Option<&str>,
) -> String {
    let lang = detect_reply_lang(&decision.issue_title, body_raw);
    let tips_override = if let Some(client) = llm {
        if should_polish_tips(decision) {
            match polish_user_tips_llm(client, lang, decision, normalized, technical).await {
                Ok(tips) if !tips.is_empty() => Some(tips),
                Ok(_) => None,
                Err(e) => {
                    eprintln!("issue tip polish llm failed: {e:#}; using template tips");
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // 缺陷类：事实结构；其余类型一律确定性模板（避免 LLM 编「关闭 Issue」等动作）
    let narrative = if super::facts::use_fact_structure(decision) {
        compose_narrative(
            lang,
            decision,
            normalized,
            body_raw,
            technical,
            tips_override.as_deref(),
        )
    } else {
        deterministic_narrative_lang(lang, decision, normalized, body_raw, technical)
    };

    assemble_comment(decision, &narrative, mentions_line, technical)
}

/// 是否让 LLM 润色「用户可先做」。安全报告一律不润色：模型不知道这是漏洞，
/// 会按通用故障排查补上「附日志抓包」「升级到最新版复现」，甚至可能引导公开 PoC。
fn should_polish_tips(decision: &IssueReviewDecision) -> bool {
    super::facts::use_fact_structure(decision) && decision.primary_type != IssueType::Security
}

/// 移交话术：不下结论，只说明已请人来看。
///
/// 对提问者这是正面信息（有人会接手，不是石沉大海），对处理人是一次点名。
/// 刻意不写类型/裁决——机器人正是因为没把握才走到这里，说了反而误导。
pub fn render_triage_handoff(
    decision: &IssueReviewDecision,
    body_raw: &str,
    owners: &[String],
) -> String {
    let zh = matches!(
        detect_reply_lang(&decision.issue_title, body_raw),
        ReplyLang::Zh
    );
    let at = owners
        .iter()
        .map(|o| format!("@{o}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut md = String::new();
    md.push_str(BOT_COMMENT_MARKER);
    md.push_str("\n\n");
    if zh {
        md.push_str("你好，谢谢反馈。\n\n");
        md.push_str(&format!("这条我判断不了，已经请 {at} 来看一下。\n"));
    } else {
        md.push_str("Hi, thanks for the report.\n\n");
        md.push_str(&format!(
            "I can't call this one with confidence, so I've asked {at} to take a look.\n"
        ));
    }
    md
}

fn compose_narrative(
    lang: ReplyLang,
    decision: &IssueReviewDecision,
    normalized: &NormalizedIssue,
    body_raw: &str,
    technical: Option<&TechnicalVerification>,
    user_tips_override: Option<&[String]>,
) -> String {
    if super::facts::use_fact_structure(decision) {
        let pack = super::facts::build_fact_pack(decision, normalized, technical);
        return super::facts::render_fact_comment(
            matches!(lang, ReplyLang::Zh),
            decision,
            &pack,
            user_tips_override,
        );
    }
    deterministic_narrative_lang(lang, decision, normalized, body_raw, technical)
}

const TIPS_SYSTEM_ZH: &str = r#"你只润色「用户可先做」操作建议。
规则：
- 只输出 2～4 行，每行一条建议，不要标题、不要编号前缀以外的格式。
- 不要写代码路径、不要写 commit、不要推断根因、不要提上游负载/用户网络一定有问题。
- 可基于已给事实提醒用户补日志/版本/换端点对比，但不得发明新事实。
- 简体中文，每条不超过 40 字。
"#;

const TIPS_SYSTEM_EN: &str = r#"You only polish user-action tips.
Rules:
- Output 2–4 lines, one tip per line. No headings.
- Do not invent paths, commits, or root causes. No upstream-load or "your network is broken" claims.
- English, each tip under ~20 words.
"#;

async fn polish_user_tips_llm(
    llm: &dyn LlmClient,
    lang: ReplyLang,
    decision: &IssueReviewDecision,
    normalized: &NormalizedIssue,
    technical: Option<&TechnicalVerification>,
) -> Result<Vec<String>> {
    let pack = super::facts::build_fact_pack(decision, normalized, technical);
    let system = match lang {
        ReplyLang::Zh => TIPS_SYSTEM_ZH,
        ReplyLang::En => TIPS_SYSTEM_EN,
    };
    let mut user = String::new();
    user.push_str("Verified facts (do not contradict):\n");
    for f in pack.verified.iter().take(6) {
        user.push_str(match lang {
            ReplyLang::Zh => &f.zh,
            ReplyLang::En => &f.en,
        });
        user.push('\n');
    }
    user.push_str("\nDefault tips to polish:\n");
    for t in &pack.user_tips {
        user.push_str(match lang {
            ReplyLang::Zh => &t.zh,
            ReplyLang::En => &t.en,
        });
        user.push('\n');
    }
    let resp = llm
        .complete(system, &[Message::user(user)], &[])
        .await
        .context("llm polish tips")?;
    let tips: Vec<String> = resp
        .text()
        .lines()
        .map(|l| {
            l.trim()
                .trim_start_matches(|c: char| {
                    c.is_ascii_digit() || c == '.' || c == ')' || c == '、'
                })
                .trim()
                .to_string()
        })
        .filter(|l| l.chars().count() >= 6 && l.chars().count() <= 80)
        .take(5)
        .collect();
    Ok(tips)
}

/// 无 LLM 时：按结论形态生成人话（默认跟 Issue 语言）。
pub fn deterministic_narrative(
    decision: &IssueReviewDecision,
    normalized: &NormalizedIssue,
    body_raw: &str,
    technical: Option<&TechnicalVerification>,
) -> String {
    let lang = detect_reply_lang(&decision.issue_title, body_raw);
    deterministic_narrative_lang(lang, decision, normalized, body_raw, technical)
}

fn deterministic_narrative_lang(
    lang: ReplyLang,
    decision: &IssueReviewDecision,
    normalized: &NormalizedIssue,
    body_raw: &str,
    technical: Option<&TechnicalVerification>,
) -> String {
    // 缺陷类统一走事实结构（机制模板化）
    if super::facts::use_fact_structure(decision) {
        let pack = super::facts::build_fact_pack(decision, normalized, technical);
        return super::facts::render_fact_comment(
            matches!(lang, ReplyLang::Zh),
            decision,
            &pack,
            None,
        );
    }
    match (lang, reply_shape(decision)) {
        (ReplyLang::Zh, ReplyShape::FullDebug) => {
            narrative_full_debug(decision, normalized, body_raw)
        }
        (ReplyLang::Zh, ReplyShape::Regression) => {
            narrative_regression(decision, normalized, body_raw)
        }
        (ReplyLang::Zh, ReplyShape::NeedsInfo) => {
            narrative_needs_info(decision, normalized, body_raw)
        }
        (ReplyLang::Zh, ReplyShape::Duplicate) => {
            narrative_duplicate(decision, normalized, body_raw)
        }
        (ReplyLang::Zh, ReplyShape::AlreadyFixed) => {
            narrative_already_fixed(decision, normalized, body_raw)
        }
        (ReplyLang::Zh, ReplyShape::NotABug) => narrative_not_a_bug(decision, normalized, body_raw),
        (ReplyLang::Zh, ReplyShape::SpamShort) => narrative_spam(decision),
        (ReplyLang::En, shape) => narrative_en(shape, decision, normalized, body_raw),
    }
}

fn greeting(lang: ReplyLang) -> &'static str {
    match lang {
        ReplyLang::Zh => "你好，谢谢反馈。\n\n",
        ReplyLang::En => "Hi, thanks for the report.\n\n",
    }
}

/// English deterministic templates (compact).
fn narrative_en(
    shape: ReplyShape,
    decision: &IssueReviewDecision,
    normalized: &NormalizedIssue,
    body_raw: &str,
) -> String {
    let mut md = String::new();
    md.push_str(greeting(ReplyLang::En));
    let symptom = symptom_lead(decision, normalized, body_raw, 200, ". ");
    match shape {
        ReplyShape::FullDebug => {
            md.push_str(&symptom);
            md.push_str(&verdict_paragraph_en(
                decision.verdict,
                decision.primary_type,
                decision.confidence,
            ));
            md.push_str("\n\n");
            let src: Vec<_> = decision
                .code_hits
                .iter()
                .filter(|h| {
                    let p = h.path.to_ascii_lowercase();
                    !p.ends_with(".md") && !p.contains("/docs/")
                })
                .take(3)
                .collect();
            if let Some(h) = src.iter().find(|h| {
                h.snippet.contains("error")
                    || h.snippet.contains("retry")
                    || normalized
                        .error_signatures
                        .iter()
                        .any(|e| h.snippet.contains(e))
            }) {
                md.push_str(&format!(
                    "The user-visible message lines up with `{}:{}`. ",
                    h.path, h.line
                ));
            }
            if !src.is_empty() {
                md.push_str("Related code: ");
                md.push_str(
                    &src.iter()
                        .map(|h| format!("`{}:{}`", h.path, h.line))
                        .collect::<Vec<_>>()
                        .join("; "),
                );
                md.push_str(". ");
            }
            if src.iter().any(|h| h.path.contains("retry")) {
                md.push_str(
                    "Typical path: request → read response → limited retries → surface error if still failing.",
                );
            }
            if body_raw.to_ascii_lowercase().contains("flash") {
                md.push_str(" Model quality issues are often separate from disconnects.");
            }
            md.push_str("\n\nTry:\n");
            md.push_str("1. Paste the full error and when it started.\n");
            md.push_str("2. Confirm app version; upgrade if possible.\n");
            md.push_str("3. Compare via terminal with the same API key (client vs upstream).\n");
            md.push_str(
                "\nAvoid deleting session/lock files to “fix network”; avoid blindly raising retry counts.\n",
            );
            if !src.is_empty() {
                md.push_str(
                    "\nIf digging further: walk up from the retry/error wrapper to the failing HTTP/SSE read, and check recent commits touching timeouts/streaming.\n",
                );
            }
        }
        ReplyShape::Regression => {
            md.push_str(&symptom);
            md.push_str("This looks like a **regression** (fixed before, back now).\n\n");
            md.push_str("Please note your version and when it returned; retest on the fix version vs current; attach full error + steps.\n");
        }
        ReplyShape::NeedsInfo => {
            md.push_str(&symptom);
            md.push_str(
                "Not enough detail yet to tell bug vs config vs environment.\n\nPlease add:\n",
            );
            md.push_str("- full error + repro steps\n- OS / app version / model / proxy\n");
            md.push_str("\nEdit the issue or comment when ready.\n");
        }
        ReplyShape::Duplicate => {
            md.push_str(&symptom);
            md.push_str("Looks similar to existing discussion");
            if let Some(n) = decision.duplicate_of {
                md.push_str(&format!(" (#{n})"));
            }
            md.push_str(
                ". Check those issues first; link/close if same, or explain version/platform differences if not.\n",
            );
        }
        ReplyShape::AlreadyFixed => {
            md.push_str(&symptom);
            md.push_str(
                "History suggests this may already be fixed in a later release. Please upgrade and retest; if it still repros, reply with version + full error + steps.\n",
            );
        }
        ReplyShape::NotABug => {
            md.push_str(&symptom);
            md.push_str(&verdict_paragraph_en(
                stance_verdict(decision),
                decision.primary_type,
                decision.confidence,
            ));
            md.push_str(match decision.primary_type {
                IssueType::Documentation => {
                    " Could you say which part you need most — configuration, deployment, or a specific scenario?\n"
                }
                IssueType::FeatureRequest => {
                    " Please add the use case and the behaviour you expect so it can be evaluated.\n"
                }
                IssueType::Question | IssueType::Support | IssueType::Configuration => {
                    " Check the README/config docs first; if you’re still stuck, share your config snippet (redacted) and the result you expect.\n"
                }
                _ => {
                    " If you do think it’s a defect, add actual vs expected plus repro steps.\n"
                }
            });
        }
        ReplyShape::SpamShort => {
            md.push_str(
                "This doesn’t look like a valid project issue (spam/promo). We may mark or close it per project policy; if that’s wrong, restate the technical problem in one sentence.\n",
            );
        }
    }
    md
}

fn verdict_paragraph_en(v: IssueVerdict, t: IssueType, _conf: f32) -> String {
    // Never expose internal confidence jargon to reporters (parity with ZH path).
    match v {
        IssueVerdict::LikelyBug | IssueVerdict::ConfirmedBug => format!(
            "This looks more like a **{}**-related software/network issue than a pure usage question.",
            t.as_str()
        ),
        IssueVerdict::NeedsInfo => "We can’t classify it yet.".into(),
        IssueVerdict::Duplicate => "It resembles existing discussion.".into(),
        // Written for the reporter: they never claimed it was a bug, so don't rebut that.
        IssueVerdict::NotABug => match t {
            IssueType::Documentation => "Noted — this is a documentation request.".into(),
            IssueType::FeatureRequest => "Noted — this is a feature request.".into(),
            IssueType::Question | IssueType::Support | IssueType::Configuration => {
                "Noted — this is a usage or configuration question.".into()
            }
            _ => "Noted — this doesn’t look like something that needs a code-level investigation."
                .into(),
        },
        IssueVerdict::AlreadyFixed => "It may already be fixed upstream.".into(),
        IssueVerdict::Regression => "It may be a regression.".into(),
        IssueVerdict::Spam | IssueVerdict::Advertisement => {
            "This looks off-topic or promotional.".into()
        }
        IssueVerdict::Unverified => {
            "Evidence is still thin—we can’t pin a root cause yet.".into()
        }
    }
}

fn narrative_full_debug(
    decision: &IssueReviewDecision,
    normalized: &NormalizedIssue,
    body_raw: &str,
) -> String {
    let mut md = String::new();
    md.push_str(greeting(ReplyLang::Zh));
    md.push_str(&symptom_lead(decision, normalized, body_raw, 200, "。"));
    md.push_str(&verdict_paragraph(
        decision.verdict,
        decision.primary_type,
        decision.confidence,
    ));
    md.push_str("\n\n");

    let src_hits: Vec<_> = decision
        .code_hits
        .iter()
        .filter(|h| {
            let p = h.path.to_ascii_lowercase();
            !p.ends_with(".md") && !p.contains("/docs/")
        })
        .collect();
    let exact = src_hits.iter().find(|h| {
        h.snippet.contains("网络连接中断")
            || h.snippet.contains("error decoding")
            || normalized
                .error_signatures
                .iter()
                .any(|e| !e.is_empty() && h.snippet.contains(e))
    });

    if let Some(h) = exact {
        md.push_str(&format!(
            "你看到的这句提示，对应 `{}:{}`（重试失败后的兜底文案）。",
            h.path, h.line
        ));
    }
    let extra: Vec<_> = src_hits
        .iter()
        .filter(|h| !exact.is_some_and(|e| e.path == h.path && e.line == h.line))
        .take(2)
        .collect();
    if !extra.is_empty() {
        md.push_str("相关实现还有：");
        for (i, h) in extra.iter().enumerate() {
            if i > 0 {
                md.push('；');
            }
            md.push_str(&format!("`{}:{}`", h.path, h.line));
        }
        md.push('。');
    }
    if exact.is_some()
        || src_hits
            .iter()
            .any(|h| h.path.contains("retry") || h.snippet.contains("重试"))
    {
        md.push_str(
            "大致链路是请求→读响应→有限次重试→仍失败再提示；「error decoding response」多半是连接中途被掐断。",
        );
    } else if src_hits.is_empty() {
        md.push_str("目前还缺足够的源码锚点，更像链路/环境问题，需要完整日志才能钉死。");
    }
    if body_raw.contains("deepseek") || body_raw.to_ascii_lowercase().contains("flash") {
        md.push_str("另外「模型变差」常与断连不是同一根因，最好分开验证。");
    }
    let steps = practical_steps(decision, normalized);
    if !steps.is_empty() {
        md.push_str("\n\n");
        for (i, step) in steps.into_iter().take(4).enumerate() {
            md.push_str(&format!("{}. {}\n", i + 1, step));
        }
    }
    let ds = donts(decision);
    if !ds.is_empty() {
        md.push('\n');
        md.push_str(&ds.into_iter().take(2).collect::<Vec<_>>().join("；"));
        md.push_str("。\n");
    }
    // 真问题倾向：自然段给维护者 1～3 句查根因方向
    if matches!(
        decision.verdict,
        IssueVerdict::LikelyBug | IssueVerdict::ConfirmedBug | IssueVerdict::Unverified
    ) {
        let tips = maintainer_root_cause_tips(decision, &src_hits);
        if !tips.is_empty() {
            md.push_str("\n若继续往根因查：");
            md.push_str(&tips.join(""));
            md.push('\n');
        }
    }
    append_missing_short(&mut md, decision);
    md
}

/// 维护者向：1～3 句如何往根因挖（基于已有命中，不编路径）。
fn maintainer_root_cause_tips(
    decision: &IssueReviewDecision,
    src_hits: &[&super::model::CodeEvidence],
) -> Vec<String> {
    let mut tips = Vec::new();
    let paths: Vec<&str> = src_hits.iter().map(|h| h.path.as_str()).collect();
    let blob = paths.join(" ");

    if blob.contains("retry")
        || src_hits
            .iter()
            .any(|h| h.snippet.contains("重试") || h.snippet.contains("重连"))
    {
        tips.push(
            "可从重试兜底提示往上追：是哪一次 HTTP/SSE 读失败触发了包装错误，对端 RST、超时还是代理掐流。".into(),
        );
    }
    if blob.contains("openai") || blob.contains("provider") || blob.contains("stream") {
        tips.push(
            "对照流式读循环与结束条件（是否半包、无 finish_reason / [DONE] 就结束），区分传输层断连与协议层解析失败。".into(),
        );
    }
    if blob.contains("auth") || blob.contains("oauth") || blob.contains("gateway") {
        tips.push(
            "同时排除鉴权/网关：token 过期或网关 4xx/5xx 有时也会被收成通用连接错误。".into(),
        );
    }
    if tips.is_empty() && !src_hits.is_empty() {
        let p0 = src_hits[0];
        tips.push(format!(
            "可从 `{}:{}` 沿调用方往上找触发条件与错误是否被吞掉/改写。",
            p0.path, p0.line
        ));
    }
    if !decision.related_commits.is_empty() {
        tips.push("也可对照近期相关提交，看是否引入超时、并发或流式读变更。".into());
    }
    tips.truncate(3);
    tips
}

fn narrative_regression(
    decision: &IssueReviewDecision,
    normalized: &NormalizedIssue,
    body_raw: &str,
) -> String {
    let mut md = String::new();
    md.push_str(greeting(ReplyLang::Zh));
    md.push_str(&symptom_lead(decision, normalized, body_raw, 180, "。"));
    md.push_str("这很像修过又出现，存在回归可能。");

    let mut bits = Vec::new();
    for c in decision.related_commits.iter().take(2) {
        bits.push(format!("历史 `{c}`"));
    }
    for p in decision.fix_prs.iter().take(2) {
        bits.push(format!("修复 {p}"));
    }
    for h in decision
        .code_hits
        .iter()
        .filter(|h| !h.path.ends_with(".md"))
        .take(2)
    {
        bits.push(format!("现状 `{}:{}`", h.path, h.line));
    }
    if !bits.is_empty() {
        md.push_str("相关线索有：");
        md.push_str(&bits.join("；"));
        md.push('。');
    }
    md.push_str(
        "\n\n可以写明当前版本、大概从哪版又开始出现，并在曾修复版本与当前版本上各复现一次，附完整报错和步骤。未确认版本差前别大改本地配置。",
    );
    if !decision.related_commits.is_empty() || !decision.code_hits.is_empty() {
        md.push_str(
            "若确认是回归，优先对相关提交做 blame/区间对比，看超时、重试或流式读是否被改动。",
        );
    }
    md.push('\n');
    append_missing_short(&mut md, decision);
    md
}

fn narrative_needs_info(
    decision: &IssueReviewDecision,
    normalized: &NormalizedIssue,
    body_raw: &str,
) -> String {
    let mut md = String::new();
    md.push_str(greeting(ReplyLang::Zh));
    md.push_str(&symptom_lead(decision, normalized, body_raw, 160, "。"));
    md.push_str("信息还不够，暂时没法判断是缺陷、配置还是环境问题。\n\n方便的话请补：\n");
    if decision.missing_fields.is_empty() {
        md.push_str("- 完整报错与复现步骤\n- 系统 / 应用版本 / 模型 / 是否代理\n");
    } else {
        for f in decision.missing_fields.iter().take(4) {
            md.push_str(&format!("- {}\n", missing_zh(f)));
        }
    }
    md.push_str("\n补全后编辑本 Issue 或再评论即可。\n");
    md
}

/// 值得写进评论的重复候选。列一条就等于向反馈者担保「这条相关」，
/// 弱召回（本地哈希 embedding 在中文上尤其松）不该占这个位置。
const DUPLICATE_LIST_MIN_SCORE: f32 = 0.5;

fn strong_duplicate_candidates(
    decision: &IssueReviewDecision,
) -> Vec<&crate::issue::model::DuplicateCandidate> {
    decision
        .duplicate_candidates
        .iter()
        .filter(|c| decision.duplicate_of != Some(c.issue_number))
        .filter(|c| c.score >= DUPLICATE_LIST_MIN_SCORE)
        .take(3)
        .collect()
}

fn narrative_duplicate(
    decision: &IssueReviewDecision,
    normalized: &NormalizedIssue,
    body_raw: &str,
) -> String {
    let mut md = String::new();
    md.push_str(greeting(ReplyLang::Zh));
    md.push_str(&symptom_lead(decision, normalized, body_raw, 140, "。"));
    md.push_str("这和已有讨论很像");
    let mut parts = Vec::new();
    if let Some(n) = decision.duplicate_of {
        parts.push(format!("#{n}"));
    }
    for c in strong_duplicate_candidates(decision) {
        parts.push(format!("#{} {}", c.issue_number, clip(&c.title, 36)));
    }
    if !parts.is_empty() {
        md.push('（');
        md.push_str(&parts.join("；"));
        md.push('）');
    }
    md.push_str(
        "。建议先看那些 Issue 的结论/workaround；同一问题可关联关闭本单，若不同请说明版本或平台差异。\n",
    );
    md
}

fn narrative_already_fixed(
    decision: &IssueReviewDecision,
    normalized: &NormalizedIssue,
    body_raw: &str,
) -> String {
    let mut md = String::new();
    md.push_str(greeting(ReplyLang::Zh));
    md.push_str(&symptom_lead(decision, normalized, body_raw, 140, "。"));
    md.push_str("历史线索显示这类问题可能已在后续版本修好");
    let mut bits = Vec::new();
    for c in decision.related_commits.iter().take(2) {
        bits.push(format!("`{c}`"));
    }
    for p in decision.fix_prs.iter().take(2) {
        bits.push(p.clone());
    }
    if !bits.is_empty() {
        md.push('（');
        md.push_str(&bits.join("、"));
        md.push('）');
    }
    md.push_str("。建议先升到较新版再试；仍复现请回帖版本号、完整报错和步骤。\n");
    md
}

fn narrative_not_a_bug(
    decision: &IssueReviewDecision,
    normalized: &NormalizedIssue,
    body_raw: &str,
) -> String {
    let mut md = String::new();
    md.push_str(greeting(ReplyLang::Zh));
    md.push_str(&symptom_lead(decision, normalized, body_raw, 140, "。"));
    md.push_str(&verdict_paragraph(
        stance_verdict(decision),
        decision.primary_type,
        decision.confidence,
    ));
    match decision.primary_type {
        IssueType::FeatureRequest => {
            md.push_str("方便的话补充一下使用场景和期望的行为，便于评估。\n");
        }
        IssueType::Question | IssueType::Support | IssueType::Configuration => {
            md.push_str("可以先看 README / 配置说明；如果仍卡住，贴一下你的配置片段（记得打码）和期望结果。\n");
        }
        IssueType::Documentation => {
            md.push_str(
                "为了便于跟进，能否说明你最需要哪部分文档——例如配置、部署，还是某个具体使用场景？\n",
            );
        }
        _ => {
            md.push_str("如果你认为是缺陷，请补充实际表现与期望行为，以及复现步骤。\n");
        }
    }
    md
}

fn narrative_spam(decision: &IssueReviewDecision) -> String {
    if decision.verdict == IssueVerdict::Advertisement {
        "你好。这条更像推广/广告，与项目问题无关。我们可能按规范标记或关闭；若是误判，请一句话说明真实技术问题。\n".into()
    } else {
        "你好。这条不太像有效的问题反馈。我们可能按规范处理；若是误判，请说明技术问题并补报错/复现。\n".into()
    }
}

fn append_missing_short(md: &mut String, decision: &IssueReviewDecision) {
    if decision.missing_fields.is_empty() {
        return;
    }
    md.push_str("\n若方便再补：");
    let parts: Vec<_> = decision
        .missing_fields
        .iter()
        .take(3)
        .map(|f| missing_zh(f))
        .collect();
    md.push_str(&parts.join("；"));
    md.push_str("。\n");
}

fn donts(d: &IssueReviewDecision) -> Vec<String> {
    let mut v = vec!["不要靠删会话/锁文件「修网络」（无关且可能丢会话）".into()];
    if d.code_hits
        .iter()
        .any(|h| h.path.contains("retry") || h.snippet.contains("重试"))
    {
        v.push("不要盲目加大重试次数（可能加重限流）".into());
    }
    v
}

/// 开场症状句：`「<症状>。」`；若只是复读原文则整句省略，让下一句直接开口。
fn symptom_lead(
    decision: &IssueReviewDecision,
    normalized: &NormalizedIssue,
    body_raw: &str,
    max: usize,
    sep: &str,
) -> String {
    let s = pick_symptom(decision, normalized);
    if echoes_body(&s, body_raw) {
        return String::new();
    }
    format!("{}{sep}", clip(&s, max))
}

/// 症状句是否只是把 Issue 原文整段复述回去（零信息复读）。
/// 判据：去掉标点空白后，症状覆盖了正文 ≥80% 的字符且是其子串。
fn echoes_body(symptom: &str, body_raw: &str) -> bool {
    fn squeeze(s: &str) -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    }
    let sym = squeeze(symptom);
    let body = squeeze(body_raw);
    if sym.is_empty() || body.is_empty() || !body.contains(&sym) {
        return false;
    }
    // 短正文：整段搬过来才算复读。
    if sym.chars().count() * 10 >= body.chars().count() * 8 {
        return true;
    }
    // 长正文：照抄一整句同样是复读，只是占全文比例小。低于这个长度更像是提炼。
    sym.chars().count() >= 30
}

fn pick_symptom(decision: &IssueReviewDecision, normalized: &NormalizedIssue) -> String {
    let candidates = [
        normalized.actual_behavior.as_str(),
        normalized.symptom.as_str(),
        decision.symptom_summary.as_str(),
        decision.issue_title.as_str(),
    ];
    const BAD: &[&str] = &[
        "遇到 bug 的页面",
        "问题描述",
        "期望的行为",
        "环境信息",
        "无",
        "本地环境",
    ];
    for c in candidates {
        let t = c.trim();
        if t.len() < 8 {
            continue;
        }
        if BAD.iter().any(|b| t == *b || t.starts_with(b)) {
            continue;
        }
        return t.to_string();
    }
    decision.issue_title.clone()
}

fn practical_steps(d: &IssueReviewDecision, n: &NormalizedIssue) -> Vec<String> {
    let mut s = Vec::new();
    match d.verdict {
        IssueVerdict::LikelyBug | IssueVerdict::ConfirmedBug | IssueVerdict::Unverified => {
            s.push("贴完整报错（含「详情:」后全文）和开始变频繁的时间。".into());
            s.push("确认应用版本；可先升到最新版再试。".into());
            if d.code_hits
                .iter()
                .any(|h| h.path.contains("retry") || h.snippet.contains("重试"))
            {
                s.push(
                    "「自动重连仍失败」= 重试用尽；可换网络/关代理，或终端直连 API 对比。".into(),
                );
            }
            s.push("用同一 API Key 在终端请求模型接口，区分客户端与上游。".into());
        }
        IssueVerdict::NeedsInfo => {
            s.push("按「请补充」补全后编辑 Issue 或再评论。".into());
        }
        IssueVerdict::Duplicate => {
            s.push("对照相关 Issue；同一问题可关联关闭。".into());
        }
        IssueVerdict::AlreadyFixed => {
            s.push("升级后再验证一次。".into());
        }
        _ => {
            s.push("补充环境与复现步骤。".into());
        }
    }
    if n.environment
        .as_object()
        .map(|o| o.is_empty())
        .unwrap_or(true)
    {
        s.push("补充系统、版本、模型、是否代理。".into());
    }
    s.truncate(4);
    s
}

/// 对外表态用的结论：`reply_shape` 已把「文档/需求/提问 + 未验证」归到 NotABug，
/// 表态句必须跟着走，否则会出现「文档诉求却索要日志和复现」这种自相矛盾的回复。
fn stance_verdict(decision: &IssueReviewDecision) -> IssueVerdict {
    if decision.verdict == IssueVerdict::Unverified && reply_shape(decision) == ReplyShape::NotABug
    {
        return IssueVerdict::NotABug;
    }
    decision.verdict
}

fn verdict_paragraph(v: IssueVerdict, t: IssueType, _conf: f32) -> String {
    // 不向用户暴露「把握/置信度」等内部用语
    match v {
        IssueVerdict::LikelyBug | IssueVerdict::ConfirmedBug => format!(
            "从你描述的错误来看，更像 **{}** 相关的程序或链路问题，而不只是用法疑问。",
            type_zh(t)
        ),
        IssueVerdict::NeedsInfo => "目前信息还不够，还不能判断是缺陷、配置还是环境问题。".into(),
        IssueVerdict::Duplicate => "和已有讨论很像，建议先对照历史 Issue，避免重复排查。".into(),
        // 这一类是写给提问者看的：对方没主张过「这是 bug」，就不要去否定它。
        // 只中性接收，具体化提问放在尾句。
        IssueVerdict::NotABug => match t {
            IssueType::Documentation => "收到，这是文档类的诉求。".into(),
            IssueType::FeatureRequest => "收到，这是功能类的诉求。".into(),
            IssueType::Question | IssueType::Support | IssueType::Configuration => {
                "收到，这是用法或配置层面的问题。".into()
            }
            _ => "收到，这条看起来不需要代码层面的排查。".into(),
        },
        IssueVerdict::AlreadyFixed => {
            "类似问题历史上有过修复，你遇到的可能是旧版本，或尚未升到修复版。".into()
        }
        IssueVerdict::Regression => {
            "现象像已知问题再现，存在回归可能，建议对照较新版本再验证。".into()
        }
        IssueVerdict::Spam | IssueVerdict::Advertisement => {
            "内容更像无关推广/垃圾信息，建议按项目规范处理。".into()
        }
        IssueVerdict::Unverified => "暂时还不能下定论，需要更多日志或复现信息。".into(),
    }
}

fn assemble_comment(
    decision: &IssueReviewDecision,
    narrative: &str,
    mentions_line: Option<&str>,
    technical: Option<&TechnicalVerification>,
) -> String {
    let mut md = String::new();
    md.push_str(BOT_COMMENT_MARKER);
    md.push_str("\n\n");
    if let Some(m) = mentions_line {
        if !m.is_empty() {
            md.push_str(m);
            md.push_str("\n\n");
        }
    }
    // 先剥实现黑话，再按证据白名单打掉幻觉 path:line / sha
    let body = strip_forbidden_phrases(narrative.trim());
    let body = ground_narrative(&body, decision, technical);
    md.push_str(&body);
    md.push('\n');
    if let Some(line) = misroute_line(decision) {
        md.push('\n');
        md.push_str(&line);
    }
    md
}

/// 错投提示：只在强信号下开口，且措辞是「看起来更像」而非「你提错了」。
/// 弱信号留给标签，不在公开评论里质疑反馈者。
fn misroute_line(decision: &IssueReviewDecision) -> Option<String> {
    if decision.misrouted_confidence < 0.8 || decision.misrouted_repos.is_empty() {
        return None;
    }
    let repos = decision
        .misrouted_repos
        .iter()
        .take(2)
        .map(|r| format!("`{r}`"))
        .collect::<Vec<_>>()
        .join("、");
    Some(format!(
        "另外，这个问题看起来更贴近 {repos}，在那边提可能会更快得到回应；如果就是本仓库的问题，忽略这句即可。\n"
    ))
}

/// 证据白名单：允许出现在评论里的 path:line 与 commit sha 前缀。
#[derive(Default)]
struct EvidenceAllowlist {
    /// path → 允许的精确行；空 set 表示该 path 任意行（少用）
    exact: std::collections::HashMap<String, std::collections::HashSet<u32>>,
    /// path → 允许的闭区间行范围（深挖函数体）
    ranges: std::collections::HashMap<String, Vec<(u32, u32)>>,
    /// 允许的 commit short sha（≥7 hex）
    commit_shas: std::collections::HashSet<String>,
    /// 允许出现的路径（无行号时也可引用路径）
    paths: std::collections::HashSet<String>,
}

fn build_allowlist(
    decision: &IssueReviewDecision,
    technical: Option<&TechnicalVerification>,
) -> EvidenceAllowlist {
    let mut a = EvidenceAllowlist::default();
    for h in &decision.code_hits {
        allow_exact(&mut a, &h.path, h.line);
    }
    if let Some(t) = technical {
        for h in &t.code_hits {
            allow_exact(&mut a, &h.path, h.line);
        }
        for h in &t.test_hits {
            allow_exact(&mut a, &h.path, h.line);
        }
        for d in &t.deep_dig {
            allow_exact(&mut a, &d.path, d.anchor_line);
            a.ranges
                .entry(d.path.clone())
                .or_default()
                .push((d.start_line, d.end_line));
            for c in &d.callers {
                allow_exact(&mut a, &c.path, c.line);
            }
            for c in &d.file_commits {
                if let Some(sha) = short_sha_token(c) {
                    a.commit_shas.insert(sha);
                }
            }
        }
        for c in &t.git_commits {
            if let Some(sha) = short_sha_token(c) {
                a.commit_shas.insert(sha);
            }
        }
    }
    for c in &decision.related_commits {
        if let Some(sha) = short_sha_token(c) {
            a.commit_shas.insert(sha);
        }
    }
    a
}

fn allow_exact(a: &mut EvidenceAllowlist, path: &str, line: u32) {
    if path.is_empty() || line == 0 {
        return;
    }
    a.paths.insert(path.to_string());
    a.exact.entry(path.to_string()).or_default().insert(line);
}

fn short_sha_token(line: &str) -> Option<String> {
    let token = line.split_whitespace().next()?;
    if token.len() >= 7 && token.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(token.chars().take(7).collect())
    } else {
        None
    }
}

impl EvidenceAllowlist {
    fn allows_path_line(&self, path: &str, line: u32) -> bool {
        let path = path.trim_matches('`').trim();
        if let Some(set) = self.exact.get(path) {
            if set.contains(&line) {
                return true;
            }
        }
        if let Some(ranges) = self.ranges.get(path) {
            if ranges.iter().any(|(s, e)| line >= *s && line <= *e) {
                return true;
            }
        }
        // 后缀匹配（模型有时省略 crates/ 前缀或只写文件名）
        for (p, set) in &self.exact {
            if path_matches(p, path) && set.contains(&line) {
                return true;
            }
        }
        for (p, ranges) in &self.ranges {
            if path_matches(p, path) && ranges.iter().any(|(s, e)| line >= *s && line <= *e) {
                return true;
            }
        }
        false
    }

    fn allows_sha(&self, sha: &str) -> bool {
        if self.commit_shas.is_empty() {
            // 无白名单时不校验 sha（避免误伤无验证路径）
            return true;
        }
        let s: String = sha.chars().take(7).collect();
        self.commit_shas
            .iter()
            .any(|a| s.starts_with(a) || a.starts_with(&s))
    }
}

fn path_matches(full: &str, cited: &str) -> bool {
    full == cited
        || full.ends_with(cited)
        || cited.ends_with(full)
        || full.ends_with(&format!("/{cited}"))
}

/// 去掉不在证据白名单中的 `path:line` 与未知 commit sha。
pub fn ground_narrative(
    text: &str,
    decision: &IssueReviewDecision,
    technical: Option<&TechnicalVerification>,
) -> String {
    let allow = build_allowlist(decision, technical);
    if allow.exact.is_empty() && allow.ranges.is_empty() && allow.commit_shas.is_empty() {
        return text.to_string();
    }

    let mut out = scrub_backtick_path_lines(text, &allow);
    if !allow.commit_shas.is_empty() {
        out = scrub_unknown_shas(&out, &allow);
    }
    while out.contains("  ") {
        out = out.replace("  ", " ");
    }
    out = out.replace(" 。", "。").replace(" ,", ",");
    out
}

/// 处理 `` `path:line` ``：不在白名单则整段删除。
fn scrub_backtick_path_lines(text: &str, allow: &EvidenceAllowlist) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        result.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('`') {
            let inner = &after[..end];
            // 支持 `path:line` 或 `path:line — note`：以第一个空白/破折号切 token
            let token = inner
                .split(|c: char| c.is_whitespace() || c == '—' || c == '|')
                .next()
                .unwrap_or(inner)
                .trim();
            if let Some((path, line)) = parse_path_line(token) {
                if allow.allows_path_line(&path, line) {
                    result.push('`');
                    result.push_str(inner);
                    result.push('`');
                }
                // else: drop invented cite
            } else {
                // 非 path:line 的反引号保留（如 `deepseek-v4`）
                result.push('`');
                result.push_str(inner);
                result.push('`');
            }
            rest = &after[end + 1..];
        } else {
            result.push_str(&rest[start..]);
            rest = "";
            break;
        }
    }
    result.push_str(rest);
    result
}

fn parse_path_line(s: &str) -> Option<(String, u32)> {
    let s = s.trim().trim_end_matches(|c: char| {
        c == ',' || c == '.' || c == ';' || c == '）' || c == ')' || c == '。'
    });
    let (path, line_s) = s.rsplit_once(':')?;
    if !looks_like_code_path(path) {
        return None;
    }
    // line_s 必须纯数字（避免 `http://x` 之类）
    if line_s.is_empty() || !line_s.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let line: u32 = line_s.parse().ok()?;
    if line == 0 {
        return None;
    }
    Some((path.to_string(), line))
}

fn looks_like_code_path(path: &str) -> bool {
    if path.len() < 4 || path.contains(' ') || path.contains("://") {
        return false;
    }
    path.contains('/')
        || path.ends_with(".rs")
        || path.ends_with(".go")
        || path.ends_with(".ts")
        || path.ends_with(".js")
        || path.ends_with(".py")
        || path.ends_with(".java")
        || path.ends_with(".c")
        || path.ends_with(".cpp")
}

fn scrub_unknown_shas(text: &str, allow: &EvidenceAllowlist) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_hexdigit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_hexdigit() {
                i += 1;
            }
            let len = i - start;
            if (7..=12).contains(&len) {
                let before_ok = start == 0
                    || (!chars[start - 1].is_ascii_alphanumeric() && chars[start - 1] != '_');
                let after_ok =
                    i == chars.len() || (!chars[i].is_ascii_alphanumeric() && chars[i] != '_');
                let sha: String = chars[start..i].iter().collect();
                if before_ok && after_ok && !allow.allows_sha(&sha) {
                    continue;
                }
            }
            for c in &chars[start..i] {
                out.push(*c);
            }
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// 兜底清洗：实现细节 + 死板小标题。
fn strip_forbidden_phrases(s: &str) -> String {
    let mut out = s.to_string();
    for bad in [
        "本地仓库",
        "当前本地仓库",
        "当前仓库",
        "只读分析本地仓库",
        "只读分析",
        "只读检索",
        "git grep",
        "自动初筛",
        "源码命中明细",
        "（自动检索）",
        "不是最终结论",
        "不是猜测",
        // 内部置信度用语（中英）
        "置信度",
        "low confidence",
        "fairly confident",
        "moderately confident",
        "(low confidence)",
        "(fairly confident)",
        "(moderately confident)",
        // 死板段标题（保留正文）
        "**定位：**",
        "**原因：**",
        "**原因 / 机制说明：**",
        "**机制说明：**",
        "**机制 / 对照：**",
        "**临时缓解建议：**",
        "**建议：**",
        "**不建议：**",
        "**不建议做的事：**",
        "**还缺什么：**",
        "**还缺什么（导致现在判不清）：**",
        "**请补充：**",
        "**请这样补充后再继续：**",
        "**相关 Issue：**",
        "**相关线索：**",
        "**对照：**",
        "**线索：**",
        "**为何像重复：**",
        "**为何更像非缺陷：**",
        "### 1. 定位",
        "### 2. 原因 / 机制说明",
        "### 2. 原因/机制说明",
        "### 3. 临时缓解建议 / 不建议做的事",
        "### 3. 临时缓解建议",
        "#### 可以做的",
        "#### 不建议做的",
        "#### 还想了解的信息",
    ] {
        out = out.replace(bad, "");
    }
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out
}

fn type_zh(t: IssueType) -> &'static str {
    match t {
        IssueType::Bug => "缺陷",
        IssueType::FeatureRequest => "功能请求",
        IssueType::Question => "使用疑问",
        IssueType::Documentation => "文档",
        IssueType::Configuration => "配置",
        IssueType::Support => "支持",
        IssueType::Security => "安全",
        IssueType::Performance => "性能",
        IssueType::Compatibility => "兼容性",
        IssueType::Spam => "垃圾信息",
        IssueType::Advertisement => "广告",
        IssueType::Abuse => "滥用",
        IssueType::Unknown => "未分类",
    }
}

fn missing_zh(f: &str) -> String {
    match f {
        "actual_behavior" => "实际现象与完整报错".into(),
        "expected_behavior" => "期望的正确行为".into(),
        "reproduction_steps" => "可稳定复现的步骤".into(),
        "error_or_log" => "完整错误日志".into(),
        "environment" => "系统、应用版本、模型名、是否代理".into(),
        "affected_scope" => "受影响的版本或组件范围".into(),
        other => other.to_string(),
    }
}

fn clip(s: &str, n: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= n {
        return t.to_string();
    }
    // 尽量在句末断开，别留「…, a…」这种半句话
    let head: String = t.chars().take(n).collect();
    let cut = head
        .rfind(['.', '。', '!', '！', '?', '？', ';', '；'])
        .filter(|i| *i >= head.len() / 2);
    match cut {
        Some(i) => head[..=i].trim_end().to_string(),
        None => format!("{head}…"),
    }
}

/// 测试辅助：确保确定性说明不会泄漏内部 code。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::model::{
        CodeEvidence, DuplicateStatus, IssueReviewDecision, IssueType, IssueVerdict,
        NormalizedIssue,
    };

    fn dec() -> IssueReviewDecision {
        IssueReviewDecision {
            issue_number: 1236,
            primary_type: IssueType::Bug,
            type_confidence: 0.8,
            type_reasons: vec![],
            completeness_score: 0.7,
            missing_fields: vec!["reproduction_steps".into()],
            can_verify: true,
            spam_score: 0.0,
            advertisement_score: 0.0,
            abuse_score: 0.0,
            prompt_injection_score: 0.0,
            duplicate_status: DuplicateStatus::NotDuplicate,
            duplicate_confidence: 0.5,
            duplicate_of: None,
            duplicate_candidates: vec![],
            duplicate_evidence: vec![],
            verdict: IssueVerdict::LikelyBug,
            confidence: 0.72,
            reasons: vec!["error_language".into(), "code_hits=13".into()],
            suggested_labels: vec![],
            suggested_comment: String::new(),
            close_recommended: false,
            auto_action_allowed: false,
            needs_human_review: false,
            vector_used: false,
            vector_degraded: false,
            analyzer_version: "t".into(),
            technical_verdict: IssueVerdict::LikelyBug,
            technical_confidence: 0.72,
            technical_evidence: vec![],
            code_paths: vec![],
            code_hits: vec![CodeEvidence {
                path: "crates/x/src/retry.rs".into(),
                line: 188,
                snippet: "网络连接中断:远端关闭或重置了连接".into(),
            }],
            related_commits: vec!["abc fix retry".into()],
            fix_prs: vec![],
            verification_ran: true,
            issue_title: "频繁网络连接中断".into(),
            symptom_summary: "自动重连仍失败".into(),
            misrouted_repos: vec![],
            misrouted_confidence: 0.0,
        }
    }

    #[test]
    fn en_unverified_never_says_confidence() {
        let mut d = dec();
        d.verdict = IssueVerdict::Unverified;
        d.primary_type = IssueType::Documentation;
        d.confidence = 0.4;
        d.verification_ran = false;
        d.code_hits.clear();
        d.issue_title = "rewind 不能用".into();
        d.symptom_summary = "docs slash-commands".into();
        let n = NormalizedIssue {
            title: "rewind 不能用".into(),
            symptom: "docs".into(),
            ..Default::default()
        };
        let text = deterministic_narrative(
            &d,
            &n,
            "https://atomcode.atomgit.com/docs/zh/slash-commands.html",
            None,
        );
        let low = text.to_ascii_lowercase();
        assert!(
            !low.contains("confidence") && !low.contains("confident") && !text.contains("置信度"),
            "must not leak confidence jargon: {text}"
        );
        assert!(
            text.contains("Evidence")
                || text.contains("thin")
                || text.contains("docs")
                || text.contains("README")
                || text.contains("Hi"),
            "{text}"
        );
    }

    #[test]
    fn deterministic_is_user_facing() {
        let n = NormalizedIssue {
            title: "频繁网络连接中断".into(),
            symptom: "网络连接中断:远端关闭".into(),
            error_signatures: vec!["网络连接中断".into(), "error decoding".into()],
            ..Default::default()
        };
        let body = "报错 网络连接中断 deepseek-v4-flash";
        let text = deterministic_narrative(&dec(), &n, body, None);
        assert!(text.contains("谢谢") || text.contains("感谢") || text.contains("你好"));
        assert!(text.contains("retry.rs:188") || text.contains("retry.rs"));
        assert!(
            !text.contains("**已核实**")
                && !text.contains("**未证实**")
                && !text.contains("**可先做**")
                && !text.contains("**维护者可接着**"),
            "no rigid section headers: {text}"
        );
        assert!(
            text.contains("根因") || text.contains("钉不死"),
            "unconfirmed root cause in natural prose: {text}"
        );
        assert!(!text.contains("**定位：**"));
        assert!(!text.contains("**原因：**"));
        assert!(!text.contains("error_language"));
        assert!(!text.contains("本地仓库"));
        assert_eq!(reply_shape(&dec()), ReplyShape::FullDebug);
        let full = assemble_comment(&dec(), &text, None, None);
        assert!(full.contains(BOT_COMMENT_MARKER));
    }

    #[test]
    fn ground_strips_invented_path_and_sha() {
        let d = dec();
        let raw = "看 `crates/x/src/retry.rs:188` 和 `crates/fake/invented.rs:9`，\
                   提交 deadbeef123 与 abcdef0 都相关。";
        // related has "abc fix retry" — only real-looking sha from allowlist is none unless we set
        let mut d = d;
        d.related_commits = vec!["abcdef0123 fix retry path".into()];
        let out = ground_narrative(raw, &d, None);
        assert!(
            out.contains("retry.rs:188") || out.contains("`crates/x/src/retry.rs:188`"),
            "kept real cite: {out}"
        );
        assert!(
            !out.contains("invented.rs"),
            "must strip invented path: {out}"
        );
        assert!(!out.contains("deadbeef"), "must strip unknown sha: {out}");
        assert!(
            out.contains("abcdef0") || out.contains("abcdef0123"),
            "allowed sha kept: {out}"
        );
    }

    #[test]
    fn shape_adapts_by_verdict() {
        let mut d = dec();
        d.verdict = IssueVerdict::NeedsInfo;
        d.code_hits.clear();
        assert_eq!(reply_shape(&d), ReplyShape::NeedsInfo);
        let n = NormalizedIssue {
            symptom: "程序闪退了但是没有日志".into(),
            ..Default::default()
        };
        let t = deterministic_narrative(&d, &n, "闪退", None);
        assert!(t.contains("补") || t.contains("不够"));
        assert!(!t.contains("**定位：**"));

        d.verdict = IssueVerdict::Duplicate;
        d.duplicate_of = Some(12);
        assert_eq!(reply_shape(&d), ReplyShape::Duplicate);
        let t = deterministic_narrative(&d, &n, "闪退", None);
        assert!(t.contains("#12") || t.contains("像"));

        d.verdict = IssueVerdict::Spam;
        assert_eq!(reply_shape(&d), ReplyShape::SpamShort);
        let t = deterministic_narrative(&d, &n, "买课", None);
        assert!(t.len() < 400);

        d.verdict = IssueVerdict::Regression;
        d.related_commits = vec!["abc fix foo".into()];
        assert_eq!(reply_shape(&d), ReplyShape::Regression);
        let t = deterministic_narrative(&d, &n, "又出现了", None);
        assert!(t.contains("回归") || t.contains("版本"));
        assert!(!t.contains("**建议：**"));
    }

    /// 线上回归：atomgit#2「希望官方提供最佳实践」被判 documentation/Unverified，
    /// 回复却拼进了 Unverified 的「需要更多日志或复现信息」——文档诉求要日志是答非所问。
    fn doc_request() -> (IssueReviewDecision, NormalizedIssue, &'static str) {
        let mut d = dec();
        d.verdict = IssueVerdict::Unverified;
        d.primary_type = IssueType::Documentation;
        d.confidence = 0.4;
        d.type_confidence = 0.4;
        d.verification_ran = false;
        d.code_hits.clear();
        d.related_commits.clear();
        d.missing_fields.clear();
        d.issue_title = "docs：希望添加最佳实践".into();
        d.symptom_summary = "如题，希望官方提供一些最佳实践".into();
        let n = NormalizedIssue::default();
        (d, n, "如题，希望官方提供一些最佳实践")
    }

    #[test]
    fn doc_request_never_asks_for_logs_or_repro() {
        let (d, n, body) = doc_request();
        assert_eq!(reply_shape(&d), ReplyShape::NotABug);
        let text = deterministic_narrative(&d, &n, body, None);
        assert!(
            !text.contains("日志") && !text.contains("复现"),
            "documentation stance must not ask for logs/repro: {text}"
        );
        assert!(
            text.contains("文档"),
            "stance should name the documentation ask: {text}"
        );
    }

    #[test]
    fn doc_request_en_never_asks_for_logs_or_repro() {
        let (mut d, n, _) = doc_request();
        d.issue_title = "docs: please add best practices".into();
        d.symptom_summary = "please add best practices".into();
        let text = deterministic_narrative(&d, &n, "please add some best practices", None);
        let low = text.to_ascii_lowercase();
        assert!(
            !low.contains("log") && !low.contains("repro"),
            "documentation stance must not ask for logs/repro: {text}"
        );
    }

    /// 线上回归：回复把维护者视角的裁决（「不是代码缺陷」）直接发给了提问者。
    /// 对方从没说这是 bug，否定一个没人提出的主张既冒犯又零信息。
    #[test]
    fn doc_request_reads_as_intake_not_verdict() {
        let (d, n, body) = doc_request();
        let text = deterministic_narrative(&d, &n, body, None);
        assert!(
            !text.contains("不是代码缺陷") && !text.contains("更像"),
            "no verdict talk aimed at the reporter: {text}"
        );
        assert!(
            !text.contains("若文档不准"),
            "the ask is a missing doc, not a wrong one: {text}"
        );
        assert!(
            text.contains('？'),
            "should ask one concrete follow-up: {text}"
        );
    }

    #[test]
    fn feature_request_reads_as_intake_not_verdict() {
        let (mut d, n, _) = doc_request();
        d.primary_type = IssueType::FeatureRequest;
        d.verdict = IssueVerdict::NotABug;
        d.confidence = 0.65;
        d.issue_title = "希望增加批量导出".into();
        d.symptom_summary = "希望能一次导出多个文件".into();
        let text = deterministic_narrative(&d, &n, "希望增加批量导出功能", None);
        assert!(
            !text.contains("缺陷"),
            "no verdict talk aimed at the reporter: {text}"
        );
        assert!(text.contains("场景"), "should ask for the use case: {text}");
    }

    #[test]
    fn doc_request_en_reads_as_intake_not_verdict() {
        let (mut d, n, _) = doc_request();
        d.issue_title = "docs: please add best practices".into();
        d.symptom_summary = "please add best practices".into();
        let text = deterministic_narrative(&d, &n, "please add some best practices", None);
        let low = text.to_ascii_lowercase();
        assert!(
            !low.contains("not a code defect") && !low.contains("docs gap"),
            "no verdict talk aimed at the reporter: {text}"
        );
        assert!(
            text.contains('?'),
            "should ask one concrete follow-up: {text}"
        );
    }

    /// 借鉴需求文档 F-07：错投仓库要引导到正确位置。
    /// 只在强信号下开口，且只建议不断言——说错「你提错地方了」很伤人。
    #[test]
    fn strong_misroute_signal_points_at_the_target_repo() {
        let mut d = dec();
        d.misrouted_repos = vec!["other/engine".into()];
        d.misrouted_confidence = 0.8;
        let full = assemble_comment(&d, "正文。", None, None);
        assert!(full.contains("other/engine"), "{full}");
        assert!(
            !full.contains("提错") && !full.contains("错误的仓库"),
            "suggest, never accuse: {full}"
        );
    }

    #[test]
    fn weak_misroute_signal_stays_silent() {
        let mut d = dec();
        d.misrouted_repos = vec!["other/engine".into()];
        d.misrouted_confidence = 0.45;
        let full = assemble_comment(&d, "正文。", None, None);
        assert!(
            !full.contains("other/engine"),
            "weak signal is for labels only: {full}"
        );
    }

    #[test]
    fn short_body_is_not_echoed_back() {
        let (d, n, body) = doc_request();
        let text = deterministic_narrative(&d, &n, body, None);
        assert!(
            !text.contains("希望官方提供一些最佳实践"),
            "must not parrot the issue body back: {text}"
        );
    }

    /// 线上回归（atomcode #1257）：正文很长时，开场句照搬了原文一整段。
    /// 覆盖率判据只挡得住短正文的整段复读，长正文里搬一段照样是复读。
    #[test]
    fn long_body_paragraph_is_not_echoed_back() {
        let para = "AtomCode 已有 ACP (Agent Client Protocol) 协议实现，用于让外部客户端\
                    通过标准协议调用 AtomCode 作为编码代理。当前实现已支持核心会话生命周期";
        let body = format!(
            "{para}，但仍缺少若干能力。\n\n## 背景\n{}\n\n## 期望\n{}",
            "x".repeat(400),
            "y".repeat(400)
        );
        assert!(
            echoes_body(para, &body),
            "a verbatim paragraph lifted from a long body is still an echo"
        );
    }

    /// 线上回归（atomcode #746）：模板给的是安全专用建议，LLM 润色又把
    /// 「附上复现环境的日志或抓包」「在最新版本复现」加了回来。
    #[test]
    fn security_reports_are_never_llm_polished() {
        let mut d = dec();
        d.primary_type = IssueType::Security;
        d.verdict = IssueVerdict::LikelyBug;
        assert!(
            !should_polish_tips(&d),
            "security tips must stay deterministic"
        );

        d.primary_type = IssueType::Bug;
        assert!(
            should_polish_tips(&d),
            "ordinary defects still get polished"
        );
    }

    /// 线上回归（atomcode #1236）：相关 Issue 列表里混进了「贪吃蛇游戏 api 报错」。
    /// 列出来就等于在说「这条相关」，弱候选不该占这个位置。
    #[test]
    fn weak_duplicate_candidates_are_not_listed() {
        use crate::issue::model::DuplicateCandidate;
        let mut d = dec();
        d.verdict = IssueVerdict::Duplicate;
        d.duplicate_of = Some(321);
        d.duplicate_candidates = vec![
            DuplicateCandidate {
                issue_number: 725,
                title: "stream read error 连接中断".into(),
                score: 0.82,
                sources: vec!["fts".into()],
                error_signature: String::new(),
            },
            DuplicateCandidate {
                issue_number: 23,
                title: "生成的贪吃蛇游戏出现api不能访问报错".into(),
                score: 0.38,
                sources: vec!["vector".into()],
                error_signature: String::new(),
            },
        ];
        let n = NormalizedIssue::default();
        let text = deterministic_narrative(&d, &n, "网络连接中断，远端关闭", None);
        assert!(text.contains("#725"), "strong candidate kept: {text}");
        assert!(
            !text.contains("#23"),
            "weak candidate must be dropped: {text}"
        );
    }

    /// 转人工必须真的把人叫来，且不能顺带把没把握的结论也说出去。
    #[test]
    fn triage_handoff_names_the_owner_and_states_no_verdict() {
        let mut d = dec();
        d.verdict = IssueVerdict::Unverified;
        d.primary_type = IssueType::Documentation;
        d.confidence = 0.4;
        d.issue_title = "更新的App连接功能好像还没生效吧".into();
        let out = render_triage_handoff(&d, "更新的App连接功能好像还没生效吧", &["alice".into()]);
        assert!(
            out.contains(BOT_COMMENT_MARKER),
            "must stay idempotent: {out}"
        );
        assert!(out.contains("@alice"), "{out}");
        for leak in ["文档", "缺陷", "重复", "把握不足", "UNVERIFIED"] {
            assert!(
                !out.contains(leak),
                "leaked a verdict it isn't sure of: {out}"
            );
        }
        assert!(out.len() < 200, "hand-off should be short: {out}");
    }

    #[test]
    fn triage_handoff_follows_issue_language() {
        let mut d = dec();
        d.issue_title = "rewind does not work".into();
        let out = render_triage_handoff(&d, "clicking rewind does nothing", &["bob".into()]);
        assert!(out.contains("@bob") && out.contains("asked"), "{out}");
    }

    /// 线上回归（atomcode #1196）：一张图片链接里的 90+ 个拉丁字符，
    /// 压过正文的 18 个汉字，让中文 Issue 收到了英文回复。
    #[test]
    fn urls_do_not_flip_reply_language() {
        let title = "更新的App连接功能好像还没生效吧";
        let body = "/APP 是这样的 ![image.png](https://raw.atomgit.com/user-images/assets/\
                    9709354/57e291f7-a04b-4200-8614-1e65f30fb0e2/image.png 'image.png')";
        assert_eq!(detect_reply_lang(title, body), ReplyLang::Zh);

        let mut d = dec();
        d.issue_title = title.into();
        let out = render_triage_handoff(&d, body, &["alice".into()]);
        assert!(
            out.contains("已经请"),
            "hand-off must follow the issue: {out}"
        );
    }

    /// 但真英文 Issue 还是英文——剥链接不等于一律判中文。
    #[test]
    fn english_issue_still_gets_english() {
        let title = "rewind does not work";
        let body = "Clicking rewind does nothing at all. I expected the session to roll back \
                    to the previous checkpoint, but the UI just stays where it was.";
        assert_eq!(detect_reply_lang(title, body), ReplyLang::En);
    }

    #[test]
    fn echo_detection_only_fires_on_full_restatement() {
        assert!(echoes_body(
            "如题，希望官方提供一些最佳实践",
            "如题，希望官方提供一些最佳实践。"
        ));
        assert!(!echoes_body(
            "自动重连仍失败",
            "报错 网络连接中断:远端关闭或重置了连接，重试 3 次后自动重连仍失败，\
             使用 deepseek-v4-flash，代理已关闭，版本 5.0.3。"
        ));
        assert!(!echoes_body("自动重连仍失败", ""));
    }

    #[test]
    fn strip_removes_implementation_jargon() {
        let s = strip_forbidden_phrases("这是只读分析本地仓库的结果，用 git grep 命中，不是猜测。");
        assert!(!s.contains("本地仓库"));
        assert!(!s.contains("git grep"));
        assert!(!s.contains("只读"));
    }
}

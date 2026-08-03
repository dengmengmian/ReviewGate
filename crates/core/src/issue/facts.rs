//! 评论用「可核验事实包」：只从 decision / deep_dig 抽字面证据，不靠 LLM 发明机制。

use super::explain::{clip, missing_en, missing_zh};
use super::model::{IssueReviewDecision, IssueType, IssueVerdict, NormalizedIssue};
use super::verify::{DeepDigBlock, TechnicalVerification};

/// 输出「修复方向」的最低置信度（decision.confidence 与 technical_confidence 取高）。
pub const FIX_DIRECTION_MIN_CONF: f32 = 0.85;

/// 一条已核实或未证实项（双语文案在渲染层选）。
#[derive(Debug, Clone)]
pub struct FactLine {
    pub zh: String,
    pub en: String,
}

#[derive(Debug, Clone, Default)]
pub struct FactPack {
    /// 可核验：错误↔代码、深挖函数体字面逻辑、文件绑定 commit。
    pub verified: Vec<FactLine>,
    /// 明确标成未证实：根因、缺字段、推测。
    pub unconfirmed: Vec<FactLine>,
    /// 给报告人的可操作建议（确定性默认；可被 LLM 润色替换）。
    pub user_tips: Vec<FactLine>,
    /// 给维护者的下一步（仅指向证据中的符号/文件）。
    pub maintainer_tips: Vec<FactLine>,
    /// 高置信缺陷时的修复方向（基于证据，非补丁）。
    pub fix_directions: Vec<FactLine>,
}

/// 从审查结果构建事实包。
pub fn build_fact_pack(
    decision: &IssueReviewDecision,
    normalized: &NormalizedIssue,
    technical: Option<&TechnicalVerification>,
) -> FactPack {
    let mut pack = FactPack::default();

    // --- 已核实：错误签名 ↔ code_hits ---
    let sigs: Vec<&str> = normalized
        .error_signatures
        .iter()
        .map(|s| s.as_str())
        .filter(|s| s.len() >= 3)
        .collect();

    let mut mapped = 0usize;
    for h in decision.code_hits.iter().take(8) {
        if is_doc_path(&h.path) {
            continue;
        }
        let snip = h.snippet.trim();
        let matched_sig = sigs.iter().find(|s| snip.contains(*s));
        if let Some(sig) = matched_sig {
            pack.verified.push(FactLine {
                zh: format!(
                    "报错片段「{}」与 `{}:{}` 源码一致：`{}`",
                    clip(sig, 40),
                    h.path,
                    h.line,
                    clip(snip, 90)
                ),
                en: format!(
                    "Error fragment “{}” matches `{}:{}`: `{}`",
                    clip(sig, 40),
                    h.path,
                    h.line,
                    clip(snip, 90)
                ),
            });
            mapped += 1;
            if mapped >= 3 {
                break;
            }
        }
    }
    // 无签名命中时仍列出前 2 条源码锚点（snippet 字面）。
    // 但要挑得住看：`fmt::Write as _,` 这种 import 残片、半截字符串字面量
    // 对维护者零价值，贴出来只会降低整条回复的可信度。
    if mapped == 0 {
        for h in decision
            .code_hits
            .iter()
            .filter(|h| !is_doc_path(&h.path))
            .filter(|h| is_meaningful_anchor(h.snippet.trim()))
            .take(2)
        {
            pack.verified.push(FactLine {
                zh: format!(
                    "相关源码锚点 `{}:{}`：`{}`",
                    h.path,
                    h.line,
                    clip(h.snippet.trim(), 90)
                ),
                en: format!(
                    "Code anchor `{}:{}`: `{}`",
                    h.path,
                    h.line,
                    clip(h.snippet.trim(), 90)
                ),
            });
        }
    }

    // 错误映射已占用的路径（机制/提交优先贴这些文件）
    let mapped_paths: Vec<String> = pack
        .verified
        .iter()
        .filter_map(|f| extract_backtick_path(&f.zh).or_else(|| extract_backtick_path(&f.en)))
        .collect();

    // --- 已核实：深挖机制（仅当函数体里真有错误签名，或路径已在错误映射中）---
    if let Some(t) = technical {
        let mut mech_n = 0usize;
        // 先：上下文含错误签名的 dig
        let mut digs: Vec<&DeepDigBlock> = t.deep_dig.iter().collect();
        digs.sort_by_key(|d| {
            let has_sig = sigs.iter().any(|s| d.context.contains(s));
            let path_mapped = mapped_paths
                .iter()
                .any(|p| d.path.contains(p) || p.contains(&d.path));
            // 0 = best
            if has_sig {
                0u8
            } else if path_mapped {
                1
            } else {
                3
            }
        });
        let issue_blob = format!("{} {}", decision.issue_title, decision.symptom_summary);
        let networkish = is_network_provider_copy(&issue_blob, &decision.code_hits);
        for d in digs {
            if !dig_block_thematic(d, &sigs, networkish, &mapped_paths) {
                continue;
            }
            // 弱符号（crate 名/库名）跳过
            if d.symbol
                .as_deref()
                .is_some_and(|s| WEAK_SYMBOLS.contains(&s))
            {
                continue;
            }
            if let Some(m) = mechanism_from_dig(d, &sigs) {
                // 深挖块常常引用的就是错误映射已经贴过的那一行，重复说一遍没有信息量
                if pack.verified.iter().any(|v| same_cited_line(&v.zh, &m.zh)) {
                    continue;
                }
                pack.verified.push(m);
                mech_n += 1;
                if mech_n >= 2 {
                    break;
                }
            }
        }
        // 文件绑定 commit：只挂与主题相关的 fix 类提交，禁止 first() 把无关 fix(auth) 写成已核实
        let mut commit_n = 0usize;
        for d in &t.deep_dig {
            let relevant = mapped_paths.iter().any(|p| path_soft_eq(p, &d.path))
                || sigs.iter().any(|s| d.context.contains(s));
            if !relevant {
                continue;
            }
            let Some(c) = pick_thematic_file_commit(decision, d) else {
                continue;
            };
            pack.verified.push(FactLine {
                zh: format!("文件 `{}` 上与本问题相关的提交：`{}`", d.path, c),
                en: format!("Thematic commit on `{}`: `{}`", d.path, c),
            });
            commit_n += 1;
            if commit_n >= 2 {
                break;
            }
        }
    } else {
        for c in decision.related_commits.iter().take(2) {
            pack.verified.push(FactLine {
                zh: format!("相关提交（关键词命中，非文件绑定）：`{}`", c),
                en: format!("Related commit (keyword hit, not file-bound): `{}`", c),
            });
        }
    }

    // 硬上限：已核实不宜刷屏（维护者扫一眼）
    pack.verified.truncate(6);

    // --- 未证实 ---
    if matches!(
        decision.verdict,
        IssueVerdict::LikelyBug
            | IssueVerdict::ConfirmedBug
            | IssueVerdict::Regression
            | IssueVerdict::Unverified
    ) {
        let blob = format!("{} {}", decision.issue_title, decision.symptom_summary);
        // 口令/配对「连接」≠ provider 网络断连；禁止裸「连接」触发上游限流/代理套话
        let networkish = is_network_provider_copy(&blob, &decision.code_hits);
        let has_code = !decision.code_hits.is_empty();
        if !has_code {
            // 一行代码都没对上时，不能说「只对上了相关代码区域」——那是自相矛盾。
            pack.unconfirmed.push(FactLine {
                zh: "根因未证实：还没有定位到对应的实现位置，无法断定触发条件。".into(),
                en: "Root cause not confirmed: no implementation site located yet, so the trigger cannot be pinned down.".into(),
            });
        } else {
            pack.unconfirmed.push(if networkish {
                FactLine {
                    zh: "根因未证实：目前只能定位到错误包装/抛出相关代码，不能证明是上游限流、网关策略、本机代理还是应用逻辑缺陷。".into(),
                    en: "Root cause not confirmed: we only located wrap/throw sites, not whether upstream limits, gateway, proxy, or app logic is at fault.".into(),
                }
            } else {
                FactLine {
                    zh: "根因未证实：目前只对上了相关代码区域，还不能断定具体缺陷分支或触发条件。".into(),
                    en: "Root cause not confirmed: related code regions were located, but the exact failing branch/trigger is not proven.".into(),
                }
            });
        }
    }
    for f in decision.missing_fields.iter().take(3) {
        pack.unconfirmed.push(FactLine {
            zh: format!("还缺{}", missing_zh(f)),
            en: format!("Still need: {}", missing_en(f)),
        });
    }
    if decision.code_hits.is_empty() && technical.map(|t| t.code_hits.is_empty()).unwrap_or(true) {
        pack.unconfirmed.push(FactLine {
            zh: "无源码锚点：尚未在仓库中检索到与报错一致的实现位置。".into(),
            en: "No code anchors: no matching implementation found for the reported error.".into(),
        });
    }

    // --- 用户可做 ---
    pack.user_tips = default_user_tips(decision, normalized);

    // --- 维护者 ---
    pack.maintainer_tips = default_maintainer_tips(decision, technical, &mapped_paths);

    // --- 高置信修复方向（非补丁，只谈基于证据的改法思路）---
    pack.fix_directions = build_fix_directions(decision, technical, &sigs);

    // 没有代码证据时要讲清楚这一点，但用人话——内部裁决名和「启发式」是给日志看的
    if pack.verified.is_empty() {
        pack.verified.push(FactLine {
            zh: "目前的判断只来自 Issue 文本，还没有对上代码层面的证据。".into(),
            en: "So far this reading comes from the issue text alone, with no code-level evidence."
                .into(),
        });
    }

    pack
}

/// dig 块是否与 Issue 主题相关（网络 issue 禁止 cc_hooks/codeintel 等）。
pub fn dig_block_thematic(
    d: &DeepDigBlock,
    error_sigs: &[&str],
    networkish: bool,
    mapped_paths: &[String],
) -> bool {
    let path = d.path.to_ascii_lowercase();
    let ctx = d.context.to_ascii_lowercase();
    let sym = d.symbol.as_deref().unwrap_or("").to_ascii_lowercase();

    // oauth / hooks / codeintel：仅当 dig 上下文本身是鉴权语义才放行；网络与其它主题一律硬拒
    let auth_ctx = ctx.contains("oauth")
        || ctx.contains("start_login")
        || ctx.contains("device code")
        || ctx.contains("口令")
        || path.contains("auth_token");
    if (path.contains("cc_hooks")
        || path.contains("codeintel")
        || path.contains("oauth")
        || path.contains("gateway_crypto")
        || sym.contains("honors_matcher")
        || sym.contains("hook_config")
        || (sym.contains("matcher") && !path.contains("provider")))
        && (networkish || !auth_ctx)
    {
        return false;
    }

    let has_strong_sig = error_sigs.iter().any(|s| {
        crate::issue::verify::error_sig_matches_text(s, &d.context)
            && !s.eq_ignore_ascii_case("error:")
            && !s.eq_ignore_ascii_case("exception:")
    });
    let path_mapped = mapped_paths.iter().any(|p| path_soft_eq(p, &d.path));

    if networkish {
        let path_transport = path.contains("provider")
            || path.contains("retry")
            || path.contains("openai")
            || path.contains("http")
            || path.contains("stream")
            || path.contains("reqwest");
        // 网络：必须是传输层路径，且上下文有实质错误签名（或已映射路径 + 强签名）
        if !path_transport {
            return false;
        }
        return has_strong_sig
            || (path_mapped
                && (ctx.contains("网络")
                    || ctx.contains("decoding")
                    || ctx.contains("connection reset")
                    || ctx.contains("重连")
                    || ctx.contains("中断")));
    }

    has_strong_sig || path_mapped
}

/// 从深挖块抽出机制句：只引用 context 里真实出现的行。
pub fn mechanism_from_dig(d: &DeepDigBlock, error_sigs: &[&str]) -> Option<FactLine> {
    let lines = parse_context_lines(&d.context);
    if lines.is_empty() {
        return None;
    }

    // 两遍：先只取含**实质**错误签名的行，再退到传输/失败文案（禁止 is_error:false）
    let mut picked: Vec<(u32, String)> = Vec::new();
    for (ln, text) in &lines {
        let t = text.trim();
        if t.is_empty() {
            continue;
        }
        let tl = t.to_ascii_lowercase();
        // 跳过 is_error / has_error 布尔字段
        if tl.contains("is_error") || tl.contains("has_error") {
            continue;
        }
        if error_sigs
            .iter()
            .any(|s| crate::issue::verify::error_sig_matches_text(s, t))
        {
            // 仅 "error:" 泛标记且行里无 error: 字面 → 已在 matches 处理
            picked.push((*ln, t.to_string()));
        }
        if picked.len() >= 2 {
            break;
        }
    }
    if picked.is_empty() {
        for (ln, text) in &lines {
            let t = text.trim();
            if t.is_empty() || t.starts_with("//") || t.starts_with("/*") || t.starts_with("///") {
                continue;
            }
            let tl = t.to_ascii_lowercase();
            if tl.contains("is_error") || tl.contains("has_error") {
                continue;
            }
            let hit_err = t.contains("失败")
                || t.contains("中断")
                || t.contains("format!")
                || t.contains("bail!")
                || t.contains("return Err")
                || tl.contains("error decoding")
                || tl.contains("connection reset")
                || t.contains("重连")
                || t.contains("远端");
            if hit_err && t.len() > 12 {
                picked.push((*ln, t.to_string()));
            }
            if picked.len() >= 2 {
                break;
            }
        }
    }
    // 无实质错误行则不编机制句（禁止锚点 is_error:false 充数）
    if picked.is_empty() {
        return None;
    }

    let sym = d.symbol.as_deref().filter(|s| s.len() >= 2).unwrap_or("?");
    let cite = picked
        .iter()
        .map(|(ln, t)| format!("`{}:{}` `{}`", d.path, ln, clip(t, 100)))
        .collect::<Vec<_>>()
        .join("；");

    // 只有真解析到包围函数才敢说「函数 X 中」；否则那只是锚点附近的一个标识符
    // （Go 的 `var (...)` 块就会这样），说成函数体是编造。
    Some(if d.symbol_is_fn {
        FactLine {
            zh: format!(
                "函数 `{sym}`（`{}:{}–{}`）中可见：{}",
                d.path, d.start_line, d.end_line, cite
            ),
            en: format!(
                "In `{sym}` (`{}:{}–{}`): {}",
                d.path, d.start_line, d.end_line, cite
            ),
        }
    } else {
        FactLine {
            zh: format!("`{}:{}` 附近可见：{}", d.path, d.anchor_line, cite),
            en: format!("Near `{}:{}`: {}", d.path, d.anchor_line, cite),
        }
    })
}

/// 解析 `  123| code` 形式的深挖上下文。
fn parse_context_lines(context: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for line in context.lines() {
        let line = line.trim_end();
        // "  12| foo" or "12| foo"
        if let Some((num, rest)) = line.split_once('|') {
            let num = num.trim();
            if let Ok(ln) = num.parse::<u32>() {
                out.push((ln, rest.trim_start().to_string()));
            }
        }
    }
    out
}

fn default_user_tips(d: &IssueReviewDecision, n: &NormalizedIssue) -> Vec<FactLine> {
    let mut tips = Vec::new();
    // 安全报告没有「报错日志」，也不是版本用错了——问它要这些是答非所问。
    if d.primary_type == IssueType::Security {
        tips.push(FactLine {
            zh: "补充受影响的版本或组件范围，以及触发所需的前置条件。".into(),
            en: "Add the affected versions/components and the preconditions needed to trigger it."
                .into(),
        });
        tips.push(FactLine {
            zh: "若已有可利用细节或 PoC，建议走项目的私密安全渠道，不要贴在公开 Issue 里。".into(),
            en: "If you have exploit details or a PoC, use the project's private security channel rather than this public issue.".into(),
        });
        return tips;
    }
    match d.verdict {
        IssueVerdict::LikelyBug | IssueVerdict::ConfirmedBug | IssueVerdict::Unverified => {
            // 报错已经贴了就别再要；已经对上源码行的缺陷也不是"版本用错了"。
            if n.error_signatures.is_empty() {
                tips.push(FactLine {
                    zh: "贴完整报错（含详情链）和开始变频繁的大致时间。".into(),
                    en: "Paste the full error (including detail chain) and roughly when it got frequent.".into(),
                });
            }
            if d.code_hits.is_empty() {
                tips.push(FactLine {
                    zh: "确认应用版本；可先升到最新版再试同一操作。".into(),
                    en: "Confirm app version; try the same action on the latest release.".into(),
                });
            }
            if d.code_hits
                .iter()
                .any(|h| h.path.contains("retry") || h.snippet.contains("重试"))
            {
                tips.push(FactLine {
                    zh: "临时换模型/端点对比：区分单端点问题与全局网络问题。".into(),
                    en: "Temporarily switch model/endpoint to separate endpoint vs global network issues.".into(),
                });
            }
        }
        IssueVerdict::NeedsInfo => {
            tips.push(FactLine {
                zh: "按上面还缺的信息补全后，编辑本 Issue 或再评论即可。".into(),
                en: "Fill in the missing details above, then edit this issue or reply.".into(),
            });
        }
        IssueVerdict::Duplicate => {
            tips.push(FactLine {
                zh: "先对照上面提到的相关 Issue；若不是同一问题请说明版本/平台差异。".into(),
                en: "Check the related issues mentioned above; if different, note version/platform deltas.".into(),
            });
        }
        IssueVerdict::AlreadyFixed => {
            tips.push(FactLine {
                zh: "升到含相关提交的版本后再复现一次。".into(),
                en: "Upgrade to a release that includes the related commits and retest.".into(),
            });
        }
        IssueVerdict::Regression => {
            tips.push(FactLine {
                zh: "在曾正常的版本与当前版本各复现一次，附版本号。".into(),
                en: "Reproduce once on a known-good version and once on current; include version numbers.".into(),
            });
        }
        _ => {
            tips.push(FactLine {
                zh: "补充实际 vs 期望与环境信息。".into(),
                en: "Add actual vs expected behavior and environment details.".into(),
            });
        }
    }
    if n.environment
        .as_object()
        .map(|o| o.is_empty())
        .unwrap_or(true)
        && !matches!(d.verdict, IssueVerdict::Spam | IssueVerdict::Advertisement)
    {
        tips.push(FactLine {
            // 「模型名 / 是否走代理」是 AI 客户端专用字段，对一个 CLI 库或后端库文不对题。
            zh: "补充操作系统与你使用的版本。".into(),
            en: "Add your OS and the version you are running.".into(),
        });
    }
    tips.truncate(4);
    tips
}

/// 是否满足「输出修复方向」门槛。
pub fn should_emit_fix_directions(decision: &IssueReviewDecision) -> bool {
    let conf = decision.confidence.max(decision.technical_confidence);
    conf >= FIX_DIRECTION_MIN_CONF
        && matches!(
            decision.verdict,
            IssueVerdict::LikelyBug | IssueVerdict::ConfirmedBug | IssueVerdict::Regression
        )
        && (decision.verification_ran || !decision.code_hits.is_empty())
}

/// 从深挖/命中生成修复方向。无足够证据时返回空（宁可不出，不编补丁）。
pub fn build_fix_directions(
    decision: &IssueReviewDecision,
    technical: Option<&TechnicalVerification>,
    error_sigs: &[&str],
) -> Vec<FactLine> {
    if !should_emit_fix_directions(decision) {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let issue_blob = format!("{} {}", decision.issue_title, decision.symptom_summary);
    let networkish = is_network_provider_copy(&issue_blob, &decision.code_hits);

    // 1) 深挖块：按与错误签名/热路径相关度排序
    if let Some(t) = technical {
        let mut digs: Vec<&DeepDigBlock> = t.deep_dig.iter().collect();
        digs.sort_by_key(|d| {
            let has_sig = error_sigs
                .iter()
                .any(|s| !s.is_empty() && d.context.contains(s));
            let hot = d.path.contains("retry")
                || d.path.contains("stream")
                || d.path.contains("openai")
                || d.path.contains("provider")
                || d.path.contains("http");
            if has_sig {
                0u8
            } else if hot {
                1
            } else {
                3
            }
        });

        for d in digs {
            if out.len() >= 3 {
                break;
            }
            if d.symbol
                .as_deref()
                .is_some_and(|s| WEAK_SYMBOLS.contains(&s))
            {
                continue;
            }
            let Some(dir) = fix_direction_from_dig(d, error_sigs, networkish) else {
                continue;
            };
            let key = format!("{}:{}", d.path, d.anchor_line);
            if !seen.insert(key) {
                continue;
            }
            out.push(dir);
        }

        // 文件历史上与主题相关的 fix 提交 → 「对齐/回修」方向（走 pick_thematic，禁 oauth/tls 冒充）
        if out.len() < 3 {
            for d in &t.deep_dig {
                let Some(c) = pick_thematic_file_commit(decision, d) else {
                    continue;
                };
                let key = format!("commit:{c}");
                if !seen.insert(key) {
                    continue;
                }
                out.push(FactLine {
                    zh: format!(
                        "对齐 `{}` 上已有提交 `{}`：核对是否已覆盖当前失败路径，未覆盖则补等价处理（勿重复造轮子）。",
                        d.path, c
                    ),
                    en: format!(
                        "Align with existing commit `{c}` on `{}`: check whether it covers this failure path; if not, add equivalent handling.",
                        d.path
                    ),
                });
                if out.len() >= 3 {
                    break;
                }
            }
        }
    }

    // 2) 无深挖时：用错误映射锚点给「包装点」级方向
    if out.is_empty() {
        for h in decision
            .code_hits
            .iter()
            .filter(|h| !is_doc_path(&h.path))
            .take(4)
        {
            let snip = h.snippet.trim();
            // 必须精确对上报错文本。曾用 `contains("error")` 兜底，结果把
            // `errNoWatchKeys = errors.New(...)` 这种**错误常量声明**也当成了抛出点。
            if !error_sigs.iter().any(|s| !s.is_empty() && snip.contains(s)) {
                continue;
            }
            let key = format!("{}:{}", h.path, h.line);
            if !seen.insert(key) {
                continue;
            }
            out.push(if is_error_declaration(snip) {
                FactLine {
                    zh: format!(
                        "`{}:{}` 定义的是一个固定错误值，调用方只能按值比对、拿不到触发场景；若要区分不同触发条件，考虑在抛出处包装上下文再返回。",
                        h.path, h.line
                    ),
                    en: format!(
                        "`{}:{}` declares a sentinel error value, so callers can only compare by identity and lose the triggering context; consider wrapping it with context at the throw site.",
                        h.path, h.line
                    ),
                }
            } else {
                FactLine {
                    zh: format!(
                        "在包装/抛出点 `{}:{}` 保留完整底层 error chain，再决定是否重试或上抛；避免只留笼统文案导致无法区分传输层与业务错误。",
                        h.path, h.line
                    ),
                    en: format!(
                        "At wrap/throw site `{}:{}`, keep the full underlying error chain before retry/propagate; avoid a single generic message that hides transport vs business failures.",
                        h.path, h.line
                    ),
                }
            });
            if out.len() >= 2 {
                break;
            }
        }
    }

    out.truncate(3);
    out
}

/// 两条证据是否在贴同一段代码——是的话第二条就是复读。
///
/// 按行号比不行：深挖块的引用里还带着窗口起点行，行号对不上；真正重复的是
/// 反引号里那段代码本身。
fn same_cited_line(a: &str, b: &str) -> bool {
    fn snippets(s: &str) -> Vec<String> {
        s.split('`')
            .map(str::trim)
            .filter(|p| p.chars().count() >= 12 && (p.contains('=') || p.contains('(')))
            .map(|p| p.chars().filter(|c| !c.is_whitespace()).collect())
            .collect()
    }
    let (sa, sb) = (snippets(a), snippets(b));
    !sa.is_empty()
        && !sb.is_empty()
        && sb.iter().any(|x| {
            sa.iter()
                .any(|y| y.contains(x.as_str()) || x.contains(y.as_str()))
        })
}

/// 这一行贴给维护者是否有信息量。过滤 import 残片、孤立标点、半截字符串。
fn is_meaningful_anchor(snippet: &str) -> bool {
    let l = snippet.trim();
    // CI / 构建配置不是缺陷现场
    if l.contains("needs.*.result") || l.starts_with("if: ") || l.starts_with("- uses:") {
        return false;
    }
    if l.chars().count() < 12 {
        return false;
    }
    let lower = l.to_ascii_lowercase();
    // import / use 残片
    if lower.starts_with("use ") || lower.starts_with("import ") || lower.starts_with("from ") {
        return false;
    }
    if l.ends_with(" as _,") || l.ends_with("as _;") {
        return false;
    }
    // 只是一段散文字符串字面量（引号开头且没有任何代码结构）
    let looks_like_prose_literal = (l.starts_with('"') || l.starts_with("r\""))
        && !l.contains("=")
        && !l.contains("=>")
        && !l.contains("fn ")
        && l.matches(' ').count() >= 5;
    if looks_like_prose_literal {
        return false;
    }
    // 必须含一点代码结构，纯注释/纯文本不算锚点
    l.contains('(')
        || l.contains('=')
        || l.contains("::")
        || l.contains("def ")
        || l.contains("fn ")
        || l.contains("func ")
        || l.contains("class ")
}

/// 是否是「错误常量/哨兵值声明」而非抛出点。对这种行说「保留 error chain」文不对题。
fn is_error_declaration(snippet: &str) -> bool {
    let s = snippet.trim();
    let assigned = s.contains('=') && !s.contains("==");
    assigned
        && (s.contains("errors.New(")
            || s.contains("fmt.Errorf(")
            || s.contains("thiserror")
            || s.contains("Error::new("))
}

fn fix_direction_from_dig(
    d: &DeepDigBlock,
    error_sigs: &[&str],
    networkish: bool,
) -> Option<FactLine> {
    let path = &d.path;
    let sym = d
        .symbol
        .as_deref()
        .filter(|s| s.len() >= 2 && !WEAK_SYMBOLS.contains(s));
    // 弱符号 / dump 辅助函数不给出修复方向（易误导）
    if let Some(s) = sym {
        let sl = s.to_ascii_lowercase();
        if sl.contains("dump") || sl.contains("wire") || sl.contains("writes_body") {
            return None;
        }
    }
    let has_sig = error_sigs
        .iter()
        .any(|s| !s.is_empty() && d.context.contains(s));
    let path_l = path.to_ascii_lowercase();
    let where_ = match sym {
        Some(s) => format!("`{s}`（`{}:{}`）", path, d.anchor_line),
        None => format!("`{}:{}`", path, d.anchor_line),
    };

    // 路径优先（避免 openai_compat 上下文里出现 retry 字样就套错模板）
    if path_l.contains("/retry") || path_l.ends_with("retry.rs") || path_l.contains("retry/") {
        return Some(FactLine {
            zh: format!(
                "在 {where_} 收紧重试策略：仅对可归类的瞬态 IO/连接错误重试，并把底层 chain 透出到日志；{}。",
                if has_sig {
                    "用户看到的统一中断文案建议附带可区分的原因码"
                } else {
                    "避免所有失败都走同一包装文案"
                }
            ),
            en: format!(
                "At {where_}, tighten retry: only retry classifiable transient IO/connection errors and surface the underlying chain in logs; avoid one generic wrap for every failure."
            ),
        });
    }

    // 流式读建议同样是传输层专用的。`_make_text_stream` 这种名字里带 stream 的
    // 普通函数也会命中，不加主题门槛就会给一个类型注解缺陷讲 SSE 半包。
    if networkish
        && (path_l.contains("openai")
            || path_l.contains("stream")
            || path_l.contains("sse")
            || (sym.is_some_and(|s| {
                let l = s.to_ascii_lowercase();
                l.contains("stream") || l.contains("chat")
            })))
    {
        return Some(FactLine {
            zh: format!(
                "在 {where_} 核对流式读结束条件（半包、无 finish_reason / [DONE]、对端 RST）：断流应记为传输层失败并带上已读字节/状态，而不是一律收成业务错误。"
            ),
            en: format!(
                "At {where_}, verify stream end conditions (partial frames, missing finish_reason/[DONE], peer RST): treat disconnect as transport failure with bytes/status, not a generic business error."
            ),
        });
    }

    // 「超时 / 连接重置 / 协议解码 / 鉴权」是传输层专用建议。只有错误签名命中
    // 还不够——一个 shell 补全缺陷也会命中签名，给它讲网络重试是文不对题。
    if networkish && (has_sig || path_l.contains("/provider/") || path_l.contains("http")) {
        return Some(FactLine {
            zh: format!(
                "在 {where_} 把错误分类做细：区分超时、连接重置、协议/解码失败与鉴权失败，各自决定重试、退避或直接失败（不要共用一个模糊文案出口）。"
            ),
            en: format!(
                "At {where_}, refine error classes (timeout, connection reset, protocol/decode, auth) and choose retry/backoff/fail per class—avoid one vague message path."
            ),
        });
    }

    None
}

fn default_maintainer_tips(
    d: &IssueReviewDecision,
    technical: Option<&TechnicalVerification>,
    anchored_paths: &[String],
) -> Vec<FactLine> {
    let mut tips = Vec::new();
    if let Some(t) = technical {
        let issue_blob = format!("{} {}", d.issue_title, d.symptom_summary);
        let networkish = is_network_provider_copy(&issue_blob, &d.code_hits);
        // 用「已和错误签名对上的路径」，不是全部 code_hits——真实检索会命中十几个
        // 文件，拿它当相关性判据等于没判，随后 take(3) 又退化成随机抓。
        // 只认「已和错误签名对上的路径」。真实检索会命中十几个文件，
        // 拿全部 code_hits 当相关性判据等于没判，take(3) 就退化成随机抓；
        // 一个锚点都没有时宁可不给方向。
        let mapped: Vec<String> = anchored_paths.to_vec();
        // 网络 issue：只指向 provider/retry dig；cc_hooks/codeintel 硬拒
        let mut digs: Vec<&DeepDigBlock> = t
            .deep_dig
            .iter()
            .filter(|dig| {
                dig_block_thematic(
                    dig,
                    &["网络连接中断", "error decoding", "connection reset"],
                    networkish,
                    &mapped,
                )
            })
            .collect();
        digs.sort_by_key(|dig| {
            let hot = dig.context.contains("网络")
                || dig.context.contains("error decoding")
                || dig.context.contains("connection reset")
                || dig.path.contains("retry")
                || dig.path.contains("openai");
            if hot {
                0u8
            } else {
                2
            }
        });
        for dig in digs.into_iter().take(3) {
            if dig
                .symbol
                .as_deref()
                .is_some_and(|s| WEAK_SYMBOLS.contains(&s))
            {
                continue;
            }
            // 只推荐与本 Issue 真有关联的符号：路径已经在检索命中里，或上下文对上了命中片段。
            // 排序用的 hot 判定是网络/AI 客户端的硬编码词，换个领域的项目全部并列，
            // take(3) 就退化成随机抓——给维护者三个不相干的符号比不给更糟。
            let relevant = mapped.iter().any(|p| path_soft_eq(p, &dig.path))
                || d.code_hits.iter().any(|h| {
                    let sn = h.snippet.trim();
                    sn.len() >= 8 && dig.context.contains(sn)
                });
            if !relevant {
                continue;
            }
            if let Some(sym) = dig.symbol.as_deref().filter(|s| s.len() >= 2) {
                tips.push(FactLine {
                    zh: format!(
                        "从 `{sym}`（`{}:{}`）沿调用方往上追触发条件与错误是否被改写。",
                        dig.path, dig.anchor_line
                    ),
                    en: format!(
                        "From `{sym}` (`{}:{}`) walk callers for trigger conditions and error rewriting.",
                        dig.path, dig.anchor_line
                    ),
                });
            }
            if !dig.file_commits.is_empty()
                && (dig.path.contains("retry")
                    || dig.path.contains("openai")
                    || dig.path.contains("provider"))
            {
                tips.push(FactLine {
                    zh: format!("对照 `{}` 上最近提交是否改动超时/重试/流式读。", dig.path),
                    en: format!(
                        "Diff recent commits on `{}` for timeout/retry/stream-read changes.",
                        dig.path
                    ),
                });
            }
            if tips.len() >= 3 {
                break;
            }
        }
    }
    if tips.is_empty() && !d.code_hits.is_empty() {
        let h = &d.code_hits[0];
        tips.push(FactLine {
            zh: format!("从 `{}:{}` 沿调用方往上找触发条件。", h.path, h.line),
            en: format!(
                "Walk callers from `{}:{}` for trigger conditions.",
                h.path, h.line
            ),
        });
    }
    tips.truncate(3);
    tips
}

/// 渲染事实向评论：内容仍分「能钉死的 / 还不行的 / 可做的」，但用自然段落，无固定小标题。
/// `user_tips_override`：若提供则覆盖默认 user_tips（LLM 润色后的纯文案行）。
pub fn render_fact_comment(
    zh: bool,
    decision: &IssueReviewDecision,
    pack: &FactPack,
    user_tips_override: Option<&[String]>,
) -> String {
    let mut md = String::new();
    let show_maint = !pack.maintainer_tips.is_empty()
        && matches!(
            decision.verdict,
            IssueVerdict::LikelyBug
                | IssueVerdict::ConfirmedBug
                | IssueVerdict::Regression
                | IssueVerdict::Unverified
        );

    if zh {
        md.push_str("你好，谢谢反馈。\n\n");
        let symptom = clip(&pick_symptom_short(decision), 140);
        if !symptom.is_empty() {
            md.push_str(&format!("你这边主要是：{symptom}\n\n"));
        }

        // 已核实 → 自然引入 + 列表（列表是证据，不是表单标题）
        if pack.verified.len() == 1 {
            md.push_str(&pack.verified[0].zh);
            md.push_str("\n\n");
        } else if !pack.verified.is_empty() {
            md.push_str("对照代码，目前能对上的有这些：\n");
            for f in &pack.verified {
                md.push_str(&format!("- {}\n", f.zh));
            }
            md.push('\n');
        }

        // 未证实 → 一段话 / 短列表，不用「未证实」标题
        if !pack.unconfirmed.is_empty() {
            let root: Vec<_> = pack
                .unconfirmed
                .iter()
                .filter(|f| f.zh.contains("根因"))
                .collect();
            let rest: Vec<_> = pack
                .unconfirmed
                .iter()
                .filter(|f| !f.zh.contains("根因"))
                .collect();
            if let Some(r) = root.first() {
                // 去掉可能的「根因未证实：」硬前缀，改成口语
                let body =
                    r.zh.trim_start_matches("根因未证实：")
                        .trim_start_matches("根因未证实")
                        .trim();
                md.push_str("不过根因还钉不死——");
                md.push_str(body);
                if !body.ends_with('。') && !body.ends_with('.') {
                    md.push('。');
                }
                md.push('\n');
            }
            if !rest.is_empty() {
                md.push_str("另外");
                if rest.len() == 1 {
                    md.push_str(&rest[0].zh);
                    if !rest[0].zh.ends_with('。') {
                        md.push('。');
                    }
                    md.push('\n');
                } else {
                    md.push_str("还缺一些信息：\n");
                    for f in rest {
                        md.push_str(&format!("- {}\n", f.zh));
                    }
                }
            }
            md.push('\n');
        }

        // 可先做
        let tips: Vec<String> = if let Some(ov) = user_tips_override {
            ov.iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .take(5)
                .collect()
        } else {
            pack.user_tips.iter().map(|t| t.zh.clone()).collect()
        };
        if !tips.is_empty() {
            md.push_str("你可以先试：\n");
            for (i, t) in tips.iter().enumerate() {
                md.push_str(&format!("{}. {}\n", i + 1, t));
            }
        }

        if show_maint {
            md.push_str("\n若还要往下挖，比较值得看的方向：\n");
            for t in &pack.maintainer_tips {
                md.push_str(&format!("- {}\n", t.zh));
            }
        }

        if !pack.fix_directions.is_empty() {
            md.push_str("\n若按上面这些代码动手修，可以优先考虑：\n");
            for t in &pack.fix_directions {
                md.push_str(&format!("- {}\n", t.zh));
            }
        }
    } else {
        md.push_str("Hi, thanks for the report.\n\n");
        let symptom = clip(&pick_symptom_short(decision), 140);
        if !symptom.is_empty() {
            md.push_str(&format!("What you’re seeing: {symptom}\n\n"));
        }

        if pack.verified.len() == 1 {
            md.push_str(&pack.verified[0].en);
            md.push_str("\n\n");
        } else if !pack.verified.is_empty() {
            md.push_str("What we can pin down in the code:\n");
            for f in &pack.verified {
                md.push_str(&format!("- {}\n", f.en));
            }
            md.push('\n');
        }

        if !pack.unconfirmed.is_empty() {
            let root: Vec<_> = pack
                .unconfirmed
                .iter()
                .filter(|f| f.en.to_ascii_lowercase().contains("root cause"))
                .collect();
            let rest: Vec<_> = pack
                .unconfirmed
                .iter()
                .filter(|f| !f.en.to_ascii_lowercase().contains("root cause"))
                .collect();
            if let Some(r) = root.first() {
                let body =
                    r.en.trim_start_matches("Root cause not confirmed: ")
                        .trim_start_matches("Root cause not confirmed:")
                        .trim();
                md.push_str("We can’t confirm the root cause yet — ");
                md.push_str(body);
                if !body.ends_with('.') {
                    md.push('.');
                }
                md.push('\n');
            }
            if !rest.is_empty() {
                if rest.len() == 1 {
                    md.push_str(&rest[0].en);
                    if !rest[0].en.ends_with('.') {
                        md.push('.');
                    }
                    md.push('\n');
                } else {
                    md.push_str("Still missing:\n");
                    for f in rest {
                        md.push_str(&format!("- {}\n", f.en));
                    }
                }
            }
            md.push('\n');
        }

        let tips: Vec<String> = if let Some(ov) = user_tips_override {
            ov.iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .take(5)
                .collect()
        } else {
            pack.user_tips.iter().map(|t| t.en.clone()).collect()
        };
        if !tips.is_empty() {
            md.push_str("You could try:\n");
            for (i, t) in tips.iter().enumerate() {
                md.push_str(&format!("{}. {}\n", i + 1, t));
            }
        }

        if show_maint {
            md.push_str("\nIf you dig further, useful directions:\n");
            for t in &pack.maintainer_tips {
                md.push_str(&format!("- {}\n", t.en));
            }
        }

        if !pack.fix_directions.is_empty() {
            md.push_str("\nIf you start a fix from the code above, consider:\n");
            for t in &pack.fix_directions {
                md.push_str(&format!("- {}\n", t.en));
            }
        }
    }
    md
}

fn pick_symptom_short(decision: &IssueReviewDecision) -> String {
    let raw = if !decision.symptom_summary.trim().is_empty() {
        decision.symptom_summary.trim().to_string()
    } else {
        decision.issue_title.clone()
    };
    // Issue 正文里偶发字面量 `\n`，展示前压成空格
    raw.replace("\\n", " ").replace("\\r", " ")
}

/// 从 deep dig 的文件历史上挑与 Issue 主题相关的提交；没有则不展示。
fn pick_thematic_file_commit(
    decision: &IssueReviewDecision,
    d: &super::verify::DeepDigBlock,
) -> Option<String> {
    let blob = format!(
        "{} {} {}",
        decision.issue_title, decision.symptom_summary, d.context
    );
    // 与 unconfirmed 同一套：禁止「口令连接」被当成 provider 网络断连
    let networkish = is_network_provider_copy(&blob, &decision.code_hits);
    let blob = blob.to_ascii_lowercase();
    for c in &d.file_commits {
        let cl = c.to_ascii_lowercase();
        let is_fix = cl.contains("fix") || cl.contains("hotfix") || cl.contains("resolv");
        if !is_fix {
            continue;
        }
        // 网络断连：只展示连接重置/重试/流式类，禁止 oauth/tls 登录修复冒充
        if networkish {
            if cl.contains("oauth")
                || cl.contains("fix(auth)")
                || cl.contains("fix(tls)")
                || (cl.contains("tls") && cl.contains("atomgit"))
                || d.path.contains("oauth")
            {
                continue;
            }
            let ok = cl.contains("disconnect")
                || cl.contains("reconnect")
                || cl.contains("connection reset")
                || cl.contains("连接重置")
                || cl.contains("连接中断")
                || cl.contains("重连")
                || cl.contains("中断")
                || cl.contains("stale")
                || cl.contains("keep-alive")
                || cl.contains("keepalive")
                || cl.contains("badrecordmac")
                || cl.contains("timedout")
                || cl.contains("os error 10054")
                || cl.contains("os error 110")
                || (cl.contains("retry")
                    && (cl.contains("reset")
                        || cl.contains("timeout")
                        || cl.contains("transient")
                        || cl.contains("transport")
                        || cl.contains("rate limit")
                        || cl.contains("连接")
                        || cl.contains("provider")));
            if ok {
                return Some(c.clone());
            }
            continue;
        }
        // 非网络：主题关键词
        let keys = [
            "stream",
            "disconnect",
            "timeout",
            "network",
            "connection",
            "连接",
            "重连",
            "中断",
            "skill",
            "dedup",
            "sync",
            "webui",
            "pair",
            "口令",
            "approval",
        ];
        let hit = keys.iter().any(|k| {
            let k = k.to_ascii_lowercase();
            if !k.is_ascii() {
                blob.contains(&k) && cl.contains(&k)
            } else {
                token_in(&blob, &k) && token_in(&cl, &k)
            }
        });
        if hit {
            return Some(c.clone());
        }
    }
    None
}

fn token_in(hay: &str, needle: &str) -> bool {
    hay.split(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .any(|t| t == needle)
}

/// Provider/传输层断连文案与 commit 主题判定。
/// **不得**仅因「连接」（口令连接/App 远程）就当成网络断连。
pub fn is_network_provider_copy(
    blob: &str,
    code_hits: &[crate::issue::model::CodeEvidence],
) -> bool {
    let l = blob.to_ascii_lowercase();
    let pairingish = blob.contains("口令")
        || blob.contains("配对")
        || l.contains("pairing")
        || l.contains("/app")
        || (blob.contains("远程") && (blob.contains("电脑") || blob.contains("设备")));
    // 明确的 pairing/口令场景：除非同时有网络/decoding 信号，否则不算 network provider
    if pairingish
        && !blob.contains("网络")
        && !blob.contains("decoding")
        && !l.contains("error decoding")
        && !blob.contains("断连")
        && !blob.contains("重连")
        && !l.contains("connection reset")
    {
        return false;
    }
    if blob.contains("网络")
        || blob.contains("断连")
        || blob.contains("重连")
        || blob.contains("中断")
        || l.contains("decoding")
        || l.contains("disconnect")
        || l.contains("timeout")
        || l.contains("connection reset")
        || (l.contains("connection")
            && (l.contains("reset") || l.contains("network") || blob.contains("中断")))
    {
        return true;
    }
    // 仅当 hit 本身是 provider 传输错误文案，才用 path 兜底（避免 pairing 误挂 retry 套话）
    code_hits.iter().any(|h| {
        let sn = h.snippet.to_ascii_lowercase();
        (h.path.contains("retry") || h.path.contains("provider") || h.path.contains("openai"))
            && (sn.contains("网络")
                || sn.contains("decoding")
                || sn.contains("connection reset")
                || sn.contains("中断")
                || sn.contains("重连"))
    })
}

fn is_doc_path(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l.ends_with(".md") || l.contains("/docs/")
}

const WEAK_SYMBOLS: &[&str] = &[
    "thiserror",
    "Error",
    "Result",
    "Option",
    "String",
    "format",
    "clone",
    "into",
    "from",
    "new",
    "default",
    "main",
    "test",
];

fn path_soft_eq(a: &str, b: &str) -> bool {
    a == b || a.ends_with(b) || b.ends_with(a) || a.contains(b) || b.contains(a)
}

/// 从 `` `path:line` `` 或 `` `path` `` 抽路径。
fn extract_backtick_path(s: &str) -> Option<String> {
    let start = s.find('`')?;
    let rest = &s[start + 1..];
    let end = rest.find('`')?;
    let inner = &rest[..end];
    let path = inner.split(':').next()?.trim();
    if path.contains('/') || path.ends_with(".rs") {
        Some(path.to_string())
    } else {
        None
    }
}

/// 是否应用结构化事实评论（缺陷/回归等需要硬证据的形态）。
pub fn use_fact_structure(decision: &IssueReviewDecision) -> bool {
    matches!(
        decision.verdict,
        IssueVerdict::LikelyBug
            | IssueVerdict::ConfirmedBug
            | IssueVerdict::Regression
            | IssueVerdict::AlreadyFixed
            | IssueVerdict::Unverified
    ) && (decision.verification_ran
        || !decision.code_hits.is_empty()
        || matches!(
            decision.primary_type,
            IssueType::Bug
                | IssueType::Security
                | IssueType::Performance
                | IssueType::Compatibility
        ))
}

#[cfg(test)]
mod tests {

    /// `missing_zh` 在 explain.rs 和 facts.rs 各有一份拷贝，而且**已经分叉**：
    /// explain 那份认识 `affected_scope`，facts 这份不认识，于是中文用户会收到
    /// 「还缺 affected_scope」——内部字段名直接发出去了。这正是 facts.rs 顶上
    /// 那句注释警告过的问题，只是换了个字段又犯一次。合并成一份实现。
    #[test]
    fn missing_field_wording_is_human_readable_for_every_known_field() {
        // completeness.rs 会产出的全部字段
        for f in [
            "actual_behavior",
            "expected_behavior",
            "reproduction_steps",
            "error_or_log",
            "environment",
            "affected_scope",
        ] {
            let zh = missing_zh(f);
            let en = missing_en(f);
            assert_ne!(zh, f, "中文措辞不能是内部字段名: {f}");
            assert!(
                !zh.contains('_'),
                "中文措辞漏了 {f}，原样吐出了字段名: {zh}"
            );
            assert!(!en.contains('_'), "英文措辞漏了 {f}: {en}");
        }
    }

    /// 线上回归（alibaba/arthas 第二轮全量 triage）：`clip` 曾在 explain.rs 和
    /// facts.rs 各有一份逐字拷贝，字节切片的 panic 只修了前者，第二轮就崩在了后者
    /// （#560 之后整批中止）。现在两处共用一个实现——这个用例守的是"别再复制回去"。
    #[test]
    fn clip_is_shared_and_handles_chinese_punctuation() {
        let body = "启动时报错，无法 attach。请问如何排查？".repeat(20);
        for n in 1..=200 {
            let _ = clip(&body, n); // 不 panic 即通过
        }
        assert_eq!(clip("短句。", 50), "短句。");
    }
    use super::*;
    use crate::issue::model::{CodeEvidence, DuplicateStatus, IssueType, IssueVerdict};
    use crate::issue::verify::{CodeHit, DeepDigBlock, InvestigationPlan, TechnicalVerification};

    /// 线上回归（AtomGit new_review/go-redis #3）：证据里第二条
    /// 「osscluster.go:38 附近可见：osscluster.go:39 ...」和第一条讲的是同一行，纯复读。
    #[test]
    fn evidence_does_not_repeat_the_same_line() {
        assert!(same_cited_line(
            "报错片段「x」与 `osscluster.go:39` 源码一致：`errWatchCrosslot = ...`",
            "`osscluster.go:38` 附近可见：`osscluster.go:39` `errWatchCrosslot = ...`"
        ));
        assert!(!same_cited_line(
            "报错片段「x」与 `osscluster.go:39` 源码一致",
            "函数 `foo`（`pool/conn.go:120`）中可见：`pool/conn.go:125` `...`"
        ));
    }

    /// 线上回归（AtomGit new_review/go-redis #3）：把 `errNoWatchKeys = errors.New(...)`
    /// 这个**错误常量声明**说成「包装/抛出点，保留完整 error chain」——文不对题，
    /// 而且它匹配上只是因为兜底条件 `contains("error")` 太松。
    #[test]
    fn error_declaration_is_not_called_a_throw_site() {
        let mut d = base_decision();
        d.verdict = IssueVerdict::LikelyBug;
        d.confidence = 0.9;
        d.technical_confidence = 0.9;
        d.code_hits = vec![
            CodeEvidence {
                path: "osscluster.go".into(),
                line: 39,
                snippet: "errWatchCrosslot  = errors.New(\"redis: Watch requires all keys to be in the same slot\")".into(),
            },
            CodeEvidence {
                path: "osscluster.go".into(),
                line: 38,
                snippet: "errNoWatchKeys    = errors.New(\"redis: Watch requires at least one key\")".into(),
            },
        ];
        let n = NormalizedIssue {
            error_signatures: vec!["redis: Watch requires all keys to be in the same slot".into()],
            ..Default::default()
        };
        let pack = build_fact_pack(&d, &n, None);
        let blob = pack
            .fix_directions
            .iter()
            .map(|f| f.zh.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !blob.contains("包装/抛出点"),
            "a sentinel declaration is not a throw site: {blob}"
        );
        assert!(
            !blob.contains("osscluster.go:38"),
            "38 never matched the reported error; only 39 did: {blob}"
        );
    }

    /// 线上回归（AtomGit new_review/go-redis #1）：报错已经贴在正文里、
    /// 而且已经精确对上了源码行，回复却还在问「贴完整报错」「升到最新版再试」。
    #[test]
    fn tips_do_not_ask_for_what_is_already_there() {
        let mut d = base_decision();
        d.verdict = IssueVerdict::LikelyBug;
        d.code_hits = vec![CodeEvidence {
            path: "osscluster.go".into(),
            line: 39,
            snippet: "errWatchCrosslot = errors.New(\"redis: Watch requires all keys\")".into(),
        }];
        let n = NormalizedIssue {
            error_signatures: vec!["redis: Watch requires all keys to be in the same slot".into()],
            ..Default::default()
        };
        let tips = default_user_tips(&d, &n);
        let blob = tips
            .iter()
            .map(|f| f.zh.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !blob.contains("贴完整报错"),
            "error is already provided: {blob}"
        );
        assert!(
            !blob.contains("最新版"),
            "an anchored code defect is not a version problem: {blob}"
        );
    }

    /// 但真没贴报错时还是要问。
    #[test]
    fn tips_still_ask_when_nothing_was_provided() {
        let mut d = base_decision();
        d.verdict = IssueVerdict::LikelyBug;
        d.code_hits.clear();
        let tips = default_user_tips(&d, &NormalizedIssue::default());
        let blob = tips
            .iter()
            .map(|f| f.zh.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(blob.contains("报错"), "{blob}");
    }

    /// 线上回归（AtomGit new_review/go-redis #3）：锚点落在 Go 的 `var (...)` 块里，
    /// 不属于任何函数，兜底抽了个邻近标识符，评论却说成「函数 errNoWatchKeys 中可见」。
    #[test]
    fn window_fallback_does_not_claim_a_function() {
        let dig = DeepDigBlock {
            path: "osscluster.go".into(),
            anchor_line: 39,
            symbol: Some("errNoWatchKeys".into()),
            symbol_is_fn: false,
            start_line: 11,
            end_line: 67,
            context: "39| errWatchCrosslot = errors.New(\"redis: Watch requires all keys\")".into(),
            ..Default::default()
        };
        let f = mechanism_from_dig(&dig, &["redis: Watch requires all keys"]).expect("line");
        assert!(
            !f.zh.contains("函数"),
            "a var block is not a function: {}",
            f.zh
        );
        assert!(f.zh.contains("osscluster.go:39"), "{}", f.zh);
    }

    /// 线上回归（AtomGit new_review/go-redis #1）：给一个 Watch/slot 的 Issue
    /// 推荐了 `happened` / `subscribing` / `BitOpAnd` 三个毫不相干的符号。
    /// 排序的 hot 判定全是网络/AI 客户端硬编码词，换个领域项目就并列，随后 take(3) 变随机抓。
    #[test]
    fn maintainer_tips_skip_unrelated_symbols() {
        let mut d = base_decision();
        d.verdict = IssueVerdict::LikelyBug;
        d.issue_title = "[Bug] Watch 跨 slot 报错".into();
        // 真实检索的 code_hits 横跨十几个文件，只有一条真正对上了错误签名
        d.code_hits = vec![
            CodeEvidence {
                path: "osscluster.go".into(),
                line: 39,
                snippet: "errWatchCrosslot = errors.New(\"redis: Watch requires all keys\")".into(),
            },
            CodeEvidence {
                path: "bitmap_commands.go".into(),
                line: 12,
                snippet: "func BitOpAnd() {}".into(),
            },
            CodeEvidence {
                path: "array_commands.go".into(),
                line: 204,
                snippet: "// happened".into(),
            },
        ];
        let t = TechnicalVerification {
            enabled: true,
            deep_dig: vec![
                DeepDigBlock {
                    path: "bitmap_commands.go".into(),
                    anchor_line: 12,
                    symbol: Some("BitOpAnd".into()),
                    symbol_is_fn: true,
                    context: "12| func BitOpAnd() {}".into(),
                    ..Default::default()
                },
                DeepDigBlock {
                    path: "osscluster.go".into(),
                    anchor_line: 39,
                    symbol: Some("errWatchCrosslot".into()),
                    symbol_is_fn: true,
                    context: "39| errWatchCrosslot = errors.New(...)".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let anchored = vec!["osscluster.go".to_string()];
        let tips = default_maintainer_tips(&d, Some(&t), &anchored);
        let blob = tips
            .iter()
            .map(|f| f.zh.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !blob.contains("BitOpAnd"),
            "must not point at unrelated symbols: {blob}"
        );
    }

    /// 线上回归（atomcode #800）：同一条评论里先说「还没有对上代码层面的证据」，
    /// 紧接着又说「只对上了相关代码区域」——没有代码命中时后半句根本不成立。
    #[test]
    fn no_code_hits_means_no_claim_of_matched_code_regions() {
        let mut d = base_decision();
        d.primary_type = IssueType::Security;
        d.verdict = IssueVerdict::LikelyBug;
        d.issue_title = "[共创大赛] SessionId反序列化绕过路径穿越过滤".into();
        d.symptom_summary = "SessionId 未做过滤，可被构造恶意会话文件实现目录穿越".into();
        d.code_hits.clear();
        d.related_commits.clear();
        let n = NormalizedIssue::default();
        let pack = build_fact_pack(&d, &n, None);
        let body = render_fact_comment(true, &d, &pack, None);
        assert!(
            !body.contains("只对上了相关代码区域"),
            "cannot claim matched code regions with zero code hits: {body}"
        );
    }

    /// 线上回归（atomcode #800 路径穿越漏洞）：安全报告被要求
    /// 「贴完整报错」「升到最新版再试」——漏洞报告没有报错，也不是版本用错了。
    #[test]
    fn security_report_is_not_asked_for_logs_or_upgrades() {
        let mut d = base_decision();
        d.primary_type = IssueType::Security;
        d.verdict = IssueVerdict::LikelyBug;
        d.issue_title = "[共创大赛] SessionId反序列化绕过路径穿越过滤".into();
        d.symptom_summary = "反序列化路径绕过了过滤，可目录穿越".into();
        d.code_hits.clear();
        let n = NormalizedIssue::default();
        let pack = build_fact_pack(&d, &n, None);
        let body = render_fact_comment(true, &d, &pack, None);
        assert!(
            !body.contains("最新版") && !body.contains("升到"),
            "a vulnerability report is not a version problem: {body}"
        );
        assert!(
            !body.contains("贴完整报错") && !body.contains("变频繁"),
            "a vulnerability report has no error log: {body}"
        );
    }

    /// 线上回归（atomcode #1252）：内部裁决名和实现黑话被原样发给了反馈者。
    #[test]
    fn internal_verdict_names_never_reach_the_reporter() {
        let mut d = base_decision();
        d.verdict = IssueVerdict::LikelyBug;
        d.code_hits.clear();
        let n = NormalizedIssue::default();
        let pack = build_fact_pack(&d, &n, None);
        let body = render_fact_comment(true, &d, &pack, None);
        for jargon in [
            "LIKELY_BUG",
            "CONFIRMED_BUG",
            "UNVERIFIED",
            "启发式",
            "无强制代码证据",
        ] {
            assert!(!body.contains(jargon), "leaked `{jargon}`: {body}");
        }
    }

    fn base_decision() -> IssueReviewDecision {
        IssueReviewDecision {
            issue_number: 1,
            primary_type: IssueType::Bug,
            type_confidence: 0.8,
            type_reasons: vec![],
            completeness_score: 0.8,
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
            confidence: 0.8,
            reasons: vec![],
            suggested_labels: vec![],
            suggested_comment: String::new(),
            close_recommended: false,
            auto_action_allowed: false,
            needs_human_review: false,
            vector_used: false,
            vector_degraded: false,
            analyzer_version: "t".into(),
            technical_verdict: IssueVerdict::LikelyBug,
            technical_confidence: 0.8,
            technical_evidence: vec![],
            code_paths: vec![],
            code_hits: vec![CodeEvidence {
                path: "src/retry.rs".into(),
                line: 10,
                snippet: "网络连接中断:远端关闭或重置了连接".into(),
            }],
            related_commits: vec![],
            fix_prs: vec![],
            verification_ran: true,
            issue_title: "网络中断".into(),
            symptom_summary: "频繁网络连接中断".into(),
            misrouted_repos: vec![],
            misrouted_confidence: 0.0,
        }
    }

    #[test]
    fn fact_pack_maps_error_and_marks_root_cause_unconfirmed() {
        let d = base_decision();
        let n = NormalizedIssue {
            error_signatures: vec!["网络连接中断".into()],
            ..Default::default()
        };
        let tech = TechnicalVerification {
            enabled: true,
            deep_dig_ran: true,
            deep_dig: vec![DeepDigBlock {
                path: "src/retry.rs".into(),
                anchor_line: 10,
                symbol: Some("wrap_network_error".into()),
                symbol_is_fn: true,
                start_line: 5,
                end_line: 15,
                context: "   8| pub fn wrap_network_error(err: &str) -> String {\n  10|     format!(\"网络连接中断:远端关闭或重置了连接 ({err})\")\n  12| }\n".into(),
                callers: vec![CodeHit {
                    path: "src/provider.rs".into(),
                    line: 4,
                    snippet: "retry_loop(3)".into(),
                    source: "caller_grep".into(),
                }],
                file_commits: vec!["abc1234 fix retry message".into()],
                notes: vec![],
            }],
            plan: InvestigationPlan::default(),
            code_hits: vec![],
            git_commits: vec!["abc1234 fix retry message".into()],
            ..Default::default()
        };
        let pack = build_fact_pack(&d, &n, Some(&tech));
        assert!(
            pack.verified
                .iter()
                .any(|f| f.zh.contains("src/retry.rs:10")),
            "verified should map error: {:?}",
            pack.verified
        );
        assert!(
            pack.verified
                .iter()
                .any(|f| f.zh.contains("wrap_network_error") && f.zh.contains("网络连接中断")),
            "mechanism from dig body: {:?}",
            pack.verified
        );
        assert!(
            pack.unconfirmed.iter().any(|f| f.zh.contains("根因未证实")),
            "must mark root cause unconfirmed: {:?}",
            pack.unconfirmed
        );
        let body = render_fact_comment(true, &d, &pack, None);
        // 自然段落，不要固定死板小标题
        assert!(!body.contains("**已核实**"));
        assert!(!body.contains("**未证实**"));
        assert!(!body.contains("**可先做**"));
        assert!(!body.contains("**维护者可接着**"));
        assert!(
            body.contains("对上") || body.contains("retry") || body.contains("wrap_network"),
            "should still present verified anchors: {body}"
        );
        assert!(
            body.contains("根因") || body.contains("钉不死"),
            "should soft-mark unconfirmed root cause: {body}"
        );
        assert!(
            body.contains("你可以先试") || body.contains("先试"),
            "natural tips lead-in: {body}"
        );
        assert!(
            body.contains("往下挖") || body.contains("wrap_network") || body.contains("调用"),
            "maintainer dig without rigid header: {body}"
        );
        assert!(!body.contains("invented.rs"));
    }

    #[test]
    fn mechanism_only_uses_context_lines() {
        let dig = DeepDigBlock {
            path: "a.rs".into(),
            anchor_line: 3,
            symbol: Some("foo".into()),
            symbol_is_fn: true,
            start_line: 1,
            end_line: 5,
            context: "   2| let x = 1;\n   3| return Err(\"boom-unique\");\n".into(),
            callers: vec![],
            file_commits: vec![],
            notes: vec![],
        };
        let m = mechanism_from_dig(&dig, &["boom-unique"]).unwrap();
        assert!(m.zh.contains("boom-unique"));
        assert!(m.zh.contains("a.rs:3"));
        assert!(!m.zh.contains("never-in-file"));
    }

    #[test]
    fn high_confidence_bug_emits_fix_directions_from_dig() {
        let mut d = base_decision();
        d.verdict = IssueVerdict::LikelyBug;
        d.confidence = 0.88;
        d.technical_confidence = 0.88;
        d.verification_ran = true;
        let n = NormalizedIssue {
            error_signatures: vec!["网络连接中断".into()],
            ..Default::default()
        };
        let tech = TechnicalVerification {
            enabled: true,
            deep_dig_ran: true,
            deep_dig: vec![DeepDigBlock {
                path: "src/provider/retry.rs".into(),
                anchor_line: 188,
                symbol: Some("wrap_network_error".into()),
                symbol_is_fn: true,
                start_line: 170,
                end_line: 200,
                context: " 188| format!(\"网络连接中断:远端关闭\")\n 190| // retry loop\n".into(),
                callers: vec![],
                file_commits: vec!["ab12cd3 fix retry on stream reset".into()],
                notes: vec![],
            }],
            plan: InvestigationPlan::default(),
            code_hits: vec![],
            git_commits: vec![],
            confidence: 0.88,
            technical_verdict: IssueVerdict::LikelyBug,
            ..Default::default()
        };
        assert!(should_emit_fix_directions(&d));
        let pack = build_fact_pack(&d, &n, Some(&tech));
        assert!(
            !pack.fix_directions.is_empty(),
            "expected fix directions: {:?}",
            pack.fix_directions
        );
        assert!(
            pack.fix_directions
                .iter()
                .any(|f| f.zh.contains("retry.rs") || f.zh.contains("wrap_network_error")),
            "must cite real anchors: {:?}",
            pack.fix_directions
        );
        let body = render_fact_comment(true, &d, &pack, None);
        assert!(
            body.contains("动手修") || body.contains("优先考虑"),
            "natural lead-in for fix directions: {body}"
        );
        assert!(
            !body.contains("置信度") && !body.contains("confidence"),
            "must not leak internal confidence jargon: {body}"
        );
        assert!(!body.contains("**修复方向**"));
    }

    #[test]
    fn low_confidence_skips_fix_directions() {
        let mut d = base_decision();
        d.confidence = 0.5;
        d.technical_confidence = 0.5;
        d.verdict = IssueVerdict::LikelyBug;
        assert!(!should_emit_fix_directions(&d));
        let pack = build_fact_pack(&d, &NormalizedIssue::default(), None);
        assert!(pack.fix_directions.is_empty());
    }

    #[test]
    fn network_issue_skips_cc_hooks_dig_in_user_facts() {
        let mut d = base_decision();
        d.verdict = IssueVerdict::LikelyBug;
        d.confidence = 0.8;
        d.issue_title = "频繁网络连接中断".into();
        d.symptom_summary = "error decoding / 网络连接中断".into();
        d.code_hits = vec![CodeEvidence {
            path: "crates/x/src/provider/retry.rs".into(),
            line: 188,
            snippet: "网络连接中断".into(),
        }];
        let n = NormalizedIssue {
            error_signatures: vec![
                "error:".into(),
                "error decoding".into(),
                "网络连接中断".into(),
            ],
            symptom: "网络连接中断".into(),
            title: "频繁网络连接中断".into(),
            ..Default::default()
        };
        let tech = TechnicalVerification {
            enabled: true,
            deep_dig_ran: true,
            deep_dig: vec![
                DeepDigBlock {
                    path: "crates/atomcode-capabilities/src/cc_hooks.rs".into(),
                    anchor_line: 1350,
                    symbol: Some("post_tool_use_honors_matcher".into()),
                    symbol_is_fn: true,
                    start_line: 1327,
                    end_line: 1377,
                    context: " 1350| is_error: false,\n 1369| is_error: false,\n".into(),
                    callers: vec![],
                    file_commits: vec![],
                    notes: vec![],
                },
                DeepDigBlock {
                    path: "crates/atomcode-capabilities/src/provider/retry.rs".into(),
                    anchor_line: 188,
                    symbol: Some("err_chain".into()),
                    symbol_is_fn: true,
                    start_line: 151,
                    end_line: 207,
                    context: " 188| \"网络连接中断:远端关闭或重置了连接\"\n 179| error decoding response body\n".into(),
                    callers: vec![],
                    file_commits: vec![],
                    notes: vec![],
                },
            ],
            plan: InvestigationPlan::default(),
            code_hits: vec![],
            git_commits: vec![],
            confidence: 0.8,
            technical_verdict: IssueVerdict::LikelyBug,
            ..Default::default()
        };
        let pack = build_fact_pack(&d, &n, Some(&tech));
        let joined = pack
            .verified
            .iter()
            .chain(pack.maintainer_tips.iter())
            .map(|f| f.zh.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !joined.contains("cc_hooks")
                && !joined.contains("post_tool_use_honors_matcher")
                && !joined.contains("is_error"),
            "must not surface hooks dig: {joined}"
        );
        assert!(
            joined.contains("retry")
                || joined.contains("err_chain")
                || joined.contains("网络连接中断"),
            "must keep provider dig: {joined}"
        );
    }

    #[test]
    fn pairing_issue_does_not_get_network_upstream_boilerplate() {
        // #1220 类：口令「连接」新电脑 ≠ provider 断连
        let mut d = base_decision();
        d.verdict = IssueVerdict::Unverified;
        d.confidence = 0.55;
        d.technical_confidence = 0.55;
        d.issue_title = "[共创大赛][Bug] GitCode App 中无法使用口令连接一台新的电脑".into();
        d.symptom_summary =
            "在 GitCode App 中无法使用 TUI 中`/app` 生成的口令 连接一台新的电脑".into();
        d.code_hits = vec![CodeEvidence {
            path: "crates/atomcode-daemon/src/auth_token.rs".into(),
            line: 10,
            snippet: "WebuiToken /app pair".into(),
        }];
        d.verification_ran = true;
        assert!(
            !is_network_provider_copy(
                &format!("{} {}", d.issue_title, d.symptom_summary),
                &d.code_hits
            ),
            "pairing/口令 must not classify as network provider"
        );
        let pack = build_fact_pack(&d, &NormalizedIssue::default(), None);
        let joined = pack
            .unconfirmed
            .iter()
            .map(|f| f.zh.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !joined.contains("上游限流")
                && !joined.contains("本机代理")
                && !joined.contains("网关策略"),
            "must not use network-disconnect boilerplate for pairing: {joined}"
        );
        assert!(
            joined.contains("相关代码区域") || joined.contains("还不能断定"),
            "generic unconfirmed copy expected: {joined}"
        );
    }

    #[test]
    fn network_disconnect_still_gets_upstream_boilerplate() {
        let d = base_decision();
        assert!(is_network_provider_copy(
            &format!("{} {}", d.issue_title, d.symptom_summary),
            &d.code_hits
        ));
        let pack = build_fact_pack(&d, &NormalizedIssue::default(), None);
        let joined = pack
            .unconfirmed
            .iter()
            .map(|f| f.zh.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            joined.contains("上游限流") || joined.contains("网关"),
            "{joined}"
        );
    }

    #[test]
    fn fix_directions_require_at_least_0_85() {
        // 边界：0.84 不出，0.85 才出（与 FIX_DIRECTION_MIN_CONF 一致）
        let mut below = base_decision();
        below.verdict = IssueVerdict::LikelyBug;
        below.confidence = 0.84;
        below.technical_confidence = 0.84;
        below.verification_ran = true;
        below.code_hits = vec![CodeEvidence {
            path: "src/provider/retry.rs".into(),
            line: 188,
            snippet: "网络连接中断".into(),
        }];
        assert!(
            !should_emit_fix_directions(&below),
            "0.84 must not emit fix directions"
        );
        let pack_below = build_fact_pack(&below, &NormalizedIssue::default(), None);
        assert!(pack_below.fix_directions.is_empty());

        let mut at = below.clone();
        at.confidence = FIX_DIRECTION_MIN_CONF;
        at.technical_confidence = FIX_DIRECTION_MIN_CONF;
        assert!(
            should_emit_fix_directions(&at),
            "exactly 0.85 must allow fix directions"
        );
    }

    #[test]
    fn fix_direction_commits_skip_auth_oauth_on_network_issue() {
        let mut d = base_decision();
        d.verdict = IssueVerdict::LikelyBug;
        d.confidence = 0.88;
        d.technical_confidence = 0.88;
        d.verification_ran = true;
        d.issue_title = "频繁网络连接中断".into();
        d.symptom_summary = "error decoding / 网络连接中断".into();
        let tech = TechnicalVerification {
            enabled: true,
            deep_dig_ran: true,
            deep_dig: vec![DeepDigBlock {
                path: "crates/x/src/provider/openai_compat.rs".into(),
                anchor_line: 498,
                symbol: Some("chat_stream".into()),
                symbol_is_fn: true,
                start_line: 400,
                end_line: 520,
                context: " 498| error decoding response body / 网络连接中断\n".into(),
                callers: vec![],
                file_commits: vec![
                    "fcf0b5b6 fix(auth): harden concurrent token recovery".into(),
                    "7182883f fix(v2/provider): 连接重置加固重连+人话提示".into(),
                ],
                notes: vec![],
            }],
            plan: InvestigationPlan::default(),
            code_hits: vec![],
            git_commits: vec![],
            confidence: 0.88,
            technical_verdict: IssueVerdict::LikelyBug,
            ..Default::default()
        };
        let pack = build_fix_directions(&d, Some(&tech), &["网络连接中断"]);
        let joined = pack
            .iter()
            .map(|f| f.zh.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !joined.contains("fcf0b5b6") && !joined.contains("fix(auth)"),
            "fix-direction commits must not use oauth/auth fix: {joined}"
        );
        // dig 本身可能已给出方向；若出现 commit 对齐，须是连接重置类
        if joined.contains("对齐") {
            assert!(
                joined.contains("7182883f") || joined.contains("连接重置"),
                "thematic commit only: {joined}"
            );
        }
    }

    #[test]
    fn verified_commits_skip_unrelated_auth_fix_on_network_issue() {
        let mut d = base_decision();
        d.verdict = IssueVerdict::LikelyBug;
        d.confidence = 0.8;
        d.issue_title = "频繁网络连接中断".into();
        d.symptom_summary = "error decoding / 网络连接中断".into();
        d.code_hits = vec![CodeEvidence {
            path: "crates/x/src/provider/openai_compat.rs".into(),
            line: 498,
            snippet: "error decoding response body".into(),
        }];
        let n = NormalizedIssue {
            error_signatures: vec!["error decoding".into(), "网络连接中断".into()],
            symptom: "网络连接中断".into(),
            title: "频繁网络连接中断".into(),
            ..Default::default()
        };
        let tech = TechnicalVerification {
            enabled: true,
            deep_dig_ran: true,
            deep_dig: vec![
                DeepDigBlock {
                    path: "crates/x/src/provider/retry.rs".into(),
                    anchor_line: 188,
                    symbol: Some("err_chain".into()),
                    symbol_is_fn: true,
                    start_line: 150,
                    end_line: 200,
                    context: " 188| 网络连接中断\n".into(),
                    callers: vec![],
                    file_commits: vec![
                        "522c6f2a fix(tls): retry AtomGit connections with TLS 1.2".into(),
                        "7182883f fix(v2/provider): 连接重置加固重连+人话提示".into(),
                    ],
                    notes: vec![],
                },
                DeepDigBlock {
                    path: "crates/x/src/auth/oauth.rs".into(),
                    anchor_line: 570,
                    symbol: Some("start_login".into()),
                    symbol_is_fn: true,
                    start_line: 546,
                    end_line: 579,
                    context: " 570| TLS 1.2 fallback also failed\n".into(),
                    callers: vec![],
                    file_commits: vec![
                        "522c6f2a fix(tls): retry AtomGit connections with TLS 1.2".into()
                    ],
                    notes: vec![],
                },
            ],
            plan: InvestigationPlan::default(),
            code_hits: vec![],
            git_commits: vec![],
            confidence: 0.8,
            technical_verdict: IssueVerdict::LikelyBug,
            ..Default::default()
        };
        let pack = build_fact_pack(&d, &n, Some(&tech));
        let joined = pack
            .verified
            .iter()
            .map(|f| f.zh.clone())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !joined.contains("522c6f2a")
                && !joined.contains("fix(tls)")
                && !joined.contains("oauth")
                && !joined.contains("start_login"),
            "must not surface oauth/tls login fix: {joined}"
        );
        assert!(
            joined.contains("7182883f") || joined.contains("连接重置"),
            "should prefer thematic connection-reset fix: {joined}"
        );
    }
}

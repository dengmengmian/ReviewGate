//! 审查结果渲染：JSON 信封 + 人类可读文本。

use reviewgate_core::gate::GateDecision;
use reviewgate_core::model::{Dimension, Finding, IntentStatus, Severity};
use reviewgate_core::review::{ReviewOutcome, ReviewWarning};
use serde::Serialize;
use std::io::IsTerminal;

// ───────────────────────── JSON 信封 ─────────────────────────

#[derive(Serialize)]
struct Summary {
    total: usize,
    kept: usize,
    filtered: usize,
    warnings: usize,
}

/// 每条发现的 JSON 视图：**位置在最前**，便于一眼看清哪文件哪行；其后是分类与内容。
/// 单独定义（而非直接序列化 Finding）以固定一个对人友好的字段顺序。
#[derive(Serialize)]
struct FindingView<'a> {
    path: &'a str,
    start_line: u32,
    end_line: u32,
    dimension: &'a str,
    severity: &'a str,
    confidence: f32,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestion: Option<&'a str>,
    /// 建议替换代码——有就显示，没有为空串（始终输出该键，便于消费方判断）。
    suggestion_code: &'a str,
    filtered: bool,
    agreed_dimensions: u8,
    /// 意图评审：映射的验收标准（其它维度为 None，JSON 跳过）。
    #[serde(skip_serializing_if = "Option::is_none")]
    criterion: Option<&'a str>,
    /// 意图评审：相对验收标准的判定（met/missing/deviation/breaking/suggestion）。
    #[serde(skip_serializing_if = "Option::is_none")]
    intent_status: Option<&'a str>,
    #[serde(skip_serializing_if = "str::is_empty")]
    existing_code: &'a str,
    #[serde(skip_serializing_if = "str::is_empty")]
    evidence: &'a str,
}

impl<'a> From<&'a Finding> for FindingView<'a> {
    fn from(f: &'a Finding) -> Self {
        FindingView {
            path: &f.path,
            start_line: f.start_line,
            end_line: f.end_line,
            dimension: f.dimension.as_str(),
            severity: f.severity.as_str(),
            confidence: (f.confidence * 100.0).round() / 100.0,
            message: &f.message,
            suggestion: f.suggestion.as_deref(),
            suggestion_code: &f.suggestion_code,
            filtered: f.filtered,
            agreed_dimensions: f.agreed_dimensions,
            criterion: f.criterion.as_deref(),
            intent_status: f.intent_status.map(|s| s.as_str()),
            existing_code: &f.existing_code,
            evidence: &f.evidence,
        }
    }
}

#[derive(Serialize)]
struct Envelope<'a> {
    decision: String,
    /// 是否未审完（请求失败/上下文超限/超时/超大文件跳过）。true 时 decision 不代表"无问题"。
    incomplete: bool,
    files_changed: usize,
    summary: Summary,
    warnings: &'a [ReviewWarning],
    findings: Vec<FindingView<'a>>,
    usage: &'a reviewgate_core::model::Usage,
}

/// 自包含的 JSON 输出：顶层判定 + 摘要 + 未审完告警 + 发现数组 + 用量。
/// 字段顺序固定（位置在前），且消费方既能拿到 PASS/WARN/BLOCK，也知道哪个维度没审完。
pub fn render_json(o: &ReviewOutcome) -> anyhow::Result<String> {
    let kept = o.findings.iter().filter(|f| !f.filtered).count();
    let env = Envelope {
        decision: o.decision.as_str().to_lowercase(), // pass | warn | block
        incomplete: o.incomplete,
        files_changed: o.files_changed,
        summary: Summary {
            total: o.findings.len(),
            kept,
            filtered: o.findings.len() - kept,
            warnings: o.warnings.len(),
        },
        warnings: &o.warnings,
        findings: o.findings.iter().map(FindingView::from).collect(),
        usage: &o.usage,
    };
    Ok(serde_json::to_string_pretty(&env)?)
}

// ───────────────────────── 终端安全 ─────────────────────────

/// 清洗 LLM 内容里的终端转义/控制字符——防止 message/suggestion 注入用户终端
/// （改颜色、清屏、伪造输出）。保留可见文本与换行。
fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect()
}

// ───────────────────────── CJK 宽度换行 ─────────────────────────

/// 字符显示宽度：东亚宽字符与 emoji 记 2 列，其余 1。
fn char_width(c: char) -> usize {
    let u = c as u32;
    let wide = (0x1100..=0x115F).contains(&u)
        || (0x2E80..=0xA4CF).contains(&u)
        || (0xAC00..=0xD7A3).contains(&u)
        || (0xF900..=0xFAFF).contains(&u)
        || (0xFE30..=0xFE4F).contains(&u)
        || (0xFF00..=0xFF60).contains(&u)
        || (0xFFE0..=0xFFE6).contains(&u)
        || (0x1F300..=0x1FAFF).contains(&u)
        || (0x20000..=0x3FFFD).contains(&u);
    if wide {
        2
    } else {
        1
    }
}

/// 按显示宽度折行（CJK 友好）。尊重已有换行。
fn wrap(s: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in s.split('\n') {
        let mut cur = String::new();
        let mut w = 0;
        for ch in para.chars() {
            let cw = char_width(ch);
            if w + cw > max && !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
                w = 0;
            }
            cur.push(ch);
            w += cw;
        }
        out.push(cur);
    }
    out
}

// ───────────────────────── 文本渲染 ─────────────────────────

struct Palette {
    on: bool,
}

impl Palette {
    fn new() -> Self {
        Palette {
            on: std::io::stdout().is_terminal(),
        }
    }
    fn paint(&self, code: &str, s: &str) -> String {
        if self.on {
            format!("\x1b[{code}m{s}\x1b[0m")
        } else {
            s.to_string()
        }
    }
    fn sev(&self, sev: Severity, s: &str) -> String {
        let code = match sev {
            Severity::High => "1;31",
            Severity::Med => "33",
            Severity::Low => "2",
        };
        self.paint(code, s)
    }
    fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }
    fn bold(&self, s: &str) -> String {
        self.paint("1", s)
    }
}

fn decision_banner(p: &Palette, d: GateDecision) -> String {
    let code = match d {
        GateDecision::Pass => "1;32",
        GateDecision::Warn => "1;33",
        GateDecision::Block => "1;31",
    };
    p.paint(code, &format!("ReviewGate: {}", d.as_str()))
}

/// 渲染人类可读文本。`show_filtered` 时展开被过滤的低置信项。
pub fn render_text(outcome: &ReviewOutcome, show_filtered: bool) -> String {
    let p = Palette::new();
    let mut out = String::new();

    if outcome.files_changed == 0 {
        return "No changes detected.\n".into();
    }

    // 意图评审发现单独走「验收清单」区，不混进常规缺陷区（避免重复）。
    let intent: Vec<&Finding> = outcome
        .findings
        .iter()
        .filter(|f| f.dimension == Dimension::Intent)
        .collect();
    let mut kept: Vec<&Finding> = outcome
        .findings
        .iter()
        .filter(|f| !f.filtered && f.dimension != Dimension::Intent)
        .collect();
    let mut filtered: Vec<&Finding> = outcome
        .findings
        .iter()
        .filter(|f| f.filtered && f.dimension != Dimension::Intent)
        .collect();
    kept.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then(a.path.cmp(&b.path))
            .then(a.start_line.cmp(&b.start_line))
    });
    filtered.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let must_fix = kept.iter().filter(|f| f.severity == Severity::High).count();
    let warnings = kept.len() - must_fix;

    out.push_str(&decision_banner(&p, outcome.decision));
    out.push('\n');
    if outcome.incomplete {
        out.push_str(&p.paint(
            "1;31",
            "Incomplete: some dimensions/units did not finish (timeout, request failure, context overflow, or oversized file skipped) - this result does not mean \"no issues\".",
        ));
        out.push('\n');
    }
    out.push('\n');
    out.push_str(&format!(
        "{} files changed · {} must fix · {} warnings · {} filtered\n",
        outcome.files_changed,
        must_fix,
        warnings,
        filtered.len()
    ));
    if outcome.usage.total_input() > 0 || outcome.usage.output_tokens > 0 {
        out.push_str(&p.dim(&format!("LLM: {}\n", outcome.usage.summary())));
    }
    out.push('\n');

    if !outcome.warnings.is_empty() {
        out.push_str(&p.paint("1;33", "Incomplete Review"));
        out.push('\n');
        out.push('\n');
        out.push_str("The following dimensions did not finish:\n");
        for w in &outcome.warnings {
            out.push_str(&format!(
                "- {}: {} ({})\n",
                sanitize(&w.dimension),
                sanitize(&w.message),
                w.kind
            ));
        }
        out.push_str("\nResult may be incomplete. Re-run with:\n");
        out.push_str("  reviewgate review --timeout 300 -v\n\n");
    }

    if !intent.is_empty() {
        out.push_str(&render_intent_checklist(&p, &intent));
    }

    if kept.is_empty() {
        out.push_str(&p.sev(Severity::Low, "No actionable issues found.\n\n"));
    } else {
        let highs: Vec<&Finding> = kept
            .iter()
            .copied()
            .filter(|f| f.severity == Severity::High)
            .collect();
        let non_highs: Vec<&Finding> = kept
            .iter()
            .copied()
            .filter(|f| f.severity != Severity::High)
            .collect();

        if !highs.is_empty() {
            out.push_str(&p.bold("Must Fix"));
            out.push_str("\n\n");
            for (i, f) in highs.into_iter().enumerate() {
                out.push_str(&render_finding(&p, f, i + 1));
                out.push('\n');
            }
        }

        if !non_highs.is_empty() {
            out.push_str(&p.bold("Warnings"));
            out.push_str("\n\n");
            for (i, f) in non_highs.into_iter().enumerate() {
                out.push_str(&render_finding(&p, f, i + 1));
                out.push('\n');
            }
        }
    }

    if !filtered.is_empty() {
        out.push_str(&p.dim("Not Shown"));
        out.push('\n');
        out.push('\n');
        if show_filtered {
            out.push_str(&p.dim(&format!("{} low-confidence findings:\n\n", filtered.len())));
            for (i, f) in filtered.iter().copied().enumerate() {
                out.push_str(&render_finding(&p, f, i + 1));
                out.push('\n');
            }
        } else {
            out.push_str(&p.dim(&format!(
                "{} low-confidence findings hidden. Run with --show-filtered to inspect them.\n\n",
                filtered.len()
            )));
        }
    }

    out.push_str(&p.dim("Next Steps"));
    out.push('\n');
    out.push('\n');
    if kept.iter().any(|f| !f.suggestion_code.trim().is_empty()) {
        out.push_str("Some findings include suggested patches. Apply manually, or run:\n");
        out.push_str("  reviewgate review --fix\n");
    } else if outcome.decision == GateDecision::Pass && outcome.warnings.is_empty() {
        out.push_str("No action required.\n");
    } else {
        out.push_str("Fix the findings above, then re-run:\n");
        out.push_str("  reviewgate review\n");
    }
    out.push_str("To debug slow reviews:\n");
    out.push_str("  reviewgate review -v --no-judge --dimensions logic\n");

    out
}

/// 意图/技术评审的「验收清单」：按验收标准分组，逐条显示满足/缺失/不符/破坏/建议。
fn render_intent_checklist(p: &Palette, intent: &[&Finding]) -> String {
    use std::collections::BTreeMap;
    let mut out = String::new();
    out.push_str(&p.bold("Intent / Acceptance Checklist"));
    out.push_str("\n\n");

    let mut by_crit: BTreeMap<&str, Vec<&Finding>> = BTreeMap::new();
    for f in intent {
        let c = f.criterion.as_deref().unwrap_or("(unspecified)");
        by_crit.entry(c).or_default().push(f);
    }
    for (crit, items) in &by_crit {
        out.push_str(&format!("• {}\n", sanitize(crit)));
        for f in items {
            let (label, color) = match f.intent_status {
                Some(IntentStatus::Met) => ("✓ met", "32"),
                Some(IntentStatus::Missing) => ("✗ missing", "1;31"),
                Some(IntentStatus::Breaking) => ("✗ breaking", "1;31"),
                Some(IntentStatus::Deviation) => ("⚠ deviation", "33"),
                Some(IntentStatus::Suggestion) => ("• suggestion", "36"),
                Some(IntentStatus::Unknown) => ("? not assessed", "2"),
                None => ("·", "0"),
            };
            out.push_str(&format!(
                "    {} {}",
                p.paint(color, label),
                p.dim(&format!("({:.0}%)", f.confidence * 100.0))
            ));
            if !f.path.is_empty() {
                let loc = if f.start_line > 0 {
                    format!("{}:{}", f.path, f.start_line)
                } else {
                    f.path.clone()
                };
                out.push_str(&p.dim(&format!(" [{loc}]")));
            }
            out.push('\n');
            out.push_str(&format!("      {}\n", sanitize(&f.message)));
            if let Some(s) = &f.suggestion {
                out.push_str(&p.dim(&format!("      → {}\n", sanitize(s))));
            }
        }
        out.push('\n');
    }
    out
}

fn render_finding(p: &Palette, f: &Finding, num: usize) -> String {
    let loc = if f.located() {
        if f.end_line > f.start_line {
            format!("{}:{}-{}", f.path, f.start_line, f.end_line)
        } else {
            format!("{}:{}", f.path, f.start_line)
        }
    } else {
        format!("{}:?", f.path)
    };
    let mut s = String::new();
    s.push_str(&format!("{}. {}\n", num, p.bold(&loc)));
    s.push_str(&p.sev(
        f.severity,
        &format!(
            "   {} / {} / confidence {:.2}",
            f.dimension.as_str(),
            f.severity.as_str(),
            f.confidence,
        ),
    ));
    if f.agreed_dimensions >= 2 {
        s.push_str(&p.dim(&format!(
            " / confirmed by {} dimensions",
            f.agreed_dimensions
        )));
    }
    s.push_str("\n\n");

    for line in wrap(&sanitize(&f.message), 88) {
        s.push_str(&format!("   {line}\n"));
    }

    let code = sanitize(&f.existing_code);
    let has_fix = !f.suggestion_code.trim().is_empty();
    if let Some(line) = code
        .lines()
        .map(|l| l.trim_end())
        .find(|l| !l.trim().is_empty())
    {
        s.push_str("\n   Current:\n");
        s.push_str(&p.paint("91", &format!("     - {line}")));
        s.push('\n');
    }

    if let Some(sug) = &f.suggestion {
        s.push_str("\n   Fix:\n");
        for line in wrap(&sanitize(sug), 84) {
            s.push_str(&p.dim(&format!("     {line}")));
            s.push('\n');
        }
    }

    if has_fix {
        s.push_str("\n   Suggested patch:\n");
        for line in code.lines().filter(|l| !l.trim().is_empty()).take(8) {
            s.push_str(&p.paint("91", &format!("     - {}", line.trim_end())));
            s.push('\n');
        }
        for line in sanitize(&f.suggestion_code).lines().take(8) {
            s.push_str(&p.paint("92", &format!("     + {}", line.trim_end())));
            s.push('\n');
        }
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use reviewgate_core::gate::GateDecision;
    use reviewgate_core::model::{Dimension, Usage};
    use reviewgate_core::review::ReviewWarning;

    #[test]
    fn sanitize_strips_escapes_keeps_newline() {
        let dirty = "正常\x1b[31m红\x1b[0m\n第二行\x07";
        let clean = sanitize(dirty);
        assert!(!clean.contains('\x1b'));
        assert!(!clean.contains('\x07'));
        assert!(clean.contains('\n'));
        assert!(clean.contains("红"));
    }

    #[test]
    fn wrap_respects_cjk_width() {
        // 5 个中文 = 10 列；max=6 → 应折成多行，每行 ≤6 列。
        let lines = wrap("一二三四五", 6);
        assert!(lines.len() >= 2);
        for l in &lines {
            assert!(display_width(l) <= 6);
        }
    }

    fn display_width(s: &str) -> usize {
        s.chars().map(char_width).sum()
    }

    fn finding(severity: Severity, filtered: bool) -> Finding {
        Finding {
            dimension: if severity == Severity::High {
                Dimension::Security
            } else {
                Dimension::Perf
            },
            confidence: if severity == Severity::High {
                0.94
            } else {
                0.67
            },
            severity,
            path: if severity == Severity::High {
                "src/auth.rs".into()
            } else {
                "src/cache.rs".into()
            },
            start_line: 42,
            end_line: 42,
            message: if severity == Severity::High {
                "SQL injection: user_id is concatenated into the query string.".into()
            } else {
                "The new lookup clones the full cache entry on every read.".into()
            },
            existing_code: "let q = format!(\"select * from users where id = {}\", user_id);"
                .into(),
            evidence: String::new(),
            suggestion: Some("Use a parameterized query.".into()),
            suggestion_code: if severity == Severity::High {
                "let q = sqlx::query(\"select * from users where id = $1\").bind(user_id);".into()
            } else {
                String::new()
            },
            reachability: reviewgate_core::model::Reachability::default(),
            filtered,
            agreed_dimensions: 1,
            criterion: None,
            intent_status: None,
        }
    }

    #[test]
    fn render_text_groups_by_user_decision() {
        let outcome = ReviewOutcome {
            findings: vec![
                finding(Severity::High, false),
                finding(Severity::Med, false),
                finding(Severity::Low, true),
            ],
            files_changed: 3,
            decision: GateDecision::Block,
            incomplete: true,
            warnings: vec![ReviewWarning {
                dimension: "logic".into(),
                kind: "timed_out",
                message: "墙钟超时".into(),
            }],
            usage: Usage {
                input_tokens: 1000,
                output_tokens: 200,
                cache_read_input_tokens: 1000,
                cache_creation_input_tokens: 0,
            },
        };

        let text = render_text(&outcome, false);
        assert!(text.contains("ReviewGate: BLOCK"));
        assert!(text.contains("3 files changed · 1 must fix · 1 warnings · 1 filtered"));
        assert!(text.contains("Incomplete Review"));
        assert!(text.contains("Must Fix"));
        assert!(text.contains("Warnings"));
        assert!(text.contains("Not Shown"));
        assert!(text.contains("Suggested patch"));
        assert!(text.contains("Next Steps"));
    }

    fn intent_finding(criterion: &str, status: IntentStatus, msg: &str) -> Finding {
        let mut f = finding(Severity::Low, false);
        f.dimension = Dimension::Intent;
        f.severity = match status {
            IntentStatus::Missing | IntentStatus::Breaking => Severity::High,
            _ => Severity::Low,
        };
        f.filtered = status == IntentStatus::Met; // met 是信息项，进清单但折叠
        f.path = String::new();
        f.start_line = 0;
        f.message = msg.into();
        f.suggestion = None;
        f.criterion = Some(criterion.into());
        f.intent_status = Some(status);
        f
    }

    #[test]
    fn intent_findings_render_as_checklist_not_in_regular_sections() {
        let outcome = ReviewOutcome {
            findings: vec![
                intent_finding(
                    "验收#1:buildURL 接受 URL 对象",
                    IntentStatus::Met,
                    "已在 buildURL 处理",
                ),
                intent_finding(
                    "验收#2:dispatch 处理 URL 对象",
                    IntentStatus::Missing,
                    "dispatchRequest 未规范化 URL 对象",
                ),
                finding(Severity::High, false), // 常规缺陷,应进 Must Fix
            ],
            files_changed: 2,
            decision: GateDecision::Warn,
            incomplete: false,
            warnings: vec![],
            usage: Usage::default(),
        };

        let text = render_text(&outcome, false);
        // 验收清单区出现,按 criterion 分组,带状态标签。
        assert!(text.contains("Intent / Acceptance Checklist"));
        assert!(text.contains("验收#2:dispatch 处理 URL 对象"));
        assert!(text.contains("met"));
        assert!(text.contains("missing"));
        assert!(text.contains("dispatchRequest 未规范化 URL 对象"));
        // 意图发现不重复出现在常规缺陷描述里（常规区只该有那条 SQL 注入缺陷）。
        assert!(text.contains("SQL injection"));
        let dispatch_hits = text.matches("dispatchRequest 未规范化 URL 对象").count();
        assert_eq!(dispatch_hits, 1, "意图发现只应出现在清单里，不重复");
    }
}

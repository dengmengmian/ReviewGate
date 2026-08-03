//! 审查结果渲染：JSON 信封 + 人类可读文本。

use crate::i18n::{GateLabel, Lang};
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
    /// 稳定指纹：复制进 `.reviewgate/ignore` 即可抑制该误报（不含行号，抗漂移）。
    fingerprint: String,
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
            fingerprint: reviewgate_core::review::fingerprint(f),
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
    /// 关键路径 incomplete 是否触发强制非 PASS。
    critical_incomplete: bool,
    files_changed: usize,
    /// 本次审查覆盖的范围。PASS 只对这个范围成立。
    #[serde(skip_serializing_if = "str::is_empty")]
    scope: &'a str,
    summary: Summary,
    warnings: &'a [ReviewWarning],
    findings: Vec<FindingView<'a>>,
    usage: &'a reviewgate_core::model::Usage,
    #[serde(skip_serializing_if = "Option::is_none")]
    cost_estimate: Option<&'a reviewgate_core::review::CostEstimate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unfinished_paths: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    advice: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unit_plan: Option<&'a reviewgate_core::review::UnitPlanSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    coverage: Option<&'a reviewgate_core::review::CoverageSnapshot>,
    /// 被排除规则挡下、未送审的文件（带原因）。让"少审了什么"可核对。
    #[serde(skip_serializing_if = "Option::is_none")]
    excluded: Option<&'a [reviewgate_core::diff::ExcludedFile]>,
}

/// 自包含的 JSON 输出：顶层判定 + 摘要 + 未审完告警 + 发现数组 + 用量。
/// 字段顺序固定（位置在前），且消费方既能拿到 PASS/WARN/BLOCK，也知道哪个维度没审完。
pub fn render_json(o: &ReviewOutcome) -> anyhow::Result<String> {
    let kept = o.findings.iter().filter(|f| !f.filtered).count();
    let unfinished = o
        .coverage
        .as_ref()
        .map(|c| c.unfinished_paths.clone())
        .unwrap_or_else(|| reviewgate_core::review::unfinished_paths(&o.warnings));
    let advice = o
        .coverage
        .as_ref()
        .map(|c| c.advice.clone())
        .unwrap_or_else(|| reviewgate_core::review::incomplete_advice(&o.warnings));
    let env = Envelope {
        decision: o.decision.as_str().to_lowercase(), // pass | warn | block
        incomplete: o.incomplete,
        critical_incomplete: o.critical_incomplete,
        files_changed: o.files_changed,
        scope: &o.scope,
        summary: Summary {
            total: o.findings.len(),
            kept,
            filtered: o.findings.len() - kept,
            warnings: o.warnings.len(),
        },
        warnings: &o.warnings,
        cost_estimate: o.cost_estimate.as_ref(),
        unfinished_paths: if unfinished.is_empty() {
            None
        } else {
            Some(unfinished)
        },
        advice: if advice.is_empty() {
            None
        } else {
            Some(advice)
        },
        unit_plan: o.unit_plan.as_ref(),
        coverage: o.coverage.as_ref(),
        excluded: if o.excluded.is_empty() {
            None
        } else {
            Some(&o.excluded)
        },
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
pub(crate) fn char_width(c: char) -> usize {
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

/// 按显示宽度折行：**按词断行**（ASCII 整词不拆开），CJK 逐字可断；超长单词兜底硬切。
/// 尊重已有换行。
fn wrap(s: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    for para in s.split('\n') {
        let mut cur = String::new();
        let mut w = 0usize;
        for unit in break_units(para) {
            // 行首不保留空白单元（断行处的空格丢弃）。
            if cur.is_empty() && unit.trim().is_empty() {
                continue;
            }
            let uw: usize = unit.chars().map(char_width).sum();
            if w + uw > max && !cur.is_empty() {
                out.push(cur.trim_end().to_string());
                cur = String::new();
                w = 0;
                if unit.trim().is_empty() {
                    continue;
                }
            }
            if uw > max {
                // 单个单元就超宽（超长 ASCII 词）：按字符硬切，避免溢出。
                for ch in unit.chars() {
                    let cw = char_width(ch);
                    if w + cw > max && !cur.is_empty() {
                        out.push(std::mem::take(&mut cur));
                        w = 0;
                    }
                    cur.push(ch);
                    w += cw;
                }
            } else {
                cur.push_str(&unit);
                w += uw;
            }
        }
        out.push(cur.trim_end().to_string());
    }
    out
}

/// 断行单元：宽字符各自成单元（逐字可断）；ASCII 按「空格段 / 非空格词」切分。
fn break_units(s: &str) -> Vec<String> {
    let mut units = Vec::new();
    let mut buf = String::new();
    let mut buf_space = false;
    for ch in s.chars() {
        if char_width(ch) == 2 {
            if !buf.is_empty() {
                units.push(std::mem::take(&mut buf));
            }
            units.push(ch.to_string());
            continue;
        }
        let is_space = ch == ' ';
        if !buf.is_empty() && is_space != buf_space {
            units.push(std::mem::take(&mut buf));
        }
        buf.push(ch);
        buf_space = is_space;
    }
    if !buf.is_empty() {
        units.push(buf);
    }
    units
}

// ───────────────────────── 文本渲染 ─────────────────────────

struct Palette {
    on: bool,
    /// 严重度的显示名与配色（团队可在配置里改，缺省即内置 high/med/low）。
    labels: reviewgate_core::config::SeverityLabels,
}

impl Palette {
    fn with_labels(labels: reviewgate_core::config::SeverityLabels) -> Self {
        // 颜色开关：尊重 `NO_COLOR`（任意值即关）；`FORCE_COLOR`/`CLICOLOR_FORCE` 可强制开
        // （管道/CI 里也上色）；否则按 stdout 是否为终端自适应。
        let no_color = std::env::var_os("NO_COLOR").is_some();
        let force = std::env::var_os("FORCE_COLOR").is_some()
            || std::env::var_os("CLICOLOR_FORCE").is_some();
        Palette {
            on: !no_color && (force || std::io::stdout().is_terminal()),
            labels,
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
        self.paint(self.labels.color(sev), s)
    }
    /// 该严重度的显示名（默认 high/med/low，可被配置改成团队用语）。
    fn sev_name(&self, sev: Severity) -> &str {
        self.labels.label(sev)
    }
    fn dim(&self, s: &str) -> String {
        self.paint("2", s)
    }
    fn bold(&self, s: &str) -> String {
        self.paint("1", s)
    }
}

/// 视觉宽度：分隔线与右对齐元信息的目标列。固定值，对宽/窄终端都稳。
const WIDTH: usize = 60;
/// 正文（发现描述/修复建议）折行宽度——比头部分隔线宽，散文读起来更顺。
const MSG_WIDTH: usize = 72;

/// 一条贯穿分隔线（dim）。
fn rule(p: &Palette) -> String {
    p.dim(&"━".repeat(WIDTH))
}

/// 带标题的顶部分隔线：`━━ ReviewGate ━━━…`（填满到 WIDTH）。
fn titled_rule(p: &Palette, title: &str) -> String {
    let head = format!("━━ {title} ");
    let used = display_width(&head);
    let tail = WIDTH.saturating_sub(used);
    p.dim(&format!("{head}{}", "━".repeat(tail)))
}

/// 区块标题：`▌ TITLE`，竖条按语境配色、标题加粗。
fn section(p: &Palette, title: &str, bar_code: &str) -> String {
    format!("{} {}", p.paint(bar_code, "▌"), p.bold(title))
}

/// 判定状态行：图标 + 判定词（本地化），整体按判定配色。
fn status_line(p: &Palette, d: GateDecision, t: Lang) -> String {
    let (icon, gate, code) = match d {
        GateDecision::Pass => ("✓", GateLabel::Pass, "1;32"),
        GateDecision::Warn => ("⚠", GateLabel::Warn, "1;33"),
        GateDecision::Block => ("✖", GateLabel::Block, "1;31"),
    };
    p.paint(code, &format!("{icon} {}", t.gate_label(gate)))
}

/// 紧凑计数：12000→"12k"、1500→"1.5k"、800→"800"。
fn human_count(n: u64) -> String {
    if n < 1000 {
        n.to_string()
    } else if n < 10_000 {
        format!("{:.1}k", n as f64 / 1000.0)
    } else {
        format!("{}k", n / 1000)
    }
}

pub(crate) fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

/// 按显示宽度截断（不加省略号）：尽量多取字符，但累计显示宽度不超过 `max`。
pub(crate) fn truncate_to_width(s: &str, max: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for c in s.chars() {
        let w = char_width(c);
        if used + w > max {
            break;
        }
        used += w;
        out.push(c);
    }
    out
}

/// 被排除文件的紧凑摘要：最多 `max` 条路径，其余折成 `(+N more)`。
fn excluded_sample(excluded: &[reviewgate_core::diff::ExcludedFile], max: usize) -> String {
    let shown: Vec<&str> = excluded.iter().take(max).map(|e| e.path.as_str()).collect();
    if excluded.len() > max {
        format!("{} (+{} more)", shown.join(", "), excluded.len() - max)
    } else {
        shown.join(", ")
    }
}

/// 被排除文件的逐条明细（路径 + 原因）。用于"全被排除"这种必须交代清楚的场景。
fn excluded_detail(excluded: &[reviewgate_core::diff::ExcludedFile]) -> String {
    excluded
        .iter()
        .map(|e| format!("  - {} ({})", e.path, e.reason.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 渲染人类可读文本。`show_filtered` 时展开被过滤的低置信项；`labels` 为严重度显示名/配色
/// （`SeverityLabels::default()` 即内置 high/med/low）。
pub fn render_text_with_labels(
    outcome: &ReviewOutcome,
    show_filtered: bool,
    labels: reviewgate_core::config::SeverityLabels,
) -> String {
    render_text_lang(outcome, show_filtered, Lang::detect(), labels)
}

/// 报告渲染的语言可注入版本（测试用，避免依赖进程 locale）。
fn render_text_lang(
    outcome: &ReviewOutcome,
    show_filtered: bool,
    t: Lang,
    labels: reviewgate_core::config::SeverityLabels,
) -> String {
    let p = Palette::with_labels(labels);
    let mut out = String::new();

    if outcome.files_changed == 0 {
        // 全被排除 ≠ 没有改动。说清楚，否则用户会把"规则写太宽"读成"这次没改东西"。
        if !outcome.excluded.is_empty() {
            return format!(
                "{}\n{}\n",
                t.all_excluded(outcome.excluded.len()),
                excluded_detail(&outcome.excluded)
            );
        }
        return format!("{}\n", t.no_changes());
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
    // total_cmp 给 f32 一个全序：即便置信度出现 NaN 也能稳定排序，不会因 partial_cmp 返回
    // None 退化成"全部相等"而打乱顺序。
    filtered.sort_by(|a, b| b.confidence.total_cmp(&a.confidence));
    let must_fix = kept.iter().filter(|f| f.severity == Severity::High).count();
    let warnings = kept.len() - must_fix;

    // ── 顶部：标题分隔线 + 状态/计数行 + LLM 行 + 收尾分隔线 ──
    out.push_str(&titled_rule(&p, "ReviewGate"));
    out.push('\n');
    // 计数行：有问题的计数才上色（must-fix 红 / warn 黄），为 0 则灰掉——一眼抓重点。
    let mf = if must_fix > 0 {
        p.paint("1;31", &t.must_fix(must_fix))
    } else {
        p.dim(&t.must_fix(0))
    };
    let wn = if warnings > 0 {
        p.paint("33", &t.warn(warnings))
    } else {
        p.dim(&t.warn(0))
    };
    out.push_str(&format!(
        "  {}    {} {} {mf} {} {wn} {} {}\n",
        status_line(&p, outcome.decision, t),
        t.files(outcome.files_changed),
        p.dim("·"),
        p.dim("·"),
        p.dim("·"),
        p.dim(&t.hidden(filtered.len())),
    ));
    if outcome.usage.total_input() > 0 || outcome.usage.output_tokens > 0 {
        let input = outcome.usage.total_input() as u64;
        let cache = outcome.usage.cache_read_input_tokens as u64;
        let pct = (cache * 100).checked_div(input).unwrap_or(0);
        out.push_str(&p.dim(&format!(
            "  LLM {} in (cache {}%) · {} out\n",
            human_count(input),
            pct,
            human_count(outcome.usage.output_tokens as u64),
        )));
    }
    if let Some(est) = &outcome.cost_estimate {
        out.push_str(&p.dim(&format!("  est {}\n", est.summary)));
    }
    if !outcome.scope.is_empty() {
        // 审的是哪一段必须写明——增量审查的 PASS 不等于整个 PR 通过。
        out.push_str(&p.dim(&format!("  {}\n", t.scope(&outcome.scope))));
    }
    if !outcome.excluded.is_empty() {
        out.push_str(&p.dim(&format!(
            "  {}\n",
            t.excluded(
                outcome.excluded.len(),
                &excluded_sample(&outcome.excluded, 4)
            )
        )));
    }
    if outcome.critical_incomplete {
        out.push_str(&p.paint(
            "1;31",
            "  ✖ critical paths incomplete — forced non-PASS (auth/payment/…)\n",
        ));
    }
    out.push_str(&rule(&p));
    out.push('\n');
    if outcome.incomplete {
        out.push('\n');
        let msg = t.incomplete_note();
        for (i, line) in wrap(msg, MSG_WIDTH).into_iter().enumerate() {
            let prefix = if i == 0 { "  ✖ " } else { "    " };
            out.push_str(&p.paint("1;31", &format!("{prefix}{line}")));
            out.push('\n');
        }
    }
    out.push('\n');

    // 大 PR：合成 unit job 清单（多单元时展示）。
    if let Some(plan) = &outcome.unit_plan {
        if plan.unit_count > 1 || plan.oversized_units > 0 {
            out.push_str(&section(&p, "UNITS (directory packing)", "36"));
            out.push_str("\n\n");
            out.push_str(&format!(
                "  {} unit(s) · {} reviewable · {} oversized\n",
                plan.unit_count, plan.reviewable_units, plan.oversized_units
            ));
            for job in &plan.units {
                let tag = if job.oversized {
                    "OVERSIZED"
                } else {
                    job.status.as_str()
                };
                let paths = if job.paths.len() > 6 {
                    format!(
                        "{} … (+{} files)",
                        job.paths[..6].join(", "),
                        job.paths.len() - 6
                    )
                } else {
                    job.paths.join(", ")
                };
                out.push_str(&format!(
                    "    • unit[{}] ~{} tok · {tag}\n      {}\n",
                    job.id, job.est_tokens, paths
                ));
            }
            out.push('\n');
        }
    }

    // 覆盖：covered / unfinished（多单元或 incomplete 时展示；干净单单元不刷屏）。
    if let Some(cov) = &outcome.coverage {
        if cov.should_surface() {
            out.push_str(&section(&p, "COVERAGE", "1;33"));
            out.push_str("\n\n");
            out.push_str(&format!(
                "  changed {} · covered {} · unfinished {} · oversized-skipped {}\n",
                cov.changed_paths.len(),
                cov.covered_paths.len(),
                cov.unfinished_paths.len(),
                cov.skipped_oversized_paths.len()
            ));
            if !cov.unfinished_paths.is_empty() {
                out.push_str(&format!(
                    "\n  Unfinished paths ({}):\n",
                    cov.unfinished_paths.len()
                ));
                for pth in cov.unfinished_paths.iter().take(40) {
                    out.push_str(&format!("    • {}\n", sanitize(pth)));
                }
                if cov.unfinished_paths.len() > 40 {
                    out.push_str(
                        &p.dim(&format!("    … {} more\n", cov.unfinished_paths.len() - 40)),
                    );
                }
            }
            if !cov.skipped_oversized_paths.is_empty() {
                out.push_str("\n  Oversized skipped:\n");
                for pth in cov.skipped_oversized_paths.iter().take(20) {
                    out.push_str(&format!("    • {}\n", sanitize(pth)));
                }
            }
            if !cov.advice.is_empty() {
                out.push_str(&format!("\n  {}\n", t.result_may_incomplete()));
                for a in &cov.advice {
                    out.push_str(&p.dim(&format!("    • {}\n", sanitize(a))));
                }
            }
            out.push('\n');
        }
    }

    if !outcome.warnings.is_empty() {
        out.push_str(&section(&p, t.sec_incomplete_review(), "1;33"));
        out.push('\n');
        out.push('\n');
        out.push_str(&format!("  {}\n", t.dims_not_finished()));
        for w in &outcome.warnings {
            out.push_str(&format!(
                "    • {}: {} ({})\n",
                sanitize(&w.dimension),
                sanitize(&w.message),
                w.kind
            ));
            if !w.paths.is_empty() {
                out.push_str(&p.dim(&format!("      paths: {}\n", w.paths.join(", "))));
            }
            if let Some(a) = &w.advice {
                out.push_str(&p.dim(&format!("      → {}\n", sanitize(a))));
            }
        }
        // Prefer coverage-driven unfinished list when present (already shown above);
        // still list path-less dimension timeouts under warnings.
        if outcome
            .coverage
            .as_ref()
            .map(|c| !c.should_surface())
            .unwrap_or(true)
        {
            let unfinished = reviewgate_core::review::unfinished_paths(&outcome.warnings);
            if !unfinished.is_empty() {
                out.push_str(&format!("\n  Unfinished paths ({}):\n", unfinished.len()));
                for pth in unfinished.iter().take(30) {
                    out.push_str(&format!("    • {}\n", sanitize(pth)));
                }
            }
            out.push_str(&format!("\n  {}\n", t.result_may_incomplete()));
            for a in reviewgate_core::review::incomplete_advice(&outcome.warnings) {
                out.push_str(&p.dim(&format!("    • {a}\n")));
            }
        }
        out.push_str(&p.dim("    reviewgate review --timeout 300 -v\n\n"));
    }

    if !intent.is_empty() {
        out.push_str(&render_intent_checklist(&p, &intent, t));
    }

    if kept.is_empty() {
        out.push_str(&p.sev(Severity::Low, &format!("  {}\n\n", t.no_actionable())));
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
            out.push_str(&section(&p, t.sec_must_fix(), "1;31"));
            out.push_str("\n\n");
            for (i, f) in highs.into_iter().enumerate() {
                out.push_str(&render_finding(&p, f, i + 1, t));
                out.push('\n');
            }
        }

        if !non_highs.is_empty() {
            out.push_str(&section(&p, t.sec_warnings(), "1;33"));
            out.push_str("\n\n");
            for (i, f) in non_highs.into_iter().enumerate() {
                out.push_str(&render_finding(&p, f, i + 1, t));
                out.push('\n');
            }
        }
    }

    if !filtered.is_empty() {
        out.push_str(&section(&p, t.sec_not_shown(), "2"));
        out.push('\n');
        out.push('\n');
        if show_filtered {
            out.push_str(&p.dim(&format!("  {}\n\n", t.low_conf_listed(filtered.len()))));
            for (i, f) in filtered.iter().copied().enumerate() {
                out.push_str(&render_finding(&p, f, i + 1, t));
                out.push('\n');
            }
        } else {
            out.push_str(&p.dim(&format!("  {}\n\n", t.low_conf_hidden(filtered.len()))));
        }
    }

    out.push_str(&section(&p, t.sec_next_steps(), "2"));
    out.push('\n');
    out.push('\n');
    if kept.iter().any(|f| !f.suggestion_code.trim().is_empty()) {
        out.push_str(&format!("  {}\n", t.next_patches()));
        out.push_str(&p.dim("    reviewgate review --fix\n"));
    } else if outcome.decision == GateDecision::Pass && outcome.warnings.is_empty() {
        out.push_str(&format!("  {}\n", t.next_no_action()));
    } else {
        out.push_str(&format!("  {}\n", t.next_fix_rerun()));
        out.push_str(&p.dim("    reviewgate review\n"));
    }
    out.push_str(&p.dim(&format!(
        "  {}reviewgate review -v --no-judge --dimensions logic\n",
        t.debug_slow_prefix()
    )));

    out
}

/// 意图/技术评审的「验收清单」：按验收标准分组，逐条显示满足/缺失/不符/破坏/建议。
fn render_intent_checklist(p: &Palette, intent: &[&Finding], t: Lang) -> String {
    use std::collections::BTreeMap;
    let mut out = String::new();
    out.push_str(&section(p, t.sec_intent_checklist(), "1;36"));
    out.push_str("\n\n");

    let mut by_crit: BTreeMap<&str, Vec<&Finding>> = BTreeMap::new();
    for f in intent {
        let c = f.criterion.as_deref().unwrap_or_else(|| t.unspecified());
        by_crit.entry(c).or_default().push(f);
    }
    for (crit, items) in &by_crit {
        out.push_str(&format!("• {}\n", sanitize(crit)));
        for f in items {
            let (label, color) = match f.intent_status {
                Some(IntentStatus::Met) => (t.intent_met(), "32"),
                Some(IntentStatus::Missing) => (t.intent_missing(), "1;31"),
                Some(IntentStatus::Breaking) => (t.intent_breaking(), "1;31"),
                Some(IntentStatus::Deviation) => (t.intent_deviation(), "33"),
                Some(IntentStatus::Suggestion) => (t.intent_suggestion(), "36"),
                Some(IntentStatus::Unknown) => (t.intent_not_assessed(), "2"),
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

fn render_finding(p: &Palette, f: &Finding, num: usize, t: Lang) -> String {
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

    // 标题行：`  N  path:line` 左对齐，`dimension · severity · NN%` 右对齐到 WIDTH（按严重度配色）。
    // 编号按严重度上色，当作彩色项目符号；路径加粗作为可点击的导航锚点。
    let left = format!(
        "  {}  {}",
        p.sev(f.severity, &num.to_string()),
        p.bold(&loc)
    );
    let left_plain = format!("  {num}  {loc}");
    let meta_plain = format!(
        "{} · {} · {:.0}%",
        f.dimension.as_str(),
        p.sev_name(f.severity),
        f.confidence * 100.0
    );
    let meta_painted = p.sev(f.severity, &meta_plain);
    // justify 按显示宽度计算间隔，但 left 含颜色码——这里用去色的 left_plain 量宽度。
    let gap = WIDTH
        .saturating_sub(display_width(&left_plain))
        .saturating_sub(display_width(&meta_plain));
    s.push_str(&format!("{left}{}{meta_painted}\n", " ".repeat(gap.max(2))));
    if f.agreed_dimensions >= 2 {
        s.push_str(&p.dim(&format!("     {}\n", t.confirmed_by(f.agreed_dimensions))));
    }
    s.push('\n');

    for line in wrap(&sanitize(&f.message), MSG_WIDTH) {
        s.push_str(&format!("     {line}\n"));
    }

    let code = sanitize(&f.existing_code);
    let has_fix = !f.suggestion_code.trim().is_empty();

    // 有补丁时统一走「Patch」差异块；否则按「Current / Fix」分别展示。
    if has_fix {
        s.push_str(&p.dim(&format!("\n     {}\n", t.patch())));
        for line in code.lines().filter(|l| !l.trim().is_empty()).take(8) {
            s.push_str(&p.paint("91", &format!("       - {}", line.trim_end())));
            s.push('\n');
        }
        for line in sanitize(&f.suggestion_code).lines().take(8) {
            s.push_str(&p.paint("92", &format!("       + {}", line.trim_end())));
            s.push('\n');
        }
    } else {
        if let Some(line) = code
            .lines()
            .map(|l| l.trim_end())
            .find(|l| !l.trim().is_empty())
        {
            s.push_str(&p.dim(&format!("\n     {}\n", t.current())));
            s.push_str(&p.paint("91", &format!("       - {line}")));
            s.push('\n');
        }
        if let Some(sug) = &f.suggestion {
            s.push_str(&p.dim(&format!("\n     {}\n", t.fix())));
            for line in wrap(&sanitize(sug), MSG_WIDTH.saturating_sub(2)) {
                s.push_str(&p.dim(&format!("       {line}")));
                s.push('\n');
            }
        }
    }

    // 指纹：复制进 `.reviewgate/ignore` 即可抑制这条误报（不含行号，抗漂移）。
    s.push_str(&p.dim(&format!(
        "\n     fp {}\n",
        reviewgate_core::review::fingerprint(f)
    )));

    s
}

// ───────────────────────── Issue 分诊 ─────────────────────────

/// Issue 分诊的终端输出。与 `review` 共用横幅、分区与宽度，
/// 让同一个工具的两条链路看起来是一件东西。
///
/// 面向人写，不是调试转储：内部枚举名（`LIKELY_BUG`）和字段名一律翻成人话，
/// 判定链（`reasons`）默认折叠，`--verbose` 才展开。
pub fn render_issue_review(
    out: &reviewgate_core::issue::ReviewOutput,
    verbose: bool,
    published: bool,
) -> String {
    use reviewgate_core::issue::IssueVerdict;

    let p = Palette::with_labels(reviewgate_core::config::SeverityLabels::default());
    let d = &out.decision;
    let plan = &out.planned;
    let mut s = String::new();

    // ── 头部：判定 + 一行关键计数
    let (icon, verdict_name, code) = match d.verdict {
        IssueVerdict::ConfirmedBug => ("✖", "确认缺陷", "1;31"),
        IssueVerdict::LikelyBug => ("●", "疑似缺陷", "1;33"),
        IssueVerdict::Regression => ("●", "疑似回归", "1;33"),
        IssueVerdict::Duplicate => ("⧉", "疑似重复", "1;36"),
        IssueVerdict::AlreadyFixed => ("✓", "可能已修复", "1;32"),
        IssueVerdict::NeedsInfo => ("?", "信息不足", "1;33"),
        IssueVerdict::NotABug => ("·", "非缺陷", "1;36"),
        IssueVerdict::Spam | IssueVerdict::Advertisement => ("✖", "垃圾/广告", "1;31"),
        IssueVerdict::Unverified => ("?", "判不准", "2"),
    };
    s.push_str(&titled_rule(&p, "ReviewGate · Issue"));
    s.push('\n');
    s.push_str(&format!(
        "  {}  {}\n",
        p.paint(code, &format!("{icon} {verdict_name}")),
        p.dim(&format!(
            "#{} · {} {:.0}% · 把握 {:.0}%",
            d.issue_number,
            issue_type_name(d.primary_type),
            d.type_confidence * 100.0,
            d.confidence * 100.0
        ))
    ));
    let mut facts: Vec<String> = Vec::new();
    if d.verification_ran {
        facts.push(format!("代码命中 {}", d.code_hits.len()));
        let dig = out
            .technical
            .as_ref()
            .map(|t| t.deep_dig.len())
            .unwrap_or(0);
        if dig > 0 {
            facts.push(format!("深挖 {dig}"));
        }
    }
    if let Some(n) = d.duplicate_of {
        facts.push(format!("关联 #{n}"));
    }
    if !d.missing_fields.is_empty() {
        facts.push(format!("缺 {} 项信息", d.missing_fields.len()));
    }
    if !facts.is_empty() {
        s.push_str(&format!("  {}\n", p.dim(&facts.join(" · "))));
    }
    s.push_str(&rule(&p));
    s.push('\n');

    // ── 证据：能点开核对的代码位置
    // 只展示**真正对上报错文本**的锚点。检索命中动辄十几处，直接 take(3)
    // 会把恰好含同名标识符的无关行也摆出来，反而拉低整条输出的可信度。
    let sigs: Vec<&str> = out
        .normalized
        .error_signatures
        .iter()
        .map(|x| x.as_str())
        .filter(|x| x.chars().count() >= 8)
        .collect();
    let anchors: Vec<&reviewgate_core::issue::CodeEvidence> = d
        .code_hits
        .iter()
        .filter(|h| sigs.iter().any(|sig| h.snippet.contains(sig)))
        .take(3)
        .collect();
    if !anchors.is_empty() {
        s.push_str(&format!("\n{}\n\n", section(&p, "证据", "36")));
        for h in anchors {
            s.push_str(&format!(
                "  {}\n",
                p.bold(&format!("{}:{}", h.path, h.line))
            ));
            let snip = truncate_to_width(h.snippet.trim(), MSG_WIDTH - 6);
            s.push_str(&format!("     {}\n", p.dim(&snip)));
        }
    } else if d.verification_ran && !d.code_hits.is_empty() {
        s.push_str(&format!("\n{}\n\n", section(&p, "证据", "36")));
        s.push_str(&format!(
            "  {}\n",
            p.dim(&format!(
                "检索到 {} 处相关代码，但没有一处与报错文本对上",
                d.code_hits.len()
            ))
        ));
    }

    // ── 计划动作：写操作一眼看全
    s.push_str(&format!("\n{}\n\n", section(&p, "计划动作", "33")));
    let comment_desc = if !plan.post_or_update_comment {
        p.dim("不发言")
    } else if plan.needs_human_notice {
        "移交给人（不下结论）".to_string()
    } else {
        "发布 / 更新机器人评论".to_string()
    };
    s.push_str(&format!("  {}  {comment_desc}\n", p.dim("评论")));
    if !plan.labels_to_add.is_empty() {
        s.push_str(&format!(
            "  {}  {}\n",
            p.dim("标签"),
            plan.labels_to_add.join(", ")
        ));
    }
    if let Some(login) = &plan.assign_to {
        s.push_str(&format!("  {}  @{login}\n", p.dim("指派")));
    }
    if plan.close {
        s.push_str(&format!("  {}  {}\n", p.dim("关闭"), p.paint("1;31", "是")));
    }
    for r in &plan.reasons_blocked {
        s.push_str(&format!(
            "  {}  {}\n",
            p.dim("拦下"),
            p.dim(&blocked_reason_name(r))
        ));
    }

    // ── 评论预览：原样要发出去的内容
    if !d.suggested_comment.trim().is_empty() {
        s.push_str(&format!("\n{}\n\n", section(&p, "评论预览", "32")));
        let body = d
            .suggested_comment
            .lines()
            .skip_while(|l| l.starts_with("<!-- reviewgate") || l.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        for line in body.trim_end().lines() {
            s.push_str(&format!("  {line}\n"));
        }
    }

    if !published {
        s.push_str(&format!(
            "\n  {}\n",
            p.dim("未发布——加 --publish 才会发出这条评论")
        ));
    }

    if verbose && !d.reasons.is_empty() {
        s.push_str(&format!("\n{}\n\n", section(&p, "判定依据", "2")));
        for r in &d.reasons {
            s.push_str(&format!("  {}\n", p.dim(r)));
        }
    }
    s
}

fn issue_type_name(t: reviewgate_core::issue::IssueType) -> &'static str {
    use reviewgate_core::issue::IssueType as T;
    match t {
        T::Bug => "缺陷",
        T::FeatureRequest => "需求",
        T::Question => "提问",
        T::Documentation => "文档",
        T::Configuration => "配置",
        T::Support => "支持",
        T::Security => "安全",
        T::Performance => "性能",
        T::Compatibility => "兼容性",
        T::Spam => "垃圾信息",
        T::Advertisement => "广告",
        T::Abuse => "辱骂",
        T::Unknown => "未定",
    }
}

/// 把 `low_confidence:0.40<0.50` 这类内部原因翻成人话。
fn blocked_reason_name(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix("low_confidence:") {
        return format!("把握不足（{rest}），未下结论");
    }
    match raw {
        "comment_disabled" => "评论已在配置中关闭".into(),
        "add_labels_disabled" => "打标签已在配置中关闭".into(),
        "no_triage_owner" => "未配置处理人，静默跳过".into(),
        "auto_action_not_allowed" => "证据不足以自动执行".into(),
        "close_disabled_by_policy" => "关闭已在配置中关闭".into(),
        other => other.to_string(),
    }
}

/// Issue 分诊的 JSON 输出。与文本是同一份信息的两种呈现：文本上看得到的字段
/// 这里都能取到，另外补上文本里折叠掉的判定链——机器不需要为了可读性省略。
pub fn render_issue_review_json(
    out: &reviewgate_core::issue::ReviewOutput,
    published: bool,
) -> anyhow::Result<String> {
    use serde::Serialize;

    #[derive(Serialize)]
    struct Scored {
        name: String,
        confidence: f32,
    }
    #[derive(Serialize)]
    struct Completeness {
        score: f32,
        missing: Vec<String>,
    }
    #[derive(Serialize)]
    struct Safety {
        spam: f32,
        advertisement: f32,
        abuse: f32,
        prompt_injection: f32,
    }
    #[derive(Serialize)]
    struct Duplicate {
        /// 用枚举自身的 serde 规则（snake_case），不要 Debug 格式化。
        status: reviewgate_core::issue::DuplicateStatus,
        confidence: f32,
        #[serde(skip_serializing_if = "Option::is_none")]
        of: Option<u64>,
        candidates: usize,
    }
    #[derive(Serialize)]
    struct Anchor<'a> {
        path: &'a str,
        line: u32,
        snippet: &'a str,
    }
    #[derive(Serialize)]
    struct Verification<'a> {
        ran: bool,
        verdict: String,
        confidence: f32,
        code_hits: usize,
        deep_dig: usize,
        anchors: Vec<Anchor<'a>>,
        paths: &'a [String],
        fix_prs: &'a [String],
    }
    #[derive(Serialize)]
    struct Planned<'a> {
        comment: bool,
        /// 这条评论是「交给人看」而非「给出结论」。
        hands_off_to_human: bool,
        labels: &'a [String],
        close: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        close_reason: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        assign_to: Option<&'a str>,
        /// 原始拦截原因（可编程判断），如 `low_confidence:0.40<0.50`。
        blocked: &'a [String],
    }
    #[derive(Serialize)]
    struct Envelope<'a> {
        issue_number: u64,
        r#type: Scored,
        verdict: Scored,
        completeness: Completeness,
        safety: Safety,
        duplicate: Duplicate,
        verification: Verification<'a>,
        planned: Planned<'a>,
        /// 原样要发出去的评论正文（含 bot marker）。
        comment: &'a str,
        /// 本次运行是否真的发布了。false = 仅预览。
        published: bool,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        misrouted_repos: Vec<&'a str>,
        reasons: &'a [String],
    }

    let d = &out.decision;
    let plan = &out.planned;
    let sigs: Vec<&str> = out
        .normalized
        .error_signatures
        .iter()
        .map(|x| x.as_str())
        .filter(|x| x.chars().count() >= 8)
        .collect();
    let env = Envelope {
        issue_number: d.issue_number,
        r#type: Scored {
            name: d.primary_type.as_str().to_string(),
            confidence: d.type_confidence,
        },
        verdict: Scored {
            name: d.verdict.as_str().to_ascii_lowercase(),
            confidence: d.confidence,
        },
        completeness: Completeness {
            score: d.completeness_score,
            missing: d.missing_fields.clone(),
        },
        safety: Safety {
            spam: d.spam_score,
            advertisement: d.advertisement_score,
            abuse: d.abuse_score,
            prompt_injection: d.prompt_injection_score,
        },
        duplicate: Duplicate {
            status: d.duplicate_status,
            confidence: d.duplicate_confidence,
            of: d.duplicate_of,
            candidates: d.duplicate_candidates.len(),
        },
        verification: Verification {
            ran: d.verification_ran,
            verdict: d.technical_verdict.as_str().to_ascii_lowercase(),
            confidence: d.technical_confidence,
            code_hits: d.code_hits.len(),
            deep_dig: out
                .technical
                .as_ref()
                .map(|t| t.deep_dig.len())
                .unwrap_or(0),
            // 与文本一致：只列真正对上报错文本的锚点
            anchors: d
                .code_hits
                .iter()
                .filter(|h| sigs.iter().any(|sig| h.snippet.contains(sig)))
                .take(3)
                .map(|h| Anchor {
                    path: &h.path,
                    line: h.line,
                    snippet: h.snippet.trim(),
                })
                .collect(),
            paths: &d.code_paths,
            fix_prs: &d.fix_prs,
        },
        planned: Planned {
            comment: plan.post_or_update_comment,
            hands_off_to_human: plan.needs_human_notice,
            labels: &plan.labels_to_add,
            close: plan.close,
            close_reason: plan.close_reason.as_deref(),
            assign_to: plan.assign_to.as_deref(),
            blocked: &plan.reasons_blocked,
        },
        comment: &d.suggested_comment,
        published,
        misrouted_repos: d.misrouted_repos.iter().map(|x| x.as_str()).collect(),
        reasons: &d.reasons,
    };
    Ok(serde_json::to_string_pretty(&env)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reviewgate_core::gate::GateDecision;
    use reviewgate_core::gate::GateDecision as GD;
    use reviewgate_core::model::{Dimension, Usage};
    use reviewgate_core::review::{
        CoverageSnapshot, ReviewWarning, UnitJobSummary, UnitPlanSummary,
    };

    fn outcome_with_units(
        decision: GateDecision,
        incomplete: bool,
        unit_plan: Option<UnitPlanSummary>,
        coverage: Option<CoverageSnapshot>,
        warnings: Vec<ReviewWarning>,
    ) -> ReviewOutcome {
        ReviewOutcome {
            findings: vec![],
            files_changed: 4,
            decision,
            incomplete,
            warnings,
            unit_plan,
            coverage,
            ..Default::default()
        }
    }

    #[test]
    fn render_text_and_json_surface_multi_unit_coverage() {
        let plan = UnitPlanSummary {
            unit_count: 2,
            reviewable_units: 1,
            oversized_units: 1,
            units: vec![
                UnitJobSummary {
                    id: 0,
                    paths: vec!["a/1.rs".into(), "a/2.rs".into()],
                    est_tokens: 100,
                    oversized: false,
                    status: "incomplete".into(),
                },
                UnitJobSummary {
                    id: 1,
                    paths: vec!["huge.rs".into()],
                    est_tokens: 99999,
                    oversized: true,
                    status: "skipped_oversized".into(),
                },
            ],
        };
        let cov = CoverageSnapshot {
            changed_paths: vec!["a/1.rs".into(), "a/2.rs".into(), "huge.rs".into()],
            planned_paths: vec!["a/1.rs".into(), "a/2.rs".into()],
            skipped_oversized_paths: vec!["huge.rs".into()],
            unfinished_paths: vec!["a/1.rs".into(), "huge.rs".into()],
            covered_paths: vec!["a/2.rs".into()],
            advice: vec!["Raise --timeout".into()],
            multi_unit: true,
            incomplete: true,
        };
        let o = outcome_with_units(
            GateDecision::Warn,
            true,
            Some(plan),
            Some(cov),
            vec![ReviewWarning::new("logic", "timed_out", "timeout")],
        );
        // Must not look like a clean PASS with no coverage story.
        assert_ne!(o.decision, GD::Pass);
        assert!(o.incomplete);
        let text = render_text_lang(&o, false, Lang::En, Default::default());
        assert!(
            text.contains("UNITS") && text.contains("unit[0]") && text.contains("huge.rs"),
            "text should list units: {text}"
        );
        assert!(
            text.contains("COVERAGE") && text.contains("Unfinished paths"),
            "text should list coverage: {text}"
        );
        assert!(text.contains("Raise --timeout") || text.contains("timeout"));
        let json = render_json(&o).unwrap();
        assert!(json.contains("unit_plan") && json.contains("coverage"));
        assert!(json.contains("unfinished_paths") || json.contains("huge.rs"));
        assert!(json.contains("\"incomplete\": true"));
    }

    #[test]
    fn clean_pass_single_unit_no_fake_coverage_section() {
        let o = ReviewOutcome {
            files_changed: 1,
            decision: GateDecision::Pass,
            incomplete: false,
            unit_plan: Some(UnitPlanSummary {
                unit_count: 1,
                reviewable_units: 1,
                oversized_units: 0,
                units: vec![UnitJobSummary {
                    id: 0,
                    paths: vec!["a.rs".into()],
                    est_tokens: 10,
                    oversized: false,
                    status: "reviewed".into(),
                }],
            }),
            coverage: Some(CoverageSnapshot {
                changed_paths: vec!["a.rs".into()],
                planned_paths: vec!["a.rs".into()],
                skipped_oversized_paths: vec![],
                unfinished_paths: vec![],
                covered_paths: vec!["a.rs".into()],
                advice: vec![],
                multi_unit: false,
                incomplete: false,
            }),
            ..Default::default()
        };
        let text = render_text_lang(&o, false, Lang::En, Default::default());
        assert!(
            !text.contains("UNITS (directory packing)"),
            "clean single-unit must not invent multi-unit section: {text}"
        );
        assert!(
            !text.contains("Unfinished paths"),
            "clean pass must not invent unfinished paths: {text}"
        );
    }

    #[test]
    fn custom_severity_label_replaces_default_name() {
        use reviewgate_core::config::{SeverityLabel, SeverityLabels};
        let outcome = ReviewOutcome {
            findings: vec![finding(Severity::High, false)],
            files_changed: 1,
            decision: GateDecision::Block,
            ..Default::default()
        };
        let labels = SeverityLabels::resolve(&[SeverityLabel {
            id: "high".into(),
            label: Some("Blocker".into()),
            color: Some("magenta".into()),
            definition: None,
        }])
        .unwrap();
        let text = render_text_lang(&outcome, false, Lang::En, labels);
        assert!(text.contains("Blocker"), "应显示团队标签：{text}");
        assert!(!text.contains(" · high · "), "不应再出现默认标签：{text}");
    }

    #[test]
    fn all_files_excluded_does_not_read_as_no_changes() {
        let o = ReviewOutcome {
            files_changed: 0,
            excluded: vec![reviewgate_core::diff::ExcludedFile {
                path: "Cargo.lock".into(),
                reason: reviewgate_core::diff::ExcludeReason::Builtin,
            }],
            ..Default::default()
        };
        let text = render_text_lang(&o, false, Lang::En, Default::default());
        assert!(
            text.contains("excluded") && text.contains("Cargo.lock"),
            "全被排除必须说清楚是排除而非无改动：{text}"
        );
        assert!(
            !text.contains("No changes detected"),
            "不能把'全被排除'渲染成'没有改动'：{text}"
        );
    }

    #[test]
    fn excluded_files_are_surfaced_in_text_and_json() {
        let o = ReviewOutcome {
            files_changed: 1,
            decision: GateDecision::Pass,
            excluded: vec![reviewgate_core::diff::ExcludedFile {
                path: "vendor/dep.go".into(),
                reason: reviewgate_core::diff::ExcludeReason::Builtin,
            }],
            ..Default::default()
        };
        let text = render_text_lang(&o, false, Lang::En, Default::default());
        assert!(
            text.contains("vendor/dep.go"),
            "文本应列出被排除文件：{text}"
        );
        let json = render_json(&o).unwrap();
        assert!(json.contains("\"excluded\""));
        assert!(json.contains("vendor/dep.go"));
        assert!(json.contains("\"builtin\""));
    }

    #[test]
    fn render_text_shows_fingerprint_for_copy() {
        let f = finding(Severity::High, false);
        let fp = reviewgate_core::review::fingerprint(&f);
        let outcome = ReviewOutcome {
            findings: vec![f],
            files_changed: 1,
            decision: GateDecision::Block,
            incomplete: false,
            warnings: vec![],
            usage: Usage::default(),
            ..Default::default()
        };
        let text = render_text_with_labels(&outcome, false, Default::default());
        assert!(
            text.contains(&fp),
            "文本输出应打印指纹供复制进 .reviewgate/ignore，实际:\n{text}"
        );
    }

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
            warnings: vec![ReviewWarning::new("logic", "timed_out", "墙钟超时")],
            usage: Usage {
                input_tokens: 1000,
                output_tokens: 200,
                cache_read_input_tokens: 1000,
                cache_creation_input_tokens: 0,
            },
            ..Default::default()
        };

        let text = render_text_lang(&outcome, false, Lang::En, Default::default());
        assert!(text.contains("BLOCK"));
        assert!(text.contains("ReviewGate"));
        assert!(text.contains("3 files · 1 must-fix · 1 warn · 1 hidden"));
        assert!(text.contains("INCOMPLETE REVIEW"));
        assert!(text.contains("MUST FIX"));
        assert!(text.contains("WARNINGS"));
        assert!(text.contains("NOT SHOWN"));
        assert!(text.contains("Patch"));
        assert!(text.contains("NEXT STEPS"));

        // 同一结果切到中文：章节/状态/计数行均本地化，命令名保持原样。
        let zh = render_text_lang(&outcome, false, Lang::Zh, Default::default());
        assert!(zh.contains("拦截"));
        assert!(zh.contains("3 个文件 · 1 必须修复 · 1 警告 · 1 隐藏"));
        assert!(zh.contains("必须修复"));
        assert!(zh.contains("后续步骤"));
        assert!(zh.contains("reviewgate review --fix"));
        assert!(!zh.contains("NEXT STEPS"));
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
            ..Default::default()
        };

        let text = render_text_lang(&outcome, false, Lang::En, Default::default());
        // 验收清单区出现,按 criterion 分组,带状态标签。
        assert!(text.contains("INTENT / ACCEPTANCE CHECKLIST"));
        assert!(text.contains("验收#2:dispatch 处理 URL 对象"));
        assert!(text.contains("met"));
        assert!(text.contains("missing"));
        assert!(text.contains("dispatchRequest 未规范化 URL 对象"));
        // 意图发现不重复出现在常规缺陷描述里（常规区只该有那条 SQL 注入缺陷）。
        assert!(text.contains("SQL injection"));
        let dispatch_hits = text.matches("dispatchRequest 未规范化 URL 对象").count();
        assert_eq!(dispatch_hits, 1, "意图发现只应出现在清单里，不重复");
    }

    #[test]
    fn render_json_includes_decision_summary_and_findings() {
        let outcome = ReviewOutcome {
            findings: vec![finding(Severity::High, false)],
            files_changed: 1,
            decision: GateDecision::Block,
            incomplete: false,
            warnings: vec![],
            usage: Usage {
                input_tokens: 500,
                output_tokens: 50,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            },
            ..Default::default()
        };
        let json = render_json(&outcome).unwrap();
        assert!(json.contains("\"decision\": \"block\""));
        assert!(json.contains("\"files_changed\": 1"));
        assert!(json.contains("\"total\": 1"));
        assert!(json.contains("\"kept\": 1"));
        assert!(json.contains("\"filtered\": 0"));
        assert!(json.contains("\"path\": \"src/auth.rs\""));
        assert!(json.contains("\"start_line\": 42"));
        assert!(json.contains("\"dimension\": \"security\""));
        // 指纹随每条 finding 输出，供用户复制进 .reviewgate/ignore 抑制误报。
        let fp = reviewgate_core::review::fingerprint(&finding(Severity::High, false));
        assert_eq!(fp.len(), 12);
        assert!(json.contains(&format!("\"fingerprint\": \"{fp}\"")));
    }

    #[test]
    fn render_json_hides_filtered_fields() {
        let mut f = finding(Severity::Low, true);
        f.confidence = 0.3;
        let outcome = ReviewOutcome {
            findings: vec![f],
            files_changed: 1,
            decision: GateDecision::Pass,
            incomplete: false,
            warnings: vec![],
            usage: Usage::default(),
            ..Default::default()
        };
        let json = render_json(&outcome).unwrap();
        assert!(json.contains("\"filtered\": true"));
        assert!(json.contains("\"decision\": \"pass\""));
    }

    #[test]
    fn render_json_includes_warnings_and_usage() {
        let outcome = ReviewOutcome {
            findings: vec![],
            files_changed: 1,
            decision: GateDecision::Pass,
            incomplete: true,
            warnings: vec![ReviewWarning::new("logic", "timed_out", "timeout")],
            usage: Usage {
                input_tokens: 100,
                output_tokens: 50,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            },
            ..Default::default()
        };
        let json = render_json(&outcome).unwrap();
        assert!(json.contains("\"warnings\""));
        assert!(json.contains("\"timed_out\""));
        assert!(json.contains("\"usage\""));
        assert!(json.contains("\"input_tokens\": 100"));
    }

    #[test]
    fn render_text_show_filtered_lists_them() {
        let outcome = ReviewOutcome {
            findings: vec![finding(Severity::Low, true)],
            files_changed: 1,
            decision: GateDecision::Pass,
            incomplete: false,
            warnings: vec![],
            usage: Usage::default(),
            ..Default::default()
        };
        let shown = render_text_lang(&outcome, true, Lang::En, Default::default());
        let hidden = render_text_lang(&outcome, false, Lang::En, Default::default());
        assert!(shown.contains("NOT SHOWN") || shown.contains("低置信"));
        assert!(!hidden.contains("The new lookup"));
    }

    #[test]
    fn render_text_no_changes_returns_localized() {
        let outcome = ReviewOutcome {
            findings: vec![],
            files_changed: 0,
            decision: GateDecision::Pass,
            incomplete: false,
            warnings: vec![],
            usage: Usage::default(),
            ..Default::default()
        };
        let en = render_text_lang(&outcome, false, Lang::En, Default::default());
        assert!(en.contains("No changes") || en.contains("no changes"));
        let zh = render_text_lang(&outcome, false, Lang::Zh, Default::default());
        assert!(zh.contains("无改动") || zh.contains("没有"));
    }

    #[test]
    fn render_intent_checklist_all_statuses() {
        let statuses = vec![
            (IntentStatus::Met, "met"),
            (IntentStatus::Missing, "missing"),
            (IntentStatus::Deviation, "deviation"),
            (IntentStatus::Breaking, "breaking"),
            (IntentStatus::Suggestion, "suggestion"),
            (IntentStatus::Unknown, "unknown"),
        ];
        let findings: Vec<Finding> = statuses
            .into_iter()
            .enumerate()
            .map(|(i, (s, _))| {
                let mut f = intent_finding(&format!("criterion {i}"), s, &format!("msg {i}"));
                f.path = format!("f{i}.rs");
                f.start_line = i as u32 + 1;
                f
            })
            .collect();
        let refs: Vec<&Finding> = findings.iter().collect();
        let p = Palette::with_labels(Default::default());
        let out = render_intent_checklist(&p, &refs, Lang::En);
        for (_, label) in [
            (IntentStatus::Met, "met"),
            (IntentStatus::Missing, "missing"),
            (IntentStatus::Deviation, "deviation"),
            (IntentStatus::Breaking, "breaking"),
            (IntentStatus::Suggestion, "suggestion"),
            (IntentStatus::Unknown, "not assessed"),
        ] {
            assert!(
                out.contains(label),
                "checklist should contain status label {label}"
            );
        }
        assert!(out.contains("f1.rs:2"));
    }

    #[test]
    fn wrap_breaks_long_ascii_words() {
        let word = "a".repeat(100);
        let lines = wrap(&word, 10);
        assert!(lines.len() > 1);
        for l in &lines {
            assert!(display_width(l) <= 10, "line too wide: {l}");
        }
    }

    #[test]
    fn break_units_splits_cjk_per_char() {
        let units = break_units("中文abc");
        assert_eq!(units, vec!["中", "文", "abc"]);
    }

    #[test]
    fn human_count_boundaries() {
        assert_eq!(human_count(999), "999");
        assert_eq!(human_count(1000), "1.0k");
        assert_eq!(human_count(1500), "1.5k");
        assert_eq!(human_count(10000), "10k");
        assert_eq!(human_count(10500), "10k");
    }

    #[test]
    fn truncate_to_width_respects_cjk_and_does_not_split_chars() {
        assert_eq!(truncate_to_width("abc", 3), "abc");
        assert_eq!(truncate_to_width("abcd", 3), "abc");
        // 中文字符各宽 2，预算 3 只能容纳 1 个。
        assert_eq!(truncate_to_width("一二", 3), "一");
        assert_eq!(truncate_to_width("一二", 4), "一二");
        // 不会把多字节字符切成非法边界。
        assert_eq!(truncate_to_width("一", 1), "");
    }

    /// Issue 分诊的终端输出要和 `review` 一套视觉：同样的横幅、分区、宽度。
    /// 这里只钉住结构与「不泄漏内部字段名」，配色留给 Palette 自己的测试。
    #[test]
    fn issue_review_render_has_the_same_shape_as_review() {
        use reviewgate_core::issue::{
            ActionPolicy, IssueReviewDecision, IssueType, IssueVerdict, NormalizedIssue,
            PlannedActions, ReviewOutput,
        };
        let _ = ActionPolicy::default();
        let decision = IssueReviewDecision {
            issue_number: 1,
            primary_type: IssueType::Bug,
            type_confidence: 0.95,
            verdict: IssueVerdict::LikelyBug,
            confidence: 0.95,
            verification_ran: true,
            code_hits: vec![reviewgate_core::issue::CodeEvidence {
                path: "osscluster.go".into(),
                line: 39,
                snippet: "errWatchCrosslot = errors.New(\"redis: Watch requires all keys\")".into(),
            }],
            suggested_comment: "你好，谢谢反馈。\n\n这是评论正文。".into(),
            ..Default::default()
        };
        let planned = PlannedActions {
            post_or_update_comment: true,
            labels_to_add: vec!["bug".into()],
            close: false,
            close_reason: None,
            reasons_blocked: vec![],
            needs_human_notice: false,
            assign_to: None,
        };
        let out = ReviewOutput {
            decision,
            // 有错误签名才会有精确锚点——没签名时证据区会明说「没对上」
            normalized: NormalizedIssue {
                error_signatures: vec!["redis: Watch requires all keys".into()],
                ..Default::default()
            },
            content_hash: String::new(),
            comments_hash: String::new(),
            planned,
            technical: None,
        };
        let s = render_issue_review(&out, false, false);

        assert!(s.contains("━━"), "缺少横幅分隔线: {s}");
        assert!(s.contains("#1"), "缺少 Issue 号: {s}");
        assert!(s.contains("osscluster.go:39"), "证据要能点开核对: {s}");
        assert!(s.contains("这是评论正文。"), "要有评论预览: {s}");
        // 内部枚举名不该出现在给人看的输出里
        assert!(!s.contains("LIKELY_BUG"), "内部裁决名泄漏: {s}");
        assert!(!s.contains("post_or_update_comment"), "内部字段名泄漏: {s}");
    }

    /// 默认不打印 reasons 那一长串判定链，`--verbose` 才展开。
    #[test]
    fn issue_review_reasons_are_behind_verbose() {
        use reviewgate_core::issue::{
            IssueReviewDecision, NormalizedIssue, PlannedActions, ReviewOutput,
        };
        let decision = IssueReviewDecision {
            issue_number: 7,
            reasons: vec!["error_language".into(), "code_hits=18".into()],
            suggested_comment: "正文".into(),
            ..Default::default()
        };
        let out = ReviewOutput {
            decision,
            normalized: NormalizedIssue::default(),
            content_hash: String::new(),
            comments_hash: String::new(),
            planned: PlannedActions {
                post_or_update_comment: true,
                labels_to_add: vec![],
                close: false,
                close_reason: None,
                reasons_blocked: vec![],
                needs_human_notice: false,
                assign_to: None,
            },
            technical: None,
        };
        assert!(!render_issue_review(&out, false, false).contains("error_language"));
        assert!(render_issue_review(&out, true, false).contains("error_language"));
    }

    /// JSON 与文本必须是同一份信息的两种呈现——文本上看得到的，JSON 里都要能取到，
    /// 否则脚本化使用的人会以为「JSON 没有就是没发生」。
    #[test]
    fn issue_review_json_carries_what_the_text_shows() {
        use reviewgate_core::issue::{
            IssueReviewDecision, IssueType, IssueVerdict, NormalizedIssue, PlannedActions,
            ReviewOutput,
        };
        let decision = IssueReviewDecision {
            issue_number: 11,
            primary_type: IssueType::Unknown,
            type_confidence: 0.30,
            verdict: IssueVerdict::Unverified,
            confidence: 0.40,
            reasons: vec!["no_strong_signal".into()],
            suggested_comment: "你好，谢谢反馈。\n\n这条我判断不了。".into(),
            ..Default::default()
        };
        let planned = PlannedActions {
            post_or_update_comment: true,
            labels_to_add: vec!["needs-triage".into()],
            close: false,
            close_reason: None,
            reasons_blocked: vec!["low_confidence:0.40<0.50".into()],
            needs_human_notice: true,
            assign_to: Some("alice".into()),
        };
        let out = ReviewOutput {
            decision,
            normalized: NormalizedIssue::default(),
            content_hash: String::new(),
            comments_hash: String::new(),
            planned,
            technical: None,
        };
        let js = render_issue_review_json(&out, false).expect("json");
        let v: serde_json::Value = serde_json::from_str(&js).expect("parseable");

        assert_eq!(v["issue_number"], 11);
        assert_eq!(v["verdict"]["name"], "unverified");
        assert_eq!(v["type"]["name"], "unknown");
        assert_eq!(v["planned"]["assign_to"], "alice");
        assert_eq!(v["planned"]["hands_off_to_human"], true);
        assert_eq!(v["published"], false);
        // 文本里被 --verbose 折叠的判定链，JSON 始终带上（机器不需要折叠）
        assert_eq!(v["reasons"][0], "no_strong_signal");
        // 被拦下的原因要能编程判断，不能只有给人看的那句话
        assert_eq!(v["planned"]["blocked"][0], "low_confidence:0.40<0.50");
        assert!(v["comment"].as_str().unwrap().contains("判断不了"));
    }
}

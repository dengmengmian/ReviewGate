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
}

impl Palette {
    fn new() -> Self {
        // 颜色开关：尊重 `NO_COLOR`（任意值即关）；`FORCE_COLOR`/`CLICOLOR_FORCE` 可强制开
        // （管道/CI 里也上色）；否则按 stdout 是否为终端自适应。
        let no_color = std::env::var_os("NO_COLOR").is_some();
        let force = std::env::var_os("FORCE_COLOR").is_some()
            || std::env::var_os("CLICOLOR_FORCE").is_some();
        Palette {
            on: !no_color && (force || std::io::stdout().is_terminal()),
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

/// 渲染人类可读文本。`show_filtered` 时展开被过滤的低置信项。
pub fn render_text(outcome: &ReviewOutcome, show_filtered: bool) -> String {
    render_text_lang(outcome, show_filtered, Lang::detect())
}

/// 报告渲染的语言可注入版本（测试用，避免依赖进程 locale）。
fn render_text_lang(outcome: &ReviewOutcome, show_filtered: bool, t: Lang) -> String {
    let p = Palette::new();
    let mut out = String::new();

    if outcome.files_changed == 0 {
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
        }
        out.push_str(&format!("\n  {}\n", t.result_may_incomplete()));
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
        f.severity.as_str(),
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

        let text = render_text_lang(&outcome, false, Lang::En);
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
        let zh = render_text_lang(&outcome, false, Lang::Zh);
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
        };

        let text = render_text_lang(&outcome, false, Lang::En);
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
}

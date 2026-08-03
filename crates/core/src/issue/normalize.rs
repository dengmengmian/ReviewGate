//! Issue 正文标准化：清洗模板噪声，抽取错误签名与复现线索。

use super::model::NormalizedIssue;

/// 将原始 title/body 标准化为结构化字段。
pub fn normalize_issue(title: &str, body: &str) -> NormalizedIssue {
    let title = title.trim().to_string();
    let body_clean = clean_body(body);
    // 现象类字段从「去掉代码块」的正文里取——代码块是复现代码/日志，
    // 直接当现象会把 ```console …``` 整段塞进开场句。
    let prose = drop_fenced_blocks(&body_clean);
    let expected_behavior = extract_section(&prose, &["expected", "期望", "预期"]);
    let actual_behavior = extract_section(&prose, &["actual", "实际", "现象"]);
    let symptom = if !actual_behavior.is_empty() && !is_markdown_heading_only(&actual_behavior) {
        actual_behavior.clone()
    } else if let Some(s) = first_substantive_line(&prose) {
        s
    } else {
        // 正文几乎只有模板/图片时，用去噪标题作现象（常含真正的错误描述）
        strip_campaign_noise(&title)
    };
    let reproduction_steps = extract_steps(&body_clean);
    let error_signatures = extract_error_signatures(&body_clean);
    let stack_symbols = extract_stack_symbols(&body_clean);
    let environment = extract_environment(&body_clean);
    let embed_text = build_embed_text(
        &title,
        &symptom,
        &reproduction_steps,
        &error_signatures,
        &environment,
    );

    NormalizedIssue {
        title,
        body_clean,
        symptom,
        expected_behavior,
        actual_behavior,
        reproduction_steps,
        environment,
        error_signatures,
        stack_symbols,
        embed_text,
    }
}

/// Issue 模板留下的样板行：勾选清单、声明、指引。它们不是「现象」。
fn is_template_boilerplate(line: &str) -> bool {
    let l = line.trim();
    if l.starts_with("- [x]") || l.starts_with("- [ ]") || l.starts_with("* [") {
        return true;
    }
    let lower = l.to_ascii_lowercase();
    [
        "i have searched",
        "please complete",
        "before submitting",
        "checklist",
        "此问题已搜索",
        "提交前请",
    ]
    .iter()
    .any(|k| lower.contains(k))
}

/// 去掉围栏代码块。它们是复现代码或日志，不该被当成「现象」的正文。
fn drop_fenced_blocks(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut in_fence = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn clean_body(body: &str) -> String {
    // 有些平台粘贴会留下字面量 \n
    let body = body
        .replace("\\r\\n", "\n")
        .replace("\\n", "\n")
        .replace("\\r", "\n");
    let mut out = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        // 常见 Issue 模板提示行降权/移除
        let lower = t.to_ascii_lowercase();
        if lower.starts_with("please complete")
            || lower.starts_with("<!--")
            || lower == "_no response_"
            || lower.starts_with("delete this section")
        {
            continue;
        }
        out.push(t.to_string());
    }
    out.join("\n")
}

/// 跳过 Issue 模板栏目名、纯图片行，取第一条有信息量的描述。
fn first_substantive_line(s: &str) -> Option<String> {
    const SKIP: &[&str] = &[
        "问题描述",
        "期望的行为",
        "环境信息",
        "实际行为",
        "预期行为",
        "复现步骤",
        "遇到 bug 的页面",
        "无",
        "本地环境",
        "description",
        "expected",
        "actual",
        "steps",
        "environment",
    ];
    // 优先：含错误/崩溃/失败等实质描述的长句
    let mut candidates: Vec<&str> = s
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| !is_markdown_heading_only(l))
        .filter(|l| !is_template_boilerplate(l))
        .filter(|l| !is_image_or_asset_only_line(l))
        .filter(|l| {
            let low = l.to_ascii_lowercase();
            let stripped = l.trim_start_matches('#').trim().to_ascii_lowercase();
            !SKIP.iter().any(|k| {
                low == *k
                    || low == format!("{k}:")
                    || low == format!("{k}：")
                    || stripped == *k
                    || stripped == format!("{k}:")
                    || stripped == format!("{k}：")
            }) && l.chars().count() >= 8
                && !is_low_value_supplement_line(l)
        })
        .collect();
    // 打分：错误关键词 > 长度
    candidates.sort_by_key(|l| {
        let low = l.to_ascii_lowercase();
        let mut score = l.chars().count() as i32;
        for k in [
            "error", "失败", "中断", "崩溃", "panic", "bug", "无法", "连接", "超时", "空", "误删",
            "延迟",
        ] {
            if low.contains(k) {
                score += 40;
            }
        }
        -score
    });
    candidates.into_iter().next().map(|s| s.to_string())
}

fn is_low_value_supplement_line(l: &str) -> bool {
    let t = l.trim();
    // 「补充：当前是 pro 计划」这类元信息不当主症状
    if t.starts_with("补充") || t.starts_with("补充：") || t.starts_with("补充:") {
        let low = t.to_ascii_lowercase();
        if !low.contains("error")
            && !low.contains("失败")
            && !low.contains("崩溃")
            && !low.contains("无法")
            && !low.contains("连接")
            && t.chars().count() < 80
        {
            return true;
        }
    }
    if t.starts_with("相关 pr") || t.starts_with("相关 PR") || t.starts_with("相关pr") {
        return true;
    }
    false
}

fn is_markdown_heading_only(l: &str) -> bool {
    let t = l.trim();
    if t.starts_with('#') {
        return true;
    }
    // 纯栏目名（无实质正文）
    let stripped = t.trim_start_matches('#').trim();
    matches!(
        stripped,
        "问题描述"
            | "期望的行为"
            | "环境信息"
            | "实际行为"
            | "预期行为"
            | "复现步骤"
            | "Description"
            | "Expected"
            | "Actual"
            | "Environment"
            | "Steps"
    )
}

/// 图片 / 纯资源链接不当作现象摘要。
pub fn is_image_or_asset_only_line(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    if t.starts_with("![") || t.starts_with("<img") {
        return true;
    }
    let low = t.to_ascii_lowercase();
    if low.contains("user-images/assets")
        && (low.contains(".png")
            || low.contains(".jpg")
            || low.contains(".gif")
            || low.contains(".webp")
            || low.contains("image"))
    {
        return true;
    }
    // 整行几乎只有 markdown 图片
    if t.starts_with('!') && t.contains("](") && t.contains(')') {
        return true;
    }
    false
}

/// 去掉活动前缀 / 类型标签，便于查重与摘要（不改原始 title 存储）。
pub fn strip_campaign_noise(title: &str) -> String {
    let mut s = title.trim().to_string();
    // 反复剥 [xxx] 前缀（共创大赛 / Bug / Feature 等）
    loop {
        let t = s.trim_start();
        if let Some(rest) = t.strip_prefix('[') {
            if let Some(end) = rest.find(']') {
                s = rest[end + 1..].trim_start().to_string();
                continue;
            }
        }
        break;
    }
    s
}

fn extract_section(body: &str, keys: &[&str]) -> String {
    let lines: Vec<&str> = body.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let lower = line.to_ascii_lowercase();
        if keys.iter().any(|k| lower.contains(k)) {
            let mut collected = Vec::new();
            for next in lines.iter().skip(i + 1) {
                let t = next.trim();
                if t.is_empty() {
                    if !collected.is_empty() {
                        break;
                    }
                    continue;
                }
                if t.starts_with('#') || t.starts_with("##") {
                    break;
                }
                collected.push(t.to_string());
                if collected.len() >= 5 {
                    break;
                }
            }
            if !collected.is_empty() {
                return collected.join(" ");
            }
        }
    }
    String::new()
}

fn extract_steps(body: &str) -> Vec<String> {
    let mut steps = Vec::new();
    let mut in_steps = false;
    for line in body.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("repro")
            || lower.contains("steps")
            || lower.contains("复现")
            || lower.contains("重现")
        {
            in_steps = true;
            continue;
        }
        if in_steps {
            let t = line.trim();
            if t.is_empty() {
                if !steps.is_empty() {
                    break;
                }
                continue;
            }
            if t.starts_with('#') {
                break;
            }
            let cleaned = t
                .trim_start_matches(|c: char| {
                    c.is_ascii_digit() || c == '.' || c == '-' || c == '*'
                })
                .trim();
            if !cleaned.is_empty() {
                steps.push(cleaned.to_string());
            }
            if steps.len() >= 12 {
                break;
            }
        }
    }
    steps
}

/// 明显是源码而不是报错的行（注释、import、定义、方法链…）。
fn looks_like_source_line(t: &str) -> bool {
    let l = t.trim();
    if l.starts_with("//") || l.starts_with('#') || l.starts_with("* ") || l.starts_with('.') {
        return true;
    }
    let lower = l.to_ascii_lowercase();
    for kw in [
        "import ", "use ", "def ", "fn ", "func ", "class ", "let ", "const ", "var ", "pub ",
        "from ", "package ", "#include", "return ",
    ] {
        if lower.starts_with(kw) {
            return true;
        }
    }
    // 赋值语句几乎不会是错误消息（`f = tempfile.NamedTemporaryFile(...)`）
    if l.contains(" = ") && !looks_like_error_text(l) {
        return true;
    }
    // 形如 `foo::Bar` / `foo();` / 结尾是 `{` `}` `,` 的多半是代码
    (l.contains("::") && (l.ends_with(';') || l.ends_with(',')))
        || l.ends_with('{')
        || l.ends_with('}')
        || l.ends_with(',')
}

/// 看起来像一条错误消息。
fn looks_like_error_text(t: &str) -> bool {
    let l = t.to_ascii_lowercase();
    [
        "error",
        "exception",
        "traceback",
        "panic",
        "failed",
        "failure",
        "cannot",
        "can't",
        "unable",
        "invalid",
        "refused",
        "denied",
        "not found",
        "timeout",
        "timed out",
        "错误",
        "报错",
        "失败",
        "异常",
        "崩溃",
    ]
    .iter()
    .any(|k| l.contains(k))
}

/// 代码块前的引导语是否在说「下面是报错」。
fn intro_announces_error(intro: &str) -> bool {
    let l = intro.to_ascii_lowercase();
    [
        "error",
        "错误",
        "报错",
        "exception",
        "traceback",
        "失败",
        "failed",
        "returns",
        "输出",
        "output",
        "得到",
        "提示",
    ]
    .iter()
    .any(|k| l.contains(k))
}

fn extract_error_signatures(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let patterns = [
        "panic!",
        "segfault",
        "access violation",
        "nullpointerexception",
        "null pointer",
        "segmentation fault",
        "error:",
        "exception:",
        "traceback",
        "errno",
        "status code",
        "econnrefused",
        "timeout",
        "stream timeout",
        "error decoding",
        "connection reset",
        "connection refused",
        "网络连接中断",
        "远端关闭",
        "自动重连",
        "连接重置",
    ];
    // 代码围栏里的行可能是报错，也可能只是复现用的源码。只有前者算签名——
    // 把 `use foo::Bar;` 当成「报错片段」再拿去匹配源码，出来的结论全是噪声。
    let mut in_fence = false;
    let mut intro = String::new();
    let mut prev_nonempty = String::new();
    for line in body.lines() {
        let t = line.trim();
        if t.starts_with("```") {
            if !in_fence {
                intro = prev_nonempty.clone();
            }
            in_fence = !in_fence;
            continue;
        }
        if !t.is_empty() && !in_fence {
            prev_nonempty = t.to_string();
        }
        if in_fence
            && (8..=200).contains(&t.chars().count())
            && !looks_like_source_line(t)
            && (looks_like_error_text(t) || intro_announces_error(&intro))
        {
            out.push(t.to_string());
        }
    }
    // 命中已知模式时留下**整行**。模式词本身（"timeout"）匹配不到任何源码，
    // 整行（"read timeout after 30s"）才能和代码里的字符串字面量对上。
    let lower_patterns: Vec<String> = patterns.iter().map(|p| p.to_ascii_lowercase()).collect();
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() || t.chars().count() > 200 || t.starts_with("```") {
            continue;
        }
        let lt = t.to_ascii_lowercase();
        let Some(hit) = lower_patterns.iter().find(|p| lt.contains(p.as_str())) else {
            continue;
        };
        if !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
        // 整行用于精确对上长错误串；模式之后的核心片段用于对上源码里的字符串字面量
        // ——代码里往往只写了报错的一部分（`"网络连接中断:远端关闭"`）。
        if let Some(pos) = lt.find(hit.as_str()) {
            let tail = t[pos + hit.len()..]
                .trim_start_matches([':', '：', ' ', '\t'])
                .trim();
            if (4..=120).contains(&tail.chars().count()) && !out.iter().any(|o| o == tail) {
                out.push(tail.to_string());
            }
        }
    }
    // 捕获类似 ERROR_CODE=FOO 或 code 0xC0000005
    for token in body.split_whitespace() {
        let t = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-');
        if t.len() >= 6
            && (t.contains("ERR") || t.starts_with("0x") || t.contains("Exception"))
            && !out.iter().any(|x| x == t)
        {
            out.push(t.to_string());
        }
        if out.len() >= 16 {
            break;
        }
    }
    out
}

fn extract_stack_symbols(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in body.lines() {
        let t = line.trim();
        // 粗匹配函数/路径符号
        if t.contains("::") || t.contains(".rs:") || t.contains(".go:") || t.contains("at ") {
            let sym = t
                .trim_start_matches(|c: char| c == '#' || c.is_ascii_digit() || c == ' ')
                .to_string();
            if sym.len() > 4 && sym.len() < 200 {
                out.push(sym);
            }
        }
        if out.len() >= 20 {
            break;
        }
    }
    out
}

fn extract_environment(body: &str) -> serde_json::Value {
    let lower = body.to_ascii_lowercase();
    let mut map = serde_json::Map::new();
    for (key, needles) in [
        (
            "os",
            &[
                "windows",
                "linux",
                "macos",
                "ubuntu",
                "darwin",
                "本地环境",
                "win11",
                "win10",
            ][..],
        ),
        (
            "app_version",
            &["version", "v0.", "v1.", "atomcode ", "v5.", "v4."][..],
        ),
    ] {
        for n in needles {
            if lower.contains(n) || body.contains(n) {
                map.insert(
                    key.to_string(),
                    serde_json::Value::String(n.trim().to_string()),
                );
                break;
            }
        }
    }
    // 形如 5.0.2 / v5.0.2 / v9。按形状认版本号，不绑定具体产品名——
    // 只认带小数点的形式会漏掉 `v9`、`v10` 这类主版本号写法。
    for token in body.split_whitespace() {
        let t = token.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-');
        if t.len() > 16
            || !t
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
        {
            continue;
        }
        let dotted = t.chars().filter(|c| *c == '.').count() >= 1
            && t.chars().any(|c| c.is_ascii_digit())
            && t.len() >= 3
            && (t.starts_with('v')
                || t.chars()
                    .next()
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false));
        let v_major = t.len() >= 2
            && (t.starts_with('v') || t.starts_with('V'))
            && t[1..].chars().all(|c| c.is_ascii_digit());
        if dotted || v_major {
            map.entry("app_version".to_string())
                .or_insert_with(|| serde_json::Value::String(t.to_string()));
        }
    }
    serde_json::Value::Object(map)
}

fn build_embed_text(
    title: &str,
    symptom: &str,
    steps: &[String],
    errors: &[String],
    env: &serde_json::Value,
) -> String {
    // 嵌入用去噪标题，降低「共创大赛」等活动词绑架向量相似度
    let mut parts = vec![strip_campaign_noise(title), symptom.to_string()];
    if is_image_or_asset_only_line(symptom) {
        parts[1] = String::new();
    }
    if !steps.is_empty() {
        parts.push(steps.join(" "));
    }
    if !errors.is_empty() {
        parts.push(errors.join(" "));
    }
    if let Some(obj) = env.as_object() {
        for (k, v) in obj {
            parts.push(format!("{k}={v}"));
        }
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 线上回归（GitHub pallets/click#3740、clap-rs/clap#6421）：把 Issue 里贴的
    /// **源码**当成了错误签名，评论于是写出「Error fragment "use clap_complete::…"
    /// matches …」。代码块里大多是复现代码，不是报错。
    #[test]
    fn source_code_in_fences_is_not_an_error_signature() {
        let body = "Reproduce with:\n\n```rust\n                    use clap_complete::{generate, shells::Bash};\n                    let cmd = Command::new(\"x\").action(ArgAction::SetTrue),\n                    // Split streams by capabilities rather than the abstract TextIO\n                    def build_cli():\n```\n";
        let sigs = extract_error_signatures(body);
        for bad in [
            "use clap_complete",
            ".action(ArgAction",
            "// Split streams",
            "def build_cli",
        ] {
            assert!(
                !sigs.iter().any(|s| s.contains(bad)),
                "source line leaked into signatures ({bad}): {sigs:?}"
            );
        }
    }

    /// 但引导语说了「返回错误」的代码块，里面就是报错本体，仍要留下。
    #[test]
    fn fenced_text_introduced_as_an_error_is_kept() {
        let body = "在集群模式下对两个 key 调用 Watch，直接返回错误：\n\n                    ```\nredis: Watch requires all keys to be in the same slot\n```\n";
        let sigs = extract_error_signatures(body);
        assert!(
            sigs.iter().any(|s| s.contains("Watch requires all keys")),
            "{sigs:?}"
        );
    }

    /// 线上回归（AtomGit new_review/go-redis #10）：版本识别写死了 AtomCode 的
    /// 前缀（v0./v1./v4./v5.），`go-redis v9` 一个都不匹配，于是判「缺环境」，
    /// 反过来向一个 Go 库索要「模型名、是否代理」。
    #[test]
    fn version_is_recognized_by_shape_not_by_product() {
        for body in [
            // #10 的真实原文：只有 `v9`，没有小数点
            "## Actual\nParsing failed.\n\n## Environment\n- go-redis v9\n",
            "环境：sdk 2.3.1，服务端 7.0",
            "- 版本：v9.5\n",
        ] {
            let env = extract_environment(body);
            assert!(
                env.as_object().map(|o| !o.is_empty()).unwrap_or(false),
                "must detect a version in: {body:?} -> {env:?}"
            );
        }
    }

    #[test]
    fn prose_without_version_is_not_environment() {
        let env = extract_environment("点保存就崩溃了，帮忙看看");
        assert!(
            env.as_object().map(|o| o.is_empty()).unwrap_or(true),
            "{env:?}"
        );
    }

    /// 线上回归（AtomGit new_review/go-redis #1）：报错原文贴在 ``` 代码块里，
    /// 提取器却只返回硬编码的模式词（"timeout" 之类），实际错误文本一个字都没留下。
    /// 结果 fact_pack 的「错误签名 ↔ 源码」精确匹配从来对不上，只能回退成
    /// 「取前两条 code_hits」，于是贴出了相邻但无关的那一行。
    #[test]
    fn fenced_error_text_becomes_the_signature() {
        let body = "## 实际现象\n在集群模式下对两个 key 调用 Watch，直接返回错误：\n\n                    ```\nredis: Watch requires all keys to be in the same slot\n```\n";
        let sigs = extract_error_signatures(body);
        assert!(
            sigs.iter()
                .any(|s| s.contains("Watch requires all keys to be in the same slot")),
            "must keep the real error text: {sigs:?}"
        );
    }

    /// 命中已知模式时也要留下整行，而不是模式词本身——
    /// 「timeout」匹配不到任何源码，「read timeout after 30s」才能。
    #[test]
    fn pattern_hit_keeps_the_whole_line() {
        let sigs = extract_error_signatures("日志里看到 read timeout after 30s，然后就断了");
        assert!(
            sigs.iter().any(|s| s.contains("read timeout after 30s")),
            "must keep the line, not just the pattern word: {sigs:?}"
        );
    }

    #[test]
    fn image_first_line_not_used_as_symptom() {
        let body = r#"![image.png](https://raw.atomgit.com/user-images/assets//x/image.png 'image.png')

Auto 模式下 Deepseek 有时仍弹出手动确认。
"#;
        let n = normalize_issue("webui auto confirm bug", body);
        assert!(
            !n.symptom.contains("![") && !n.symptom.contains("user-images"),
            "symptom must skip image: {}",
            n.symptom
        );
        assert!(
            n.symptom.contains("确认")
                || n.symptom.contains("Deepseek")
                || n.symptom.contains("Auto"),
            "symptom should take text: {}",
            n.symptom
        );
    }

    #[test]
    fn low_value_supplement_not_preferred_over_error_line() {
        let body = r#"补充：当前是atomcode的pro计划

工作时间使用GLM，提示 API error 网络连接失败 次数较多
"#;
        let n = normalize_issue("[Bug] GLM 网络连接失败", body);
        assert!(
            !n.symptom.contains("pro计划") && !n.symptom.starts_with("补充"),
            "must not use plan supplement as symptom: {}",
            n.symptom
        );
        assert!(
            n.symptom.contains("网络") || n.symptom.contains("失败") || n.symptom.contains("GLM"),
            "prefer failure description: {}",
            n.symptom
        );
    }

    #[test]
    fn template_only_body_falls_back_to_title_symptom() {
        let body = r#"### 问题描述

![image.png](https://raw.atomgit.com/user-images/assets/x/image.png)

补充：当前是atomcode的pro计划

### 复现步骤

同上
"#;
        let n = normalize_issue(
            "[共创大赛][Bug] 工作时间使用GLM5.2，提示API error 网络连接失败次数较多",
            body,
        );
        assert!(
            !n.symptom.contains("###")
                && !n.symptom.contains("问题描述")
                && !n.symptom.contains("pro计划"),
            "bad symptom: {}",
            n.symptom
        );
        assert!(
            n.symptom.contains("网络") || n.symptom.contains("GLM") || n.symptom.contains("失败"),
            "title-derived symptom: {}",
            n.symptom
        );
    }

    #[test]
    fn literal_backslash_n_unescaped_in_body() {
        let body = "telemetry 描述不清。\\n\\n相关 PR: !845";
        let n = normalize_issue("docs: telemetry", body);
        assert!(
            !n.body_clean.contains("\\n"),
            "literal \\\\n should become newline: {}",
            n.body_clean
        );
        assert!(n.symptom.contains("telemetry") || n.body_clean.contains("telemetry"));
    }

    #[test]
    fn strip_campaign_noise_removes_contest_prefix() {
        let t = strip_campaign_noise("[共创大赛][Bug] request_user_input 直接返回错误");
        assert!(!t.contains("共创大赛"), "{t}");
        assert!(t.contains("request_user_input"), "{t}");
    }

    #[test]
    fn normalize_extracts_errors_and_steps() {
        let body = r#"
## Expected
save succeeds

## Actual
access violation crash

## Steps to reproduce
1. open settings
2. click save

## Environment
Windows 11, version 0.6.1
"#;
        let n = normalize_issue("Windows save crash", body);
        assert!(n
            .error_signatures
            .iter()
            .any(|e| e.contains("access violation")));
        assert!(n.reproduction_steps.len() >= 2);
        assert!(!n.embed_text.is_empty());
        assert!(n
            .body_clean
            .to_ascii_lowercase()
            .contains("access violation"));
    }
}

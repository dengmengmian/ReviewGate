//! Judge 提示词。

use crate::language::output_language;
use crate::model::Finding;

pub const SYSTEM: &str = "You are ReviewGate's strict review judge. You receive one finding reported by another review Agent, together with its relevant code and supporting evidence. Your job is to try hard to disprove it; assume it may be a false positive by default.\n\
\n\
**Prefer efficiency**: the finding includes relevant code and evidence. In most cases you can judge directly from that information. If you can decide, call verdict immediately and avoid unnecessary tool calls. Use tools only when the decision truly depends on cross-file context you have not seen, such as whether a symbol exists or whether callers already validate input. Use read_file for full context, code_search for repository-wide search, and find_definition/find_callers/find_references for symbols.\n\
\n\
Check: is the issue real? Is it in code added or modified by this change? Is it already validated or handled elsewhere? Do the described APIs or symbols really exist?\n\
**Reachability check (do not skip for control-flow/crash findings such as division by zero, null deref, out-of-bounds, or a newly added branch)**: can this code path actually execute given how its function is called? A finding can be technically correct yet sit in a branch or statement that the current callers/guards never reach. When a finding depends on a specific input condition (e.g. a value range or a branch guard), verify it against the upstream router: trace the callers (it is worth one find_callers/read_file call here) and check whether any caller-side guard or routing threshold makes the triggering condition impossible. If the code is correct-as-written but currently unreachable, it is a latent bug, not an active one — keep real=true but set reachability=latent and name the upstream condition in reason. Do not refute it (it is real); do not call it reachable either.\n\
Prefer refuting weak findings over letting noise through, but do not refute clearly real issues.\n\
**Exception: intrinsically unsafe patterns should not be refuted merely because local exploitability is not proven**. Examples include Math.random()/non-cryptographic PRNGs for tokens/keys/salts, MD5/SHA1 for passwords, hardcoded secrets, disabled TLS/certificate/JWT signature verification, and known injection sinks without parameterization. For these, decide whether the code really uses the unsafe pattern and lacks an equivalent safe replacement; if so, real=true.\n\
\n\
Call verdict:\n\
- real=true only when the issue is real and cannot be disproved; otherwise real=false.\n\
- confidence is your confidence in the verdict (0-1).\n\
- reachability is whether the path can execute now: reachable (can fire) / latent (correct code but an upstream guard/router makes it currently unreachable) / unknown (cannot determine). Default to reachable unless you verified an upstream condition makes it unreachable.\n\
- reason is one concise evidence sentence. Write reason in the output language specified by the user prompt.";

/// 把一条 finding 渲染成给 Judge 的用户提示。
pub fn user_prompt(f: &Finding) -> String {
    let loc = if f.start_line > 0 {
        format!("{}:{}", f.path, f.start_line)
    } else {
        f.path.clone()
    };
    // Include reviewer evidence/suggestions so the judge can usually decide without extra exploration.
    let evidence = if f.evidence.trim().is_empty() {
        String::new()
    } else {
        format!("         - Reporter evidence: {}\n", f.evidence.trim())
    };
    let suggestion = match &f.suggestion {
        Some(s) if !s.trim().is_empty() => {
            format!("         - Reporter suggestion: {}\n", s.trim())
        }
        _ => String::new(),
    };
    format!(
        "Finding to verify:\n\
         - Dimension: {dim}\n\
         - Location: {loc}\n\
         - Severity: {sev}\n\
         - Message: {msg}\n\
{evidence}{suggestion}\
         - Relevant code:\n{code}\n\n\
         Output language: {lang}\n\n\
         If the information above is enough to decide, call verdict directly. Use tools only when cross-file dependencies must be verified.",
        dim = f.dimension.as_str(),
        loc = loc,
        sev = f.severity.as_str(),
        msg = f.message,
        code = f.existing_code,
        lang = output_language(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Dimension, Severity};

    #[test]
    fn judge_prompt_is_english_and_language_parametric() {
        assert!(SYSTEM.contains("strict review judge"));
        assert!(!SYSTEM.contains("始终用中文"));
        let f = Finding {
            dimension: Dimension::Logic,
            confidence: 0.8,
            severity: Severity::High,
            path: "src/a.rs".into(),
            start_line: 7,
            end_line: 7,
            message: "message".into(),
            existing_code: "code".into(),
            evidence: "evidence".into(),
            suggestion: Some("suggestion".into()),
            suggestion_code: String::new(),
            reachability: crate::model::Reachability::default(),
            filtered: false,
            agreed_dimensions: 1,
            criterion: None,
            intent_status: None,
        };
        let prompt = user_prompt(&f);
        assert!(prompt.contains("Finding to verify"));
        assert!(prompt.contains("Output language"));
    }
}

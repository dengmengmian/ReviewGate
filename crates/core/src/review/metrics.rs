//! 单次运行质量指标：写本地 JSONL，供发版门槛与本地复盘。

use crate::gate::GateDecision;
use crate::model::Usage;
use crate::review::cost::CostEstimate;
use serde::Serialize;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// 一次审查的落盘指标。
#[derive(Debug, Clone, Serialize)]
pub struct RunMetrics {
    pub ts_unix: u64,
    pub decision: String,
    pub incomplete: bool,
    pub files_changed: usize,
    pub findings_total: usize,
    pub findings_kept: usize,
    pub warnings: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub est_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub est_cost_usd: Option<f64>,
    /// 关键路径未审完是否触发强制失败。
    pub critical_incomplete: bool,
}

impl RunMetrics {
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        decision: GateDecision,
        incomplete: bool,
        files_changed: usize,
        findings_total: usize,
        findings_kept: usize,
        warnings: usize,
        usage: &Usage,
        duration_ms: u64,
        profile: Option<&str>,
        cost: Option<&CostEstimate>,
        critical_incomplete: bool,
    ) -> Self {
        let ts_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            ts_unix,
            decision: decision.as_str().to_lowercase(),
            incomplete,
            files_changed,
            findings_total,
            findings_kept,
            warnings,
            input_tokens: usage.total_input() as u64,
            output_tokens: usage.output_tokens as u64,
            duration_ms,
            profile: profile.map(|s| s.to_string()),
            est_input_tokens: cost.map(|c| c.est_input_tokens),
            est_cost_usd: cost.and_then(|c| c.est_cost_usd),
            critical_incomplete,
        }
    }

    /// 追加写入 `.reviewgate/cache/metrics.jsonl`（目录自带 gitignore）。
    pub fn append_jsonl(&self, repo_root: &Path) -> std::io::Result<()> {
        let dir = repo_root.join(".reviewgate").join("cache");
        std::fs::create_dir_all(&dir)?;
        // Ensure cache dir is ignored if we create it fresh.
        let gi = dir.join(".gitignore");
        if !gi.exists() {
            let _ = std::fs::write(&gi, "*\n");
        }
        let path = dir.join("metrics.jsonl");
        let line = serde_json::to_string(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        writeln!(f, "{line}")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Usage;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn append_jsonl_writes_line() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("rg-metrics-{nanos}"));
        std::fs::create_dir_all(&root).unwrap();
        let m = RunMetrics::build(
            GateDecision::Block,
            false,
            2,
            3,
            1,
            0,
            &Usage::default(),
            42,
            Some("gate"),
            None,
            false,
        );
        m.append_jsonl(&root).unwrap();
        let text = std::fs::read_to_string(root.join(".reviewgate/cache/metrics.jsonl")).unwrap();
        assert!(text.contains("\"decision\":\"block\""));
        assert!(text.contains("\"duration_ms\":42"));
        std::fs::remove_dir_all(&root).ok();
    }
}

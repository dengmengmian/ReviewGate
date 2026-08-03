//! 发现会话：把一次 review 的结果落盘成可查询、可标记的状态文件。
//!
//! 解决的问题：审查结果原来只存在于一次运行的 stdout 里，agent 想逐条修就得重跑一次。
//! 落盘后，agent 的循环变成 `list → 修一条 → resolve → list`，不再重复烧 token。
//!
//! 位置 `.reviewgate/cache/findings.json`（与增量缓存同目录，自带 `.gitignore`）——
//! 它是**本地运行态**，不是团队共享产物；误报共享走 `.reviewgate/ignore` 指纹。
//!
//! 语义边界：一次 `reviewgate review` = 一个新 session，`resolve` 只在该 session 内有效。
//! 重跑后若问题仍在，它会以 open 重新出现——resolve 标记的是"这一轮已处理"，
//! 不是"永久消音"（那是 `.reviewgate/ignore` 的职责）。

use crate::model::Finding;
use crate::review::suppress::fingerprint;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 当前 session 文件格式版本。
///
/// v2 起每条记录带 `seq`（短序号）。改 schema 必须 bump——否则旧文件会以一个难懂的
/// 解析错误炸出来，而不是那句"重跑 reviewgate review"。
pub const SESSION_VERSION: u32 = 2;

/// 一条发现在本轮的处理状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FindingStatus {
    /// 尚未处理。
    Open,
    /// 本轮已处理（修了 / 判断为不用改）。
    Resolved,
}

/// 一条发现 + 本轮处理状态。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingRecord {
    /// 本轮序号（1 起）。给人和 agent 用的短别名——12 位指纹不好念、不好在对话里引用。
    /// 只在本 session 内有效；跨运行请用 `id`（指纹）。
    pub seq: usize,
    /// 稳定 ID：与 `.reviewgate/ignore` 用的是同一个指纹（同一处问题跨运行一致）。
    pub id: String,
    pub status: FindingStatus,
    /// resolve 时的备注（可选）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// resolve 时间（RFC3339）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
    /// 完整发现内容（含建议修复代码，供 agent 直接消费）。
    pub finding: Finding,
}

/// 一次审查的发现会话。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingSession {
    pub version: u32,
    /// 本次运行标识（创建时间的 epoch 秒，同秒重跑会覆盖——够用且无需随机数）。
    pub run_id: String,
    pub created_at: String,
    /// 闸口判定：pass | warn | block。
    pub decision: String,
    /// 本次审查时的 HEAD sha。`--since-last-review` 以它为基准只审之后新增的改动。
    /// 非 git 仓库或取不到时为 `None`——那时下一次增量审查会明确拒绝，而不是猜一个基准。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    pub files_changed: usize,
    /// 审查是否未完整——为 true 时"没有 open 发现"不等于"没问题"。
    pub incomplete: bool,
    pub records: Vec<FindingRecord>,
}

fn session_path(repo_root: &Path) -> PathBuf {
    repo_root
        .join(".reviewgate")
        .join("cache")
        .join("findings.json")
}

impl FindingSession {
    /// 从审查结果构造会话。`now_secs` 为 UNIX 秒（显式传入，便于测试）。
    pub fn new(
        findings: &[Finding],
        decision: &str,
        files_changed: usize,
        incomplete: bool,
        now_secs: u64,
        head_sha: Option<String>,
    ) -> Self {
        let mut records: Vec<FindingRecord> = Vec::with_capacity(findings.len());
        for f in findings {
            let id = unique_id(fingerprint(f), &records);
            records.push(FindingRecord {
                seq: records.len() + 1,
                id,
                status: FindingStatus::Open,
                note: None,
                resolved_at: None,
                finding: f.clone(),
            });
        }
        Self {
            version: SESSION_VERSION,
            run_id: now_secs.to_string(),
            created_at: crate::issue::format_unix_secs_rfc3339(now_secs),
            decision: decision.to_string(),
            head_sha,
            files_changed,
            incomplete,
            records,
        }
    }

    /// 写入 `.reviewgate/cache/findings.json`（目录自带 `.gitignore`，不会被自己审到）。
    pub fn save(&self, repo_root: &Path) -> Result<()> {
        let path = session_path(repo_root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
            let gi = parent.join(".gitignore");
            if !gi.exists() {
                std::fs::write(gi, "*\n")?;
            }
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
    }

    /// 读取会话。文件缺失 → 明确报错（而不是装作"没有发现"）。
    pub fn load(repo_root: &Path) -> Result<Self> {
        let path = session_path(repo_root);
        let text = std::fs::read_to_string(&path).with_context(|| {
            format!(
                "no review session at {} — run `reviewgate review` first",
                path.display()
            )
        })?;
        // 先只读版本号再严格解析：旧版本的字段结构不同，直接 strict parse 会先炸出
        // 一句看不懂的 JSON 错误，永远走不到下面那句"重跑 reviewgate review"。
        #[derive(Deserialize)]
        struct VersionProbe {
            version: u32,
        }
        let probe: VersionProbe = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        if probe.version != SESSION_VERSION {
            anyhow::bail!(
                "review session at {} has version {} (expected {}) — re-run `reviewgate review`",
                path.display(),
                probe.version,
                SESSION_VERSION
            );
        }
        let s: FindingSession = serde_json::from_str(&text)
            .with_context(|| format!("failed to parse {}", path.display()))?;
        Ok(s)
    }

    /// 按序号或 ID 前缀查找。纯数字先按本轮序号解释（`findings show 3`），
    /// 其余按指纹前缀（只敲前几位即可）。命中多条则报错，不猜。
    pub fn find(&self, id_or_seq: &str) -> Result<&FindingRecord> {
        let key = id_or_seq.trim();
        if let Ok(seq) = key.parse::<usize>() {
            return self
                .records
                .iter()
                .find(|r| r.seq == seq)
                .ok_or_else(|| anyhow::anyhow!("no finding #{seq} in the current review session"));
        }
        let hits: Vec<&FindingRecord> = self
            .records
            .iter()
            .filter(|r| r.id.starts_with(key))
            .collect();
        match hits.len() {
            1 => Ok(hits[0]),
            0 => anyhow::bail!("no finding with id `{key}` in the current review session"),
            n => anyhow::bail!(
                "id `{key}` is ambiguous ({n} matches: {})",
                hits.iter()
                    .map(|r| r.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    /// 标记为已处理。已是 resolved 时原样返回（幂等，agent 重试安全）。
    pub fn resolve(
        &mut self,
        id_or_seq: &str,
        note: Option<String>,
        now_secs: u64,
    ) -> Result<&FindingRecord> {
        let id = self.find(id_or_seq)?.id.clone();
        let rec = self
            .records
            .iter_mut()
            .find(|r| r.id == id)
            .expect("just located");
        if rec.status == FindingStatus::Open {
            rec.status = FindingStatus::Resolved;
            rec.resolved_at = Some(crate::issue::format_unix_secs_rfc3339(now_secs));
        }
        if note.is_some() {
            rec.note = note;
        }
        Ok(rec)
    }

    /// 按状态筛选。`include_filtered=false` 时排除被闸口过滤的低置信项（默认视图）。
    pub fn select(
        &self,
        status: Option<FindingStatus>,
        include_filtered: bool,
    ) -> Vec<&FindingRecord> {
        self.records
            .iter()
            .filter(|r| status.is_none_or(|s| r.status == s))
            .filter(|r| include_filtered || !r.finding.filtered)
            .collect()
    }
}

/// 指纹碰撞时补后缀，保证 session 内 ID 唯一（去重后仍可能出现同路径同维度同代码的两条）。
fn unique_id(base: String, existing: &[FindingRecord]) -> String {
    if !existing.iter().any(|r| r.id == base) {
        return base;
    }
    for n in 2..1000 {
        let candidate = format!("{base}-{n}");
        if !existing.iter().any(|r| r.id == candidate) {
            return candidate;
        }
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Dimension, Severity};

    fn finding(path: &str, code: &str) -> Finding {
        Finding {
            dimension: Dimension::Security,
            confidence: 0.9,
            severity: Severity::High,
            path: path.into(),
            start_line: 1,
            end_line: 1,
            message: "问题".into(),
            existing_code: code.into(),
            evidence: String::new(),
            suggestion: None,
            suggestion_code: String::new(),
            reachability: Default::default(),
            filtered: false,
            agreed_dimensions: 1,
            criterion: None,
            intent_status: None,
        }
    }

    fn tmp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rg_session_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn ids_are_fingerprints_and_unique() {
        let a = finding("a.rs", "let x = 1;");
        let b = finding("b.rs", "let y = 2;");
        let s = FindingSession::new(
            &[a.clone(), b, a.clone()],
            "block",
            2,
            false,
            1_700_000_000,
            None,
        );
        assert_eq!(s.records.len(), 3);
        assert_eq!(s.records[0].id, fingerprint(&a));
        // 同一处重复出现时 ID 加后缀，不覆盖。
        assert_ne!(s.records[0].id, s.records[2].id);
        assert!(s.records[2].id.starts_with(&s.records[0].id));
        assert!(s.records.iter().all(|r| r.status == FindingStatus::Open));
        assert_eq!(s.created_at, "2023-11-14T22:13:20Z");
    }

    #[test]
    fn save_load_roundtrip_and_gitignore_written() {
        let root = tmp_root("roundtrip");
        let s = FindingSession::new(
            &[finding("a.rs", "code")],
            "warn",
            1,
            false,
            1_700_000_000,
            Some("abc123".into()),
        );
        s.save(&root).unwrap();
        assert!(root.join(".reviewgate/cache/.gitignore").exists());
        let loaded = FindingSession::load(&root).unwrap();
        assert_eq!(loaded.records.len(), 1);
        assert_eq!(loaded.decision, "warn");
        assert_eq!(
            loaded.head_sha.as_deref(),
            Some("abc123"),
            "增量审查要靠它定基准，必须往返保真"
        );
        assert_eq!(loaded.records[0].finding.message, "问题");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stale_version_reports_the_version_not_a_parse_error() {
        // 旧版本的 session 缺少新字段。必须给出"重跑 reviewgate review"的可执行提示，
        // 而不是一句看不懂的 JSON 解析错误——那正是 bump 版本号要避免的。
        let root = tmp_root("stale");
        let cache = root.join(".reviewgate").join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(
            cache.join("findings.json"),
            r#"{"version":1,"run_id":"1","created_at":"x","decision":"block","files_changed":1,"incomplete":false,
                 "records":[{"id":"abc","status":"open","finding":{"dimension":"security","confidence":0.9,
                 "severity":"high","path":"a.rs","start_line":1,"end_line":1,"message":"m",
                 "existing_code":"x","evidence":"","suggestion_code":"","filtered":false,"agreed_dimensions":1}}]}"#,
        )
        .unwrap();
        let err = FindingSession::load(&root).unwrap_err().to_string();
        assert!(err.contains("version 1"), "应指明旧版本号：{err}");
        assert!(err.contains("reviewgate review"), "应给出下一步：{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn load_without_session_errors_clearly() {
        let root = tmp_root("missing");
        let err = FindingSession::load(&root).unwrap_err().to_string();
        assert!(err.contains("reviewgate review"), "错误应给出下一步：{err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolve_is_idempotent_and_prefix_addressable() {
        let mut s = FindingSession::new(
            &[finding("a.rs", "code")],
            "block",
            1,
            false,
            1_700_000_000,
            None,
        );
        let full = s.records[0].id.clone();
        let rec = s
            .resolve(&full[..4], Some("已修".into()), 1_700_000_060)
            .unwrap();
        assert_eq!(rec.status, FindingStatus::Resolved);
        assert_eq!(rec.note.as_deref(), Some("已修"));
        assert_eq!(rec.resolved_at.as_deref(), Some("2023-11-14T22:14:20Z"));
        // 再 resolve 一次不改时间、不报错。
        let rec = s.resolve(&full, None, 1_700_009_999).unwrap();
        assert_eq!(rec.resolved_at.as_deref(), Some("2023-11-14T22:14:20Z"));
    }

    #[test]
    fn sequence_numbers_are_stable_short_aliases() {
        let mut s = FindingSession::new(
            &[finding("a.rs", "one"), finding("b.rs", "two")],
            "block",
            2,
            false,
            1,
            None,
        );
        assert_eq!(s.records[0].seq, 1);
        assert_eq!(s.records[1].seq, 2);
        // 序号可直接寻址，和指纹前缀等价。
        assert_eq!(s.find("2").unwrap().id, s.records[1].id);
        assert_eq!(s.find(&s.records[1].id[..4]).unwrap().seq, 2);
        assert!(
            s.find("99").is_err(),
            "不存在的序号必须报错而不是回退成前缀匹配"
        );
        let rec = s.resolve("1", None, 2).unwrap();
        assert_eq!(rec.seq, 1);
        assert_eq!(rec.status, FindingStatus::Resolved);
    }

    #[test]
    fn unknown_id_errors_instead_of_guessing() {
        let mut s = FindingSession::new(&[finding("a.rs", "code")], "block", 1, false, 1, None);
        assert!(s.resolve("zzzz", None, 2).is_err());
        assert!(s.find("zzzz").is_err());
    }

    #[test]
    fn select_filters_by_status_and_hides_filtered_by_default() {
        let mut low = finding("b.rs", "other");
        low.filtered = true;
        let mut s = FindingSession::new(&[finding("a.rs", "code"), low], "warn", 2, false, 1, None);
        assert_eq!(s.select(None, false).len(), 1, "默认隐藏被过滤项");
        assert_eq!(s.select(None, true).len(), 2);
        let id = s.records[0].id.clone();
        s.resolve(&id, None, 2).unwrap();
        assert_eq!(s.select(Some(FindingStatus::Open), false).len(), 0);
        assert_eq!(s.select(Some(FindingStatus::Resolved), false).len(), 1);
    }
}

//! 本地 Issue 存储：SQLite 元数据 + FTS5 + embedding BLOB（语义召回）。

use super::model::{DuplicateCandidate, IssueReviewDecision, StoredIssue};
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS issues (
    repo_id TEXT NOT NULL,
    issue_number INTEGER NOT NULL,
    title TEXT NOT NULL,
    body_raw TEXT NOT NULL DEFAULT '',
    body_clean TEXT NOT NULL DEFAULT '',
    state TEXT NOT NULL DEFAULT 'open',
    labels_json TEXT NOT NULL DEFAULT '[]',
    author TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT '',
    updated_at TEXT NOT NULL DEFAULT '',
    closed_at TEXT,
    error_signature TEXT NOT NULL DEFAULT '',
    stack_symbols_json TEXT NOT NULL DEFAULT '[]',
    source_updated_at TEXT NOT NULL DEFAULT '',
    content_hash TEXT NOT NULL DEFAULT '',
    comments_hash TEXT NOT NULL DEFAULT '',
    embedding BLOB,
    embedding_model TEXT,
    embedding_version TEXT,
    embedding_content_hash TEXT,
    last_synced_at TEXT NOT NULL DEFAULT '',
    last_reviewed_at TEXT,
    PRIMARY KEY (repo_id, issue_number)
);

CREATE TABLE IF NOT EXISTS issue_reviews (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id TEXT NOT NULL,
    issue_number INTEGER NOT NULL,
    analyzer_version TEXT NOT NULL,
    decision_json TEXT NOT NULL,
    content_hash TEXT NOT NULL DEFAULT '',
    comments_hash TEXT NOT NULL DEFAULT '',
    analyzed_at TEXT NOT NULL,
    published_comment_id TEXT,
    UNIQUE(repo_id, issue_number, analyzer_version, content_hash)
);

-- 动作审计：每次判定都落一行，包括被闸门拦下、最终什么都没做的那些。
CREATE TABLE IF NOT EXISTS issue_action_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    repo_id TEXT NOT NULL,
    issue_number INTEGER NOT NULL,
    decided_at TEXT NOT NULL,
    primary_type TEXT NOT NULL DEFAULT '',
    verdict TEXT NOT NULL DEFAULT '',
    confidence REAL NOT NULL DEFAULT 0,
    planned_comment INTEGER NOT NULL DEFAULT 0,
    planned_close INTEGER NOT NULL DEFAULT 0,
    labels_json TEXT NOT NULL DEFAULT '[]',
    blocked_json TEXT NOT NULL DEFAULT '[]',
    executed INTEGER NOT NULL DEFAULT 0,
    published_comment_id TEXT
);

CREATE INDEX IF NOT EXISTS idx_audit_repo_time
    ON issue_action_audit(repo_id, decided_at);

CREATE TABLE IF NOT EXISTS issue_sync_state (
    repository_id TEXT PRIMARY KEY,
    last_successful_sync_at TEXT,
    sync_started_at TEXT,
    sync_completed_at TEXT,
    index_version TEXT NOT NULL DEFAULT '1'
);

CREATE VIRTUAL TABLE IF NOT EXISTS issues_fts USING fts5(
    title,
    body_clean,
    error_signature,
    content='issues',
    content_rowid='rowid'
);

CREATE TRIGGER IF NOT EXISTS issues_ai AFTER INSERT ON issues BEGIN
  INSERT INTO issues_fts(rowid, title, body_clean, error_signature)
  VALUES (new.rowid, new.title, new.body_clean, new.error_signature);
END;
CREATE TRIGGER IF NOT EXISTS issues_ad AFTER DELETE ON issues BEGIN
  INSERT INTO issues_fts(issues_fts, rowid, title, body_clean, error_signature)
  VALUES ('delete', old.rowid, old.title, old.body_clean, old.error_signature);
END;
CREATE TRIGGER IF NOT EXISTS issues_au AFTER UPDATE ON issues BEGIN
  INSERT INTO issues_fts(issues_fts, rowid, title, body_clean, error_signature)
  VALUES ('delete', old.rowid, old.title, old.body_clean, old.error_signature);
  INSERT INTO issues_fts(rowid, title, body_clean, error_signature)
  VALUES (new.rowid, new.title, new.body_clean, new.error_signature);
END;
"#;

/// 反向错误匹配的最短签名长度：太短的片段会把无关 Issue 也拽进来。
const MIN_REVERSE_MATCH_LEN: usize = 16;

/// 反向匹配最多回扫多少条历史 Issue。
const REVERSE_MATCH_SCAN: usize = 500;

/// Issue 预处理动作统计（按仓库）。
#[derive(Debug, Clone, Default)]
pub struct ActionStats {
    pub total: usize,
    /// 计划发言的条数（不代表已发出，见 `executed`）。
    pub commented: usize,
    pub closed: usize,
    /// 真正对平台执行过动作的条数。
    pub executed: usize,
    /// 因置信度不足被闸门拦下的条数。
    pub gated_low_confidence: usize,
    pub avg_confidence: f32,
    pub by_verdict: Vec<(String, usize)>,
}

/// 一条等待人工接手的记录。
#[derive(Debug, Clone)]
pub struct GatedIssue {
    pub issue_number: u64,
    pub decided_at: String,
    pub primary_type: String,
    pub verdict: String,
    pub confidence: f32,
    pub handed_off: bool,
}

/// 每个仓库一份本地库：`<data_dir>/issues.db`。
pub struct IssueStore {
    conn: Connection,
    pub repo_id: String,
    pub path: PathBuf,
}

impl IssueStore {
    /// 打开或创建仓库 Issue 库。
    pub fn open(data_dir: &Path, repo_id: &str) -> Result<Self> {
        std::fs::create_dir_all(data_dir)
            .with_context(|| format!("create issue data dir {}", data_dir.display()))?;
        let path = data_dir.join("issues.db");
        let conn =
            Connection::open(&path).with_context(|| format!("open sqlite {}", path.display()))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn,
            repo_id: repo_id.to_string(),
            path,
        })
    }

    /// 内存库（单测）。
    pub fn open_in_memory(repo_id: &str) -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn,
            repo_id: repo_id.to_string(),
            path: PathBuf::from(":memory:"),
        })
    }

    pub fn upsert_issue(&self, issue: &StoredIssue) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO issues (
                repo_id, issue_number, title, body_raw, body_clean, state, labels_json,
                author, created_at, updated_at, closed_at, error_signature, stack_symbols_json,
                source_updated_at, content_hash, comments_hash, embedding, embedding_model,
                embedding_version, embedding_content_hash, last_synced_at, last_reviewed_at
            ) VALUES (
                ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22
            )
            ON CONFLICT(repo_id, issue_number) DO UPDATE SET
                title=excluded.title,
                body_raw=excluded.body_raw,
                body_clean=excluded.body_clean,
                state=excluded.state,
                labels_json=excluded.labels_json,
                author=excluded.author,
                created_at=excluded.created_at,
                updated_at=excluded.updated_at,
                closed_at=excluded.closed_at,
                error_signature=excluded.error_signature,
                stack_symbols_json=excluded.stack_symbols_json,
                source_updated_at=excluded.source_updated_at,
                content_hash=excluded.content_hash,
                comments_hash=excluded.comments_hash,
                -- 正文变更且本次未写入新 embedding 时必须清空，禁止沿用旧向量
                embedding=CASE
                    WHEN excluded.embedding IS NOT NULL THEN excluded.embedding
                    WHEN excluded.content_hash != issues.content_hash THEN NULL
                    ELSE issues.embedding
                END,
                embedding_model=CASE
                    WHEN excluded.embedding IS NOT NULL THEN excluded.embedding_model
                    WHEN excluded.content_hash != issues.content_hash THEN NULL
                    ELSE issues.embedding_model
                END,
                embedding_version=CASE
                    WHEN excluded.embedding IS NOT NULL THEN excluded.embedding_version
                    WHEN excluded.content_hash != issues.content_hash THEN NULL
                    ELSE issues.embedding_version
                END,
                embedding_content_hash=CASE
                    WHEN excluded.embedding IS NOT NULL THEN excluded.embedding_content_hash
                    WHEN excluded.content_hash != issues.content_hash THEN NULL
                    ELSE issues.embedding_content_hash
                END,
                last_synced_at=excluded.last_synced_at,
                last_reviewed_at=COALESCE(excluded.last_reviewed_at, issues.last_reviewed_at)
            "#,
            params![
                issue.repo_id,
                issue.issue_number as i64,
                issue.title,
                issue.body_raw,
                issue.body_clean,
                issue.state,
                issue.labels_json,
                issue.author,
                issue.created_at,
                issue.updated_at,
                issue.closed_at,
                issue.error_signature,
                issue.stack_symbols_json,
                issue.source_updated_at,
                issue.content_hash,
                issue.comments_hash,
                issue.embedding,
                issue.embedding_model,
                issue.embedding_version,
                issue.embedding_content_hash,
                issue.last_synced_at,
                issue.last_reviewed_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_embedding(
        &self,
        issue_number: u64,
        embedding: &[u8],
        model: &str,
        version: &str,
        content_hash: &str,
    ) -> Result<()> {
        self.conn.execute(
            r#"UPDATE issues SET embedding=?1, embedding_model=?2, embedding_version=?3,
                embedding_content_hash=?4 WHERE repo_id=?5 AND issue_number=?6"#,
            params![
                embedding,
                model,
                version,
                content_hash,
                self.repo_id,
                issue_number as i64
            ],
        )?;
        Ok(())
    }

    pub fn get_issue(&self, issue_number: u64) -> Result<Option<StoredIssue>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT repo_id, issue_number, title, body_raw, body_clean, state, labels_json,
                      author, created_at, updated_at, closed_at, error_signature, stack_symbols_json,
                      source_updated_at, content_hash, comments_hash, embedding, embedding_model,
                      embedding_version, embedding_content_hash, last_synced_at, last_reviewed_at
               FROM issues WHERE repo_id=?1 AND issue_number=?2"#,
        )?;
        let row = stmt
            .query_row(params![self.repo_id, issue_number as i64], |r| {
                Ok(StoredIssue {
                    repo_id: r.get(0)?,
                    issue_number: r.get::<_, i64>(1)? as u64,
                    title: r.get(2)?,
                    body_raw: r.get(3)?,
                    body_clean: r.get(4)?,
                    state: r.get(5)?,
                    labels_json: r.get(6)?,
                    author: r.get(7)?,
                    created_at: r.get(8)?,
                    updated_at: r.get(9)?,
                    closed_at: r.get(10)?,
                    error_signature: r.get(11)?,
                    stack_symbols_json: r.get(12)?,
                    source_updated_at: r.get(13)?,
                    content_hash: r.get(14)?,
                    comments_hash: r.get(15)?,
                    embedding: r.get(16)?,
                    embedding_model: r.get(17)?,
                    embedding_version: r.get(18)?,
                    embedding_content_hash: r.get(19)?,
                    last_synced_at: r.get(20)?,
                    last_reviewed_at: r.get(21)?,
                })
            })
            .optional()?;
        Ok(row)
    }

    pub fn list_issue_numbers(&self) -> Result<Vec<u64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT issue_number FROM issues WHERE repo_id=?1 ORDER BY issue_number")?;
        let rows = stmt.query_map(params![self.repo_id], |r| Ok(r.get::<_, i64>(0)? as u64))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// 还没 triage 过的 Issue 号，**旧的优先**，最多 `limit` 条。
    ///
    /// 长跑模式据此分批消化积压：一轮只处理一部分，剩下的留在库里，下一轮继续——
    /// 不会因为一次同步进来几百条就一口气全跑掉。
    pub fn untriaged_issues(&self, limit: usize) -> Result<Vec<u64>> {
        let mut stmt = self.conn.prepare(
            "SELECT issue_number FROM issues
             WHERE repo_id=?1 AND last_reviewed_at IS NULL
             ORDER BY issue_number LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![self.repo_id, limit as i64], |r| {
            Ok(r.get::<_, i64>(0)? as u64)
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// FTS5 全文候选。
    pub fn fts_search(&self, query: &str, limit: usize) -> Result<Vec<DuplicateCandidate>> {
        let q = sanitize_fts_query(query);
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            r#"
            SELECT i.issue_number, i.title, i.error_signature, bm25(issues_fts) AS rank
            FROM issues_fts
            JOIN issues i ON i.rowid = issues_fts.rowid
            WHERE issues_fts MATCH ?1 AND i.repo_id = ?2
            ORDER BY rank
            LIMIT ?3
            "#,
        )?;
        let rows = stmt.query_map(params![q, self.repo_id, limit as i64], |r| {
            let rank: f64 = r.get(3)?;
            // bm25 越低越好 → 转成 0..1 分数
            let score = (1.0 / (1.0 + rank.abs())) as f32;
            Ok(DuplicateCandidate {
                issue_number: r.get::<_, i64>(0)? as u64,
                title: r.get(1)?,
                score,
                sources: vec!["fts5".into()],
                error_signature: r.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// 错误签名精确匹配。
    /// 反向精确匹配：用**当前正文**去命中历史 Issue 已提取的错误签名。
    ///
    /// 正向匹配要求提问者自己的报错能被提取成签名；可现实里很多人是把错误串
    /// 揉在一句口语里（「直接返回 redis: xxx，没法用」），提取不出签名，
    /// 于是和早先规规矩矩贴了报错的那条对不上。方向反过来就能接上。
    pub fn reverse_error_match(
        &self,
        body: &str,
        exclude: u64,
        limit: usize,
    ) -> Result<Vec<DuplicateCandidate>> {
        if body.trim().len() < MIN_REVERSE_MATCH_LEN {
            return Ok(Vec::new());
        }
        // error_signature 字段存的是多条签名 join(",") 后的串，SQL 侧没法直接比，
        // 拉回来按条拆开再比。
        let mut stmt = self.conn.prepare(
            r#"SELECT issue_number, title, error_signature FROM issues
               WHERE repo_id=?1 AND issue_number!=?2 AND error_signature!=''
               ORDER BY issue_number DESC LIMIT ?3"#,
        )?;
        let rows = stmt.query_map(
            params![self.repo_id, exclude as i64, REVERSE_MATCH_SCAN as i64],
            |r| {
                Ok((
                    r.get::<_, i64>(0)? as u64,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )?;
        let mut out = Vec::new();
        for row in rows {
            let (number, title, sigs) = row?;
            let Some(hit) = sigs
                .split(',')
                .map(str::trim)
                .filter(|s| s.chars().count() >= MIN_REVERSE_MATCH_LEN)
                .find(|s| body.contains(*s))
            else {
                continue;
            };
            out.push(DuplicateCandidate {
                issue_number: number,
                title,
                score: 0.9,
                sources: vec!["exact_error".into()],
                error_signature: hit.to_string(),
            });
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    pub fn exact_error_match(
        &self,
        signatures: &[String],
        exclude: u64,
        limit: usize,
    ) -> Result<Vec<DuplicateCandidate>> {
        if signatures.is_empty() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for sig in signatures {
            if sig.trim().is_empty() {
                continue;
            }
            let mut stmt = self.conn.prepare(
                r#"SELECT issue_number, title, error_signature FROM issues
                   WHERE repo_id=?1 AND issue_number!=?2
                     AND (error_signature LIKE ?3 OR body_clean LIKE ?3)
                   LIMIT ?4"#,
            )?;
            let pat = format!("%{sig}%");
            let rows = stmt.query_map(
                params![self.repo_id, exclude as i64, pat, limit as i64],
                |r| {
                    Ok(DuplicateCandidate {
                        issue_number: r.get::<_, i64>(0)? as u64,
                        title: r.get(1)?,
                        score: 0.92,
                        sources: vec!["exact_error".into()],
                        error_signature: r.get(2)?,
                    })
                },
            )?;
            for row in rows {
                out.push(row?);
            }
        }
        Ok(out)
    }

    /// 向量语义候选：余弦相似度（embedding BLOB = little-endian f32 序列）。
    pub fn vector_search(
        &self,
        query_embedding: &[f32],
        exclude: u64,
        limit: usize,
        min_similarity: f32,
    ) -> Result<Vec<DuplicateCandidate>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT issue_number, title, error_signature, embedding FROM issues
               WHERE repo_id=?1 AND issue_number!=?2 AND embedding IS NOT NULL"#,
        )?;
        let rows = stmt.query_map(params![self.repo_id, exclude as i64], |r| {
            Ok((
                r.get::<_, i64>(0)? as u64,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Vec<u8>>(3)?,
            ))
        })?;
        let mut scored = Vec::new();
        for row in rows {
            let (num, title, err, blob) = row?;
            let emb = bytes_to_f32s(&blob);
            if emb.is_empty() || emb.len() != query_embedding.len() {
                continue;
            }
            let sim = cosine_similarity(query_embedding, &emb);
            if sim >= min_similarity {
                scored.push(DuplicateCandidate {
                    issue_number: num,
                    title,
                    score: sim,
                    sources: vec!["vector".into()],
                    error_signature: err,
                });
            }
        }
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(limit);
        Ok(scored)
    }

    pub fn save_review(
        &self,
        decision: &IssueReviewDecision,
        content_hash: &str,
        comments_hash: &str,
        published_comment_id: Option<&str>,
    ) -> Result<()> {
        let now = chrono_like_now();
        let json = serde_json::to_string(decision)?;
        self.conn.execute(
            r#"INSERT INTO issue_reviews (
                repo_id, issue_number, analyzer_version, decision_json,
                content_hash, comments_hash, analyzed_at, published_comment_id
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
            ON CONFLICT(repo_id, issue_number, analyzer_version, content_hash) DO UPDATE SET
                decision_json=excluded.decision_json,
                comments_hash=excluded.comments_hash,
                analyzed_at=excluded.analyzed_at,
                published_comment_id=COALESCE(excluded.published_comment_id, issue_reviews.published_comment_id)
            "#,
            params![
                self.repo_id,
                decision.issue_number as i64,
                decision.analyzer_version,
                json,
                content_hash,
                comments_hash,
                now,
                published_comment_id,
            ],
        )?;
        self.conn.execute(
            "UPDATE issues SET last_reviewed_at=?1 WHERE repo_id=?2 AND issue_number=?3",
            params![now, self.repo_id, decision.issue_number as i64],
        )?;
        Ok(())
    }

    /// 记录一次判定与它实际引发（或未引发）的动作。
    /// `executed` 为假表示只是预演或被策略/闸门挡下——这类恰恰最需要留痕。
    pub fn record_action_audit(
        &self,
        decision: &IssueReviewDecision,
        planned: &super::action::PlannedActions,
        executed: bool,
        published_comment_id: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO issue_action_audit (
                repo_id, issue_number, decided_at, primary_type, verdict, confidence,
                planned_comment, planned_close, labels_json, blocked_json,
                executed, published_comment_id
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)"#,
            params![
                self.repo_id,
                decision.issue_number as i64,
                chrono_like_now(),
                decision.primary_type.as_str(),
                decision.verdict.as_str(),
                decision.confidence as f64,
                planned.post_or_update_comment as i64,
                planned.close as i64,
                serde_json::to_string(&planned.labels_to_add)?,
                serde_json::to_string(&planned.reasons_blocked)?,
                executed as i64,
                published_comment_id,
            ],
        )?;
        Ok(())
    }

    /// 本仓库的预处理统计（长跑模式下用来看闸门到底拦下了多少）。
    pub fn action_stats(&self) -> Result<ActionStats> {
        let mut stats = ActionStats::default();
        let mut stmt = self.conn.prepare(
            r#"SELECT verdict, confidence, planned_comment, planned_close, blocked_json, executed
               FROM issue_action_audit WHERE repo_id=?1"#,
        )?;
        let rows = stmt.query_map(params![self.repo_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, f64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })?;
        let mut conf_sum = 0.0f64;
        let mut by_verdict: std::collections::BTreeMap<String, usize> = Default::default();
        for row in rows {
            let (verdict, conf, planned_comment, planned_close, blocked, executed) = row?;
            stats.total += 1;
            conf_sum += conf;
            *by_verdict.entry(verdict).or_default() += 1;
            if planned_comment == 1 {
                stats.commented += 1;
            }
            if planned_close == 1 {
                stats.closed += 1;
            }
            if executed == 1 {
                stats.executed += 1;
            }
            if blocked.contains("low_confidence") {
                stats.gated_low_confidence += 1;
            }
        }
        if stats.total > 0 {
            stats.avg_confidence = (conf_sum / stats.total as f64) as f32;
        }
        stats.by_verdict = by_verdict.into_iter().collect();
        Ok(stats)
    }

    /// 被闸门拦下、等待人工接手的 Issue（每条只取最近一次判定）。
    /// 统计数字回答「有多少」，这个回答「是哪几条」——不然没人接得住。
    pub fn gated_issues(&self) -> Result<Vec<GatedIssue>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT issue_number, MAX(decided_at) AS t, primary_type, verdict, confidence, planned_comment
               FROM issue_action_audit
               WHERE repo_id=?1 AND blocked_json LIKE '%low_confidence%'
               GROUP BY issue_number
               ORDER BY issue_number DESC"#,
        )?;
        let rows = stmt.query_map(params![self.repo_id], |r| {
            Ok(GatedIssue {
                issue_number: r.get::<_, i64>(0)? as u64,
                decided_at: r.get(1)?,
                primary_type: r.get(2)?,
                verdict: r.get(3)?,
                confidence: r.get::<_, f64>(4)? as f32,
                handed_off: r.get::<_, i64>(5)? == 1,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn latest_review(&self, issue_number: u64) -> Result<Option<IssueReviewDecision>> {
        let mut stmt = self.conn.prepare(
            r#"SELECT decision_json FROM issue_reviews
               WHERE repo_id=?1 AND issue_number=?2
               ORDER BY analyzed_at DESC LIMIT 1"#,
        )?;
        let row: Option<String> = stmt
            .query_row(params![self.repo_id, issue_number as i64], |r| r.get(0))
            .optional()?;
        match row {
            Some(j) => Ok(Some(serde_json::from_str(&j)?)),
            None => Ok(None),
        }
    }

    pub fn set_published_comment(
        &self,
        issue_number: u64,
        content_hash: &str,
        comment_id: &str,
    ) -> Result<()> {
        self.conn.execute(
            r#"UPDATE issue_reviews SET published_comment_id=?1
               WHERE repo_id=?2 AND issue_number=?3 AND content_hash=?4"#,
            params![comment_id, self.repo_id, issue_number as i64, content_hash],
        )?;
        Ok(())
    }

    pub fn get_sync_cursor(&self) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT last_successful_sync_at FROM issue_sync_state WHERE repository_id=?1",
        )?;
        let v: Option<Option<String>> = stmt
            .query_row(params![self.repo_id], |r| r.get(0))
            .optional()?;
        Ok(v.flatten())
    }

    pub fn set_sync_cursor(&self, ts: &str) -> Result<()> {
        let now = chrono_like_now();
        self.conn.execute(
            r#"INSERT INTO issue_sync_state (repository_id, last_successful_sync_at, sync_started_at, sync_completed_at, index_version)
               VALUES (?1,?2,?3,?4,'1')
               ON CONFLICT(repository_id) DO UPDATE SET
                 last_successful_sync_at=excluded.last_successful_sync_at,
                 sync_completed_at=excluded.sync_completed_at"#,
            params![self.repo_id, ts, now, now],
        )?;
        Ok(())
    }

    pub fn count_issues(&self) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM issues WHERE repo_id=?1",
            params![self.repo_id],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }
}

/// FTS5 查询消毒：拆词并 OR 连接，去掉特殊字符。
pub fn sanitize_fts_query(raw: &str) -> String {
    let tokens: Vec<String> = raw
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|t| t.len() >= 2)
        .take(12)
        .map(|t| t.to_string())
        .collect();
    tokens.join(" OR ")
}

pub fn f32s_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(v.len() * 4);
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

pub fn bytes_to_f32s(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

fn chrono_like_now() -> String {
    // 避免引入 chrono 依赖：RFC3339-ish UTC via system time
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::issue::embedding::{embed_local, EMBED_MODEL, EMBED_VERSION};
    use crate::issue::hash::content_hash;
    use crate::issue::model::IssueVerdict;

    fn sample(num: u64, title: &str, body: &str, err: &str) -> StoredIssue {
        let emb = embed_local(&format!("{title}\n{body}\n{err}"));
        StoredIssue {
            repo_id: "o/r".into(),
            issue_number: num,
            title: title.into(),
            body_raw: body.into(),
            body_clean: body.into(),
            state: "open".into(),
            labels_json: "[]".into(),
            author: "u".into(),
            created_at: "t".into(),
            updated_at: "t".into(),
            closed_at: None,
            error_signature: err.into(),
            stack_symbols_json: "[]".into(),
            source_updated_at: "t".into(),
            content_hash: content_hash(title, body),
            comments_hash: "x".into(),
            embedding: Some(f32s_to_bytes(&emb)),
            embedding_model: Some(EMBED_MODEL.into()),
            embedding_version: Some(EMBED_VERSION.into()),
            embedding_content_hash: Some(content_hash(title, body)),
            last_synced_at: "t".into(),
            last_reviewed_at: None,
        }
    }

    #[test]
    fn upsert_fts_and_vector_search() {
        let store = IssueStore::open_in_memory("o/r").unwrap();
        store
            .upsert_issue(&sample(
                1,
                "Windows save crash access violation",
                "click save then access violation on Windows",
                "access violation",
            ))
            .unwrap();
        store
            .upsert_issue(&sample(
                2,
                "docs typo in readme",
                "fix spelling in documentation",
                "",
            ))
            .unwrap();
        store
            .upsert_issue(&sample(
                3,
                "crash when saving config",
                "save button causes access violation",
                "access violation",
            ))
            .unwrap();

        assert_eq!(store.count_issues().unwrap(), 3);

        let fts = store.fts_search("access violation save", 10).unwrap();
        assert!(
            fts.iter()
                .any(|c| c.issue_number == 1 || c.issue_number == 3),
            "fts should hit crash issues: {fts:?}"
        );

        let exact = store
            .exact_error_match(&["access violation".into()], 99, 10)
            .unwrap();
        assert!(exact.len() >= 2);

        let q = embed_local("Windows access violation when saving");
        let vec = store.vector_search(&q, 99, 5, 0.1).unwrap();
        assert!(
            vec.iter().any(|c| c.sources.iter().any(|s| s == "vector")),
            "vector path must contribute candidates: {vec:?}"
        );
        assert!(vec[0].score > 0.0);
    }

    #[test]
    fn content_hash_change_clears_stale_embedding_when_absent() {
        let store = IssueStore::open_in_memory("o/r").unwrap();
        let mut a = sample(
            1,
            "title A",
            "body with access violation",
            "access violation",
        );
        store.upsert_issue(&a).unwrap();
        assert!(store.get_issue(1).unwrap().unwrap().embedding.is_some());

        // 正文变更，但本次未带新 embedding（模拟 embed 失败 / 关闭）
        a.title = "title A changed".into();
        a.body_raw = "totally different docs-only body".into();
        a.body_clean = a.body_raw.clone();
        a.content_hash = content_hash(&a.title, &a.body_raw);
        a.embedding = None;
        a.embedding_model = None;
        a.embedding_version = None;
        a.embedding_content_hash = None;
        store.upsert_issue(&a).unwrap();

        let got = store.get_issue(1).unwrap().unwrap();
        assert!(
            got.embedding.is_none(),
            "stale embedding must be cleared when content_hash changes"
        );
        assert!(got.embedding_model.is_none());
        assert!(got.embedding_content_hash.is_none());

        // 同 content_hash 再次 upsert 无 embedding 时，若已清空则保持空；
        // 若重新写入 embedding 应保留
        let emb = embed_local("title A changed\ntotally different");
        a.embedding = Some(f32s_to_bytes(&emb));
        a.embedding_model = Some(EMBED_MODEL.into());
        a.embedding_version = Some(EMBED_VERSION.into());
        a.embedding_content_hash = Some(a.content_hash.clone());
        store.upsert_issue(&a).unwrap();
        assert!(store.get_issue(1).unwrap().unwrap().embedding.is_some());
    }

    /// 借鉴需求文档 6.3 / F-12：每次判定与动作都要可审计，
    /// 尤其是「被闸门拦下」的那些——否则长跑模式下没人知道跳过了多少。
    #[test]
    fn audit_counts_executed_and_gated_runs() {
        use crate::issue::action::PlannedActions;
        let store = IssueStore::open_in_memory("o/r").unwrap();

        let published = IssueReviewDecision {
            issue_number: 1,
            verdict: IssueVerdict::NotABug,
            confidence: 0.65,
            ..Default::default()
        };
        let plan_published = PlannedActions {
            post_or_update_comment: true,
            labels_to_add: vec![],
            close: false,
            close_reason: None,
            reasons_blocked: vec![],
            needs_human_notice: false,
            assign_to: None,
        };
        store
            .record_action_audit(&published, &plan_published, true, Some("c-1"))
            .unwrap();

        let gated = IssueReviewDecision {
            issue_number: 2,
            verdict: IssueVerdict::Unverified,
            confidence: 0.4,
            ..Default::default()
        };
        let plan_gated = PlannedActions {
            post_or_update_comment: false,
            labels_to_add: vec!["needs-triage".into()],
            close: false,
            close_reason: None,
            reasons_blocked: vec!["low_confidence:0.40<0.50".into()],
            needs_human_notice: false,
            assign_to: None,
        };
        store
            .record_action_audit(&gated, &plan_gated, false, None)
            .unwrap();

        let gated = store.gated_issues().unwrap();
        assert_eq!(
            gated.len(),
            1,
            "must be able to name which issue is waiting"
        );
        assert_eq!(gated[0].issue_number, 2);
        assert!(
            !gated[0].handed_off,
            "no owner configured -> nobody notified"
        );

        let stats = store.action_stats().unwrap();
        assert_eq!(stats.total, 2);
        assert_eq!(stats.commented, 1);
        assert_eq!(stats.gated_low_confidence, 1);
        assert_eq!(stats.closed, 0);
        assert!(
            stats
                .by_verdict
                .iter()
                .any(|(v, n)| v == "NOT_A_BUG" && *n == 1),
            "{:?}",
            stats.by_verdict
        );
    }
}

//! SQLite 事件队列：Webhook 投递 + Worker 消费。

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

const QUEUE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS webhook_deliveries (
    delivery_id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    action TEXT NOT NULL DEFAULT '',
    repo_id TEXT NOT NULL DEFAULT '',
    issue_number INTEGER,
    payload TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    received_at TEXT NOT NULL,
    processed_at TEXT,
    last_error TEXT
);
CREATE INDEX IF NOT EXISTS idx_webhook_status ON webhook_deliveries(status, received_at);
"#;

#[derive(Debug, Clone)]
pub struct WebhookDelivery {
    pub delivery_id: String,
    pub event_type: String,
    pub action: String,
    pub repo_id: String,
    pub issue_number: Option<u64>,
    pub payload: String,
    pub status: String,
    pub attempts: i64,
}

pub struct EventQueue {
    conn: Connection,
}

impl EventQueue {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn =
            Connection::open(path).with_context(|| format!("open queue {}", path.display()))?;
        conn.execute_batch(QUEUE_SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(QUEUE_SCHEMA)?;
        Ok(Self { conn })
    }

    /// 幂等入队：同一 delivery_id 只插一次，返回是否新建。
    pub fn enqueue(
        &self,
        delivery_id: &str,
        event_type: &str,
        action: &str,
        repo_id: &str,
        issue_number: Option<u64>,
        payload: &str,
    ) -> Result<bool> {
        let now = super::pipeline::iso_now();
        let changed = self.conn.execute(
            r#"INSERT OR IGNORE INTO webhook_deliveries
               (delivery_id, event_type, action, repo_id, issue_number, payload, status, attempts, received_at)
               VALUES (?1,?2,?3,?4,?5,?6,'pending',0,?7)"#,
            params![
                delivery_id,
                event_type,
                action,
                repo_id,
                issue_number.map(|n| n as i64),
                payload,
                now
            ],
        )?;
        Ok(changed > 0)
    }

    pub fn claim_next(&self) -> Result<Option<WebhookDelivery>> {
        let tx = self.conn.unchecked_transaction()?;
        let row: Option<WebhookDelivery> = tx
            .query_row(
                r#"SELECT delivery_id, event_type, action, repo_id, issue_number, payload, attempts
                   FROM webhook_deliveries
                   WHERE status IN ('pending','retry_pending')
                   ORDER BY received_at ASC LIMIT 1"#,
                [],
                |r| {
                    Ok(WebhookDelivery {
                        delivery_id: r.get(0)?,
                        event_type: r.get(1)?,
                        action: r.get(2)?,
                        repo_id: r.get(3)?,
                        issue_number: r.get::<_, Option<i64>>(4)?.map(|n| n as u64),
                        payload: r.get(5)?,
                        status: "processing".into(),
                        attempts: r.get::<_, i64>(6)? + 1,
                    })
                },
            )
            .optional()?;
        let Some(d) = row else {
            return Ok(None);
        };
        tx.execute(
            "UPDATE webhook_deliveries SET status='processing', attempts=attempts+1 WHERE delivery_id=?1",
            params![d.delivery_id],
        )?;
        tx.commit()?;
        Ok(Some(d))
    }

    pub fn mark_completed(&self, delivery_id: &str) -> Result<()> {
        let now = super::pipeline::iso_now();
        self.conn.execute(
            "UPDATE webhook_deliveries SET status='completed', processed_at=?1, last_error=NULL WHERE delivery_id=?2",
            params![now, delivery_id],
        )?;
        Ok(())
    }

    pub fn mark_failed(&self, delivery_id: &str, err: &str, retry: bool) -> Result<()> {
        let status = if retry {
            "retry_pending"
        } else {
            "dead_letter"
        };
        self.conn.execute(
            "UPDATE webhook_deliveries SET status=?1, last_error=?2 WHERE delivery_id=?3",
            params![status, err, delivery_id],
        )?;
        Ok(())
    }

    pub fn count_by_status(&self, status: &str) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM webhook_deliveries WHERE status=?1",
            params![status],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_claim_complete_idempotent() {
        let q = EventQueue::open_in_memory().unwrap();
        assert!(q
            .enqueue("d1", "issues", "opened", "o/r", Some(1), "{}")
            .unwrap());
        assert!(!q
            .enqueue("d1", "issues", "opened", "o/r", Some(1), "{}")
            .unwrap());
        let d = q.claim_next().unwrap().unwrap();
        assert_eq!(d.delivery_id, "d1");
        assert_eq!(d.issue_number, Some(1));
        q.mark_completed("d1").unwrap();
        assert!(q.claim_next().unwrap().is_none());
        assert_eq!(q.count_by_status("completed").unwrap(), 1);
    }
}

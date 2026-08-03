//! Issue 内容哈希：驱动增量重分析。

use sha2::{Digest, Sha256};

/// 标题+正文内容哈希。
pub fn content_hash(title: &str, body: &str) -> String {
    let mut h = Sha256::new();
    h.update(normalize_ws(title).as_bytes());
    h.update(b"\0");
    h.update(normalize_ws(body).as_bytes());
    hex::encode(h.finalize())
}

/// 有效评论集合哈希（id + updated_at + body）。
pub fn comments_hash(comments: &[(u64, &str, &str)]) -> String {
    let mut items: Vec<(u64, String, String)> = comments
        .iter()
        .map(|(id, updated, body)| (*id, updated.to_string(), normalize_ws(body)))
        .collect();
    items.sort_by_key(|(id, _, _)| *id);
    let mut h = Sha256::new();
    for (id, updated, body) in items {
        h.update(id.to_string().as_bytes());
        h.update(b"\0");
        h.update(updated.as_bytes());
        h.update(b"\0");
        h.update(body.as_bytes());
        h.update(b"\n");
    }
    hex::encode(h.finalize())
}

fn normalize_ws(s: &str) -> String {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_stable_and_sensitive() {
        let a = content_hash("Crash on save", "steps:\n1. open");
        let b = content_hash("Crash on save", "steps:\n1. open");
        let c = content_hash("Crash on save", "steps:\n1. close");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn comments_hash_order_independent() {
        let h1 = comments_hash(&[(2, "t2", "b2"), (1, "t1", "b1")]);
        let h2 = comments_hash(&[(1, "t1", "b1"), (2, "t2", "b2")]);
        assert_eq!(h1, h2);
        let h3 = comments_hash(&[(1, "t1", "b1-changed"), (2, "t2", "b2")]);
        assert_ne!(h1, h3);
    }
}

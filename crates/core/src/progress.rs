//! 审查实时进度：跨并行 Agent 共享的轻量进度沉淀，供 CLI 单行实时渲染。
//!
//! core 只记聚合计数 + 最近一次活动，不做任何 IO/渲染（渲染在 CLI 侧、且仅 TTY 才开）。
//! 线程安全（多个维度 Agent 并发更新）。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// 审查进度沉淀。用 `Arc<Progress>` 在编排与各 Agent 间共享。
#[derive(Default)]
pub struct Progress {
    tool_calls: AtomicUsize,
    last: Mutex<String>,
}

impl Progress {
    pub fn new() -> Self {
        Self::default()
    }

    /// 记录一次工具调用：累加计数，并把「维度: 工具 目标」设为最近活动。
    pub fn record_tool(&self, dim: &str, tool: &str, target: &str) {
        self.tool_calls.fetch_add(1, Ordering::Relaxed);
        let activity = if target.is_empty() {
            format!("{dim}: {tool}")
        } else {
            format!("{dim}: {tool} {target}")
        };
        if let Ok(mut g) = self.last.lock() {
            *g = activity;
        }
    }

    /// 当前快照：(工具调用总数, 最近活动文本)。
    pub fn snapshot(&self) -> (usize, String) {
        let n = self.tool_calls.load(Ordering::Relaxed);
        let last = self.last.lock().map(|g| g.clone()).unwrap_or_default();
        (n, last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_progress_starts_empty() {
        let p = Progress::default();
        assert_eq!(p.snapshot(), (0, String::new()));
    }

    #[test]
    fn records_count_and_latest_activity() {
        let p = Progress::new();
        assert_eq!(p.snapshot(), (0, String::new()));

        p.record_tool("logic", "read_file", "src/a.rs");
        p.record_tool("security", "code_search", "eval(");
        let (n, last) = p.snapshot();
        assert_eq!(n, 2);
        assert_eq!(last, "security: code_search eval(");

        // 空目标只显示 维度: 工具
        p.record_tool("intent", "task_done", "");
        let (n, last) = p.snapshot();
        assert_eq!(n, 3);
        assert_eq!(last, "intent: task_done");
    }
}

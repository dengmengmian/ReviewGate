//! Agent tool-use 循环（维度专家）。
//!
//! 本模块只放**类型与配置**；运行循环在 [`run`]，控制工具定义在 [`control_tools`]。
//! `report_finding` / `task_done` 是控制工具，由循环内部拦截处理（前者收集 Finding，后者终止）。

mod control_tools;
mod prompt;
mod run;

pub use prompt::{
    build_user_prompt, dimension_focus_block, intent_system_prompt, shared_system_prompt,
};
pub use run::{run_agent, run_agent_with_stats};

use crate::model::{ContentBlock, Dimension, Finding, Usage};
use std::collections::BTreeMap;

/// 同一工具以**相同参数**连续调用达到此次数即触发循环熔断，短路返回提示，
/// 不再真正执行——防止 Agent 在固定轮次内空转烧 token。
const LOOP_GUARD_LIMIT: usize = 3;

/// 单个维度 Agent 的配置。
pub struct AgentConfig {
    pub dimension: Dimension,
    pub system_prompt: String,
    pub max_rounds: usize,
    /// 是否打印每轮进度到 stderr。
    pub verbose: bool,
    /// 墙钟上限：每轮开始前检查，超时则**优雅收尾**（保留已上报的发现），不丢工作。
    pub timeout: Option<std::time::Duration>,
    /// 输入 token 预算（取 provider 的 `max_input_tokens`）。每轮发送前预检，
    /// 估算超预算则**确定性地**提前收尾并标记上下文溢出，避免撞 provider 的 400。None = 不检查。
    pub max_input_tokens: Option<usize>,
}

/// Agent 退出原因。区分**正常收尾**与**未审完**，供上层闸口"未审完不放行"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentExitReason {
    /// 模型主动 task_done 或自然结束。
    Completed,
    /// 走完 max_rounds（末轮已被强制收口，视为可接受的完成）。
    MaxRounds,
    /// 墙钟超时提前收尾。
    TimedOut,
    /// `complete()` 请求失败（含上下文超限的 4xx、网络错误）。
    RequestFailed,
    /// 发送前预检估算超输入预算，主动收尾（未撞 API）。
    ContextOverflow,
}

impl AgentExitReason {
    /// 是否算"未审完"——超时/请求失败/上下文溢出都意味着没审完整。
    /// `Completed`/`MaxRounds` 视为完成（末轮已强制收口，避免误报）。
    pub fn is_incomplete(self) -> bool {
        matches!(
            self,
            AgentExitReason::TimedOut
                | AgentExitReason::RequestFailed
                | AgentExitReason::ContextOverflow
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AgentExitReason::Completed => "completed",
            AgentExitReason::MaxRounds => "max_rounds",
            AgentExitReason::TimedOut => "timed_out",
            AgentExitReason::RequestFailed => "request_failed",
            AgentExitReason::ContextOverflow => "context_overflow",
        }
    }
}

/// 单个维度 Agent 的运行统计，用于定位慢在哪里。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentStats {
    pub llm_requests: usize,
    pub tool_calls: usize,
    pub findings_reported: usize,
    pub task_done_calls: usize,
    /// 被循环熔断短路的工具调用次数。
    pub loop_guarded: usize,
    pub tool_counts: BTreeMap<String, usize>,
    /// 累计 token 用量（含缓存命中）。
    pub usage: Usage,
}

impl AgentStats {
    fn record_tool(&mut self, name: &str) {
        self.tool_calls += 1;
        *self.tool_counts.entry(name.to_string()).or_default() += 1;
        if name == "report_finding" {
            self.findings_reported += 1;
        } else if name == "task_done" {
            self.task_done_calls += 1;
        }
    }

    pub fn tool_summary(&self) -> String {
        if self.tool_counts.is_empty() {
            return "无工具调用".into();
        }
        self.tool_counts
            .iter()
            .map(|(name, count)| format!("{name}={count}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Agent 运行结果 + 统计。
pub struct AgentRun {
    pub findings: Vec<Finding>,
    pub stats: AgentStats,
    /// 退出原因：用于在输出中提示该维度是否未审完。
    pub exit_reason: AgentExitReason,
}

impl AgentRun {
    /// 是否因超时提前收尾。
    pub fn timed_out(&self) -> bool {
        self.exit_reason == AgentExitReason::TimedOut
    }
    /// 是否未审完（超时/请求失败/上下文溢出）。
    pub fn incomplete(&self) -> bool {
        self.exit_reason.is_incomplete()
    }
}

impl AgentConfig {
    /// 用默认提示构造一个维度 Agent。
    pub fn for_dimension(dimension: Dimension) -> Self {
        Self {
            dimension,
            // 共享（维度无关）系统提示，配合缓存跨维度复用；维度聚焦点放进首条 user 消息。
            system_prompt: shared_system_prompt(),
            max_rounds: 12,
            verbose: false,
            timeout: None,
            max_input_tokens: None,
        }
    }
}

/// 取响应里的纯文本（便于调试）。
pub fn assistant_text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

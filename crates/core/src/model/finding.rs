//! 审查结果模型：维度、严重度、Finding。

use serde::{Deserialize, Serialize};

/// 审查维度。每个维度由一个独立的专家 Agent 负责。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dimension {
    /// 安全：注入、越权、密钥泄露、不安全反序列化等。
    Security,
    /// 性能：N+1、无谓拷贝、复杂度、热路径分配等。
    Perf,
    /// 逻辑正确性：边界条件、空值、并发、错误处理等。
    Logic,
    /// 规范：命名、风格、分语言 checklist。
    Style,
    /// AI 代码专项：幻觉 API、看似合理实则错误、假设漂移、复制未适配等。
    AiSmell,
    /// 业务/项目规则：领域语义、权限边界、状态机、金额/订单/库存等。
    /// 不在 [`Dimension::ALL`] 默认集内——仅在配置了业务规则或显式指定时启用。
    Business,
    /// 意图/技术评审：实现是否符合传入的意图/需求/验收标准（完整性、与意图不符、破坏既有行为、方案风险）。
    /// 不在 [`Dimension::ALL`] 内——仅在提供了 `--intent`（意图/参考文档）时由独立的整体性 Agent 运行。
    Intent,
}

impl Dimension {
    /// 全部维度，顺序即并行编排顺序。
    pub const ALL: [Dimension; 5] = [
        Dimension::Security,
        Dimension::Perf,
        Dimension::Logic,
        Dimension::Style,
        Dimension::AiSmell,
    ];

    /// 稳定的字符串标识（用于配置 key、日志、JSON）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Dimension::Security => "security",
            Dimension::Perf => "perf",
            Dimension::Logic => "logic",
            Dimension::Style => "style",
            Dimension::AiSmell => "ai_smell",
            Dimension::Business => "business",
            Dimension::Intent => "intent",
        }
    }
}

impl std::fmt::Display for Dimension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 严重度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Low,
    Med,
    High,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Low => "low",
            Severity::Med => "med",
            Severity::High => "high",
        }
    }
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 可达性：问题在当前控制流下是否真的会被执行到。
///
/// 由证伪 Judge 评估。区分「真问题且现在就能触发」与「真问题但当前路径打不到」
/// （潜伏雷）——后者作为提示有价值，但不应阻断合并。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reachability {
    /// 当前代码路径可触发。
    Reachable,
    /// 代码本身成立，但调用方路由/上游 guard 使该分支或语句当前不可达（潜伏雷）。
    Latent,
    /// 未评估或无法判定——按可达对待（保守不降级）。
    #[default]
    Unknown,
}

impl Reachability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Reachability::Reachable => "reachable",
            Reachability::Latent => "latent",
            Reachability::Unknown => "unknown",
        }
    }
}

/// 一条审查发现。
///
/// 注意：`start_line` / `end_line` 优先取自 LLM 直接报出的标注行号，引擎再用
/// `existing_code` 锚点校验/兜底（见 `relocate`）。校验失败、或 LLM 未给行号时两者为 0。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// 所属维度。
    pub dimension: Dimension,
    /// 置信度 0.0–1.0，由证伪 Judge（M2.3）给出。初始可为 Agent 自评。
    pub confidence: f32,
    /// 严重度。
    pub severity: Severity,
    /// 文件路径（相对仓库根）。
    pub path: String,
    /// 起始行（1-based）；0 表示行号重定位失败。
    pub start_line: u32,
    /// 结束行（1-based，含）；0 表示行号重定位失败。
    pub end_line: u32,
    /// 问题描述。
    pub message: String,
    /// LLM 提供的定位锚点：改动文件中真实存在的一段代码。
    pub existing_code: String,
    /// 支撑证据（代码片段/搜索结果），用于反幻觉与透明展示。
    #[serde(default)]
    pub evidence: String,
    /// 建议修复说明（自由文字，可选）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<String>,
    /// 建议的**替换代码**（修复后应有的代码）。与 `existing_code` 配对可渲染
    /// red→green diff、并支持一键应用。没有则为空串。
    #[serde(default)]
    pub suggestion_code: String,
    /// 可达性：由 Judge 评估。`Latent`（潜伏雷）在闸口处永不阻断、最多 Warn。
    /// 缺省 `Unknown`（按可达对待，不降级）。
    #[serde(default)]
    pub reachability: Reachability,
    /// 是否被闸口过滤（低于 warn 阈值）。被过滤项仍保留以便用户展开查看。
    #[serde(default)]
    pub filtered: bool,
    /// 独立标记同一处问题的**不同维度**数（去重时填）。≥2 表示多维度交叉印证，
    /// 是较强的「真问题」信号；用于证伪后的置信度加分。1/0 视为单维度。
    #[serde(default)]
    pub agreed_dimensions: u8,
}

impl Finding {
    /// 行号重定位是否成功。
    pub fn located(&self) -> bool {
        self.start_line > 0
    }
}

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
    /// **不在 [`Dimension::ALL`] 默认集内**——质量闸口默认不管纯风格（噪声，交给 linter）；
    /// 用 `--dimensions style`（或 `...,style`）显式开启。
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

/// 意图评审里一条发现相对某条验收标准的判定（需求锚定，不依赖 diff 行号）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentStatus {
    /// 该验收项已满足（信息项，进验收清单、不计入闸口）。
    Met,
    /// 缺失：需求点/分支/调用方适配未实现。
    Missing,
    /// 与意图不符：实现做错或误解需求。
    Deviation,
    /// 破坏既有行为/契约（意图未要求）。
    Breaking,
    /// 方案风险或更优解（建议级）。
    Suggestion,
    /// 未核对：该验收标准没有被评审给出明确判定（结构化兜底填充，保证清单覆盖每条标准）。
    Unknown,
}

impl IntentStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            IntentStatus::Met => "met",
            IntentStatus::Missing => "missing",
            IntentStatus::Deviation => "deviation",
            IntentStatus::Breaking => "breaking",
            IntentStatus::Suggestion => "suggestion",
            IntentStatus::Unknown => "unknown",
        }
    }
}

impl Dimension {
    /// 默认自动运行的缺陷维度集（`--dimensions all` 即此集），顺序即并行编排顺序。
    /// **Style 不在默认集**：作为质量闸口，纯风格/格式问题属噪声（评测证实 style 命中真缺陷≈0
    /// 却拉低精度），交给 linter/formatter；需要时用 `--dimensions style` 显式开启。
    /// 这与 Business/Intent 同为 opt-in 维度。
    pub const ALL: [Dimension; 4] = [
        Dimension::Security,
        Dimension::Perf,
        Dimension::Logic,
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
    /// 意图评审专用：该发现映射到的验收标准/意图点（需求锚定）。其它维度为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criterion: Option<String>,
    /// 意图评审专用：相对验收标准的判定（met/missing/deviation/breaking/suggestion）。其它维度为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_status: Option<IntentStatus>,
}

impl Finding {
    /// 行号重定位是否成功。
    pub fn located(&self) -> bool {
        self.start_line > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_all_and_strings() {
        assert_eq!(Dimension::ALL.len(), 4);
        assert!(Dimension::ALL.contains(&Dimension::Security));
        // Style/Business/Intent 都是 opt-in，不在默认集。
        assert!(!Dimension::ALL.contains(&Dimension::Style));
        assert!(!Dimension::ALL.contains(&Dimension::Business));
        assert!(!Dimension::ALL.contains(&Dimension::Intent));
        assert_eq!(Dimension::AiSmell.as_str(), "ai_smell");
        assert_eq!(format!("{}", Dimension::Logic), "logic");
    }

    #[test]
    fn severity_order_and_strings() {
        assert!(Severity::Low < Severity::Med);
        assert!(Severity::Med < Severity::High);
        assert_eq!(Severity::High.as_str(), "high");
        assert_eq!(format!("{}", Severity::Low), "low");
    }

    #[test]
    fn reachability_defaults_to_unknown() {
        let r: Reachability = Default::default();
        assert_eq!(r, Reachability::Unknown);
        assert_eq!(Reachability::Reachable.as_str(), "reachable");
        assert_eq!(Reachability::Latent.as_str(), "latent");
    }

    #[test]
    fn intent_status_strings() {
        assert_eq!(IntentStatus::Met.as_str(), "met");
        assert_eq!(IntentStatus::Deviation.as_str(), "deviation");
        assert_eq!(IntentStatus::Unknown.as_str(), "unknown");
    }

    #[test]
    fn finding_located_only_when_start_positive() {
        let mut f = Finding {
            dimension: Dimension::Security,
            confidence: 0.8,
            severity: Severity::High,
            path: "a.rs".into(),
            start_line: 0,
            end_line: 0,
            message: "m".into(),
            existing_code: "x".into(),
            evidence: String::new(),
            suggestion: None,
            suggestion_code: String::new(),
            reachability: Reachability::Unknown,
            filtered: false,
            agreed_dimensions: 1,
            criterion: None,
            intent_status: None,
        };
        assert!(!f.located());
        f.start_line = 5;
        assert!(f.located());
    }

    #[test]
    fn finding_with_suggestion_serializes_it() {
        let f = Finding {
            dimension: Dimension::Security,
            confidence: 0.8,
            severity: Severity::High,
            path: "a.rs".into(),
            start_line: 1,
            end_line: 1,
            message: "m".into(),
            existing_code: "x".into(),
            evidence: String::new(),
            suggestion: Some("fix it".into()),
            suggestion_code: String::new(),
            reachability: Reachability::Unknown,
            filtered: false,
            agreed_dimensions: 1,
            criterion: None,
            intent_status: None,
        };
        let json = serde_json::to_string(&f).unwrap();
        assert!(json.contains("\"suggestion\":\"fix it\""));
    }
}

//! Issue Review 领域模型。

use serde::{Deserialize, Serialize};

/// 机器人评论幂等标记。
pub const BOT_COMMENT_MARKER: &str = "<!-- reviewgate:issue-review:v1 -->";

/// Issue 类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IssueType {
    Bug,
    FeatureRequest,
    Question,
    Documentation,
    Configuration,
    Support,
    Security,
    Performance,
    Compatibility,
    Spam,
    Advertisement,
    Abuse,
    #[default]
    Unknown,
}

impl IssueType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bug => "bug",
            Self::FeatureRequest => "feature_request",
            Self::Question => "question",
            Self::Documentation => "documentation",
            Self::Configuration => "configuration",
            Self::Support => "support",
            Self::Security => "security",
            Self::Performance => "performance",
            Self::Compatibility => "compatibility",
            Self::Spam => "spam",
            Self::Advertisement => "advertisement",
            Self::Abuse => "abuse",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "bug" => Self::Bug,
            "feature_request" | "feature" => Self::FeatureRequest,
            "question" => Self::Question,
            "documentation" | "docs" => Self::Documentation,
            "configuration" | "config" => Self::Configuration,
            "support" => Self::Support,
            "security" => Self::Security,
            "performance" | "perf" => Self::Performance,
            "compatibility" => Self::Compatibility,
            "spam" => Self::Spam,
            "advertisement" | "ad" => Self::Advertisement,
            "abuse" => Self::Abuse,
            _ => Self::Unknown,
        }
    }
}

/// 重复判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DuplicateStatus {
    ExactDuplicate,
    ProbableDuplicate,
    SameSymptomDifferentRootCause,
    Regression,
    Related,
    #[default]
    NotDuplicate,
}

impl DuplicateStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExactDuplicate => "exact_duplicate",
            Self::ProbableDuplicate => "probable_duplicate",
            Self::SameSymptomDifferentRootCause => "same_symptom_different_root_cause",
            Self::Regression => "regression",
            Self::Related => "related",
            Self::NotDuplicate => "not_duplicate",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "exact_duplicate" => Self::ExactDuplicate,
            "probable_duplicate" => Self::ProbableDuplicate,
            "same_symptom_different_root_cause" => Self::SameSymptomDifferentRootCause,
            "regression" => Self::Regression,
            "related" => Self::Related,
            _ => Self::NotDuplicate,
        }
    }
}

/// 技术/综合结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IssueVerdict {
    ConfirmedBug,
    LikelyBug,
    #[default]
    Unverified,
    NotABug,
    Duplicate,
    Regression,
    AlreadyFixed,
    Spam,
    Advertisement,
    NeedsInfo,
}

impl IssueVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ConfirmedBug => "CONFIRMED_BUG",
            Self::LikelyBug => "LIKELY_BUG",
            Self::Unverified => "UNVERIFIED",
            Self::NotABug => "NOT_A_BUG",
            Self::Duplicate => "DUPLICATE",
            Self::Regression => "REGRESSION",
            Self::AlreadyFixed => "ALREADY_FIXED",
            Self::Spam => "SPAM",
            Self::Advertisement => "ADVERTISEMENT",
            Self::NeedsInfo => "NEEDS_INFO",
        }
    }
}

/// 平台原始 Issue（与 GitHub JSON 字段对齐的简化结构）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawIssue {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    pub state: String,
    #[serde(default)]
    pub labels: Vec<RawLabel>,
    #[serde(default)]
    pub user: Option<RawUser>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub closed_at: Option<String>,
    /// 若为 PR 则有此字段；Issue 同步时排除。
    #[serde(default)]
    pub pull_request: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawLabel {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawUser {
    pub login: String,
    #[serde(default, rename = "type")]
    pub user_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawComment {
    pub id: u64,
    #[serde(default)]
    pub body: String,
    pub updated_at: String,
    #[serde(default)]
    pub user: Option<RawUser>,
}

/// 标准化后的 Issue 内容。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NormalizedIssue {
    pub title: String,
    pub body_clean: String,
    pub symptom: String,
    pub expected_behavior: String,
    pub actual_behavior: String,
    pub reproduction_steps: Vec<String>,
    pub environment: serde_json::Value,
    pub error_signatures: Vec<String>,
    pub stack_symbols: Vec<String>,
    /// 用于 embedding 的压缩文本。
    pub embed_text: String,
}

/// 入库的 Issue 记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredIssue {
    pub repo_id: String,
    pub issue_number: u64,
    pub title: String,
    pub body_raw: String,
    pub body_clean: String,
    pub state: String,
    pub labels_json: String,
    pub author: String,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub error_signature: String,
    pub stack_symbols_json: String,
    pub source_updated_at: String,
    pub content_hash: String,
    pub comments_hash: String,
    pub embedding: Option<Vec<u8>>,
    pub embedding_model: Option<String>,
    pub embedding_version: Option<String>,
    pub embedding_content_hash: Option<String>,
    pub last_synced_at: String,
    pub last_reviewed_at: Option<String>,
}

/// 查重候选。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateCandidate {
    pub issue_number: u64,
    pub title: String,
    pub score: f32,
    pub sources: Vec<String>,
    pub error_signature: String,
}

/// 一次 triage 决策（可审计、可发布）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IssueReviewDecision {
    pub issue_number: u64,
    pub primary_type: IssueType,
    pub type_confidence: f32,
    pub type_reasons: Vec<String>,
    pub completeness_score: f32,
    pub missing_fields: Vec<String>,
    pub can_verify: bool,
    pub spam_score: f32,
    pub advertisement_score: f32,
    pub abuse_score: f32,
    pub prompt_injection_score: f32,
    pub duplicate_status: DuplicateStatus,
    pub duplicate_confidence: f32,
    pub duplicate_of: Option<u64>,
    pub duplicate_candidates: Vec<DuplicateCandidate>,
    pub duplicate_evidence: Vec<String>,
    pub verdict: IssueVerdict,
    pub confidence: f32,
    pub reasons: Vec<String>,
    pub suggested_labels: Vec<String>,
    pub suggested_comment: String,
    pub close_recommended: bool,
    pub auto_action_allowed: bool,
    pub needs_human_review: bool,
    pub vector_used: bool,
    pub vector_degraded: bool,
    pub analyzer_version: String,
    /// Phase 2 技术验证结论（若跳过则为 Unverified + skipped_reason 在 reasons 中）。
    #[serde(default)]
    pub technical_verdict: IssueVerdict,
    #[serde(default)]
    pub technical_confidence: f32,
    #[serde(default)]
    pub technical_evidence: Vec<String>,
    #[serde(default)]
    pub code_paths: Vec<String>,
    /// 真实仓库检索命中（路径:行 + 摘要），供用户向评论展示。
    #[serde(default)]
    pub code_hits: Vec<CodeEvidence>,
    #[serde(default)]
    pub related_commits: Vec<String>,
    #[serde(default)]
    pub fix_prs: Vec<String>,
    #[serde(default)]
    pub verification_ran: bool,
    /// Issue 标题（评论里复述，方便对照）。
    #[serde(default)]
    pub issue_title: String,
    /// 标准化后的现象摘要。
    #[serde(default)]
    pub symptom_summary: String,
    /// 疑似错投的目标仓库（`owner/repo`）。正交于类型与裁决：
    /// 一个 bug 报到了错仓库，它仍然是 bug。空 = 未检出。
    #[serde(default)]
    pub misrouted_repos: Vec<String>,
    /// 错投判断的置信度；低于动作闸门时只打标签、不在评论里引导。
    #[serde(default)]
    pub misrouted_confidence: f32,
}

/// 面向用户展示的代码证据。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodeEvidence {
    pub path: String,
    pub line: u32,
    pub snippet: String,
}

impl IssueReviewDecision {
    pub fn analyzer_version() -> &'static str {
        "issue-review-v2"
    }
}

/// 评论发布结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishResult {
    pub issue_number: u64,
    pub comment_id: String,
    pub created: bool,
    pub updated: bool,
}

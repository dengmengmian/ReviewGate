//! Issue Review / Issue Triage：同步、查重、分类、技术验证、Webhook 与多平台发布。

pub mod action;
pub mod classify;
pub mod comment;
pub mod completeness;
pub mod duplicate;
pub mod embedding;
pub mod explain;
pub mod facts;
pub mod hash;
pub mod judge;
pub mod mentions;
pub mod misrouted;
pub mod model;
pub mod normalize;
pub mod pipeline;
pub mod platform;
pub mod queue;
pub mod safety;
pub mod serve;
pub mod store;
pub mod verify;
pub mod webhook;

pub use action::{plan_actions, ActionPolicy, PlannedActions, SAFE_LABELS};
pub use comment::{is_bot_comment, render_comment, render_comment_with_mentions};
pub use embedding::{Embedder, FailingEmbedder, LocalEmbedder};
pub use explain::{
    deterministic_narrative, generate_user_comment, generate_user_comment_sync, reply_shape,
    ReplyShape,
};
pub use facts::{
    build_fact_pack, build_fix_directions, render_fact_comment, should_emit_fix_directions,
    FactPack, FIX_DIRECTION_MIN_CONF,
};
pub use hash::{comments_hash, content_hash};
pub use mentions::{format_mention_line, resolve_mentions, MentionConfig};
pub use model::*;
pub use normalize::normalize_issue;
pub use pipeline::{
    finalize_comment, format_review_text, format_unix_secs_rfc3339, ingest_raw, iso_now,
    publish_decision, review_issue, review_issue_with_llm, sync_from_platform, triage_stored,
    IssueReviewConfig, ReviewOutput,
};
pub use platform::{
    build_platform, map_v5_issue, AtomGitIssuePlatform, FixturePlatform, GitHubIssuePlatform,
    GitLabIssuePlatform, GiteeIssuePlatform, GiteeStyleIssuePlatform, HttpDoer, IssueForge,
    IssuePlatform, ReqwestDoer,
};
pub use queue::{EventQueue, WebhookDelivery};
pub use serve::{drain_queue_once, run_webhook_server, ServeConfig};
pub use store::IssueStore;
pub use verify::{
    build_plan, deepen_level1, resolve_repo_root, should_deep_dig, should_verify, verify_level0,
    DeepDigBlock, TechnicalVerification,
};
pub use webhook::{
    parse_github_event, parse_gitlab_event, verify_github_signature, verify_gitlab_token,
    ParsedWebhook,
};

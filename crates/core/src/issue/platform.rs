//! 多平台 Issue 适配：GitHub / GitLab / Gitee / AtomGit（可注入 HTTP）。

use super::comment::is_bot_comment;
use super::model::{RawComment, RawIssue, RawLabel, RawUser, BOT_COMMENT_MARKER};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

/// 支持的 Issue 平台。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueForge {
    GitHub,
    GitLab,
    Gitee,
    AtomGit,
}

impl IssueForge {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "github" | "gh" => Some(Self::GitHub),
            "gitlab" | "gl" => Some(Self::GitLab),
            "gitee" => Some(Self::Gitee),
            "atomgit" | "atom" => Some(Self::AtomGit),
            _ => None,
        }
    }

    pub fn default_api_base(self) -> &'static str {
        match self {
            Self::GitHub => "https://api.github.com",
            Self::GitLab => "https://gitlab.com/api/v4",
            Self::Gitee => "https://gitee.com/api/v5",
            // https://docs.atomgit.com/docs/apis/ → /api/v5
            Self::AtomGit => "https://api.atomgit.com/api/v5",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::GitLab => "gitlab",
            Self::Gitee => "gitee",
            Self::AtomGit => "atomgit",
        }
    }
}

/// 窄接口：同步、发布、标签、关闭。
#[async_trait]
pub trait IssuePlatform: Send + Sync {
    async fn get_issue(&self, number: u64) -> Result<RawIssue>;
    async fn list_issues_page(
        &self,
        page: u32,
        per_page: u32,
        since: Option<&str>,
    ) -> Result<Vec<RawIssue>>;
    async fn list_comments(&self, number: u64) -> Result<Vec<RawComment>>;
    async fn create_comment(&self, number: u64, body: &str) -> Result<String>;
    async fn update_comment(&self, comment_id: &str, body: &str) -> Result<()>;
    async fn find_bot_comment(&self, number: u64) -> Result<Option<String>>;
    async fn add_labels(&self, number: u64, labels: &[String]) -> Result<()>;
    async fn close_issue(&self, number: u64, reason: &str) -> Result<()>;
    /// 指派处理人。默认未实现——没在真机验证过的写操作宁可报错，也不能假成功。
    async fn assign(&self, number: u64, login: &str) -> Result<()> {
        let _ = (number, login);
        bail!("assign is not implemented for this forge")
    }
}

/// 最小 HTTP 抽象。
#[async_trait]
pub trait HttpDoer: Send + Sync {
    async fn request_json(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<(u16, Value)>;
}

pub struct ReqwestDoer {
    client: reqwest::Client,
}

impl ReqwestDoer {
    pub fn new() -> Result<Self> {
        Ok(Self {
            client: crate::llm::http::shared_http_client()?,
        })
    }
}

#[async_trait]
impl HttpDoer for ReqwestDoer {
    async fn request_json(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<(u16, Value)> {
        let mut req = match method {
            "GET" => self.client.get(url),
            "POST" => self.client.post(url),
            "PATCH" => self.client.patch(url),
            "PUT" => self.client.put(url),
            other => bail!("unsupported method {other}"),
        };
        for (k, v) in headers {
            req = req.header(*k, v);
        }
        req = req.header("User-Agent", "ReviewGate-IssueReview");
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.context("http send")?;
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        let val = if text.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&text).unwrap_or(Value::String(text))
        };
        Ok((status, val))
    }
}

// ─── GitHub ─────────────────────────────────────────────────────────────────

pub struct GitHubIssuePlatform<H: HttpDoer> {
    pub api_base: String,
    pub repo: String,
    pub token: String,
    http: H,
}

impl<H: HttpDoer> GitHubIssuePlatform<H> {
    pub fn new(
        api_base: impl Into<String>,
        repo: impl Into<String>,
        token: impl Into<String>,
        http: H,
    ) -> Self {
        Self {
            api_base: api_base.into().trim_end_matches('/').to_string(),
            repo: repo.into(),
            token: token.into(),
            http,
        }
    }

    fn headers(&self) -> Vec<(&'static str, String)> {
        vec![
            ("Authorization", format!("Bearer {}", self.token)),
            ("Accept", "application/vnd.github+json".into()),
        ]
    }
}

#[async_trait]
impl<H: HttpDoer> IssuePlatform for GitHubIssuePlatform<H> {
    async fn get_issue(&self, number: u64) -> Result<RawIssue> {
        let url = format!("{}/repos/{}/issues/{number}", self.api_base, self.repo);
        let (status, val) = self
            .http
            .request_json("GET", &url, &self.headers(), None)
            .await?;
        if !(200..300).contains(&status) {
            bail!("get_issue {number} status={status} body={val}");
        }
        Ok(serde_json::from_value(val).context("parse issue")?)
    }

    async fn list_issues_page(
        &self,
        page: u32,
        per_page: u32,
        since: Option<&str>,
    ) -> Result<Vec<RawIssue>> {
        let mut url = format!(
            "{}/repos/{}/issues?state=all&filter=all&per_page={per_page}&page={page}&direction=asc&sort=updated",
            self.api_base, self.repo
        );
        if let Some(s) = since {
            url.push_str(&format!("&since={s}"));
        }
        let (status, val) = self
            .http
            .request_json("GET", &url, &self.headers(), None)
            .await?;
        if !(200..300).contains(&status) {
            bail!("list_issues status={status} body={val}");
        }
        let items: Vec<RawIssue> = serde_json::from_value(val).context("parse issues list")?;
        Ok(items
            .into_iter()
            .filter(|i| i.pull_request.is_none())
            .collect())
    }

    async fn list_comments(&self, number: u64) -> Result<Vec<RawComment>> {
        let url = format!(
            "{}/repos/{}/issues/{number}/comments?per_page=100",
            self.api_base, self.repo
        );
        let (status, val) = self
            .http
            .request_json("GET", &url, &self.headers(), None)
            .await?;
        if !(200..300).contains(&status) {
            bail!("list_comments status={status}");
        }
        Ok(serde_json::from_value(val).context("parse comments")?)
    }

    async fn create_comment(&self, number: u64, body: &str) -> Result<String> {
        let url = format!(
            "{}/repos/{}/issues/{number}/comments",
            self.api_base, self.repo
        );
        let (status, val) = self
            .http
            .request_json(
                "POST",
                &url,
                &self.headers(),
                Some(serde_json::json!({ "body": body })),
            )
            .await?;
        if !(200..300).contains(&status) {
            bail!("create_comment status={status} body={val}");
        }
        Ok(val
            .get("id")
            .and_then(|v| v.as_u64())
            .context("comment id missing")?
            .to_string())
    }

    async fn update_comment(&self, comment_id: &str, body: &str) -> Result<()> {
        let url = format!(
            "{}/repos/{}/issues/comments/{comment_id}",
            self.api_base, self.repo
        );
        let (status, val) = self
            .http
            .request_json(
                "PATCH",
                &url,
                &self.headers(),
                Some(serde_json::json!({ "body": body })),
            )
            .await?;
        if !(200..300).contains(&status) {
            bail!("update_comment status={status} body={val}");
        }
        Ok(())
    }

    async fn find_bot_comment(&self, number: u64) -> Result<Option<String>> {
        for c in self.list_comments(number).await? {
            if is_bot_comment(&c.body) || c.body.contains(BOT_COMMENT_MARKER) {
                return Ok(Some(c.id.to_string()));
            }
        }
        Ok(None)
    }

    async fn assign(&self, number: u64, login: &str) -> Result<()> {
        let url = format!(
            "{}/repos/{}/issues/{number}/assignees",
            self.api_base, self.repo
        );
        let (status, val) = self
            .http
            .request_json(
                "POST",
                &url,
                &self.headers(),
                Some(serde_json::json!({ "assignees": [login] })),
            )
            .await?;
        if !(200..300).contains(&status) {
            bail!("github assign status={status} body={val}");
        }
        Ok(())
    }

    async fn add_labels(&self, number: u64, labels: &[String]) -> Result<()> {
        if labels.is_empty() {
            return Ok(());
        }
        let url = format!(
            "{}/repos/{}/issues/{number}/labels",
            self.api_base, self.repo
        );
        let (status, val) = self
            .http
            .request_json(
                "POST",
                &url,
                &self.headers(),
                Some(serde_json::json!({ "labels": labels })),
            )
            .await?;
        if !(200..300).contains(&status) {
            bail!("add_labels status={status} body={val}");
        }
        Ok(())
    }

    async fn close_issue(&self, number: u64, _reason: &str) -> Result<()> {
        let url = format!("{}/repos/{}/issues/{number}", self.api_base, self.repo);
        let (status, val) = self
            .http
            .request_json(
                "PATCH",
                &url,
                &self.headers(),
                Some(serde_json::json!({ "state": "closed" })),
            )
            .await?;
        if !(200..300).contains(&status) {
            bail!("close_issue status={status} body={val}");
        }
        Ok(())
    }
}

// ─── GitLab ─────────────────────────────────────────────────────────────────

pub struct GitLabIssuePlatform<H: HttpDoer> {
    pub api_base: String,
    /// project id 或 URL 编码 path
    pub project: String,
    pub token: String,
    http: H,
}

impl<H: HttpDoer> GitLabIssuePlatform<H> {
    pub fn new(
        api_base: impl Into<String>,
        project: impl Into<String>,
        token: impl Into<String>,
        http: H,
    ) -> Self {
        let project = project.into();
        // path with slash → encode
        let project = if project.contains('/') && !project.chars().all(|c| c.is_ascii_digit()) {
            project.replace('/', "%2F")
        } else {
            project
        };
        Self {
            api_base: api_base.into().trim_end_matches('/').to_string(),
            project,
            token: token.into(),
            http,
        }
    }

    fn headers(&self) -> Vec<(&'static str, String)> {
        vec![("PRIVATE-TOKEN", self.token.clone())]
    }

    fn map_issue(v: &Value) -> Option<RawIssue> {
        let number = v.get("iid").and_then(|x| x.as_u64())?;
        let title = v
            .get("title")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let body = v
            .get("description")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        let state = v
            .get("state")
            .and_then(|x| x.as_str())
            .unwrap_or("opened")
            .to_string();
        let labels = v
            .get("labels")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| {
                        l.as_str().map(|s| RawLabel {
                            name: s.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let user = v
            .pointer("/author/username")
            .and_then(|x| x.as_str())
            .map(|login| RawUser {
                login: login.to_string(),
                user_type: Some("User".into()),
            });
        Some(RawIssue {
            number,
            title,
            body,
            state,
            labels,
            user,
            created_at: v
                .get("created_at")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            updated_at: v
                .get("updated_at")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            closed_at: v
                .get("closed_at")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
            pull_request: None,
        })
    }
}

#[async_trait]
impl<H: HttpDoer> IssuePlatform for GitLabIssuePlatform<H> {
    async fn get_issue(&self, number: u64) -> Result<RawIssue> {
        let url = format!(
            "{}/projects/{}/issues/{number}",
            self.api_base, self.project
        );
        let (status, val) = self
            .http
            .request_json("GET", &url, &self.headers(), None)
            .await?;
        if !(200..300).contains(&status) {
            bail!("gitlab get_issue status={status}");
        }
        Self::map_issue(&val).context("map gitlab issue")
    }

    async fn list_issues_page(
        &self,
        page: u32,
        per_page: u32,
        since: Option<&str>,
    ) -> Result<Vec<RawIssue>> {
        let mut url = format!(
            "{}/projects/{}/issues?state=all&per_page={per_page}&page={page}&order_by=updated_at&sort=asc",
            self.api_base, self.project
        );
        if let Some(s) = since {
            url.push_str(&format!("&updated_after={s}"));
        }
        let (status, val) = self
            .http
            .request_json("GET", &url, &self.headers(), None)
            .await?;
        if !(200..300).contains(&status) {
            bail!("gitlab list status={status}");
        }
        let arr = val.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().filter_map(Self::map_issue).collect())
    }

    async fn list_comments(&self, number: u64) -> Result<Vec<RawComment>> {
        let url = format!(
            "{}/projects/{}/issues/{number}/notes?per_page=100",
            self.api_base, self.project
        );
        let (status, val) = self
            .http
            .request_json("GET", &url, &self.headers(), None)
            .await?;
        if !(200..300).contains(&status) {
            bail!("gitlab notes status={status}");
        }
        let mut out = Vec::new();
        if let Some(arr) = val.as_array() {
            for n in arr {
                let id = n.get("id").and_then(|x| x.as_u64()).unwrap_or(0);
                let body = n
                    .get("body")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let updated = n
                    .get("updated_at")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let user = n
                    .pointer("/author/username")
                    .and_then(|x| x.as_str())
                    .map(|login| RawUser {
                        login: login.to_string(),
                        user_type: Some("User".into()),
                    });
                out.push(RawComment {
                    id,
                    body,
                    updated_at: updated,
                    user,
                });
            }
        }
        Ok(out)
    }

    async fn create_comment(&self, number: u64, body: &str) -> Result<String> {
        let url = format!(
            "{}/projects/{}/issues/{number}/notes",
            self.api_base, self.project
        );
        let (status, val) = self
            .http
            .request_json(
                "POST",
                &url,
                &self.headers(),
                Some(serde_json::json!({ "body": body })),
            )
            .await?;
        if !(200..300).contains(&status) {
            bail!("gitlab create note status={status}");
        }
        Ok(val
            .get("id")
            .and_then(|x| x.as_u64())
            .unwrap_or(0)
            .to_string())
    }

    async fn update_comment(&self, comment_id: &str, body: &str) -> Result<()> {
        // GitLab: PUT /projects/:id/issues/:issue_iid/notes/:note_id — need issue iid; store as note id only
        // Use issues notes endpoint via search is hard; use generic notes API
        let url = format!(
            "{}/projects/{}/notes/{comment_id}",
            self.api_base, self.project
        );
        let (status, val) = self
            .http
            .request_json(
                "PUT",
                &url,
                &self.headers(),
                Some(serde_json::json!({ "body": body })),
            )
            .await?;
        // fallback: some instances require issue-scoped path — try issue notes if failed
        if !(200..300).contains(&status) {
            bail!("gitlab update note status={status} body={val}");
        }
        Ok(())
    }

    async fn find_bot_comment(&self, number: u64) -> Result<Option<String>> {
        for c in self.list_comments(number).await? {
            if is_bot_comment(&c.body) {
                return Ok(Some(c.id.to_string()));
            }
        }
        Ok(None)
    }

    /// GitLab 的 assignee 只认数值 user id，得先按 username 查一次。
    async fn assign(&self, number: u64, login: &str) -> Result<()> {
        let lookup = format!("{}/users?username={login}", self.api_base);
        let (status, val) = self
            .http
            .request_json("GET", &lookup, &self.headers(), None)
            .await?;
        if !(200..300).contains(&status) {
            bail!("gitlab user lookup status={status} body={val}");
        }
        let uid = val
            .as_array()
            .and_then(|a| a.first())
            .and_then(|u| u.get("id"))
            .and_then(|i| i.as_i64())
            .ok_or_else(|| anyhow::anyhow!("gitlab user `{login}` not found"))?;
        let url = format!(
            "{}/projects/{}/issues/{number}",
            self.api_base, self.project
        );
        let (status, val) = self
            .http
            .request_json(
                "PUT",
                &url,
                &self.headers(),
                Some(serde_json::json!({ "assignee_ids": [uid] })),
            )
            .await?;
        if !(200..300).contains(&status) {
            bail!("gitlab assign status={status} body={val}");
        }
        Ok(())
    }

    async fn add_labels(&self, number: u64, labels: &[String]) -> Result<()> {
        if labels.is_empty() {
            return Ok(());
        }
        let url = format!(
            "{}/projects/{}/issues/{number}",
            self.api_base, self.project
        );
        let (status, val) = self
            .http
            .request_json(
                "PUT",
                &url,
                &self.headers(),
                Some(serde_json::json!({ "add_labels": labels.join(",") })),
            )
            .await?;
        if !(200..300).contains(&status) {
            bail!("gitlab add_labels status={status} body={val}");
        }
        Ok(())
    }

    async fn close_issue(&self, number: u64, _reason: &str) -> Result<()> {
        let url = format!(
            "{}/projects/{}/issues/{number}",
            self.api_base, self.project
        );
        let (status, val) = self
            .http
            .request_json(
                "PUT",
                &url,
                &self.headers(),
                Some(serde_json::json!({ "state_event": "close" })),
            )
            .await?;
        if !(200..300).contains(&status) {
            bail!("gitlab close status={status} body={val}");
        }
        Ok(())
    }
}

// ─── Gitee / AtomGit 共有的 v5 风格路径 ─────────────────────────────────────
// 文档：https://docs.atomgit.com/docs/apis/
//   GET/POST /api/v5/repos/:owner/:repo/issues[/:number]
//   GET/POST /api/v5/repos/:owner/:repo/issues/:number/comments
//   GET/PATCH /api/v5/repos/:owner/:repo/issues/comments/:id
// 鉴权（AtomGit 文档三种都支持）：
//   Authorization: Bearer <token>  |  PRIVATE-TOKEN: <token>  |  ?access_token=

/// v5 风格鉴权策略。
#[derive(Debug, Clone, Copy)]
enum V5AuthStyle {
    /// AtomGit / GitCode：同一套后端（同一 token 通用，api.atomgit.com 与
    /// api.gitcode.com 读到同一批数据），只是域名不同。
    AtomGit,
    /// 码云 Gitee：`token` + query access_token。
    Gitee,
}

/// Gitee / AtomGit 共用实现（路径相同，鉴权不同）。
pub struct GiteeStyleIssuePlatform<H: HttpDoer> {
    pub api_base: String,
    pub repo: String,
    pub token: String,
    auth: V5AuthStyle,
    http: H,
}

/// 兼容旧名。
pub type GiteeIssuePlatform<H> = GiteeStyleIssuePlatform<H>;
/// AtomGit / AtomCode 专用类型别名。
pub type AtomGitIssuePlatform<H> = GiteeStyleIssuePlatform<H>;

impl<H: HttpDoer> GiteeStyleIssuePlatform<H> {
    pub fn new_gitee(
        api_base: impl Into<String>,
        repo: impl Into<String>,
        token: impl Into<String>,
        http: H,
    ) -> Self {
        Self {
            api_base: api_base.into().trim_end_matches('/').to_string(),
            repo: repo.into(),
            token: token.into(),
            auth: V5AuthStyle::Gitee,
            http,
        }
    }

    pub fn new_atomgit(
        api_base: impl Into<String>,
        repo: impl Into<String>,
        token: impl Into<String>,
        http: H,
    ) -> Self {
        Self {
            api_base: api_base.into().trim_end_matches('/').to_string(),
            repo: repo.into(),
            token: token.into(),
            auth: V5AuthStyle::AtomGit,
            http,
        }
    }

    /// 旧构造函数：按 Gitee 鉴权（保持兼容）。
    pub fn new(
        api_base: impl Into<String>,
        repo: impl Into<String>,
        token: impl Into<String>,
        http: H,
    ) -> Self {
        Self::new_gitee(api_base, repo, token, http)
    }

    fn headers(&self) -> Vec<(&'static str, String)> {
        match self.auth {
            // 官方文档优先 Bearer；部分网关也认 PRIVATE-TOKEN
            V5AuthStyle::AtomGit => vec![
                ("Authorization", format!("Bearer {}", self.token)),
                ("Accept", "application/json".into()),
            ],
            V5AuthStyle::Gitee => vec![("Authorization", format!("token {}", self.token))],
        }
    }

    fn url(&self, path_and_query: &str) -> String {
        let base = format!(
            "{}{}",
            self.api_base,
            if path_and_query.starts_with('/') {
                path_and_query.to_string()
            } else {
                format!("/{path_and_query}")
            }
        );
        match self.auth {
            // AtomGit：token 已在 header；仍附 access_token 提高兼容性
            V5AuthStyle::AtomGit | V5AuthStyle::Gitee => {
                let sep = if base.contains('?') { "&" } else { "?" };
                format!("{base}{sep}access_token={}", self.token)
            }
        }
    }

    /// AtomGit 的 update-issue 有三个坑，缺一不可：body 必须带 `title`（只传状态
    /// 字段时解析器判定「一个参数都没传」）；状态值是 `close` 而不是 `closed`；
    /// 路径是 Gitee 风格 `/repos/{owner}/issues/{n}`，`repo` 放 body。
    /// 因此要先取回标题，多一次 GET 换正确性。
    async fn close_issue_v5(&self, number: u64) -> Result<()> {
        let issue = self
            .get_issue(number)
            .await
            .with_context(|| format!("atomgit close_issue: fetch #{number} title"))?;
        let (owner, repo) = self
            .repo
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!("gitcode repo must be owner/repo, got {}", self.repo))?;
        let url = self.url(&format!("/repos/{owner}/issues/{number}"));
        let (status, val) = self
            .http
            .request_json(
                "PATCH",
                &url,
                &self.headers(),
                Some(serde_json::json!({
                    "repo": repo,
                    "title": issue.title,
                    "state": "close",
                })),
            )
            .await?;
        if !(200..300).contains(&status) {
            bail!("atomgit close_issue status={status} body={val}");
        }
        Ok(())
    }

    /// 指派：与 close 同一个接口，同样要求 body 里带 `title` 才会解析其余字段。
    async fn assign_v5(&self, number: u64, login: &str) -> Result<()> {
        let issue = self
            .get_issue(number)
            .await
            .with_context(|| format!("assign: fetch #{number} title"))?;
        let (owner, repo) = self
            .repo
            .split_once('/')
            .ok_or_else(|| anyhow::anyhow!("repo must be owner/repo, got {}", self.repo))?;
        let url = self.url(&format!("/repos/{owner}/issues/{number}"));
        let (status, val) = self
            .http
            .request_json(
                "PATCH",
                &url,
                &self.headers(),
                Some(serde_json::json!({
                    "repo": repo,
                    "title": issue.title,
                    "assignee": login,
                })),
            )
            .await?;
        if !(200..300).contains(&status) {
            bail!("v5 assign status={status} body={val}");
        }
        Ok(())
    }

    fn issue_path(&self, number: u64) -> String {
        format!("/repos/{}/issues/{number}", self.repo)
    }
}

/// 把 Gitee/AtomGit v5 Issue JSON 归一成 RawIssue。
pub fn map_v5_issue(v: &Value) -> Option<RawIssue> {
    let number = v
        .get("number")
        .and_then(|x| {
            x.as_u64()
                .or_else(|| x.as_i64().map(|n| n as u64))
                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        })
        .or_else(|| v.get("id").and_then(|x| x.as_u64()))?;
    let title = v
        .get("title")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let body = v
        .get("body")
        .or_else(|| v.get("description"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    let state = v
        .get("state")
        .and_then(|x| x.as_str())
        .unwrap_or("open")
        .to_string();
    let labels = map_v5_labels(v.get("labels"));
    let user = map_v5_user(v.get("user").or_else(|| v.get("author")));
    Some(RawIssue {
        number,
        title,
        body,
        state,
        labels,
        user,
        created_at: v
            .get("created_at")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        updated_at: v
            .get("updated_at")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        closed_at: v
            .get("closed_at")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
        // v5 仓库 issues 列表一般不含 PR；有 pull_request 字段则排除
        pull_request: v.get("pull_request").cloned(),
    })
}

fn map_v5_labels(v: Option<&Value>) -> Vec<RawLabel> {
    let Some(arr) = v.and_then(|x| x.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|l| {
            if let Some(s) = l.as_str() {
                return Some(RawLabel {
                    name: s.to_string(),
                });
            }
            l.get("name").and_then(|n| n.as_str()).map(|s| RawLabel {
                name: s.to_string(),
            })
        })
        .collect()
}

fn map_v5_user(v: Option<&Value>) -> Option<RawUser> {
    let u = v?;
    let login = u
        .get("login")
        .or_else(|| u.get("username"))
        .or_else(|| u.get("name"))
        .and_then(|x| x.as_str())?
        .to_string();
    let user_type = u
        .get("type")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    Some(RawUser { login, user_type })
}

fn map_v5_comment(v: &Value) -> Option<RawComment> {
    let id = v
        .get("id")
        .and_then(|x| x.as_u64().or_else(|| x.as_i64().map(|n| n as u64)))?;
    let body = v
        .get("body")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let updated_at = v
        .get("updated_at")
        .or_else(|| v.get("created_at"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    Some(RawComment {
        id,
        body,
        updated_at,
        user: map_v5_user(v.get("user").or_else(|| v.get("author"))),
    })
}

#[async_trait]
impl<H: HttpDoer> IssuePlatform for GiteeStyleIssuePlatform<H> {
    async fn get_issue(&self, number: u64) -> Result<RawIssue> {
        let url = self.url(&self.issue_path(number));
        let (status, val) = self
            .http
            .request_json("GET", &url, &self.headers(), None)
            .await?;
        if !(200..300).contains(&status) {
            bail!("v5 get_issue status={status} body={val}");
        }
        map_v5_issue(&val).context("map v5 issue")
    }

    async fn list_issues_page(
        &self,
        page: u32,
        per_page: u32,
        _since: Option<&str>,
    ) -> Result<Vec<RawIssue>> {
        // AtomGit/Gitee：state=all，按 updated 排序；since 兼容性差，忽略由上层 overlap 兜底
        let path = format!(
            "/repos/{}/issues?state=all&page={page}&per_page={per_page}&sort=updated&direction=asc",
            self.repo
        );
        let url = self.url(&path);
        let (status, val) = self
            .http
            .request_json("GET", &url, &self.headers(), None)
            .await?;
        if !(200..300).contains(&status) {
            bail!("v5 list_issues status={status} body={val}");
        }
        let arr = val.as_array().cloned().unwrap_or_default();
        Ok(arr
            .iter()
            .filter_map(map_v5_issue)
            .filter(|i| i.pull_request.is_none())
            .collect())
    }

    async fn list_comments(&self, number: u64) -> Result<Vec<RawComment>> {
        let path = format!(
            "/repos/{}/issues/{number}/comments?per_page=100&page=1",
            self.repo
        );
        let url = self.url(&path);
        let (status, val) = self
            .http
            .request_json("GET", &url, &self.headers(), None)
            .await?;
        if !(200..300).contains(&status) {
            bail!("v5 list_comments status={status}");
        }
        let arr = val.as_array().cloned().unwrap_or_default();
        Ok(arr.iter().filter_map(map_v5_comment).collect())
    }

    async fn create_comment(&self, number: u64, body: &str) -> Result<String> {
        // Create Issue Comment: POST /repos/:owner/:repo/issues/:number/comments
        let path = format!("/repos/{}/issues/{number}/comments", self.repo);
        let url = self.url(&path);
        let (status, val) = self
            .http
            .request_json(
                "POST",
                &url,
                &self.headers(),
                Some(serde_json::json!({ "body": body })),
            )
            .await?;
        // AtomGit POST 成功常为 201
        if !(200..300).contains(&status) {
            bail!("v5 create_comment status={status} body={val}");
        }
        let id = val
            .get("id")
            .and_then(|x| x.as_u64().or_else(|| x.as_i64().map(|n| n as u64)))
            .context("v5 comment id missing")?;
        Ok(id.to_string())
    }

    async fn update_comment(&self, comment_id: &str, body: &str) -> Result<()> {
        // Edit Repository Issue Comment: PATCH /repos/:owner/:repo/issues/comments/:id
        let path = format!("/repos/{}/issues/comments/{comment_id}", self.repo);
        let url = self.url(&path);
        let (status, val) = self
            .http
            .request_json(
                "PATCH",
                &url,
                &self.headers(),
                Some(serde_json::json!({ "body": body })),
            )
            .await?;
        if !(200..300).contains(&status) {
            bail!("v5 update_comment status={status} body={val}");
        }
        Ok(())
    }

    async fn find_bot_comment(&self, number: u64) -> Result<Option<String>> {
        // 取最新一条 ReviewGate 评论 id（列表可能新在前或后，两边扫）
        let comments = self.list_comments(number).await?;
        let mut found: Option<(u64, String)> = None;
        for c in comments {
            let body = c.body.replace('\u{feff}', "");
            if is_bot_comment(&body) || body.contains("ReviewGate Issue Review") {
                found = Some((c.id, c.id.to_string()));
            }
        }
        Ok(found.map(|(_, id)| id))
    }

    async fn add_labels(&self, number: u64, labels: &[String]) -> Result<()> {
        if labels.is_empty() {
            return Ok(());
        }
        // 打标签一律走独立 labels 子资源。曾经给 AtomGit 写过 PATCH issue 的实现，
        // 真机实测一直是 400（平台只接受子资源），从没工作过——只因 add_labels 默认关闭而没暴露。
        {
            let path = format!("/repos/{}/issues/{number}/labels", self.repo);
            let url = self.url(&path);
            let (status, val) = self
                .http
                .request_json(
                    "POST",
                    &url,
                    &self.headers(),
                    Some(serde_json::json!(labels)),
                )
                .await?;
            if !(200..300).contains(&status) {
                bail!("v5 add_labels status={status} body={val}");
            }
            Ok(())
        }
    }

    async fn assign(&self, number: u64, login: &str) -> Result<()> {
        self.assign_v5(number, login).await
    }

    async fn close_issue(&self, number: u64, _reason: &str) -> Result<()> {
        if matches!(self.auth, V5AuthStyle::AtomGit) {
            return self.close_issue_v5(number).await;
        }
        // 更新 Issue：PATCH body state=closed
        let path = self.issue_path(number);
        let url = self.url(&path);
        let (status, val) = self
            .http
            .request_json(
                "PATCH",
                &url,
                &self.headers(),
                Some(serde_json::json!({ "state": "closed" })),
            )
            .await?;
        if !(200..300).contains(&status) {
            bail!("v5 close_issue status={status} body={val}");
        }
        Ok(())
    }
}

/// 按 forge 构造平台客户端。
pub fn build_platform(
    forge: IssueForge,
    api_base: &str,
    repo: &str,
    token: &str,
    http: ReqwestDoer,
) -> Box<dyn IssuePlatform> {
    let base = if api_base.is_empty() {
        forge.default_api_base().to_string()
    } else {
        api_base.to_string()
    };
    match forge {
        IssueForge::GitHub => Box::new(GitHubIssuePlatform::new(base, repo, token, http)),
        IssueForge::GitLab => Box::new(GitLabIssuePlatform::new(base, repo, token, http)),
        IssueForge::Gitee => Box::new(GiteeStyleIssuePlatform::new_gitee(base, repo, token, http)),
        IssueForge::AtomGit => Box::new(GiteeStyleIssuePlatform::new_atomgit(
            base, repo, token, http,
        )),
    }
}

// ─── Fixture ────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct FixturePlatform {
    /// 记录指派结果，供离线断言。
    pub assigned: Mutex<HashMap<u64, String>>,
    issues: Mutex<HashMap<u64, RawIssue>>,
    comments: Mutex<HashMap<u64, Vec<RawComment>>>,
    labels: Mutex<HashMap<u64, Vec<String>>>,
    closed: Mutex<HashMap<u64, bool>>,
    next_comment_id: Mutex<u64>,
}

impl FixturePlatform {
    pub fn new() -> Self {
        Self {
            assigned: Mutex::new(HashMap::new()),
            issues: Mutex::new(HashMap::new()),
            comments: Mutex::new(HashMap::new()),
            labels: Mutex::new(HashMap::new()),
            closed: Mutex::new(HashMap::new()),
            next_comment_id: Mutex::new(1000),
        }
    }

    pub fn seed_issue(&self, issue: RawIssue) {
        self.issues.lock().unwrap().insert(issue.number, issue);
    }

    pub fn comment_count(&self, number: u64) -> usize {
        self.comments
            .lock()
            .unwrap()
            .get(&number)
            .map(|c| c.len())
            .unwrap_or(0)
    }

    pub fn comments_for(&self, number: u64) -> Vec<RawComment> {
        self.comments
            .lock()
            .unwrap()
            .get(&number)
            .cloned()
            .unwrap_or_default()
    }

    pub fn labels_for(&self, number: u64) -> Vec<String> {
        self.labels
            .lock()
            .unwrap()
            .get(&number)
            .cloned()
            .unwrap_or_default()
    }

    pub fn is_closed(&self, number: u64) -> bool {
        *self.closed.lock().unwrap().get(&number).unwrap_or(&false)
    }
}

#[async_trait]
impl IssuePlatform for FixturePlatform {
    async fn assign(&self, number: u64, login: &str) -> Result<()> {
        self.assigned
            .lock()
            .unwrap()
            .insert(number, login.to_string());
        Ok(())
    }

    async fn get_issue(&self, number: u64) -> Result<RawIssue> {
        self.issues
            .lock()
            .unwrap()
            .get(&number)
            .cloned()
            .with_context(|| format!("fixture issue #{number} missing"))
    }

    async fn list_issues_page(
        &self,
        page: u32,
        per_page: u32,
        _since: Option<&str>,
    ) -> Result<Vec<RawIssue>> {
        let mut all: Vec<RawIssue> = self.issues.lock().unwrap().values().cloned().collect();
        all.sort_by_key(|i| i.number);
        let start = ((page.saturating_sub(1)) * per_page) as usize;
        Ok(all
            .into_iter()
            .skip(start)
            .take(per_page as usize)
            .collect())
    }

    async fn list_comments(&self, number: u64) -> Result<Vec<RawComment>> {
        Ok(self.comments_for(number))
    }

    async fn create_comment(&self, number: u64, body: &str) -> Result<String> {
        let mut id_guard = self.next_comment_id.lock().unwrap();
        *id_guard += 1;
        let id = *id_guard;
        drop(id_guard);
        let c = RawComment {
            id,
            body: body.to_string(),
            updated_at: "now".into(),
            user: Some(RawUser {
                login: "reviewgate-bot".into(),
                user_type: Some("Bot".into()),
            }),
        };
        self.comments
            .lock()
            .unwrap()
            .entry(number)
            .or_default()
            .push(c);
        Ok(id.to_string())
    }

    async fn update_comment(&self, comment_id: &str, body: &str) -> Result<()> {
        let id: u64 = comment_id.parse().context("comment id")?;
        let mut map = self.comments.lock().unwrap();
        for list in map.values_mut() {
            if let Some(c) = list.iter_mut().find(|c| c.id == id) {
                c.body = body.to_string();
                c.updated_at = "now2".into();
                return Ok(());
            }
        }
        bail!("comment {comment_id} not found");
    }

    async fn find_bot_comment(&self, number: u64) -> Result<Option<String>> {
        for c in self.comments_for(number) {
            if is_bot_comment(&c.body) {
                return Ok(Some(c.id.to_string()));
            }
        }
        Ok(None)
    }

    async fn add_labels(&self, number: u64, labels: &[String]) -> Result<()> {
        let mut map = self.labels.lock().unwrap();
        let entry = map.entry(number).or_default();
        for l in labels {
            if !entry.contains(l) {
                entry.push(l.clone());
            }
        }
        Ok(())
    }

    async fn close_issue(&self, number: u64, _reason: &str) -> Result<()> {
        self.closed.lock().unwrap().insert(number, true);
        if let Some(i) = self.issues.lock().unwrap().get_mut(&number) {
            i.state = "closed".into();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fixture_labels_and_close() {
        let p = FixturePlatform::new();
        p.seed_issue(RawIssue {
            number: 1,
            title: "t".into(),
            body: Some("b".into()),
            state: "open".into(),
            labels: vec![],
            user: None,
            created_at: "t".into(),
            updated_at: "t".into(),
            closed_at: None,
            pull_request: None,
        });
        p.add_labels(1, &["needs-info".into(), "bug".into()])
            .await
            .unwrap();
        assert_eq!(p.labels_for(1).len(), 2);
        p.close_issue(1, "spam").await.unwrap();
        assert!(p.is_closed(1));
    }

    #[tokio::test]
    async fn fixture_publish_is_idempotent_update() {
        let p = FixturePlatform::new();
        p.seed_issue(RawIssue {
            number: 7,
            title: "t".into(),
            body: Some("b".into()),
            state: "open".into(),
            labels: vec![RawLabel { name: "bug".into() }],
            user: Some(RawUser {
                login: "u".into(),
                user_type: Some("User".into()),
            }),
            created_at: "t".into(),
            updated_at: "t".into(),
            closed_at: None,
            pull_request: None,
        });
        let body1 = format!("{BOT_COMMENT_MARKER}\n\nfirst");
        let id1 = p.create_comment(7, &body1).await.unwrap();
        assert_eq!(p.comment_count(7), 1);
        let found = p.find_bot_comment(7).await.unwrap().unwrap();
        assert_eq!(found, id1);
        let body2 = format!("{BOT_COMMENT_MARKER}\n\nsecond");
        p.update_comment(&found, &body2).await.unwrap();
        assert_eq!(p.comment_count(7), 1);
    }

    #[test]
    fn forge_parse() {
        assert_eq!(IssueForge::parse("gitlab"), Some(IssueForge::GitLab));
        assert_eq!(IssueForge::parse("gitee"), Some(IssueForge::Gitee));
        assert_eq!(IssueForge::parse("atomgit"), Some(IssueForge::AtomGit));
        assert_eq!(IssueForge::parse("atom"), Some(IssueForge::AtomGit));
        // GitCode 是 AtomGit 的旧称，同一套后端（同 token 通用、同一批数据），
        // 不单独建 forge。
        assert_eq!(IssueForge::parse("gitcode"), None);
        assert_eq!(
            IssueForge::AtomGit.default_api_base(),
            "https://api.atomgit.com/api/v5"
        );
    }

    /// 曾经这里断言 AtomGit 用 `PATCH /issues/{n}` 打标签、不走子资源。
    /// 那个断言只检查了请求形状，没检查平台接不接受——真机实测一直是 400，
    /// 功能从未工作过，只因 add_labels 默认关闭而没暴露。
    #[tokio::test]
    async fn v5_add_labels_uses_the_labels_sub_resource() {
        use std::sync::Mutex;
        struct Capture {
            calls: Mutex<Vec<(String, String, Option<Value>)>>,
        }
        #[async_trait]
        impl HttpDoer for Capture {
            async fn request_json(
                &self,
                method: &str,
                url: &str,
                _headers: &[(&str, String)],
                body: Option<Value>,
            ) -> Result<(u16, Value)> {
                self.calls
                    .lock()
                    .unwrap()
                    .push((method.to_string(), url.to_string(), body));
                Ok((201, serde_json::json!([{"name": "bug"}])))
            }
        }
        // gitcode.com 是同一后端的旧域名，同样走这条实现。
        for base in [
            "https://api.atomgit.com/api/v5",
            "https://api.gitcode.com/api/v5",
        ] {
            let http = Capture {
                calls: Mutex::new(Vec::new()),
            };
            let p = GiteeStyleIssuePlatform::new_atomgit(base, "o/r", "tok", http);
            p.add_labels(7, &["bug".into(), "needs-info".into()])
                .await
                .unwrap();
            let calls = p.http.calls.lock().unwrap();
            assert_eq!(calls[0].0, "POST", "{base}");
            assert!(
                calls[0].1.contains("/repos/o/r/issues/7/labels"),
                "must use the labels sub-resource: {}",
                calls[0].1
            );
            let body = calls[0].2.as_ref().unwrap();
            assert_eq!(
                body.as_array().map(|a| a.len()),
                Some(2),
                "body is a bare array"
            );
        }
    }

    #[test]
    fn map_v5_issue_labels_as_strings_or_objects() {
        let v = serde_json::json!({
            "number": 42,
            "title": "crash",
            "body": "panic",
            "state": "open",
            "labels": ["bug", {"name": "P0"}],
            "user": {"login": "alice"},
            "created_at": "t",
            "updated_at": "t"
        });
        let i = map_v5_issue(&v).unwrap();
        assert_eq!(i.number, 42);
        assert_eq!(i.labels.len(), 2);
        assert_eq!(i.user.as_ref().unwrap().login, "alice");
    }

    #[test]
    fn atomgit_auth_headers_use_bearer() {
        // 用假 HttpDoer 不需要，直接构造检查 headers 逻辑
        struct Noop;
        #[async_trait]
        impl HttpDoer for Noop {
            async fn request_json(
                &self,
                _: &str,
                _: &str,
                _: &[(&str, String)],
                _: Option<Value>,
            ) -> Result<(u16, Value)> {
                Ok((200, Value::Null))
            }
        }
        let p = GiteeStyleIssuePlatform::new_atomgit(
            "https://api.atomgit.com/api/v5",
            "o/r",
            "tok123",
            Noop,
        );
        let h = p.headers();
        assert_eq!(h[0].0, "Authorization");
        assert_eq!(h[0].1, "Bearer tok123");
        let url = p.url("/repos/o/r/issues/1");
        assert!(url.starts_with("https://api.atomgit.com/api/v5/repos/o/r/issues/1"));
        assert!(url.contains("access_token=tok123"));
    }
}

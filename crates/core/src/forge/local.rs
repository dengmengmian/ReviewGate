//! 本地兜底：从**已认证的 `gh` / `glab` CLI** 取 PR/MR 上下文，省掉「本地跑还要先配 token」。
//!
//! 只在 CI 环境变量（`GITHUB_*` / `CI_*` / `REVIEWGATE_*`）解析不出上下文时才启用，
//! CI 行为完全不变。取到的 token 只用于本次请求，不打印、不落盘。
//!
//! 平台按 `origin` 远端主机判定：`github*` → `gh`，`gitlab*` → `glab`。
//! 任一步失败都返回 `None`，调用方跳过评论并给出可执行提示——绝不半路乱发到错误的仓库。
//!
//! **主机必须先过白名单**：API base 是从 `origin` 远端字符串推出来的，而请求要带上
//! `gh auth token` 拿到的真 token。若不校验，一个 `git@github.evil.com:a/b.git` 远端
//! 就能把用户的 GitHub token 送到攻击者服务器。因此只信任 `gh auth status` /
//! `glab auth status` 明确报告已登录的主机——没登录过的主机一律不发。

use super::{Forge, ForgeContext};
use std::collections::HashSet;
use tokio::process::Command;

/// 从本地 CLI 解析上下文。非 git 仓库 / CLI 缺失 / 未认证 / 当前分支没有 PR → `None`。
pub async fn resolve_context_from_cli() -> Option<ForgeContext> {
    let origin = capture("git", &["remote", "get-url", "origin"]).await?;
    let host = remote_host(&origin)?;
    if host.contains("github") {
        gh_context(&host).await
    } else if host.contains("gitlab") {
        glab_context(&host).await
    } else {
        None
    }
}

/// 从 git 远端 URL 抽主机名。支持 `https://host/a/b.git` 与 `git@host:a/b.git`。
pub fn remote_host(url: &str) -> Option<String> {
    let url = url.trim();
    if let Some(rest) = url.split("://").nth(1) {
        // 去掉可能的 user@ 与端口。
        let authority = rest.split('/').next()?;
        let authority = authority.rsplit('@').next()?;
        return Some(authority.split(':').next()?.to_ascii_lowercase());
    }
    if let Some(rest) = url.split_once('@') {
        return Some(rest.1.split(':').next()?.to_ascii_lowercase());
    }
    None
}

/// 从 `gh auth status` 输出里取**已登录**的主机名集合。
///
/// gh 的输出形如：顶格一行主机名，随后是缩进的账号明细。只认顶格行，避免把
/// "Logged in to github.com account …" 里出现的字符串当成另一个主机。
pub fn parse_gh_hosts(status: &str) -> HashSet<String> {
    status
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with(char::is_whitespace))
        .map(|l| l.trim().trim_end_matches(':').to_ascii_lowercase())
        .filter(|l| l.contains('.') && !l.contains(' '))
        .collect()
}

/// 该主机是否在已登录白名单里。精确匹配——子域名不继承信任。
pub fn is_trusted_host(host: &str, trusted: &HashSet<String>) -> bool {
    trusted.contains(&host.to_ascii_lowercase())
}

/// GitHub：`gh repo view` / `gh pr view` / `gh auth token`（三者都需要 gh 已登录该主机）。
async fn gh_context(host: &str) -> Option<ForgeContext> {
    // 先校验主机：token 只能发给 gh 自己登录过的主机。
    let status = capture_merged("gh", &["auth", "status"]).await?;
    if !is_trusted_host(host, &parse_gh_hosts(&status)) {
        eprintln!("  [forge] `gh` is not authenticated to {host}; refusing to send a token there");
        return None;
    }
    let repo = capture(
        "gh",
        &[
            "repo",
            "view",
            "--json",
            "nameWithOwner",
            "-q",
            ".nameWithOwner",
        ],
    )
    .await?;
    let number: u64 = capture("gh", &["pr", "view", "--json", "number", "-q", ".number"])
        .await?
        .trim()
        .parse()
        .ok()?;
    // 必须显式指定 --hostname：多主机登录时 `gh auth token` 默认返回**活跃主机**的 token，
    // 那可能不是我们刚校验过的这台，等于把 A 站的 token 发给 B 站。
    let token_args = gh_token_args(host);
    let token = capture("gh", &token_args).await?.trim().to_string();
    if token.is_empty() {
        return None;
    }
    Some(ForgeContext {
        forge: Forge::GitHub,
        api_base: github_api_base(host),
        repo: repo.trim().to_string(),
        number,
        token,
    })
}

/// 取 token 的命令参数。必须显式指定 `--hostname`：多主机登录时 `gh auth token` 默认返回
/// **活跃主机**的 token，那可能不是我们刚校验过的这台，等于把 A 站的 token 发给 B 站。
fn gh_token_args(host: &str) -> [&str; 4] {
    ["auth", "token", "--hostname", host]
}

/// GitHub Enterprise 的 API base 与 github.com 不同（`/api/v3` 前缀）。
///
/// 只有 `github.com` 精确匹配走 api.github.com；其余（含 `*.github.com` 形态的自建实例）
/// 一律按 GHE 处理——把某个 `x.github.com` 的请求发去 api.github.com 会打到错误的实例。
pub fn github_api_base(host: &str) -> String {
    if host == "github.com" {
        "https://api.github.com".to_string()
    } else {
        format!("https://{host}/api/v3")
    }
}

/// GitLab：`glab mr view -F json` 取 iid/project_id，`glab auth status --show-token` 取 token。
async fn glab_context(host: &str) -> Option<ForgeContext> {
    // 同 GitHub：先确认 glab 确实登录过这个主机，再谈 token。
    let status = capture_merged("glab", &["auth", "status", "--show-token"]).await?;
    if !is_trusted_host(host, &parse_gh_hosts(&status)) {
        eprintln!(
            "  [forge] `glab` is not authenticated to {host}; refusing to send a token there"
        );
        return None;
    }
    let json = capture("glab", &["mr", "view", "-F", "json"]).await?;
    let (number, project) = parse_glab_mr(&json)?;
    let token = parse_glab_token(&status)?;
    Some(ForgeContext {
        forge: Forge::GitLab,
        api_base: format!("https://{host}/api/v4"),
        repo: project,
        number,
        token,
    })
}

/// 从 `glab mr view -F json` 输出里取 (iid, project_id)。两者缺一不可——
/// 没有 project_id 就无法拼出 GitLab 的 notes 端点，宁可返回 None 也不猜。
pub fn parse_glab_mr(json: &str) -> Option<(u64, String)> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let iid = v.get("iid").and_then(|x| x.as_u64())?;
    let project = v.get("project_id").and_then(|x| x.as_u64())?;
    Some((iid, project.to_string()))
}

/// 从 `glab auth status --show-token` 输出里取 token 行（`Token: glpat-...`）。
pub fn parse_glab_token(text: &str) -> Option<String> {
    for line in text.lines() {
        if let Some((_, rest)) = line.split_once("Token:") {
            let token = rest.trim();
            // 打码过的 token（glab 不带 --show-token 时输出 `**...`）不可用。
            if !token.is_empty() && !token.starts_with('*') {
                return Some(token.to_string());
            }
        }
    }
    None
}

/// 跑一条命令取 stdout；非零退出或命令不存在 → `None`（兜底路径不该因此报错）。
async fn capture(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 同 [`capture`]，但把 stderr 也并进来（glab 的状态输出走 stderr）。
async fn capture_merged(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    let mut s = String::from_utf8_lossy(&out.stdout).into_owned();
    s.push('\n');
    s.push_str(&String::from_utf8_lossy(&out.stderr));
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_host_parses_https_and_ssh() {
        assert_eq!(
            remote_host("https://github.com/a/b.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            remote_host("git@github.com:a/b.git").as_deref(),
            Some("github.com")
        );
        assert_eq!(
            remote_host("ssh://git@gitlab.example.com:2222/a/b.git").as_deref(),
            Some("gitlab.example.com")
        );
        assert_eq!(
            remote_host("https://user@git.corp.io/a/b").as_deref(),
            Some("git.corp.io")
        );
        assert_eq!(remote_host("/local/path/repo"), None);
    }

    #[test]
    fn github_api_base_handles_enterprise() {
        assert_eq!(github_api_base("github.com"), "https://api.github.com");
        assert_eq!(
            github_api_base("github.corp.io"),
            "https://github.corp.io/api/v3"
        );
        // 自建实例即使叫 x.github.com，也不能被当成公有 github.com。
        assert_eq!(
            github_api_base("ghe.github.com"),
            "https://ghe.github.com/api/v3"
        );
    }

    #[test]
    fn parse_glab_mr_needs_both_iid_and_project() {
        let ok = r#"{"iid": 42, "project_id": 7, "title": "x"}"#;
        assert_eq!(parse_glab_mr(ok), Some((42, "7".to_string())));
        assert_eq!(parse_glab_mr(r#"{"iid": 42}"#), None);
        assert_eq!(parse_glab_mr("not json"), None);
    }

    #[test]
    fn parse_glab_token_ignores_masked_output() {
        let shown = "gitlab.com\n  ✓ Logged in\n  ✓ Token: glpat-abc123\n";
        assert_eq!(parse_glab_token(shown).as_deref(), Some("glpat-abc123"));
        let masked = "gitlab.com\n  ✓ Token: **************\n";
        assert_eq!(parse_glab_token(masked), None);
        assert_eq!(parse_glab_token("no token here"), None);
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;

    #[test]
    fn token_lookup_is_pinned_to_the_validated_host() {
        // 取 token 的命令必须带 --hostname，否则多主机登录下会拿到活跃主机的 token，
        // 把刚做的白名单校验架空。
        let args = gh_token_args("github.corp.io");
        assert_eq!(args, ["auth", "token", "--hostname", "github.corp.io"]);
    }

    #[test]
    fn only_hosts_gh_is_authenticated_to_are_trusted() {
        // `gh auth status` 列出的已登录主机才可信。
        let status = "github.com\n  ✓ Logged in to github.com account alice (keyring)\n  - Active account: true\n";
        let hosts = parse_gh_hosts(status);
        assert!(hosts.contains("github.com"));
        // 攻击者把 origin 指到 github.evil.com：主机名里含 "github"，但 gh 没登录过它。
        // 必须判定为不可信——否则 token 会被发到攻击者服务器。
        assert!(!hosts.contains("github.evil.com"));
        assert!(!is_trusted_host("github.evil.com", &hosts));
        assert!(is_trusted_host("github.com", &hosts));
    }

    #[test]
    fn enterprise_host_is_trusted_only_when_logged_in() {
        let status = "github.corp.io\n  ✓ Logged in to github.corp.io account bob (oauth_token)\n";
        let hosts = parse_gh_hosts(status);
        assert!(is_trusted_host("github.corp.io", &hosts));
        assert!(!is_trusted_host("github.com", &hosts));
    }

    #[test]
    fn empty_status_trusts_nothing() {
        let hosts = parse_gh_hosts("");
        assert!(!is_trusted_host("github.com", &hosts));
    }
}

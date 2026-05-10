//! GitHub API client wrapper. Thin layer over `octocrab` to keep the
//! handler-side code free of `octocrab` types and to centralize error
//! mapping into our `thiserror` enum.

use serde::Serialize;

/// Connected GitHub user, cached after `validate()` returns Ok.
#[derive(Debug, Clone, Serialize)]
pub struct User {
    pub login: String,
    pub avatar_url: String,
}

/// Per-PR summary returned to the dashboard. Drops fields the panel
/// doesn't render; the user clicks `url` to see full details on github.com.
#[derive(Debug, Clone, Serialize)]
pub struct PrSummary {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub draft: bool,
    pub updated_at: String, // RFC 3339
    pub url: String,        // html_url
}

/// Per-issue summary; same rationale as `PrSummary`.
#[derive(Debug, Clone, Serialize)]
pub struct IssueSummary {
    pub number: u64,
    pub title: String,
    pub author: String,
    pub labels: Vec<String>,
    pub updated_at: String,
    pub url: String,
}

/// Plan 10b — created-PR info returned by `create_pull_request`. The
/// `html_url` is what we persist on the session row; `number` is for
/// future webhook-correlation work.
#[derive(Debug, Clone, Serialize)]
pub struct PrInfo {
    pub number: u64,
    pub html_url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum GithubError {
    #[error("github api: {0}")]
    Api(String),
    #[error("network: {0}")]
    Network(String),
}

/// Wraps an `octocrab::Octocrab` instance configured with PAT auth + an
/// optional alternate base URL (used by mockito tests to redirect to a
/// local mock server).
pub struct Client {
    inner: octocrab::Octocrab,
}

impl Client {
    /// Build a client from `GITHUB_TOKEN` env var. Returns `None` if the
    /// var is unset or empty. Does NOT validate the token — call
    /// `validate()` for that.
    pub fn from_env() -> Option<Self> {
        let token = std::env::var("GITHUB_TOKEN").ok().filter(|s| !s.is_empty())?;
        Self::with_token(&token, None).ok()
    }

    /// Construct a client with a specific token and optional base URL.
    /// The base URL override is the seam tests use to redirect to a
    /// mockito server; production callers should pass `None`.
    pub fn with_token(token: &str, base_url: Option<String>) -> Result<Self, GithubError> {
        let mut builder = octocrab::Octocrab::builder().personal_token(token.to_string());
        if let Some(url) = base_url {
            builder = builder
                .base_uri(url)
                .map_err(|e| GithubError::Api(format!("base_uri: {e}")))?;
        }
        let inner = builder
            .build()
            .map_err(|e| GithubError::Api(format!("build: {e}")))?;
        Ok(Self { inner })
    }

    /// Hit `GET /user` and return the connected `User`. Returns `Err` on
    /// any HTTP error (including 401 invalid-token). Caller decides whether
    /// to treat that as "not connected" vs surface the error verbatim.
    pub async fn validate(&self) -> Result<User, GithubError> {
        let u: octocrab::models::Author = self.inner.current().user().await.map_err(map_err)?;
        Ok(User {
            login: u.login,
            avatar_url: u.avatar_url.to_string(),
        })
    }

    /// List open PRs for `owner/name`. Caps at 50 per the spec; for v1
    /// that's enough (users with more open PRs use github.com directly).
    pub async fn list_open_prs(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Vec<PrSummary>, GithubError> {
        let page = self
            .inner
            .pulls(owner, name)
            .list()
            .state(octocrab::params::State::Open)
            .per_page(50)
            .send()
            .await
            .map_err(map_err)?;
        Ok(page
            .items
            .into_iter()
            .map(|p| PrSummary {
                number: p.number,
                title: p.title.unwrap_or_default(),
                author: p.user.map(|u| u.login).unwrap_or_default(),
                draft: p.draft.unwrap_or(false),
                updated_at: p.updated_at.map(|t| t.to_rfc3339()).unwrap_or_default(),
                url: p.html_url.map(|u| u.to_string()).unwrap_or_default(),
            })
            .collect())
    }

    /// List open issues for `owner/name`. GitHub's API returns PRs in the
    /// issues list; we filter them out (every PR is an issue, but only
    /// non-PR issues belong here).
    pub async fn list_open_issues(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Vec<IssueSummary>, GithubError> {
        let page = self
            .inner
            .issues(owner, name)
            .list()
            .state(octocrab::params::State::Open)
            .per_page(50)
            .send()
            .await
            .map_err(map_err)?;
        Ok(page
            .items
            .into_iter()
            .filter(|i| i.pull_request.is_none())
            .map(|i| IssueSummary {
                number: i.number,
                title: i.title,
                author: i.user.login,
                labels: i.labels.into_iter().map(|l| l.name).collect(),
                updated_at: i.updated_at.to_rfc3339(),
                url: i.html_url.to_string(),
            })
            .collect())
    }

    /// Validate a repo exists (and the PAT can see it). Returns Ok on
    /// 200, Err on any other status. Used by the link-repo endpoint
    /// before we write to the DB.
    pub async fn validate_repo(&self, owner: &str, name: &str) -> Result<(), GithubError> {
        self.inner
            .repos(owner, name)
            .get()
            .await
            .map_err(map_err)?;
        Ok(())
    }

    /// Plan 10b — fetch the repo's default branch name (e.g. "main" or
    /// "master"). Used to pre-populate the base-branch field in the
    /// OpenPrModal. Caller falls back to "main" on Err.
    pub async fn default_branch(&self, owner: &str, name: &str) -> Result<String, GithubError> {
        let repo = self.inner.repos(owner, name).get().await.map_err(map_err)?;
        Ok(repo.default_branch.unwrap_or_else(|| "main".to_string()))
    }

    /// Plan 10b Task 4 — fetch the comma-separated `x-oauth-scopes`
    /// header from `GET /user`. Returns Vec of trimmed scope strings,
    /// or empty Vec if the header is absent (which can happen if the
    /// PAT has no scopes set, common for fine-grained PATs).
    ///
    /// Uses octocrab's low-level `_get` so we get back the raw
    /// `http::Response` and can read response headers — the typed
    /// `current().user()` path only deserializes the body.
    pub async fn token_scopes(&self) -> Result<Vec<String>, GithubError> {
        let resp = self.inner._get("/user").await.map_err(map_err)?;
        let scopes = resp
            .headers()
            .get("x-oauth-scopes")
            .and_then(|v| v.to_str().ok())
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        Ok(scopes)
    }

    /// Plan 10b — create a PR via `POST /repos/{owner}/{name}/pulls`.
    /// Returns the html_url + number on success; surfaces octocrab's
    /// error verbatim on failure (e.g. 422 if a PR already exists for
    /// the head branch on the remote, 404 if base branch missing).
    // Single internal caller (open_pr handler); a builder/struct would be
    // YAGNI for 8 args, all of which are required and all of which the
    // caller already has in scope.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_pull_request(
        &self,
        owner: &str,
        name: &str,
        head: &str,
        base: &str,
        title: &str,
        body: &str,
        draft: bool,
    ) -> Result<PrInfo, GithubError> {
        let pr = self
            .inner
            .pulls(owner, name)
            .create(title, head, base)
            .body(body)
            .draft(draft)
            .send()
            .await
            .map_err(map_err)?;
        Ok(PrInfo {
            number: pr.number,
            html_url: pr.html_url.map(|u| u.to_string()).unwrap_or_default(),
        })
    }
}

/// Map an `octocrab::Error` into our two-variant `GithubError`.
///
/// `octocrab::Error` is non-exhaustive. Its `Display` impl only renders
/// the variant name (e.g. "GitHub"), which is useless for surfacing 422
/// validation errors to the UI; `Debug` includes the status code +
/// response body, so we prefer it.
///
/// The `Network` vs `Api` split is best-effort: we substring-match the
/// Debug repr for "connect" / "dns" / "timeout" rather than pattern-match
/// the variant. This can misclassify if octocrab ever embeds those
/// substrings in a non-network context (e.g. a GitHub error message that
/// happens to contain the word "timeout"). For Plan 10b that's
/// cosmetic — both variants are surfaced by current callers as a 502
/// with the same string body, so the distinction is purely for log
/// readability. If a future caller needs to differentiate (e.g. retry
/// only on `Network`), tighten this to variant-pattern matching against
/// `octocrab::Error::Hyper` / `octocrab::Error::Http` etc.
fn map_err(e: octocrab::Error) -> GithubError {
    let s = format!("{e:?}");
    if s.contains("connect") || s.contains("dns") || s.contains("timeout") {
        GithubError::Network(s)
    } else {
        GithubError::Api(s)
    }
}

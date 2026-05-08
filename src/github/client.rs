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
}

fn map_err(e: octocrab::Error) -> GithubError {
    // octocrab::Error is non-exhaustive; the Display impl is informative
    // enough for our purposes (UI surfaces it verbatim).
    let s = format!("{e}");
    if s.contains("connect") || s.contains("dns") || s.contains("timeout") {
        GithubError::Network(s)
    } else {
        GithubError::Api(s)
    }
}

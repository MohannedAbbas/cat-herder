//! GitHub OAuth single sign-on.
//!
//! Implements the OAuth 2.0 authorization-code flow against GitHub:
//!   1. `GET /v1/auth/github/login` redirects the browser to GitHub's authorize URL.
//!   2. `GET /v1/auth/github/callback?code=…` exchanges the code for an access token,
//!      fetches the authenticated user, checks their login against an allow-list,
//!      and issues an admin session (reusing [`Auth`]'s session store).
//!
//! Credentials and the allow-list come from the environment (see [`OAuthConfig`]);
//! nothing is hardcoded. The HTTP endpoints are in [`crate::admin`]; this module
//! holds the config parsing plus the GitHub client calls so the request/response
//! plumbing (and the allow-list / URL logic) is unit-testable without a network.

use cat_herder_domain::ports::IdentityProvider;

/// GitHub OAuth endpoints (stable, public).
pub const GITHUB_AUTHORIZE_URL: &str = "https://github.com/login/oauth/authorize";
pub const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
pub const GITHUB_USER_URL: &str = "https://api.github.com/user";

/// GitHub OAuth configuration, loaded from the environment.
#[derive(Clone)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Base URL of this deployment (e.g. `https://cat-herder.example.com`); the
    /// callback URL is derived by appending [`Self::CALLBACK_PATH`].
    pub redirect_url: String,
    /// GitHub logins permitted to become admin (case-insensitive).
    pub allowed_users: Vec<String>,
}

impl OAuthConfig {
    /// Path of the OAuth callback endpoint (registered on the router).
    pub const CALLBACK_PATH: &str = "/v1/auth/github/callback";

    /// Load from `GITHUB_OAUTH_CLIENT_ID`, `GITHUB_OAUTH_CLIENT_SECRET`,
    /// `GITHUB_OAUTH_REDIRECT_URL`, and `GITHUB_OAUTH_ALLOWED_USERS` (comma-separated).
    /// Returns `None` when any of the required values are missing/empty, in which
    /// case GitHub SSO is simply unavailable and password login still works.
    pub fn from_env() -> Option<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Build from an arbitrary key→value lookup (lets tests avoid touching the
    /// process-global environment).
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let required = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let allowed_users = parse_allowed_users(lookup("GITHUB_OAUTH_ALLOWED_USERS").as_deref());
        Some(Self {
            client_id: required("GITHUB_OAUTH_CLIENT_ID")?,
            client_secret: required("GITHUB_OAUTH_CLIENT_SECRET")?,
            redirect_url: required("GITHUB_OAUTH_REDIRECT_URL")?,
            allowed_users,
        })
    }

    /// The absolute callback URL to register with GitHub and send in the token
    /// exchange (GitHub requires them to match).
    pub fn callback_url(&self) -> String {
        format!(
            "{}{}",
            self.redirect_url.trim_end_matches('/'),
            Self::CALLBACK_PATH
        )
    }

    /// Whether the given GitHub login may sign in. Matching is case-insensitive
    /// (GitHub logins are case-insensitive).
    pub fn is_allowed(&self, login: &str) -> bool {
        let login = login.trim();
        !login.is_empty() && self.allowed_users.iter().any(|u| u.eq_ignore_ascii_case(login))
    }

    /// The URL to redirect the browser to in order to start the flow.
    pub fn authorize_url(&self) -> String {
        format!(
            "{GITHUB_AUTHORIZE_URL}?client_id={}&redirect_uri={}&scope=read:user",
            urlencode(&self.client_id),
            urlencode(&self.callback_url()),
        )
    }
}

/// Parse a comma-separated allow-list, trimming whitespace and dropping blanks.
pub fn parse_allowed_users(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Minimal percent-encoding for URL query components (RFC 3986 unreserved kept).
fn urlencode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// An authenticated GitHub user (subset of the `/user` payload we care about).
#[derive(Debug, Clone, PartialEq)]
pub struct GitHubUser {
    pub login: String,
}

/// Errors surfaced by the GitHub client.
#[derive(Debug)]
pub enum OAuthError {
    /// GitHub rejected the token exchange (bad/expired code, misconfigured client).
    TokenExchange(String),
    /// The user fetch failed.
    UserFetch(String),
    /// Transport/JSON error.
    Transport(String),
}

impl std::fmt::Display for OAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OAuthError::TokenExchange(m) => write!(f, "token exchange failed: {m}"),
            OAuthError::UserFetch(m) => write!(f, "user fetch failed: {m}"),
            OAuthError::Transport(m) => write!(f, "transport error: {m}"),
        }
    }
}

impl std::error::Error for OAuthError {}

/// The GitHub OAuth client: performs the token exchange and user fetch over HTTP.
#[derive(Clone)]
pub struct GitHubOAuth {
    config: OAuthConfig,
    http: reqwest::Client,
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(serde::Deserialize)]
struct UserResponse {
    login: String,
}

impl GitHubOAuth {
    pub fn new(config: OAuthConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::new(),
        }
    }

    pub fn config(&self) -> &OAuthConfig {
        &self.config
    }

    /// Exchange an authorization code for an access token.
    pub async fn exchange_code(&self, code: &str) -> Result<String, OAuthError> {
        let callback_url = self.config.callback_url();
        let params = [
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.as_str()),
            ("code", code),
            ("redirect_uri", callback_url.as_str()),
        ];
        let resp = self
            .http
            .post(GITHUB_TOKEN_URL)
            .header(reqwest::header::ACCEPT, "application/json")
            .form(&params)
            .send()
            .await
            .map_err(|e| OAuthError::Transport(e.to_string()))?;
        let body: TokenResponse = resp
            .json()
            .await
            .map_err(|e| OAuthError::Transport(e.to_string()))?;
        match body.access_token {
            Some(t) if !t.is_empty() => Ok(t),
            _ => Err(OAuthError::TokenExchange(
                body.error_description
                    .or(body.error)
                    .unwrap_or_else(|| "no access token returned".into()),
            )),
        }
    }

    /// Fetch the authenticated user for the given access token.
    pub async fn fetch_user(&self, access_token: &str) -> Result<GitHubUser, OAuthError> {
        let resp = self
            .http
            .get(GITHUB_USER_URL)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header(reqwest::header::USER_AGENT, "cat-herder")
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|e| OAuthError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(OAuthError::UserFetch(format!("HTTP {}", resp.status())));
        }
        let user: UserResponse = resp
            .json()
            .await
            .map_err(|e| OAuthError::Transport(e.to_string()))?;
        Ok(GitHubUser { login: user.login })
    }

    /// Run the full flow: exchange the code, fetch the user, enforce the allow-list.
    /// Returns the GitHub login on success.
    pub async fn authenticate(&self, code: &str) -> Result<String, AuthOutcome> {
        let token = self.exchange_code(code).await.map_err(|_| AuthOutcome::ExchangeFailed)?;
        let user = self.fetch_user(&token).await.map_err(|_| AuthOutcome::UserFetchFailed)?;
        if !self.config.is_allowed(&user.login) {
            return Err(AuthOutcome::NotAllowed);
        }
        Ok(user.login)
    }
}

/// Why a GitHub login attempt failed (mapped to HTTP responses by the handler).
#[derive(Debug, PartialEq)]
pub enum AuthOutcome {
    ExchangeFailed,
    UserFetchFailed,
    NotAllowed,
}

impl IdentityProvider for GitHubOAuth {
    /// Not used by the browser flow; validates a GitHub *access token* by resolving
    /// the user it belongs to and requiring that user to be allow-listed. Returns a
    /// fixed tenant (single-tenant control plane today).
    async fn validate(
        &self,
        token: &str,
    ) -> cat_herder_domain::ports::DomainResult<cat_herder_domain::entities::TenantId> {
        let user = self.fetch_user(token).await?;
        if self.config.is_allowed(&user.login) {
            Ok(cat_herder_domain::entities::TenantId(1))
        } else {
            Err("user not in allow-list".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(allowed: &[&str]) -> OAuthConfig {
        OAuthConfig {
            client_id: "cid".into(),
            client_secret: "secret".into(),
            redirect_url: "https://cat-herder.example.com".into(),
            allowed_users: allowed.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn parse_allowed_users_trims_and_drops_blanks() {
        assert_eq!(
            parse_allowed_users(Some(" alice , bob ,, ")),
            vec!["alice".to_string(), "bob".to_string()]
        );
    }

    #[test]
    fn parse_allowed_users_none_or_empty() {
        assert!(parse_allowed_users(None).is_empty());
        assert!(parse_allowed_users(Some("")).is_empty());
        assert!(parse_allowed_users(Some("  ,  ")).is_empty());
    }

    #[test]
    fn is_allowed_is_case_insensitive_and_rejects_unknowns() {
        let c = cfg(&["Octocat"]);
        assert!(c.is_allowed("octocat"));
        assert!(c.is_allowed("  OctoCat  "));
        assert!(!c.is_allowed("someone-else"));
        assert!(!c.is_allowed(""));
    }

    #[test]
    fn is_allowed_empty_list_denies_everyone() {
        let c = cfg(&[]);
        assert!(!c.is_allowed("octocat"));
    }

    #[test]
    fn callback_url_handles_trailing_slash() {
        let mut c = cfg(&[]);
        assert_eq!(
            c.callback_url(),
            "https://cat-herder.example.com/v1/auth/github/callback"
        );
        c.redirect_url = "https://cat-herder.example.com/".into();
        assert_eq!(
            c.callback_url(),
            "https://cat-herder.example.com/v1/auth/github/callback"
        );
    }

    #[test]
    fn authorize_url_includes_encoded_params() {
        let c = cfg(&[]);
        let url = c.authorize_url();
        assert!(url.starts_with(GITHUB_AUTHORIZE_URL));
        assert!(url.contains("client_id=cid"));
        // The callback URL's slashes and colons must be percent-encoded.
        assert!(url.contains("redirect_uri=https%3A%2F%2Fcat-herder.example.com%2Fv1%2Fauth%2Fgithub%2Fcallback"));
        assert!(url.contains("scope=read:user"));
    }

    #[test]
    fn from_lookup_requires_all_secrets() {
        let lookup = |k: &str| -> Option<String> {
            match k {
                "GITHUB_OAUTH_CLIENT_ID" => Some("cid".into()),
                "GITHUB_OAUTH_CLIENT_SECRET" => Some("sec".into()),
                "GITHUB_OAUTH_REDIRECT_URL" => Some("https://x.example".into()),
                "GITHUB_OAUTH_ALLOWED_USERS" => Some("alice,bob".into()),
                _ => None,
            }
        };
        let c = OAuthConfig::from_lookup(lookup).expect("configured");
        assert_eq!(c.client_id, "cid");
        assert_eq!(c.allowed_users, vec!["alice".to_string(), "bob".to_string()]);

        // Missing client secret => SSO disabled.
        assert!(OAuthConfig::from_lookup(|k| (k != "GITHUB_OAUTH_CLIENT_SECRET").then(|| "x".into())).is_none());
    }

    #[test]
    fn from_lookup_treats_blank_as_missing() {
        let lookup = |k: &str| -> Option<String> {
            if k == "GITHUB_OAUTH_CLIENT_ID" { Some("  ".into()) } else { Some("v".into()) }
        };
        assert!(OAuthConfig::from_lookup(lookup).is_none());
    }

    #[test]
    fn urlencode_encodes_reserved() {
        assert_eq!(urlencode("a b/c:d"), "a%20b%2Fc%3Ad");
        assert_eq!(urlencode("abc-._~"), "abc-._~");
    }
}

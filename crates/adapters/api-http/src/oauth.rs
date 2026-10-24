//! OAuth 2.0 single sign-on (GitHub and GitLab).
//!
//! Both providers use the OAuth 2.0 authorization-code flow:
//!   1. `GET /v1/auth/<provider>/login` redirects the browser to the provider's
//!      authorize URL.
//!   2. `GET /v1/auth/<provider>/callback?code=…` exchanges the code for an access
//!      token, fetches the authenticated user, checks their login against an
//!      allow-list, and issues an admin session (reusing [`Auth`]'s session store).
//!
//! Credentials and the allow-list come from the environment; nothing is hardcoded.
//! The HTTP endpoints are in [`crate::admin`]; this module holds the config parsing
//! plus the provider client calls so the request/response plumbing (and the
//! allow-list / URL logic) is unit-testable without a network.
//!
//! The flow is shared through [`OAuthProvider`] (provider descriptors) and
//! [`OAuthClient`] (the generic HTTP client). `GitHubOAuth` / `GitLabOAuth` are thin
//! newtypes over the shared client so the two providers stay identical in behaviour.

use cat_herder_domain::ports::IdentityProvider;

/// Static description of an OAuth provider's endpoints and identity semantics.
pub struct OAuthProvider {
    /// Name used in env var prefixes (`GITHUB_` / `GITLAB_`), route paths, cookies,
    /// and log messages. Upper-case.
    pub name: &'static str,
    /// Default public-cloud OAuth endpoints. Overridable at runtime via
    /// `<NAME>_OAUTH_BASE_URL` (see [`OAuthConfig::authorize_url`] /
    /// [`OAuthClient::exchange_code`]).
    pub authorize_url: &'static str,
    pub token_url: &'static str,
    pub user_url: &'static str,
    /// Path segments appended to the base-URL override to rebuild each endpoint.
    /// Defaults match the public-cloud URLs above.
    pub authorize_path: &'static str,
    pub token_path: &'static str,
    pub user_path: &'static str,
    /// Space-separated OAuth scopes requested at authorize time.
    pub scope: &'static str,
    /// Router path of the callback endpoint (also appended to the redirect base).
    pub callback_path: &'static str,
    /// Cookie name holding the CSRF `state` value.
    pub state_cookie: &'static str,
}

/// GitHub provider descriptor.
pub const GITHUB_AUTHORIZE_URL: &str = "https://github.com/login/oauth/authorize";
pub const GITHUB_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
pub const GITHUB_USER_URL: &str = "https://api.github.com/user";

pub const GITHUB: OAuthProvider = OAuthProvider {
    name: "GITHUB",
    authorize_url: GITHUB_AUTHORIZE_URL,
    token_url: GITHUB_TOKEN_URL,
    user_url: GITHUB_USER_URL,
    authorize_path: "/login/oauth/authorize",
    token_path: "/login/oauth/access_token",
    user_path: "/user",
    scope: "read:user",
    callback_path: "/v1/auth/github/callback",
    state_cookie: "gh_oauth_state",
};

/// GitLab provider descriptor (gitlab.com).
pub const GITLAB_AUTHORIZE_URL: &str = "https://gitlab.com/oauth/authorize";
pub const GITLAB_TOKEN_URL: &str = "https://gitlab.com/oauth/token";
pub const GITLAB_USER_URL: &str = "https://gitlab.com/api/v4/user";

pub const GITLAB: OAuthProvider = OAuthProvider {
    name: "GITLAB",
    authorize_url: GITLAB_AUTHORIZE_URL,
    token_url: GITLAB_TOKEN_URL,
    user_url: GITLAB_USER_URL,
    authorize_path: "/oauth/authorize",
    token_path: "/oauth/token",
    // GitLab's API base differs from the web/oauth base; a self-managed instance
    // served from `<base>` exposes the API under `<base>/api/v4`.
    user_path: "/api/v4/user",
    scope: "read_user",
    callback_path: "/v1/auth/gitlab/callback",
    state_cookie: "gl_oauth_state",
};

impl OAuthProvider {
    /// Env var name for the given suffix (e.g. `GITHUB_OAUTH_CLIENT_ID`).
    fn env_key(&self, suffix: &str) -> String {
        format!("{}_OAUTH_{}", self.name, suffix)
    }

    /// Env var name for the base-URL override (e.g. `GITHUB_OAUTH_BASE_URL`).
    fn base_url_env_key(&self) -> String {
        format!("{}_OAUTH_BASE_URL", self.name)
    }
}

/// An OAuth configuration, loaded from the environment. The provider determines the
/// env var prefix, endpoints, callback path, and scope.
#[derive(Clone)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    /// Base URL of this deployment (e.g. `https://cat-herder.example.com`); the
    /// callback URL is derived by appending the provider's callback path.
    pub redirect_url: String,
    /// Provider user logins permitted to become admin (case-insensitive).
    pub allowed_users: Vec<String>,
    /// Which provider this config belongs to.
    pub provider: &'static OAuthProvider,
    /// Optional base-URL override (e.g. a self-managed GitLab or GitHub Enterprise
    /// host). When set, each endpoint is rebuilt as `<base><path>`; when `None`, the
    /// provider's public-cloud URLs are used. Populated from `<NAME>_OAUTH_BASE_URL`.
    pub base_url: Option<String>,
}

impl OAuthConfig {
    /// The authorize endpoint, honouring [`Self::base_url`] if set.
    fn resolved_authorize_url(&self) -> String {
        resolve_url(self.base_url.as_deref(), self.provider.authorize_path, self.provider.authorize_url)
    }

    /// The token endpoint, honouring [`Self::base_url`] if set.
    pub fn token_url(&self) -> String {
        resolve_url(self.base_url.as_deref(), self.provider.token_path, self.provider.token_url)
    }

    /// The user endpoint, honouring [`Self::base_url`] if set.
    pub fn user_url(&self) -> String {
        resolve_url(self.base_url.as_deref(), self.provider.user_path, self.provider.user_url)
    }

    /// Load GitHub config from `GITHUB_OAUTH_*`.
    pub fn from_env() -> Option<Self> {
        Self::from_env_for(&GITHUB)
    }

    /// Load GitLab config from `GITLAB_OAUTH_*`.
    pub fn from_env_gitlab() -> Option<Self> {
        Self::from_env_for(&GITLAB)
    }

    /// Load config for `provider` from the environment.
    pub fn from_env_for(provider: &'static OAuthProvider) -> Option<Self> {
        Self::from_lookup_for(provider, |key| std::env::var(key).ok())
    }

    /// Build GitHub config from an arbitrary key→value lookup (lets tests avoid
    /// touching the process-global environment).
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        Self::from_lookup_for(&GITHUB, lookup)
    }

    /// Build config for `provider` from an arbitrary key→value lookup.
    ///
    /// Reads `<PROVIDER>_OAUTH_CLIENT_ID`, `..._CLIENT_SECRET`, `..._REDIRECT_URL`,
    /// and `..._ALLOWED_USERS` (comma-separated). Returns `None` when any of the
    /// required distinct values are missing/empty, in which case that SSO provider is
    /// simply unavailable and the other login methods still work.
    pub fn from_lookup_for(
        provider: &'static OAuthProvider,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Option<Self> {
        let required = |key: &str| {
            lookup(key)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let allowed_users =
            parse_allowed_users(lookup(&provider.env_key("ALLOWED_USERS")).as_deref());
        let base_url = lookup(&provider.base_url_env_key())
            .map(|v| v.trim().trim_end_matches('/').to_string())
            .filter(|v| !v.is_empty());
        Some(Self {
            client_id: required(&provider.env_key("CLIENT_ID"))?,
            client_secret: required(&provider.env_key("CLIENT_SECRET"))?,
            redirect_url: required(&provider.env_key("REDIRECT_URL"))?,
            allowed_users,
            provider,
            base_url,
        })
    }

    /// The absolute callback URL to register with the provider and send in the token
    /// exchange (providers require them to match).
    pub fn callback_url(&self) -> String {
        format!(
            "{}{}",
            self.redirect_url.trim_end_matches('/'),
            self.provider.callback_path
        )
    }

    /// Whether the given provider login may sign in. Matching is case-insensitive
    /// (provider logins are case-insensitive).
    pub fn is_allowed(&self, login: &str) -> bool {
        let login = login.trim();
        !login.is_empty() && self.allowed_users.iter().any(|u| u.eq_ignore_ascii_case(login))
    }

    /// The URL to redirect the browser to in order to start the flow.
    pub fn authorize_url(&self) -> String {
        format!(
            "{}?client_id={}&redirect_uri={}&scope={}",
            self.resolved_authorize_url(),
            urlencode(&self.client_id),
            urlencode(&self.callback_url()),
            self.provider.scope,
        )
    }
}

/// Resolve an endpoint: `<base><path>` when a base URL override is configured,
/// otherwise the provider's default absolute URL.
fn resolve_url(base: Option<&str>, path: &str, default_url: &str) -> String {
    match base {
        Some(base) => format!("{}{}", base.trim_end_matches('/'), path),
        None => default_url.to_string(),
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

/// An authenticated provider user (subset of the user payload we care about).
#[derive(Debug, Clone, PartialEq)]
pub struct OAuthUser {
    /// The user's login (GitHub `login` / GitLab `username`).
    pub login: String,
}

/// Errors surfaced by the provider client.
#[derive(Debug)]
pub enum OAuthError {
    /// The provider rejected the token exchange (bad/expired code, misconfigured client).
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

#[derive(serde::Deserialize)]
struct TokenResponse {
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// User payload. GitHub returns `login`; GitLab returns `username` (and no `login`).
#[derive(serde::Deserialize)]
struct UserResponse {
    #[serde(default)]
    login: Option<String>,
    #[serde(default)]
    username: Option<String>,
}

/// Generic OAuth client: performs the token exchange and user fetch over HTTP for
/// whichever provider the wrapped [`OAuthConfig`] describes.
#[derive(Clone)]
pub struct OAuthClient {
    config: OAuthConfig,
    http: reqwest::Client,
}

impl OAuthClient {
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
            ("grant_type", "authorization_code"),
            ("redirect_uri", callback_url.as_str()),
        ];
        let resp = self
            .http
            .post(self.config.token_url())
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
    pub async fn fetch_user(&self, access_token: &str) -> Result<OAuthUser, OAuthError> {
        let resp = self
            .http
            .get(self.config.user_url())
            .header(reqwest::header::ACCEPT, "application/json")
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
        // GitHub: `login`. GitLab: `username`.
        match user.login.or(user.username) {
            Some(login) if !login.is_empty() => Ok(OAuthUser { login }),
            _ => Err(OAuthError::UserFetch("no login/username in user payload".into())),
        }
    }

    /// Run the full flow: exchange the code, fetch the user, enforce the allow-list.
    /// Returns the provider login on success.
    pub async fn authenticate(&self, code: &str) -> Result<String, AuthOutcome> {
        let token = self.exchange_code(code).await.map_err(|_| AuthOutcome::ExchangeFailed)?;
        let user = self.fetch_user(&token).await.map_err(|_| AuthOutcome::UserFetchFailed)?;
        if !self.config.is_allowed(&user.login) {
            return Err(AuthOutcome::NotAllowed);
        }
        Ok(user.login)
    }
}

/// Why a login attempt failed (mapped to HTTP responses by the handler).
#[derive(Debug, PartialEq)]
pub enum AuthOutcome {
    ExchangeFailed,
    UserFetchFailed,
    NotAllowed,
}

impl IdentityProvider for OAuthClient {
    /// Not used by the browser flow; validates a provider *access token* by resolving
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

/// GitHub OAuth client (a thin newtype over the shared [`OAuthClient`]).
#[derive(Clone)]
pub struct GitHubOAuth(OAuthClient);

impl GitHubOAuth {
    pub fn new(config: OAuthConfig) -> Self {
        Self(OAuthClient::new(config))
    }

    pub fn config(&self) -> &OAuthConfig {
        self.0.config()
    }

    pub async fn exchange_code(&self, code: &str) -> Result<String, OAuthError> {
        self.0.exchange_code(code).await
    }

    pub async fn fetch_user(&self, access_token: &str) -> Result<OAuthUser, OAuthError> {
        self.0.fetch_user(access_token).await
    }

    pub async fn authenticate(&self, code: &str) -> Result<String, AuthOutcome> {
        self.0.authenticate(code).await
    }
}

impl IdentityProvider for GitHubOAuth {
    async fn validate(
        &self,
        token: &str,
    ) -> cat_herder_domain::ports::DomainResult<cat_herder_domain::entities::TenantId> {
        self.0.validate(token).await
    }
}

/// GitLab OAuth client (a thin newtype over the shared [`OAuthClient`]).
#[derive(Clone)]
pub struct GitLabOAuth(OAuthClient);

impl GitLabOAuth {
    pub fn new(config: OAuthConfig) -> Self {
        Self(OAuthClient::new(config))
    }

    pub fn config(&self) -> &OAuthConfig {
        self.0.config()
    }

    pub async fn exchange_code(&self, code: &str) -> Result<String, OAuthError> {
        self.0.exchange_code(code).await
    }

    pub async fn fetch_user(&self, access_token: &str) -> Result<OAuthUser, OAuthError> {
        self.0.fetch_user(access_token).await
    }

    pub async fn authenticate(&self, code: &str) -> Result<String, AuthOutcome> {
        self.0.authenticate(code).await
    }
}

impl IdentityProvider for GitLabOAuth {
    async fn validate(
        &self,
        token: &str,
    ) -> cat_herder_domain::ports::DomainResult<cat_herder_domain::entities::TenantId> {
        self.0.validate(token).await
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
            provider: &GITHUB,
            base_url: None,
        }
    }

    fn gl_cfg(allowed: &[&str]) -> OAuthConfig {
        OAuthConfig {
            client_id: "gcid".into(),
            client_secret: "gsecret".into(),
            redirect_url: "https://cat-herder.example.com".into(),
            allowed_users: allowed.iter().map(|s| s.to_string()).collect(),
            provider: &GITLAB,
            base_url: None,
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
    fn base_url_override_rebuilds_endpoints() {
        // Defaults when no override is set.
        let c = cfg(&[]);
        assert_eq!(c.token_url(), GITHUB_TOKEN_URL);
        assert_eq!(c.user_url(), GITHUB_USER_URL);
        assert!(c.authorize_url().starts_with(GITHUB_AUTHORIZE_URL));

        // With GITHUB_OAUTH_BASE_URL set, each endpoint is rebuilt against it.
        let lookup = |k: &str| -> Option<String> {
            match k {
                "GITHUB_OAUTH_CLIENT_ID" => Some("cid".into()),
                "GITHUB_OAUTH_CLIENT_SECRET" => Some("sec".into()),
                "GITHUB_OAUTH_REDIRECT_URL" => Some("https://x.example".into()),
                "GITHUB_OAUTH_BASE_URL" => Some("https://ghe.example.com/".into()),
                _ => None,
            }
        };
        let c = OAuthConfig::from_lookup(lookup).expect("configured");
        assert_eq!(c.base_url.as_deref(), Some("https://ghe.example.com"));
        assert_eq!(c.token_url(), "https://ghe.example.com/login/oauth/access_token");
        assert_eq!(c.user_url(), "https://ghe.example.com/user");
        assert!(c.authorize_url().starts_with("https://ghe.example.com/login/oauth/authorize"));

        // GitLab override keeps the /api/v4 user path.
        let gl = OAuthConfig {
            base_url: Some("https://gitlab.example.com".into()),
            ..gl_cfg(&[])
        };
        assert_eq!(gl.token_url(), "https://gitlab.example.com/oauth/token");
        assert_eq!(gl.user_url(), "https://gitlab.example.com/api/v4/user");
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

    // --- GitLab ------------------------------------------------------------

    #[test]
    fn gitlab_from_lookup_reads_gitlab_env_vars() {
        let lookup = |k: &str| -> Option<String> {
            match k {
                "GITLAB_OAUTH_CLIENT_ID" => Some("gl-client".into()),
                "GITLAB_OAUTH_CLIENT_SECRET" => Some("gl-secret".into()),
                "GITLAB_OAUTH_REDIRECT_URL" => Some("https://cat.example.com".into()),
                "GITLAB_OAUTH_ALLOWED_USERS" => Some("alice, bob".into()),
                _ => None,
            }
        };
        let c = OAuthConfig::from_lookup_for(&GITLAB, lookup).expect("configured");
        assert_eq!(c.client_id, "gl-client");
        assert_eq!(c.client_secret, "gl-secret");
        assert_eq!(c.provider.name, "GITLAB");
        assert_eq!(c.allowed_users, vec!["alice".to_string(), "bob".to_string()]);
    }

    #[test]
    fn gitlab_from_env_helper_uses_gitlab_prefix() {
        // The gitlab-specific entrypoint must look up GITLAB_OAUTH_* keys.
        let lookup = |k: &str| -> Option<String> {
            match k {
                "GITLAB_OAUTH_CLIENT_ID" => Some("gl-client".into()),
                "GITLAB_OAUTH_CLIENT_SECRET" => Some("gl-secret".into()),
                "GITLAB_OAUTH_REDIRECT_URL" => Some("https://cat.example.com".into()),
                _ => None,
            }
        };
        let c = OAuthConfig::from_lookup_for(&GITLAB, lookup).expect("configured");
        // ALLOWED_USERS omitted => empty list (denies everyone) but still configured.
        assert!(c.allowed_users.is_empty());
        assert!(!c.is_allowed("alice"));
    }

    #[test]
    fn gitlab_from_lookup_requires_all_secrets() {
        // Missing client secret => GitLab SSO disabled.
        assert!(OAuthConfig::from_lookup_for(&GITLAB, |k| {
            (k != "GITLAB_OAUTH_CLIENT_SECRET").then(|| "x".into())
        })
        .is_none());
        // Missing client id => disabled.
        assert!(OAuthConfig::from_lookup_for(&GITLAB, |k| {
            (k != "GITLAB_OAUTH_CLIENT_ID").then(|| "x".into())
        })
        .is_none());
        // Missing redirect URL => disabled.
        assert!(OAuthConfig::from_lookup_for(&GITLAB, |k| {
            (k != "GITLAB_OAUTH_REDIRECT_URL").then(|| "x".into())
        })
        .is_none());
        // Nothing configured => disabled.
        assert!(OAuthConfig::from_lookup_for(&GITLAB, |_| None).is_none());
    }

    #[test]
    fn gitlab_ignores_github_env_vars() {
        // Only GitHub vars set => GitLab stays disabled (distinct prefixes).
        let lookup = |k: &str| -> Option<String> {
            match k {
                "GITHUB_OAUTH_CLIENT_ID"
                | "GITHUB_OAUTH_CLIENT_SECRET"
                | "GITHUB_OAUTH_REDIRECT_URL" => Some("x".into()),
                _ => None,
            }
        };
        assert!(OAuthConfig::from_lookup_for(&GITLAB, lookup).is_none());
    }

    #[test]
    fn gitlab_is_allowed_case_insensitive() {
        let c = gl_cfg(&["Alice"]);
        assert!(c.is_allowed("alice"));
        assert!(c.is_allowed("  ALICE  "));
        assert!(!c.is_allowed("bob"));
        assert!(!c.is_allowed(""));
        assert!(!gl_cfg(&[]).is_allowed("alice"));
    }

    #[test]
    fn gitlab_callback_url_and_authorize_url() {
        let mut c = gl_cfg(&[]);
        assert_eq!(
            c.callback_url(),
            "https://cat-herder.example.com/v1/auth/gitlab/callback"
        );
        c.redirect_url = "https://cat-herder.example.com/".into();
        assert_eq!(
            c.callback_url(),
            "https://cat-herder.example.com/v1/auth/gitlab/callback"
        );

        let url = c.authorize_url();
        assert!(url.starts_with(GITLAB_AUTHORIZE_URL));
        assert!(url.contains("client_id=gcid"));
        assert!(url.contains("redirect_uri=https%3A%2F%2Fcat-herder.example.com%2Fv1%2Fauth%2Fgitlab%2Fcallback"));
        assert!(url.contains("scope=read_user"));
    }
}

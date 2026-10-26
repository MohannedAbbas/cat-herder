//! GitHub App installation-token minting (`PLAN.md` §11.3).
//!
//! A GitHub App installation token is a short-lived (~1h), App-scoped bearer
//! credential minted by signing an RS256 JWT with the App's private key and POSTing to
//! the installation token endpoint. A single App covers **every repo it is installed
//! on** — the broker resolves the installation for a target repo dynamically (via the
//! App's `/repos/{owner}/{repo}/installation` endpoint) rather than holding per-repo or
//! per-account config, then mints an installation token.
//!
//! The private key is kept out of the sandbox and out of the domain: this adapter
//! lives in the broker/control-plane trust boundary.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

/// Configuration for a GitHub App.
#[derive(Debug, Clone)]
pub struct GitHubAppConfig {
    /// The GitHub App's numeric id (`iss` in the JWT).
    pub app_id: u64,
    /// The App's RSA private key, PEM-encoded (PKCS#1 or PKCS#8).
    pub private_key_pem: String,
    /// API base URL (defaults to `https://api.github.com`; overridable for GHE/tests).
    pub api_base: String,
}

impl Default for GitHubAppConfig {
    fn default() -> Self {
        Self {
            app_id: 0,
            private_key_pem: String::new(),
            api_base: "https://api.github.com".into(),
        }
    }
}

/// Mints GitHub App installation tokens.
#[derive(Clone)]
pub struct GitHubApp {
    http: reqwest::Client,
    config: GitHubAppConfig,
}

/// The JWT lifetime window (GitHub requires < 10 min expiry).
const JWT_TTL_SECS: u64 = 600;
/// Clock-skew allowance subtracted from `iat`.
const JWT_SKEW_SECS: u64 = 60;

#[derive(Deserialize)]
struct TokenResponse {
    token: String,
    expires_at: String,
}

#[derive(Deserialize)]
struct InstallationResponse {
    id: u64,
}

impl GitHubApp {
    /// Build a client, validating the private key up front so a bad key fails at
    /// construction rather than at first mint.
    pub fn new(config: GitHubAppConfig) -> Result<Self, String> {
        // Exercise the key once: a malformed/unloadable key errors here.
        let _ = Self::encode_jwt(&config, SystemTime::now())?;
        Ok(Self {
            http: reqwest::Client::new(),
            config,
        })
    }

    /// Mint a short-lived installation token for the repo `owner/repo`, resolving the
    /// App's installation for that repo first. Returns `(token, expires_at)`.
    pub async fn mint_repo_token(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<(String, SystemTime), String> {
        let installation_id = self.resolve_installation_id(owner, repo).await?;
        self.mint_installation_token(installation_id).await
    }

    /// Resolve the App's installation id for a repo (`GET /repos/{owner}/{repo}/installation`).
    async fn resolve_installation_id(&self, owner: &str, repo: &str) -> Result<u64, String> {
        let jwt = Self::encode_jwt(&self.config, SystemTime::now())?;
        let base = self.config.api_base.trim_end_matches('/');
        let url = format!("{base}/repos/{owner}/{repo}/installation");

        let resp = self
            .http
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header(reqwest::header::USER_AGENT, "cat-herder-broker")
            .bearer_auth(&jwt)
            .send()
            .await
            .map_err(|e| format!("installation lookup failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("installation lookup returned {status}: {body}"));
        }

        let ir: InstallationResponse = resp
            .json()
            .await
            .map_err(|e| format!("bad installation response: {e}"))?;
        Ok(ir.id)
    }

    /// Mint an installation token for a known installation id.
    async fn mint_installation_token(
        &self,
        installation_id: u64,
    ) -> Result<(String, SystemTime), String> {
        let jwt = Self::encode_jwt(&self.config, SystemTime::now())?;
        let base = self.config.api_base.trim_end_matches('/');
        let url = format!("{base}/app/installations/{installation_id}/access_tokens");

        let resp = self
            .http
            .post(&url)
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header(reqwest::header::USER_AGENT, "cat-herder-broker")
            .bearer_auth(&jwt)
            .send()
            .await
            .map_err(|e| format!("mint request failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("GitHub App mint returned {status}: {body}"));
        }

        let tr: TokenResponse = resp
            .json()
            .await
            .map_err(|e| format!("bad response: {e}"))?;
        let expires = chrono::DateTime::parse_from_rfc3339(&tr.expires_at)
            .map(|d| d.into())
            .unwrap_or_else(|_| SystemTime::now() + Duration::from_secs(3600));
        Ok((tr.token, expires))
    }

    /// Sign an RS256 JWT asserting the App identity, with a short expiry.
    fn encode_jwt(config: &GitHubAppConfig, now: SystemTime) -> Result<String, String> {
        use jsonwebtoken::{Algorithm, EncodingKey, Header};

        let key = EncodingKey::from_rsa_pem(config.private_key_pem.as_bytes())
            .map_err(|e| format!("invalid GitHub App private key: {e}"))?;
        let iat = now
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .saturating_sub(JWT_SKEW_SECS);
        let claims = serde_json::json!({
            "iat": iat,
            "exp": iat + JWT_TTL_SECS,
            "iss": config.app_id.to_string(),
        });
        jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &key)
            .map_err(|e| format!("JWT signing failed: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    /// A throwaway 2048-bit RSA private key (PKCS#8 PEM), used only by tests.
    fn test_key() -> String {
        include_str!("test_key.pem").to_string()
    }

    fn config(api_base: String) -> GitHubAppConfig {
        GitHubAppConfig {
            app_id: 456,
            private_key_pem: test_key(),
            api_base,
        }
    }

    fn decode_segment(seg: &str) -> serde_json::Value {
        let bytes = URL_SAFE_NO_PAD.decode(seg).expect("valid base64url");
        serde_json::from_slice(&bytes).expect("valid json")
    }

    #[test]
    fn jwt_is_rs256_with_expected_claims() {
        let cfg = config("https://api.github.com".into());
        let jwt = GitHubApp::encode_jwt(&cfg, SystemTime::now()).expect("sign ok");

        let segments: Vec<&str> = jwt.split('.').collect();
        assert_eq!(segments.len(), 3, "JWT has header.payload.signature");

        let header = decode_segment(segments[0]);
        assert_eq!(header["alg"], "RS256");
        assert_eq!(header["typ"], "JWT");

        let payload = decode_segment(segments[1]);
        assert_eq!(payload["iss"], "456");
        let exp = payload["exp"].as_u64().expect("exp is a number");
        let iat = payload["iat"].as_u64().expect("iat is a number");
        assert!(exp > iat, "exp is after iat");
        assert!(
            exp - iat <= JWT_TTL_SECS,
            "expiry within the allowed window"
        );
    }

    #[test]
    fn new_rejects_malformed_key() {
        let result = GitHubApp::new(GitHubAppConfig {
            app_id: 456,
            private_key_pem: "-----BEGIN PRIVATE KEY-----\nnot-a-key\n-----END PRIVATE KEY-----"
                .into(),
            ..Default::default()
        });
        match result {
            Err(e) => assert!(e.contains("invalid GitHub App private key"), "{e}"),
            Ok(_) => panic!("malformed key should be rejected"),
        }
    }

    #[tokio::test]
    async fn mint_resolves_installation_then_mints() {
        use wiremock::matchers::{header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // Installation resolution: the App asks which installation covers org/repo.
        Mock::given(method("GET"))
            .and(path("/repos/org/repo/installation"))
            .and(header("accept", "application/vnd.github+json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 123,
            })))
            .mount(&server)
            .await;

        // Mint for that installation.
        Mock::given(method("POST"))
            .and(path("/app/installations/123/access_tokens"))
            .and(header("accept", "application/vnd.github+json"))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "token": "ghs_install_token",
                "expires_at": "2030-01-01T00:00:00Z",
            })))
            .mount(&server)
            .await;

        let app = GitHubApp::new(config(server.uri())).expect("valid key");
        let (token, expires) = app.mint_repo_token("org", "repo").await.expect("mint ok");
        assert_eq!(token, "ghs_install_token");
        assert!(expires > SystemTime::now());
    }

    #[tokio::test]
    async fn mint_errors_when_installation_lookup_fails() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not installed"))
            .mount(&server)
            .await;

        let app = GitHubApp::new(config(server.uri())).expect("valid key");
        let err = app.mint_repo_token("org", "repo").await.unwrap_err();
        assert!(err.contains("404"), "error carries status: {err}");
    }

    #[tokio::test]
    async fn mint_errors_on_mint_failure() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": 123,
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad credentials"))
            .mount(&server)
            .await;

        let app = GitHubApp::new(config(server.uri())).expect("valid key");
        let err = app.mint_repo_token("org", "repo").await.unwrap_err();
        assert!(err.contains("401"), "error carries status: {err}");
    }
}

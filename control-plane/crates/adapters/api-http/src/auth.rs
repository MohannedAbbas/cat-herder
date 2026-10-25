//! Local admin authentication: a single seeded admin user with a bcrypt-hashed
//! password, and in-memory session tokens delivered via an HttpOnly cookie.
//!
//! This is a pragmatic first pass (matches PLAN.md §3's SSO goal, swappable later):
//! credentials come from a Kubernetes Secret, sessions live in memory (a restart
//! logs admins out), and every admin endpoint is guarded by [`Auth::valid_session`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

const DEFAULT_SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Session lifetime, overridable via `CAT_HERDER_SESSION_TTL_SECS` (e.g. `3600`).
/// Falls back to the 24h default when unset, blank, or unparsable.
fn session_ttl_from_env() -> Duration {
    std::env::var("CAT_HERDER_SESSION_TTL_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&secs| secs > 0)
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_SESSION_TTL)
}

#[derive(Clone)]
pub struct Auth {
    admin_username: String,
    admin_password_hash: String,
    sessions: Arc<RwLock<HashMap<String, Instant>>>,
    /// How long an issued session token stays valid.
    session_ttl: Duration,
}

impl Auth {
    /// Build from the seeded admin username + plaintext password (bcrypt-hashed here).
    /// The session TTL is read from `CAT_HERDER_SESSION_TTL_SECS` (default 24h).
    pub fn new(username: impl Into<String>, password: &str) -> Self {
        Self::with_session_ttl(username, password, session_ttl_from_env())
    }

    /// Build with an explicit session TTL (lets tests avoid the process environment).
    pub fn with_session_ttl(
        username: impl Into<String>,
        password: &str,
        session_ttl: Duration,
    ) -> Self {
        let hash = bcrypt::hash(password, bcrypt::DEFAULT_COST).unwrap_or_default();
        Self {
            admin_username: username.into(),
            admin_password_hash: hash,
            sessions: Arc::new(RwLock::new(HashMap::new())),
            session_ttl,
        }
    }

    pub fn admin_username(&self) -> &str {
        &self.admin_username
    }

    /// Verify credentials and, on success, create a session. Returns the token.
    pub async fn login(&self, username: &str, password: &str) -> Option<String> {
        let ok = username == self.admin_username
            && bcrypt::verify(password, &self.admin_password_hash).unwrap_or(false);
        if !ok {
            return None;
        }
        Some(self.create_session().await)
    }

    /// Issue a session without a password check. Used by SSO providers (e.g. GitHub
    /// OAuth) *after* they have independently authenticated the identity against an
    /// allow-list. Reuses the same in-memory session store as password login.
    pub async fn create_session(&self) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        self.sessions
            .write()
            .await
            .insert(token.clone(), Instant::now() + self.session_ttl);
        token
    }

    pub async fn valid_session(&self, token: &str) -> bool {
        match self.sessions.read().await.get(token) {
            Some(expiry) => *expiry > Instant::now(),
            None => false,
        }
    }

    pub async fn logout(&self, token: &str) {
        self.sessions.write().await.remove(token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn password_login_issues_valid_session() {
        let auth = Auth::new("admin", "hunter2");
        assert!(auth.login("admin", "wrong").await.is_none());
        let token = auth.login("admin", "hunter2").await.expect("login ok");
        assert!(auth.valid_session(&token).await);
        auth.logout(&token).await;
        assert!(!auth.valid_session(&token).await);
    }

    #[tokio::test]
    async fn create_session_issues_valid_session_without_password() {
        // Simulates the SSO path: a session minted after external authentication.
        let auth = Auth::new("admin", "hunter2");
        let token = auth.create_session().await;
        assert!(auth.valid_session(&token).await);
    }

    #[tokio::test]
    async fn create_session_tokens_are_unique() {
        let auth = Auth::new("admin", "hunter2");
        let a = auth.create_session().await;
        let b = auth.create_session().await;
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn unknown_token_is_invalid() {
        let auth = Auth::new("admin", "hunter2");
        assert!(!auth.valid_session("nope").await);
    }
}

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

const SESSION_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone)]
pub struct Auth {
    admin_username: String,
    admin_password_hash: String,
    sessions: Arc<RwLock<HashMap<String, Instant>>>,
}

impl Auth {
    /// Build from the seeded admin username + plaintext password (bcrypt-hashed here).
    pub fn new(username: impl Into<String>, password: &str) -> Self {
        let hash = bcrypt::hash(password, bcrypt::DEFAULT_COST).unwrap_or_default();
        Self {
            admin_username: username.into(),
            admin_password_hash: hash,
            sessions: Arc::new(RwLock::new(HashMap::new())),
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
        let token = uuid::Uuid::new_v4().to_string();
        self.sessions
            .write()
            .await
            .insert(token.clone(), Instant::now() + SESSION_TTL);
        Some(token)
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

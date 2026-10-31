//! Conversion between the domain [`CredentialGrant`] and the wire [`Grant`] message.
//!
//! `SystemTime` is encoded as Unix epoch seconds on the wire; `revoked_at: None` maps
//! to `revoked_at == 0`. The secret is deliberately absent from the wire grant — only
//! metadata is transmitted as part of the grant record.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cat_herder_domain::entities::{CredentialGrant, CredentialGrantStatus};

use crate::proto::v1::Grant;

pub(crate) fn to_proto_grant(g: &CredentialGrant) -> Grant {
    Grant {
        id: g.id.clone(),
        run_id: g.run_id.0,
        target: g.target.clone(),
        action: g.action.clone(),
        scope: g.scope.clone(),
        status: status_to_str(g.status).to_string(),
        expires_at: to_secs(g.expires_at),
        created_at: to_secs(g.created_at),
        revoked_at: g.revoked_at.map(to_secs).unwrap_or(0),
        token_id: g.token_id.clone().unwrap_or_default(),
    }
}

pub(crate) fn from_proto_grant(g: &Grant) -> CredentialGrant {
    CredentialGrant {
        id: g.id.clone(),
        run_id: cat_herder_domain::entities::RunId(g.run_id),
        target: g.target.clone(),
        action: g.action.clone(),
        scope: g.scope.clone(),
        status: status_from_str(&g.status),
        expires_at: from_secs(g.expires_at),
        created_at: from_secs(g.created_at),
        revoked_at: (g.revoked_at != 0).then(|| from_secs(g.revoked_at)),
        token_id: (!g.token_id.is_empty()).then(|| g.token_id.clone()),
    }
}

fn status_to_str(s: CredentialGrantStatus) -> &'static str {
    match s {
        CredentialGrantStatus::Active => "active",
        CredentialGrantStatus::Revoked => "revoked",
        CredentialGrantStatus::Expired => "expired",
    }
}

fn status_from_str(s: &str) -> CredentialGrantStatus {
    match s {
        "revoked" => CredentialGrantStatus::Revoked,
        "expired" => CredentialGrantStatus::Expired,
        _ => CredentialGrantStatus::Active,
    }
}

pub(crate) fn to_secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn from_secs(s: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_round_trips() {
        let created = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let expires = created + Duration::from_secs(3600);
        let revoked = created + Duration::from_secs(60);
        let grant = CredentialGrant {
            id: "cred-123".into(),
            run_id: cat_herder_domain::entities::RunId(7),
            target: "github.com/org/repo".into(),
            action: "clone".into(),
            scope: "read".into(),
            status: CredentialGrantStatus::Revoked,
            expires_at: expires,
            created_at: created,
            revoked_at: Some(revoked),
            token_id: Some("tok-9".into()),
        };

        let proto = to_proto_grant(&grant);
        assert_eq!(proto.status, "revoked");
        assert_eq!(proto.revoked_at, to_secs(revoked));
        assert_eq!(proto.token_id, "tok-9");

        let back = from_proto_grant(&proto);
        assert_eq!(back.id, "cred-123");
        assert_eq!(back.run_id, cat_herder_domain::entities::RunId(7));
        assert_eq!(back.target, "github.com/org/repo");
        assert_eq!(back.status, CredentialGrantStatus::Revoked);
        assert_eq!(back.expires_at, expires);
        assert_eq!(back.created_at, created);
        assert_eq!(back.revoked_at, Some(revoked));
        assert_eq!(back.token_id.as_deref(), Some("tok-9"));
    }

    #[test]
    fn unset_optional_fields_map_to_none() {
        let proto = Grant {
            id: "cred-x".into(),
            run_id: 1,
            target: "github.com/a/b".into(),
            action: "push".into(),
            scope: "write".into(),
            status: "active".into(),
            expires_at: 0,
            created_at: 0,
            revoked_at: 0,
            token_id: String::new(),
        };
        let grant = from_proto_grant(&proto);
        assert_eq!(grant.status, CredentialGrantStatus::Active);
        assert!(grant.revoked_at.is_none());
        assert!(grant.token_id.is_none());
    }

    #[test]
    fn unknown_status_defaults_to_active() {
        assert_eq!(status_from_str("bogus"), CredentialGrantStatus::Active);
    }
}

//! Authorization model (`PLAN.md` §11.8): a small declarative policy DSL over the
//! domain entities, with RBAC for coarse roles and attribute-based rules for fine
//! grained policy. OPA/Cedar is the Phase-3 upgrade — v1 is this embedded engine.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// RBAC roles, ordered by privilege (`Viewer < Operator < Admin`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Viewer,
    Operator,
    Admin,
}

/// The five authorization decision points (§11.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// May this actor start a run?
    RunCreate,
    /// May this run call this tool?
    ToolInvoke,
    /// May this run request this credential target/scope?
    CredentialMint,
    /// Who may approve a pending approval?
    Approval,
    /// Who may mark a result reviewed/approved?
    ResultReview,
}

/// An attribute-based rule: when a request for `decision` carries `attribute == value`,
/// the actor's role must be at least `requires`. Rules are additive — the strictest
/// matching rule wins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AbacRule {
    pub decision: Decision,
    pub attribute: String,
    pub value: String,
    pub requires: Role,
}

/// A declarative policy: RBAC minimum roles per decision plus ABAC rules. Serialized
/// as JSON (the wire/editor form) and stored in settings; a built-in [`Policy::default`]
/// applies when no policy is configured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Policy {
    pub min_roles: BTreeMap<Decision, Role>,
    pub rules: Vec<AbacRule>,
}

impl Default for Policy {
    fn default() -> Self {
        use Decision::*;
        Self {
            // Sensible v1 defaults: mutating decisions require an operator.
            min_roles: BTreeMap::from([
                (RunCreate, Role::Operator),
                (ToolInvoke, Role::Operator),
                (CredentialMint, Role::Operator),
                (Approval, Role::Operator),
                (ResultReview, Role::Operator),
            ]),
            rules: Vec::new(),
        }
    }
}

/// An authorization request: the decision, the actor's role, and the resource
/// attributes the ABAC rules match against.
#[derive(Debug, Clone)]
pub struct AccessRequest {
    pub decision: Decision,
    pub role: Role,
    pub attributes: BTreeMap<String, String>,
}

/// Decide whether `request` is allowed under `policy`. A decision with no explicit
/// minimum role denies by default (requires `Admin`).
pub fn authorize(policy: &Policy, request: &AccessRequest) -> bool {
    let mut required = policy
        .min_roles
        .get(&request.decision)
        .copied()
        .unwrap_or(Role::Admin);
    for rule in &policy.rules {
        if rule.decision == request.decision
            && request.attributes.get(&rule.attribute).map(String::as_str)
                == Some(rule.value.as_str())
        {
            required = required.max(rule.requires);
        }
    }
    request.role >= required
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(decision: Decision, role: Role) -> AccessRequest {
        AccessRequest {
            decision,
            role,
            attributes: BTreeMap::new(),
        }
    }

    #[test]
    fn rbac_default_min_roles() {
        let policy = Policy::default();
        // Viewers can't create runs; operators and admins can.
        assert!(!authorize(&policy, &req(Decision::RunCreate, Role::Viewer)));
        assert!(authorize(
            &policy,
            &req(Decision::RunCreate, Role::Operator)
        ));
        assert!(authorize(&policy, &req(Decision::RunCreate, Role::Admin)));
        // Same for the other mutating decisions.
        assert!(!authorize(&policy, &req(Decision::Approval, Role::Viewer)));
        assert!(authorize(&policy, &req(Decision::Approval, Role::Operator)));
    }

    #[test]
    fn abac_rule_raises_required_role() {
        // "write-scoped credentials require admin"
        let policy = Policy {
            min_roles: Policy::default().min_roles,
            rules: vec![AbacRule {
                decision: Decision::CredentialMint,
                attribute: "scope".into(),
                value: "write".into(),
                requires: Role::Admin,
            }],
        };

        let read = AccessRequest {
            decision: Decision::CredentialMint,
            role: Role::Operator,
            attributes: BTreeMap::from([("scope".into(), "read".into())]),
        };
        assert!(authorize(&policy, &read), "read is operator-allowed");

        let write_operator = AccessRequest {
            decision: Decision::CredentialMint,
            role: Role::Operator,
            attributes: BTreeMap::from([("scope".into(), "write".into())]),
        };
        assert!(!authorize(&policy, &write_operator), "write needs admin");

        let write_admin = AccessRequest {
            decision: Decision::CredentialMint,
            role: Role::Admin,
            attributes: BTreeMap::from([("scope".into(), "write".into())]),
        };
        assert!(authorize(&policy, &write_admin));
    }

    #[test]
    fn abac_rule_does_not_match_other_decisions() {
        let policy = Policy {
            min_roles: Policy::default().min_roles,
            rules: vec![AbacRule {
                decision: Decision::CredentialMint,
                attribute: "scope".into(),
                value: "write".into(),
                requires: Role::Admin,
            }],
        };
        // The same attribute on a *different* decision doesn't trigger the rule.
        let other = AccessRequest {
            decision: Decision::ToolInvoke,
            role: Role::Operator,
            attributes: BTreeMap::from([("scope".into(), "write".into())]),
        };
        assert!(authorize(&policy, &other));
    }

    #[test]
    fn missing_min_role_denies_by_default() {
        let policy = Policy {
            min_roles: BTreeMap::new(), // nothing configured
            rules: Vec::new(),
        };
        assert!(!authorize(
            &policy,
            &req(Decision::RunCreate, Role::Operator)
        ));
        assert!(authorize(&policy, &req(Decision::RunCreate, Role::Admin)));
    }

    #[test]
    fn policy_serializes_round_trip() {
        let policy = Policy {
            min_roles: Policy::default().min_roles,
            rules: vec![AbacRule {
                decision: Decision::Approval,
                attribute: "repo".into(),
                value: "prod/*".into(),
                requires: Role::Admin,
            }],
        };
        let json = serde_json::to_string(&policy).expect("serialize");
        let back: Policy = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, policy);
    }
}

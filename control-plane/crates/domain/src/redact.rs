//! Secret redaction (`PLAN.md` §16): a defense-in-depth layer that scrubs credentials,
//! tokens, and API keys from events, traces, and logs before write — the "secret never
//! reaches the model" rule (§11.3) extended to every write path. Even if a secret
//! accidentally leaks into an event payload, it is scrubbed here.

use serde_json::Value;

/// Known credential prefixes. A token starting with one of these (and long enough to
/// be a real credential) is scrubbed wholesale.
const SECRET_PREFIXES: &[&str] = &[
    "ghp_", "gho_", "ghu_", "ghs_", "ghr_", // GitHub PATs
    "sk-", "sk_",                          // OpenAI / generic
    "AKIA", "ASIA",                        // AWS access keys
    "xoxb-", "xoxp-", "xoxa-", "xoxr-",    // Slack
    "glpat-",                              // GitLab PAT
];

/// Minimum run of credential characters after a prefix before it is treated as a real
/// secret (avoids scrubbing prose that merely mentions a prefix).
const MIN_SECRET_LEN: usize = 16;

const REDACTED: &str = "[REDACTED]";

/// Whether `b` is a credential character (the body of a token).
fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Scrub a single credential prefix from `s`, replacing `prefix + <body>` with
/// `[REDACTED]` when the body is long enough to be a real credential.
fn redact_prefix(s: &str, prefix: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        match rest.find(prefix) {
            Some(idx) => {
                out.push_str(&rest[..idx]);
                let after = &rest[idx + prefix.len()..];
                let run = after.bytes().take_while(|b| is_token_char(*b)).count();
                if run >= MIN_SECRET_LEN {
                    out.push_str(REDACTED);
                    rest = &after[run..];
                } else {
                    // Too short to be a credential — keep the prefix and move on.
                    out.push_str(prefix);
                    rest = after;
                }
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    out
}

/// Scrub an `Authorization: Bearer <token>` / `Bearer <token>` sequence.
fn redact_bearer(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        match rest.find("Bearer ") {
            Some(idx) => {
                out.push_str(&rest[..idx + "Bearer ".len()]);
                let after = &rest[idx + "Bearer ".len()..];
                let run = after.bytes().take_while(|b| is_token_char(*b) || *b == b'.').count();
                if run > 0 {
                    out.push_str(REDACTED);
                    rest = &after[run..];
                } else {
                    rest = after;
                }
            }
            None => {
                out.push_str(rest);
                break;
            }
        }
    }
    out
}

/// Scrub credential material from an arbitrary string (log line, event field, …).
pub fn redact(input: &str) -> String {
    let mut out = input.to_string();
    for prefix in SECRET_PREFIXES {
        out = redact_prefix(&out, prefix);
    }
    out = redact_bearer(&out);
    out
}

/// Recursively scrub every string in a JSON value (the event payload shape).
pub fn redact_value(value: &mut Value) {
    match value {
        Value::String(s) => {
            let redacted = redact(s);
            if redacted != *s {
                *s = redacted;
            }
        }
        Value::Array(items) => {
            for item in items {
                redact_value(item);
            }
        }
        Value::Object(map) => {
            for value in map.values_mut() {
                redact_value(value);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_known_credential_prefixes() {
        assert_eq!(redact("token is ghp_abcdefghijklmnopqrstuvwxyz0123456789"), "token is [REDACTED]");
        assert_eq!(redact("key: sk-abcdefghijklmnopqrstuvwxyz"), "key: [REDACTED]");
        assert_eq!(redact("aws AKIAIOSFODNN7EXAMPLE"), "aws [REDACTED]");
        assert_eq!(redact("slack xoxb-0123456789-0123456789-abcdefgh"), "slack [REDACTED]");
        assert_eq!(redact("gitlab glpat-abcdefghijklmnopqrstuvwxyz"), "gitlab [REDACTED]");
    }

    #[test]
    fn leaves_short_or_unrelated_text_alone() {
        // A prefix without a credential body, or unrelated prose, is untouched.
        assert_eq!(redact("the ghp_ prefix"), "the ghp_ prefix");
        assert_eq!(redact("hello world"), "hello world");
        assert_eq!(redact("sk-"), "sk-");
    }

    #[test]
    fn scrubs_bearer_tokens() {
        assert_eq!(
            redact("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.secret"),
            "Authorization: Bearer [REDACTED]"
        );
    }

    #[test]
    fn scrubs_secrets_nested_in_json() {
        let mut v: Value = serde_json::json!({
            "report": "used token ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "args": ["sk-abcdefghijklmnopqrstuvwxyz", "plain"],
        });
        redact_value(&mut v);
        // The secret substrings are gone; unrelated strings remain.
        let report = v["report"].as_str().unwrap();
        assert!(report.contains("[REDACTED]"));
        assert!(!report.contains("ghp_abcdefghijklmnopqrstuvwxyz0123456789"));
        assert!(v["args"][0].as_str().unwrap().contains("[REDACTED]"));
        assert_eq!(v["args"][1], "plain");
    }
}

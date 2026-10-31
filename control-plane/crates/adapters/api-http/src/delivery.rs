//! Terminal result delivery (generic reply channel).
//!
//! When a run is created with a `callback_url`, the control plane POSTs the run's
//! terminal result back to that URL once it reaches a terminal state, so a triggerer
//! does not have to poll the UI. Delivery is best-effort: any error is logged and
//! never affects the run's lifecycle.
//!
//! The POST body is signed with HMAC-SHA256 using the shared webhook secret
//! ([`super::WEBHOOK_SECRET_KEY`]) in the `X-Cat-Herder-Signature: sha256=<hex>` header
//! so the receiver can authenticate it. The agent report is redacted with the domain
//! redactor before being sent.

use cat_herder_domain::entities::{Run, RunState};
use cat_herder_domain::ports::SettingRepository;
use cat_herder_domain::redact::redact;
use ring::hmac;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::AppState;

/// Header carrying the HMAC-SHA256 signature of the body.
pub const SIGNATURE_HEADER: &str = "X-Cat-Herder-Signature";

/// Env var holding an optional comma-separated hostname allowlist.
pub const ALLOWLIST_ENV: &str = "CAT_HERDER_CALLBACK_ALLOWLIST";

/// Deliver a run's terminal result to its `callback_url` (no-op if unset).
///
/// Best-effort: every failure path is logged and swallowed so the caller's run
/// lifecycle is never affected.
pub async fn deliver_result(state: &AppState, run: &Run) {
    let Some(url) = run.callback_url.as_deref() else {
        return;
    };
    if let Err(e) = try_deliver(state, run, url).await {
        eprintln!("callback delivery failed for run {}: {e}", run.id.0);
    }
}

async fn try_deliver(state: &AppState, run: &Run, url: &str) -> Result<(), String> {
    validate_callback_url(url).await?;

    let body = build_body(run);
    let payload = serde_json::to_vec(&body).map_err(|e| e.to_string())?;

    let secret = state
        .settings
        .get(super::WEBHOOK_SECRET_KEY)
        .await
        .map_err(|e| e.to_string())?;

    let mut req = reqwest::Client::new()
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, "cat-herder")
        .body(payload.clone());

    if let Some(secret) = secret.filter(|s| !s.is_empty()) {
        let sig = sign_body(secret.as_bytes(), &payload);
        req = req.header(SIGNATURE_HEADER, format!("sha256={sig}"));
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("callback returned HTTP {}", resp.status()));
    }
    Ok(())
}

/// The JSON body POSTed to the callback URL.
fn build_body(run: &Run) -> serde_json::Value {
    serde_json::json!({
        "run_id": run.id.0,
        "state": state_str(run.state),
        "trigger": run.trigger,
        "report": run.report.as_deref().map(redact),
        "pr_url": run.pr_url,
        "finished_at": run.finished_at.map(to_iso),
    })
}

/// The lowercase snake_case string form of a run state (matching the wire form).
fn state_str(state: RunState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| format!("{state:?}").to_lowercase())
}

/// Format a `SystemTime` as an ISO-8601 UTC string.
fn to_iso(t: std::time::SystemTime) -> String {
    let ms = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    super::millis_to_iso(ms)
}

/// Compute the hex HMAC-SHA256 of `body` keyed by `secret`.
pub fn sign_body(secret: &[u8], body: &[u8]) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    let tag = hmac::sign(&key, body);
    hex(tag.as_ref())
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// SSRF guard: validate a callback URL before delivery.
///
/// Rules:
/// * scheme must be `https`;
/// * no userinfo (`user:pass@host`);
/// * the host must not be a literal private/reserved/loopback/link-local address, and
///   after resolution none of the resolved addresses may be in those ranges;
/// * if `CAT_HERDER_CALLBACK_ALLOWLIST` is set, the host must be in it.
pub async fn validate_callback_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid url: {e}"))?;

    if parsed.scheme() != "https" {
        return Err(format!("scheme must be https, got {:?}", parsed.scheme()));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err("url must not contain userinfo".into());
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| "url has no host".to_string())?;

    if let Some(allow) = allowlist() {
        if !allow.iter().any(|h| h.eq_ignore_ascii_case(host)) {
            return Err(format!("host {host:?} not in callback allowlist"));
        }
    }

    // A literal IP host: check directly.
    if let Ok(ip) = host.parse::<IpAddr>() {
        if is_forbidden_ip(&ip) {
            return Err(format!("host address {ip} is not allowed"));
        }
        return Ok(());
    }

    // Strip brackets for IPv6 literals (Url::host_str keeps them off already in most
    // cases, but be defensive).
    let bare = host.trim_start_matches('[').trim_end_matches(']');

    // Resolve and reject if ANY resolved address is forbidden.
    let addrs = tokio::net::lookup_host((bare, parsed.port_or_known_default().unwrap_or(443)))
        .await
        .map_err(|e| format!("failed to resolve {bare:?}: {e}"))?;
    let mut any = false;
    for addr in addrs {
        any = true;
        if is_forbidden_ip(&addr.ip()) {
            return Err(format!("resolved address {} is not allowed", addr.ip()));
        }
    }
    if !any {
        return Err(format!("host {bare:?} resolved to no addresses"));
    }
    Ok(())
}

fn allowlist() -> Option<Vec<String>> {
    let raw = std::env::var(ALLOWLIST_ENV).ok()?;
    let list: Vec<String> = raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if list.is_empty() { None } else { Some(list) }
}

/// Whether an IP is private/reserved/loopback/link-local (SSRF-blocked).
pub fn is_forbidden_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_forbidden_v4(v4),
        IpAddr::V6(v6) => is_forbidden_v6(v6),
    }
}

fn is_forbidden_v4(ip: &Ipv4Addr) -> bool {
    let o = ip.octets();
    ip.is_unspecified() // 0.0.0.0
        || ip.is_loopback() // 127/8
        || ip.is_private() // 10/8, 172.16/12, 192.168/16
        || ip.is_link_local() // 169.254/16
        || ip.is_broadcast()
        || ip.is_documentation()
        || o[0] == 0 // 0.0.0.0/8
        || (o[0] == 100 && (64..=127).contains(&o[1])) // 100.64/10 CGNAT
}

fn is_forbidden_v6(ip: &Ipv6Addr) -> bool {
    if ip.is_unspecified() || ip.is_loopback() {
        return true; // ::, ::1
    }
    let seg0 = ip.segments()[0];
    // fc00::/7 unique-local
    if (seg0 & 0xfe00) == 0xfc00 {
        return true;
    }
    // fe80::/10 link-local
    if (seg0 & 0xffc0) == 0xfe80 {
        return true;
    }
    // IPv4-mapped (::ffff:a.b.c.d): validate the embedded IPv4.
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_forbidden_v4(&v4);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use cat_herder_domain::entities::{RunId, RunKind};

    fn run(report: Option<&str>, callback: Option<&str>) -> Run {
        Run {
            id: RunId(7),
            kind: RunKind::Agent,
            state: RunState::Succeeded,
            task_spec: "x".into(),
            session_id: None,
            max_concurrency: 1,
            created_at: std::time::SystemTime::UNIX_EPOCH,
            trigger: "api".into(),
            actor: None,
            model: None,
            repo: None,
            language: None,
            workflow_ref: None,
            runtime_tier: Default::default(),
            started_at: None,
            finished_at: Some(std::time::SystemTime::UNIX_EPOCH),
            pr_url: Some("https://example.com/pr/1".into()),
            report: report.map(str::to_string),
            input_tokens: 0,
            output_tokens: 0,
            callback_url: callback.map(str::to_string),
        }
    }

    #[test]
    fn signature_is_deterministic_hex_hmac_sha256() {
        // Known-answer: HMAC-SHA256(key="key", msg="The quick brown fox jumps over the lazy dog")
        let sig = sign_body(b"key", b"The quick brown fox jumps over the lazy dog");
        assert_eq!(
            sig,
            "f7bc83f430538424b13298e6aa6fb143ef4d59a14946175997479dbc2d1a3cd8"
        );
        // Deterministic and 64 hex chars.
        assert_eq!(sig, sign_body(b"key", b"The quick brown fox jumps over the lazy dog"));
        assert_eq!(sig.len(), 64);
    }

    #[test]
    fn body_redacts_report_and_shape() {
        let secret_report = "leaked ghp_abcdefghijklmnopqrstuvwxyz0123456789 token";
        let body = build_body(&run(Some(secret_report), Some("https://example.com/cb")));
        assert_eq!(body["run_id"], 7);
        assert_eq!(body["state"], "succeeded");
        assert_eq!(body["trigger"], "api");
        assert_eq!(body["pr_url"], "https://example.com/pr/1");
        let report = body["report"].as_str().unwrap();
        assert!(report.contains("[REDACTED]"));
        assert!(!report.contains("ghp_abcdefghijklmnopqrstuvwxyz"));
        assert!(body["finished_at"].is_string());
    }

    #[test]
    fn body_has_null_report_and_pr_when_absent() {
        let mut r = run(None, None);
        r.pr_url = None;
        let body = build_body(&r);
        assert!(body["report"].is_null());
        assert!(body["pr_url"].is_null());
    }

    #[test]
    fn rejects_non_https_scheme() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        for url in [
            "http://example.com/cb",
            "ftp://example.com/cb",
            "file:///etc/passwd",
        ] {
            assert!(
                rt.block_on(validate_callback_url(url)).is_err(),
                "should reject {url}"
            );
        }
    }

    #[test]
    fn rejects_userinfo() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let err = rt
            .block_on(validate_callback_url("https://user:pass@example.com/cb"))
            .unwrap_err();
        assert!(err.contains("userinfo"), "{err}");
    }

    #[test]
    fn rejects_forbidden_literal_addresses() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        for url in [
            "https://127.0.0.1/cb",
            "https://10.0.0.1/cb",
            "https://172.16.0.1/cb",
            "https://192.168.1.1/cb",
            "https://169.254.169.254/cb",
            "https://0.0.0.0/cb",
            "https://[::1]/cb",
            "https://[::]/cb",
            "https://[fc00::1]/cb",
            "https://[fe80::1]/cb",
        ] {
            let res = rt.block_on(validate_callback_url(url));
            assert!(res.is_err(), "should reject {url}: {res:?}");
        }
    }

    #[test]
    fn accepts_public_literal_ip() {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        assert!(rt.block_on(validate_callback_url("https://93.184.216.34/cb")).is_ok());
    }

    #[test]
    fn forbidden_ip_helper_covers_ranges() {
        for ip in [
            "10.1.2.3",
            "172.20.0.1",
            "192.168.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "0.0.0.0",
            "::1",
            "::",
            "fc00::1",
            "fd00::1",
            "fe80::1",
        ] {
            assert!(is_forbidden_ip(&ip.parse().unwrap()), "{ip} should be forbidden");
        }
        for ip in ["93.184.216.34", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(!is_forbidden_ip(&ip.parse().unwrap()), "{ip} should be allowed");
        }
    }
}

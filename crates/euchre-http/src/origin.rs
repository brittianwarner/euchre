//! Public-origin resolution (TS `server/public-origin.ts`) and the Origin checks.
//!
//! The public origin comes from the request `Host` (or the HTTP/2 authority) matched
//! exactly against the configured origins. `X-Forwarded-Host` and
//! `X-Forwarded-Proto` are never read: behind Railway's TLS edge the request arrives
//! as plain `http`, and the configured origin supplies the scheme.

use axum::http::{HeaderMap, Uri, header};
use euchre_server_core::config::OriginPolicy;

use crate::error::ApiError;

/// The resolved public origin of the current request, for example
/// `https://euchre.sh`. Inserted as a request extension by the front door.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicOrigin(pub String);

/// `url.host` of the request as WHATWG `new URL("http://" + host)` computes it:
/// lowercase, IDNA-normalized, default port 80 dropped. `None` when there is no host
/// or it is not a bare `host[:port]`.
pub fn request_host(headers: &HeaderMap, uri: &Uri) -> Option<String> {
    let raw = match headers.get(header::HOST) {
        Some(value) => value.to_str().ok()?.to_owned(),
        None => uri.authority()?.as_str().to_owned(),
    };
    normalize_host(&raw)
}

/// Normalize a raw `Host` value to WHATWG `url.host`, or `None` when it is not a
/// bare `host[:port]` (userinfo, path, query or fragment are refused).
pub fn normalize_host(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.contains(['/', '\\', '?', '#', '@']) {
        return None;
    }
    let url = url::Url::parse(&format!("http://{raw}/")).ok()?;
    let host = url.host_str()?;
    Some(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_owned(),
    })
}

/// The configured origin for this request's host (TS `publicOriginFor(url)`).
pub fn resolve_public_origin(
    policy: &OriginPolicy,
    headers: &HeaderMap,
    uri: &Uri,
) -> Option<String> {
    let host = request_host(headers, uri)?;
    policy.public_origin_for_host(&host).map(str::to_owned)
}

/// The request `Origin` header, when present and readable.
pub fn origin_header(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())
}

/// `Origin not allowed`, the TS session-route message.
pub const ORIGIN_NOT_ALLOWED: &str = "Origin not allowed";

/// CSRF for ambient (cookie) callers: `Origin` must equal the public origin exactly.
/// A missing Origin fails too, as `request.headers.get('origin') !== url.origin` did.
pub fn require_same_origin(headers: &HeaderMap, public_origin: &str) -> Result<(), ApiError> {
    if origin_header(headers) == Some(public_origin) {
        Ok(())
    } else {
        Err(ApiError::forbidden(ORIGIN_NOT_ALLOWED))
    }
}

/// D1: a present Origin must be allowlisted; an absent one passes.
pub fn require_allowed_origin(headers: &HeaderMap, policy: &OriginPolicy) -> Result<(), ApiError> {
    let origin = match headers.get(header::ORIGIN) {
        None => None,
        Some(value) => Some(
            value
                .to_str()
                .map_err(|_| ApiError::forbidden(ORIGIN_NOT_ALLOWED))?,
        ),
    };
    if policy.check(origin) {
        Ok(())
    } else {
        Err(ApiError::forbidden(ORIGIN_NOT_ALLOWED))
    }
}

#[cfg(test)]
mod tests {
    //! Port of `server/public-origin.test.ts`.

    use super::*;

    fn resolver(origins: &[&str]) -> anyhow::Result<OriginPolicy> {
        OriginPolicy::strict(origins.iter().map(|s| (*s).to_owned()).collect())
    }

    fn origin_for(policy: &OriginPolicy, host: &str) -> Option<String> {
        let uri: Uri = format!("http://{host}/api/table-session/game")
            .parse()
            .ok()?;
        resolve_public_origin(policy, &HeaderMap::new(), &uri)
    }

    const CONFIGURED: [&str; 3] = [
        "https://euchre.sh",
        "https://www.euchre.sh",
        "https://euchre-production-9809.up.railway.app",
    ];

    #[test]
    fn preserves_each_configured_host_for_session_checks_and_cookie_bound_websockets()
    -> anyhow::Result<()> {
        let policy = resolver(&CONFIGURED)?;
        for host in [
            "euchre.sh",
            "www.euchre.sh",
            "euchre-production-9809.up.railway.app",
        ] {
            assert_eq!(
                origin_for(&policy, host).as_deref(),
                Some(format!("https://{host}").as_str())
            );
        }
        Ok(())
    }

    #[test]
    fn rejects_unconfigured_hosts() -> anyhow::Result<()> {
        let policy = resolver(&CONFIGURED)?;
        for host in [
            "attacker.example",
            "euchre.sh.attacker.example",
            "www.euchre.sh:444",
        ] {
            assert_eq!(origin_for(&policy, host), None, "{host}");
        }
        Ok(())
    }

    #[test]
    fn rejects_invalid_configured_origins() {
        for origin in [
            "https://euchre.sh/path",
            "https://user:password@euchre.sh",
            "ftp://euchre.sh",
        ] {
            assert!(resolver(&[origin]).is_err(), "{origin}");
        }
    }

    #[test]
    fn rejects_ambiguous_schemes_for_the_same_host() {
        assert!(resolver(&["http://euchre.sh", "https://euchre.sh"]).is_err());
    }

    #[test]
    fn host_header_wins_and_is_normalized() -> anyhow::Result<()> {
        let policy = resolver(&["https://euchre.sh"])?;
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "EUCHRE.sh:80".parse()?);
        let uri: Uri = "/api/session".parse()?;
        assert_eq!(
            resolve_public_origin(&policy, &headers, &uri).as_deref(),
            Some("https://euchre.sh")
        );
        Ok(())
    }

    #[test]
    fn forwarded_host_is_never_trusted() -> anyhow::Result<()> {
        let policy = resolver(&["https://euchre.sh"])?;
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, "attacker.example".parse()?);
        headers.insert("x-forwarded-host", "euchre.sh".parse()?);
        let uri: Uri = "/".parse()?;
        assert_eq!(resolve_public_origin(&policy, &headers, &uri), None);
        Ok(())
    }

    #[test]
    fn malformed_hosts_resolve_to_nothing() {
        for raw in [
            "",
            "a@euchre.sh",
            "euchre.sh/x",
            "euchre.sh?x",
            "euchre.sh:99999",
        ] {
            assert_eq!(normalize_host(raw), None, "{raw}");
        }
        assert_eq!(normalize_host("[::1]:3000").as_deref(), Some("[::1]:3000"));
    }
}

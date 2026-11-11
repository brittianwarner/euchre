//! Who is calling (05 §11.1): the guest session from `Authorization: Bearer` first,
//! then the `euchre_guest` cookie; the email identity from `X-Euchre-Identity` first,
//! then the `euchre_email` cookie (D2).

use axum::http::{HeaderMap, header};
use euchre_server_core::auth::identity::IDENTITY_HEADER;
use euchre_server_core::auth::{Authenticator, GuestAudience, Identity};

use crate::cookies::{GUEST_COOKIE, IDENTITY_COOKIE, read_cookie};

/// How the guest session was presented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Session {
    /// A valid `Authorization: Bearer <session jwt>` (desktop). Not ambient, so it
    /// is exempt from the CSRF Origin check.
    Bearer(String),
    /// A valid `euchre_guest` cookie (browser). Ambient: state changes need
    /// `Origin == public origin`.
    Cookie(String),
    /// An `Authorization: Bearer` header that did not verify. The cookie is not
    /// consulted: a client that sends a bearer means to use it.
    InvalidBearer,
    /// No bearer, and no valid cookie.
    Anonymous,
}

/// The resolved caller of one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    /// The guest session.
    pub session: Session,
    /// The email-nickname identity, when one verified.
    pub identity: Option<Identity>,
}

impl Caller {
    /// Resolve the caller from request headers. Never fails; invalid credentials
    /// resolve to [`Session::InvalidBearer`] / [`Session::Anonymous`] / no identity.
    pub fn resolve(auth: &Authenticator, headers: &HeaderMap) -> Self {
        let session = match bearer_token(headers) {
            Some(token) => match auth.verify_session_bearer(token) {
                Ok(user_id) => Session::Bearer(user_id),
                Err(_) => Session::InvalidBearer,
            },
            None => match read_cookie(headers, GUEST_COOKIE) {
                Some(token) => match auth.verify_guest(&token, GuestAudience::Session) {
                    Ok(user_id) => Session::Cookie(user_id),
                    Err(_) => Session::Anonymous,
                },
                None => Session::Anonymous,
            },
        };
        let identity = headers
            .get(IDENTITY_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned)
            .or_else(|| read_cookie(headers, IDENTITY_COOKIE))
            .and_then(|value| auth.resolve_identity(&value));
        Self { session, identity }
    }

    /// The guest user id (`guest-<uuid>`), when the session verified.
    pub fn guest_user_id(&self) -> Option<&str> {
        match &self.session {
            Session::Bearer(id) | Session::Cookie(id) => Some(id),
            Session::InvalidBearer | Session::Anonymous => None,
        }
    }

    /// True when a verified bearer authenticated the request (CSRF-exempt).
    pub fn is_bearer(&self) -> bool {
        matches!(self.session, Session::Bearer(_))
    }

    /// True when the request is ambient (no bearer header at all): cookie callers and
    /// anonymous browsers. These get the CSRF Origin check on state changes.
    pub fn is_ambient(&self) -> bool {
        matches!(self.session, Session::Cookie(_) | Session::Anonymous)
    }
}

/// The token of an `Authorization: Bearer <token>` header (scheme case-insensitive).
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    let token = token.trim();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use euchre_server_core::auth::AuthKeys;
    use euchre_server_core::config::Secret;

    fn auth() -> Authenticator {
        Authenticator::new(AuthKeys {
            guest: Secret::new("g".repeat(40)),
            profile: Secret::new("p".repeat(40)),
            internal: Secret::new("i".repeat(40)),
            identity: Secret::new("d".repeat(40)),
            jwt_issuer: None,
            allow_unsigned_email: false,
        })
    }

    #[test]
    fn bearer_wins_over_cookie() -> anyhow::Result<()> {
        let auth = auth();
        let a = auth.sign_guest("guest-a", GuestAudience::Session)?;
        let b = auth.sign_guest("guest-b", GuestAudience::Session)?;
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, format!("Bearer {a}").parse()?);
        headers.insert(header::COOKIE, format!("euchre_guest={b}").parse()?);
        let caller = Caller::resolve(&auth, &headers);
        assert_eq!(caller.session, Session::Bearer("guest-a".into()));
        assert!(caller.is_bearer());
        assert!(!caller.is_ambient());
        Ok(())
    }

    #[test]
    fn an_invalid_bearer_never_falls_back_to_the_cookie() -> anyhow::Result<()> {
        let auth = auth();
        let b = auth.sign_guest("guest-b", GuestAudience::Session)?;
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer nope".parse()?);
        headers.insert(header::COOKIE, format!("euchre_guest={b}").parse()?);
        let caller = Caller::resolve(&auth, &headers);
        assert_eq!(caller.session, Session::InvalidBearer);
        assert_eq!(caller.guest_user_id(), None);
        Ok(())
    }

    #[test]
    fn an_actors_token_is_not_a_session() -> anyhow::Result<()> {
        let auth = auth();
        let actors = auth.sign_guest("guest-a", GuestAudience::Actors)?;
        let mut headers = HeaderMap::new();
        headers.insert(header::COOKIE, format!("euchre_guest={actors}").parse()?);
        assert_eq!(Caller::resolve(&auth, &headers).session, Session::Anonymous);
        Ok(())
    }

    #[test]
    fn identity_header_wins_over_cookie() -> anyhow::Result<()> {
        let auth = auth();
        let ann = auth.identity_for_email("ann@example.com")?;
        let bob = auth.identity_for_email("bob@example.com")?;
        let mut headers = HeaderMap::new();
        headers.insert(IDENTITY_HEADER, auth.issue_identity_token(&ann)?.parse()?);
        headers.insert(
            header::COOKIE,
            format!("euchre_email={}", auth.issue_identity_token(&bob)?).parse()?,
        );
        assert_eq!(Caller::resolve(&auth, &headers).identity, Some(ann));
        // The legacy plain email is refused unless EUCHRE_ALLOW_UNSIGNED_EMAIL=1.
        let mut legacy = HeaderMap::new();
        legacy.insert(header::COOKIE, "euchre_email=ann%40example.com".parse()?);
        assert_eq!(Caller::resolve(&auth, &legacy).identity, None);
        Ok(())
    }

    #[test]
    fn bearer_scheme_parsing() -> anyhow::Result<()> {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "bearer  abc ".parse()?);
        assert_eq!(bearer_token(&headers), Some("abc"));
        headers.insert(header::AUTHORIZATION, "Basic abc".parse()?);
        assert_eq!(bearer_token(&headers), None);
        headers.insert(header::AUTHORIZATION, "Bearer ".parse()?);
        assert_eq!(bearer_token(&headers), None);
        Ok(())
    }
}

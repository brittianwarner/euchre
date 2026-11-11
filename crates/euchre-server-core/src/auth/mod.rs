//! Token families: guest, profile, internal, identity (05 §2, §10.2, §11.4). Owned by lane SERVER-FOUNDATION.
//!
//! Each family lives in its own module and is byte-compatible with the TS app:
//!
//! | Module | Family | TS source |
//! |---|---|---|
//! | [`guest`] | guest session (`euchre-session`, 400 d) and actor (`euchre-actors`, 2 h) JWTs | `src/lib/actors/auth/guest.ts`, `verify.ts` |
//! | [`profile`] | hand-rolled HS256 profile JWT (30 min, 1 h cap) | `session.server.ts` `mintProfileToken`, `player-profile/auth.ts` |
//! | [`internal`] | `EUCHRE_INTERNAL_TOKEN` and the per-match token | `player-profile/auth.ts`, `euchre-table/queues.ts` |
//! | [`identity`] | email nickname: normalization, user id, signed identity token (D2) | `session.server.ts` |
//! | [`device`] | desktop device enrollment and the on-disk device record (D5) | new (05 §11.4) |
//!
//! [`Authenticator`] bundles the resolved secrets so callers never handle raw keys.
//! Every function that depends on time has an `_at` variant with an explicit clock
//! for tests; the plain variant reads the system clock.
//!
//! Verification never panics and never logs token contents. Every failure is an
//! [`AuthError`] whose [`AuthError::code`] is the wire code (`invalid_token`,
//! `internal_token_mismatch`, `bad_request`).

pub mod device;
pub mod guest;
pub mod identity;
pub mod internal;
pub mod profile;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{AuthSecrets, Secret, ServerConfig};

pub use device::{DeviceEnrollment, DeviceRecord};
pub use guest::{GuestAudience, PlayerClaims};
pub use identity::Identity;
pub use profile::ProfileClaims;

/// Why a token or credential was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// A JWT failed verification. The message is the precise reason, meant for logs
    /// (profile actor errors carry it as the message, as TS does). Code `invalid_token`.
    #[error("{0}")]
    InvalidToken(String),
    /// The internal token was absent, wrong or unconfigured. Code `internal_token_mismatch`.
    /// One code for every case by design (no oracle).
    #[error("{0}")]
    InternalTokenMismatch(&'static str),
    /// An email nickname failed normalization. Code `bad_request`.
    #[error("That doesn\u{2019}t look like an email address yet.")]
    InvalidEmail,
    /// Signing failed (serialization or crypto). Never expected; code `internal_error`.
    #[error("token signing failed: {0}")]
    Signing(String),
}

impl AuthError {
    /// The wire code for this error.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidToken(_) => "invalid_token",
            Self::InternalTokenMismatch(_) => "internal_token_mismatch",
            Self::InvalidEmail => "bad_request",
            Self::Signing(_) => "internal_error",
        }
    }

    /// Convert into a public rivetkit `user/<code>` error carrying this message.
    pub fn into_user_error(self) -> anyhow::Error {
        let code = self.code();
        crate::rivet::user_error(code, self.to_string())
    }

    pub(crate) fn invalid(reason: impl Into<String>) -> Self {
        Self::InvalidToken(reason.into())
    }
}

/// Current unix time in whole seconds (JS `Math.floor(Date.now() / 1000)`).
pub fn now_secs() -> i64 {
    now_ms().div_euclid(1000)
}

/// Current unix time in milliseconds (JS `Date.now()`).
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// The resolved secrets every token family needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthKeys {
    /// Guest JWT key.
    pub guest: Secret,
    /// Profile JWT key.
    pub profile: Secret,
    /// Deployment internal token.
    pub internal: Secret,
    /// Identity token key (D2).
    pub identity: Secret,
    /// Required `iss` on profile tokens, also set by the Rust minter.
    pub jwt_issuer: Option<String>,
    /// Accept the legacy plain-email identity cookie (`EUCHRE_ALLOW_UNSIGNED_EMAIL=1`).
    pub allow_unsigned_email: bool,
}

impl AuthKeys {
    /// Keys from resolved config secrets.
    pub fn from_secrets(secrets: &AuthSecrets, allow_unsigned_email: bool) -> Self {
        Self {
            guest: secrets.guest.value.clone(),
            profile: secrets.profile.value.clone(),
            internal: secrets.internal.value.clone(),
            identity: secrets.identity.value.clone(),
            jwt_issuer: secrets.jwt_issuer.clone(),
            allow_unsigned_email,
        }
    }
}

/// Cheap-to-clone facade over every token family, bound to one set of keys.
#[derive(Debug, Clone)]
pub struct Authenticator {
    keys: Arc<AuthKeys>,
}

impl Authenticator {
    /// Build from explicit keys.
    pub fn new(keys: AuthKeys) -> Self {
        Self {
            keys: Arc::new(keys),
        }
    }

    /// Build from a server config.
    pub fn from_config(cfg: &ServerConfig) -> Self {
        Self::new(AuthKeys::from_secrets(
            &cfg.secrets,
            cfg.allow_unsigned_email,
        ))
    }

    /// The keys in use.
    pub fn keys(&self) -> &AuthKeys {
        &self.keys
    }

    // ----- guest -----

    /// Mint a guest JWT for `user_id` (TS `signGuest`).
    pub fn sign_guest(&self, user_id: &str, aud: GuestAudience) -> Result<String, AuthError> {
        self.sign_guest_at(user_id, aud, now_secs())
    }

    /// [`Authenticator::sign_guest`] at an explicit `iat` (unix seconds).
    pub fn sign_guest_at(
        &self,
        user_id: &str,
        aud: GuestAudience,
        iat: i64,
    ) -> Result<String, AuthError> {
        guest::sign(self.keys.guest.as_bytes(), user_id, aud, iat)
    }

    /// Verify a guest JWT and return its `sub` (TS `verifyGuest`).
    pub fn verify_guest(&self, token: &str, aud: GuestAudience) -> Result<String, AuthError> {
        self.verify_guest_at(token, aud, now_secs())
    }

    /// [`Authenticator::verify_guest`] at an explicit `now` (unix seconds).
    pub fn verify_guest_at(
        &self,
        token: &str,
        aud: GuestAudience,
        now: i64,
    ) -> Result<String, AuthError> {
        guest::verify(self.keys.guest.as_bytes(), token, aud, now)
    }

    /// Verify a table connection token (TS `verifyPlayer`, HS256 path only).
    pub fn verify_player(&self, token: &str) -> Result<PlayerClaims, AuthError> {
        self.verify_player_at(token, now_secs())
    }

    /// [`Authenticator::verify_player`] at an explicit `now` (unix seconds).
    pub fn verify_player_at(&self, token: &str, now: i64) -> Result<PlayerClaims, AuthError> {
        guest::verify_player(self.keys.guest.as_bytes(), token, now)
    }

    // ----- profile -----

    /// Mint a 30-minute profile token (TS `mintProfileToken`).
    pub fn mint_profile_token(&self, user_id: &str) -> Result<String, AuthError> {
        self.mint_profile_token_at(user_id, now_secs())
    }

    /// [`Authenticator::mint_profile_token`] at an explicit `iat` (unix seconds).
    pub fn mint_profile_token_at(&self, user_id: &str, iat: i64) -> Result<String, AuthError> {
        profile::mint(
            self.keys.profile.as_bytes(),
            user_id,
            iat,
            self.keys.jwt_issuer.as_deref(),
        )
    }

    /// Verify a profile token (TS `verifyProfileToken(token, Date.now())`).
    pub fn verify_profile_token(&self, token: &str) -> Result<ProfileClaims, AuthError> {
        self.verify_profile_token_at(token, now_ms())
    }

    /// [`Authenticator::verify_profile_token`] at an explicit `now` (unix milliseconds).
    pub fn verify_profile_token_at(
        &self,
        token: &str,
        now_ms: i64,
    ) -> Result<ProfileClaims, AuthError> {
        profile::verify(
            self.keys.profile.as_bytes(),
            token,
            now_ms,
            self.keys.jwt_issuer.as_deref(),
        )
    }

    // ----- internal -----

    /// The deployment internal token, to attach to actor-to-actor calls.
    pub fn internal_token(&self) -> &Secret {
        &self.keys.internal
    }

    /// TS `assertInternalToken`: constant-time over SHA-256 digests.
    pub fn assert_internal_token(&self, token: Option<&str>) -> Result<(), AuthError> {
        internal::assert_internal_token(Some(&self.keys.internal), token)
    }

    /// Boolean form of [`Authenticator::assert_internal_token`] (TS profile `tokenOk`).
    pub fn internal_token_ok(&self, token: Option<&str>) -> bool {
        self.assert_internal_token(token).is_ok()
    }

    // ----- identity -----

    /// Normalize an email nickname and derive its identity (TS `normalizeEmail` + `deriveUserId`).
    pub fn identity_for_email(&self, raw: &str) -> Result<Identity, AuthError> {
        identity::Identity::from_email(raw).ok_or(AuthError::InvalidEmail)
    }

    /// Issue a signed identity token for `identity` (D2).
    pub fn issue_identity_token(&self, identity: &Identity) -> Result<String, AuthError> {
        self.issue_identity_token_at(identity, now_secs())
    }

    /// [`Authenticator::issue_identity_token`] at an explicit `iat` (unix seconds).
    pub fn issue_identity_token_at(
        &self,
        identity: &Identity,
        iat: i64,
    ) -> Result<String, AuthError> {
        identity::sign(self.keys.identity.as_bytes(), identity, iat)
    }

    /// Verify a signed identity token.
    pub fn verify_identity_token(&self, token: &str) -> Result<Identity, AuthError> {
        self.verify_identity_token_at(token, now_secs())
    }

    /// [`Authenticator::verify_identity_token`] at an explicit `now` (unix seconds).
    pub fn verify_identity_token_at(&self, token: &str, now: i64) -> Result<Identity, AuthError> {
        identity::verify(self.keys.identity.as_bytes(), token, now)
    }

    /// Resolve an `euchre_email` cookie or `X-Euchre-Identity` header value: a signed
    /// identity token, or (only with `EUCHRE_ALLOW_UNSIGNED_EMAIL=1`) a legacy plain email.
    pub fn resolve_identity(&self, value: &str) -> Option<Identity> {
        self.resolve_identity_at(value, now_secs())
    }

    /// [`Authenticator::resolve_identity`] at an explicit `now` (unix seconds).
    pub fn resolve_identity_at(&self, value: &str, now: i64) -> Option<Identity> {
        identity::resolve(
            self.keys.identity.as_bytes(),
            value,
            self.keys.allow_unsigned_email,
            now,
        )
    }

    // ----- device -----

    /// Enroll a new desktop device: fresh `guest-<uuid>` and a 400-day session token.
    pub fn enroll_device(&self) -> Result<DeviceEnrollment, AuthError> {
        self.enroll_device_at(now_secs())
    }

    /// [`Authenticator::enroll_device`] at an explicit `iat` (unix seconds).
    pub fn enroll_device_at(&self, iat: i64) -> Result<DeviceEnrollment, AuthError> {
        device::enroll(self.keys.guest.as_bytes(), iat)
    }

    /// Re-mint a device session token for the same user id (`POST /api/device/refresh`).
    pub fn refresh_device(&self, bearer: &str) -> Result<DeviceEnrollment, AuthError> {
        self.refresh_device_at(bearer, now_secs())
    }

    /// [`Authenticator::refresh_device`] at an explicit `now` (unix seconds).
    pub fn refresh_device_at(&self, bearer: &str, now: i64) -> Result<DeviceEnrollment, AuthError> {
        device::refresh(self.keys.guest.as_bytes(), bearer, now)
    }

    /// Verify a device (or cookie) bearer: a guest session token. Returns the guest id.
    pub fn verify_session_bearer(&self, bearer: &str) -> Result<String, AuthError> {
        self.verify_guest(bearer, GuestAudience::Session)
    }
}

/// Shared JWT helpers for the jose-compatible families (guest, identity).
pub(crate) mod jwt {
    use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
    use serde::Serialize;
    use serde_json::{Map, Value};

    use super::AuthError;

    /// Sign `claims` with header exactly `{"alg":"HS256"}` (jose `setProtectedHeader({alg})`).
    pub fn sign_bare_hs256<T: Serialize>(key: &[u8], claims: &T) -> Result<String, AuthError> {
        let header = Header {
            typ: None,
            ..Header::new(Algorithm::HS256)
        };
        jsonwebtoken::encode(&header, claims, &EncodingKey::from_secret(key))
            .map_err(|e| AuthError::Signing(e.to_string()))
    }

    /// jose 6 header checks for `jwtVerify` with `algorithms: ["HS256"]`, in jose's
    /// order: `validateCrit` (the only recognized extension is `b64`; like jose's verify
    /// path, duplicate `crit` entries are not rejected), `validateB64`, then the `alg`
    /// allowlist, and finally the JWT rule against an unencoded payload. A header that
    /// does not decode is left to jsonwebtoken, which rejects it.
    fn validate_header(token: &str) -> Result<(), AuthError> {
        use base64::Engine as _;
        let Some(raw) = token.split('.').next() else {
            return Ok(());
        };
        let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw) else {
            return Ok(());
        };
        let Ok(Value::Object(header)) = serde_json::from_slice::<Value>(&bytes) else {
            return Ok(());
        };
        let mut unencoded = false;
        if let Some(crit) = header.get("crit") {
            unencoded = validate_crit(&header, crit)?;
        }
        match header.get("alg").and_then(Value::as_str) {
            None | Some("") => {
                return Err(AuthError::invalid(
                    "JWS \"alg\" (Algorithm) Header Parameter missing or invalid",
                ));
            }
            Some("HS256") => {}
            Some(_) => {
                return Err(AuthError::invalid(
                    "\"alg\" (Algorithm) Header Parameter value not allowed",
                ));
            }
        }
        if unencoded {
            return Err(AuthError::invalid("JWTs MUST NOT use unencoded payload"));
        }
        Ok(())
    }

    /// The `crit` half of [`validate_header`] (jose `validateCrit` then `validateB64`);
    /// returns true when the token asks for an unencoded payload (`b64: false`).
    fn validate_crit(header: &Map<String, Value>, crit: &Value) -> Result<bool, AuthError> {
        let Value::Array(items) = crit else {
            return Err(crit_not_array());
        };
        if items.is_empty() {
            return Err(crit_not_array());
        }
        let mut names = Vec::with_capacity(items.len());
        for item in items {
            match item.as_str() {
                Some(name) if !name.is_empty() => names.push(name),
                _ => return Err(crit_not_array()),
            }
        }
        for name in &names {
            if *name != "b64" {
                return Err(AuthError::invalid(format!(
                    "Extension Header Parameter \"{name}\" is not recognized"
                )));
            }
            if !header.contains_key(*name) {
                return Err(AuthError::invalid(format!(
                    "Extension Header Parameter \"{name}\" is missing"
                )));
            }
        }
        match header.get("b64") {
            Some(Value::Bool(b64)) => Ok(!b64),
            _ => Err(AuthError::invalid(
                "The \"b64\" (base64url-encode payload) Header Parameter must be a boolean",
            )),
        }
    }

    fn crit_not_array() -> AuthError {
        AuthError::invalid(
            "\"crit\" (Critical) Header Parameter MUST be an array of non-empty strings when present",
        )
    }

    /// Expected claims for [`verify_jose_hs256`].
    pub struct Expect<'a> {
        pub issuer: &'a str,
        pub audience: &'a str,
    }

    /// HS256 verify with jose 6 `jwtVerify` claim semantics (tolerance 0), plus a
    /// required `exp` (every token these families mint has one).
    ///
    /// jsonwebtoken checks the signature with the algorithm pinned to HS256 (a token
    /// asking for `none` or any other algorithm fails before claims are read); every
    /// claim check is then done here, in jose's order, on the raw JSON payload.
    pub fn verify_jose_hs256(
        key: &[u8],
        token: &str,
        expect: &Expect<'_>,
        now: i64,
    ) -> Result<Map<String, Value>, AuthError> {
        if token.split('.').count() != 3 {
            return Err(AuthError::invalid("Invalid Compact JWS"));
        }
        validate_header(token)?;
        let mut validation = Validation::new(Algorithm::HS256);
        validation.algorithms = vec![Algorithm::HS256];
        validation.leeway = 0;
        validation.validate_exp = false;
        validation.validate_nbf = false;
        validation.validate_aud = false;
        validation.required_spec_claims.clear();
        let data =
            jsonwebtoken::decode::<Value>(token, &DecodingKey::from_secret(key), &validation)
                .map_err(|e| AuthError::invalid(format!("jwt rejected: {e}")))?;
        let header = &data.header;
        if header.alg != Algorithm::HS256 {
            return Err(AuthError::invalid("unsupported token algorithm"));
        }
        let Value::Object(payload) = data.claims else {
            return Err(AuthError::invalid(
                "JWT Claims Set must be a top-level JSON object",
            ));
        };
        // jose 6 `validateClaimsSet` checks presence in the order of its `requiredClaims`
        // set: iss, then aud.
        for claim in ["iss", "aud"] {
            if !payload.contains_key(claim) {
                return Err(AuthError::invalid(format!(
                    "missing required \"{claim}\" claim"
                )));
            }
        }
        if payload.get("iss").and_then(Value::as_str) != Some(expect.issuer) {
            return Err(AuthError::invalid("unexpected \"iss\" claim value"));
        }
        let aud_ok = match payload.get("aud") {
            Some(Value::String(aud)) => aud == expect.audience,
            Some(Value::Array(items)) => items.iter().any(|v| v.as_str() == Some(expect.audience)),
            _ => false,
        };
        if !aud_ok {
            return Err(AuthError::invalid("unexpected \"aud\" claim value"));
        }
        let now = now as f64;
        let numeric = |claim: &str| -> Result<Option<f64>, AuthError> {
            match payload.get(claim) {
                None => Ok(None),
                Some(Value::Number(n)) => Ok(n.as_f64()),
                Some(_) => Err(AuthError::invalid(format!(
                    "\"{claim}\" claim must be a number"
                ))),
            }
        };
        let _iat = numeric("iat")?;
        if let Some(nbf) = numeric("nbf")?
            && nbf > now
        {
            return Err(AuthError::invalid("\"nbf\" claim timestamp check failed"));
        }
        match numeric("exp")? {
            Some(exp) if exp <= now => {
                Err(AuthError::invalid("\"exp\" claim timestamp check failed"))
            }
            Some(_) => Ok(payload),
            None => Err(AuthError::invalid("missing required \"exp\" claim")),
        }
    }
}

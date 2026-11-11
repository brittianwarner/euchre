//! The one error this crate raises: a TS `UserError` carrying a protocol code.
//!
//! TS throws `protocolError(code, message)` (`guards.ts`); the actor lane maps a
//! [`ProfileError`] to `user_error(code.as_str(), message)`. Messages are copied
//! byte for byte from TS (D27).

use std::fmt;

use euchre_protocol::ProtocolErrorCode;

/// A rejected input or a corrupt row, as `protocolError(code, message)`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ProfileError {
    /// The machine code (`internal_error`, `invalid_persona`, `forbidden`, ...).
    pub code: ProtocolErrorCode,
    /// The exact TS message.
    pub message: String,
}

impl ProfileError {
    /// Any code with any message.
    pub fn new(code: ProtocolErrorCode, message: impl Into<String>) -> ProfileError {
        ProfileError {
            code,
            message: message.into(),
        }
    }

    /// `protocolError('internal_error', message)`.
    pub fn internal(message: impl Into<String>) -> ProfileError {
        ProfileError::new(ProtocolErrorCode::InternalError, message)
    }

    /// `protocolError('invalid_persona', message)`.
    pub fn invalid_persona(message: impl Into<String>) -> ProfileError {
        ProfileError::new(ProtocolErrorCode::InvalidPersona, message)
    }

    /// `protocolError('forbidden', message)`.
    pub fn forbidden(message: impl Into<String>) -> ProfileError {
        ProfileError::new(ProtocolErrorCode::Forbidden, message)
    }
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for ProfileError {}

/// `Result` with a [`ProfileError`].
pub type ProfileResult<T> = Result<T, ProfileError>;

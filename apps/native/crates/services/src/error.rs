//! UI boundary: match the code for localized copy; diagnostics are for logs only.
use serde::{Deserialize, Serialize};
use std::{fmt, io};

pub type Result<T> = std::result::Result<T, ServiceError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceErrorCode {
    Offline,
    NotFound,
    TooLarge,
    InvalidProject,
    PermissionDenied,
    DiskFull,
    InvalidInput,
    InvalidData,
    Unavailable,
    Unknown,
}

/// The message (including Display/Debug/source) is developer diagnostics, never UI copy.
/// UI owns the sentence-case copy and document-name substitution from review item 1.
#[derive(Debug)]
pub struct ServiceError {
    pub code: ServiceErrorCode,
    diagnostic: anyhow::Error,
}
impl ServiceError {
    pub fn new(code: ServiceErrorCode, diagnostic: impl Into<anyhow::Error>) -> Self {
        Self {
            code,
            diagnostic: diagnostic.into(),
        }
    }
    pub fn message(code: ServiceErrorCode, diagnostic: impl Into<String>) -> Self {
        Self::new(code, anyhow::Error::msg(diagnostic.into()))
    }
    pub fn diagnostic(&self) -> &anyhow::Error {
        &self.diagnostic
    }
    fn context(self, message: impl fmt::Display + Send + Sync + 'static) -> Self {
        Self {
            code: self.code,
            diagnostic: self.diagnostic.context(message),
        }
    }
}
impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.diagnostic, f)
    }
}
impl std::error::Error for ServiceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.diagnostic.as_ref())
    }
}
impl From<io::Error> for ServiceError {
    fn from(error: io::Error) -> Self {
        use io::ErrorKind::*;
        let code = match error.kind() {
            NotFound => ServiceErrorCode::NotFound,
            PermissionDenied => ServiceErrorCode::PermissionDenied,
            StorageFull => ServiceErrorCode::DiskFull,
            ConnectionRefused | ConnectionReset | ConnectionAborted | NotConnected
            | NetworkDown | NetworkUnreachable | HostUnreachable | TimedOut => {
                ServiceErrorCode::Offline
            }
            _ => ServiceErrorCode::Unknown,
        };
        Self::new(code, error)
    }
}
impl From<anyhow::Error> for ServiceError {
    fn from(error: anyhow::Error) -> Self {
        // Preserve classifications through host anyhow contexts, without parsing messages.
        let code = error
            .chain()
            .find_map(|cause| {
                cause.downcast_ref::<Self>().map(|e| e.code).or_else(|| {
                    cause
                        .downcast_ref::<io::Error>()
                        .map(|e| Self::from(io::Error::from(e.kind())).code)
                })
            })
            .unwrap_or(ServiceErrorCode::Unknown);
        Self::new(code, error)
    }
}
macro_rules! conversion {
    ($error:ty, $code:ident) => {
        impl From<$error> for ServiceError {
            fn from(error: $error) -> Self {
                Self::new(ServiceErrorCode::$code, error)
            }
        }
    };
}
conversion!(serde_json::Error, InvalidData);
conversion!(url::ParseError, InvalidInput);
conversion!(semver::Error, InvalidData);
conversion!(std::num::ParseIntError, InvalidData);
conversion!(std::sync::mpsc::RecvError, Unavailable);
impl<T: Send + Sync + 'static> From<std::sync::mpsc::SendError<T>> for ServiceError {
    fn from(error: std::sync::mpsc::SendError<T>) -> Self {
        Self::new(ServiceErrorCode::Unavailable, error)
    }
}

pub(crate) trait Context<T> {
    fn context(self, message: impl fmt::Display + Send + Sync + 'static) -> Result<T>;
}
impl<T, E: Into<ServiceError>> Context<T> for std::result::Result<T, E> {
    fn context(self, message: impl fmt::Display + Send + Sync + 'static) -> Result<T> {
        self.map_err(|error| error.into().context(message))
    }
}
impl<T> Context<T> for Option<T> {
    fn context(self, message: impl fmt::Display + Send + Sync + 'static) -> Result<T> {
        self.ok_or_else(|| ServiceError::message(ServiceErrorCode::Unknown, message.to_string()))
    }
}

#[macro_export]
macro_rules! service_bail {
    ($code:ident; $($arg:tt)*) => {
        return Err($crate::error::ServiceError::message($crate::error::ServiceErrorCode::$code, format!($($arg)*)))
    };
    ($($arg:tt)*) => { $crate::service_bail!(Unknown; $($arg)*) };
}
#[macro_export]
macro_rules! service_ensure {
    ($condition:expr, $code:ident; $($arg:tt)*) => {
        if !$condition { $crate::service_bail!($code; $($arg)*) }
    };
    ($condition:expr, $($arg:tt)*) => { $crate::service_ensure!($condition, InvalidInput; $($arg)*) };
}

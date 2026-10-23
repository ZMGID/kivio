//! Stable failure codes for model discovery without changing the model RPC shape.
use crate::HarnessError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CatalogFailureCode {
    Timeout,
    Failed,
    MissingExecutable,
    AuthRequired,
}
impl CatalogFailureCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Failed => "failed",
            Self::MissingExecutable => "missing_executable",
            Self::AuthRequired => "auth_required",
        }
    }
    pub fn allows_stale(self) -> bool {
        matches!(self, Self::Timeout | Self::Failed)
    }
}
impl std::fmt::Display for CatalogFailureCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
#[derive(Clone, Debug, thiserror::Error)]
#[error("model discovery {code}: {message}")]
pub struct CatalogFailure {
    pub code: CatalogFailureCode,
    pub message: String,
}
impl CatalogFailure {
    pub fn classify(error: &HarnessError) -> CatalogFailureCode {
        match error {
            HarnessError::Discovery(error) => return error.code,
            HarnessError::NotInstalled(_) => return CatalogFailureCode::MissingExecutable,
            HarnessError::Io(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return CatalogFailureCode::MissingExecutable;
            }
            HarnessError::Io(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                return CatalogFailureCode::Timeout;
            }
            _ => {}
        }
        let text = error.to_string().to_ascii_lowercase();
        if text.contains("enoent")
            || text.contains("harness binary not found")
            || text.contains("no such file or directory")
        {
            return CatalogFailureCode::MissingExecutable;
        }
        let auth_status = text
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|part| matches!(part, "401" | "403"));
        if auth_status
            || [
                "not logged in",
                "authentication",
                "unauthenticated",
                "not authenticated",
                "unauthorized",
                "auth_required",
                "api key",
            ]
            .iter()
            .any(|pattern| text.contains(pattern))
        {
            return CatalogFailureCode::AuthRequired;
        }
        if [
            "timeout",
            "timed out",
            "deadline has elapsed",
            "did not complete within",
        ]
        .iter()
        .any(|pattern| text.contains(pattern))
        {
            return CatalogFailureCode::Timeout;
        }
        CatalogFailureCode::Failed
    }
}
impl From<HarnessError> for CatalogFailure {
    fn from(error: HarnessError) -> Self {
        if let HarnessError::Discovery(failure) = error {
            return failure;
        }
        Self {
            code: Self::classify(&error),
            message: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_errors_have_conservative_stable_codes() {
        for (message, expected) in [
            ("not logged in", CatalogFailureCode::AuthRequired),
            ("Authentication failed", CatalogFailureCode::AuthRequired),
            ("unauthenticated", CatalogFailureCode::AuthRequired),
            ("invalid API key", CatalogFailureCode::AuthRequired),
            ("HTTP 401", CatalogFailureCode::AuthRequired),
            ("status: 403", CatalogFailureCode::AuthRequired),
            ("spawn ENOENT", CatalogFailureCode::MissingExecutable),
            ("operation timed out", CatalogFailureCode::Timeout),
            ("model v401 not found", CatalogFailureCode::Failed),
            ("rate limit 429", CatalogFailureCode::Failed),
        ] {
            assert_eq!(
                CatalogFailure::classify(&HarnessError::Protocol(message.into())),
                expected,
                "{message}"
            );
        }
        assert_eq!(
            CatalogFailure::classify(&HarnessError::NotInstalled("cli".into())),
            CatalogFailureCode::MissingExecutable
        );
        assert_eq!(
            CatalogFailure::classify(&std::io::Error::from(std::io::ErrorKind::NotFound).into()),
            CatalogFailureCode::MissingExecutable
        );
        let failure = CatalogFailure {
            code: CatalogFailureCode::Timeout,
            message: "custom deadline".into(),
        };
        assert_eq!(
            CatalogFailure::from(HarnessError::from(failure)).code,
            CatalogFailureCode::Timeout
        );
    }
}

//! Application identity shared by desktop hosts without environment or credential access.
//!
//! Storage, user-home resources and OAuth namespaces must be selected together.
//! In particular, a native preview must never inherit the production credential service.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppIdentity {
    Production,
    Development,
    NativePreview,
}

impl AppIdentity {
    pub const fn from_identifier(identifier: &str) -> Option<Self> {
        match identifier.as_bytes() {
            b"com.zmair.kivio" => Some(Self::Production),
            b"com.zmair.kivio.dev" => Some(Self::Development),
            b"com.zmair.kivio.gpui.preview" => Some(Self::NativePreview),
            _ => None,
        }
    }

    pub const fn identifier(self) -> &'static str {
        match self {
            Self::Production => "com.zmair.kivio",
            Self::Development => "com.zmair.kivio.dev",
            Self::NativePreview => "com.zmair.kivio.gpui.preview",
        }
    }

    pub const fn product_name(self) -> &'static str {
        match self {
            Self::Production => "Kivio Desktop",
            Self::Development => "Kivio Dev",
            Self::NativePreview => "Kivio Native",
        }
    }

    pub const fn user_home_name(self) -> &'static str {
        match self {
            Self::Production => ".kivio",
            Self::Development => ".kivio-dev",
            Self::NativePreview => ".kivio-gpui-preview",
        }
    }

    pub const fn oauth_service(self) -> &'static str {
        match self {
            Self::Production => "Kivio.ModelOAuth",
            Self::Development => "Kivio.Dev.ModelOAuth",
            Self::NativePreview => "Kivio.GpuiPreview.ModelOAuth",
        }
    }

    /// Both development hosts opt out of production defaults and legacy migration.
    pub const fn is_development(self) -> bool {
        !matches!(self, Self::Production)
    }
}

#[cfg(test)]
mod tests {
    use super::AppIdentity;

    #[test]
    fn supported_hosts_keep_data_home_and_credentials_in_separate_namespaces() {
        // These are persisted namespace contracts, not names generated from the implementation.
        let channels = [
            (
                "com.zmair.kivio",
                "Kivio Desktop",
                ".kivio",
                "Kivio.ModelOAuth",
                false,
            ),
            (
                "com.zmair.kivio.dev",
                "Kivio Dev",
                ".kivio-dev",
                "Kivio.Dev.ModelOAuth",
                true,
            ),
            (
                "com.zmair.kivio.gpui.preview",
                "Kivio Native",
                ".kivio-gpui-preview",
                "Kivio.GpuiPreview.ModelOAuth",
                true,
            ),
        ];
        for (identifier, product, home, credentials, development) in channels {
            let identity = AppIdentity::from_identifier(identifier).unwrap();
            assert_eq!(identity.identifier(), identifier);
            assert_eq!(identity.product_name(), product);
            assert_eq!(identity.user_home_name(), home);
            assert_eq!(identity.oauth_service(), credentials);
            assert_eq!(identity.is_development(), development);
        }
        // A misspelled or future identity cannot silently fall through to production.
        assert_eq!(
            AppIdentity::from_identifier("com.zmair.kivio.preview"),
            None
        );
        assert_eq!(AppIdentity::from_identifier(""), None);
    }
}

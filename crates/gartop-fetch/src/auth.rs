//! Exchange the personal API key for a short-lived bearer token, with an on-disk cache.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::client::{ApiError, Http};

/// Path of the token endpoint relative to the API base URL.
pub const TOKEN_PATH: &str = "/personalapitoken-service/personal-api/personal-api-key/token";

/// Renew the token when less than this many seconds of validity remain.
const RENEW_MARGIN_SECS: i64 = 3600;

/// A bearer token (a JWT) and its expiry in seconds since the Unix epoch.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Token {
    pub jwt: String,
    pub expires_at: i64,
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Token")
            .field("jwt", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl Token {
    /// Build a token from a raw JWT, reading `exp` from its payload (the signature is not checked).
    pub fn from_jwt(jwt: &str) -> Result<Self, ApiError> {
        #[derive(Deserialize)]
        struct Claims {
            exp: i64,
        }
        let jwt = jwt.trim().trim_matches('"').to_owned();
        let payload = jwt
            .split('.')
            .nth(1)
            .ok_or_else(|| ApiError::BadToken("not a JWT".into()))?;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload.trim_end_matches('='))
            .map_err(|e| ApiError::BadToken(format!("payload is not base64url: {e}")))?;
        let claims: Claims = serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::BadToken(format!("payload has no exp claim: {e}")))?;
        Ok(Self {
            jwt,
            expires_at: claims.exp,
        })
    }

    /// Whether the token is still good for at least the renewal margin.
    #[must_use]
    pub fn is_fresh(&self, now: i64) -> bool {
        self.expires_at - now > RENEW_MARGIN_SECS
    }
}

/// Provides bearer tokens, reusing a cached one while it is fresh.
#[derive(Debug)]
pub struct TokenProvider {
    api_key: String,
    cache: PathBuf,
    current: Option<Token>,
}

impl TokenProvider {
    #[must_use]
    pub fn new(api_key: String, cache: PathBuf) -> Self {
        let current = read_cache(&cache);
        Self {
            api_key,
            cache,
            current,
        }
    }

    /// A fresh token, requesting a new one if needed.
    pub fn token(&mut self, http: &Http) -> Result<&Token, ApiError> {
        let now = chrono::Utc::now().timestamp();
        if !self.current.as_ref().is_some_and(|t| t.is_fresh(now)) {
            self.renew(http)?;
        }
        Ok(self.current.as_ref().expect("token set by renew"))
    }

    /// Request a new token unconditionally (for example after a 401).
    pub fn renew(&mut self, http: &Http) -> Result<&Token, ApiError> {
        tracing::debug!("requesting a new access token");
        let body = http.post_api_key(TOKEN_PATH, &self.api_key)?;
        let token = Token::from_jwt(&body)?;
        if let Err(e) = write_cache(&self.cache, &token) {
            tracing::warn!("could not cache token at {}: {e}", self.cache.display());
        }
        Ok(self.current.insert(token))
    }
}

fn read_cache(path: &Path) -> Option<Token> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_cache(path: &Path, token: &Token) -> std::io::Result<()> {
    use std::io::Write as _;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut file = opts.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(serde_json::to_string(token)?.as_bytes())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Build an unsigned JWT with the given `exp` claim.
    pub fn fake_jwt(exp: i64) -> String {
        let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        format!(
            "{}.{}.sig",
            enc.encode(r#"{"alg":"none"}"#),
            enc.encode(format!(r#"{{"exp":{exp},"sub":"x"}}"#))
        )
    }

    #[test]
    fn decodes_exp() {
        let t = Token::from_jwt(&fake_jwt(1_700_000_000)).unwrap();
        assert_eq!(t.expires_at, 1_700_000_000);
        assert!(t.is_fresh(1_700_000_000 - 7200));
        assert!(!t.is_fresh(1_700_000_000 - 60));
    }

    #[test]
    fn rejects_garbage() {
        assert!(Token::from_jwt("hello").is_err());
        assert!(Token::from_jwt("a.!!!.c").is_err());
    }

    #[test]
    fn cache_roundtrip_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/token");
        let t = Token::from_jwt(&fake_jwt(42)).unwrap();
        write_cache(&path, &t).unwrap();
        assert_eq!(read_cache(&path), Some(t));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}

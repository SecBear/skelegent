//! OMP (Oh My Pi) authentication provider.
//!
//! Reads Anthropic OAuth credentials from OMP's `agent.db` SQLite database.
//! Handles token expiry checking (5-minute buffer) and automatic refresh
//! via Anthropic's OAuth token endpoint.

use async_trait::async_trait;
use skg_auth::{AuthError, AuthProvider, AuthRequest, AuthToken};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{debug, warn};

/// Buffer before actual expiry at which we consider a token stale.
const EXPIRY_BUFFER: Duration = Duration::from_secs(5 * 60);

/// Anthropic OAuth token refresh endpoint.
const REFRESH_URL: &str = "https://api.anthropic.com/v1/oauth/token";

/// JSON shape stored in `auth_credentials.data`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
struct OmpCredential {
    access: String,
    refresh: String,
    /// Milliseconds since Unix epoch.
    expires: u64,
}

impl OmpCredential {
    fn expires_at(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_millis(self.expires)
    }

    /// True if the token expires within `EXPIRY_BUFFER` of now.
    fn is_expiring_soon(&self) -> bool {
        let deadline = SystemTime::now() + EXPIRY_BUFFER;
        self.expires_at() < deadline
    }
}

/// Cached token held behind RwLock.
#[derive(Debug, Clone)]
struct CachedToken {
    access: String,
    expires_at: SystemTime,
}

impl CachedToken {
    fn from_credential(cred: &OmpCredential) -> Self {
        Self {
            access: cred.access.clone(),
            expires_at: cred.expires_at(),
        }
    }

    fn is_valid(&self) -> bool {
        let deadline = SystemTime::now() + EXPIRY_BUFFER;
        self.expires_at > deadline
    }

    fn to_auth_token(&self) -> AuthToken {
        AuthToken::new(
            self.access.as_bytes().to_vec(),
            Some(self.expires_at),
        )
    }
}

/// Authentication provider that reads Anthropic OAuth tokens from OMP's
/// `agent.db` SQLite database and handles automatic refresh.
pub struct OmpAuthProvider {
    db_path: PathBuf,
    cache: Arc<RwLock<Option<CachedToken>>>,
    http: reqwest::Client,
}

impl OmpAuthProvider {
    /// Create a provider using the default OMP database path
    /// (`~/.omp/agent/agent.db`).
    pub fn new() -> Result<Self, AuthError> {
        let home = std::env::var("HOME")
            .map_err(|_| AuthError::BackendError("HOME not set".into()))?;
        let db_path = PathBuf::from(home).join(".omp/agent/agent.db");
        Ok(Self::with_db_path(db_path))
    }

    /// Create a provider with an explicit database path.
    pub fn with_db_path(db_path: PathBuf) -> Self {
        Self {
            db_path,
            cache: Arc::new(RwLock::new(None)),
            http: reqwest::Client::new(),
        }
    }

    /// Read the most recent Anthropic OAuth credential from the database.
    fn read_from_db(&self) -> Result<OmpCredential, AuthError> {
        if !self.db_path.exists() {
            return Err(AuthError::BackendError(format!(
                "OMP database not found at {} — is OMP installed?",
                self.db_path.display()
            )));
        }

        let conn = rusqlite::Connection::open_with_flags(
            &self.db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| {
            AuthError::BackendError(format!(
                "failed to open {}: {e}",
                self.db_path.display()
            ))
        })?;

        let data: String = conn
            .query_row(
                "SELECT data FROM auth_credentials \
                 WHERE provider = 'anthropic' AND credential_type = 'oauth' \
                 AND disabled_cause IS NULL \
                 ORDER BY updated_at DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .map_err(|e| {
                AuthError::AuthFailed(format!(
                    "no anthropic oauth credential in agent.db: {e}"
                ))
            })?;

        serde_json::from_str::<OmpCredential>(&data).map_err(|e| {
            AuthError::BackendError(format!("malformed credential JSON: {e}"))
        })
    }

    /// Refresh the OAuth token via Anthropic's token endpoint.
    async fn refresh_token(
        &self,
        refresh: &str,
    ) -> Result<OmpCredential, AuthError> {
        debug!("refreshing Anthropic OAuth token");

        let resp = self
            .http
            .post(REFRESH_URL)
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh),
            ])
            .send()
            .await
            .map_err(|e| {
                AuthError::BackendError(format!("token refresh request failed: {e}"))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AuthError::AuthFailed(format!(
                "token refresh returned {status}: {body}"
            )));
        }

        #[derive(serde::Deserialize)]
        struct RefreshResponse {
            access_token: String,
            refresh_token: String,
            expires_in: u64,
        }

        let parsed: RefreshResponse = resp.json().await.map_err(|e| {
            AuthError::BackendError(format!("failed to parse refresh response: {e}"))
        })?;

        let expires_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
            + parsed.expires_in * 1000;

        Ok(OmpCredential {
            access: parsed.access_token,
            refresh: parsed.refresh_token,
            expires: expires_ms,
        })
    }

    /// Persist a refreshed credential back to the database.
    fn update_db(&self, cred: &OmpCredential) -> Result<(), AuthError> {
        let conn = rusqlite::Connection::open(&self.db_path).map_err(|e| {
            AuthError::BackendError(format!(
                "failed to open {} for write: {e}",
                self.db_path.display()
            ))
        })?;

        let json = serde_json::to_string(cred).map_err(|e| {
            AuthError::BackendError(format!("failed to serialize credential: {e}"))
        })?;

        conn.execute(
            "UPDATE auth_credentials SET data = ?1, updated_at = unixepoch() \
             WHERE id = (
               SELECT id FROM auth_credentials \
               WHERE provider = 'anthropic' AND credential_type = 'oauth' \
               ORDER BY updated_at DESC LIMIT 1
             )",
            [&json],
        )
        .map_err(|e| {
            AuthError::BackendError(format!("failed to update credential: {e}"))
        })?;

        debug!("updated anthropic oauth credential in agent.db");
        Ok(())
    }
}

impl std::fmt::Debug for OmpAuthProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OmpAuthProvider")
            .field("db_path", &self.db_path)
            .finish()
    }
}

#[async_trait]
impl AuthProvider for OmpAuthProvider {
    async fn provide(&self, _request: &AuthRequest) -> Result<AuthToken, AuthError> {
        // 1. Check cache.
        {
            let cached = self.cache.read().await;
            if let Some(ref token) = *cached {
                if token.is_valid() {
                    debug!("using cached OMP token");
                    return Ok(token.to_auth_token());
                }
            }
        }

        // 2. Acquire write lock — only one caller refreshes.
        let mut cached = self.cache.write().await;

        // Double-check: another task may have refreshed while we waited.
        if let Some(ref token) = *cached {
            if token.is_valid() {
                return Ok(token.to_auth_token());
            }
        }

        // 3. Read from DB.
        let cred = self.read_from_db()?;

        if !cred.is_expiring_soon() {
            let ct = CachedToken::from_credential(&cred);
            let auth_token = ct.to_auth_token();
            *cached = Some(ct);
            return Ok(auth_token);
        }

        // 4. Token expired or expiring soon — refresh.
        debug!("OMP token expired or expiring soon, attempting refresh");
        match self.refresh_token(&cred.refresh).await {
            Ok(new_cred) => {
                // Persist to DB (best-effort — don't fail the auth).
                if let Err(e) = self.update_db(&new_cred) {
                    warn!("failed to persist refreshed token to DB: {e}");
                }
                let ct = CachedToken::from_credential(&new_cred);
                let auth_token = ct.to_auth_token();
                *cached = Some(ct);
                Ok(auth_token)
            }
            Err(e) => {
                warn!("token refresh failed: {e}");
                Err(AuthError::AuthFailed(format!(
                    "token expired and refresh failed: {e}"
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_deserialization() {
        let json = r#"{"access":"sk-ant-oat-abc123","refresh":"sk-ant-ort-xyz789","expires":1773175991850}"#;
        let cred: OmpCredential = serde_json::from_str(json).unwrap();
        assert_eq!(cred.access, "sk-ant-oat-abc123");
        assert_eq!(cred.refresh, "sk-ant-ort-xyz789");
        assert_eq!(cred.expires, 1773175991850);
    }

    #[test]
    fn credential_expires_at_conversion() {
        let cred = OmpCredential {
            access: String::new(),
            refresh: String::new(),
            expires: 1_773_175_991_850, // ms
        };
        let expected = UNIX_EPOCH + Duration::from_millis(1_773_175_991_850);
        assert_eq!(cred.expires_at(), expected);
    }

    #[test]
    fn expiring_soon_with_future_token() {
        let far_future_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 3_600_000; // 1 hour from now

        let cred = OmpCredential {
            access: "a".into(),
            refresh: "r".into(),
            expires: far_future_ms,
        };
        assert!(!cred.is_expiring_soon());
    }

    #[test]
    fn expiring_soon_with_past_token() {
        let past_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            - 60_000; // 1 minute ago

        let cred = OmpCredential {
            access: "a".into(),
            refresh: "r".into(),
            expires: past_ms,
        };
        assert!(cred.is_expiring_soon());
    }

    #[test]
    fn expiring_soon_within_buffer() {
        // 4 minutes from now — within the 5-minute buffer.
        let soon_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 4 * 60 * 1000;

        let cred = OmpCredential {
            access: "a".into(),
            refresh: "r".into(),
            expires: soon_ms,
        };
        assert!(cred.is_expiring_soon());
    }

    #[test]
    fn missing_db_returns_backend_error() {
        let provider = OmpAuthProvider::with_db_path(
            PathBuf::from("/tmp/nonexistent-omp-test/agent.db"),
        );
        let err = provider.read_from_db().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("not found"), "unexpected error: {msg}");
    }

    #[tokio::test]
    async fn provide_with_missing_db() {
        let provider = OmpAuthProvider::with_db_path(
            PathBuf::from("/tmp/nonexistent-omp-test/agent.db"),
        );
        let result = provider.provide(&AuthRequest::new()).await;
        assert!(result.is_err());
    }

    #[test]
    fn cached_token_validity() {
        let far_future = SystemTime::now() + Duration::from_secs(3600);
        let ct = CachedToken {
            access: "tok".into(),
            expires_at: far_future,
        };
        assert!(ct.is_valid());

        let past = SystemTime::now() - Duration::from_secs(60);
        let ct_expired = CachedToken {
            access: "tok".into(),
            expires_at: past,
        };
        assert!(!ct_expired.is_valid());
    }
}

use serde::Deserialize;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Claude Code ist nicht eingeloggt (Credentials-Datei fehlt: {0})")]
    NotLoggedIn(PathBuf),
    #[error("Credentials-Datei konnte nicht gelesen werden: {0}")]
    Io(#[from] std::io::Error),
    #[error("Credentials-Datei hat ein unerwartetes Format: {0}")]
    Parse(#[from] serde_json::Error),
    #[error("Kein Home-Verzeichnis gefunden")]
    NoHomeDir,
}

#[derive(Debug, Deserialize, Clone)]
struct CredentialsFile {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: OauthSection,
}

#[derive(Debug, Deserialize, Clone)]
struct OauthSection {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "expiresAt")]
    expires_at_ms: i64,
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
    #[serde(rename = "rateLimitTier")]
    rate_limit_tier: Option<String>,
}

/// Aktive Claude-Login-Session, gelesen aus der lokalen Credentials-Datei,
/// die die offizielle Claude Code CLI beim Login/laufenden Betrieb pflegt.
/// Dieses Modul schreibt diese Datei nie und macht keinen eigenen OAuth-Handshake -
/// es liest ausschliesslich, was Claude Code selbst dort bereits abgelegt hat.
#[derive(Debug, Clone)]
pub struct Session {
    pub access_token: String,
    pub expires_at_ms: i64,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
}

impl Session {
    pub fn is_expired(&self) -> bool {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        now_ms >= self.expires_at_ms
    }
}

/// Pfad zur Claude Code Credentials-Datei. Identisch auf Windows/macOS/Linux,
/// da Claude Code ueberall im Home-Verzeichnis unter .claude/ ablegt.
pub fn credentials_path() -> Result<PathBuf, AuthError> {
    let home = dirs::home_dir().ok_or(AuthError::NoHomeDir)?;
    Ok(home.join(".claude").join(".credentials.json"))
}

pub fn load_session() -> Result<Session, AuthError> {
    load_session_from(&credentials_path()?)
}

pub fn load_session_from(path: &std::path::Path) -> Result<Session, AuthError> {
    if !path.exists() {
        return Err(AuthError::NotLoggedIn(path.to_path_buf()));
    }
    let raw = std::fs::read_to_string(path)?;
    let parsed: CredentialsFile = serde_json::from_str(&raw)?;
    Ok(Session {
        access_token: parsed.claude_ai_oauth.access_token,
        expires_at_ms: parsed.claude_ai_oauth.expires_at_ms,
        subscription_type: parsed.claude_ai_oauth.subscription_type,
        rate_limit_tier: parsed.claude_ai_oauth.rate_limit_tier,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_fixture(dir: &std::path::Path, expires_at_ms: i64) -> PathBuf {
        let path = dir.join(".credentials.json");
        let body = serde_json::json!({
            "claudeAiOauth": {
                "accessToken": "test-token-abc",
                "refreshToken": "test-refresh-abc",
                "expiresAt": expires_at_ms,
                "refreshTokenExpiresAt": expires_at_ms + 1_000_000,
                "scopes": ["user:inference"],
                "subscriptionType": "pro",
                "rateLimitTier": "default_claude_ai"
            }
        });
        std::fs::write(&path, serde_json::to_string_pretty(&body).unwrap()).unwrap();
        path
    }

    #[test]
    fn loads_valid_credentials_file() {
        let tmp = std::env::temp_dir().join(format!("uc-test-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let future_ms = (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis()
            as i64)
            + 3_600_000;
        let path = write_fixture(&tmp, future_ms);

        let session = load_session_from(&path).expect("should parse fixture");
        assert_eq!(session.access_token, "test-token-abc");
        assert_eq!(session.subscription_type.as_deref(), Some("pro"));
        assert!(!session.is_expired());

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn detects_expired_session() {
        let tmp = std::env::temp_dir().join(format!("uc-test-exp-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let past_ms = (SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64)
            - 1_000;
        let path = write_fixture(&tmp, past_ms);

        let session = load_session_from(&path).expect("should parse fixture");
        assert!(session.is_expired());

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn missing_file_reports_not_logged_in() {
        let missing = std::env::temp_dir().join("uc-test-does-not-exist-xyz.json");
        let err = load_session_from(&missing).unwrap_err();
        assert!(matches!(err, AuthError::NotLoggedIn(_)));
    }
}

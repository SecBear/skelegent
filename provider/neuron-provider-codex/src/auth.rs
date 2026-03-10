//! JWT account ID extraction for Codex OAuth tokens.

use base64::Engine;

/// Extract the ChatGPT account ID from a Codex JWT access token.
///
/// The JWT payload contains a claim at `https://api.openai.com/auth`
/// with a `chatgpt_account_id` field. Returns `None` if the token
/// is not a valid JWT or lacks the expected claim.
pub fn extract_account_id(access_token: &str) -> Option<String> {
    let parts: Vec<&str> = access_token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&payload).ok()?;
    claims
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(|id| id.as_str())
        .map(String::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn make_jwt(claims: &serde_json::Value) -> String {
        let header =
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"RS256","kid":"x"}"#);
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(claims).unwrap());
        let sig = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"fake-sig");
        format!("{header}.{payload}.{sig}")
    }

    #[test]
    fn extracts_account_id() {
        let claims = serde_json::json!({
            "https://api.openai.com/auth": {
                "chatgpt_account_id": "acct_abc123"
            }
        });
        let jwt = make_jwt(&claims);
        assert_eq!(extract_account_id(&jwt), Some("acct_abc123".into()));
    }

    #[test]
    fn returns_none_for_missing_claim() {
        let claims = serde_json::json!({"sub": "user"});
        let jwt = make_jwt(&claims);
        assert_eq!(extract_account_id(&jwt), None);
    }

    #[test]
    fn returns_none_for_non_jwt() {
        assert_eq!(extract_account_id("not-a-jwt"), None);
        assert_eq!(extract_account_id("sk-ant-api123"), None);
    }
}

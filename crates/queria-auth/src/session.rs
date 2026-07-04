use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use rand::RngCore;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionClaims {
    pub user_id: String,
    pub email: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuedSessionToken {
    pub raw_token: String,
    pub token_prefix: String,
    pub token_hash: String,
}

#[derive(Clone, Debug, Default)]
pub struct SessionIssuer;

impl SessionIssuer {
    #[must_use]
    pub fn issue_session_token(&self) -> IssuedSessionToken {
        let mut random_bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut random_bytes);
        let raw_token = format!("qsess_{}", URL_SAFE_NO_PAD.encode(random_bytes));
        let token_prefix = raw_token.chars().take(16).collect::<String>();
        let token_hash = Self::hash_session_token(&raw_token);

        IssuedSessionToken {
            raw_token,
            token_prefix,
            token_hash,
        }
    }

    #[must_use]
    pub fn hash_session_token(raw_token: &str) -> String {
        let digest = Sha256::digest(raw_token.as_bytes());
        format!("sha256:{}", URL_SAFE_NO_PAD.encode(digest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issued_session_token_uses_hashable_opaque_value() {
        let issued = SessionIssuer.issue_session_token();

        assert!(issued.raw_token.starts_with("qsess_"));
        assert_eq!(issued.token_prefix.len(), 16);
        assert_ne!(issued.token_hash, issued.raw_token);
        assert_eq!(
            SessionIssuer::hash_session_token(&issued.raw_token),
            issued.token_hash
        );
    }
}

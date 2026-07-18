use crate::QueriaResult;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

/// Opaque invite token issued once to the creator; only hash+prefix are stored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuedOrgInviteToken {
    pub raw_token: String,
    pub token_prefix: String,
    pub token_hash: String,
}

#[derive(Clone, Debug, Default)]
pub struct OrgInviteTokenIssuer;

impl OrgInviteTokenIssuer {
    pub fn issue(&self) -> QueriaResult<IssuedOrgInviteToken> {
        let mut random_bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut random_bytes);
        let raw_token = format!("qinv_{}", URL_SAFE_NO_PAD.encode(random_bytes));
        let token_prefix = raw_token.chars().take(14).collect::<String>();
        let token_hash = Self::hash_token(&raw_token);

        Ok(IssuedOrgInviteToken {
            raw_token,
            token_prefix,
            token_hash,
        })
    }

    /// Same storage format as agent tokens: `sha256:` + URL-safe base64 digest.
    #[must_use]
    pub fn hash_token(raw_token: &str) -> String {
        let digest = Sha256::digest(raw_token.as_bytes());
        format!("sha256:{}", URL_SAFE_NO_PAD.encode(digest))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_token_hash_does_not_store_raw_token() {
        let issued = OrgInviteTokenIssuer
            .issue()
            .expect("invite token should issue");

        assert!(issued.raw_token.starts_with("qinv_"));
        assert_eq!(issued.token_prefix.len(), 14);
        assert_ne!(issued.token_hash, issued.raw_token);
        assert!(!issued.token_hash.contains(&issued.raw_token));
        assert_eq!(
            OrgInviteTokenIssuer::hash_token(&issued.raw_token),
            issued.token_hash
        );
    }
}

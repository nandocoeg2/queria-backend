use crate::permissions::{AgentTokenPermissions, AgentToolPermission};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use queria_core::ids::AgentTokenId;
use queria_core::{QueriaError, QueriaResult};
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IssuedAgentToken {
    pub id: AgentTokenId,
    pub raw_token: String,
    pub token_prefix: String,
    pub token_hash: String,
    pub expires_at: Option<DateTime<Utc>>,
    pub permissions: AgentTokenPermissions,
}

#[derive(Clone, Debug, Default)]
pub struct AgentTokenIssuer;

impl AgentTokenIssuer {
    pub fn issue(
        &self,
        permissions: AgentTokenPermissions,
        expires_at: Option<DateTime<Utc>>,
    ) -> QueriaResult<IssuedAgentToken> {
        if permissions.tools.is_empty() {
            return Err(QueriaError::Validation(
                "agent token must allow at least one tool".to_owned(),
            ));
        }

        let mut random_bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut random_bytes);
        let raw_token = format!("qria_{}", URL_SAFE_NO_PAD.encode(random_bytes));
        let token_prefix = raw_token.chars().take(14).collect::<String>();
        let token_hash = Self::hash_token(&raw_token);

        Ok(IssuedAgentToken {
            id: AgentTokenId::new(),
            raw_token,
            token_prefix,
            token_hash,
            expires_at,
            permissions,
        })
    }

    pub fn hash_token(raw_token: &str) -> String {
        let digest = Sha256::digest(raw_token.as_bytes());
        format!("sha256:{}", URL_SAFE_NO_PAD.encode(digest))
    }
}

pub fn default_agent_tools() -> Vec<AgentToolPermission> {
    vec![
        AgentToolPermission::RetrieveContext,
        AgentToolPermission::SearchKnowledge,
        AgentToolPermission::ProposeMemory,
        AgentToolPermission::ListProjects,
        AgentToolPermission::GetSource,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issued_token_persists_prefix_and_hash_without_raw_token_reuse() {
        let permissions = AgentTokenPermissions {
            allow_global_knowledge: true,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: default_agent_tools(),
        };

        let issued = AgentTokenIssuer
            .issue(permissions, None)
            .expect("token should issue");

        assert!(issued.raw_token.starts_with("qria_"));
        assert_eq!(issued.token_prefix.len(), 14);
        assert_ne!(issued.token_hash, issued.raw_token);
        assert_eq!(
            AgentTokenIssuer::hash_token(&issued.raw_token),
            issued.token_hash
        );
    }
}

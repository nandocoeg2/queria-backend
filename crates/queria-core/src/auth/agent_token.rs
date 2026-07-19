use super::permissions::{AgentTokenPermissions, AgentToolPermission};
use crate::ids::AgentTokenId;
use crate::{QueriaError, QueriaResult};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
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

/// Legacy default tool set: propose-only write path (no IndexMemory).
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

    #[test]
    fn default_agent_tools_remains_propose_only_without_index_memory() {
        let tools = default_agent_tools();
        assert!(
            !tools.contains(&AgentToolPermission::IndexMemory),
            "legacy default tokens must not gain index_memory"
        );
        assert!(
            !tools.contains(&AgentToolPermission::IndexLocal),
            "legacy default tokens must not gain index_local"
        );
        assert!(
            !tools.contains(&AgentToolPermission::ManageNeedsReview),
            "legacy default tokens must not gain manage_needs_review"
        );
        assert!(tools.contains(&AgentToolPermission::ProposeMemory));
        assert!(tools.contains(&AgentToolPermission::RetrieveContext));
        assert!(tools.contains(&AgentToolPermission::SearchKnowledge));
        assert!(tools.contains(&AgentToolPermission::ListProjects));
        assert!(tools.contains(&AgentToolPermission::GetSource));
    }

    #[test]
    fn issuer_accepts_manage_needs_review_tool() {
        let permissions = AgentTokenPermissions {
            allow_global_knowledge: false,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: vec![
                AgentToolPermission::RetrieveContext,
                AgentToolPermission::ManageNeedsReview,
            ],
        };
        let issued = AgentTokenIssuer
            .issue(permissions, None)
            .expect("ManageNeedsReview token should issue");
        assert!(
            issued
                .permissions
                .can_call(&AgentToolPermission::ManageNeedsReview)
        );
        assert!(
            !issued
                .permissions
                .can_call(&AgentToolPermission::IndexMemory)
        );
    }

    #[test]
    fn issuer_accepts_index_local_tool() {
        let permissions = AgentTokenPermissions {
            allow_global_knowledge: false,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: vec![
                AgentToolPermission::RetrieveContext,
                AgentToolPermission::IndexLocal,
            ],
        };
        let issued = AgentTokenIssuer
            .issue(permissions, None)
            .expect("IndexLocal token should issue");
        assert!(
            issued
                .permissions
                .can_call(&AgentToolPermission::IndexLocal)
        );
        assert!(
            !issued
                .permissions
                .can_call(&AgentToolPermission::IndexMemory)
        );
    }

    #[test]
    fn issuer_accepts_index_memory_with_propose_memory() {
        let permissions = AgentTokenPermissions {
            allow_global_knowledge: false,
            project_slugs: vec!["fjulian-me".to_owned()],
            tools: vec![
                AgentToolPermission::RetrieveContext,
                AgentToolPermission::ProposeMemory,
                AgentToolPermission::IndexMemory,
            ],
        };
        let issued = AgentTokenIssuer
            .issue(permissions.clone(), None)
            .expect("IndexMemory + ProposeMemory token should issue");
        assert!(
            issued
                .permissions
                .can_call(&AgentToolPermission::IndexMemory)
        );
        assert!(
            issued
                .permissions
                .can_call(&AgentToolPermission::ProposeMemory)
        );
    }
}

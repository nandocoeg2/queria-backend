use crate::ids::{ChunkId, ProjectId, SourceDocumentId};
use crate::model::KnowledgeScope;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RetrieveContextRequest {
    pub project_id: ProjectId,
    pub query: String,
    pub include_global: bool,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrieveContextResponse {
    pub project_id: ProjectId,
    pub query: String,
    pub items: Vec<RetrievedContextItem>,
    pub generated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievedContextItem {
    pub chunk_id: ChunkId,
    pub source_document_id: SourceDocumentId,
    pub scope: KnowledgeScope,
    pub title: String,
    pub body: String,
    pub citation: Citation,
    pub score: f32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Citation {
    pub source_uri: String,
    pub source_path: Option<String>,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
}

impl RetrieveContextRequest {
    pub fn validate(&self) -> crate::QueriaResult<()> {
        let query = self.query.trim();
        if query.is_empty() {
            return Err(crate::QueriaError::Validation(
                "query must not be blank".to_owned(),
            ));
        }

        if query.len() > 512 {
            return Err(crate::QueriaError::Validation(
                "query must be at most 512 bytes".to_owned(),
            ));
        }

        if !(1..=20).contains(&self.limit) {
            return Err(crate::QueriaError::Validation(
                "limit must be between 1 and 20".to_owned(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retrieve_context_request_rejects_blank_query() {
        let request = RetrieveContextRequest {
            project_id: ProjectId::new(),
            query: "  ".to_owned(),
            include_global: true,
            limit: 5,
        };

        assert!(request.validate().is_err());
    }

    #[test]
    fn retrieve_context_request_accepts_bounded_query() {
        let request = RetrieveContextRequest {
            project_id: ProjectId::new(),
            query: "how to deploy fjulian-me".to_owned(),
            include_global: true,
            limit: 8,
        };

        request.validate().expect("bounded query should be valid");
    }
}

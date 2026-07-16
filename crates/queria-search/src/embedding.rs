use async_trait::async_trait;
use queria_core::ids::ChunkId;
use queria_core::model::KnowledgeScope;
use queria_core::{QueriaError, QueriaResult};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq)]
pub struct EmbeddingDocument {
    pub chunk_id: ChunkId,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EmbeddingVector {
    values: Vec<f32>,
}

impl EmbeddingVector {
    pub fn new(values: Vec<f32>, expected_dimension: usize) -> QueriaResult<Self> {
        if values.len() != expected_dimension || values.iter().any(|value| !value.is_finite()) {
            return Err(QueriaError::Validation(format!(
                "embedding vector dimension must be {expected_dimension} with finite values"
            )));
        }
        Ok(Self { values })
    }

    #[must_use]
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    #[must_use]
    pub fn into_values(self) -> Vec<f32> {
        self.values
    }
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed_documents(
        &self,
        inputs: &[EmbeddingDocument],
    ) -> QueriaResult<Vec<EmbeddingVector>>;
    async fn embed_query(&self, query: &str) -> QueriaResult<EmbeddingVector>;
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VectorPayload {
    pub organization_id: Uuid,
    pub project_id: Option<Uuid>,
    pub scope: KnowledgeScope,
    pub embedding_profile_version: String,
    pub is_active: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VectorPoint {
    pub id: Uuid,
    pub vector: EmbeddingVector,
    pub payload: VectorPayload,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VectorSearchRequest {
    pub organization_id: Uuid,
    pub project_id: Uuid,
    pub include_global: bool,
    pub embedding_profile_version: String,
    pub vector: EmbeddingVector,
    pub limit: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VectorCandidate {
    pub chunk_id: ChunkId,
    pub score: f32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VectorIndexHealth {
    pub collection: String,
    pub points_count: u64,
}

#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait VectorIndex: Send + Sync {
    async fn ensure_collection(&self) -> QueriaResult<()>;
    async fn upsert(&self, points: &[VectorPoint]) -> QueriaResult<()>;
    async fn search(&self, request: VectorSearchRequest) -> QueriaResult<Vec<VectorCandidate>>;
    async fn delete(&self, point_ids: &[Uuid]) -> QueriaResult<()>;
    async fn health(&self) -> QueriaResult<VectorIndexHealth>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_vector_requires_configured_dimension() {
        let error = EmbeddingVector::new(vec![0.1, 0.2], 1024)
            .expect_err("mismatched dimensions must fail");

        assert!(error.to_string().contains("dimension"));
    }

    #[test]
    fn embedding_vector_rejects_non_finite_values() {
        let error = EmbeddingVector::new(vec![f32::NAN], 1)
            .expect_err("non-finite embedding values must fail");

        assert!(error.to_string().contains("finite"));
    }
}

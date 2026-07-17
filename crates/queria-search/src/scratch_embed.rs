//! Synchronous Voyage embed + Qdrant upsert for project-scoped scratch (IMP-13).

use crate::embedding::{
    EmbeddingDocument, EmbeddingProvider, VectorIndex, VectorPayload, VectorPoint,
};
use crate::qdrant::{QdrantClient, QdrantConfig};
use crate::voyage::VoyageClient;
use queria_core::ids::ChunkId;
use queria_core::model::KnowledgeScope;
use queria_core::scratch_content_hash;
use queria_core::{AppConfig, QueriaError, QueriaResult};
use queria_db::embedding::{
    EmbeddingChunkRecord, canonical_embedding_text, embedding_content_hash,
};
use queria_db::repositories::{
    AuthenticatedAgentToken, IndexMemoryParams, IndexMemoryResult, IndexedMemoryRecord,
    MarkScratchChunkReadyParams, PgProjectRepository,
};
use std::time::Duration;

/// Build Voyage + Qdrant clients from app config for the sync index_memory path.
pub fn build_embed_clients(
    config: &AppConfig,
) -> QueriaResult<(VoyageClient, QdrantClient, EmbedProfile)> {
    let dimension = usize::try_from(config.embedding.dimension)
        .map_err(|_| QueriaError::Config("embedding dimension is invalid".to_owned()))?;
    let voyage = VoyageClient::new(
        config.embedding.voyage_api_key.clone(),
        config.embedding.model.clone(),
        dimension,
        Duration::from_secs(config.embedding.timeout_seconds),
        config.embedding.max_retries,
    )?;
    let qdrant = QdrantClient::new(QdrantConfig {
        url: config.qdrant.url.clone(),
        api_key: config.qdrant.api_key.clone(),
        collection: config.qdrant.collection.clone(),
        vector_name: config.qdrant.vector_name.clone(),
        dimension,
    })?;
    let profile = EmbedProfile {
        provider: "voyage".to_owned(),
        model: config.embedding.model.clone(),
        dimension,
        profile_version: config.embedding.profile_version.clone(),
    };
    Ok((voyage, qdrant, profile))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmbedProfile {
    pub provider: String,
    pub model: String,
    pub dimension: usize,
    pub profile_version: String,
}

/// Full index_memory path: insert scratch (or idempotent hit) then sync embed + Qdrant.
///
/// On embed/Qdrant failure for a newly created row: deletes the scratch item so no
/// ready-searchable orphan remains (VAL-DL-032/033). Idempotent hits skip re-embed
/// when chunk is already ready.
pub async fn index_memory_with_sync_embed<E, V>(
    repository: &PgProjectRepository,
    agent: &AuthenticatedAgentToken,
    params: IndexMemoryParams,
    provider: &E,
    index: &V,
    profile: &EmbedProfile,
) -> QueriaResult<IndexMemoryResult>
where
    E: EmbeddingProvider,
    V: VectorIndex,
{
    let record = repository.index_memory_for_agent(agent, params).await?;

    if !record.created {
        // Idempotent: do not re-embed; return existing ids.
        return Ok(IndexMemoryResult {
            knowledge_item_id: record.knowledge_item_id,
            chunk_id: record.chunk_id,
            project_id: record.project_id,
            status: record.status,
            scope: record.scope,
            title: record.title,
            content_hash: record.content_hash,
            created: false,
            idempotent: true,
        });
    }

    match embed_and_upsert_scratch(provider, index, profile, &record).await {
        Ok(embedding_hash) => {
            let dimension = i32::try_from(profile.dimension).map_err(|_| {
                QueriaError::Config("embedding dimension exceeds database range".to_owned())
            })?;
            if let Err(error) = repository
                .mark_scratch_chunk_ready(&MarkScratchChunkReadyParams {
                    chunk_id: record.chunk_id,
                    qdrant_point_id: record.chunk_id,
                    embedding_content_hash: embedding_hash,
                    provider: profile.provider.clone(),
                    model: profile.model.clone(),
                    dimension,
                    profile_version: profile.profile_version.clone(),
                })
                .await
            {
                // Point may exist in Qdrant; best-effort delete then roll back PG.
                let _ = index.delete(&[record.chunk_id]).await;
                let _ = repository
                    .delete_scratch_knowledge_item(record.knowledge_item_id, record.organization_id)
                    .await;
                return Err(error);
            }
            Ok(IndexMemoryResult {
                knowledge_item_id: record.knowledge_item_id,
                chunk_id: record.chunk_id,
                project_id: record.project_id,
                status: record.status,
                scope: record.scope,
                title: record.title,
                content_hash: record.content_hash,
                created: true,
                idempotent: false,
            })
        }
        Err(error) => {
            let _ = index.delete(&[record.chunk_id]).await;
            let _ = repository
                .delete_scratch_knowledge_item(record.knowledge_item_id, record.organization_id)
                .await;
            Err(error)
        }
    }
}

async fn embed_and_upsert_scratch<E, V>(
    provider: &E,
    index: &V,
    profile: &EmbedProfile,
    record: &IndexedMemoryRecord,
) -> QueriaResult<String>
where
    E: EmbeddingProvider,
    V: VectorIndex,
{
    let chunk = EmbeddingChunkRecord {
        chunk_id: record.chunk_id,
        organization_id: record.organization_id,
        project_id: Some(record.project_id),
        scope: KnowledgeScope::Project,
        title: record.title.clone(),
        source_path: format!("scratch/{}", record.knowledge_item_id),
        body: record.body.clone(),
        content_hash: record.content_hash.clone(),
        qdrant_point_id: None,
    };
    let text = canonical_embedding_text(&chunk);
    let documents = [EmbeddingDocument {
        chunk_id: ChunkId::from_uuid(record.chunk_id),
        text,
    }];
    let mut vectors = provider.embed_documents(&documents).await?;
    if vectors.len() != 1 {
        return Err(QueriaError::Infrastructure(
            "embedding response count did not match scratch document".to_owned(),
        ));
    }
    let vector = vectors.pop().expect("len checked");
    index.ensure_collection().await?;
    index
        .upsert(&[VectorPoint {
            id: record.chunk_id,
            vector,
            payload: VectorPayload {
                organization_id: record.organization_id,
                project_id: Some(record.project_id),
                scope: KnowledgeScope::Project,
                embedding_profile_version: profile.profile_version.clone(),
                is_active: true,
            },
        }])
        .await?;
    Ok(embedding_content_hash(
        &chunk,
        &profile.provider,
        &profile.model,
        profile.dimension,
        &profile.profile_version,
    ))
}

/// Helper for callers that have body text only (hash computation).
#[must_use]
pub fn content_hash_for_body(body: &str) -> String {
    scratch_content_hash(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_for_body_matches_core_helper() {
        assert_eq!(
            content_hash_for_body("  a  b "),
            scratch_content_hash("  a  b ")
        );
    }

    #[test]
    fn embed_profile_carries_dimension() {
        let profile = EmbedProfile {
            provider: "voyage".to_owned(),
            model: "voyage-4".to_owned(),
            dimension: 1024,
            profile_version: "voyage-4-1024-v1".to_owned(),
        };
        assert_eq!(profile.dimension, 1024);
    }
}

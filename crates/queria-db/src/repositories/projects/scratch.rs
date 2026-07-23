use queria_core::{QueriaError, QueriaResult};
use uuid::Uuid;

use super::super::types::{MarkScratchChunkReadyParams, to_infrastructure_error};
use super::PgProjectRepository;

impl PgProjectRepository {
    /// Mark scratch chunk embedding ready after successful Voyage+Qdrant (IMP-13).
    pub async fn mark_scratch_chunk_ready(
        &self,
        params: &MarkScratchChunkReadyParams,
    ) -> QueriaResult<()> {
        let result = sqlx::query(
            "update chunk
             set embedding_provider = $2,
                 embedding_model = $3,
                 embedding_dimension = $4,
                 embedding_profile_version = $5,
                 embedding_content_hash = $6,
                 qdrant_point_id = $7,
                 embedding_status = 'ready',
                 embedding_error = null,
                 embedded_at = now(),
                 embedding_updated_at = now()
             where id = $1
               and exists (
                 select 1 from knowledge_item ki
                 where ki.id = chunk.knowledge_item_id
                   and ki.status = 'scratch'
               )",
        )
        .bind(params.chunk_id)
        .bind(&params.provider)
        .bind(&params.model)
        .bind(params.dimension)
        .bind(&params.profile_version)
        .bind(&params.embedding_content_hash)
        .bind(params.qdrant_point_id)
        .execute(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        if result.rows_affected() != 1 {
            return Err(QueriaError::Infrastructure(format!(
                "scratch chunk {} could not be marked ready",
                params.chunk_id
            )));
        }
        Ok(())
    }
    /// Roll back a newly created scratch item when embed/Qdrant fails (VAL-DL-033).
    /// Cascades to chunk and related rows; source_document cleaned explicitly.
    pub async fn delete_scratch_knowledge_item(
        &self,
        knowledge_item_id: Uuid,
        organization_id: Uuid,
    ) -> QueriaResult<()> {
        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;

        let source_document_id: Option<Uuid> = sqlx::query_scalar(
            "select source_document_id
             from knowledge_item
             where id = $1
               and organization_id = $2
               and status = 'scratch'",
        )
        .bind(knowledge_item_id)
        .bind(organization_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?
        .flatten();

        let deleted = sqlx::query(
            "delete from knowledge_item
             where id = $1
               and organization_id = $2
               and status = 'scratch'",
        )
        .bind(knowledge_item_id)
        .bind(organization_id)
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        if deleted.rows_affected() == 0 {
            return Err(QueriaError::NotFound(format!(
                "scratch knowledge_item {knowledge_item_id}"
            )));
        }

        if let Some(source_id) = source_document_id {
            sqlx::query(
                "delete from source_document
                 where id = $1
                   and organization_id = $2
                   and uri like 'queria://scratch/%'",
            )
            .bind(source_id)
            .bind(organization_id)
            .execute(&mut *transaction)
            .await
            .map_err(to_infrastructure_error)?;
        }

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;
        Ok(())
    }
}

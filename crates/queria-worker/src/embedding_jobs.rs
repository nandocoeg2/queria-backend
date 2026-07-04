use async_trait::async_trait;
use mockall::automock;
use queria_core::ids::{ChunkId, IngestionJobId};
use queria_core::{QueriaError, QueriaResult};
use queria_db::embedding::{
    EmbeddingChunkRecord, EmbeddingCompletion, PgEmbeddingRepository, canonical_embedding_text,
    embedding_content_hash,
};
use queria_db::ingestion::IngestionJobRecord;
use queria_search::embedding::{
    EmbeddingDocument, EmbeddingProvider, VectorIndex, VectorPayload, VectorPoint,
};
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmbeddingWorkerConfig {
    pub provider: String,
    pub model: String,
    pub dimension: usize,
    pub profile_version: String,
    pub batch_size: i64,
    pub retry_backoff_base_seconds: i64,
    pub retry_backoff_max_seconds: i64,
}

impl Default for EmbeddingWorkerConfig {
    fn default() -> Self {
        Self {
            provider: "voyage".to_owned(),
            model: "voyage-4".to_owned(),
            dimension: 1024,
            profile_version: "voyage-4-1024-v1".to_owned(),
            batch_size: 64,
            retry_backoff_base_seconds: 30,
            retry_backoff_max_seconds: 600,
        }
    }
}

#[automock]
#[async_trait]
pub trait EmbeddingJobStore: Send + Sync {
    async fn claim_next(&self, worker_id: &str) -> QueriaResult<Option<IngestionJobRecord>>;
    async fn claim_chunk_batch(
        &self,
        job_id: IngestionJobId,
        batch_size: i64,
        profile_version: &str,
    ) -> QueriaResult<Vec<EmbeddingChunkRecord>>;
    async fn mark_batch_ready(
        &self,
        completions: &[EmbeddingCompletion],
        provider: &str,
        model: &str,
        dimension: i32,
        profile_version: &str,
    ) -> QueriaResult<()>;
    async fn mark_batch_failed(&self, chunk_ids: &[Uuid], error: &str) -> QueriaResult<()>;
    async fn qdrant_delete_points(&self, job_id: IngestionJobId) -> QueriaResult<Vec<Uuid>>;
    async fn complete_job(&self, job_id: IngestionJobId, result: Value) -> QueriaResult<bool>;
    async fn fail_job(&self, job_id: IngestionJobId, error: &str) -> QueriaResult<bool>;
    async fn release_job_for_retry(
        &self,
        job_id: IngestionJobId,
        error: &str,
        backoff_seconds: i64,
    ) -> QueriaResult<bool>;
    async fn cancellation_requested(&self, job_id: IngestionJobId) -> QueriaResult<bool>;
}

pub async fn run_one<S, E, V>(
    store: &S,
    provider: &E,
    index: &V,
    config: &EmbeddingWorkerConfig,
    worker_id: &str,
) -> QueriaResult<bool>
where
    S: EmbeddingJobStore,
    E: EmbeddingProvider,
    V: VectorIndex,
{
    let Some(job) = store.claim_next(worker_id).await? else {
        return Ok(false);
    };
    let job_id = IngestionJobId::from_uuid(job.id);
    let result = match job.job_type.as_str() {
        "embedding_backfill" => run_backfill(store, provider, index, config, job_id).await,
        "qdrant_delete" => run_delete(store, index, job_id).await,
        _ => Err(QueriaError::Validation(format!(
            "unsupported worker job type {}",
            job.job_type
        ))),
    };
    if let Err(error) = result {
        let sanitized = sanitized_error(&error);
        if is_retryable_embedding_error(&error) {
            store
                .release_job_for_retry(
                    job_id,
                    &sanitized,
                    retry_backoff_seconds(config, job.attempts),
                )
                .await?;
        } else {
            store.fail_job(job_id, &sanitized).await?;
        }
    }
    Ok(true)
}

async fn run_backfill<S, E, V>(
    store: &S,
    provider: &E,
    index: &V,
    config: &EmbeddingWorkerConfig,
    job_id: IngestionJobId,
) -> QueriaResult<()>
where
    S: EmbeddingJobStore,
    E: EmbeddingProvider,
    V: VectorIndex,
{
    let mut processed = 0_u64;
    loop {
        if store.cancellation_requested(job_id).await? {
            return Err(QueriaError::Validation(
                "embedding backfill cancelled".to_owned(),
            ));
        }
        let chunks = store
            .claim_chunk_batch(job_id, config.batch_size, &config.profile_version)
            .await?;
        if chunks.is_empty() {
            store
                .complete_job(job_id, json!({ "processed_chunks": processed }))
                .await?;
            return Ok(());
        }
        let documents = chunks
            .iter()
            .map(|chunk| EmbeddingDocument {
                chunk_id: ChunkId::from_uuid(chunk.chunk_id),
                text: canonical_embedding_text(chunk),
            })
            .collect::<Vec<_>>();
        let vectors = match provider.embed_documents(&documents).await {
            Ok(vectors) if vectors.len() == chunks.len() => vectors,
            Ok(_) => {
                let error = QueriaError::Infrastructure(
                    "embedding response count did not match chunk batch".to_owned(),
                );
                fail_chunk_batch(store, &chunks, &error).await?;
                return Err(error);
            }
            Err(error) => {
                fail_chunk_batch(store, &chunks, &error).await?;
                return Err(error);
            }
        };
        let points = chunks
            .iter()
            .zip(vectors)
            .map(|(chunk, vector)| VectorPoint {
                id: chunk.chunk_id,
                vector,
                payload: VectorPayload {
                    organization_id: chunk.organization_id,
                    project_id: chunk.project_id,
                    scope: chunk.scope,
                    embedding_profile_version: config.profile_version.clone(),
                    is_active: true,
                },
            })
            .collect::<Vec<_>>();
        if let Err(error) = index.upsert(&points).await {
            fail_chunk_batch(store, &chunks, &error).await?;
            return Err(error);
        }
        let completions = chunks
            .iter()
            .map(|chunk| EmbeddingCompletion {
                chunk_id: chunk.chunk_id,
                qdrant_point_id: chunk.chunk_id,
                embedding_content_hash: embedding_content_hash(
                    chunk,
                    &config.provider,
                    &config.model,
                    config.dimension,
                    &config.profile_version,
                ),
            })
            .collect::<Vec<_>>();
        store
            .mark_batch_ready(
                &completions,
                &config.provider,
                &config.model,
                i32::try_from(config.dimension).map_err(|_| {
                    QueriaError::Config("embedding dimension exceeds database range".to_owned())
                })?,
                &config.profile_version,
            )
            .await?;
        processed += u64::try_from(chunks.len()).map_err(|_| {
            QueriaError::Infrastructure("embedding batch count overflow".to_owned())
        })?;
    }
}

async fn run_delete<S, V>(store: &S, index: &V, job_id: IngestionJobId) -> QueriaResult<()>
where
    S: EmbeddingJobStore,
    V: VectorIndex,
{
    if store.cancellation_requested(job_id).await? {
        return Err(QueriaError::Validation(
            "Qdrant delete cancelled".to_owned(),
        ));
    }
    let point_ids = store.qdrant_delete_points(job_id).await?;
    index.delete(&point_ids).await?;
    store
        .complete_job(
            job_id,
            json!({
                "deleted_points": point_ids.len()
            }),
        )
        .await?;
    Ok(())
}

async fn fail_chunk_batch<S>(
    store: &S,
    chunks: &[EmbeddingChunkRecord],
    error: &QueriaError,
) -> QueriaResult<()>
where
    S: EmbeddingJobStore,
{
    let chunk_ids = chunks
        .iter()
        .map(|chunk| chunk.chunk_id)
        .collect::<Vec<_>>();
    store
        .mark_batch_failed(&chunk_ids, &sanitized_error(error))
        .await
}

fn sanitized_error(error: &QueriaError) -> String {
    error.to_string().chars().take(500).collect()
}

fn is_retryable_embedding_error(error: &QueriaError) -> bool {
    let QueriaError::Infrastructure(message) = error else {
        return false;
    };
    message.contains("429")
        || message.contains("Too Many Requests")
        || message.contains("status 5")
        || message.contains("timed out")
        || message.contains("connection")
}

fn retry_backoff_seconds(config: &EmbeddingWorkerConfig, attempts: i32) -> i64 {
    let base = config.retry_backoff_base_seconds.max(1);
    let max = config.retry_backoff_max_seconds.max(base);
    let exponent = u32::try_from(attempts.saturating_sub(1).min(5)).unwrap_or(0);
    base.saturating_mul(1_i64 << exponent).min(max)
}

#[async_trait]
impl EmbeddingJobStore for PgEmbeddingRepository {
    async fn claim_next(&self, worker_id: &str) -> QueriaResult<Option<IngestionJobRecord>> {
        PgEmbeddingRepository::claim_next(self, worker_id).await
    }

    async fn claim_chunk_batch(
        &self,
        job_id: IngestionJobId,
        batch_size: i64,
        profile_version: &str,
    ) -> QueriaResult<Vec<EmbeddingChunkRecord>> {
        PgEmbeddingRepository::claim_chunk_batch(self, job_id, batch_size, profile_version).await
    }

    async fn mark_batch_ready(
        &self,
        completions: &[EmbeddingCompletion],
        provider: &str,
        model: &str,
        dimension: i32,
        profile_version: &str,
    ) -> QueriaResult<()> {
        PgEmbeddingRepository::mark_batch_ready(
            self,
            completions,
            provider,
            model,
            dimension,
            profile_version,
        )
        .await
    }

    async fn mark_batch_failed(&self, chunk_ids: &[Uuid], error: &str) -> QueriaResult<()> {
        PgEmbeddingRepository::mark_batch_failed(self, chunk_ids, error).await
    }

    async fn qdrant_delete_points(&self, job_id: IngestionJobId) -> QueriaResult<Vec<Uuid>> {
        PgEmbeddingRepository::qdrant_delete_points(self, job_id).await
    }

    async fn complete_job(&self, job_id: IngestionJobId, result: Value) -> QueriaResult<bool> {
        PgEmbeddingRepository::complete_job(self, job_id, result).await
    }

    async fn fail_job(&self, job_id: IngestionJobId, error: &str) -> QueriaResult<bool> {
        PgEmbeddingRepository::fail_job(self, job_id, error).await
    }

    async fn release_job_for_retry(
        &self,
        job_id: IngestionJobId,
        error: &str,
        backoff_seconds: i64,
    ) -> QueriaResult<bool> {
        PgEmbeddingRepository::release_job_for_retry(self, job_id, error, backoff_seconds).await
    }

    async fn cancellation_requested(&self, job_id: IngestionJobId) -> QueriaResult<bool> {
        PgEmbeddingRepository::cancellation_requested(self, job_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use mockall::Sequence;
    use queria_core::model::KnowledgeScope;
    use queria_search::embedding::EmbeddingVector;

    #[tokio::test]
    async fn no_embedding_job_returns_idle() {
        let mut store = MockEmbeddingJobStore::new();
        store.expect_claim_next().once().returning(|_| Ok(None));
        let provider = queria_search::embedding::MockEmbeddingProvider::new();
        let index = queria_search::embedding::MockVectorIndex::new();

        assert!(
            !run_one(
                &store,
                &provider,
                &index,
                &EmbeddingWorkerConfig::default(),
                "worker-1"
            )
            .await
            .expect("worker iteration should succeed")
        );
    }

    #[tokio::test]
    async fn embedding_backfill_upserts_then_marks_batch_ready() {
        let job = embedding_job();
        let job_id = IngestionJobId::from_uuid(job.id);
        let chunk = embedding_chunk();
        let chunk_id = chunk.chunk_id;
        let mut sequence = Sequence::new();
        let mut store = MockEmbeddingJobStore::new();
        store
            .expect_claim_next()
            .once()
            .return_once(move |_| Ok(Some(job)));
        store
            .expect_cancellation_requested()
            .times(2)
            .returning(|_| Ok(false));
        store
            .expect_claim_chunk_batch()
            .once()
            .in_sequence(&mut sequence)
            .return_once(move |_, _, _| Ok(vec![chunk]));
        store
            .expect_claim_chunk_batch()
            .once()
            .in_sequence(&mut sequence)
            .returning(|_, _, _| Ok(Vec::new()));
        store
            .expect_mark_batch_ready()
            .once()
            .withf(move |completions, _, _, dimension, _| {
                completions.len() == 1 && completions[0].chunk_id == chunk_id && *dimension == 2
            })
            .returning(|_, _, _, _, _| Ok(()));
        store
            .expect_complete_job()
            .once()
            .withf(move |seen_job_id, result| {
                *seen_job_id == job_id && result["processed_chunks"] == 1
            })
            .returning(|_, _| Ok(true));
        let mut provider = queria_search::embedding::MockEmbeddingProvider::new();
        provider
            .expect_embed_documents()
            .once()
            .withf(|documents| documents.len() == 1 && documents[0].text.contains("Deploy SOP"))
            .returning(|_| Ok(vec![EmbeddingVector::new(vec![0.1, 0.2], 2)?]));
        let mut index = queria_search::embedding::MockVectorIndex::new();
        index
            .expect_upsert()
            .once()
            .withf(move |points| points.len() == 1 && points[0].id == chunk_id)
            .returning(|_| Ok(()));
        let config = EmbeddingWorkerConfig {
            dimension: 2,
            profile_version: "test-v1".to_owned(),
            ..EmbeddingWorkerConfig::default()
        };

        assert!(
            run_one(&store, &provider, &index, &config, "worker-1")
                .await
                .expect("backfill should succeed")
        );
    }

    #[tokio::test]
    async fn retryable_provider_failure_requeues_job_instead_of_failing_it() {
        let job = embedding_job();
        let job_id = IngestionJobId::from_uuid(job.id);
        let chunk = embedding_chunk();
        let chunk_id = chunk.chunk_id;
        let mut store = MockEmbeddingJobStore::new();
        store
            .expect_claim_next()
            .once()
            .return_once(move |_| Ok(Some(job)));
        store
            .expect_cancellation_requested()
            .once()
            .returning(|_| Ok(false));
        store
            .expect_claim_chunk_batch()
            .once()
            .return_once(move |_, _, _| Ok(vec![chunk]));
        store
            .expect_mark_batch_failed()
            .once()
            .withf(move |chunk_ids, error| {
                chunk_ids == [chunk_id] && error.contains("429 Too Many Requests")
            })
            .returning(|_, _| Ok(()));
        store
            .expect_release_job_for_retry()
            .once()
            .withf(move |seen_job_id, error, backoff_seconds| {
                *seen_job_id == job_id
                    && error.contains("429 Too Many Requests")
                    && *backoff_seconds > 0
            })
            .returning(|_, _, _| Ok(true));
        store.expect_fail_job().never();
        let mut provider = queria_search::embedding::MockEmbeddingProvider::new();
        provider.expect_embed_documents().once().returning(|_| {
            Err(QueriaError::Infrastructure(
                "Voyage request failed with status 429 Too Many Requests; request_id=test"
                    .to_owned(),
            ))
        });
        let index = queria_search::embedding::MockVectorIndex::new();

        assert!(
            run_one(
                &store,
                &provider,
                &index,
                &EmbeddingWorkerConfig::default(),
                "worker-1"
            )
            .await
            .expect("retryable provider error should requeue the job")
        );
    }

    fn embedding_job() -> IngestionJobRecord {
        IngestionJobRecord {
            id: Uuid::now_v7(),
            organization_id: Uuid::now_v7(),
            project_id: Some(Uuid::now_v7()),
            source_document_id: None,
            status: "running".to_owned(),
            job_type: "embedding_backfill".to_owned(),
            payload: json!({}),
            locked_by: Some("worker-1".to_owned()),
            locked_at: Some(Utc::now()),
            attempts: 1,
            error_message: None,
            result: json!({}),
            retry_of_id: None,
            cancel_requested_at: None,
            started_at: Some(Utc::now()),
            finished_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn embedding_chunk() -> EmbeddingChunkRecord {
        EmbeddingChunkRecord {
            chunk_id: Uuid::now_v7(),
            organization_id: Uuid::now_v7(),
            project_id: Some(Uuid::now_v7()),
            scope: KnowledgeScope::Project,
            title: "Deploy SOP".to_owned(),
            source_path: "docs/deploy.md".to_owned(),
            body: "Deploy through CI.".to_owned(),
            content_hash: "source-hash".to_owned(),
            qdrant_point_id: None,
        }
    }
}

use async_trait::async_trait;
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
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmbeddingWorkerConfig {
    pub provider: String,
    pub model: String,
    pub dimension: usize,
    pub profile_version: String,
    pub batch_size: i64,
    pub request_interval_ms: u64,
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
            request_interval_ms: 0,
            retry_backoff_base_seconds: 30,
            retry_backoff_max_seconds: 600,
        }
    }
}

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
    async fn mark_batch_failed(
        &self,
        chunk_ids: &[Uuid],
        error: &str,
        retryable: bool,
    ) -> QueriaResult<()>;
    async fn qdrant_delete_points(&self, job_id: IngestionJobId) -> QueriaResult<Vec<Uuid>>;
    async fn complete_job(&self, job_id: IngestionJobId, result: Value) -> QueriaResult<bool>;
    async fn fail_job(&self, job_id: IngestionJobId, error: &str) -> QueriaResult<bool>;
    async fn release_job_for_retry(
        &self,
        job_id: IngestionJobId,
        error: &str,
        backoff_seconds: i64,
    ) -> QueriaResult<bool>;
    async fn pause_job_for_request_interval(
        &self,
        job_id: IngestionJobId,
        delay_millis: i64,
        processed_chunks: u64,
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
    let backoff_seconds = retry_backoff_seconds(config, job.attempts);
    let result = match job.job_type.as_str() {
        "embedding_backfill" => {
            run_backfill(
                store,
                provider,
                index,
                config,
                job_id,
                job.attempts,
                backoff_seconds,
            )
            .await
        }
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
                .release_job_for_retry(job_id, &sanitized, backoff_seconds)
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
    job_attempts: i32,
    backoff_seconds: i64,
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
                log_embedding_batch_failure(
                    job_id,
                    job_attempts,
                    &chunks,
                    &error,
                    retry_after_at(SystemTime::now(), backoff_seconds),
                );
                fail_chunk_batch(store, &chunks, &error).await?;
                return Err(error);
            }
            Err(error) => {
                log_embedding_batch_failure(
                    job_id,
                    job_attempts,
                    &chunks,
                    &error,
                    retry_after_at(SystemTime::now(), backoff_seconds),
                );
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
            log_embedding_batch_failure(
                job_id,
                job_attempts,
                &chunks,
                &error,
                retry_after_at(SystemTime::now(), backoff_seconds),
            );
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
        if let Some(delay_millis) = request_interval_millis(config) {
            store
                .pause_job_for_request_interval(job_id, delay_millis, processed)
                .await?;
            return Ok(());
        }
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
    let retryable = is_retryable_embedding_error(error);
    store
        .mark_batch_failed(&chunk_ids, &sanitized_error(error), retryable)
        .await
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BatchFailureLogContext {
    chunk_count: usize,
    provider_status: Option<String>,
    sample_source_path: Option<String>,
}

fn log_embedding_batch_failure(
    job_id: IngestionJobId,
    attempts: i32,
    chunks: &[EmbeddingChunkRecord],
    error: &QueriaError,
    retry_after_at: u64,
) {
    let context = batch_failure_log_context(chunks, error);
    tracing::warn!(
        job_id = %job_id.as_uuid(),
        attempts,
        chunk_count = context.chunk_count,
        provider_status = context.provider_status.as_deref().unwrap_or("unknown"),
        retry_after_at,
        sample_source_path = context.sample_source_path.as_deref().unwrap_or(""),
        error = %sanitized_error(error),
        "embedding batch failed"
    );
}

fn batch_failure_log_context(
    chunks: &[EmbeddingChunkRecord],
    error: &QueriaError,
) -> BatchFailureLogContext {
    BatchFailureLogContext {
        chunk_count: chunks.len(),
        provider_status: provider_status(error),
        sample_source_path: chunks.first().map(|chunk| chunk.source_path.clone()),
    }
}

fn provider_status(error: &QueriaError) -> Option<String> {
    let message = error.to_string();
    let (_, status) = message.split_once("status ")?;
    let status = status.split(';').next().unwrap_or(status).trim();
    (!status.is_empty()).then(|| status.to_owned())
}

fn retry_after_at(now: SystemTime, backoff_seconds: i64) -> u64 {
    let backoff = u64::try_from(backoff_seconds.max(1)).unwrap_or(1);
    let retry_time = now.checked_add(Duration::from_secs(backoff)).unwrap_or(now);
    retry_time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn request_interval_millis(config: &EmbeddingWorkerConfig) -> Option<i64> {
    (config.request_interval_ms > 0)
        .then(|| i64::try_from(config.request_interval_ms).unwrap_or(i64::MAX))
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

    async fn mark_batch_failed(
        &self,
        chunk_ids: &[Uuid],
        error: &str,
        retryable: bool,
    ) -> QueriaResult<()> {
        PgEmbeddingRepository::mark_batch_failed(self, chunk_ids, error, retryable).await
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

    async fn pause_job_for_request_interval(
        &self,
        job_id: IngestionJobId,
        delay_millis: i64,
        processed_chunks: u64,
    ) -> QueriaResult<bool> {
        PgEmbeddingRepository::pause_job_for_request_interval(
            self,
            job_id,
            delay_millis,
            processed_chunks,
        )
        .await
    }

    async fn cancellation_requested(&self, job_id: IngestionJobId) -> QueriaResult<bool> {
        PgEmbeddingRepository::cancellation_requested(self, job_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use queria_core::model::KnowledgeScope;
    use queria_search::embedding::{
        EmbeddingDocument, EmbeddingVector, VectorIndexHealth, VectorPoint, VectorSearchRequest,
    };
    use std::sync::Mutex;

    struct OkProvider;
    #[async_trait]
    impl EmbeddingProvider for OkProvider {
        async fn embed_documents(
            &self,
            inputs: &[EmbeddingDocument],
        ) -> QueriaResult<Vec<EmbeddingVector>> {
            inputs
                .iter()
                .map(|_| EmbeddingVector::new(vec![0.1, 0.2], 2))
                .collect()
        }
        async fn embed_query(&self, _query: &str) -> QueriaResult<EmbeddingVector> {
            EmbeddingVector::new(vec![0.1, 0.2], 2)
        }
    }

    struct FailProvider;
    #[async_trait]
    impl EmbeddingProvider for FailProvider {
        async fn embed_documents(
            &self,
            _inputs: &[EmbeddingDocument],
        ) -> QueriaResult<Vec<EmbeddingVector>> {
            Err(QueriaError::Infrastructure(
                "Voyage request failed with status 429 Too Many Requests; request_id=test"
                    .to_owned(),
            ))
        }
        async fn embed_query(&self, _query: &str) -> QueriaResult<EmbeddingVector> {
            Err(QueriaError::Infrastructure("unused".to_owned()))
        }
    }

    struct OkIndex;
    #[async_trait]
    impl VectorIndex for OkIndex {
        async fn ensure_collection(&self) -> QueriaResult<()> {
            Ok(())
        }
        async fn upsert(&self, _points: &[VectorPoint]) -> QueriaResult<()> {
            Ok(())
        }
        async fn search(
            &self,
            _request: VectorSearchRequest,
        ) -> QueriaResult<Vec<queria_search::embedding::VectorCandidate>> {
            Ok(Vec::new())
        }
        async fn delete(&self, _point_ids: &[Uuid]) -> QueriaResult<()> {
            Ok(())
        }
        async fn health(&self) -> QueriaResult<VectorIndexHealth> {
            Ok(VectorIndexHealth {
                collection: "test".to_owned(),
                points_count: 0,
            })
        }
    }

    #[derive(Default)]
    struct FakeEmbeddingJobStore {
        claim_next: Mutex<Option<QueriaResult<Option<IngestionJobRecord>>>>,
        claim_chunk_batches: Mutex<Vec<QueriaResult<Vec<EmbeddingChunkRecord>>>>,
        mark_batch_ready_calls: Mutex<Vec<(Vec<Uuid>, i32)>>,
        mark_batch_failed_calls: Mutex<Vec<(Vec<Uuid>, String, bool)>>,
        complete_job_calls: Mutex<Vec<(IngestionJobId, Value)>>,
        fail_job_calls: Mutex<Vec<(IngestionJobId, String)>>,
        release_job_calls: Mutex<Vec<(IngestionJobId, String, i64)>>,
        pause_job_calls: Mutex<Vec<(IngestionJobId, i64, u64)>>,
        cancellation_results: Mutex<Vec<bool>>,
    }

    #[async_trait]
    impl EmbeddingJobStore for FakeEmbeddingJobStore {
        async fn claim_next(
            &self,
            _worker_id: &str,
        ) -> QueriaResult<Option<IngestionJobRecord>> {
            self.claim_next
                .lock()
                .expect("lock")
                .take()
                .unwrap_or(Ok(None))
        }

        async fn claim_chunk_batch(
            &self,
            _job_id: IngestionJobId,
            _batch_size: i64,
            _profile_version: &str,
        ) -> QueriaResult<Vec<EmbeddingChunkRecord>> {
            let mut batches = self.claim_chunk_batches.lock().expect("lock");
            if batches.is_empty() {
                Ok(Vec::new())
            } else {
                batches.remove(0)
            }
        }

        async fn mark_batch_ready(
            &self,
            completions: &[EmbeddingCompletion],
            _provider: &str,
            _model: &str,
            dimension: i32,
            _profile_version: &str,
        ) -> QueriaResult<()> {
            self.mark_batch_ready_calls.lock().expect("lock").push((
                completions.iter().map(|c| c.chunk_id).collect(),
                dimension,
            ));
            Ok(())
        }

        async fn mark_batch_failed(
            &self,
            chunk_ids: &[Uuid],
            error: &str,
            retryable: bool,
        ) -> QueriaResult<()> {
            self.mark_batch_failed_calls.lock().expect("lock").push((
                chunk_ids.to_vec(),
                error.to_owned(),
                retryable,
            ));
            Ok(())
        }

        async fn qdrant_delete_points(
            &self,
            _job_id: IngestionJobId,
        ) -> QueriaResult<Vec<Uuid>> {
            Ok(Vec::new())
        }

        async fn complete_job(
            &self,
            job_id: IngestionJobId,
            result: Value,
        ) -> QueriaResult<bool> {
            self.complete_job_calls
                .lock()
                .expect("lock")
                .push((job_id, result));
            Ok(true)
        }

        async fn fail_job(&self, job_id: IngestionJobId, error: &str) -> QueriaResult<bool> {
            self.fail_job_calls
                .lock()
                .expect("lock")
                .push((job_id, error.to_owned()));
            Ok(true)
        }

        async fn release_job_for_retry(
            &self,
            job_id: IngestionJobId,
            error: &str,
            backoff_seconds: i64,
        ) -> QueriaResult<bool> {
            self.release_job_calls.lock().expect("lock").push((
                job_id,
                error.to_owned(),
                backoff_seconds,
            ));
            Ok(true)
        }

        async fn pause_job_for_request_interval(
            &self,
            job_id: IngestionJobId,
            delay_millis: i64,
            processed_chunks: u64,
        ) -> QueriaResult<bool> {
            self.pause_job_calls.lock().expect("lock").push((
                job_id,
                delay_millis,
                processed_chunks,
            ));
            Ok(true)
        }

        async fn cancellation_requested(
            &self,
            _job_id: IngestionJobId,
        ) -> QueriaResult<bool> {
            let mut results = self.cancellation_results.lock().expect("lock");
            if results.is_empty() {
                Ok(false)
            } else {
                Ok(results.remove(0))
            }
        }
    }

    #[tokio::test]
    async fn no_embedding_job_returns_idle() {
        let store = FakeEmbeddingJobStore {
            claim_next: Mutex::new(Some(Ok(None))),
            ..Default::default()
        };

        assert!(
            !run_one(
                &store,
                &OkProvider,
                &OkIndex,
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
        let store = FakeEmbeddingJobStore {
            claim_next: Mutex::new(Some(Ok(Some(job)))),
            claim_chunk_batches: Mutex::new(vec![Ok(vec![chunk]), Ok(Vec::new())]),
            cancellation_results: Mutex::new(vec![false, false]),
            ..Default::default()
        };
        let config = EmbeddingWorkerConfig {
            dimension: 2,
            profile_version: "test-v1".to_owned(),
            ..EmbeddingWorkerConfig::default()
        };

        assert!(
            run_one(&store, &OkProvider, &OkIndex, &config, "worker-1")
                .await
                .expect("backfill should succeed")
        );

        let ready = store.mark_batch_ready_calls.lock().expect("lock");
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].0, vec![chunk_id]);
        assert_eq!(ready[0].1, 2);
        let completed = store.complete_job_calls.lock().expect("lock");
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].0, job_id);
        assert_eq!(completed[0].1["processed_chunks"], 1);
    }

    #[tokio::test]
    async fn paced_backfill_iteration_returns_without_waiting_for_interval() {
        let job = embedding_job();
        let job_id = IngestionJobId::from_uuid(job.id);
        let chunk = embedding_chunk();
        let chunk_id = chunk.chunk_id;
        let store = FakeEmbeddingJobStore {
            claim_next: Mutex::new(Some(Ok(Some(job)))),
            claim_chunk_batches: Mutex::new(vec![Ok(vec![chunk])]),
            cancellation_results: Mutex::new(vec![false]),
            ..Default::default()
        };
        let config = EmbeddingWorkerConfig {
            dimension: 2,
            profile_version: "test-v1".to_owned(),
            request_interval_ms: 60_000,
            ..EmbeddingWorkerConfig::default()
        };

        tokio::time::timeout(
            Duration::from_millis(25),
            run_one(&store, &OkProvider, &OkIndex, &config, "worker-1"),
        )
        .await
        .expect("paced worker should not sleep while holding a running job")
        .expect("backfill should release iteration cleanly");

        let ready = store.mark_batch_ready_calls.lock().expect("lock");
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].0, vec![chunk_id]);
        assert_eq!(ready[0].1, 2);
        let pauses = store.pause_job_calls.lock().expect("lock");
        assert_eq!(pauses.len(), 1);
        assert_eq!(pauses[0].0, job_id);
        assert_eq!(pauses[0].1, 60_000);
        assert_eq!(pauses[0].2, 1);
        assert!(store.complete_job_calls.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn retryable_provider_failure_requeues_job_instead_of_failing_it() {
        let job = embedding_job();
        let job_id = IngestionJobId::from_uuid(job.id);
        let chunk = embedding_chunk();
        let chunk_id = chunk.chunk_id;
        let store = FakeEmbeddingJobStore {
            claim_next: Mutex::new(Some(Ok(Some(job)))),
            claim_chunk_batches: Mutex::new(vec![Ok(vec![chunk])]),
            cancellation_results: Mutex::new(vec![false]),
            ..Default::default()
        };

        assert!(
            run_one(
                &store,
                &FailProvider,
                &OkIndex,
                &EmbeddingWorkerConfig::default(),
                "worker-1"
            )
            .await
            .expect("retryable provider error should requeue the job")
        );

        let failed = store.mark_batch_failed_calls.lock().expect("lock");
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, vec![chunk_id]);
        assert!(failed[0].1.contains("429 Too Many Requests"));
        assert!(failed[0].2);
        let released = store.release_job_calls.lock().expect("lock");
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].0, job_id);
        assert!(released[0].1.contains("429 Too Many Requests"));
        assert!(released[0].2 > 0);
        assert!(store.fail_job_calls.lock().expect("lock").is_empty());
    }

    #[test]
    fn retry_backoff_uses_configured_cap() {
        let config = EmbeddingWorkerConfig {
            retry_backoff_base_seconds: 15,
            retry_backoff_max_seconds: 60,
            ..EmbeddingWorkerConfig::default()
        };

        assert_eq!(retry_backoff_seconds(&config, 1), 15);
        assert_eq!(retry_backoff_seconds(&config, 2), 30);
        assert_eq!(retry_backoff_seconds(&config, 7), 60);
    }

    #[test]
    fn batch_failure_log_context_captures_operational_fields() {
        let chunk = embedding_chunk();
        let error = QueriaError::Infrastructure(
            "Voyage request failed with status 429 Too Many Requests; request_id=test".to_owned(),
        );

        let context = batch_failure_log_context(&[chunk], &error);

        assert_eq!(context.chunk_count, 1);
        assert_eq!(
            context.provider_status.as_deref(),
            Some("429 Too Many Requests")
        );
        assert_eq!(
            context.sample_source_path.as_deref(),
            Some("docs/deploy.md")
        );
    }

    #[test]
    fn retry_after_at_uses_epoch_seconds() {
        let now = UNIX_EPOCH + Duration::from_secs(100);

        assert_eq!(retry_after_at(now, 30), 130);
        assert_eq!(retry_after_at(now, 0), 101);
    }

    #[test]
    fn request_interval_uses_configured_pacing() {
        let disabled = EmbeddingWorkerConfig {
            request_interval_ms: 0,
            ..EmbeddingWorkerConfig::default()
        };
        let paced = EmbeddingWorkerConfig {
            request_interval_ms: 250,
            ..EmbeddingWorkerConfig::default()
        };

        assert_eq!(request_interval_millis(&disabled), None);
        assert_eq!(request_interval_millis(&paced), Some(250));
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

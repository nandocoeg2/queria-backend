use async_trait::async_trait;
use mockall::automock;
use queria_core::QueriaResult;
use queria_db::ingestion::{ApplyManifestResult, GitIngestionSourceRecord, IngestionJobRecord};
use queria_ingestion::model::PreparedGitManifest;
use queria_ingestion::service::{GitIngestionService, GitIngestionSource};

#[automock]
#[async_trait]
pub trait JobStore: Send + Sync {
    async fn claim_next(&self, worker_id: &str) -> QueriaResult<Option<IngestionJobRecord>>;
    async fn load_source(
        &self,
        job_id: queria_core::ids::IngestionJobId,
    ) -> QueriaResult<Option<GitIngestionSourceRecord>>;
    async fn cancellation_requested(
        &self,
        job_id: queria_core::ids::IngestionJobId,
    ) -> QueriaResult<bool>;
    async fn apply_manifest(
        &self,
        job_id: queria_core::ids::IngestionJobId,
        pipeline_identity: &str,
        manifest: &PreparedGitManifest,
    ) -> QueriaResult<ApplyManifestResult>;
    async fn mark_failed(
        &self,
        job_id: queria_core::ids::IngestionJobId,
        error: &str,
    ) -> QueriaResult<bool>;
}

#[automock]
#[async_trait]
pub trait ManifestPreparer: Send + Sync {
    async fn prepare(&self, source: GitIngestionSourceRecord) -> QueriaResult<PreparedGitManifest>;
}

pub async fn run_one<S, P>(store: &S, preparer: &P, worker_id: &str) -> QueriaResult<bool>
where
    S: JobStore,
    P: ManifestPreparer,
{
    let Some(job) = store.claim_next(worker_id).await? else {
        return Ok(false);
    };
    let job_id = queria_core::ids::IngestionJobId::from_uuid(job.id);
    let Some(source) = store.load_source(job_id).await? else {
        store
            .mark_failed(job_id, "Git ingestion source is unavailable")
            .await?;
        return Ok(true);
    };
    if store.cancellation_requested(job_id).await? {
        store.mark_failed(job_id, "ingestion cancelled").await?;
        return Ok(true);
    }
    let manifest = match preparer.prepare(source).await {
        Ok(manifest) => manifest,
        Err(error) => {
            store.mark_failed(job_id, &sanitized_error(&error)).await?;
            return Ok(true);
        }
    };
    if store.cancellation_requested(job_id).await? {
        store.mark_failed(job_id, "ingestion cancelled").await?;
        return Ok(true);
    }
    match store.apply_manifest(job_id, worker_id, &manifest).await {
        Ok(_) => {}
        Err(error) => {
            store.mark_failed(job_id, &sanitized_error(&error)).await?;
            return Ok(true);
        }
    }
    Ok(true)
}

#[async_trait]
impl JobStore for queria_db::ingestion::PgIngestionRepository {
    async fn claim_next(&self, worker_id: &str) -> QueriaResult<Option<IngestionJobRecord>> {
        queria_db::ingestion::PgIngestionRepository::claim_next(self, worker_id).await
    }

    async fn load_source(
        &self,
        job_id: queria_core::ids::IngestionJobId,
    ) -> QueriaResult<Option<GitIngestionSourceRecord>> {
        self.load_source_for_job(job_id).await
    }

    async fn cancellation_requested(
        &self,
        job_id: queria_core::ids::IngestionJobId,
    ) -> QueriaResult<bool> {
        queria_db::ingestion::PgIngestionRepository::cancellation_requested(self, job_id).await
    }

    async fn apply_manifest(
        &self,
        job_id: queria_core::ids::IngestionJobId,
        pipeline_identity: &str,
        manifest: &PreparedGitManifest,
    ) -> QueriaResult<ApplyManifestResult> {
        self.apply_git_manifest(job_id, pipeline_identity, manifest)
            .await
    }

    async fn mark_failed(
        &self,
        job_id: queria_core::ids::IngestionJobId,
        error: &str,
    ) -> QueriaResult<bool> {
        queria_db::ingestion::PgIngestionRepository::mark_failed(self, job_id, error).await
    }
}

#[async_trait]
impl<G, S> ManifestPreparer for GitIngestionService<G, S>
where
    G: queria_ingestion::git::GitRepositoryGateway,
    S: queria_ingestion::scanner::SecretScanner,
{
    async fn prepare(&self, source: GitIngestionSourceRecord) -> QueriaResult<PreparedGitManifest> {
        GitIngestionService::prepare(
            self,
            GitIngestionSource {
                path: source.path,
                uri: source.uri,
                trusted_auto_approve: source.trusted_auto_approve,
            },
        )
        .await
    }
}

fn sanitized_error(error: &queria_core::QueriaError) -> String {
    error.to_string().chars().take(500).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use queria_core::QueriaError;
    use std::path::PathBuf;
    use uuid::Uuid;

    #[tokio::test]
    async fn no_queued_job_returns_idle() {
        let mut store = MockJobStore::new();
        store.expect_claim_next().once().returning(|_| Ok(None));
        let preparer = MockManifestPreparer::new();

        assert!(
            !run_one(&store, &preparer, "worker-1")
                .await
                .expect("worker should run")
        );
    }

    #[tokio::test]
    async fn successful_job_is_applied_and_completed() {
        let job = job();
        let job_id = queria_core::ids::IngestionJobId::from_uuid(job.id);
        let source = source(job.source_document_id.expect("source id"));
        let manifest = manifest();
        let mut store = MockJobStore::new();
        store
            .expect_claim_next()
            .once()
            .return_once(move |_| Ok(Some(job)));
        store
            .expect_load_source()
            .once()
            .return_once(move |_| Ok(Some(source)));
        store
            .expect_cancellation_requested()
            .times(2)
            .returning(|_| Ok(false));
        store.expect_apply_manifest().once().returning(|_, _, _| {
            Ok(ApplyManifestResult {
                indexed_files: 1,
                knowledge_items: 1,
                chunks: 1,
                ..Default::default()
            })
        });
        let mut preparer = MockManifestPreparer::new();
        preparer
            .expect_prepare()
            .once()
            .return_once(move |_| Ok(manifest));

        assert!(
            run_one(&store, &preparer, "worker-1")
                .await
                .expect("worker should run")
        );
        assert_eq!(job_id.as_uuid(), job_id.as_uuid());
    }

    #[tokio::test]
    async fn preparation_failure_marks_job_failed() {
        let job = job();
        let source = source(job.source_document_id.expect("source id"));
        let mut store = MockJobStore::new();
        store
            .expect_claim_next()
            .once()
            .return_once(move |_| Ok(Some(job)));
        store
            .expect_load_source()
            .once()
            .return_once(move |_| Ok(Some(source)));
        store
            .expect_cancellation_requested()
            .once()
            .returning(|_| Ok(false));
        store
            .expect_mark_failed()
            .once()
            .withf(|_, error| error.contains("secret scan failed"))
            .returning(|_, _| Ok(true));
        let mut preparer = MockManifestPreparer::new();
        preparer
            .expect_prepare()
            .once()
            .returning(|_| Err(QueriaError::Validation("secret scan failed".to_owned())));

        assert!(
            run_one(&store, &preparer, "worker-1")
                .await
                .expect("worker should survive job failure")
        );
    }

    #[tokio::test]
    async fn cancelled_job_never_reaches_manifest_preparation() {
        let job = job();
        let source = source(job.source_document_id.expect("source id"));
        let mut store = MockJobStore::new();
        store
            .expect_claim_next()
            .once()
            .return_once(move |_| Ok(Some(job)));
        store
            .expect_load_source()
            .once()
            .return_once(move |_| Ok(Some(source)));
        store
            .expect_cancellation_requested()
            .once()
            .returning(|_| Ok(true));
        store.expect_mark_failed().once().returning(|_, _| Ok(true));
        let preparer = MockManifestPreparer::new();

        assert!(
            run_one(&store, &preparer, "worker-1")
                .await
                .expect("worker should cancel job")
        );
    }

    fn job() -> IngestionJobRecord {
        IngestionJobRecord {
            id: Uuid::now_v7(),
            organization_id: Uuid::now_v7(),
            project_id: Some(Uuid::now_v7()),
            source_document_id: Some(Uuid::now_v7()),
            status: "running".to_owned(),
            job_type: "git_ingestion".to_owned(),
            payload: serde_json::json!({}),
            locked_by: Some("worker-1".to_owned()),
            locked_at: Some(Utc::now()),
            attempts: 1,
            error_message: None,
            result: serde_json::json!({}),
            retry_of_id: None,
            cancel_requested_at: None,
            started_at: Some(Utc::now()),
            finished_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn source(source_document_id: Uuid) -> GitIngestionSourceRecord {
        GitIngestionSourceRecord {
            source_document_id,
            path: PathBuf::from("/tmp/repository"),
            uri: "git@github.com:nandocoeg2/fjulian.me.git".to_owned(),
            trusted_auto_approve: true,
        }
    }

    fn manifest() -> PreparedGitManifest {
        PreparedGitManifest {
            commit_sha: "a".repeat(40),
            branch: "main".to_owned(),
            content_hash: "hash".to_owned(),
            trusted_auto_approve: true,
            files: Vec::new(),
        }
    }
}

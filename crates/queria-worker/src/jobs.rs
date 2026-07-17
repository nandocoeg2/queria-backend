use async_trait::async_trait;
use queria_core::QueriaResult;
use queria_db::ingestion::{ApplyManifestResult, GitIngestionSourceRecord, IngestionJobRecord};
use queria_ingestion::model::PreparedGitManifest;
use queria_ingestion::service::{GitIngestionService, GitIngestionSource};

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
    use std::sync::Mutex;
    use uuid::Uuid;

    #[derive(Default)]
    struct FakeJobStore {
        claim_next: Mutex<Option<QueriaResult<Option<IngestionJobRecord>>>>,
        load_source: Mutex<Option<QueriaResult<Option<GitIngestionSourceRecord>>>>,
        cancellation_results: Mutex<Vec<bool>>,
        apply_result: Mutex<Option<QueriaResult<ApplyManifestResult>>>,
        mark_failed_errors: Mutex<Vec<String>>,
        apply_called: Mutex<bool>,
    }

    #[async_trait]
    impl JobStore for FakeJobStore {
        async fn claim_next(&self, _worker_id: &str) -> QueriaResult<Option<IngestionJobRecord>> {
            self.claim_next
                .lock()
                .expect("lock")
                .take()
                .unwrap_or(Ok(None))
        }

        async fn load_source(
            &self,
            _job_id: queria_core::ids::IngestionJobId,
        ) -> QueriaResult<Option<GitIngestionSourceRecord>> {
            self.load_source
                .lock()
                .expect("lock")
                .take()
                .unwrap_or(Ok(None))
        }

        async fn cancellation_requested(
            &self,
            _job_id: queria_core::ids::IngestionJobId,
        ) -> QueriaResult<bool> {
            let mut results = self.cancellation_results.lock().expect("lock");
            if results.is_empty() {
                Ok(false)
            } else {
                Ok(results.remove(0))
            }
        }

        async fn apply_manifest(
            &self,
            _job_id: queria_core::ids::IngestionJobId,
            _pipeline_identity: &str,
            _manifest: &PreparedGitManifest,
        ) -> QueriaResult<ApplyManifestResult> {
            *self.apply_called.lock().expect("lock") = true;
            self.apply_result
                .lock()
                .expect("lock")
                .take()
                .unwrap_or(Ok(ApplyManifestResult {
                    indexed_files: 1,
                    knowledge_items: 1,
                    chunks: 1,
                    ..Default::default()
                }))
        }

        async fn mark_failed(
            &self,
            _job_id: queria_core::ids::IngestionJobId,
            error: &str,
        ) -> QueriaResult<bool> {
            self.mark_failed_errors
                .lock()
                .expect("lock")
                .push(error.to_owned());
            Ok(true)
        }
    }

    struct FakeManifestPreparer {
        result: Mutex<Option<QueriaResult<PreparedGitManifest>>>,
        prepare_called: Mutex<bool>,
    }

    impl FakeManifestPreparer {
        fn ok(manifest: PreparedGitManifest) -> Self {
            Self {
                result: Mutex::new(Some(Ok(manifest))),
                prepare_called: Mutex::new(false),
            }
        }

        fn err(error: QueriaError) -> Self {
            Self {
                result: Mutex::new(Some(Err(error))),
                prepare_called: Mutex::new(false),
            }
        }

        fn unused() -> Self {
            Self {
                result: Mutex::new(None),
                prepare_called: Mutex::new(false),
            }
        }
    }

    #[async_trait]
    impl ManifestPreparer for FakeManifestPreparer {
        async fn prepare(
            &self,
            _source: GitIngestionSourceRecord,
        ) -> QueriaResult<PreparedGitManifest> {
            *self.prepare_called.lock().expect("lock") = true;
            self.result
                .lock()
                .expect("lock")
                .take()
                .expect("prepare should not be called")
        }
    }

    #[tokio::test]
    async fn no_queued_job_returns_idle() {
        let store = FakeJobStore {
            claim_next: Mutex::new(Some(Ok(None))),
            ..Default::default()
        };
        let preparer = FakeManifestPreparer::unused();

        assert!(
            !run_one(&store, &preparer, "worker-1")
                .await
                .expect("worker should run")
        );
        assert!(!*preparer.prepare_called.lock().expect("lock"));
    }

    #[tokio::test]
    async fn successful_job_is_applied_and_completed() {
        let job = job();
        let job_id = queria_core::ids::IngestionJobId::from_uuid(job.id);
        let source = source(job.source_document_id.expect("source id"));
        let manifest = manifest();
        let store = FakeJobStore {
            claim_next: Mutex::new(Some(Ok(Some(job)))),
            load_source: Mutex::new(Some(Ok(Some(source)))),
            cancellation_results: Mutex::new(vec![false, false]),
            apply_result: Mutex::new(Some(Ok(ApplyManifestResult {
                indexed_files: 1,
                knowledge_items: 1,
                chunks: 1,
                ..Default::default()
            }))),
            ..Default::default()
        };
        let preparer = FakeManifestPreparer::ok(manifest);

        assert!(
            run_one(&store, &preparer, "worker-1")
                .await
                .expect("worker should run")
        );
        assert!(*store.apply_called.lock().expect("lock"));
        assert!(*preparer.prepare_called.lock().expect("lock"));
        assert!(store.mark_failed_errors.lock().expect("lock").is_empty());
        assert_eq!(job_id.as_uuid(), job_id.as_uuid());
    }

    #[tokio::test]
    async fn preparation_failure_marks_job_failed() {
        let job = job();
        let source = source(job.source_document_id.expect("source id"));
        let store = FakeJobStore {
            claim_next: Mutex::new(Some(Ok(Some(job)))),
            load_source: Mutex::new(Some(Ok(Some(source)))),
            cancellation_results: Mutex::new(vec![false]),
            ..Default::default()
        };
        let preparer =
            FakeManifestPreparer::err(QueriaError::Validation("secret scan failed".to_owned()));

        assert!(
            run_one(&store, &preparer, "worker-1")
                .await
                .expect("worker should survive job failure")
        );
        let errors = store.mark_failed_errors.lock().expect("lock");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("secret scan failed"));
        assert!(!*store.apply_called.lock().expect("lock"));
    }

    #[tokio::test]
    async fn cancelled_job_never_reaches_manifest_preparation() {
        let job = job();
        let source = source(job.source_document_id.expect("source id"));
        let store = FakeJobStore {
            claim_next: Mutex::new(Some(Ok(Some(job)))),
            load_source: Mutex::new(Some(Ok(Some(source)))),
            cancellation_results: Mutex::new(vec![true]),
            ..Default::default()
        };
        let preparer = FakeManifestPreparer::unused();

        assert!(
            run_one(&store, &preparer, "worker-1")
                .await
                .expect("worker should cancel job")
        );
        assert!(!*preparer.prepare_called.lock().expect("lock"));
        assert_eq!(store.mark_failed_errors.lock().expect("lock").len(), 1);
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

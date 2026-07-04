use chrono::{DateTime, Utc};
use queria_core::ids::{IngestionJobId, SourceDocumentId};
use queria_core::{QueriaError, QueriaResult};
use queria_ingestion::model::{PreparedFile, PreparedGitManifest};
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

const CLAIM_JOB_SQL: &str = "with candidate as (
       select id
       from ingestion_job
       where status = 'queued'
         and job_type = 'git_ingestion'
       order by created_at
       for update skip locked
       limit 1
     )
     update ingestion_job job
     set status = 'running',
         locked_by = $1,
         locked_at = now(),
         started_at = coalesce(started_at, now()),
         attempts = attempts + 1,
         updated_at = now()
     from candidate
     where job.id = candidate.id
     returning job.id, job.organization_id, job.project_id, job.source_document_id,
               job.status::text as status, job.job_type, job.payload, job.locked_by,
               job.locked_at, job.attempts, job.error_message, job.result,
               job.retry_of_id, job.cancel_requested_at, job.started_at,
               job.finished_at, job.created_at, job.updated_at";

#[derive(Clone, Debug, PartialEq)]
pub struct IngestionJobRecord {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub project_id: Option<Uuid>,
    pub source_document_id: Option<Uuid>,
    pub status: String,
    pub job_type: String,
    pub payload: Value,
    pub locked_by: Option<String>,
    pub locked_at: Option<DateTime<Utc>>,
    pub attempts: i32,
    pub error_message: Option<String>,
    pub result: Value,
    pub retry_of_id: Option<Uuid>,
    pub cancel_requested_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobMutation<T> {
    Updated(T),
    NotFound,
    InvalidState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitIngestionSourceRecord {
    pub source_document_id: Uuid,
    pub path: PathBuf,
    pub uri: String,
    pub trusted_auto_approve: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ApplyManifestResult {
    pub indexed_files: u64,
    pub unchanged_files: u64,
    pub deprecated_files: u64,
    pub knowledge_items: u64,
    pub chunks: u64,
}

#[derive(Debug)]
struct RootSource {
    id: Uuid,
    organization_id: Uuid,
    project_id: Uuid,
    trusted_auto_approve: bool,
}

#[derive(Clone, Debug)]
pub struct PgIngestionRepository {
    pool: PgPool,
}

impl PgIngestionRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn trigger(
        &self,
        user_id: Uuid,
        source_document_id: SourceDocumentId,
    ) -> QueriaResult<Option<IngestionJobRecord>> {
        sqlx::query(
            "with accessible_source as (
               select sd.organization_id, sd.project_id, sd.id
               from source_document sd
               join user_account u on u.organization_id = sd.organization_id
               where u.id = $1
                 and sd.id = $2
                 and sd.kind = 'git_repo'
                 and sd.source_root_id is null
                 and sd.is_active
             )
             insert into ingestion_job(
               organization_id, project_id, source_document_id, job_type, payload
             )
             select organization_id, project_id, id, 'git_ingestion',
                    jsonb_build_object('triggered_by_user_id', $1::text)
             from accessible_source
             on conflict (source_document_id, job_type)
               where source_document_id is not null and status in ('queued', 'running')
             do update set updated_at = ingestion_job.updated_at
             returning id, organization_id, project_id, source_document_id,
                       status::text as status, job_type, payload, locked_by, locked_at,
                       attempts, error_message, result, retry_of_id,
                       cancel_requested_at, started_at, finished_at, created_at, updated_at",
        )
        .bind(user_id)
        .bind(source_document_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(job_from_row)
        .transpose()
    }

    pub async fn list_for_user(
        &self,
        user_id: Uuid,
        status: Option<&str>,
        limit: i64,
    ) -> QueriaResult<Vec<IngestionJobRecord>> {
        sqlx::query(
            "select job.id, job.organization_id, job.project_id, job.source_document_id,
                    job.status::text as status, job.job_type, job.payload, job.locked_by,
                    job.locked_at, job.attempts, job.error_message, job.result,
                    job.retry_of_id, job.cancel_requested_at, job.started_at,
                    job.finished_at, job.created_at, job.updated_at
             from ingestion_job job
             join user_account u on u.organization_id = job.organization_id
             where u.id = $1
               and ($2::text is null or job.status::text = $2)
             order by job.created_at desc
             limit $3",
        )
        .bind(user_id)
        .bind(status)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .into_iter()
        .map(job_from_row)
        .collect()
    }

    pub async fn get_for_user(
        &self,
        user_id: Uuid,
        job_id: IngestionJobId,
    ) -> QueriaResult<Option<IngestionJobRecord>> {
        sqlx::query(
            "select job.id, job.organization_id, job.project_id, job.source_document_id,
                    job.status::text as status, job.job_type, job.payload, job.locked_by,
                    job.locked_at, job.attempts, job.error_message, job.result,
                    job.retry_of_id, job.cancel_requested_at, job.started_at,
                    job.finished_at, job.created_at, job.updated_at
             from ingestion_job job
             join user_account u on u.organization_id = job.organization_id
             where u.id = $1 and job.id = $2",
        )
        .bind(user_id)
        .bind(job_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(job_from_row)
        .transpose()
    }

    pub async fn retry(
        &self,
        user_id: Uuid,
        job_id: IngestionJobId,
    ) -> QueriaResult<JobMutation<IngestionJobRecord>> {
        let row = sqlx::query(
            "with retryable as (
               select job.*
               from ingestion_job job
               join user_account u on u.organization_id = job.organization_id
               where u.id = $1
                 and job.id = $2
                 and job.status in ('failed', 'cancelled')
             )
             insert into ingestion_job(
               organization_id, project_id, source_document_id, job_type, payload, retry_of_id
             )
             select organization_id, project_id, source_document_id, job_type, payload, id
             from retryable
             on conflict (source_document_id, job_type)
               where source_document_id is not null and status in ('queued', 'running')
             do nothing
             returning id, organization_id, project_id, source_document_id,
                       status::text as status, job_type, payload, locked_by, locked_at,
                       attempts, error_message, result, retry_of_id,
                       cancel_requested_at, started_at, finished_at, created_at, updated_at",
        )
        .bind(user_id)
        .bind(job_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        if let Some(row) = row {
            return Ok(JobMutation::Updated(job_from_row(row)?));
        }

        self.classify_failed_mutation(user_id, job_id).await
    }

    pub async fn cancel(
        &self,
        user_id: Uuid,
        job_id: IngestionJobId,
    ) -> QueriaResult<JobMutation<IngestionJobRecord>> {
        let row = sqlx::query(
            "update ingestion_job job
             set status = case when status = 'queued' then 'cancelled'::ingestion_status else status end,
                 cancel_requested_at = now(),
                 finished_at = case when status = 'queued' then now() else finished_at end,
                 updated_at = now()
             from user_account u
             where u.id = $1
               and u.organization_id = job.organization_id
               and job.id = $2
               and job.status in ('queued', 'running')
             returning job.id, job.organization_id, job.project_id, job.source_document_id,
                       job.status::text as status, job.job_type, job.payload, job.locked_by,
                       job.locked_at, job.attempts, job.error_message, job.result,
                       job.retry_of_id, job.cancel_requested_at, job.started_at,
                       job.finished_at, job.created_at, job.updated_at",
        )
        .bind(user_id)
        .bind(job_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        if let Some(row) = row {
            return Ok(JobMutation::Updated(job_from_row(row)?));
        }

        self.classify_failed_mutation(user_id, job_id).await
    }

    pub async fn claim_next(&self, worker_id: &str) -> QueriaResult<Option<IngestionJobRecord>> {
        sqlx::query(CLAIM_JOB_SQL)
            .bind(worker_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(to_infrastructure_error)?
            .map(job_from_row)
            .transpose()
    }

    pub async fn load_source_for_job(
        &self,
        job_id: IngestionJobId,
    ) -> QueriaResult<Option<GitIngestionSourceRecord>> {
        sqlx::query(
            "select sd.id, sd.source_path, sd.uri, sd.metadata
             from ingestion_job job
             join source_document sd on sd.id = job.source_document_id
             where job.id = $1
               and job.status = 'running'
               and job.job_type = 'git_ingestion'
               and sd.kind = 'git_repo'
               and sd.source_root_id is null
               and sd.is_active",
        )
        .bind(job_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(|row| {
            let path = row
                .try_get::<Option<String>, _>("source_path")
                .map_err(to_infrastructure_error)?
                .ok_or_else(|| {
                    QueriaError::Validation("Git source has no local source_path".to_owned())
                })?;
            let metadata: Value = row.try_get("metadata").map_err(to_infrastructure_error)?;
            let uri: String = metadata
                .get("ssh_uri")
                .and_then(Value::as_str)
                .map_or_else(
                    || row.try_get("uri").map_err(to_infrastructure_error),
                    |value| Ok(value.to_owned()),
                )?;
            Ok(GitIngestionSourceRecord {
                source_document_id: row.try_get("id").map_err(to_infrastructure_error)?,
                path: PathBuf::from(path),
                uri,
                trusted_auto_approve: metadata
                    .get("trusted_auto_approve")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .transpose()
    }

    pub async fn apply_git_manifest(
        &self,
        job_id: IngestionJobId,
        pipeline_identity: &str,
        manifest: &PreparedGitManifest,
    ) -> QueriaResult<ApplyManifestResult> {
        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;
        let root_row = sqlx::query(
            "select sd.id, sd.organization_id, sd.project_id, sd.metadata
             from ingestion_job job
             join source_document sd on sd.id = job.source_document_id
             where job.id = $1
               and job.status = 'running'
               and job.job_type = 'git_ingestion'
               and sd.kind = 'git_repo'
               and sd.source_root_id is null
               and sd.is_active
             for update of job, sd",
        )
        .bind(job_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?
        .ok_or_else(|| QueriaError::NotFound("running Git ingestion job".to_owned()))?;
        let metadata: Value = root_row
            .try_get("metadata")
            .map_err(to_infrastructure_error)?;
        let root = RootSource {
            id: root_row.try_get("id").map_err(to_infrastructure_error)?,
            organization_id: root_row
                .try_get("organization_id")
                .map_err(to_infrastructure_error)?,
            project_id: root_row
                .try_get::<Option<Uuid>, _>("project_id")
                .map_err(to_infrastructure_error)?
                .ok_or_else(|| {
                    QueriaError::Validation("Git source must belong to a project".to_owned())
                })?,
            trusted_auto_approve: metadata
                .get("trusted_auto_approve")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                && manifest.trusted_auto_approve,
        };

        let existing_rows = sqlx::query(
            "select id, source_path, content_hash
             from source_document
             where source_root_id = $1 and is_active
             for update",
        )
        .bind(root.id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;
        let mut existing = existing_rows
            .into_iter()
            .map(|row| {
                Ok((
                    row.try_get::<String, _>("source_path")?,
                    (
                        row.try_get::<Uuid, _>("id")?,
                        row.try_get::<String, _>("content_hash")?,
                    ),
                ))
            })
            .collect::<Result<HashMap<_, _>, sqlx::Error>>()
            .map_err(to_infrastructure_error)?;

        let mut result = ApplyManifestResult::default();
        for file in &manifest.files {
            match existing.remove(&file.path) {
                Some((source_id, content_hash)) if content_hash == file.content_hash => {
                    update_unchanged_child(
                        &mut transaction,
                        source_id,
                        manifest,
                        pipeline_identity,
                    )
                    .await?;
                    result.unchanged_files += 1;
                }
                Some((source_id, _)) => {
                    retire_generated_knowledge(&mut transaction, source_id, "superseded").await?;
                    update_changed_child(
                        &mut transaction,
                        source_id,
                        file,
                        manifest,
                        pipeline_identity,
                    )
                    .await?;
                    index_file(
                        &mut transaction,
                        &root,
                        source_id,
                        file,
                        manifest,
                        pipeline_identity,
                        &mut result,
                    )
                    .await?;
                }
                None => {
                    let source_id = insert_child_source(
                        &mut transaction,
                        &root,
                        file,
                        manifest,
                        pipeline_identity,
                    )
                    .await?;
                    index_file(
                        &mut transaction,
                        &root,
                        source_id,
                        file,
                        manifest,
                        pipeline_identity,
                        &mut result,
                    )
                    .await?;
                }
            }
        }

        for (path, (source_id, _)) in existing {
            retire_generated_knowledge(&mut transaction, source_id, "deprecated").await?;
            sqlx::query(
                "update source_document
                 set is_active = false, indexed_at = now(), updated_at = now()
                 where id = $1",
            )
            .bind(source_id)
            .execute(&mut *transaction)
            .await
            .map_err(to_infrastructure_error)?;
            insert_audit(
                &mut transaction,
                &root,
                pipeline_identity,
                "source.deprecated",
                source_id,
                json!({"path": path, "job_id": job_id.to_string()}),
            )
            .await?;
            result.deprecated_files += 1;
        }

        sqlx::query(
            "update source_document
             set branch = $2, commit_sha = $3, content_hash = $4,
                 indexed_at = now(), updated_at = now(),
                 metadata = metadata || jsonb_build_object(
                   'last_pipeline_identity', $5::text,
                   'last_ingestion_job_id', $6::text
                 )
             where id = $1",
        )
        .bind(root.id)
        .bind(&manifest.branch)
        .bind(&manifest.commit_sha)
        .bind(&manifest.content_hash)
        .bind(pipeline_identity)
        .bind(job_id.to_string())
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;
        insert_audit(
            &mut transaction,
            &root,
            pipeline_identity,
            "ingestion.applied",
            root.id,
            json!({
                "job_id": job_id.to_string(),
                "commit_sha": manifest.commit_sha,
                "result": result,
                "trusted_auto_approve": root.trusted_auto_approve
            }),
        )
        .await?;
        let result_value = serde_json::to_value(&result).map_err(|error| {
            QueriaError::Infrastructure(format!("failed to serialize ingestion result: {error}"))
        })?;
        let completed = sqlx::query(
            "update ingestion_job
             set status = 'succeeded', result = $2, error_message = null,
                 finished_at = now(), locked_by = null, locked_at = null, updated_at = now()
             where id = $1 and status = 'running' and cancel_requested_at is null",
        )
        .bind(job_id.as_uuid())
        .bind(result_value)
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?
        .rows_affected();
        if completed != 1 {
            return Err(QueriaError::Validation(
                "ingestion job was cancelled before commit".to_owned(),
            ));
        }
        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;
        Ok(result)
    }

    pub async fn cancellation_requested(&self, job_id: IngestionJobId) -> QueriaResult<bool> {
        sqlx::query_scalar::<_, bool>(
            "select cancel_requested_at is not null from ingestion_job where id = $1",
        )
        .bind(job_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .ok_or_else(|| QueriaError::NotFound("ingestion job".to_owned()))
    }

    pub async fn mark_failed(
        &self,
        job_id: IngestionJobId,
        error_message: &str,
    ) -> QueriaResult<bool> {
        let updated = sqlx::query(
            "update ingestion_job
             set status = case when cancel_requested_at is null
                               then 'failed'::ingestion_status
                               else 'cancelled'::ingestion_status end,
                 error_message = case when cancel_requested_at is null then $2 else null end,
                 finished_at = now(), locked_by = null, locked_at = null, updated_at = now()
             where id = $1 and status = 'running'",
        )
        .bind(job_id.as_uuid())
        .bind(error_message)
        .execute(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .rows_affected();
        Ok(updated == 1)
    }

    pub async fn recover_expired_leases(&self, lease_seconds: i64) -> QueriaResult<u64> {
        let result = sqlx::query(
            "update ingestion_job
             set status = 'queued', locked_by = null, locked_at = null, updated_at = now(),
                 error_message = 'worker lease expired; job requeued'
             where status = 'running'
               and job_type = 'git_ingestion'
               and locked_at < now() - make_interval(secs => $1)
               and cancel_requested_at is null",
        )
        .bind(lease_seconds as f64)
        .execute(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;
        Ok(result.rows_affected())
    }

    async fn classify_failed_mutation(
        &self,
        user_id: Uuid,
        job_id: IngestionJobId,
    ) -> QueriaResult<JobMutation<IngestionJobRecord>> {
        let exists = sqlx::query_scalar::<_, bool>(
            "select exists(
               select 1 from ingestion_job job
               join user_account u on u.organization_id = job.organization_id
               where u.id = $1 and job.id = $2
             )",
        )
        .bind(user_id)
        .bind(job_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        Ok(if exists {
            JobMutation::InvalidState
        } else {
            JobMutation::NotFound
        })
    }
}

async fn update_unchanged_child(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    source_id: Uuid,
    manifest: &PreparedGitManifest,
    pipeline_identity: &str,
) -> QueriaResult<()> {
    sqlx::query(
        "update source_document
         set branch = $2, commit_sha = $3, indexed_at = now(), updated_at = now(),
             metadata = metadata || jsonb_build_object('last_pipeline_identity', $4::text)
         where id = $1",
    )
    .bind(source_id)
    .bind(&manifest.branch)
    .bind(&manifest.commit_sha)
    .bind(pipeline_identity)
    .execute(&mut **transaction)
    .await
    .map_err(to_infrastructure_error)?;
    Ok(())
}

async fn update_changed_child(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    source_id: Uuid,
    file: &PreparedFile,
    manifest: &PreparedGitManifest,
    pipeline_identity: &str,
) -> QueriaResult<()> {
    sqlx::query(
        "update source_document
         set title = $2, branch = $3, commit_sha = $4, content_hash = $5,
             metadata = $6, indexed_at = now(), is_active = true, updated_at = now()
         where id = $1",
    )
    .bind(source_id)
    .bind(&file.path)
    .bind(&manifest.branch)
    .bind(&manifest.commit_sha)
    .bind(&file.content_hash)
    .bind(file_metadata(file, manifest, pipeline_identity))
    .execute(&mut **transaction)
    .await
    .map_err(to_infrastructure_error)?;
    Ok(())
}

async fn insert_child_source(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    root: &RootSource,
    file: &PreparedFile,
    manifest: &PreparedGitManifest,
    pipeline_identity: &str,
) -> QueriaResult<Uuid> {
    sqlx::query(
        "insert into source_document(
           organization_id, project_id, source_root_id, kind, uri, title,
           source_path, branch, commit_sha, content_hash, metadata, indexed_at
         ) values ($1, $2, $3, 'git_repo', $4, $5, $6, $7, $8, $9, $10, now())
         returning id",
    )
    .bind(root.organization_id)
    .bind(root.project_id)
    .bind(root.id)
    .bind(format!("queria-git://{}/{}", root.id, file.path))
    .bind(&file.path)
    .bind(&file.path)
    .bind(&manifest.branch)
    .bind(&manifest.commit_sha)
    .bind(&file.content_hash)
    .bind(file_metadata(file, manifest, pipeline_identity))
    .fetch_one(&mut **transaction)
    .await
    .and_then(|row| row.try_get("id"))
    .map_err(to_infrastructure_error)
}

async fn retire_generated_knowledge(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    source_id: Uuid,
    status: &str,
) -> QueriaResult<()> {
    sqlx::query(
        "delete from chunk
         where knowledge_item_id in (
           select id from knowledge_item
           where source_document_id = $1 and generated_by = 'trusted_git_pipeline'
         )",
    )
    .bind(source_id)
    .execute(&mut **transaction)
    .await
    .map_err(to_infrastructure_error)?;
    sqlx::query(
        "update approval
         set status = 'rejected', reason = 'source replaced by Git ingestion', decided_at = now()
         where status = 'pending'
           and knowledge_item_id in (
             select id from knowledge_item
             where source_document_id = $1 and generated_by = 'trusted_git_pipeline'
           )",
    )
    .bind(source_id)
    .execute(&mut **transaction)
    .await
    .map_err(to_infrastructure_error)?;
    sqlx::query(
        "update knowledge_item
         set status = $2::knowledge_status, updated_at = now()
         where source_document_id = $1
           and generated_by = 'trusted_git_pipeline'
           and status in ('draft', 'proposed', 'approved')",
    )
    .bind(source_id)
    .bind(status)
    .execute(&mut **transaction)
    .await
    .map_err(to_infrastructure_error)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn index_file(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    root: &RootSource,
    source_id: Uuid,
    file: &PreparedFile,
    manifest: &PreparedGitManifest,
    pipeline_identity: &str,
    result: &mut ApplyManifestResult,
) -> QueriaResult<()> {
    for knowledge in &file.knowledge {
        let knowledge_id = sqlx::query(
            "insert into knowledge_item(
               organization_id, project_id, source_document_id, scope, status,
               title, body, category, tags, approved_at, stable_key, generated_by
             ) values (
               $1, $2, $3, 'project',
               case when $4 then 'approved'::knowledge_status else 'proposed'::knowledge_status end,
               $5, $6, $7, $8,
               case when $4 then now() else null end,
               $9, 'trusted_git_pipeline'
             ) returning id",
        )
        .bind(root.organization_id)
        .bind(root.project_id)
        .bind(source_id)
        .bind(root.trusted_auto_approve)
        .bind(format!("{}: {}", file.path, knowledge.title))
        .bind(&knowledge.body)
        .bind(&knowledge.category)
        .bind(vec!["git".to_owned(), file.parser.clone()])
        .bind(&knowledge.stable_key)
        .fetch_one(&mut **transaction)
        .await
        .and_then(|row| row.try_get::<Uuid, _>("id"))
        .map_err(to_infrastructure_error)?;
        sqlx::query(
            "insert into approval(
               knowledge_item_id, requested_by, status, reason, policy_snapshot, decided_at
             ) values (
               $1, $2,
               case when $3 then 'approved'::approval_status else 'pending'::approval_status end,
               case when $3 then 'trusted Git pipeline auto-approval' else null end,
               $4,
               case when $3 then now() else null end
             )",
        )
        .bind(knowledge_id)
        .bind(pipeline_identity)
        .bind(root.trusted_auto_approve)
        .bind(json!({
            "policy": "trusted_git_pipeline",
            "allowlisted": true,
            "secret_scan_passed": true,
            "pipeline_identity": pipeline_identity,
            "commit_sha": manifest.commit_sha
        }))
        .execute(&mut **transaction)
        .await
        .map_err(to_infrastructure_error)?;
        result.knowledge_items += 1;

        if root.trusted_auto_approve {
            for chunk in &knowledge.chunks {
                sqlx::query(
                    "insert into chunk(
                       knowledge_item_id, source_document_id, chunk_index, body,
                       token_count, content_hash, metadata
                     ) values ($1, $2, $3, $4, $5, $6, $7)",
                )
                .bind(knowledge_id)
                .bind(source_id)
                .bind(i32::try_from(chunk.chunk_index).map_err(|_| {
                    QueriaError::Validation("chunk index exceeds database range".to_owned())
                })?)
                .bind(&chunk.body)
                .bind(
                    i32::try_from(chunk.body.split_whitespace().count()).map_err(|_| {
                        QueriaError::Validation("token estimate exceeds database range".to_owned())
                    })?,
                )
                .bind(&chunk.content_hash)
                .bind(json!({
                    "line_start": chunk.line_start,
                    "line_end": chunk.line_end,
                    "citation_path": chunk.citation_path,
                    "parser": file.parser,
                    "commit_sha": manifest.commit_sha,
                    "pipeline_identity": pipeline_identity,
                    "stable_key": chunk.stable_key
                }))
                .execute(&mut **transaction)
                .await
                .map_err(to_infrastructure_error)?;
                result.chunks += 1;
            }
        }
    }
    insert_audit(
        transaction,
        root,
        pipeline_identity,
        "source.indexed",
        source_id,
        json!({
            "path": file.path,
            "content_hash": file.content_hash,
            "commit_sha": manifest.commit_sha,
            "knowledge_items": file.knowledge.len(),
            "auto_approved": root.trusted_auto_approve
        }),
    )
    .await?;
    result.indexed_files += 1;
    Ok(())
}

fn file_metadata(
    file: &PreparedFile,
    manifest: &PreparedGitManifest,
    pipeline_identity: &str,
) -> Value {
    json!({
        "parser": file.parser,
        "size_bytes": file.size_bytes,
        "commit_sha": manifest.commit_sha,
        "pipeline_identity": pipeline_identity,
        "generated": true
    })
}

async fn insert_audit(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    root: &RootSource,
    pipeline_identity: &str,
    action: &str,
    resource_id: Uuid,
    metadata: Value,
) -> QueriaResult<()> {
    sqlx::query(
        "insert into audit_log(
           organization_id, actor_type, actor_id, action,
           resource_type, resource_id, metadata
         ) values ($1, 'pipeline', $2, $3, 'source_document', $4, $5)",
    )
    .bind(root.organization_id)
    .bind(pipeline_identity)
    .bind(action)
    .bind(resource_id.to_string())
    .bind(metadata)
    .execute(&mut **transaction)
    .await
    .map_err(to_infrastructure_error)?;
    Ok(())
}

fn job_from_row(row: sqlx::postgres::PgRow) -> QueriaResult<IngestionJobRecord> {
    Ok(IngestionJobRecord {
        id: row.try_get("id").map_err(to_infrastructure_error)?,
        organization_id: row
            .try_get("organization_id")
            .map_err(to_infrastructure_error)?,
        project_id: row.try_get("project_id").map_err(to_infrastructure_error)?,
        source_document_id: row
            .try_get("source_document_id")
            .map_err(to_infrastructure_error)?,
        status: row.try_get("status").map_err(to_infrastructure_error)?,
        job_type: row.try_get("job_type").map_err(to_infrastructure_error)?,
        payload: row.try_get("payload").map_err(to_infrastructure_error)?,
        locked_by: row.try_get("locked_by").map_err(to_infrastructure_error)?,
        locked_at: row.try_get("locked_at").map_err(to_infrastructure_error)?,
        attempts: row.try_get("attempts").map_err(to_infrastructure_error)?,
        error_message: row
            .try_get("error_message")
            .map_err(to_infrastructure_error)?,
        result: row.try_get("result").unwrap_or_else(|_| json!({})),
        retry_of_id: row
            .try_get("retry_of_id")
            .map_err(to_infrastructure_error)?,
        cancel_requested_at: row
            .try_get("cancel_requested_at")
            .map_err(to_infrastructure_error)?,
        started_at: row.try_get("started_at").map_err(to_infrastructure_error)?,
        finished_at: row
            .try_get("finished_at")
            .map_err(to_infrastructure_error)?,
        created_at: row.try_get("created_at").map_err(to_infrastructure_error)?,
        updated_at: row.try_get("updated_at").map_err(to_infrastructure_error)?,
    })
}

fn to_infrastructure_error(error: sqlx::Error) -> QueriaError {
    QueriaError::Infrastructure(format!("ingestion repository failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claim_query_uses_skip_locked_and_atomic_update() {
        let normalized = CLAIM_JOB_SQL.to_ascii_lowercase();
        assert!(normalized.contains("for update skip locked"));
        assert!(normalized.contains("update ingestion_job"));
        assert!(normalized.contains("attempts = attempts + 1"));
        assert!(normalized.contains("job_type = 'git_ingestion'"));
    }

    #[test]
    fn job_ids_remain_typed_at_repository_boundary() {
        let raw = Uuid::now_v7();
        assert_eq!(IngestionJobId::from_uuid(raw).as_uuid(), raw);
        assert_ne!(queria_core::ids::ProjectId::new().as_uuid(), raw);
    }
}

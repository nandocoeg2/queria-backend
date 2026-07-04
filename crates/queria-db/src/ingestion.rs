use chrono::{DateTime, Utc};
use queria_core::ids::{IngestionJobId, SourceDocumentId};
use queria_core::{QueriaError, QueriaResult};
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;

const CLAIM_JOB_SQL: &str = "with candidate as (
       select id
       from ingestion_job
       where status = 'queued'
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

    pub async fn mark_succeeded(
        &self,
        job_id: IngestionJobId,
        result: &Value,
    ) -> QueriaResult<bool> {
        let updated = sqlx::query(
            "update ingestion_job
             set status = 'succeeded', result = $2, error_message = null,
                 finished_at = now(), locked_by = null, locked_at = null, updated_at = now()
             where id = $1 and status = 'running' and cancel_requested_at is null",
        )
        .bind(job_id.as_uuid())
        .bind(result)
        .execute(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .rows_affected();
        Ok(updated == 1)
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
    }

    #[test]
    fn job_ids_remain_typed_at_repository_boundary() {
        let raw = Uuid::now_v7();
        assert_eq!(IngestionJobId::from_uuid(raw).as_uuid(), raw);
        assert_ne!(queria_core::ids::ProjectId::new().as_uuid(), raw);
    }
}

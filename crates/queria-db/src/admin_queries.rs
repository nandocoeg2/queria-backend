use crate::repositories::KnowledgeItemRecord;
use chrono::{DateTime, Utc};
use queria_core::ids::SourceDocumentId;
use queria_core::{QueriaError, QueriaResult};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AuditLogRecord {
    pub id: Uuid,
    pub organization_id: Option<Uuid>,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub ip_hash: Option<String>,
    pub user_agent_hash: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SourceDocumentDetailRecord {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub kind: String,
    pub uri: String,
    pub title: String,
    pub source_path: Option<String>,
    pub branch: Option<String>,
    pub commit_sha: Option<String>,
    pub content_hash: String,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    pub latest_ingestion_id: Option<Uuid>,
    pub latest_ingestion_status: Option<String>,
    pub latest_ingestion_started_at: Option<DateTime<Utc>>,
    pub latest_ingestion_finished_at: Option<DateTime<Utc>>,
    pub latest_ingestion_error_message: Option<String>,
    pub latest_ingestion_result: Option<serde_json::Value>,

    pub chunks_pending: i64,
    pub chunks_processing: i64,
    pub chunks_ready: i64,
    pub chunks_failed: i64,
    pub chunks_stale: i64,

    pub content_preview: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct DashboardSummaryRecord {
    pub project_count: i64,
    pub source_count: i64,
    pub pending_approvals_count: i64,
    pub chunks_pending: i64,
    pub chunks_processing: i64,
    pub chunks_ready: i64,
    pub chunks_failed: i64,
    pub chunks_stale: i64,
    pub failed_jobs_count: i64,

    pub latest_ingestion_id: Option<Uuid>,
    pub latest_ingestion_status: Option<String>,
    pub latest_ingestion_started_at: Option<DateTime<Utc>>,
    pub latest_ingestion_finished_at: Option<DateTime<Utc>>,
    pub latest_ingestion_error_message: Option<String>,

    pub latest_evaluation_id: Option<Uuid>,
    pub latest_evaluation_project_slug: Option<String>,
    pub latest_evaluation_score: Option<f64>,
    pub latest_evaluation_passed: Option<bool>,
    pub latest_evaluation_created_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct PgAdminQueriesRepository {
    pool: PgPool,
}

impl PgAdminQueriesRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list_knowledge_items(
        &self,
        user_id: Uuid,
        scope: Option<&str>,
        project_slug: Option<&str>,
        category: Option<&str>,
        status: Option<&str>,
        tag: Option<&str>,
        cursor: Option<Uuid>,
        limit: u32,
    ) -> QueriaResult<Vec<KnowledgeItemRecord>> {
        // Limit max values to 100
        let page_limit = limit.min(100) as i64;
        let scope_enum = scope.map(|s| s.trim().to_lowercase());
        let status_enum = status.map(|s| s.trim().to_lowercase());

        let rows = sqlx::query(
            "select ki.id, ki.project_id, ki.source_document_id, ki.scope::text as scope,
                    ki.status::text as status, ki.title, ki.body, ki.category,
                    ki.tags, ki.approved_at, ki.created_at, ki.updated_at
             from knowledge_item ki
             join org_membership m on m.organization_id = ki.organization_id
             left join project p on p.id = ki.project_id
             where m.user_id = $1
               and ($2::text is null or ki.scope::text = $2)
               and ($3::text is null or p.slug = $3)
               and ($4::text is null or ki.category = $4)
               and ($5::text is null or ki.status::text = $5)
               and ($6::text is null or ki.tags @> array[$6])
               and ($7::uuid is null or ki.id < $7)
             order by ki.id desc
             limit $8",
        )
        .bind(user_id)
        .bind(scope_enum)
        .bind(project_slug)
        .bind(category)
        .bind(status_enum)
        .bind(tag)
        .bind(cursor)
        .bind(page_limit)
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        rows.into_iter().map(knowledge_item_from_row).collect()
    }

    pub async fn get_source_document_detail(
        &self,
        user_id: Uuid,
        source_document_id: SourceDocumentId,
    ) -> QueriaResult<Option<SourceDocumentDetailRecord>> {
        let source_uuid = source_document_id.as_uuid();

        // 1. Get the source document details
        let source_row = sqlx::query(
            "select sd.id, sd.project_id, sd.kind::text as kind, sd.uri, sd.title,
                    sd.source_path, sd.branch, sd.commit_sha, sd.content_hash,
                    sd.metadata, sd.created_at, sd.updated_at
             from source_document sd
             join org_membership m on m.organization_id = sd.organization_id
             where m.user_id = $1 and sd.id = $2",
        )
        .bind(user_id)
        .bind(source_uuid)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        let Some(s_row) = source_row else {
            return Ok(None);
        };

        // 2. Get the latest ingestion job
        let latest_job = sqlx::query(
            "select id, status::text as status, started_at, finished_at, error_message, result
             from ingestion_job
             where source_document_id = $1
             order by created_at desc
             limit 1",
        )
        .bind(source_uuid)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        // 3. Get chunk aggregates (including child files if it's a repository)
        let counts_row = sqlx::query(
            "select
               count(case when embedding_status = 'pending' then 1 end) as pending,
               count(case when embedding_status = 'processing' then 1 end) as processing,
               count(case when embedding_status = 'ready' then 1 end) as ready,
               count(case when embedding_status = 'failed' then 1 end) as failed,
               count(case when embedding_status = 'stale' then 1 end) as stale
             from chunk
             where source_document_id in (
                 select id from source_document where source_root_id = $1 or id = $1
             )",
        )
        .bind(source_uuid)
        .fetch_one(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        // 4. Get content preview from first approved knowledge item
        let preview: Option<String> = sqlx::query_scalar(
            "select body
             from knowledge_item
             where source_document_id in (
                 select id from source_document where source_root_id = $1 or id = $1
             ) and status = 'approved'
             order by id
             limit 1",
        )
        .bind(source_uuid)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        let preview_bounded = preview.map(|mut text| {
            if text.len() > 1000 {
                text.truncate(1000);
                text.push_str("\n... [truncated preview]");
            }
            text
        });

        Ok(Some(SourceDocumentDetailRecord {
            id: s_row.try_get("id").map_err(to_infrastructure_error)?,
            project_id: s_row
                .try_get("project_id")
                .map_err(to_infrastructure_error)?,
            kind: s_row.try_get("kind").map_err(to_infrastructure_error)?,
            uri: s_row.try_get("uri").map_err(to_infrastructure_error)?,
            title: s_row.try_get("title").map_err(to_infrastructure_error)?,
            source_path: s_row
                .try_get("source_path")
                .map_err(to_infrastructure_error)?,
            branch: s_row.try_get("branch").map_err(to_infrastructure_error)?,
            commit_sha: s_row
                .try_get("commit_sha")
                .map_err(to_infrastructure_error)?,
            content_hash: s_row
                .try_get("content_hash")
                .map_err(to_infrastructure_error)?,
            metadata: s_row.try_get("metadata").map_err(to_infrastructure_error)?,
            created_at: s_row
                .try_get("created_at")
                .map_err(to_infrastructure_error)?,
            updated_at: s_row
                .try_get("updated_at")
                .map_err(to_infrastructure_error)?,

            latest_ingestion_id: latest_job
                .as_ref()
                .map(|j| j.try_get("id"))
                .transpose()
                .map_err(to_infrastructure_error)?,
            latest_ingestion_status: latest_job
                .as_ref()
                .map(|j| j.try_get("status"))
                .transpose()
                .map_err(to_infrastructure_error)?,
            // Job columns may be SQL NULL even when a job row exists (e.g. succeeded
            // jobs clear error_message). Decode as Option and flatten into the field.
            latest_ingestion_started_at: latest_job
                .as_ref()
                .map(|j| j.try_get::<Option<DateTime<Utc>>, _>("started_at"))
                .transpose()
                .map_err(to_infrastructure_error)?
                .flatten(),
            latest_ingestion_finished_at: latest_job
                .as_ref()
                .map(|j| j.try_get::<Option<DateTime<Utc>>, _>("finished_at"))
                .transpose()
                .map_err(to_infrastructure_error)?
                .flatten(),
            latest_ingestion_error_message: latest_job
                .as_ref()
                .map(|j| j.try_get::<Option<String>, _>("error_message"))
                .transpose()
                .map_err(to_infrastructure_error)?
                .flatten(),
            latest_ingestion_result: latest_job
                .as_ref()
                .map(|j| j.try_get::<Option<serde_json::Value>, _>("result"))
                .transpose()
                .map_err(to_infrastructure_error)?
                .flatten(),

            chunks_pending: counts_row
                .try_get("pending")
                .map_err(to_infrastructure_error)?,
            chunks_processing: counts_row
                .try_get("processing")
                .map_err(to_infrastructure_error)?,
            chunks_ready: counts_row
                .try_get("ready")
                .map_err(to_infrastructure_error)?,
            chunks_failed: counts_row
                .try_get("failed")
                .map_err(to_infrastructure_error)?,
            chunks_stale: counts_row
                .try_get("stale")
                .map_err(to_infrastructure_error)?,

            content_preview: preview_bounded,
        }))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn list_audit_logs(
        &self,
        user_id: Uuid,
        actor_id: Option<&str>,
        action: Option<&str>,
        resource_type: Option<&str>,
        resource_id: Option<&str>,
        cursor: Option<Uuid>,
        limit: u32,
    ) -> QueriaResult<Vec<AuditLogRecord>> {
        let page_limit = limit.min(100) as i64;

        let rows = sqlx::query(
            "select al.id, al.organization_id, al.actor_type, al.actor_id,
                    al.action, al.resource_type, al.resource_id, al.ip_hash,
                    al.user_agent_hash, al.metadata, al.created_at
             from audit_log al
             join org_membership m on m.organization_id = al.organization_id
             where m.user_id = $1
               and ($2::text is null or al.actor_id = $2)
               and ($3::text is null or al.action = $3)
               and ($4::text is null or al.resource_type = $4)
               and ($5::text is null or al.resource_id = $5)
               and ($6::uuid is null or al.id < $6)
             order by al.id desc
             limit $7",
        )
        .bind(user_id)
        .bind(actor_id)
        .bind(action)
        .bind(resource_type)
        .bind(resource_id)
        .bind(cursor)
        .bind(page_limit)
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        rows.into_iter().map(audit_log_from_row).collect()
    }

    pub async fn get_dashboard_summary(
        &self,
        user_id: Uuid,
    ) -> QueriaResult<DashboardSummaryRecord> {
        // 1. Get organizational stats
        let stats = sqlx::query(
            "select
               (select count(*) from project p join org_membership m on m.organization_id = p.organization_id where m.user_id = $1) as project_count,
               (select count(*) from source_document sd join org_membership m on m.organization_id = sd.organization_id where m.user_id = $1 and sd.source_root_id is null and sd.is_active) as source_count,
               (select count(*) from approval a join knowledge_item ki on ki.id = a.knowledge_item_id join org_membership m on m.organization_id = ki.organization_id where m.user_id = $1 and a.status = 'pending') as pending_approvals,
               (select count(*) from ingestion_job j join org_membership m on m.organization_id = j.organization_id where m.user_id = $1 and j.status = 'failed' and j.created_at >= now() - interval '7 days') as failed_jobs"
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        // 2. Get chunk aggregates for the organization
        let chunks = sqlx::query(
            "select
               count(case when c.embedding_status = 'pending' then 1 end) as pending,
               count(case when c.embedding_status = 'processing' then 1 end) as processing,
               count(case when c.embedding_status = 'ready' then 1 end) as ready,
               count(case when c.embedding_status = 'failed' then 1 end) as failed,
               count(case when c.embedding_status = 'stale' then 1 end) as stale
             from chunk c
             join knowledge_item ki on ki.id = c.knowledge_item_id
             join org_membership m on m.organization_id = ki.organization_id
             where m.user_id = $1",
        )
        .bind(user_id)
        .fetch_one(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        // 3. Get the latest ingestion job in the organization
        let latest_job = sqlx::query(
            "select j.id, j.status::text as status, j.started_at, j.finished_at, j.error_message
             from ingestion_job j
             join org_membership m on m.organization_id = j.organization_id
             where m.user_id = $1
             order by j.created_at desc
             limit 1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        // 4. Get the latest evaluation run in the organization
        let latest_eval = sqlx::query(
            "select r.id, r.project_slug, r.regression_score::double precision as score, (r.status = 'passed') as passed, r.created_at
             from evaluation_report r
             join project p on p.slug = r.project_slug
             join org_membership m on m.organization_id = p.organization_id
             where m.user_id = $1
             order by r.created_at desc
             limit 1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        Ok(DashboardSummaryRecord {
            project_count: stats
                .try_get("project_count")
                .map_err(to_infrastructure_error)?,
            source_count: stats
                .try_get("source_count")
                .map_err(to_infrastructure_error)?,
            pending_approvals_count: stats
                .try_get("pending_approvals")
                .map_err(to_infrastructure_error)?,
            failed_jobs_count: stats
                .try_get("failed_jobs")
                .map_err(to_infrastructure_error)?,

            chunks_pending: chunks.try_get("pending").map_err(to_infrastructure_error)?,
            chunks_processing: chunks
                .try_get("processing")
                .map_err(to_infrastructure_error)?,
            chunks_ready: chunks.try_get("ready").map_err(to_infrastructure_error)?,
            chunks_failed: chunks.try_get("failed").map_err(to_infrastructure_error)?,
            chunks_stale: chunks.try_get("stale").map_err(to_infrastructure_error)?,

            latest_ingestion_id: latest_job
                .as_ref()
                .map(|j| j.try_get("id"))
                .transpose()
                .map_err(to_infrastructure_error)?,
            latest_ingestion_status: latest_job
                .as_ref()
                .map(|j| j.try_get("status"))
                .transpose()
                .map_err(to_infrastructure_error)?,
            latest_ingestion_started_at: latest_job
                .as_ref()
                .map(|j| j.try_get::<Option<DateTime<Utc>>, _>("started_at"))
                .transpose()
                .map_err(to_infrastructure_error)?
                .flatten(),
            latest_ingestion_finished_at: latest_job
                .as_ref()
                .map(|j| j.try_get::<Option<DateTime<Utc>>, _>("finished_at"))
                .transpose()
                .map_err(to_infrastructure_error)?
                .flatten(),
            latest_ingestion_error_message: latest_job
                .as_ref()
                .map(|j| j.try_get::<Option<String>, _>("error_message"))
                .transpose()
                .map_err(to_infrastructure_error)?
                .flatten(),

            latest_evaluation_id: latest_eval
                .as_ref()
                .map(|e| e.try_get("id"))
                .transpose()
                .map_err(to_infrastructure_error)?,
            latest_evaluation_project_slug: latest_eval
                .as_ref()
                .map(|e| e.try_get("project_slug"))
                .transpose()
                .map_err(to_infrastructure_error)?,
            latest_evaluation_score: latest_eval
                .as_ref()
                .map(|e| e.try_get("score"))
                .transpose()
                .map_err(to_infrastructure_error)?,
            latest_evaluation_passed: latest_eval
                .as_ref()
                .map(|e| e.try_get("passed"))
                .transpose()
                .map_err(to_infrastructure_error)?,
            latest_evaluation_created_at: latest_eval
                .as_ref()
                .map(|e| e.try_get("created_at"))
                .transpose()
                .map_err(to_infrastructure_error)?,
        })
    }
}

fn knowledge_item_from_row(row: sqlx::postgres::PgRow) -> QueriaResult<KnowledgeItemRecord> {
    let scope_str: String = row.try_get("scope").map_err(to_infrastructure_error)?;
    let status_str: String = row.try_get("status").map_err(to_infrastructure_error)?;

    Ok(KnowledgeItemRecord {
        id: row.try_get("id").map_err(to_infrastructure_error)?,
        project_id: row.try_get("project_id").map_err(to_infrastructure_error)?,
        source_document_id: row
            .try_get("source_document_id")
            .map_err(to_infrastructure_error)?,
        scope: scope_str,
        status: status_str,
        title: row.try_get("title").map_err(to_infrastructure_error)?,
        body: row.try_get("body").map_err(to_infrastructure_error)?,
        category: row.try_get("category").map_err(to_infrastructure_error)?,
        tags: row.try_get("tags").map_err(to_infrastructure_error)?,
        approved_at: row
            .try_get("approved_at")
            .map_err(to_infrastructure_error)?,
        created_at: row.try_get("created_at").map_err(to_infrastructure_error)?,
        updated_at: row.try_get("updated_at").map_err(to_infrastructure_error)?,
    })
}

fn audit_log_from_row(row: sqlx::postgres::PgRow) -> QueriaResult<AuditLogRecord> {
    Ok(AuditLogRecord {
        id: row.try_get("id").map_err(to_infrastructure_error)?,
        organization_id: row
            .try_get("organization_id")
            .map_err(to_infrastructure_error)?,
        actor_type: row.try_get("actor_type").map_err(to_infrastructure_error)?,
        actor_id: row.try_get("actor_id").map_err(to_infrastructure_error)?,
        action: row.try_get("action").map_err(to_infrastructure_error)?,
        resource_type: row
            .try_get("resource_type")
            .map_err(to_infrastructure_error)?,
        resource_id: row
            .try_get("resource_id")
            .map_err(to_infrastructure_error)?,
        ip_hash: row.try_get("ip_hash").map_err(to_infrastructure_error)?,
        user_agent_hash: row
            .try_get("user_agent_hash")
            .map_err(to_infrastructure_error)?,
        metadata: row.try_get("metadata").map_err(to_infrastructure_error)?,
        created_at: row.try_get("created_at").map_err(to_infrastructure_error)?,
    })
}

fn to_infrastructure_error(error: sqlx::Error) -> QueriaError {
    QueriaError::Infrastructure(format!("admin queries repository failed: {error}"))
}

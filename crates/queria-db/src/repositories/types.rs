use chrono::{DateTime, Utc};
use queria_core::auth::permissions::AgentTokenPermissions;
use queria_core::contracts::{Citation, KnowledgeLane, RetrievedContextItem};
use queria_core::ids::{ApprovalId, ChunkId, SourceDocumentId};
use queria_core::model::{KnowledgeScope, KnowledgeStatus};
use queria_core::{QueriaError, QueriaResult};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectRecord {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub default_embedding_model: String,
    pub include_global_default: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateProjectParams {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub default_embedding_model: String,
    pub include_global_default: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SourceDocumentRecord {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub kind: String,
    pub uri: String,
    pub title: String,
    pub source_path: Option<String>,
    pub branch: Option<String>,
    pub commit_sha: Option<String>,
    pub content_hash: String,
    pub metadata: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RegisterSourceDocumentParams {
    pub project_slug: String,
    pub kind: String,
    pub uri: String,
    pub title: String,
    pub source_path: Option<String>,
    pub branch: Option<String>,
    pub commit_sha: Option<String>,
    pub content_hash: String,
    pub metadata: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentTokenRecord {
    pub id: Uuid,
    pub name: String,
    pub token_prefix: String,
    pub allow_global_knowledge: bool,
    pub permissions: AgentTokenPermissions,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreateAgentTokenParams {
    pub name: String,
    pub token_prefix: String,
    pub token_hash: String,
    pub permissions: AgentTokenPermissions,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuthenticatedAgentToken {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub name: String,
    pub token_prefix: String,
    pub permissions: AgentTokenPermissions,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProposedMemoryRecord {
    pub knowledge_item_id: Uuid,
    pub status: String,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProposeMemoryParams {
    pub project_slug: String,
    pub title: String,
    pub body: String,
    pub category: String,
    pub tags: Vec<String>,
}

/// Params for agent `index_memory` (project-scoped scratch write, IMP-13/22).
#[derive(Clone, Debug, PartialEq)]
pub struct IndexMemoryParams {
    pub project_id: Option<Uuid>,
    pub project_slug: Option<String>,
    pub title: String,
    pub body: String,
    pub category: String,
    pub tags: Vec<String>,
    pub content_hash: String,
}

/// Result of inserting or no-op resolving a scratch knowledge item + chunk.
#[derive(Clone, Debug, PartialEq)]
pub struct IndexedMemoryRecord {
    pub knowledge_item_id: Uuid,
    pub chunk_id: Uuid,
    pub project_id: Uuid,
    pub organization_id: Uuid,
    pub status: String,
    pub scope: String,
    pub title: String,
    pub body: String,
    pub content_hash: String,
    pub created: bool,
}

/// Compact record returned after embedding is marked ready (or idempotent hit).
#[derive(Clone, Debug, PartialEq)]
pub struct IndexMemoryResult {
    pub knowledge_item_id: Uuid,
    pub chunk_id: Uuid,
    pub project_id: Uuid,
    pub status: String,
    pub scope: String,
    pub title: String,
    pub content_hash: String,
    pub created: bool,
    pub idempotent: bool,
}

/// Fields required to mark a scratch chunk embedding ready after sync embed.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkScratchChunkReadyParams {
    pub chunk_id: Uuid,
    pub qdrant_point_id: Uuid,
    pub embedding_content_hash: String,
    pub provider: String,
    pub model: String,
    pub dimension: i32,
    pub profile_version: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApprovalRecord {
    pub id: Uuid,
    pub knowledge_item_id: Uuid,
    pub project_id: Option<Uuid>,
    pub source_document_id: Option<Uuid>,
    pub scope: String,
    pub knowledge_status: String,
    pub title: String,
    pub body: String,
    pub category: String,
    pub tags: Vec<String>,
    pub requested_by: String,
    pub reviewer_user_id: Option<Uuid>,
    pub approval_status: String,
    pub reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub decided_at: Option<DateTime<Utc>>,
    pub approved_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KnowledgeItemRecord {
    pub id: Uuid,
    pub project_id: Option<Uuid>,
    pub source_document_id: Option<Uuid>,
    pub scope: String,
    pub status: String,
    pub title: String,
    pub body: String,
    pub category: String,
    pub tags: Vec<String>,
    pub approved_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ApprovedKnowledgeRecord {
    pub approval: ApprovalRecord,
    pub chunk_id: Uuid,
    pub source_document_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompleteSetupParams {
    pub organization_slug: String,
    pub organization_name: String,
    pub admin_email: String,
    pub password_hash: String,
    pub setup_token_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatedAdmin {
    pub organization_id: Uuid,
    pub user_id: Uuid,
    pub organization_slug: String,
    pub email: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthUser {
    pub id: Uuid,
    pub email: String,
    pub password_hash: String,
    /// Sole `org_membership.organization_id` when present (preferred over legacy).
    pub membership_organization_id: Option<Uuid>,
    /// Legacy `user_account.organization_id` (always set; NOT NULL in schema).
    pub organization_id: Uuid,
    pub is_platform_super_admin: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedSession {
    pub user_id: Uuid,
    pub email: String,
    pub expires_at: DateTime<Utc>,
    /// Home org for tenant routes; None for platform super-admin without membership.
    pub active_organization_id: Option<Uuid>,
    pub is_platform_super_admin: bool,
}

pub(crate) fn project_from_row(row: sqlx::postgres::PgRow) -> QueriaResult<ProjectRecord> {
    Ok(ProjectRecord {
        id: row.try_get("id").map_err(to_infrastructure_error)?,
        slug: row.try_get("slug").map_err(to_infrastructure_error)?,
        name: row.try_get("name").map_err(to_infrastructure_error)?,
        description: row
            .try_get("description")
            .map_err(to_infrastructure_error)?,
        default_embedding_model: row
            .try_get("default_embedding_model")
            .map_err(to_infrastructure_error)?,
        include_global_default: row
            .try_get("include_global_default")
            .map_err(to_infrastructure_error)?,
        created_at: row.try_get("created_at").map_err(to_infrastructure_error)?,
        updated_at: row.try_get("updated_at").map_err(to_infrastructure_error)?,
    })
}

pub(crate) fn source_from_row(row: sqlx::postgres::PgRow) -> QueriaResult<SourceDocumentRecord> {
    Ok(SourceDocumentRecord {
        id: row.try_get("id").map_err(to_infrastructure_error)?,
        project_id: row.try_get("project_id").map_err(to_infrastructure_error)?,
        kind: row.try_get("kind").map_err(to_infrastructure_error)?,
        uri: row.try_get("uri").map_err(to_infrastructure_error)?,
        title: row.try_get("title").map_err(to_infrastructure_error)?,
        source_path: row
            .try_get("source_path")
            .map_err(to_infrastructure_error)?,
        branch: row.try_get("branch").map_err(to_infrastructure_error)?,
        commit_sha: row.try_get("commit_sha").map_err(to_infrastructure_error)?,
        content_hash: row
            .try_get("content_hash")
            .map_err(to_infrastructure_error)?,
        metadata: row.try_get("metadata").map_err(to_infrastructure_error)?,
        created_at: row.try_get("created_at").map_err(to_infrastructure_error)?,
        updated_at: row.try_get("updated_at").map_err(to_infrastructure_error)?,
    })
}

pub(crate) fn approval_from_row(row: sqlx::postgres::PgRow) -> QueriaResult<ApprovalRecord> {
    Ok(ApprovalRecord {
        id: row.try_get("id").map_err(to_infrastructure_error)?,
        knowledge_item_id: row
            .try_get("knowledge_item_id")
            .map_err(to_infrastructure_error)?,
        project_id: row.try_get("project_id").map_err(to_infrastructure_error)?,
        source_document_id: row
            .try_get("source_document_id")
            .map_err(to_infrastructure_error)?,
        scope: row.try_get("scope").map_err(to_infrastructure_error)?,
        knowledge_status: row
            .try_get("knowledge_status")
            .map_err(to_infrastructure_error)?,
        title: row.try_get("title").map_err(to_infrastructure_error)?,
        body: row.try_get("body").map_err(to_infrastructure_error)?,
        category: row.try_get("category").map_err(to_infrastructure_error)?,
        tags: row.try_get("tags").map_err(to_infrastructure_error)?,
        requested_by: row
            .try_get("requested_by")
            .map_err(to_infrastructure_error)?,
        reviewer_user_id: row
            .try_get("reviewer_user_id")
            .map_err(to_infrastructure_error)?,
        approval_status: row
            .try_get("approval_status")
            .map_err(to_infrastructure_error)?,
        reason: row.try_get("reason").map_err(to_infrastructure_error)?,
        created_at: row.try_get("created_at").map_err(to_infrastructure_error)?,
        decided_at: row.try_get("decided_at").map_err(to_infrastructure_error)?,
        approved_at: row
            .try_get("approved_at")
            .map_err(to_infrastructure_error)?,
    })
}

pub(crate) fn knowledge_item_from_row(
    row: sqlx::postgres::PgRow,
) -> QueriaResult<KnowledgeItemRecord> {
    Ok(KnowledgeItemRecord {
        id: row.try_get("id").map_err(to_infrastructure_error)?,
        project_id: row.try_get("project_id").map_err(to_infrastructure_error)?,
        source_document_id: row
            .try_get("source_document_id")
            .map_err(to_infrastructure_error)?,
        scope: row.try_get("scope").map_err(to_infrastructure_error)?,
        status: row.try_get("status").map_err(to_infrastructure_error)?,
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

pub(crate) fn agent_token_from_row(row: sqlx::postgres::PgRow) -> QueriaResult<AgentTokenRecord> {
    let permissions: Value = row
        .try_get("permissions")
        .map_err(to_infrastructure_error)?;
    Ok(AgentTokenRecord {
        id: row.try_get("id").map_err(to_infrastructure_error)?,
        name: row.try_get("name").map_err(to_infrastructure_error)?,
        token_prefix: row
            .try_get("token_prefix")
            .map_err(to_infrastructure_error)?,
        allow_global_knowledge: row
            .try_get("allow_global_knowledge")
            .map_err(to_infrastructure_error)?,
        permissions: parse_agent_permissions(permissions)?,
        expires_at: row.try_get("expires_at").map_err(to_infrastructure_error)?,
        revoked_at: row.try_get("revoked_at").map_err(to_infrastructure_error)?,
        last_used_at: row
            .try_get("last_used_at")
            .map_err(to_infrastructure_error)?,
        created_at: row.try_get("created_at").map_err(to_infrastructure_error)?,
    })
}

pub(crate) fn authenticated_agent_token_from_row(
    row: sqlx::postgres::PgRow,
) -> QueriaResult<AuthenticatedAgentToken> {
    let permissions: Value = row
        .try_get("permissions")
        .map_err(to_infrastructure_error)?;
    Ok(AuthenticatedAgentToken {
        id: row.try_get("id").map_err(to_infrastructure_error)?,
        organization_id: row
            .try_get("organization_id")
            .map_err(to_infrastructure_error)?,
        name: row.try_get("name").map_err(to_infrastructure_error)?,
        token_prefix: row
            .try_get("token_prefix")
            .map_err(to_infrastructure_error)?,
        permissions: parse_agent_permissions(permissions)?,
    })
}

pub(crate) fn retrieved_item_from_row(
    row: sqlx::postgres::PgRow,
) -> QueriaResult<RetrievedContextItem> {
    let scope: String = row.try_get("scope").map_err(to_infrastructure_error)?;
    let source_document_id: Uuid = row
        .try_get("source_document_id")
        .map_err(to_infrastructure_error)?;

    // Legacy substring search remains approved-only; always lean trusted citation.
    let status = KnowledgeStatus::Approved;
    Ok(RetrievedContextItem {
        chunk_id: ChunkId::from_uuid(row.try_get("chunk_id").map_err(to_infrastructure_error)?),
        source_document_id: SourceDocumentId::from_uuid(source_document_id),
        scope: parse_knowledge_scope(&scope)?,
        status,
        lane: KnowledgeLane::from_status(status),
        title: row.try_get("title").map_err(to_infrastructure_error)?,
        body: row.try_get("body").map_err(to_infrastructure_error)?,
        citation: Citation {
            source_uri: row.try_get("source_uri").map_err(to_infrastructure_error)?,
            source_path: row
                .try_get("source_path")
                .map_err(to_infrastructure_error)?,
            line_start: parse_optional_u32(
                row.try_get::<Option<String>, _>("line_start")
                    .map_err(to_infrastructure_error)?,
            )?,
            line_end: parse_optional_u32(
                row.try_get::<Option<String>, _>("line_end")
                    .map_err(to_infrastructure_error)?,
            )?,
        },
        score: row.try_get("score").map_err(to_infrastructure_error)?,
    })
}

pub(crate) fn parse_agent_permissions(value: Value) -> QueriaResult<AgentTokenPermissions> {
    serde_json::from_value(value).map_err(|error| {
        QueriaError::Infrastructure(format!(
            "database returned invalid agent token permissions: {error}"
        ))
    })
}

pub(crate) fn parse_knowledge_scope(value: &str) -> QueriaResult<KnowledgeScope> {
    match value {
        "global" => Ok(KnowledgeScope::Global),
        "project" => Ok(KnowledgeScope::Project),
        _ => Err(QueriaError::Infrastructure(format!(
            "database returned unknown knowledge scope: {value}"
        ))),
    }
}

pub(crate) fn parse_optional_u32(value: Option<String>) -> QueriaResult<Option<u32>> {
    value
        .map(|raw| {
            raw.parse::<u32>().map_err(|error| {
                QueriaError::Infrastructure(format!("invalid chunk line metadata: {error}"))
            })
        })
        .transpose()
}

pub(crate) async fn organization_id_for_user(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
) -> QueriaResult<Uuid> {
    sqlx::query_scalar::<_, Uuid>("select organization_id from user_account where id = $1")
        .bind(user_id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(to_infrastructure_error)
}

pub(crate) async fn count_accessible_project_slugs(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    organization_id: Uuid,
    project_slugs: &[String],
) -> QueriaResult<i64> {
    sqlx::query_scalar::<_, i64>(
        "select count(*)
         from project
         where organization_id = $1
           and slug = any($2)",
    )
    .bind(organization_id)
    .bind(project_slugs)
    .fetch_one(&mut **transaction)
    .await
    .map_err(to_infrastructure_error)
}

pub(crate) async fn project_id_for_slug(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    organization_id: Uuid,
    project_slug: &str,
) -> QueriaResult<Option<Uuid>> {
    sqlx::query_scalar::<_, Uuid>(
        "select id
         from project
         where organization_id = $1
           and slug = $2",
    )
    .bind(organization_id)
    .bind(project_slug)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(to_infrastructure_error)
}

pub(crate) async fn approval_for_update(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    approval_id: ApprovalId,
) -> QueriaResult<Option<ApprovalRecord>> {
    sqlx::query(
        "select a.id, a.knowledge_item_id, ki.project_id, ki.source_document_id,
                ki.scope::text as scope, ki.status::text as knowledge_status,
                ki.title, ki.body, ki.category, ki.tags,
                a.requested_by, a.reviewer_user_id, a.status::text as approval_status,
                a.reason, a.created_at, a.decided_at, ki.approved_at
         from approval a
         join knowledge_item ki on ki.id = a.knowledge_item_id
         join user_account u on u.organization_id = ki.organization_id
         where u.id = $1
           and a.id = $2
         for update of a, ki",
    )
    .bind(user_id)
    .bind(approval_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(to_infrastructure_error)?
    .map(approval_from_row)
    .transpose()
}

pub(crate) async fn ensure_approval_source_document(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    approval: &ApprovalRecord,
) -> QueriaResult<Uuid> {
    if let Some(source_document_id) = approval.source_document_id {
        return Ok(source_document_id);
    }

    let organization_id = organization_id_for_user(transaction, user_id).await?;
    let uri = format!("queria://knowledge-items/{}", approval.knowledge_item_id);
    let content_hash = format!("knowledge_item:{}:source:v1", approval.knowledge_item_id);
    let metadata = json!({
        "generated_from": "approval",
        "approval_id": approval.id,
        "knowledge_item_id": approval.knowledge_item_id
    });

    sqlx::query(
        "insert into source_document(
           organization_id, project_id, kind, uri, title, source_path,
           content_hash, metadata
         )
         values ($1, $2, 'manual_note', $3, $4, $5, $6, $7)
         on conflict (organization_id, project_id, uri, content_hash) do update
         set updated_at = source_document.updated_at
         returning id",
    )
    .bind(organization_id)
    .bind(approval.project_id)
    .bind(uri)
    .bind(&approval.title)
    .bind(format!(
        "queria://knowledge-items/{}",
        approval.knowledge_item_id
    ))
    .bind(content_hash)
    .bind(metadata)
    .fetch_one(&mut **transaction)
    .await
    .and_then(|row| row.try_get::<Uuid, _>("id"))
    .map_err(to_infrastructure_error)
}

pub(crate) async fn insert_approval_audit_log(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    action: &str,
    approval_id: Uuid,
    knowledge_item_id: Uuid,
    metadata: Value,
) -> QueriaResult<()> {
    let organization_id = organization_id_for_user(transaction, user_id).await?;
    let metadata = json!({
        "approval_id": approval_id,
        "knowledge_item_id": knowledge_item_id,
        "details": metadata
    });

    sqlx::query(
        "insert into audit_log(
           organization_id, actor_type, actor_id, action,
           resource_type, resource_id, metadata
         )
         values ($1, 'user', $2, $3, 'approval', $4, $5)",
    )
    .bind(organization_id)
    .bind(user_id.to_string())
    .bind(action)
    .bind(approval_id.to_string())
    .bind(metadata)
    .execute(&mut **transaction)
    .await
    .map(|_| ())
    .map_err(to_infrastructure_error)
}

pub(crate) fn to_infrastructure_error(error: sqlx::Error) -> QueriaError {
    QueriaError::Infrastructure(format!("database repository failed: {error}"))
}

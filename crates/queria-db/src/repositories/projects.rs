use queria_core::contracts::RetrievedContextItem;
use queria_core::ids::{
    AgentTokenId, ApprovalId, KnowledgeItemId, ProjectId, SourceDocumentId,
};
use queria_core::{QueriaError, QueriaResult};
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::types::{
    AgentTokenRecord, ApprovalRecord, ApprovedKnowledgeRecord, AuthenticatedAgentToken,
    CreateAgentTokenParams, CreateProjectParams, KnowledgeItemRecord, ProjectRecord,
    ProposeMemoryParams, ProposedMemoryRecord, RegisterSourceDocumentParams,
    SourceDocumentRecord, agent_token_from_row, approval_for_update, approval_from_row,
    authenticated_agent_token_from_row, count_accessible_project_slugs,
    ensure_approval_source_document, insert_approval_audit_log, knowledge_item_from_row,
    organization_id_for_user, project_from_row, project_id_for_slug, retrieved_item_from_row,
    source_from_row, to_infrastructure_error,
};

#[derive(Clone, Debug)]
pub struct PgProjectRepository {
    pool: PgPool,
}

impl PgProjectRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn list_projects(&self, user_id: Uuid) -> QueriaResult<Vec<ProjectRecord>> {
        sqlx::query(
            "select p.id, p.slug, p.name, p.description, p.default_embedding_model,
                    p.include_global_default, p.created_at, p.updated_at
             from project p
             join user_account u on u.organization_id = p.organization_id
             where u.id = $1
             order by p.slug",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .into_iter()
        .map(project_from_row)
        .collect()
    }

    pub async fn get_project_by_slug(
        &self,
        user_id: Uuid,
        slug: &str,
    ) -> QueriaResult<Option<ProjectRecord>> {
        sqlx::query(
            "select p.id, p.slug, p.name, p.description, p.default_embedding_model,
                    p.include_global_default, p.created_at, p.updated_at
             from project p
             join user_account u on u.organization_id = p.organization_id
             where u.id = $1
               and p.slug = $2",
        )
        .bind(user_id)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(project_from_row)
        .transpose()
    }

    pub async fn create_project(
        &self,
        user_id: Uuid,
        params: CreateProjectParams,
    ) -> QueriaResult<ProjectRecord> {
        let row = sqlx::query(
            "with requester as (
               select organization_id
               from user_account
               where id = $1
             )
             insert into project(
               organization_id, slug, name, description,
               default_embedding_model, include_global_default
             )
             select organization_id, $2, $3, $4, $5, $6
             from requester
             on conflict (organization_id, slug) do nothing
             returning id, slug, name, description, default_embedding_model,
                       include_global_default, created_at, updated_at",
        )
        .bind(user_id)
        .bind(&params.slug)
        .bind(&params.name)
        .bind(&params.description)
        .bind(&params.default_embedding_model)
        .bind(params.include_global_default)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        let Some(row) = row else {
            return Err(QueriaError::Validation(
                "project slug already exists or requester does not exist".to_owned(),
            ));
        };

        project_from_row(row)
    }

    pub async fn register_source_document(
        &self,
        user_id: Uuid,
        params: RegisterSourceDocumentParams,
    ) -> QueriaResult<SourceDocumentRecord> {
        let row = sqlx::query(
            "with scoped_project as (
               select p.id as project_id, p.organization_id
               from project p
               join user_account u on u.organization_id = p.organization_id
               where u.id = $1
                 and p.slug = $2
             )
             insert into source_document(
               organization_id, project_id, kind, uri, title, source_path,
               branch, commit_sha, content_hash, metadata
             )
             select organization_id, project_id, $3::source_kind, $4, $5, $6,
                    $7, $8, $9, $10
             from scoped_project
             on conflict (organization_id, project_id, uri, content_hash) do nothing
             returning id, project_id, kind::text as kind, uri, title, source_path,
                       branch, commit_sha, content_hash, metadata, created_at, updated_at",
        )
        .bind(user_id)
        .bind(&params.project_slug)
        .bind(&params.kind)
        .bind(&params.uri)
        .bind(&params.title)
        .bind(&params.source_path)
        .bind(&params.branch)
        .bind(&params.commit_sha)
        .bind(&params.content_hash)
        .bind(&params.metadata)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        let Some(row) = row else {
            return Err(QueriaError::Validation(
                "source already exists or project is not accessible".to_owned(),
            ));
        };

        source_from_row(row)
    }

    pub async fn list_source_documents(
        &self,
        user_id: Uuid,
        project_slug: &str,
    ) -> QueriaResult<Vec<SourceDocumentRecord>> {
        sqlx::query(
            "select sd.id, sd.project_id, sd.kind::text as kind, sd.uri, sd.title,
                    sd.source_path, sd.branch, sd.commit_sha, sd.content_hash,
                    sd.metadata, sd.created_at, sd.updated_at
             from source_document sd
             join project p on p.id = sd.project_id
             join user_account u on u.organization_id = sd.organization_id
             where u.id = $1
               and p.slug = $2
               and sd.source_root_id is null
             order by sd.created_at desc, sd.title",
        )
        .bind(user_id)
        .bind(project_slug)
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .into_iter()
        .map(source_from_row)
        .collect()
    }

    pub async fn get_source_document(
        &self,
        user_id: Uuid,
        source_document_id: SourceDocumentId,
    ) -> QueriaResult<Option<SourceDocumentRecord>> {
        sqlx::query(
            "select sd.id, sd.project_id, sd.kind::text as kind, sd.uri, sd.title,
                    sd.source_path, sd.branch, sd.commit_sha, sd.content_hash,
                    sd.metadata, sd.created_at, sd.updated_at
             from source_document sd
             join user_account u on u.organization_id = sd.organization_id
             where u.id = $1
               and sd.id = $2",
        )
        .bind(user_id)
        .bind(source_document_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(source_from_row)
        .transpose()
    }

    pub async fn search_approved_chunks(
        &self,
        user_id: Uuid,
        project_id: ProjectId,
        query: &str,
        include_global: bool,
        limit: u32,
    ) -> QueriaResult<Vec<RetrievedContextItem>> {
        let pattern = format!("%{}%", query.trim());
        sqlx::query(
            "select c.id as chunk_id,
                    coalesce(c.source_document_id, ki.source_document_id) as source_document_id,
                    ki.scope::text as scope,
                    ki.title,
                    c.body,
                    coalesce(sd.uri, '') as source_uri,
                    sd.source_path,
                    c.metadata->>'line_start' as line_start,
                    c.metadata->>'line_end' as line_end,
                    case
                      when c.body ilike $4 then 1.0::real
                      when ki.title ilike $4 then 0.8::real
                      else 0.5::real
                    end as score
             from chunk c
             join knowledge_item ki on ki.id = c.knowledge_item_id
             left join source_document sd on sd.id = coalesce(c.source_document_id, ki.source_document_id)
             join user_account u on u.organization_id = ki.organization_id
             where u.id = $1
               and ki.status = 'approved'
               and coalesce(c.source_document_id, ki.source_document_id) is not null
               and exists (
                 select 1
                 from project p
                 join user_account requester on requester.organization_id = p.organization_id
                 where requester.id = $1
                   and p.id = $2
               )
               and (
                 (ki.scope = 'project' and ki.project_id = $2)
                 or (ki.scope = 'global' and $3 and ki.project_id is null)
               )
               and (
                 c.body ilike $4
                 or ki.title ilike $4
                 or ki.category ilike $4
               )
             order by
               case when ki.scope = 'project' then 0 else 1 end,
               score desc,
               c.created_at desc
             limit $5",
        )
        .bind(user_id)
        .bind(project_id.as_uuid())
        .bind(include_global)
        .bind(&pattern)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .into_iter()
        .map(retrieved_item_from_row)
        .collect()
    }

    pub async fn create_agent_token(
        &self,
        user_id: Uuid,
        params: CreateAgentTokenParams,
    ) -> QueriaResult<AgentTokenRecord> {
        if params.permissions.project_slugs.is_empty() {
            return Err(QueriaError::Validation(
                "agent token must allow at least one project".to_owned(),
            ));
        }

        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;
        let organization_id = organization_id_for_user(&mut transaction, user_id).await?;
        let allowed_project_count = count_accessible_project_slugs(
            &mut transaction,
            organization_id,
            &params.permissions.project_slugs,
        )
        .await?;

        if allowed_project_count != params.permissions.project_slugs.len() as i64 {
            return Err(QueriaError::Validation(
                "agent token contains an inaccessible project slug".to_owned(),
            ));
        }

        let primary_project_id = if params.permissions.project_slugs.len() == 1 {
            project_id_for_slug(
                &mut transaction,
                organization_id,
                &params.permissions.project_slugs[0],
            )
            .await?
        } else {
            None
        };

        let permissions_json = serde_json::to_value(&params.permissions).map_err(|error| {
            QueriaError::Validation(format!("invalid agent token permissions: {error}"))
        })?;

        let row = sqlx::query(
            "insert into agent_token(
               organization_id, project_id, name, token_prefix, token_hash,
               allow_global_knowledge, permissions, expires_at
             )
             values ($1, $2, $3, $4, $5, $6, $7, $8)
             returning id, name, token_prefix, allow_global_knowledge, permissions,
                       expires_at, revoked_at, last_used_at, created_at",
        )
        .bind(organization_id)
        .bind(primary_project_id)
        .bind(&params.name)
        .bind(&params.token_prefix)
        .bind(&params.token_hash)
        .bind(params.permissions.allow_global_knowledge)
        .bind(&permissions_json)
        .bind(params.expires_at)
        .fetch_one(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        agent_token_from_row(row)
    }

    pub async fn list_agent_tokens(&self, user_id: Uuid) -> QueriaResult<Vec<AgentTokenRecord>> {
        sqlx::query(
            "select at.id, at.name, at.token_prefix, at.allow_global_knowledge,
                    at.permissions, at.expires_at, at.revoked_at,
                    at.last_used_at, at.created_at
             from agent_token at
             join user_account u on u.organization_id = at.organization_id
             where u.id = $1
             order by at.created_at desc",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .into_iter()
        .map(agent_token_from_row)
        .collect()
    }

    pub async fn get_agent_token(
        &self,
        user_id: Uuid,
        agent_token_id: AgentTokenId,
    ) -> QueriaResult<Option<AgentTokenRecord>> {
        sqlx::query(
            "select at.id, at.name, at.token_prefix, at.allow_global_knowledge,
                    at.permissions, at.expires_at, at.revoked_at,
                    at.last_used_at, at.created_at
             from agent_token at
             join user_account u on u.organization_id = at.organization_id
             where u.id = $1
               and at.id = $2",
        )
        .bind(user_id)
        .bind(agent_token_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(agent_token_from_row)
        .transpose()
    }

    pub async fn revoke_agent_token(
        &self,
        user_id: Uuid,
        agent_token_id: AgentTokenId,
    ) -> QueriaResult<Option<AgentTokenRecord>> {
        sqlx::query(
            "update agent_token at
             set revoked_at = coalesce(at.revoked_at, now())
             from user_account u
             where u.organization_id = at.organization_id
               and u.id = $1
               and at.id = $2
             returning at.id, at.name, at.token_prefix, at.allow_global_knowledge,
                       at.permissions, at.expires_at, at.revoked_at,
                       at.last_used_at, at.created_at",
        )
        .bind(user_id)
        .bind(agent_token_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(agent_token_from_row)
        .transpose()
    }

    pub async fn authenticate_agent_token(
        &self,
        token_hash: &str,
    ) -> QueriaResult<Option<AuthenticatedAgentToken>> {
        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;
        let row = sqlx::query(
            "select id, organization_id, name, token_prefix, permissions
             from agent_token
             where token_hash = $1
               and revoked_at is null
               and (expires_at is null or expires_at > now())",
        )
        .bind(token_hash)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let Some(row) = row else {
            return Ok(None);
        };

        let token_id: Uuid = row.try_get("id").map_err(to_infrastructure_error)?;
        sqlx::query("update agent_token set last_used_at = now() where id = $1")
            .bind(token_id)
            .execute(&mut *transaction)
            .await
            .map_err(to_infrastructure_error)?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        authenticated_agent_token_from_row(row).map(Some)
    }

    pub async fn list_projects_for_agent(
        &self,
        agent: &AuthenticatedAgentToken,
    ) -> QueriaResult<Vec<ProjectRecord>> {
        sqlx::query(
            "select p.id, p.slug, p.name, p.description, p.default_embedding_model,
                    p.include_global_default, p.created_at, p.updated_at
             from project p
             where p.organization_id = $1
               and p.slug = any($2)
             order by p.slug",
        )
        .bind(agent.organization_id)
        .bind(&agent.permissions.project_slugs)
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .into_iter()
        .map(project_from_row)
        .collect()
    }

    pub async fn get_source_document_for_agent(
        &self,
        agent: &AuthenticatedAgentToken,
        source_document_id: SourceDocumentId,
    ) -> QueriaResult<Option<SourceDocumentRecord>> {
        sqlx::query(
            "select sd.id, sd.project_id, sd.kind::text as kind, sd.uri, sd.title,
                    sd.source_path, sd.branch, sd.commit_sha, sd.content_hash,
                    sd.metadata, sd.created_at, sd.updated_at
             from source_document sd
             join project p on p.id = sd.project_id
             where sd.organization_id = $1
               and sd.id = $2
               and p.slug = any($3)",
        )
        .bind(agent.organization_id)
        .bind(source_document_id.as_uuid())
        .bind(&agent.permissions.project_slugs)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(source_from_row)
        .transpose()
    }

    pub async fn search_approved_chunks_for_agent(
        &self,
        agent: &AuthenticatedAgentToken,
        project_id: ProjectId,
        query: &str,
        include_global: bool,
        limit: u32,
    ) -> QueriaResult<Vec<RetrievedContextItem>> {
        let pattern = format!("%{}%", query.trim());
        let allow_global = include_global && agent.permissions.allow_global_knowledge;
        sqlx::query(
            "select c.id as chunk_id,
                    coalesce(c.source_document_id, ki.source_document_id) as source_document_id,
                    ki.scope::text as scope,
                    ki.title,
                    c.body,
                    coalesce(sd.uri, '') as source_uri,
                    sd.source_path,
                    c.metadata->>'line_start' as line_start,
                    c.metadata->>'line_end' as line_end,
                    case
                      when c.body ilike $5 then 1.0::real
                      when ki.title ilike $5 then 0.8::real
                      else 0.5::real
                    end as score
             from chunk c
             join knowledge_item ki on ki.id = c.knowledge_item_id
             left join source_document sd on sd.id = coalesce(c.source_document_id, ki.source_document_id)
             where ki.organization_id = $1
               and ki.status = 'approved'
               and coalesce(c.source_document_id, ki.source_document_id) is not null
               and exists (
                 select 1
                 from project p
                 where p.organization_id = $1
                   and p.id = $2
                   and p.slug = any($3)
               )
               and (
                 (ki.scope = 'project' and ki.project_id = $2)
                 or (ki.scope = 'global' and $4 and ki.project_id is null)
               )
               and (
                 c.body ilike $5
                 or ki.title ilike $5
                 or ki.category ilike $5
               )
             order by
               case when ki.scope = 'project' then 0 else 1 end,
               score desc,
               c.created_at desc
             limit $6",
        )
        .bind(agent.organization_id)
        .bind(project_id.as_uuid())
        .bind(&agent.permissions.project_slugs)
        .bind(allow_global)
        .bind(&pattern)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .into_iter()
        .map(retrieved_item_from_row)
        .collect()
    }

    pub async fn propose_memory_for_agent(
        &self,
        agent: &AuthenticatedAgentToken,
        params: ProposeMemoryParams,
    ) -> QueriaResult<ProposedMemoryRecord> {
        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;
        let project_id = sqlx::query_scalar::<_, Uuid>(
            "select id
             from project
             where organization_id = $1
               and slug = $2
               and slug = any($3)",
        )
        .bind(agent.organization_id)
        .bind(&params.project_slug)
        .bind(&agent.permissions.project_slugs)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let Some(project_id) = project_id else {
            return Err(QueriaError::PermissionDenied);
        };

        let knowledge_item_id = sqlx::query(
            "insert into knowledge_item(
               organization_id, project_id, scope, status, title, body, category, tags
             )
             values ($1, $2, 'project', 'proposed', $3, $4, $5, $6)
             returning id",
        )
        .bind(agent.organization_id)
        .bind(project_id)
        .bind(&params.title)
        .bind(&params.body)
        .bind(&params.category)
        .bind(&params.tags)
        .fetch_one(&mut *transaction)
        .await
        .and_then(|row| row.try_get::<Uuid, _>("id"))
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "insert into approval(knowledge_item_id, requested_by, status)
             values ($1, $2, 'pending')",
        )
        .bind(knowledge_item_id)
        .bind(format!("agent:{}:{}", agent.token_prefix, agent.name))
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        Ok(ProposedMemoryRecord {
            knowledge_item_id,
            status: "proposed".to_owned(),
            title: params.title,
        })
    }

    pub async fn list_approvals(
        &self,
        user_id: Uuid,
        status: Option<&str>,
    ) -> QueriaResult<Vec<ApprovalRecord>> {
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
               and ($2::text is null or a.status::text = $2)
             order by a.created_at desc",
        )
        .bind(user_id)
        .bind(status)
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .into_iter()
        .map(approval_from_row)
        .collect()
    }

    pub async fn get_approval(
        &self,
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
               and a.id = $2",
        )
        .bind(user_id)
        .bind(approval_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(approval_from_row)
        .transpose()
    }

    pub async fn get_knowledge_item(
        &self,
        user_id: Uuid,
        knowledge_item_id: KnowledgeItemId,
    ) -> QueriaResult<Option<KnowledgeItemRecord>> {
        sqlx::query(
            "select ki.id, ki.project_id, ki.source_document_id, ki.scope::text as scope,
                    ki.status::text as status, ki.title, ki.body, ki.category,
                    ki.tags, ki.approved_at, ki.created_at, ki.updated_at
             from knowledge_item ki
             join user_account u on u.organization_id = ki.organization_id
             where u.id = $1
               and ki.id = $2",
        )
        .bind(user_id)
        .bind(knowledge_item_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(knowledge_item_from_row)
        .transpose()
    }

    pub async fn approve_approval(
        &self,
        user_id: Uuid,
        approval_id: ApprovalId,
    ) -> QueriaResult<Option<ApprovedKnowledgeRecord>> {
        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;
        let approval = approval_for_update(&mut transaction, user_id, approval_id).await?;
        let Some(mut approval) = approval else {
            return Ok(None);
        };

        if approval.approval_status != "pending" || approval.knowledge_status != "proposed" {
            return Err(QueriaError::Validation(
                "approval is not pending for a proposed knowledge item".to_owned(),
            ));
        }

        let source_document_id =
            ensure_approval_source_document(&mut transaction, user_id, &approval).await?;

        sqlx::query(
            "update knowledge_item
             set status = 'approved',
                 source_document_id = $2,
                 approved_at = now(),
                 updated_at = now()
             where id = $1",
        )
        .bind(approval.knowledge_item_id)
        .bind(source_document_id)
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "update approval
             set status = 'approved',
                 reviewer_user_id = $2,
                 decided_at = now()
             where id = $1",
        )
        .bind(approval.id)
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let chunk_id = sqlx::query(
            "insert into chunk(
               knowledge_item_id, source_document_id, chunk_index, body,
               token_count, content_hash, metadata
             )
             values ($1, $2, 0, $3, 0, $4, $5)
             on conflict (knowledge_item_id, chunk_index) do update
             set source_document_id = excluded.source_document_id,
                 body = excluded.body,
                 content_hash = excluded.content_hash,
                 metadata = excluded.metadata
             returning id",
        )
        .bind(approval.knowledge_item_id)
        .bind(source_document_id)
        .bind(&approval.body)
        .bind(format!(
            "knowledge_item:{}:approved:v1",
            approval.knowledge_item_id
        ))
        .bind(json!({
            "approval_id": approval.id,
            "line_start": 1,
            "line_end": approval.body.lines().count().max(1)
        }))
        .fetch_one(&mut *transaction)
        .await
        .and_then(|row| row.try_get::<Uuid, _>("id"))
        .map_err(to_infrastructure_error)?;

        insert_approval_audit_log(
            &mut transaction,
            user_id,
            "approval.approved",
            approval.id,
            approval.knowledge_item_id,
            json!({
                "chunk_id": chunk_id,
                "source_document_id": source_document_id
            }),
        )
        .await?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        approval.source_document_id = Some(source_document_id);
        approval.approval_status = "approved".to_owned();
        approval.knowledge_status = "approved".to_owned();
        approval.reviewer_user_id = Some(user_id);

        Ok(Some(ApprovedKnowledgeRecord {
            approval,
            chunk_id,
            source_document_id,
        }))
    }

    pub async fn reject_approval(
        &self,
        user_id: Uuid,
        approval_id: ApprovalId,
        reason: String,
    ) -> QueriaResult<Option<ApprovalRecord>> {
        let reason = reason.trim().to_owned();
        if reason.is_empty() || reason.len() > 2_000 {
            return Err(QueriaError::Validation(
                "rejection reason must be between 1 and 2000 bytes".to_owned(),
            ));
        }

        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;
        let approval = approval_for_update(&mut transaction, user_id, approval_id).await?;
        let Some(mut approval) = approval else {
            return Ok(None);
        };

        if approval.approval_status != "pending" || approval.knowledge_status != "proposed" {
            return Err(QueriaError::Validation(
                "approval is not pending for a proposed knowledge item".to_owned(),
            ));
        }

        sqlx::query(
            "update knowledge_item
             set status = 'rejected',
                 updated_at = now()
             where id = $1",
        )
        .bind(approval.knowledge_item_id)
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "update approval
             set status = 'rejected',
                 reviewer_user_id = $2,
                 reason = $3,
                 decided_at = now()
             where id = $1",
        )
        .bind(approval.id)
        .bind(user_id)
        .bind(&reason)
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        insert_approval_audit_log(
            &mut transaction,
            user_id,
            "approval.rejected",
            approval.id,
            approval.knowledge_item_id,
            json!({ "reason": reason }),
        )
        .await?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        approval.approval_status = "rejected".to_owned();
        approval.knowledge_status = "rejected".to_owned();
        approval.reviewer_user_id = Some(user_id);
        approval.reason = Some(reason);

        Ok(Some(approval))
    }

    pub async fn seed_fjulian_me_registry(&self) -> QueriaResult<()> {
        sqlx::query(
            "with first_org as (
               select id as organization_id
               from organization
               order by created_at asc
               limit 1
             ),
             upsert_project as (
               insert into project(
                 organization_id, slug, name, description,
                 default_embedding_model, include_global_default
               )
               select organization_id, 'fjulian-me', 'fjulian.me',
                      'Personal Astro site used as the first Queria source registry project.',
                      'voyage-4', true
               from first_org
               on conflict (organization_id, slug) do nothing
               returning id, organization_id
             ),
             scoped_project as (
               select id, organization_id
               from upsert_project
               union all
               select p.id, p.organization_id
               from project p
               join first_org o on o.organization_id = p.organization_id
               where p.slug = 'fjulian-me'
               limit 1
             )
             insert into source_document(
               organization_id, project_id, kind, uri, title, source_path,
               branch, commit_sha, content_hash, metadata
             )
             select organization_id, id, 'git_repo', 'file:///Users/fernandojulian/project/fjulian/fjulian.me',
                    'fjulian.me Git repository', '/Users/fernandojulian/project/fjulian/fjulian.me',
                    null, null, 'registry:fjulian-me:/Users/fernandojulian/project/fjulian/fjulian.me',
                    '{\"seeded\":true,\"seed\":\"first_project_registry\"}'::jsonb
             from scoped_project
             on conflict (organization_id, project_id, uri, content_hash) do nothing",
        )
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(to_infrastructure_error)
    }
}

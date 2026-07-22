use queria_core::contracts::RetrievedContextItem;
use queria_core::ids::{AgentTokenId, ApprovalId, KnowledgeItemId, ProjectId, SourceDocumentId};
use queria_core::{QueriaError, QueriaResult};
use serde_json::json;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::types::{
    AgentTokenRecord, ApprovalRecord, ApprovedKnowledgeRecord, AuthenticatedAgentToken,
    CreateAgentTokenParams, CreateProjectParams, IndexLocalFileParams, IndexMemoryParams,
    IndexedLocalFileRecord, IndexedMemoryRecord, KnowledgeItemRecord, MarkScratchChunkReadyParams,
    NeedsReviewActionRecord, NeedsReviewItemRecord, ProjectRecord, ProposeMemoryParams,
    ProposedMemoryRecord, RegisterSourceDocumentParams, SourceDocumentRecord, agent_token_from_row,
    approval_for_update, approval_from_row, authenticated_agent_token_from_row,
    count_accessible_project_slugs, ensure_approval_source_document, insert_approval_audit_log,
    knowledge_item_from_row, needs_review_item_from_row, organization_id_for_user,
    project_from_row, project_id_for_slug, retrieved_item_from_row, source_from_row,
    to_infrastructure_error,
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
             join org_membership m on m.organization_id = p.organization_id
             where m.user_id = $1
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
             join org_membership m on m.organization_id = p.organization_id
             where m.user_id = $1
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
               from org_membership
               where user_id = $1
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
               join org_membership m on m.organization_id = p.organization_id
               where m.user_id = $1
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
             join org_membership m on m.organization_id = sd.organization_id
             where m.user_id = $1
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
             join org_membership m on m.organization_id = sd.organization_id
             where m.user_id = $1
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
             join org_membership m on m.organization_id = ki.organization_id
             where m.user_id = $1
               and ki.status = 'approved'
               and coalesce(c.source_document_id, ki.source_document_id) is not null
               and exists (
                 select 1
                 from project p
                 join org_membership requester on requester.organization_id = p.organization_id
                 where requester.user_id = $1
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
             join org_membership m on m.organization_id = at.organization_id
             where m.user_id = $1
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
             join org_membership m on m.organization_id = at.organization_id
             where m.user_id = $1
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
             from org_membership m
             where m.organization_id = at.organization_id
               and m.user_id = $1
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

    /// Resolve an agent-accessible project by id and/or slug (IMP-13).
    ///
    /// Rejects requests that do not resolve to a single project in the token scope.
    /// Never returns a global scope target; scratch is always project-scoped.
    pub async fn resolve_project_for_agent(
        &self,
        agent: &AuthenticatedAgentToken,
        project_id: Option<Uuid>,
        project_slug: Option<&str>,
    ) -> QueriaResult<ProjectRecord> {
        let slug = project_slug
            .map(str::trim)
            .filter(|value| !value.is_empty());

        match (project_id, slug) {
            (Some(id), Some(slug)) => {
                let row = sqlx::query(
                    "select p.id, p.slug, p.name, p.description, p.default_embedding_model,
                            p.include_global_default, p.created_at, p.updated_at
                     from project p
                     where p.organization_id = $1
                       and p.id = $2
                       and p.slug = $3
                       and p.slug = any($4)",
                )
                .bind(agent.organization_id)
                .bind(id)
                .bind(slug)
                .bind(&agent.permissions.project_slugs)
                .fetch_optional(&self.pool)
                .await
                .map_err(to_infrastructure_error)?;
                row.map(project_from_row)
                    .transpose()?
                    .ok_or(QueriaError::PermissionDenied)
            }
            (Some(id), None) => {
                let row = sqlx::query(
                    "select p.id, p.slug, p.name, p.description, p.default_embedding_model,
                            p.include_global_default, p.created_at, p.updated_at
                     from project p
                     where p.organization_id = $1
                       and p.id = $2
                       and p.slug = any($3)",
                )
                .bind(agent.organization_id)
                .bind(id)
                .bind(&agent.permissions.project_slugs)
                .fetch_optional(&self.pool)
                .await
                .map_err(to_infrastructure_error)?;
                row.map(project_from_row)
                    .transpose()?
                    .ok_or(QueriaError::PermissionDenied)
            }
            (None, Some(slug)) => {
                let row = sqlx::query(
                    "select p.id, p.slug, p.name, p.description, p.default_embedding_model,
                            p.include_global_default, p.created_at, p.updated_at
                     from project p
                     where p.organization_id = $1
                       and p.slug = $2
                       and p.slug = any($3)",
                )
                .bind(agent.organization_id)
                .bind(slug)
                .bind(&agent.permissions.project_slugs)
                .fetch_optional(&self.pool)
                .await
                .map_err(to_infrastructure_error)?;
                row.map(project_from_row)
                    .transpose()?
                    .ok_or(QueriaError::PermissionDenied)
            }
            (None, None) => Err(QueriaError::Validation("invalid_project".to_owned())),
        }
    }

    /// Insert project-scoped scratch knowledge_item + chunk, or return existing
    /// row when `(project_id, content_hash)` already has active scratch (IMP-13/22).
    ///
    /// Does **not** mutate approved/trusted items. Never creates global scope rows.
    /// Caller must run sync Voyage embed + Qdrant upsert, then
    /// [`Self::mark_scratch_chunk_ready`] or [`Self::delete_scratch_knowledge_item`]
    /// on failure so no searchable orphan remains.
    pub async fn index_memory_for_agent(
        &self,
        agent: &AuthenticatedAgentToken,
        params: IndexMemoryParams,
    ) -> QueriaResult<IndexedMemoryRecord> {
        if params.content_hash.trim().is_empty() {
            return Err(QueriaError::Validation("invalid_content_hash".to_owned()));
        }

        let project = self
            .resolve_project_for_agent(agent, params.project_id, params.project_slug.as_deref())
            .await?;

        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;

        // Idempotent lookup: same project + content_hash → existing scratch (IMP-22).
        if let Some(existing) = sqlx::query(
            "select ki.id as knowledge_item_id, c.id as chunk_id, ki.project_id,
                    ki.organization_id, ki.status::text as status, ki.scope::text as scope,
                    ki.title, ki.body, ki.content_hash
             from knowledge_item ki
             join chunk c on c.knowledge_item_id = ki.id and c.chunk_index = 0
             where ki.project_id = $1
               and ki.status = 'scratch'
               and ki.content_hash = $2
             limit 1",
        )
        .bind(project.id)
        .bind(&params.content_hash)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?
        {
            transaction
                .commit()
                .await
                .map_err(to_infrastructure_error)?;
            return Ok(IndexedMemoryRecord {
                knowledge_item_id: existing
                    .try_get("knowledge_item_id")
                    .map_err(to_infrastructure_error)?,
                chunk_id: existing
                    .try_get("chunk_id")
                    .map_err(to_infrastructure_error)?,
                project_id: existing
                    .try_get("project_id")
                    .map_err(to_infrastructure_error)?,
                organization_id: existing
                    .try_get("organization_id")
                    .map_err(to_infrastructure_error)?,
                status: existing
                    .try_get("status")
                    .map_err(to_infrastructure_error)?,
                scope: existing.try_get("scope").map_err(to_infrastructure_error)?,
                title: existing.try_get("title").map_err(to_infrastructure_error)?,
                body: existing.try_get("body").map_err(to_infrastructure_error)?,
                content_hash: existing
                    .try_get("content_hash")
                    .map_err(to_infrastructure_error)?,
                created: false,
            });
        }

        // Guard: never overwrite approved by matching hash under scratch uniqueness
        // (partial unique index only covers status=scratch). Explicit no-op on trusted.
        let approved_collision: Option<Uuid> = sqlx::query_scalar(
            "select id from knowledge_item
             where project_id = $1
               and status = 'approved'
               and content_hash = $2
             limit 1",
        )
        .bind(project.id)
        .bind(&params.content_hash)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;
        // approved_collision is informational: we still create a separate scratch row
        // (VAL-DL-054). Do not mutate the approved id.
        let _ = approved_collision;

        let knowledge_item_id = sqlx::query(
            "insert into knowledge_item(
               organization_id, project_id, scope, status, title, body,
               category, tags, content_hash, generated_by
             )
             values ($1, $2, 'project', 'scratch', $3, $4, $5, $6, $7, $8)
             returning id",
        )
        .bind(agent.organization_id)
        .bind(project.id)
        .bind(&params.title)
        .bind(&params.body)
        .bind(&params.category)
        .bind(&params.tags)
        .bind(&params.content_hash)
        .bind(format!("agent:{}:{}", agent.token_prefix, agent.name))
        .fetch_one(&mut *transaction)
        .await
        .and_then(|row| row.try_get::<Uuid, _>("id"))
        .map_err(to_infrastructure_error)?;

        let source_uri = format!("queria://scratch/{knowledge_item_id}");
        let source_document_id = sqlx::query(
            "insert into source_document(
               organization_id, project_id, kind, uri, title, source_path,
               content_hash, metadata, is_active
             )
             values ($1, $2, 'manual_note', $3, $4, $5, $6, $7, true)
             returning id",
        )
        .bind(agent.organization_id)
        .bind(project.id)
        .bind(&source_uri)
        .bind(&params.title)
        .bind(format!("scratch/{knowledge_item_id}"))
        .bind(format!("scratch:{}", params.content_hash))
        .bind(json!({
            "generated_from": "index_memory",
            "knowledge_item_id": knowledge_item_id,
            "agent_token_id": agent.id,
            "agent_token_prefix": agent.token_prefix
        }))
        .fetch_one(&mut *transaction)
        .await
        .and_then(|row| row.try_get::<Uuid, _>("id"))
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "update knowledge_item
             set source_document_id = $2, updated_at = now()
             where id = $1",
        )
        .bind(knowledge_item_id)
        .bind(source_document_id)
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let chunk_id = sqlx::query(
            "insert into chunk(
               knowledge_item_id, source_document_id, chunk_index, body,
               token_count, content_hash, search_title, metadata,
               embedding_status
             )
             values ($1, $2, 0, $3, 0, $4, $5, $6, 'pending')
             returning id",
        )
        .bind(knowledge_item_id)
        .bind(source_document_id)
        .bind(&params.body)
        .bind(&params.content_hash)
        .bind(&params.title)
        .bind(json!({
            "line_start": 1,
            "line_end": params.body.lines().count().max(1),
            "lane": "scratch"
        }))
        .fetch_one(&mut *transaction)
        .await
        .and_then(|row| row.try_get::<Uuid, _>("id"))
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "insert into audit_log(
               organization_id, actor_type, actor_id, action,
               resource_type, resource_id, metadata
             )
             values ($1, 'agent', $2, 'index_memory', 'knowledge_item', $3, $4)",
        )
        .bind(agent.organization_id)
        .bind(format!("agent:{}:{}", agent.token_prefix, agent.name))
        .bind(knowledge_item_id.to_string())
        .bind(json!({
            "project_id": project.id,
            "chunk_id": chunk_id,
            "content_hash": params.content_hash,
            "status": "scratch",
            "scope": "project",
            "agent_token_id": agent.id,
            "agent_token_prefix": agent.token_prefix
        }))
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        Ok(IndexedMemoryRecord {
            knowledge_item_id,
            chunk_id,
            project_id: project.id,
            organization_id: agent.organization_id,
            status: "scratch".to_owned(),
            scope: "project".to_owned(),
            title: params.title,
            body: params.body,
            content_hash: params.content_hash,
            created: true,
        })
    }

    /// Find project in agent home org by slug (any status; no token allowlist check).
    pub async fn find_project_by_slug_in_org(
        &self,
        organization_id: Uuid,
        slug: &str,
    ) -> QueriaResult<Option<ProjectRecord>> {
        sqlx::query(
            "select p.id, p.slug, p.name, p.description, p.default_embedding_model,
                    p.include_global_default, p.created_at, p.updated_at
             from project p
             where p.organization_id = $1
               and p.slug = $2",
        )
        .bind(organization_id)
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(project_from_row)
        .transpose()
    }

    /// Find project by origin_url stored in source_document.metadata (local_git_index).
    pub async fn find_project_by_origin_in_org(
        &self,
        organization_id: Uuid,
        origin_url: &str,
    ) -> QueriaResult<Option<ProjectRecord>> {
        if origin_url.trim().is_empty() {
            return Ok(None);
        }
        sqlx::query(
            "select p.id, p.slug, p.name, p.description, p.default_embedding_model,
                    p.include_global_default, p.created_at, p.updated_at
             from project p
             join source_document sd on sd.project_id = p.id
             where p.organization_id = $1
               and sd.metadata->>'origin_url' = $2
               and (sd.metadata->>'local_git_index')::boolean is true
             order by p.created_at asc
             limit 1",
        )
        .bind(organization_id)
        .bind(origin_url)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(project_from_row)
        .transpose()
    }

    /// Earliest local-git origin_url recorded for a project, if any.
    pub async fn find_origin_for_project(
        &self,
        project_id: Uuid,
    ) -> QueriaResult<Option<String>> {
        let row = sqlx::query(
            "select sd.metadata->>'origin_url' as origin_url
             from source_document sd
             where sd.project_id = $1
               and (sd.metadata->>'local_git_index')::boolean is true
               and nullif(sd.metadata->>'origin_url', '') is not null
             order by sd.created_at asc
             limit 1",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        Ok(row
            .and_then(|r| r.try_get::<Option<String>, _>("origin_url").ok())
            .flatten()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()))
    }

    /// Auto-create a project for agent IndexLocal (home org only).
    pub async fn create_project_for_agent(
        &self,
        agent: &AuthenticatedAgentToken,
        slug: &str,
        name: &str,
    ) -> QueriaResult<ProjectRecord> {
        let row = sqlx::query(
            "insert into project(
               organization_id, slug, name, description,
               default_embedding_model, include_global_default
             )
             values ($1, $2, $3, $4, 'voyage-4', true)
             on conflict (organization_id, slug) do nothing
             returning id, slug, name, description, default_embedding_model,
                       include_global_default, created_at, updated_at",
        )
        .bind(agent.organization_id)
        .bind(slug)
        .bind(name)
        .bind(Some("Auto-created by index-local"))
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;

        if let Some(row) = row {
            return project_from_row(row);
        }

        // Race or pre-existing: re-read.
        self.find_project_by_slug_in_org(agent.organization_id, slug)
            .await?
            .ok_or_else(|| QueriaError::Infrastructure("project create race unresolved".to_owned()))
    }

    /// Append slug to agent_token.permissions.project_slugs JSONB when missing.
    pub async fn attach_project_slug_to_agent_token(
        &self,
        agent_token_id: Uuid,
        slug: &str,
    ) -> QueriaResult<()> {
        sqlx::query(
            "update agent_token
             set permissions = jsonb_set(
               permissions,
               '{project_slugs}',
               coalesce(permissions->'project_slugs', '[]'::jsonb) || jsonb_build_array($2::text)
             )
             where id = $1
               and not (coalesce(permissions->'project_slugs', '[]'::jsonb) ? $2)",
        )
        .bind(agent_token_id)
        .bind(slug)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(to_infrastructure_error)
    }

    /// Insert needs_review knowledge_item + chunk for one local file (idempotent on project+path+hash).
    pub async fn index_local_file_for_agent(
        &self,
        agent: &AuthenticatedAgentToken,
        params: IndexLocalFileParams,
    ) -> QueriaResult<IndexedLocalFileRecord> {
        if params.content_hash.trim().is_empty() {
            return Err(QueriaError::Validation("invalid_content_hash".to_owned()));
        }
        if params.path.trim().is_empty() {
            return Err(QueriaError::Validation("invalid_path".to_owned()));
        }

        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;

        // Ensure project belongs to agent home org.
        let project_ok: Option<Uuid> =
            sqlx::query_scalar("select id from project where id = $1 and organization_id = $2")
                .bind(params.project_id)
                .bind(agent.organization_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(to_infrastructure_error)?;
        if project_ok.is_none() {
            return Err(QueriaError::PermissionDenied);
        }

        // Idempotent via chunk metadata logical_path + content_hash.
        if let Some(existing) = sqlx::query(
            "select ki.id as knowledge_item_id, c.id as chunk_id,
                    coalesce(c.source_document_id, ki.source_document_id) as source_document_id,
                    ki.project_id, c.content_hash
             from knowledge_item ki
             join chunk c on c.knowledge_item_id = ki.id and c.chunk_index = 0
             where ki.project_id = $1
               and ki.status = 'needs_review'
               and c.content_hash = $2
               and c.metadata->>'logical_path' = $3
             limit 1",
        )
        .bind(params.project_id)
        .bind(&params.content_hash)
        .bind(&params.path)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?
        {
            transaction
                .commit()
                .await
                .map_err(to_infrastructure_error)?;
            return Ok(IndexedLocalFileRecord {
                knowledge_item_id: existing
                    .try_get("knowledge_item_id")
                    .map_err(to_infrastructure_error)?,
                chunk_id: existing
                    .try_get("chunk_id")
                    .map_err(to_infrastructure_error)?,
                source_document_id: existing
                    .try_get("source_document_id")
                    .map_err(to_infrastructure_error)?,
                project_id: existing
                    .try_get("project_id")
                    .map_err(to_infrastructure_error)?,
                content_hash: existing
                    .try_get("content_hash")
                    .map_err(to_infrastructure_error)?,
                created: false,
            });
        }

        // New content_hash for same logical_path: supersede prior needs_review so
        // stale NR rows do not accumulate. Approved (promoted) items are left alone.
        sqlx::query(Self::supersede_prior_needs_review_sql())
            .bind(params.project_id)
            .bind(&params.path)
            .bind(&params.content_hash)
            .execute(&mut *transaction)
            .await
            .map_err(to_infrastructure_error)?;

        let title = params.path.clone();
        let source_uri = format!(
            "queria://local-git/{}?path={}",
            params.project_id, params.path
        );
        let source_content_hash = format!("local-git:{}:{}", params.path, params.content_hash);
        let source_document_id = sqlx::query(
            "insert into source_document(
               organization_id, project_id, kind, uri, title, source_path,
               branch, commit_sha, content_hash, metadata, is_active
             )
             values ($1, $2, 'manual_note', $3, $4, $5, $6, $7, $8, $9, true)
             on conflict (organization_id, project_id, uri, content_hash) do update
               set branch = excluded.branch,
                   commit_sha = excluded.commit_sha,
                   metadata = excluded.metadata,
                   updated_at = now()
             returning id",
        )
        .bind(agent.organization_id)
        .bind(params.project_id)
        .bind(&source_uri)
        .bind(&title)
        .bind(&params.path)
        .bind(&params.branch)
        .bind(&params.commit_sha)
        .bind(&source_content_hash)
        .bind(json!({
            "local_git_index": true,
            "origin_url": params.origin_url,
            "commit_sha": params.commit_sha,
            "branch": params.branch,
            "logical_path": params.path,
            "local_path_hint": params.local_path_hint,
            "agent_token_id": agent.id,
            "agent_token_prefix": agent.token_prefix
        }))
        .fetch_one(&mut *transaction)
        .await
        .and_then(|row| row.try_get::<Uuid, _>("id"))
        .map_err(to_infrastructure_error)?;

        let knowledge_item_id = sqlx::query(
            "insert into knowledge_item(
               organization_id, project_id, source_document_id, scope, status,
               title, body, category, tags, content_hash, generated_by
             )
             values ($1, $2, $3, 'project', 'needs_review', $4, $5, $6, $7, $8, $9)
             returning id",
        )
        .bind(agent.organization_id)
        .bind(params.project_id)
        .bind(source_document_id)
        .bind(&title)
        .bind(&params.body)
        .bind("local_git")
        .bind(vec!["local-git".to_owned(), "needs-review".to_owned()])
        .bind(&params.content_hash)
        .bind(format!("agent:{}:{}", agent.token_prefix, agent.name))
        .fetch_one(&mut *transaction)
        .await
        .and_then(|row| row.try_get::<Uuid, _>("id"))
        .map_err(to_infrastructure_error)?;

        let chunk_id = sqlx::query(
            "insert into chunk(
               knowledge_item_id, source_document_id, chunk_index, body,
               token_count, content_hash, search_title, metadata,
               embedding_status
             )
             values ($1, $2, 0, $3, 0, $4, $5, $6, 'pending')
             returning id",
        )
        .bind(knowledge_item_id)
        .bind(source_document_id)
        .bind(&params.body)
        .bind(&params.content_hash)
        .bind(&title)
        .bind(json!({
            "line_start": 1,
            "line_end": params.body.lines().count().max(1),
            "lane": "needs_review",
            "logical_path": params.path,
            "origin_url": params.origin_url,
            "commit_sha": params.commit_sha,
            "branch": params.branch,
            "local_git_index": true
        }))
        .fetch_one(&mut *transaction)
        .await
        .and_then(|row| row.try_get::<Uuid, _>("id"))
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "insert into audit_log(
               organization_id, actor_type, actor_id, action,
               resource_type, resource_id, metadata
             )
             values ($1, 'agent', $2, 'index_local', 'knowledge_item', $3, $4)",
        )
        .bind(agent.organization_id)
        .bind(format!("agent:{}:{}", agent.token_prefix, agent.name))
        .bind(knowledge_item_id.to_string())
        .bind(json!({
            "project_id": params.project_id,
            "chunk_id": chunk_id,
            "content_hash": params.content_hash,
            "status": "needs_review",
            "scope": "project",
            "logical_path": params.path,
            "origin_url": params.origin_url,
            "agent_token_id": agent.id
        }))
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        Ok(IndexedLocalFileRecord {
            knowledge_item_id,
            chunk_id,
            source_document_id,
            project_id: params.project_id,
            content_hash: params.content_hash,
            created: true,
        })
    }

    /// Enqueue embedding_backfill for agent-accessible project (IndexLocal path).
    pub async fn enqueue_embedding_backfill_for_agent(
        &self,
        agent: &AuthenticatedAgentToken,
        project_id: Uuid,
        embedding_profile_version: &str,
    ) -> QueriaResult<Option<Uuid>> {
        sqlx::query_scalar(
            "with accessible_project as (
               select p.organization_id, p.id
               from project p
               where p.organization_id = $1
                 and p.id = $2
             )
             insert into ingestion_job(
               organization_id, project_id, job_type, payload
             )
             select organization_id, id, 'embedding_backfill',
                    jsonb_build_object(
                      'triggered_by_agent_token_id', $3::text,
                      'embedding_profile_version', $4::text,
                      'source', 'index_local'
                    )
             from accessible_project
             on conflict (project_id, job_type)
               where project_id is not null
                 and source_document_id is null
                 and job_type = 'embedding_backfill'
                 and status in ('queued', 'running')
             do update set updated_at = ingestion_job.updated_at
             returning id",
        )
        .bind(agent.organization_id)
        .bind(project_id)
        .bind(agent.id.to_string())
        .bind(embedding_profile_version)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)
    }

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
             join org_membership m on m.organization_id = ki.organization_id
             where m.user_id = $1
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
             join org_membership m on m.organization_id = ki.organization_id
             where m.user_id = $1
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
             join org_membership m on m.organization_id = ki.organization_id
             where m.user_id = $1
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

    /// Count knowledge items with status `needs_review` for one project.
    ///
    /// Project-scoped only (no org filter beyond the project row); intended for
    /// agent status surfaces after the project has already been resolved in scope.
    pub async fn count_needs_review_items(&self, project_id: ProjectId) -> QueriaResult<i64> {
        sqlx::query_scalar(
            "select count(*)::bigint
             from knowledge_item
             where project_id = $1
               and status = 'needs_review'",
        )
        .bind(project_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(to_infrastructure_error)
    }

    /// List knowledge items in `needs_review` for the admin session org (IMP-L4).
    pub async fn list_needs_review(
        &self,
        user_id: Uuid,
        project_slug: Option<&str>,
        limit: u32,
    ) -> QueriaResult<Vec<NeedsReviewItemRecord>> {
        let page_limit = limit.min(200) as i64;
        let slug = project_slug.map(|s| s.trim()).filter(|s| !s.is_empty());

        sqlx::query(
            "select ki.id as knowledge_item_id,
                    ki.project_id,
                    p.slug as project_slug,
                    ki.source_document_id,
                    ki.title,
                    coalesce(sd.source_path, c.metadata->>'logical_path') as path,
                    coalesce(
                      nullif(sd.metadata->>'origin_url', ''),
                      nullif(c.metadata->>'origin_url', '')
                    ) as origin_url,
                    coalesce(sd.commit_sha, nullif(c.metadata->>'commit_sha', '')) as commit_sha,
                    coalesce(sd.branch, nullif(c.metadata->>'branch', '')) as branch,
                    ki.category,
                    ki.created_at,
                    ki.updated_at
             from knowledge_item ki
             join org_membership m on m.organization_id = ki.organization_id
             left join project p on p.id = ki.project_id
             left join source_document sd on sd.id = ki.source_document_id
             left join lateral (
               select metadata
               from chunk
               where knowledge_item_id = ki.id
               order by chunk_index asc
               limit 1
             ) c on true
             where m.user_id = $1
               and ki.status = 'needs_review'
               and ($2::text is null or p.slug = $2)
             order by p.slug nulls last,
                      coalesce(
                        nullif(sd.metadata->>'origin_url', ''),
                        nullif(c.metadata->>'origin_url', ''),
                        ''
                      ),
                      coalesce(sd.commit_sha, nullif(c.metadata->>'commit_sha', ''), ''),
                      ki.created_at desc
             limit $3",
        )
        .bind(user_id)
        .bind(slug)
        .bind(page_limit)
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .into_iter()
        .map(needs_review_item_from_row)
        .collect()
    }

    /// Promote one needs_review item to approved (trusted). Updates chunk metadata lane.
    pub async fn promote_needs_review(
        &self,
        user_id: Uuid,
        knowledge_item_id: KnowledgeItemId,
    ) -> QueriaResult<Option<NeedsReviewActionRecord>> {
        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;

        let locked = sqlx::query(
            "select ki.id, ki.project_id, ki.source_document_id, ki.scope::text as scope,
                    ki.status::text as status, ki.title, ki.body, ki.category,
                    ki.tags, ki.approved_at, ki.created_at, ki.updated_at,
                    ki.organization_id
             from knowledge_item ki
             join org_membership m on m.organization_id = ki.organization_id
             where m.user_id = $1
               and ki.id = $2
             for update of ki",
        )
        .bind(user_id)
        .bind(knowledge_item_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let Some(row) = locked else {
            return Ok(None);
        };

        let status: String = row.try_get("status").map_err(to_infrastructure_error)?;
        if status != "needs_review" {
            return Err(QueriaError::Validation(
                "knowledge item is not in needs_review".to_owned(),
            ));
        }

        let organization_id: Uuid = row
            .try_get("organization_id")
            .map_err(to_infrastructure_error)?;

        sqlx::query(
            "update knowledge_item
             set status = 'approved',
                 approved_at = now(),
                 updated_at = now()
             where id = $1
               and status = 'needs_review'",
        )
        .bind(knowledge_item_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        // Align chunk lane metadata with trusted/approved (retrieve derives lane from status).
        sqlx::query(
            "update chunk
             set metadata = jsonb_set(
               coalesce(metadata, '{}'::jsonb),
               '{lane}',
               '\"trusted\"'::jsonb,
               true
             )
             where knowledge_item_id = $1",
        )
        .bind(knowledge_item_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let chunk_ids: Vec<Uuid> = sqlx::query_scalar(
            "select id from chunk where knowledge_item_id = $1 order by chunk_index",
        )
        .bind(knowledge_item_id.as_uuid())
        .fetch_all(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "insert into audit_log(
               organization_id, actor_type, actor_id, action,
               resource_type, resource_id, metadata
             )
             values ($1, 'user', $2, 'needs_review.promoted', 'knowledge_item', $3, $4)",
        )
        .bind(organization_id)
        .bind(user_id.to_string())
        .bind(knowledge_item_id.as_uuid().to_string())
        .bind(json!({
            "from_status": "needs_review",
            "to_status": "approved",
            "chunk_ids": chunk_ids,
        }))
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let knowledge_item = knowledge_item_from_row(
            sqlx::query(
                "select ki.id, ki.project_id, ki.source_document_id, ki.scope::text as scope,
                        ki.status::text as status, ki.title, ki.body, ki.category,
                        ki.tags, ki.approved_at, ki.created_at, ki.updated_at
                 from knowledge_item ki
                 where ki.id = $1",
            )
            .bind(knowledge_item_id.as_uuid())
            .fetch_one(&mut *transaction)
            .await
            .map_err(to_infrastructure_error)?,
        )?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        Ok(Some(NeedsReviewActionRecord {
            knowledge_item,
            chunk_ids,
        }))
    }

    /// Reject one needs_review item (status → rejected). Updates chunk metadata lane.
    pub async fn reject_needs_review(
        &self,
        user_id: Uuid,
        knowledge_item_id: KnowledgeItemId,
        reason: Option<String>,
    ) -> QueriaResult<Option<NeedsReviewActionRecord>> {
        let reason = reason
            .map(|r| r.trim().to_owned())
            .filter(|r| !r.is_empty());
        if let Some(ref r) = reason
            && r.len() > 2_000
        {
            return Err(QueriaError::Validation(
                "rejection reason must be at most 2000 bytes".to_owned(),
            ));
        }

        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;

        let locked = sqlx::query(
            "select ki.id, ki.project_id, ki.source_document_id, ki.scope::text as scope,
                    ki.status::text as status, ki.title, ki.body, ki.category,
                    ki.tags, ki.approved_at, ki.created_at, ki.updated_at,
                    ki.organization_id
             from knowledge_item ki
             join org_membership m on m.organization_id = ki.organization_id
             where m.user_id = $1
               and ki.id = $2
             for update of ki",
        )
        .bind(user_id)
        .bind(knowledge_item_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let Some(row) = locked else {
            return Ok(None);
        };

        let status: String = row.try_get("status").map_err(to_infrastructure_error)?;
        if status != "needs_review" {
            return Err(QueriaError::Validation(
                "knowledge item is not in needs_review".to_owned(),
            ));
        }

        let organization_id: Uuid = row
            .try_get("organization_id")
            .map_err(to_infrastructure_error)?;

        sqlx::query(
            "update knowledge_item
             set status = 'rejected',
                 updated_at = now()
             where id = $1
               and status = 'needs_review'",
        )
        .bind(knowledge_item_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "update chunk
             set metadata = jsonb_set(
               coalesce(metadata, '{}'::jsonb),
               '{lane}',
               '\"rejected\"'::jsonb,
               true
             )
             where knowledge_item_id = $1",
        )
        .bind(knowledge_item_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let chunk_ids: Vec<Uuid> = sqlx::query_scalar(
            "select id from chunk where knowledge_item_id = $1 order by chunk_index",
        )
        .bind(knowledge_item_id.as_uuid())
        .fetch_all(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "insert into audit_log(
               organization_id, actor_type, actor_id, action,
               resource_type, resource_id, metadata
             )
             values ($1, 'user', $2, 'needs_review.rejected', 'knowledge_item', $3, $4)",
        )
        .bind(organization_id)
        .bind(user_id.to_string())
        .bind(knowledge_item_id.as_uuid().to_string())
        .bind(json!({
            "from_status": "needs_review",
            "to_status": "rejected",
            "reason": reason,
            "chunk_ids": chunk_ids,
        }))
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let knowledge_item = knowledge_item_from_row(
            sqlx::query(
                "select ki.id, ki.project_id, ki.source_document_id, ki.scope::text as scope,
                        ki.status::text as status, ki.title, ki.body, ki.category,
                        ki.tags, ki.approved_at, ki.created_at, ki.updated_at
                 from knowledge_item ki
                 where ki.id = $1",
            )
            .bind(knowledge_item_id.as_uuid())
            .fetch_one(&mut *transaction)
            .await
            .map_err(to_infrastructure_error)?,
        )?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        Ok(Some(NeedsReviewActionRecord {
            knowledge_item,
            chunk_ids,
        }))
    }

    /// List needs_review items for an agent token (home org + project_slugs scope).
    pub async fn list_needs_review_for_agent(
        &self,
        agent: &AuthenticatedAgentToken,
        project_slug: Option<&str>,
        limit: u32,
    ) -> QueriaResult<Vec<NeedsReviewItemRecord>> {
        let page_limit = limit.min(200) as i64;
        let slug = project_slug.map(|s| s.trim()).filter(|s| !s.is_empty());
        if let Some(s) = slug
            && !agent.permissions.project_slugs.iter().any(|p| p == s)
        {
            return Err(QueriaError::PermissionDenied);
        }

        sqlx::query(
            "select ki.id as knowledge_item_id,
                    ki.project_id,
                    p.slug as project_slug,
                    ki.source_document_id,
                    ki.title,
                    coalesce(sd.source_path, c.metadata->>'logical_path') as path,
                    coalesce(
                      nullif(sd.metadata->>'origin_url', ''),
                      nullif(c.metadata->>'origin_url', '')
                    ) as origin_url,
                    coalesce(sd.commit_sha, nullif(c.metadata->>'commit_sha', '')) as commit_sha,
                    coalesce(sd.branch, nullif(c.metadata->>'branch', '')) as branch,
                    ki.category,
                    ki.created_at,
                    ki.updated_at
             from knowledge_item ki
             join project p on p.id = ki.project_id
             left join source_document sd on sd.id = ki.source_document_id
             left join lateral (
               select metadata
               from chunk
               where knowledge_item_id = ki.id
               order by chunk_index asc
               limit 1
             ) c on true
             where ki.organization_id = $1
               and ki.status = 'needs_review'
               and p.slug = any($2)
               and ($3::text is null or p.slug = $3)
             order by p.slug nulls last,
                      coalesce(
                        nullif(sd.metadata->>'origin_url', ''),
                        nullif(c.metadata->>'origin_url', ''),
                        ''
                      ),
                      coalesce(sd.commit_sha, nullif(c.metadata->>'commit_sha', ''), ''),
                      ki.created_at desc
             limit $4",
        )
        .bind(agent.organization_id)
        .bind(&agent.permissions.project_slugs)
        .bind(slug)
        .bind(page_limit)
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .into_iter()
        .map(needs_review_item_from_row)
        .collect()
    }

    /// Promote one needs_review item for an agent (org + project_slugs scope).
    pub async fn promote_needs_review_for_agent(
        &self,
        agent: &AuthenticatedAgentToken,
        knowledge_item_id: KnowledgeItemId,
    ) -> QueriaResult<Option<NeedsReviewActionRecord>> {
        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;

        let locked = sqlx::query(
            "select ki.id, ki.project_id, ki.source_document_id, ki.scope::text as scope,
                    ki.status::text as status, ki.title, ki.body, ki.category,
                    ki.tags, ki.approved_at, ki.created_at, ki.updated_at,
                    ki.organization_id
             from knowledge_item ki
             join project p on p.id = ki.project_id
             where ki.organization_id = $1
               and ki.id = $2
               and p.slug = any($3)
             for update of ki",
        )
        .bind(agent.organization_id)
        .bind(knowledge_item_id.as_uuid())
        .bind(&agent.permissions.project_slugs)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let Some(row) = locked else {
            return Ok(None);
        };

        let status: String = row.try_get("status").map_err(to_infrastructure_error)?;
        if status != "needs_review" {
            return Err(QueriaError::Validation(
                "knowledge item is not in needs_review".to_owned(),
            ));
        }

        sqlx::query(
            "update knowledge_item
             set status = 'approved',
                 approved_at = now(),
                 updated_at = now()
             where id = $1
               and status = 'needs_review'",
        )
        .bind(knowledge_item_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "update chunk
             set metadata = jsonb_set(
               coalesce(metadata, '{}'::jsonb),
               '{lane}',
               '\"trusted\"'::jsonb,
               true
             )
             where knowledge_item_id = $1",
        )
        .bind(knowledge_item_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let chunk_ids: Vec<Uuid> = sqlx::query_scalar(
            "select id from chunk where knowledge_item_id = $1 order by chunk_index",
        )
        .bind(knowledge_item_id.as_uuid())
        .fetch_all(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "insert into audit_log(
               organization_id, actor_type, actor_id, action,
               resource_type, resource_id, metadata
             )
             values ($1, 'agent', $2, 'needs_review.promoted', 'knowledge_item', $3, $4)",
        )
        .bind(agent.organization_id)
        .bind(format!("agent:{}:{}", agent.token_prefix, agent.name))
        .bind(knowledge_item_id.as_uuid().to_string())
        .bind(json!({
            "from_status": "needs_review",
            "to_status": "approved",
            "chunk_ids": chunk_ids,
            "agent_token_id": agent.id,
            "agent_token_prefix": agent.token_prefix
        }))
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let knowledge_item = knowledge_item_from_row(
            sqlx::query(
                "select ki.id, ki.project_id, ki.source_document_id, ki.scope::text as scope,
                        ki.status::text as status, ki.title, ki.body, ki.category,
                        ki.tags, ki.approved_at, ki.created_at, ki.updated_at
                 from knowledge_item ki
                 where ki.id = $1",
            )
            .bind(knowledge_item_id.as_uuid())
            .fetch_one(&mut *transaction)
            .await
            .map_err(to_infrastructure_error)?,
        )?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        Ok(Some(NeedsReviewActionRecord {
            knowledge_item,
            chunk_ids,
        }))
    }

    /// Reject one needs_review item for an agent (org + project_slugs scope).
    pub async fn reject_needs_review_for_agent(
        &self,
        agent: &AuthenticatedAgentToken,
        knowledge_item_id: KnowledgeItemId,
        reason: Option<String>,
    ) -> QueriaResult<Option<NeedsReviewActionRecord>> {
        let reason = reason
            .map(|r| r.trim().to_owned())
            .filter(|r| !r.is_empty());
        if let Some(ref r) = reason
            && r.len() > 2_000
        {
            return Err(QueriaError::Validation(
                "rejection reason must be at most 2000 bytes".to_owned(),
            ));
        }

        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;

        let locked = sqlx::query(
            "select ki.id, ki.project_id, ki.source_document_id, ki.scope::text as scope,
                    ki.status::text as status, ki.title, ki.body, ki.category,
                    ki.tags, ki.approved_at, ki.created_at, ki.updated_at,
                    ki.organization_id
             from knowledge_item ki
             join project p on p.id = ki.project_id
             where ki.organization_id = $1
               and ki.id = $2
               and p.slug = any($3)
             for update of ki",
        )
        .bind(agent.organization_id)
        .bind(knowledge_item_id.as_uuid())
        .bind(&agent.permissions.project_slugs)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let Some(row) = locked else {
            return Ok(None);
        };

        let status: String = row.try_get("status").map_err(to_infrastructure_error)?;
        if status != "needs_review" {
            return Err(QueriaError::Validation(
                "knowledge item is not in needs_review".to_owned(),
            ));
        }

        sqlx::query(
            "update knowledge_item
             set status = 'rejected',
                 updated_at = now()
             where id = $1
               and status = 'needs_review'",
        )
        .bind(knowledge_item_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "update chunk
             set metadata = jsonb_set(
               coalesce(metadata, '{}'::jsonb),
               '{lane}',
               '\"rejected\"'::jsonb,
               true
             )
             where knowledge_item_id = $1",
        )
        .bind(knowledge_item_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let chunk_ids: Vec<Uuid> = sqlx::query_scalar(
            "select id from chunk where knowledge_item_id = $1 order by chunk_index",
        )
        .bind(knowledge_item_id.as_uuid())
        .fetch_all(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "insert into audit_log(
               organization_id, actor_type, actor_id, action,
               resource_type, resource_id, metadata
             )
             values ($1, 'agent', $2, 'needs_review.rejected', 'knowledge_item', $3, $4)",
        )
        .bind(agent.organization_id)
        .bind(format!("agent:{}:{}", agent.token_prefix, agent.name))
        .bind(knowledge_item_id.as_uuid().to_string())
        .bind(json!({
            "from_status": "needs_review",
            "to_status": "rejected",
            "reason": reason,
            "chunk_ids": chunk_ids,
            "agent_token_id": agent.id,
            "agent_token_prefix": agent.token_prefix
        }))
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let knowledge_item = knowledge_item_from_row(
            sqlx::query(
                "select ki.id, ki.project_id, ki.source_document_id, ki.scope::text as scope,
                        ki.status::text as status, ki.title, ki.body, ki.category,
                        ki.tags, ki.approved_at, ki.created_at, ki.updated_at
                 from knowledge_item ki
                 where ki.id = $1",
            )
            .bind(knowledge_item_id.as_uuid())
            .fetch_one(&mut *transaction)
            .await
            .map_err(to_infrastructure_error)?,
        )?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        Ok(Some(NeedsReviewActionRecord {
            knowledge_item,
            chunk_ids,
        }))
    }

    /// Bulk promote needs_review items matching project + origin + commit (IMP-L4).
    ///
    /// When both `origin_url` and `commit_sha` are empty/None, requires
    /// `force_project_all = true` (default false) to avoid project-wide wildmatch.
    pub async fn promote_needs_review_by_origin_commit(
        &self,
        user_id: Uuid,
        project_slug: &str,
        origin_url: Option<&str>,
        commit_sha: Option<&str>,
        force_project_all: bool,
    ) -> QueriaResult<Vec<NeedsReviewActionRecord>> {
        let ids = self
            .list_needs_review_ids_for_origin_commit(
                user_id,
                project_slug,
                origin_url,
                commit_sha,
                force_project_all,
            )
            .await?;
        let mut results = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(record) = self
                .promote_needs_review(user_id, KnowledgeItemId::from_uuid(id))
                .await?
            {
                results.push(record);
            }
        }
        Ok(results)
    }

    /// Bulk reject needs_review items matching project + origin + commit.
    ///
    /// When both `origin_url` and `commit_sha` are empty/None, requires
    /// `force_project_all = true` (default false) to avoid project-wide wildmatch.
    pub async fn reject_needs_review_by_origin_commit(
        &self,
        user_id: Uuid,
        project_slug: &str,
        origin_url: Option<&str>,
        commit_sha: Option<&str>,
        reason: Option<String>,
        force_project_all: bool,
    ) -> QueriaResult<Vec<NeedsReviewActionRecord>> {
        let ids = self
            .list_needs_review_ids_for_origin_commit(
                user_id,
                project_slug,
                origin_url,
                commit_sha,
                force_project_all,
            )
            .await?;
        let mut results = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(record) = self
                .reject_needs_review(user_id, KnowledgeItemId::from_uuid(id), reason.clone())
                .await?
            {
                results.push(record);
            }
        }
        Ok(results)
    }

    /// Pure guard: bulk by origin/commit must not wildmatch a whole project unless forced.
    pub fn bulk_origin_commit_allowed(
        origin_url: Option<&str>,
        commit_sha: Option<&str>,
        force_project_all: bool,
    ) -> Result<(), &'static str> {
        let origin = origin_url.map(str::trim).filter(|s| !s.is_empty());
        let commit = commit_sha.map(str::trim).filter(|s| !s.is_empty());
        if origin.is_none() && commit.is_none() && !force_project_all {
            Err("origin_url or commit_sha required for bulk")
        } else {
            Ok(())
        }
    }

    async fn list_needs_review_ids_for_origin_commit(
        &self,
        user_id: Uuid,
        project_slug: &str,
        origin_url: Option<&str>,
        commit_sha: Option<&str>,
        force_project_all: bool,
    ) -> QueriaResult<Vec<Uuid>> {
        Self::bulk_origin_commit_allowed(origin_url, commit_sha, force_project_all)
            .map_err(|m| QueriaError::Validation(m.to_owned()))?;

        let origin = origin_url.map(str::trim).filter(|s| !s.is_empty());
        let commit = commit_sha.map(str::trim).filter(|s| !s.is_empty());

        sqlx::query_scalar(
            "select ki.id
             from knowledge_item ki
             join org_membership m on m.organization_id = ki.organization_id
             join project p on p.id = ki.project_id
             left join source_document sd on sd.id = ki.source_document_id
             left join lateral (
               select metadata
               from chunk
               where knowledge_item_id = ki.id
               order by chunk_index asc
               limit 1
             ) c on true
             where m.user_id = $1
               and ki.status = 'needs_review'
               and p.slug = $2
               and (
                 $3::text is null
                 or coalesce(
                      nullif(sd.metadata->>'origin_url', ''),
                      nullif(c.metadata->>'origin_url', ''),
                      ''
                    ) = $3
               )
               and (
                 $4::text is null
                 or coalesce(sd.commit_sha, nullif(c.metadata->>'commit_sha', ''), '') = $4
               )
             order by ki.created_at asc",
        )
        .bind(user_id)
        .bind(project_slug)
        .bind(origin)
        .bind(commit)
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)
    }

    /// Unit-test SQL contracts for index_memory (no live DB required).
    #[cfg(test)]
    pub(crate) fn index_memory_idempotent_lookup_sql() -> &'static str {
        "select ki.id as knowledge_item_id, c.id as chunk_id, ki.project_id,
                    ki.organization_id, ki.status::text as status, ki.scope::text as scope,
                    ki.title, ki.body, ki.content_hash
             from knowledge_item ki
             join chunk c on c.knowledge_item_id = ki.id and c.chunk_index = 0
             where ki.project_id = $1
               and ki.status = 'scratch'
               and ki.content_hash = $2
             limit 1"
    }

    #[cfg(test)]
    pub(crate) fn index_memory_insert_sql_snippet() -> &'static str {
        "values ($1, $2, 'project', 'scratch', $3, $4, $5, $6, $7, $8)"
    }

    #[cfg(test)]
    pub(crate) fn list_needs_review_sql_contract() -> &'static str {
        "and ki.status = 'needs_review'"
    }

    #[cfg(test)]
    pub(crate) fn promote_needs_review_status_sql() -> &'static str {
        "set status = 'approved'"
    }

    #[cfg(test)]
    pub(crate) fn reject_needs_review_status_sql() -> &'static str {
        "set status = 'rejected'"
    }

    /// SQL: supersede prior needs_review for same project + logical_path when re-indexing a new hash.
    /// Bind order: $1 project_id, $2 logical_path, $3 new content_hash (excluded).
    pub(crate) fn supersede_prior_needs_review_sql() -> &'static str {
        "update knowledge_item ki
         set status = 'superseded',
             updated_at = now()
         from chunk c
         where c.knowledge_item_id = ki.id
           and c.chunk_index = 0
           and ki.project_id = $1
           and ki.status = 'needs_review'
           and c.metadata->>'logical_path' = $2
           and c.content_hash is distinct from $3"
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

#[cfg(test)]
mod index_memory_tests {
    use super::PgProjectRepository;

    /// VAL-DL-018 / IMP-22: lookup is keyed by project + content_hash + scratch only.
    #[test]
    fn idempotent_lookup_filters_scratch_and_hash() {
        let sql = PgProjectRepository::index_memory_idempotent_lookup_sql();
        assert!(sql.contains("ki.status = 'scratch'"));
        assert!(sql.contains("ki.content_hash = $2"));
        assert!(sql.contains("ki.project_id = $1"));
        assert!(!sql.contains("status = 'approved'"));
    }

    /// VAL-DL-008 / VAL-DL-013: insert always project-scoped scratch.
    #[test]
    fn insert_sql_is_project_scoped_scratch() {
        let sql = PgProjectRepository::index_memory_insert_sql_snippet();
        assert!(sql.contains("'project'"));
        assert!(sql.contains("'scratch'"));
        assert!(!sql.contains("'global'"));
        assert!(!sql.contains("'approved'"));
    }
}

#[cfg(test)]
mod needs_review_tests {
    use super::PgProjectRepository;

    /// IMP-L4: list filters needs_review only.
    #[test]
    fn list_needs_review_filters_status() {
        assert_eq!(
            PgProjectRepository::list_needs_review_sql_contract(),
            "and ki.status = 'needs_review'"
        );
    }

    /// IMP-L4: promote sets approved (trusted path).
    #[test]
    fn promote_sets_approved() {
        assert!(
            PgProjectRepository::promote_needs_review_status_sql().contains("status = 'approved'")
        );
    }

    /// IMP-L4: reject sets rejected.
    #[test]
    fn reject_sets_rejected() {
        assert!(
            PgProjectRepository::reject_needs_review_status_sql().contains("status = 'rejected'")
        );
    }

    /// Re-index with new hash supersedes prior needs_review for same logical_path.
    #[test]
    fn supersede_prior_needs_review_sql_contract() {
        let sql = PgProjectRepository::supersede_prior_needs_review_sql();
        assert!(sql.contains("status = 'superseded'"));
        assert!(sql.contains("ki.status = 'needs_review'"));
        assert!(sql.contains("ki.project_id = $1"));
        assert!(sql.contains("c.metadata->>'logical_path' = $2"));
        assert!(sql.contains("c.content_hash is distinct from $3"));
        // Must not touch approved / promoted items.
        assert!(!sql.contains("status = 'approved'"));
        assert!(!sql.contains("status != 'needs_review'"));
    }

    /// Bulk without origin+commit rejects unless force_project_all.
    #[test]
    fn bulk_rejects_without_origin_and_commit() {
        let err = PgProjectRepository::bulk_origin_commit_allowed(None, None, false)
            .expect_err("empty origin+commit without force");
        assert_eq!(err, "origin_url or commit_sha required for bulk");

        assert!(
            PgProjectRepository::bulk_origin_commit_allowed(None, None, true).is_ok(),
            "force_project_all allows empty origin+commit"
        );
        assert!(
            PgProjectRepository::bulk_origin_commit_allowed(Some("git@h:a.git"), None, false)
                .is_ok()
        );
        assert!(
            PgProjectRepository::bulk_origin_commit_allowed(None, Some("abc123"), false).is_ok()
        );
        assert!(
            PgProjectRepository::bulk_origin_commit_allowed(Some("  "), Some(""), false).is_err(),
            "whitespace-only origin/commit still empty"
        );
    }
}

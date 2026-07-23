use queria_core::{QueriaError, QueriaResult};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use super::super::types::{
    AuthenticatedAgentToken, IndexLocalFileParams, IndexMemoryParams, IndexedLocalFileRecord,
    IndexedMemoryRecord, ProjectRecord, project_from_row, to_infrastructure_error,
};
use super::PgProjectRepository;

impl PgProjectRepository {
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
    pub async fn find_origin_for_project(&self, project_id: Uuid) -> QueriaResult<Option<String>> {
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
}

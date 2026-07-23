use queria_core::ids::{KnowledgeItemId, ProjectId};
use queria_core::{QueriaError, QueriaResult};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use super::super::types::{
    AuthenticatedAgentToken, NeedsReviewActionRecord, NeedsReviewItemRecord,
    knowledge_item_from_row, needs_review_item_from_row, to_infrastructure_error,
};
use super::PgProjectRepository;

impl PgProjectRepository {
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
}

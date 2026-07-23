use queria_core::contracts::RetrievedContextItem;
use queria_core::ids::{ProjectId, SourceDocumentId};
use queria_core::{QueriaError, QueriaResult};
use sqlx::Row;
use uuid::Uuid;

use super::super::types::{
    AuthenticatedAgentToken, ProjectRecord, ProposeMemoryParams, ProposedMemoryRecord,
    SourceDocumentRecord, project_from_row, retrieved_item_from_row, source_from_row,
    to_infrastructure_error,
};
use super::PgProjectRepository;

impl PgProjectRepository {
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
}

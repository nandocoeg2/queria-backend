use queria_core::QueriaResult;
use queria_core::contracts::RetrievedContextItem;
use queria_core::ids::ProjectId;
use uuid::Uuid;

use super::super::types::{retrieved_item_from_row, to_infrastructure_error};
use super::PgProjectRepository;

impl PgProjectRepository {
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
}

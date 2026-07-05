use queria_core::contracts::{Citation, RetrievedContextItem};
use queria_core::ids::{ChunkId, ProjectId, SourceDocumentId};
use queria_core::model::KnowledgeScope;
use queria_core::{QueriaError, QueriaResult};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

pub const LEXICAL_SEARCH_SQL: &str = "
with access as (
  select $1::uuid as organization_id, $2::uuid as project_id, $3::boolean as include_global
),
strict_query as (
  select websearch_to_tsquery('simple', $4) as value
),
relaxed_query as (
  select case
    when array_to_string(tsvector_to_array(to_tsvector('simple', $4)), ' | ') = '' then null
    else to_tsquery('simple', array_to_string(tsvector_to_array(to_tsvector('simple', $4)), ' | '))
  end as value
)
select c.id as chunk_id,
       (coalesce(ts_rank_cd(c.search_vector, strict_query.value), 0.0) * 2.0 +
        coalesce(ts_rank_cd(c.search_vector, relaxed_query.value), 0.0))::real as score
from access, strict_query, relaxed_query, chunk c
join knowledge_item k on k.id = c.knowledge_item_id
left join source_document sd on sd.id = c.source_document_id
where k.organization_id = access.organization_id
  and k.status = 'approved'
  and (sd.id is null or sd.is_active)
  and (
    k.project_id = access.project_id
    or (access.include_global and k.scope = 'global')
  )
  and (
    c.search_vector @@ strict_query.value
    or (relaxed_query.value is not null and c.search_vector @@ relaxed_query.value)
  )
order by score desc, c.id
limit $5";

pub const HYDRATE_SQL: &str = "
with access as (
  select $1::uuid as organization_id, $3::uuid as project_id, $4::boolean as include_global
),
requested as (
  select chunk_id, ordinal
  from unnest($2::uuid[]) with ordinality as ranked(chunk_id, ordinal)
)
select c.id as chunk_id, c.source_document_id, k.scope::text as scope,
       k.title, c.body, sd.uri as source_uri, sd.source_path, c.metadata
from access, requested
join chunk c on c.id = requested.chunk_id
join knowledge_item k on k.id = c.knowledge_item_id
join source_document sd on sd.id = c.source_document_id
where k.organization_id = access.organization_id
  and k.status = 'approved'
  and sd.is_active
  and (
    k.project_id = access.project_id
    or (access.include_global and k.scope = 'global')
  )
order by requested.ordinal";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetrievalAccess {
    pub organization_id: Uuid,
    pub project_id: ProjectId,
    pub include_global: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DbRankedChunk {
    pub chunk_id: ChunkId,
    pub score: f32,
}

#[derive(Clone, Debug)]
pub struct PgHybridRetrievalRepository {
    pool: PgPool,
}

impl PgHybridRetrievalRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn authorize_user(
        &self,
        user_id: Uuid,
        project_id: ProjectId,
        include_global: bool,
    ) -> QueriaResult<RetrievalAccess> {
        let organization_id = sqlx::query_scalar::<_, Uuid>(
            "select p.organization_id
             from project p
             join user_account u on u.organization_id = p.organization_id
             where u.id = $1 and p.id = $2",
        )
        .bind(user_id)
        .bind(project_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .ok_or(QueriaError::PermissionDenied)?;
        Ok(RetrievalAccess {
            organization_id,
            project_id,
            include_global,
        })
    }

    pub async fn authorize_agent(
        &self,
        organization_id: Uuid,
        allowed_project_slugs: &[String],
        allow_global: bool,
        project_id: ProjectId,
        include_global: bool,
    ) -> QueriaResult<RetrievalAccess> {
        let allowed = sqlx::query_scalar::<_, bool>(
            "select exists(
               select 1 from project p
               where p.id = $1
                 and p.organization_id = $2
                 and p.slug = any($3)
             )",
        )
        .bind(project_id.as_uuid())
        .bind(organization_id)
        .bind(allowed_project_slugs)
        .fetch_one(&self.pool)
        .await
        .map_err(to_infrastructure_error)?;
        if !allowed {
            return Err(QueriaError::PermissionDenied);
        }
        Ok(RetrievalAccess {
            organization_id,
            project_id,
            include_global: include_global && allow_global,
        })
    }

    pub async fn lexical_search(
        &self,
        access: &RetrievalAccess,
        query: &str,
        limit: i64,
    ) -> QueriaResult<Vec<DbRankedChunk>> {
        sqlx::query(LEXICAL_SEARCH_SQL)
            .bind(access.organization_id)
            .bind(access.project_id.as_uuid())
            .bind(access.include_global)
            .bind(query)
            .bind(limit)
            .fetch_all(&self.pool)
            .await
            .map_err(to_infrastructure_error)?
            .into_iter()
            .map(|row| {
                Ok(DbRankedChunk {
                    chunk_id: ChunkId::from_uuid(
                        row.try_get("chunk_id").map_err(to_infrastructure_error)?,
                    ),
                    score: row.try_get("score").map_err(to_infrastructure_error)?,
                })
            })
            .collect()
    }

    pub async fn hydrate(
        &self,
        access: &RetrievalAccess,
        chunk_ids: &[ChunkId],
    ) -> QueriaResult<Vec<RetrievedContextItem>> {
        if chunk_ids.is_empty() {
            return Ok(Vec::new());
        }
        let raw_ids = chunk_ids
            .iter()
            .map(|chunk_id| chunk_id.as_uuid())
            .collect::<Vec<_>>();
        sqlx::query(HYDRATE_SQL)
            .bind(access.organization_id)
            .bind(&raw_ids)
            .bind(access.project_id.as_uuid())
            .bind(access.include_global)
            .fetch_all(&self.pool)
            .await
            .map_err(to_infrastructure_error)?
            .into_iter()
            .map(retrieved_item_from_row)
            .collect()
    }
}

fn retrieved_item_from_row(row: sqlx::postgres::PgRow) -> QueriaResult<RetrievedContextItem> {
    let scope = parse_scope(
        &row.try_get::<String, _>("scope")
            .map_err(to_infrastructure_error)?,
    )?;
    let metadata: Value = row.try_get("metadata").map_err(to_infrastructure_error)?;
    Ok(RetrievedContextItem {
        chunk_id: ChunkId::from_uuid(row.try_get("chunk_id").map_err(to_infrastructure_error)?),
        source_document_id: SourceDocumentId::from_uuid(
            row.try_get("source_document_id")
                .map_err(to_infrastructure_error)?,
        ),
        scope,
        title: row.try_get("title").map_err(to_infrastructure_error)?,
        body: row.try_get("body").map_err(to_infrastructure_error)?,
        citation: Citation {
            source_uri: row.try_get("source_uri").map_err(to_infrastructure_error)?,
            source_path: row
                .try_get("source_path")
                .map_err(to_infrastructure_error)?,
            line_start: metadata_u32(&metadata, "line_start")?,
            line_end: metadata_u32(&metadata, "line_end")?,
        },
        score: 0.0,
    })
}

fn metadata_u32(metadata: &Value, key: &str) -> QueriaResult<Option<u32>> {
    metadata
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| {
            u32::try_from(value).map_err(|_| {
                QueriaError::Infrastructure(format!(
                    "database returned {key} outside the supported range"
                ))
            })
        })
        .transpose()
}

fn parse_scope(scope: &str) -> QueriaResult<KnowledgeScope> {
    match scope {
        "global" => Ok(KnowledgeScope::Global),
        "project" => Ok(KnowledgeScope::Project),
        value => Err(QueriaError::Infrastructure(format!(
            "database returned invalid knowledge scope {value}"
        ))),
    }
}

fn to_infrastructure_error(error: sqlx::Error) -> QueriaError {
    QueriaError::Infrastructure(format!("hybrid retrieval repository failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_search_is_scope_filtered_and_uses_simple_fts() {
        let normalized = LEXICAL_SEARCH_SQL.to_ascii_lowercase();

        assert!(normalized.contains("websearch_to_tsquery('simple'"));
        assert!(normalized.contains("k.status = 'approved'"));
        assert!(normalized.contains("access.include_global"));
        assert!(normalized.contains("ts_rank_cd"));
    }

    #[test]
    fn lexical_search_has_bounded_relaxed_candidates() {
        let sql = LEXICAL_SEARCH_SQL.to_ascii_lowercase();
        assert!(sql.contains("websearch_to_tsquery('simple'"));
        assert!(sql.contains("to_tsvector('simple'"));
        assert!(sql.contains(" | "));
        assert!(sql.contains("k.status = 'approved'"));
        assert!(sql.contains("k.organization_id = access.organization_id"));
        assert!(sql.contains("access.include_global"));
    }

    #[test]
    fn hydration_rechecks_authorization() {
        let normalized = HYDRATE_SQL.to_ascii_lowercase();

        assert!(normalized.contains("unnest($2::uuid[]) with ordinality"));
        assert!(normalized.contains("k.organization_id = access.organization_id"));
        assert!(normalized.contains("k.status = 'approved'"));
    }
}

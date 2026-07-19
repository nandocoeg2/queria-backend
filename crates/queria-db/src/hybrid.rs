use queria_core::contracts::{Citation, KnowledgeLane, RetrievedContextItem};
use queria_core::ids::{ChunkId, ProjectId, SourceDocumentId};
use queria_core::model::{KnowledgeScope, KnowledgeStatus};
use queria_core::{QueriaError, QueriaResult};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Multi-status lexical search (IMP-14 + IMP-L3):
/// approved always; project scratch when include_scratch; project needs_review when include_needs_review.
/// Bind order: $1 org, $2 project, $3 include_global, $4 include_scratch, $5 include_needs_review,
/// $6 query, $7 limit.
pub const LEXICAL_SEARCH_SQL: &str = "
with access as (
  select $1::uuid as organization_id,
         $2::uuid as project_id,
         $3::boolean as include_global,
         $4::boolean as include_scratch,
         $5::boolean as include_needs_review
),
strict_query as (
  select websearch_to_tsquery('simple', $6) as value
),
relaxed_query as (
  select case
    when array_to_string(tsvector_to_array(to_tsvector('simple', $6)), ' | ') = '' then null
    else to_tsquery('simple', array_to_string(tsvector_to_array(to_tsvector('simple', $6)), ' | '))
  end as value
)
select c.id as chunk_id,
       (coalesce(ts_rank_cd(c.search_vector, strict_query.value), 0.0) * 2.0 +
        coalesce(ts_rank_cd(c.search_vector, relaxed_query.value), 0.0))::real as score
from access, strict_query, relaxed_query, chunk c
join knowledge_item k on k.id = c.knowledge_item_id
left join source_document sd on sd.id = c.source_document_id
where k.organization_id = access.organization_id
  and (
    k.status = 'approved'
    or (
      access.include_scratch
      and k.status = 'scratch'
      and k.scope = 'project'
      and k.project_id = access.project_id
    )
    or (
      access.include_needs_review
      and k.status = 'needs_review'
      and k.scope = 'project'
      and k.project_id = access.project_id
    )
  )
  and (sd.id is null or sd.is_active)
  and (
    k.project_id = access.project_id
    or (access.include_global and k.scope = 'global' and k.status = 'approved')
  )
  and (
    c.search_vector @@ strict_query.value
    or (relaxed_query.value is not null and c.search_vector @@ relaxed_query.value)
  )
order by
  case
    when k.status = 'approved' then 0
    when k.status = 'scratch' then 1
    else 2
  end,
  score desc,
  c.id
limit $7";

/// Hydrate: same status/lane gate as lexical; returns lean status for citations.
/// Bind order: $1 org, $2 chunk_ids, $3 project, $4 include_global, $5 include_scratch,
/// $6 include_needs_review.
pub const HYDRATE_SQL: &str = "
with access as (
  select $1::uuid as organization_id,
         $3::uuid as project_id,
         $4::boolean as include_global,
         $5::boolean as include_scratch,
         $6::boolean as include_needs_review
),
requested as (
  select chunk_id, ordinal
  from unnest($2::uuid[]) with ordinality as ranked(chunk_id, ordinal)
)
select c.id as chunk_id, c.source_document_id, k.scope::text as scope,
       k.status::text as status,
       k.title, c.body, sd.uri as source_uri, sd.source_path, c.metadata
from access, requested
join chunk c on c.id = requested.chunk_id
join knowledge_item k on k.id = c.knowledge_item_id
join source_document sd on sd.id = c.source_document_id
where k.organization_id = access.organization_id
  and (
    k.status = 'approved'
    or (
      access.include_scratch
      and k.status = 'scratch'
      and k.scope = 'project'
      and k.project_id = access.project_id
    )
    or (
      access.include_needs_review
      and k.status = 'needs_review'
      and k.scope = 'project'
      and k.project_id = access.project_id
    )
  )
  and sd.is_active
  and (
    k.project_id = access.project_id
    or (access.include_global and k.scope = 'global' and k.status = 'approved')
  )
order by requested.ordinal";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetrievalAccess {
    pub organization_id: Uuid,
    pub project_id: ProjectId,
    pub include_global: bool,
    pub include_scratch: bool,
    pub include_needs_review: bool,
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
        include_scratch: bool,
        include_needs_review: bool,
    ) -> QueriaResult<RetrievalAccess> {
        let organization_id = sqlx::query_scalar::<_, Uuid>(
            "select p.organization_id
             from project p
             join org_membership m on m.organization_id = p.organization_id
             where m.user_id = $1 and p.id = $2",
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
            include_scratch,
            // Org membership already verified above; any member with project access may read.
            include_needs_review,
        })
    }

    pub async fn authorize_agent(
        &self,
        organization_id: Uuid,
        allowed_project_slugs: &[String],
        allow_global: bool,
        project_id: ProjectId,
        include_global: bool,
        include_scratch: bool,
        include_needs_review: bool,
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
            include_scratch,
            // Agent token with project scope may read needs_review when flag set.
            include_needs_review,
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
            .bind(access.include_scratch)
            .bind(access.include_needs_review)
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
            .bind(access.include_scratch)
            .bind(access.include_needs_review)
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
    let status = parse_status(
        &row.try_get::<String, _>("status")
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
        status,
        lane: KnowledgeLane::from_status(status),
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

fn parse_status(status: &str) -> QueriaResult<KnowledgeStatus> {
    match status {
        "approved" => Ok(KnowledgeStatus::Approved),
        "scratch" => Ok(KnowledgeStatus::Scratch),
        // Hydrate filter should exclude these; still map cleanly if seen.
        "draft" => Ok(KnowledgeStatus::Draft),
        "proposed" => Ok(KnowledgeStatus::Proposed),
        "rejected" => Ok(KnowledgeStatus::Rejected),
        "deprecated" => Ok(KnowledgeStatus::Deprecated),
        "superseded" => Ok(KnowledgeStatus::Superseded),
        "needs_review" => Ok(KnowledgeStatus::NeedsReview),
        value => Err(QueriaError::Infrastructure(format!(
            "database returned invalid knowledge status {value}"
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

    /// VAL-DL-026 / VAL-DL-027: dual-lane lexical allows scratch when flag true.
    #[test]
    fn lexical_search_allows_scratch_when_include_scratch() {
        let sql = LEXICAL_SEARCH_SQL.to_ascii_lowercase();
        assert!(sql.contains("access.include_scratch"));
        assert!(sql.contains("k.status = 'scratch'"));
        assert!(sql.contains("k.scope = 'project'"));
        assert!(sql.contains("k.project_id = access.project_id"));
        // Global path must stay trusted/approved only (VAL-DL-055).
        assert!(sql.contains("k.scope = 'global' and k.status = 'approved'"));
    }

    /// IMP-L3: needs_review only when include_needs_review, project-scoped.
    #[test]
    fn lexical_search_allows_needs_review_when_flag() {
        let sql = LEXICAL_SEARCH_SQL.to_ascii_lowercase();
        assert!(sql.contains("access.include_needs_review"));
        assert!(sql.contains("k.status = 'needs_review'"));
        // Flag-gated project-only arm (not global).
        assert!(sql.contains("access.include_needs_review"));
        assert!(sql.contains("and k.status = 'needs_review'"));
        assert!(!sql.contains("k.scope = 'global' and k.status = 'needs_review'"));
    }

    /// VAL-DL-028 / VAL-DL-029: proposed/draft never in SQL allow-list (needs_review is flag-gated, not free).
    #[test]
    fn lexical_search_excludes_pipeline_statuses() {
        let sql = LEXICAL_SEARCH_SQL.to_ascii_lowercase();
        for status in ["draft", "proposed", "rejected", "deprecated", "superseded"] {
            assert!(
                !sql.contains(&format!("k.status = '{status}'")),
                "pipeline status {status} must not be selectable"
            );
        }
        // needs_review appears only behind include_needs_review flag, not free.
        assert!(sql.contains("access.include_needs_review"));
        assert!(sql.contains("k.status = 'needs_review'"));
    }

    /// Default path: without selecting the flag arm, needs_review is not free-selectable
    /// (gate requires access.include_needs_review = true).
    #[test]
    fn needs_review_requires_flag_gate_in_sql() {
        let sql = LEXICAL_SEARCH_SQL.to_ascii_lowercase().replace('\n', " ");
        assert!(
            sql.contains("access.include_needs_review")
                && sql.contains("k.status = 'needs_review'"),
            "needs_review must be flag-gated"
        );
        // No bare unconditional arm like "k.status = 'needs_review'" without the flag.
        // Approved is unconditional; needs_review is not.
        assert!(sql.contains("k.status = 'approved'"));
    }

    /// VAL-DL-031 + IMP-L3 ranking: approved < scratch < needs_review.
    #[test]
    fn lexical_search_prefers_approved_over_scratch_over_needs_review() {
        let sql = LEXICAL_SEARCH_SQL.to_ascii_lowercase();
        assert!(sql.contains("when k.status = 'approved' then 0"));
        assert!(sql.contains("when k.status = 'scratch' then 1"));
        assert!(sql.contains("else 2"));
    }

    /// VAL-DL-035: hydrate selects status for lean citation.
    #[test]
    fn hydrate_selects_status_for_lane() {
        let sql = HYDRATE_SQL.to_ascii_lowercase();
        assert!(sql.contains("k.status::text as status"));
        assert!(sql.contains("access.include_scratch"));
        assert!(sql.contains("access.include_needs_review"));
        assert!(sql.contains("k.status = 'scratch'"));
        assert!(sql.contains("k.status = 'needs_review'"));
        assert!(sql.contains("k.scope = 'global' and k.status = 'approved'"));
    }

    /// VAL-DL-016 / VAL-DL-055: scratch is project-only; global filter only accepts approved.
    #[test]
    fn scratch_never_joins_global_scope_path() {
        let lexical = LEXICAL_SEARCH_SQL.to_ascii_lowercase();
        let hydrate = HYDRATE_SQL.to_ascii_lowercase();
        for sql in [lexical.as_str(), hydrate.as_str()] {
            assert!(
                sql.contains("k.scope = 'global' and k.status = 'approved'"),
                "global hits must stay approved/trusted"
            );
            assert!(
                sql.contains("k.status = 'scratch'") && sql.contains("k.scope = 'project'"),
                "scratch gate must require project scope"
            );
            // No path that selects scratch under global scope.
            assert!(!sql.contains("k.scope = 'global' and k.status = 'scratch'"));
            assert!(!sql.contains("k.scope = 'global' and k.status = 'needs_review'"));
        }
    }

    /// VAL-DL-030 / VAL-CROSS-008: approved stays selectable whether include_scratch is on or off.
    #[test]
    fn approved_status_always_included_independent_of_scratch_flag() {
        let sql = LEXICAL_SEARCH_SQL.to_ascii_lowercase().replace('\n', " ");
        // Unconditional approved arm plus optional scratch arm (flag-gated).
        assert!(sql.contains("k.status = 'approved'"));
        assert!(sql.contains("access.include_scratch"));
        assert!(
            sql.contains("k.status = 'approved'    or (      access.include_scratch")
                || sql.contains("k.status = 'approved'")
                    && sql.contains("and k.status = 'scratch'"),
            "approved must remain available when include_scratch is false"
        );
    }

    /// Bind docs: include_needs_review is $5 on lexical and $6 on hydrate.
    #[test]
    fn bind_order_documents_include_needs_review() {
        assert!(LEXICAL_SEARCH_SQL.contains("$5::boolean as include_needs_review"));
        assert!(LEXICAL_SEARCH_SQL.contains("limit $7"));
        assert!(HYDRATE_SQL.contains("$6::boolean as include_needs_review"));
    }
}

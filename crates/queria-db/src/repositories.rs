use chrono::{DateTime, Utc};
use mockall::automock;
use queria_auth::permissions::AgentTokenPermissions;
use queria_core::QueriaError;
use queria_core::QueriaResult;
use queria_core::contracts::{Citation, RetrievedContextItem};
use queria_core::ids::{AgentTokenId, ChunkId, ProjectId, SourceDocumentId};
use queria_core::model::KnowledgeScope;
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[automock]
pub trait KnowledgeRepository: Send + Sync {
    fn search_approved_chunks(
        &self,
        project_id: ProjectId,
        query: &str,
        limit: u32,
    ) -> QueriaResult<Vec<RetrievedContextItem>>;
}

#[automock]
pub trait SourceRepository: Send + Sync {
    fn get_source_document(&self, source_document_id: SourceDocumentId) -> QueriaResult<String>;
}

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

#[derive(Clone, Debug)]
pub struct PgAuthRepository {
    pool: PgPool,
}

#[derive(Clone, Debug)]
pub struct PgProjectRepository {
    pool: PgPool,
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedSession {
    pub user_id: Uuid,
    pub email: String,
    pub expires_at: DateTime<Utc>,
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

impl PgAuthRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn setup_required(&self) -> QueriaResult<bool> {
        sqlx::query_scalar::<_, bool>("select not exists(select 1 from user_account)")
            .fetch_one(&self.pool)
            .await
            .map_err(to_infrastructure_error)
    }

    pub async fn complete_first_run(
        &self,
        params: CompleteSetupParams,
    ) -> QueriaResult<CreatedAdmin> {
        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;

        let setup_required =
            sqlx::query_scalar::<_, bool>("select not exists(select 1 from user_account)")
                .fetch_one(&mut *transaction)
                .await
                .map_err(to_infrastructure_error)?;

        if !setup_required {
            return Err(QueriaError::Validation(
                "first-run setup has already been completed".to_owned(),
            ));
        }

        let organization_id = sqlx::query(
            "insert into organization(slug, name)
             values ($1, $2)
             returning id",
        )
        .bind(&params.organization_slug)
        .bind(&params.organization_name)
        .fetch_one(&mut *transaction)
        .await
        .and_then(|row| row.try_get::<Uuid, _>("id"))
        .map_err(to_infrastructure_error)?;

        let user_id = sqlx::query(
            "insert into user_account(organization_id, email, password_hash, role)
             values ($1, $2, $3, 'admin')
             returning id",
        )
        .bind(organization_id)
        .bind(&params.admin_email)
        .bind(&params.password_hash)
        .fetch_one(&mut *transaction)
        .await
        .and_then(|row| row.try_get::<Uuid, _>("id"))
        .map_err(to_infrastructure_error)?;

        sqlx::query(
            "insert into setup_state(id, setup_token_hash, consumed_at, consumed_by_user_id)
             values (true, $1, now(), $2)
             on conflict (id) do update
             set setup_token_hash = excluded.setup_token_hash,
                 consumed_at = excluded.consumed_at,
                 consumed_by_user_id = excluded.consumed_by_user_id,
                 updated_at = now()",
        )
        .bind(&params.setup_token_hash)
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        Ok(CreatedAdmin {
            organization_id,
            user_id,
            organization_slug: params.organization_slug,
            email: params.admin_email,
        })
    }

    pub async fn find_user_by_email(&self, email: &str) -> QueriaResult<Option<AuthUser>> {
        sqlx::query(
            "select id, email, password_hash
             from user_account
             where lower(email) = lower($1)",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(|row| {
            Ok(AuthUser {
                id: row.try_get("id")?,
                email: row.try_get("email")?,
                password_hash: row.try_get("password_hash")?,
            })
        })
        .transpose()
        .map_err(to_infrastructure_error)
    }

    pub async fn create_session(
        &self,
        user_id: Uuid,
        token_prefix: &str,
        token_hash: &str,
        expires_at: DateTime<Utc>,
    ) -> QueriaResult<Uuid> {
        sqlx::query(
            "insert into user_session(user_id, token_prefix, token_hash, expires_at)
             values ($1, $2, $3, $4)
             returning id",
        )
        .bind(user_id)
        .bind(token_prefix)
        .bind(token_hash)
        .bind(expires_at)
        .fetch_one(&self.pool)
        .await
        .and_then(|row| row.try_get::<Uuid, _>("id"))
        .map_err(to_infrastructure_error)
    }

    pub async fn find_session_by_hash(
        &self,
        token_hash: &str,
    ) -> QueriaResult<Option<AuthenticatedSession>> {
        sqlx::query(
            "select u.id as user_id, u.email, s.expires_at
             from user_session s
             join user_account u on u.id = s.user_id
             where s.token_hash = $1
               and s.revoked_at is null
               and s.expires_at > now()",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(|row| {
            Ok(AuthenticatedSession {
                user_id: row.try_get("user_id")?,
                email: row.try_get("email")?,
                expires_at: row.try_get("expires_at")?,
            })
        })
        .transpose()
        .map_err(to_infrastructure_error)
    }
}

fn project_from_row(row: sqlx::postgres::PgRow) -> QueriaResult<ProjectRecord> {
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

fn source_from_row(row: sqlx::postgres::PgRow) -> QueriaResult<SourceDocumentRecord> {
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

fn agent_token_from_row(row: sqlx::postgres::PgRow) -> QueriaResult<AgentTokenRecord> {
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

fn authenticated_agent_token_from_row(
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

fn retrieved_item_from_row(row: sqlx::postgres::PgRow) -> QueriaResult<RetrievedContextItem> {
    let scope: String = row.try_get("scope").map_err(to_infrastructure_error)?;
    let source_document_id: Uuid = row
        .try_get("source_document_id")
        .map_err(to_infrastructure_error)?;

    Ok(RetrievedContextItem {
        chunk_id: ChunkId::from_uuid(row.try_get("chunk_id").map_err(to_infrastructure_error)?),
        source_document_id: SourceDocumentId::from_uuid(source_document_id),
        scope: parse_knowledge_scope(&scope)?,
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

fn parse_agent_permissions(value: Value) -> QueriaResult<AgentTokenPermissions> {
    serde_json::from_value(value).map_err(|error| {
        QueriaError::Infrastructure(format!(
            "database returned invalid agent token permissions: {error}"
        ))
    })
}

fn parse_knowledge_scope(value: &str) -> QueriaResult<KnowledgeScope> {
    match value {
        "global" => Ok(KnowledgeScope::Global),
        "project" => Ok(KnowledgeScope::Project),
        _ => Err(QueriaError::Infrastructure(format!(
            "database returned unknown knowledge scope: {value}"
        ))),
    }
}

fn parse_optional_u32(value: Option<String>) -> QueriaResult<Option<u32>> {
    value
        .map(|raw| {
            raw.parse::<u32>().map_err(|error| {
                QueriaError::Infrastructure(format!("invalid chunk line metadata: {error}"))
            })
        })
        .transpose()
}

async fn organization_id_for_user(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
) -> QueriaResult<Uuid> {
    sqlx::query_scalar::<_, Uuid>("select organization_id from user_account where id = $1")
        .bind(user_id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(to_infrastructure_error)
}

async fn count_accessible_project_slugs(
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

async fn project_id_for_slug(
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

fn to_infrastructure_error(error: sqlx::Error) -> QueriaError {
    QueriaError::Infrastructure(format!("database repository failed: {error}"))
}

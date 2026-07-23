use queria_core::ids::AgentTokenId;
use queria_core::{QueriaError, QueriaResult};
use sqlx::Row;
use uuid::Uuid;

use super::super::types::{
    AgentTokenRecord, AuthenticatedAgentToken, CreateAgentTokenParams, agent_token_from_row,
    authenticated_agent_token_from_row, count_accessible_project_slugs, organization_id_for_user,
    project_id_for_slug, to_infrastructure_error,
};
use super::PgProjectRepository;

impl PgProjectRepository {
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
}

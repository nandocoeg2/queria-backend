use chrono::{DateTime, Utc};
use mockall::automock;
use queria_core::QueriaError;
use queria_core::QueriaResult;
use queria_core::contracts::RetrievedContextItem;
use queria_core::ids::{ProjectId, SourceDocumentId};
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

#[derive(Clone, Debug)]
pub struct PgAuthRepository {
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

fn to_infrastructure_error(error: sqlx::Error) -> QueriaError {
    QueriaError::Infrastructure(format!("database repository failed: {error}"))
}

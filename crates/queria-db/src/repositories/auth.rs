use chrono::{DateTime, Utc};
use queria_core::{QueriaError, QueriaResult};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::types::{
    AuthUser, AuthenticatedSession, CompleteSetupParams, CreatedAdmin, to_infrastructure_error,
};

/// Resolve session home org for multi-org v1.
/// Prefer sole membership over legacy `user_account.organization_id`.
/// Super-admin (or anyone) without membership gets `None` (no invented home org).
#[must_use]
pub fn resolve_active_organization_id(
    membership_organization_id: Option<Uuid>,
    _legacy_organization_id: Uuid,
) -> Option<Uuid> {
    membership_organization_id
}

#[derive(Clone, Debug)]
pub struct PgAuthRepository {
    pool: PgPool,
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

        // Align membership with legacy organization_id for first-run admin.
        sqlx::query(
            "insert into org_membership(user_id, organization_id, role)
             values ($1, $2, 'org_admin')
             on conflict do nothing",
        )
        .bind(user_id)
        .bind(organization_id)
        .execute(&mut *transaction)
        .await
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
            "select u.id,
                    u.email,
                    u.password_hash,
                    u.organization_id,
                    u.is_platform_super_admin,
                    m.organization_id as membership_organization_id
             from user_account u
             left join org_membership m on m.user_id = u.id
             where lower(u.email) = lower($1)",
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
                organization_id: row.try_get("organization_id")?,
                membership_organization_id: row.try_get("membership_organization_id")?,
                is_platform_super_admin: row.try_get("is_platform_super_admin")?,
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
        active_organization_id: Option<Uuid>,
    ) -> QueriaResult<Uuid> {
        sqlx::query(
            "insert into user_session(
               user_id, token_prefix, token_hash, expires_at, active_organization_id
             )
             values ($1, $2, $3, $4, $5)
             returning id",
        )
        .bind(user_id)
        .bind(token_prefix)
        .bind(token_hash)
        .bind(expires_at)
        .bind(active_organization_id)
        .fetch_one(&self.pool)
        .await
        .and_then(|row| row.try_get::<Uuid, _>("id"))
        .map_err(to_infrastructure_error)
    }

    pub async fn find_session_by_hash(
        &self,
        token_hash: &str,
    ) -> QueriaResult<Option<AuthenticatedSession>> {
        // Prefer sole membership org over whatever was persisted (re-validate membership).
        // Super-admin effective flag is DB-only here; env bootstrap applied at API layer.
        sqlx::query(
            "select u.id as user_id,
                    u.email,
                    s.expires_at,
                    u.is_platform_super_admin,
                    m.organization_id as membership_organization_id,
                    s.active_organization_id as session_active_organization_id
             from user_session s
             join user_account u on u.id = s.user_id
             left join org_membership m on m.user_id = u.id
             where s.token_hash = $1
               and s.revoked_at is null
               and s.expires_at > now()",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(|row| {
            let membership_organization_id: Option<Uuid> =
                row.try_get("membership_organization_id")?;
            // Membership is source of truth when present; otherwise keep stored session
            // value only if no membership (super-admin null path uses None).
            let active_organization_id =
                resolve_active_organization_id(membership_organization_id, Uuid::nil());

            Ok(AuthenticatedSession {
                user_id: row.try_get("user_id")?,
                email: row.try_get("email")?,
                expires_at: row.try_get("expires_at")?,
                active_organization_id,
                is_platform_super_admin: row.try_get("is_platform_super_admin")?,
            })
        })
        .transpose()
        .map_err(to_infrastructure_error)
    }

    /// Whether email is listed in platform super-admin env bootstrap (case-insensitive).
    #[must_use]
    pub fn email_in_platform_super_admin_list(email: &str, configured_emails: &[String]) -> bool {
        let needle = email.trim().to_ascii_lowercase();
        configured_emails
            .iter()
            .any(|entry| entry.trim().to_ascii_lowercase() == needle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_active_organization_prefers_membership_over_legacy() {
        let membership = Uuid::now_v7();
        let legacy = Uuid::now_v7();
        assert_eq!(
            resolve_active_organization_id(Some(membership), legacy),
            Some(membership)
        );
    }

    #[test]
    fn resolve_active_organization_is_none_without_membership() {
        let legacy = Uuid::now_v7();
        // Super-admin / orphan without membership: do not invent home from legacy alone.
        assert_eq!(resolve_active_organization_id(None, legacy), None);
    }

    #[test]
    fn platform_super_admin_email_list_is_case_insensitive() {
        let list = vec!["Nando@Example.COM".to_owned(), "ops@coria.test".to_owned()];
        assert!(PgAuthRepository::email_in_platform_super_admin_list(
            "nando@example.com",
            &list
        ));
        assert!(!PgAuthRepository::email_in_platform_super_admin_list(
            "other@example.com",
            &list
        ));
    }
}

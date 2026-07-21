use chrono::{DateTime, Utc};
use queria_core::{QueriaError, QueriaResult};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::types::to_infrastructure_error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrganizationRecord {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrgInviteRecord {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub email: String,
    pub role: String,
    pub token_prefix: String,
    pub expires_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrgMemberRecord {
    pub user_id: Uuid,
    pub email: String,
    pub role: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateOrganizationParams {
    pub slug: String,
    pub name: String,
    pub first_admin_email: String,
    pub token_prefix: String,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub invited_by_user_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateOrgInviteParams {
    pub organization_id: Uuid,
    pub email: String,
    pub role: String,
    pub token_prefix: String,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub invited_by_user_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptOrgInviteParams {
    pub token_hash: String,
    pub password_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedOrgInvite {
    pub user_id: Uuid,
    pub email: String,
    pub organization_id: Uuid,
    pub organization_slug: String,
    pub role: String,
    pub created_user: bool,
}

/// Invite row loaded for accept validation (includes security fields).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrgInviteForAccept {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub organization_slug: String,
    pub email: String,
    pub role: String,
    pub expires_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug)]
pub struct PgOrgRepository {
    pool: PgPool,
}

impl PgOrgRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn list_organizations(&self) -> QueriaResult<Vec<OrganizationRecord>> {
        sqlx::query(
            "select id, slug, name, created_at
             from organization
             order by slug",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .into_iter()
        .map(organization_from_row)
        .collect()
    }

    pub async fn get_organization_by_slug(
        &self,
        slug: &str,
    ) -> QueriaResult<Option<OrganizationRecord>> {
        sqlx::query(
            "select id, slug, name, created_at
             from organization
             where slug = $1",
        )
        .bind(slug)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(organization_from_row)
        .transpose()
    }

    /// Create organization + first admin invite in one transaction.
    pub async fn create_organization_with_invite(
        &self,
        params: CreateOrganizationParams,
    ) -> QueriaResult<(OrganizationRecord, OrgInviteRecord)> {
        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;

        let org_row = sqlx::query(
            "insert into organization(slug, name)
             values ($1, $2)
             on conflict (slug) do nothing
             returning id, slug, name, created_at",
        )
        .bind(&params.slug)
        .bind(&params.name)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let Some(org_row) = org_row else {
            return Err(QueriaError::Validation(
                "organization_slug_exists".to_owned(),
            ));
        };

        let organization = organization_from_row(org_row)?;

        let invite_row = sqlx::query(
            "insert into org_invite(
               organization_id, email, role, token_hash, token_prefix,
               invited_by_user_id, expires_at
             )
             values ($1, $2, 'org_admin', $3, $4, $5, $6)
             returning id, organization_id, email, role, token_prefix,
                       expires_at, accepted_at, revoked_at, created_at",
        )
        .bind(organization.id)
        .bind(&params.first_admin_email)
        .bind(&params.token_hash)
        .bind(&params.token_prefix)
        .bind(params.invited_by_user_id)
        .bind(params.expires_at)
        .fetch_one(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let invite = invite_from_row(invite_row)?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        Ok((organization, invite))
    }

    pub async fn create_invite(
        &self,
        params: CreateOrgInviteParams,
    ) -> QueriaResult<OrgInviteRecord> {
        sqlx::query(
            "insert into org_invite(
               organization_id, email, role, token_hash, token_prefix,
               invited_by_user_id, expires_at
             )
             values ($1, $2, $3, $4, $5, $6, $7)
             returning id, organization_id, email, role, token_prefix,
                       expires_at, accepted_at, revoked_at, created_at",
        )
        .bind(params.organization_id)
        .bind(&params.email)
        .bind(&params.role)
        .bind(&params.token_hash)
        .bind(&params.token_prefix)
        .bind(params.invited_by_user_id)
        .bind(params.expires_at)
        .fetch_one(&self.pool)
        .await
        .map_err(to_infrastructure_error)
        .and_then(invite_from_row)
    }

    /// Membership role for user in org, if any.
    pub async fn membership_role(
        &self,
        user_id: Uuid,
        organization_id: Uuid,
    ) -> QueriaResult<Option<String>> {
        sqlx::query_scalar(
            "select role from org_membership
             where user_id = $1 and organization_id = $2",
        )
        .bind(user_id)
        .bind(organization_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)
    }

    pub async fn list_members(&self, organization_id: Uuid) -> QueriaResult<Vec<OrgMemberRecord>> {
        sqlx::query(
            "select m.user_id, u.email, m.role, m.created_at
             from org_membership m
             join user_account u on u.id = m.user_id
             where m.organization_id = $1
             order by u.email",
        )
        .bind(organization_id)
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .into_iter()
        .map(|row| {
            Ok(OrgMemberRecord {
                user_id: row.try_get("user_id")?,
                email: row.try_get("email")?,
                role: row.try_get("role")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect::<Result<_, sqlx::Error>>()
        .map_err(to_infrastructure_error)
    }

    pub async fn find_invite_by_token_hash(
        &self,
        token_hash: &str,
    ) -> QueriaResult<Option<OrgInviteForAccept>> {
        sqlx::query(
            "select i.id, i.organization_id, o.slug as organization_slug,
                    i.email, i.role, i.expires_at, i.accepted_at, i.revoked_at
             from org_invite i
             join organization o on o.id = i.organization_id
             where i.token_hash = $1",
        )
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(|row| {
            Ok(OrgInviteForAccept {
                id: row.try_get("id")?,
                organization_id: row.try_get("organization_id")?,
                organization_slug: row.try_get("organization_slug")?,
                email: row.try_get("email")?,
                role: row.try_get("role")?,
                expires_at: row.try_get("expires_at")?,
                accepted_at: row.try_get("accepted_at")?,
                revoked_at: row.try_get("revoked_at")?,
            })
        })
        .transpose()
        .map_err(to_infrastructure_error)
    }

    /// Accept invite: create user if needed, membership, sync organization_id, mark used.
    /// Rejects second-org membership (unique user_id).
    pub async fn accept_invite(
        &self,
        params: AcceptOrgInviteParams,
    ) -> QueriaResult<AcceptedOrgInvite> {
        let mut transaction = self.pool.begin().await.map_err(to_infrastructure_error)?;

        let invite_row = sqlx::query(
            "select i.id, i.organization_id, o.slug as organization_slug,
                    i.email, i.role, i.expires_at, i.accepted_at, i.revoked_at
             from org_invite i
             join organization o on o.id = i.organization_id
             where i.token_hash = $1
             for update of i",
        )
        .bind(&params.token_hash)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let Some(invite_row) = invite_row else {
            return Err(QueriaError::Validation("invite_invalid".to_owned()));
        };

        let invite = OrgInviteForAccept {
            id: invite_row.try_get("id").map_err(to_infrastructure_error)?,
            organization_id: invite_row
                .try_get("organization_id")
                .map_err(to_infrastructure_error)?,
            organization_slug: invite_row
                .try_get("organization_slug")
                .map_err(to_infrastructure_error)?,
            email: invite_row
                .try_get("email")
                .map_err(to_infrastructure_error)?,
            role: invite_row
                .try_get("role")
                .map_err(to_infrastructure_error)?,
            expires_at: invite_row
                .try_get("expires_at")
                .map_err(to_infrastructure_error)?,
            accepted_at: invite_row
                .try_get("accepted_at")
                .map_err(to_infrastructure_error)?,
            revoked_at: invite_row
                .try_get("revoked_at")
                .map_err(to_infrastructure_error)?,
        };

        if invite.revoked_at.is_some() {
            return Err(QueriaError::Validation("invite_revoked".to_owned()));
        }
        if invite.accepted_at.is_some() {
            return Err(QueriaError::Validation("invite_already_used".to_owned()));
        }
        if invite.expires_at <= Utc::now() {
            return Err(QueriaError::Validation("invite_expired".to_owned()));
        }

        let email = invite.email.trim().to_ascii_lowercase();

        // Lock candidate user row (if any) before membership check.
        let existing_user_id: Option<Uuid> = sqlx::query_scalar(
            "select id from user_account
             where lower(email) = lower($1)
             order by created_at asc
             limit 1
             for update",
        )
        .bind(&email)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        let (user_id, created_user) = if let Some(user_id) = existing_user_id {
            let membership_org: Option<Uuid> =
                sqlx::query_scalar("select organization_id from org_membership where user_id = $1")
                    .bind(user_id)
                    .fetch_optional(&mut *transaction)
                    .await
                    .map_err(to_infrastructure_error)?;

            if let Some(existing_org) = membership_org {
                if existing_org != invite.organization_id {
                    return Err(QueriaError::Validation(
                        "already_member_of_other_org".to_owned(),
                    ));
                }
                // Same org: mark invite accepted (idempotent re-accept safety).
                sqlx::query(
                    "update org_invite
                     set accepted_at = now()
                     where id = $1 and accepted_at is null",
                )
                .bind(invite.id)
                .execute(&mut *transaction)
                .await
                .map_err(to_infrastructure_error)?;

                transaction
                    .commit()
                    .await
                    .map_err(to_infrastructure_error)?;

                return Ok(AcceptedOrgInvite {
                    user_id,
                    email,
                    organization_id: invite.organization_id,
                    organization_slug: invite.organization_slug,
                    role: invite.role,
                    created_user: false,
                });
            }

            // Existing user with no membership: attach to invite org.
            sqlx::query(
                "update user_account
                 set organization_id = $1,
                     password_hash = $2,
                     updated_at = now()
                 where id = $3",
            )
            .bind(invite.organization_id)
            .bind(&params.password_hash)
            .bind(user_id)
            .execute(&mut *transaction)
            .await
            .map_err(to_infrastructure_error)?;

            sqlx::query(
                "insert into org_membership(user_id, organization_id, role)
                 values ($1, $2, $3)",
            )
            .bind(user_id)
            .bind(invite.organization_id)
            .bind(&invite.role)
            .execute(&mut *transaction)
            .await
            .map_err(map_membership_insert_error)?;

            (user_id, false)
        } else {
            // Map invite role to legacy user_account.role for bootstrap compatibility.
            let legacy_role = if invite.role == "org_admin" {
                "admin"
            } else {
                "member"
            };

            let user_id: Uuid = sqlx::query_scalar(
                "insert into user_account(
                   organization_id, email, password_hash, role
                 )
                 values ($1, $2, $3, $4)
                 returning id",
            )
            .bind(invite.organization_id)
            .bind(&email)
            .bind(&params.password_hash)
            .bind(legacy_role)
            .fetch_one(&mut *transaction)
            .await
            .map_err(to_infrastructure_error)?;

            sqlx::query(
                "insert into org_membership(user_id, organization_id, role)
                 values ($1, $2, $3)",
            )
            .bind(user_id)
            .bind(invite.organization_id)
            .bind(&invite.role)
            .execute(&mut *transaction)
            .await
            .map_err(map_membership_insert_error)?;

            (user_id, true)
        };

        sqlx::query(
            "update org_invite
             set accepted_at = now()
             where id = $1 and accepted_at is null",
        )
        .bind(invite.id)
        .execute(&mut *transaction)
        .await
        .map_err(to_infrastructure_error)?;

        transaction
            .commit()
            .await
            .map_err(to_infrastructure_error)?;

        Ok(AcceptedOrgInvite {
            user_id,
            email,
            organization_id: invite.organization_id,
            organization_slug: invite.organization_slug,
            role: invite.role,
            created_user,
        })
    }

    /// Whether any invite for this org still stores the given raw-token-looking value
    /// (used in tests only via hash lookup helpers).
    pub async fn invite_token_hash_exists(&self, token_hash: &str) -> QueriaResult<bool> {
        sqlx::query_scalar("select exists(select 1 from org_invite where token_hash = $1)")
            .bind(token_hash)
            .fetch_one(&self.pool)
            .await
            .map_err(to_infrastructure_error)
    }

    pub async fn invite_stores_raw_token(
        &self,
        invite_id: Uuid,
        raw_token: &str,
    ) -> QueriaResult<bool> {
        // No plaintext column: compare token_hash column value equals raw (should never).
        sqlx::query_scalar(
            "select exists(
               select 1 from org_invite
               where id = $1 and (token_hash = $2 or token_prefix = $2)
             )",
        )
        .bind(invite_id)
        .bind(raw_token)
        .fetch_one(&self.pool)
        .await
        .map_err(to_infrastructure_error)
    }

    pub async fn get_invite_token_hash(&self, invite_id: Uuid) -> QueriaResult<Option<String>> {
        sqlx::query_scalar("select token_hash from org_invite where id = $1")
            .bind(invite_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(to_infrastructure_error)
    }
}

fn organization_from_row(row: sqlx::postgres::PgRow) -> QueriaResult<OrganizationRecord> {
    Ok(OrganizationRecord {
        id: row.try_get("id").map_err(to_infrastructure_error)?,
        slug: row.try_get("slug").map_err(to_infrastructure_error)?,
        name: row.try_get("name").map_err(to_infrastructure_error)?,
        created_at: row.try_get("created_at").map_err(to_infrastructure_error)?,
    })
}

fn invite_from_row(row: sqlx::postgres::PgRow) -> QueriaResult<OrgInviteRecord> {
    Ok(OrgInviteRecord {
        id: row.try_get("id").map_err(to_infrastructure_error)?,
        organization_id: row
            .try_get("organization_id")
            .map_err(to_infrastructure_error)?,
        email: row.try_get("email").map_err(to_infrastructure_error)?,
        role: row.try_get("role").map_err(to_infrastructure_error)?,
        token_prefix: row
            .try_get("token_prefix")
            .map_err(to_infrastructure_error)?,
        expires_at: row.try_get("expires_at").map_err(to_infrastructure_error)?,
        accepted_at: row
            .try_get("accepted_at")
            .map_err(to_infrastructure_error)?,
        revoked_at: row.try_get("revoked_at").map_err(to_infrastructure_error)?,
        created_at: row.try_get("created_at").map_err(to_infrastructure_error)?,
    })
}

fn map_membership_insert_error(error: sqlx::Error) -> QueriaError {
    if let sqlx::Error::Database(db) = &error
        && (db.constraint() == Some("idx_org_membership_one_org_per_user")
            || db.code().as_deref() == Some("23505"))
    {
        return QueriaError::Validation("already_member_of_other_org".to_owned());
    }
    to_infrastructure_error(error)
}

use queria_core::{QueriaError, QueriaResult};
use uuid::Uuid;

use super::super::types::{
    CreateProjectParams, ProjectRecord, project_from_row, to_infrastructure_error,
};
use super::PgProjectRepository;

impl PgProjectRepository {
    pub async fn list_projects(&self, user_id: Uuid) -> QueriaResult<Vec<ProjectRecord>> {
        sqlx::query(
            "select p.id, p.slug, p.name, p.description, p.default_embedding_model,
                    p.include_global_default, p.created_at, p.updated_at
             from project p
             join org_membership m on m.organization_id = p.organization_id
             where m.user_id = $1
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
             join org_membership m on m.organization_id = p.organization_id
             where m.user_id = $1
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
               from org_membership
               where user_id = $1
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
}

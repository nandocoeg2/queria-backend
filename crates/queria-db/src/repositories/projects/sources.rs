use queria_core::ids::SourceDocumentId;
use queria_core::{QueriaError, QueriaResult};
use uuid::Uuid;

use super::super::types::{
    RegisterSourceDocumentParams, SourceDocumentRecord, source_from_row, to_infrastructure_error,
};
use super::PgProjectRepository;

impl PgProjectRepository {
    pub async fn register_source_document(
        &self,
        user_id: Uuid,
        params: RegisterSourceDocumentParams,
    ) -> QueriaResult<SourceDocumentRecord> {
        let row = sqlx::query(
            "with scoped_project as (
               select p.id as project_id, p.organization_id
               from project p
               join org_membership m on m.organization_id = p.organization_id
               where m.user_id = $1
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
             join org_membership m on m.organization_id = sd.organization_id
             where m.user_id = $1
               and p.slug = $2
               and sd.source_root_id is null
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
             join org_membership m on m.organization_id = sd.organization_id
             where m.user_id = $1
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
}

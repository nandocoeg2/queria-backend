use queria_core::QueriaResult;

use super::super::types::to_infrastructure_error;
use super::PgProjectRepository;

impl PgProjectRepository {
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

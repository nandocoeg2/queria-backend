use crate::embeddings;
use queria_core::contracts::RetrieveContextRequest;
use queria_search::retrieval::{RetrievalPrincipal, build_pg_retrieval_service};

pub async fn probe(
    project_slug: &str,
    query: &str,
    include_global: bool,
    limit: u32,
) -> anyhow::Result<()> {
    let (config, pool, user_id, project_id) = embeddings::context(project_slug).await?;
    let service = build_pg_retrieval_service(&config, pool)?;
    let response = service
        .retrieve_context(
            &RetrievalPrincipal::User { user_id },
            RetrieveContextRequest {
                project_id,
                query: query.to_owned(),
                include_global,
                limit,
            },
        )
        .await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

use crate::embeddings;
use queria_core::contracts::RetrieveContextRequest;
use queria_core::evaluation::{
    evaluation_limit, parse_golden_questions_jsonl, score_evaluation_report,
};
use queria_search::retrieval::{RetrievalPrincipal, build_pg_retrieval_service};
use std::path::PathBuf;

pub async fn run(project_slug: &str) -> anyhow::Result<()> {
    let golden_path = PathBuf::from(format!("tests/golden_questions/{project_slug}.jsonl"));
    let content = std::fs::read_to_string(&golden_path)?;
    let questions = parse_golden_questions_jsonl(&content)?
        .into_iter()
        .filter(|question| question.project_slug == project_slug)
        .collect::<Vec<_>>();
    if questions.is_empty() {
        anyhow::bail!("no golden questions found for project {project_slug}");
    }

    let (config, pool, user_id, project_id) = embeddings::context(project_slug).await?;
    let service = build_pg_retrieval_service(&config, pool)?;
    let mut responses = Vec::with_capacity(questions.len());
    for question in &questions {
        let request = RetrieveContextRequest {
            project_id,
            query: question.query.clone(),
            include_global: question.include_global,
            limit: evaluation_limit(question.minimum_items),
        };
        let response = service
            .retrieve_context(&RetrievalPrincipal::User { user_id }, request)
            .await
            .map_err(|error| error.to_string());
        responses.push(response);
    }

    let report = score_evaluation_report(
        project_slug,
        &golden_path.display().to_string(),
        &questions,
        responses,
    );
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

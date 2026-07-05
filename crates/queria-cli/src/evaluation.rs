use crate::embeddings;
use queria_db::evaluation::PgEvaluationRepository;
use queria_search::evaluation::EvaluationExecutor;
use std::path::PathBuf;
use std::time::Duration;

const EVALUATION_RETRY_ATTEMPTS: usize = 3;
const EVALUATION_RETRY_DELAY_MS: u64 = 750;

pub async fn run(project_slug: &str) -> anyhow::Result<()> {
    let golden_path = PathBuf::from(format!("tests/golden_questions/{project_slug}.jsonl"));
    if !golden_path.exists() {
        anyhow::bail!(
            "no golden questions file found at {}",
            golden_path.display()
        );
    }

    let (config, pool, user_id, project_id) = embeddings::context(project_slug).await?;
    let service = queria_search::retrieval::build_pg_retrieval_service(&config, pool.clone())?;

    let executor = EvaluationExecutor::new(
        service,
        EVALUATION_RETRY_ATTEMPTS,
        Duration::from_millis(EVALUATION_RETRY_DELAY_MS),
    );

    let report = executor
        .run(user_id, project_slug, project_id, &golden_path)
        .await?;

    let repository = PgEvaluationRepository::new(pool);
    let evaluation = repository
        .insert_for_project_slug(user_id, project_slug, &report)
        .await?
        .ok_or_else(|| anyhow::anyhow!("project not found or permission denied"))?;

    println!("{}", serde_json::to_string_pretty(&evaluation)?);
    Ok(())
}

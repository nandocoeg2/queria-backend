use queria_core::AppConfig;
use queria_core::ids::ProjectId;
use queria_db::embedding::PgEmbeddingRepository;
use sqlx::Row;
use uuid::Uuid;

pub async fn backfill(project_slug: &str) -> anyhow::Result<()> {
    let (config, pool, user_id, project_id) = context(project_slug).await?;
    let repository = PgEmbeddingRepository::new(pool);
    let job = repository
        .enqueue_backfill(user_id, project_id, &config.embedding.profile_version)
        .await?
        .ok_or_else(|| anyhow::anyhow!("project is not accessible"))?;
    println!("{}", serde_json::to_string_pretty(&job)?);
    Ok(())
}

pub async fn status(project_slug: &str) -> anyhow::Result<()> {
    let (config, pool, _, project_id) = context(project_slug).await?;
    let counts = PgEmbeddingRepository::new(pool)
        .status_counts(project_id, &config.embedding.profile_version)
        .await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "project": project_slug,
            "embedding_profile_version": config.embedding.profile_version,
            "counts": counts
        }))?
    );
    Ok(())
}

pub async fn context(
    project_slug: &str,
) -> anyhow::Result<(AppConfig, sqlx::PgPool, Uuid, ProjectId)> {
    let config = AppConfig::from_env()?;
    let pool = queria_db::pool::connect(&config.database_url).await?;
    queria_db::migrate::run_migrations(&pool).await?;
    let row = sqlx::query(
        "select u.id as user_id, p.id as project_id
         from user_account u
         join project p on p.organization_id = u.organization_id
         where lower(u.email) = lower($1) and p.slug = $2",
    )
    .bind(&config.first_admin_email)
    .bind(project_slug)
    .fetch_optional(&pool)
    .await?
    .ok_or_else(|| anyhow::anyhow!("admin or project not found"))?;
    Ok((
        config,
        pool,
        row.try_get("user_id")?,
        ProjectId::from_uuid(row.try_get("project_id")?),
    ))
}

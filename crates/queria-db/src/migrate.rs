use queria_core::{QueriaError, QueriaResult};
use sqlx::{Executor, PgPool, Row};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Migration {
    pub version: &'static str,
    pub name: &'static str,
    pub sql: &'static str,
}

pub async fn run_migrations(pool: &PgPool) -> QueriaResult<()> {
    pool.execute(
        "create table if not exists _queria_migration (
            version text primary key,
            name text not null,
            applied_at timestamptz not null default now()
        )",
    )
    .await
    .map_err(to_infrastructure_error)?;

    for migration in bundled_migrations() {
        if migration_already_applied(pool, migration.version).await? {
            continue;
        }

        if migration.version == "20260704000100" && table_exists(pool, "organization").await? {
            record_migration(pool, migration).await?;
            continue;
        }

        sqlx::raw_sql(migration.sql)
            .execute(pool)
            .await
            .map_err(to_infrastructure_error)?;
        record_migration(pool, migration).await?;
    }

    Ok(())
}

async fn migration_already_applied(pool: &PgPool, version: &str) -> QueriaResult<bool> {
    sqlx::query_scalar::<_, bool>(
        "select exists(select 1 from _queria_migration where version = $1)",
    )
    .bind(version)
    .fetch_one(pool)
    .await
    .map_err(to_infrastructure_error)
}

async fn table_exists(pool: &PgPool, table_name: &str) -> QueriaResult<bool> {
    sqlx::query(
        "select exists(
            select 1
            from information_schema.tables
            where table_schema = 'public'
              and table_name = $1
        ) as exists",
    )
    .bind(table_name)
    .fetch_one(pool)
    .await
    .and_then(|row| row.try_get::<bool, _>("exists"))
    .map_err(to_infrastructure_error)
}

async fn record_migration(pool: &PgPool, migration: Migration) -> QueriaResult<()> {
    sqlx::query(
        "insert into _queria_migration(version, name)
         values ($1, $2)
         on conflict (version) do nothing",
    )
    .bind(migration.version)
    .bind(migration.name)
    .execute(pool)
    .await
    .map_err(to_infrastructure_error)?;
    Ok(())
}

fn to_infrastructure_error(error: sqlx::Error) -> QueriaError {
    QueriaError::Infrastructure(format!("database migration failed: {error}"))
}

pub fn bundled_migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: "20260704000100",
            name: "baseline",
            sql: include_str!("../../../migrations/20260704000100_baseline.sql"),
        },
        Migration {
            version: "20260704000200",
            name: "sessions_and_setup",
            sql: include_str!("../../../migrations/20260704000200_sessions_and_setup.sql"),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn baseline_migration_contains_mvp_tables() {
        let migrations = bundled_migrations();
        let sql = migrations
            .iter()
            .map(|migration| migration.sql)
            .collect::<Vec<_>>()
            .join("\n");

        for table in [
            "organization",
            "project",
            "source_document",
            "knowledge_item",
            "chunk",
            "approval",
            "agent_token",
            "setup_state",
            "user_session",
            "audit_log",
            "ingestion_job",
        ] {
            assert!(
                sql.contains(&format!("create table {table}")),
                "missing {table}"
            );
        }
    }
}

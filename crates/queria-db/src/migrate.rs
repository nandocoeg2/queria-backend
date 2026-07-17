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
        Migration {
            version: "20260704000300",
            name: "source_branch_and_retrieval_indexes",
            sql: include_str!(
                "../../../migrations/20260704000300_source_branch_and_retrieval_indexes.sql"
            ),
        },
        Migration {
            version: "20260704000400",
            name: "git_ingestion",
            sql: include_str!("../../../migrations/20260704000400_git_ingestion.sql"),
        },
        Migration {
            version: "20260704000500",
            name: "hybrid_retrieval",
            sql: include_str!("../../../migrations/20260704000500_hybrid_retrieval.sql"),
        },
        Migration {
            version: "20260704000600",
            name: "embedding_retry_backoff",
            sql: include_str!("../../../migrations/20260704000600_embedding_retry_backoff.sql"),
        },
        Migration {
            version: "20260704000700",
            name: "evaluation_reports",
            sql: include_str!("../../../migrations/20260704000700_evaluation_reports.sql"),
        },
        Migration {
            version: "20260705000100",
            name: "backup_records",
            sql: include_str!("../../../migrations/20260705000100_backup_records.sql"),
        },
        Migration {
            version: "20260717000100",
            name: "knowledge_status_scratch",
            sql: include_str!("../../../migrations/20260717000100_knowledge_status_scratch.sql"),
        },
        Migration {
            version: "20260717000200",
            name: "scratch_content_hash",
            sql: include_str!("../../../migrations/20260717000200_scratch_content_hash.sql"),
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

    #[test]
    fn bundled_migrations_include_source_branch_upgrade() {
        let migrations = bundled_migrations();

        assert!(
            migrations
                .iter()
                .any(|migration| migration.version == "20260704000300"
                    && migration
                        .sql
                        .contains("add column if not exists branch text")),
            "missing source branch migration"
        );
    }

    #[test]
    fn bundled_migrations_include_git_ingestion_lifecycle() {
        let migrations = bundled_migrations();
        let migration = migrations
            .iter()
            .find(|migration| migration.version == "20260704000400")
            .expect("missing git ingestion migration");

        for required_sql in [
            "source_root_id",
            "is_active",
            "stable_key",
            "generated_by",
            "cancel_requested_at",
            "retry_of_id",
            "idx_ingestion_job_one_active_per_source",
            "idx_source_document_active_child_path",
            "trusted_auto_approve",
        ] {
            assert!(
                migration.sql.contains(required_sql),
                "git ingestion migration is missing {required_sql}"
            );
        }
    }

    #[test]
    fn bundled_migrations_include_hybrid_retrieval_state() {
        let migrations = bundled_migrations();
        let migration = migrations
            .iter()
            .find(|migration| migration.version == "20260704000500")
            .expect("missing hybrid retrieval migration");

        for required_sql in [
            "create type embedding_status",
            "search_vector tsvector",
            "embedding_content_hash",
            "embedding_profile_version",
            "idx_chunk_search_vector",
            "idx_chunk_embedding_claim",
        ] {
            assert!(
                migration.sql.contains(required_sql),
                "hybrid retrieval migration is missing {required_sql}"
            );
        }
    }

    #[test]
    fn bundled_migrations_include_embedding_retry_backoff() {
        let migrations = bundled_migrations();
        let migration = migrations
            .iter()
            .find(|migration| migration.version == "20260704000600")
            .expect("missing embedding retry backoff migration");

        for required_sql in [
            "retry_after_at",
            "idx_ingestion_job_embedding_retry_ready",
            "status = 'queued'",
        ] {
            assert!(
                migration.sql.contains(required_sql),
                "embedding retry migration is missing {required_sql}"
            );
        }
    }

    #[test]
    fn bundled_migrations_include_evaluation_reports() {
        let migrations = bundled_migrations();
        let migration = migrations
            .iter()
            .find(|migration| migration.version == "20260704000700")
            .expect("missing evaluation report migration");

        for required_sql in [
            "create table if not exists evaluation_report",
            "report_json jsonb not null",
            "create index if not exists idx_evaluation_report_project_created",
        ] {
            assert!(
                migration.sql.contains(required_sql),
                "evaluation report migration is missing {required_sql}"
            );
        }
    }

    #[test]
    fn bundled_migrations_include_knowledge_status_scratch() {
        let migrations = bundled_migrations();
        let migration = migrations
            .iter()
            .find(|migration| migration.version == "20260717000100")
            .expect("missing knowledge_status scratch migration");

        assert!(
            migration
                .sql
                .contains("ALTER TYPE knowledge_status ADD VALUE")
                && migration.sql.contains("'scratch'"),
            "scratch migration must ADD VALUE 'scratch' to knowledge_status"
        );
        // YAGNI: lane is derived from status; no dedicated lane column.
        assert!(
            !migration.sql.to_lowercase().contains("add column"),
            "scratch migration must not add columns; extend enum only"
        );
    }

    /// IMP-22: partial unique index on (project_id, content_hash) for scratch only.
    #[test]
    fn bundled_migrations_include_scratch_content_hash() {
        let migrations = bundled_migrations();
        let migration = migrations
            .iter()
            .find(|migration| migration.version == "20260717000200")
            .expect("missing scratch content_hash migration");

        for required in [
            "content_hash",
            "idx_knowledge_item_scratch_content_hash",
            "status = 'scratch'",
            "project_id",
        ] {
            assert!(
                migration.sql.contains(required),
                "scratch content_hash migration missing {required}"
            );
        }
        // Must not force uniqueness against approved/trusted rows.
        assert!(
            migration.sql.contains("where status = 'scratch'"),
            "unique index must be partial on scratch status only"
        );
    }
}

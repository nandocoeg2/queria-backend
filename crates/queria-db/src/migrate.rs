#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Migration {
    pub version: &'static str,
    pub name: &'static str,
    pub sql: &'static str,
}

pub fn bundled_migrations() -> Vec<Migration> {
    vec![Migration {
        version: "20260704000100",
        name: "baseline",
        sql: include_str!("../../../migrations/20260704000100_baseline.sql"),
    }]
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

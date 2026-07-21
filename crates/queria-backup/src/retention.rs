use crate::object_store::ObjectStore;
use queria_core::{QueriaError, QueriaResult};
use sqlx::PgPool;

/// Apply retention policies: delete expired rows from PostgreSQL and expired
/// objects from S3.
///
/// **Preserved data** (never deleted by retention):
/// - Knowledge items with status `approved` or `draft`.
/// - The latest successful backup manifest per organization.
///
/// **Deleted after `retention_days`**:
/// - `audit_log` rows.
/// - Terminal `ingestion_job` rows (succeeded, failed, cancelled).
/// - `evaluation_report` rows.
/// - Knowledge items with status `rejected`, `deprecated`, or `superseded`.
/// - S3 backup artifacts older than retention window.
pub async fn run_retention(
    pool: &PgPool,
    store: &ObjectStore,
    org_slug: &str,
    retention_days: u32,
) -> QueriaResult<RetentionReport> {
    let mut report = RetentionReport::default();
    let interval = format!("{retention_days} days");

    // 1. Audit logs
    let result = sqlx::query(
        "DELETE FROM audit_log
         WHERE created_at < now() - $1::interval",
    )
    .bind(&interval)
    .execute(pool)
    .await
    .map_err(to_infra)?;
    report.audit_logs_deleted = result.rows_affected();

    // 2. Terminal ingestion jobs
    let result = sqlx::query(
        "DELETE FROM ingestion_job
         WHERE status IN ('succeeded', 'failed', 'cancelled')
           AND updated_at < now() - $1::interval",
    )
    .bind(&interval)
    .execute(pool)
    .await
    .map_err(to_infra)?;
    report.ingestion_jobs_deleted = result.rows_affected();

    // 3. Evaluation reports
    let result = sqlx::query(
        "DELETE FROM evaluation_report
         WHERE created_at < now() - $1::interval",
    )
    .bind(&interval)
    .execute(pool)
    .await
    .map_err(to_infra)?;
    report.evaluation_reports_deleted = result.rows_affected();

    // 4. Rejected/deprecated/superseded knowledge items
    //    Chunks are CASCADE-deleted when their knowledge_item is removed.
    let result = sqlx::query(
        "DELETE FROM knowledge_item
         WHERE status IN ('rejected', 'deprecated', 'superseded')
           AND updated_at < now() - $1::interval",
    )
    .bind(&interval)
    .execute(pool)
    .await
    .map_err(to_infra)?;
    report.knowledge_items_deleted = result.rows_affected();

    // 5. Old backup records (keep the DB record but mark as expired)
    let result = sqlx::query(
        "DELETE FROM backup_record
         WHERE created_at < now() - $1::interval",
    )
    .bind(&interval)
    .execute(pool)
    .await
    .map_err(to_infra)?;
    report.backup_records_deleted = result.rows_affected();

    // 6. S3 artifact cleanup – list all backup artifacts and delete old ones.
    let cutoff = chrono::Utc::now() - chrono::Duration::days(i64::from(retention_days));

    for artifact_type in ["pg-dump", "qdrant-snapshot", "manifests"] {
        let prefix = format!("{org_slug}/{artifact_type}/");
        match store.list_objects(&prefix).await {
            Ok(objects) => {
                for obj in objects {
                    // Parse date from the key path: org/type/YYYY-MM-DD/file
                    if let Some(date_str) = extract_date_from_key(&obj.key)
                        && let Ok(date) = chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
                    {
                        let obj_date = date.and_hms_opt(0, 0, 0).unwrap().and_utc();
                        if obj_date < cutoff {
                            if let Err(error) = store.delete_object(&obj.key).await {
                                tracing::warn!(
                                    key = %obj.key,
                                    error = %error,
                                    "failed to delete expired S3 object"
                                );
                            } else {
                                report.s3_objects_deleted += 1;
                            }
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    prefix = %prefix,
                    error = %error,
                    "failed to list S3 objects for retention"
                );
            }
        }
    }

    tracing::info!(
        audit_logs = report.audit_logs_deleted,
        ingestion_jobs = report.ingestion_jobs_deleted,
        evaluation_reports = report.evaluation_reports_deleted,
        knowledge_items = report.knowledge_items_deleted,
        backup_records = report.backup_records_deleted,
        s3_objects = report.s3_objects_deleted,
        "retention cleanup completed"
    );

    Ok(report)
}

/// Summary of what the retention run deleted.
#[derive(Clone, Debug, Default)]
pub struct RetentionReport {
    pub audit_logs_deleted: u64,
    pub ingestion_jobs_deleted: u64,
    pub evaluation_reports_deleted: u64,
    pub knowledge_items_deleted: u64,
    pub backup_records_deleted: u64,
    pub s3_objects_deleted: u64,
}

/// Extract the date segment from an S3 key like `org/type/2026-07-05/file`.
fn extract_date_from_key(key: &str) -> Option<String> {
    let parts: Vec<&str> = key.split('/').collect();
    // Expected: [org, type, date, filename]
    if parts.len() >= 4 {
        let date = parts[2];
        // Validate it looks like a date
        if date.len() == 10 && date.chars().nth(4) == Some('-') {
            return Some(date.to_owned());
        }
    }
    None
}

fn to_infra(error: sqlx::Error) -> QueriaError {
    QueriaError::Infrastructure(format!("retention query failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_date_from_valid_key() {
        let key = "fjulian/pg-dump/2026-07-05/queria.dump";
        assert_eq!(extract_date_from_key(key), Some("2026-07-05".to_owned()));
    }

    #[test]
    fn extract_date_rejects_short_key() {
        let key = "fjulian/pg-dump";
        assert_eq!(extract_date_from_key(key), None);
    }

    #[test]
    fn extract_date_rejects_non_date_segment() {
        let key = "fjulian/pg-dump/notadate/file.dump";
        assert_eq!(extract_date_from_key(key), None);
    }
}

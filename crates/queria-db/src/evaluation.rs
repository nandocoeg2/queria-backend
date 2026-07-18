use chrono::{DateTime, Utc};
use queria_core::evaluation::EvaluationReport;
use queria_core::{QueriaError, QueriaResult};
use serde::Serialize;
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EvaluationReportRecord {
    pub id: Uuid,
    pub organization_id: Uuid,
    pub project_id: Uuid,
    pub project_slug: String,
    pub golden_question_file: String,
    pub status: String,
    pub total_questions: i32,
    pub passed_questions: i32,
    pub failed_questions: i32,
    pub regression_score: f32,
    pub report_json: Value,
    pub created_by: Uuid,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct PgEvaluationRepository {
    pool: PgPool,
}

impl PgEvaluationRepository {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn insert_for_project_slug(
        &self,
        user_id: Uuid,
        project_slug: &str,
        report: &EvaluationReport,
    ) -> QueriaResult<Option<EvaluationReportRecord>> {
        let total_questions = checked_i32(report.total_questions, "total_questions")?;
        let passed_questions = checked_i32(report.passed_questions, "passed_questions")?;
        let failed_questions = checked_i32(report.failed_questions, "failed_questions")?;
        if !report.regression_score.is_finite()
            || !(0.0_f32..=1.0_f32).contains(&report.regression_score)
        {
            return Err(QueriaError::Validation(
                "regression_score must be between 0 and 1".to_owned(),
            ));
        }
        let report_json = serde_json::to_value(report).map_err(|error| {
            QueriaError::Infrastructure(format!("failed to serialize evaluation report: {error}"))
        })?;

        sqlx::query(
            "with accessible_project as (
               select p.organization_id, p.id
               from project p
               join org_membership m on m.organization_id = p.organization_id
               where m.user_id = $1 and p.slug = $2
             )
             insert into evaluation_report(
               organization_id, project_id, project_slug, golden_question_file, status,
               total_questions, passed_questions, failed_questions, regression_score,
               report_json, created_by
             )
             select organization_id, id, $2, $3, $4,
                    $5, $6, $7, $8, $9, $1
             from accessible_project
             returning id, organization_id, project_id, project_slug, golden_question_file,
                       status, total_questions, passed_questions, failed_questions,
                       regression_score, report_json, created_by, created_at",
        )
        .bind(user_id)
        .bind(project_slug)
        .bind(&report.golden_question_file)
        .bind(report_status(report))
        .bind(total_questions)
        .bind(passed_questions)
        .bind(failed_questions)
        .bind(report.regression_score)
        .bind(report_json)
        .fetch_optional(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .map(record_from_row)
        .transpose()
    }

    pub async fn list_for_project_slug(
        &self,
        user_id: Uuid,
        project_slug: &str,
        limit: i64,
    ) -> QueriaResult<Vec<EvaluationReportRecord>> {
        sqlx::query(
            "select report.id, report.organization_id, report.project_id,
                    report.project_slug, report.golden_question_file, report.status,
                    report.total_questions, report.passed_questions,
                    report.failed_questions, report.regression_score,
                    report.report_json, report.created_by, report.created_at
             from evaluation_report report
             join org_membership m on m.organization_id = report.organization_id
             join project p on p.id = report.project_id
             where m.user_id = $1 and p.slug = $2
             order by report.created_at desc, report.id desc
             limit $3",
        )
        .bind(user_id)
        .bind(project_slug)
        .bind(limit.clamp(1, 100))
        .fetch_all(&self.pool)
        .await
        .map_err(to_infrastructure_error)?
        .into_iter()
        .map(record_from_row)
        .collect()
    }

    pub async fn latest_for_project_slug(
        &self,
        user_id: Uuid,
        project_slug: &str,
    ) -> QueriaResult<Option<EvaluationReportRecord>> {
        Ok(self
            .list_for_project_slug(user_id, project_slug, 1)
            .await?
            .into_iter()
            .next())
    }
}

fn report_status(report: &EvaluationReport) -> &'static str {
    if report.passed { "passed" } else { "failed" }
}

fn checked_i32(value: usize, field: &str) -> QueriaResult<i32> {
    i32::try_from(value)
        .map_err(|_| QueriaError::Validation(format!("{field} exceeds database range")))
}

fn record_from_row(row: sqlx::postgres::PgRow) -> QueriaResult<EvaluationReportRecord> {
    Ok(EvaluationReportRecord {
        id: row.try_get("id").map_err(to_infrastructure_error)?,
        organization_id: row
            .try_get("organization_id")
            .map_err(to_infrastructure_error)?,
        project_id: row.try_get("project_id").map_err(to_infrastructure_error)?,
        project_slug: row
            .try_get("project_slug")
            .map_err(to_infrastructure_error)?,
        golden_question_file: row
            .try_get("golden_question_file")
            .map_err(to_infrastructure_error)?,
        status: row.try_get("status").map_err(to_infrastructure_error)?,
        total_questions: row
            .try_get("total_questions")
            .map_err(to_infrastructure_error)?,
        passed_questions: row
            .try_get("passed_questions")
            .map_err(to_infrastructure_error)?,
        failed_questions: row
            .try_get("failed_questions")
            .map_err(to_infrastructure_error)?,
        regression_score: row
            .try_get("regression_score")
            .map_err(to_infrastructure_error)?,
        report_json: row
            .try_get("report_json")
            .map_err(to_infrastructure_error)?,
        created_by: row.try_get("created_by").map_err(to_infrastructure_error)?,
        created_at: row.try_get("created_at").map_err(to_infrastructure_error)?,
    })
}

fn to_infrastructure_error(error: sqlx::Error) -> QueriaError {
    QueriaError::Infrastructure(format!("evaluation repository failed: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_status_tracks_passed_flag() {
        let mut report = EvaluationReport {
            project: "fjulian-me".to_owned(),
            golden_question_file: "tests/golden_questions/fjulian-me.jsonl".to_owned(),
            total_questions: 1,
            passed_questions: 1,
            failed_questions: 0,
            passed: true,
            regression_score: 1.0,
            results: Vec::new(),
        };

        assert_eq!(report_status(&report), "passed");

        report.passed = false;
        assert_eq!(report_status(&report), "failed");
    }
}

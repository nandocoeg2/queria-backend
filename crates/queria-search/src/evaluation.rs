use async_trait::async_trait;
use queria_core::contracts::{RetrievalMode, RetrieveContextRequest, RetrieveContextResponse};
use queria_core::evaluation::{
    EvaluationReport, evaluation_limit, parse_golden_questions_jsonl, score_evaluation_report,
};
use queria_core::ids::ProjectId;
use queria_core::{QueriaError, QueriaResult};
use std::path::Path;
use std::time::Duration;
use uuid::Uuid;

#[mockall::automock]
#[async_trait]
pub trait EvaluationRetriever: Send + Sync {
    async fn retrieve(
        &self,
        user_id: Uuid,
        request: RetrieveContextRequest,
    ) -> QueriaResult<RetrieveContextResponse>;
}

pub struct EvaluationExecutor<R> {
    retrieval: R,
    retry_attempts: usize,
    retry_delay: Duration,
}

impl<R: EvaluationRetriever> EvaluationExecutor<R> {
    #[must_use]
    pub fn new(retrieval: R, retry_attempts: usize, retry_delay: Duration) -> Self {
        Self {
            retrieval,
            retry_attempts,
            retry_delay,
        }
    }

    pub async fn run(
        &self,
        user_id: Uuid,
        project_slug: &str,
        project_id: ProjectId,
        dataset_path: &Path,
    ) -> QueriaResult<EvaluationReport> {
        let content = std::fs::read_to_string(dataset_path).map_err(|error| {
            QueriaError::Validation(format!(
                "failed to read golden questions file at {}: {}",
                dataset_path.display(),
                error
            ))
        })?;
        let questions = parse_golden_questions_jsonl(&content)?
            .into_iter()
            .filter(|question| question.project_slug == project_slug)
            .collect::<Vec<_>>();
        if questions.is_empty() {
            return Err(QueriaError::Validation(format!(
                "no golden questions found for project {project_slug}"
            )));
        }

        let mut responses = Vec::with_capacity(questions.len());
        for question in &questions {
            let request = RetrieveContextRequest {
                project_id,
                query: question.query.clone(),
                include_global: question.include_global,
                limit: evaluation_limit(question.minimum_items),
            };
            let response = self.retrieve_with_retry(user_id, request).await;
            responses.push(response);
        }

        Ok(score_evaluation_report(
            project_slug,
            &dataset_path.display().to_string(),
            &questions,
            responses,
        ))
    }

    async fn retrieve_with_retry(
        &self,
        user_id: Uuid,
        request: RetrieveContextRequest,
    ) -> Result<RetrieveContextResponse, String> {
        for attempt in 0..self.retry_attempts {
            let response = self
                .retrieval
                .retrieve(user_id, request.clone())
                .await
                .map_err(|error| error.to_string())?;

            if should_retry(&response) {
                if attempt + 1 < self.retry_attempts {
                    tracing::warn!(
                        attempt = attempt + 1,
                        project_id = %request.project_id,
                        "empty semantic fallback during evaluation; retrying"
                    );
                    tokio::time::sleep(self.retry_delay).await;
                    continue;
                } else {
                    return Err("evaluation retry attempts exhausted".to_owned());
                }
            }
            return Ok(response);
        }
        Err("evaluation retry attempts exhausted".to_owned())
    }
}

fn should_retry(response: &RetrieveContextResponse) -> bool {
    response.retrieval.mode == RetrievalMode::LexicalFallback && response.items.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use queria_core::contracts::RetrievalDiagnostics;

    fn test_response(mode: RetrievalMode, has_items: bool) -> RetrieveContextResponse {
        RetrieveContextResponse {
            project_id: ProjectId::new(),
            query: "query".to_owned(),
            items: if has_items {
                vec![queria_core::contracts::RetrievedContextItem {
                    chunk_id: queria_core::ids::ChunkId::new(),
                    source_document_id: queria_core::ids::SourceDocumentId::new(),
                    scope: queria_core::model::KnowledgeScope::Project,
                    title: "title".to_owned(),
                    body: "body".to_owned(),
                    citation: queria_core::contracts::Citation {
                        source_uri: "uri".to_owned(),
                        source_path: None,
                        line_start: None,
                        line_end: None,
                    },
                    score: 1.0,
                }]
            } else {
                vec![]
            },
            retrieval: RetrievalDiagnostics {
                mode,
                lexical_candidates: 0,
                semantic_candidates: 0,
                embedding_profile_version: "test".to_owned(),
            },
            generated_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_retrieve_immediate_success() {
        let mut mock = MockEvaluationRetriever::new();
        mock.expect_retrieve()
            .once()
            .returning(|_, _| Ok(test_response(RetrievalMode::Hybrid, true)));

        let executor = EvaluationExecutor::new(mock, 3, Duration::from_millis(1));
        let res = executor
            .retrieve_with_retry(
                Uuid::nil(),
                RetrieveContextRequest {
                    project_id: ProjectId::new(),
                    query: "test".to_owned(),
                    include_global: false,
                    limit: 5,
                },
            )
            .await;

        assert!(res.is_ok());
        let resp = res.unwrap();
        assert_eq!(resp.retrieval.mode, RetrievalMode::Hybrid);
    }

    #[tokio::test]
    async fn test_retrieve_retry_then_success() {
        let mut mock = MockEvaluationRetriever::new();
        // First returns empty lexical fallback (triggers retry)
        // Second returns hybrid success
        let call_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        mock.expect_retrieve().times(2).returning(move |_, _| {
            let count = call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if count == 0 {
                Ok(test_response(RetrievalMode::LexicalFallback, false))
            } else {
                Ok(test_response(RetrievalMode::Hybrid, true))
            }
        });

        let executor = EvaluationExecutor::new(mock, 3, Duration::from_millis(1));
        let res = executor
            .retrieve_with_retry(
                Uuid::nil(),
                RetrieveContextRequest {
                    project_id: ProjectId::new(),
                    query: "test".to_owned(),
                    include_global: false,
                    limit: 5,
                },
            )
            .await;

        assert!(res.is_ok());
        let resp = res.unwrap();
        assert_eq!(resp.retrieval.mode, RetrievalMode::Hybrid);
    }

    #[tokio::test]
    async fn test_retrieve_exhausted_retries() {
        let mut mock = MockEvaluationRetriever::new();
        mock.expect_retrieve()
            .times(3)
            .returning(|_, _| Ok(test_response(RetrievalMode::LexicalFallback, false)));

        let executor = EvaluationExecutor::new(mock, 3, Duration::from_millis(1));
        let res = executor
            .retrieve_with_retry(
                Uuid::nil(),
                RetrieveContextRequest {
                    project_id: ProjectId::new(),
                    query: "test".to_owned(),
                    include_global: false,
                    limit: 5,
                },
            )
            .await;

        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), "evaluation retry attempts exhausted");
    }
}

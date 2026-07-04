use chrono::Utc;
use queria_core::QueriaResult;
use queria_core::contracts::{RetrieveContextRequest, RetrieveContextResponse};
use queria_db::repositories::KnowledgeRepository;

pub struct RetrievalService<R> {
    repository: R,
}

impl<R> RetrievalService<R>
where
    R: KnowledgeRepository,
{
    #[must_use]
    pub fn new(repository: R) -> Self {
        Self { repository }
    }

    pub fn retrieve_context(
        &self,
        request: RetrieveContextRequest,
    ) -> QueriaResult<RetrieveContextResponse> {
        request.validate()?;
        let items = self.repository.search_approved_chunks(
            request.project_id,
            &request.query,
            request.limit,
        )?;

        Ok(RetrieveContextResponse {
            project_id: request.project_id,
            query: request.query,
            items,
            generated_at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use queria_core::contracts::{Citation, RetrievedContextItem};
    use queria_core::ids::{ChunkId, ProjectId, SourceDocumentId};
    use queria_core::model::KnowledgeScope;
    use queria_db::repositories::MockKnowledgeRepository;

    #[test]
    fn retrieve_context_uses_mocked_repository_and_preserves_citations() {
        let project_id = ProjectId::new();
        let mut repository = MockKnowledgeRepository::new();
        repository
            .expect_search_approved_chunks()
            .withf(move |seen_project_id, query, limit| {
                *seen_project_id == project_id && query == "deploy flow" && *limit == 3
            })
            .returning(|_, _, _| {
                Ok(vec![RetrievedContextItem {
                    chunk_id: ChunkId::new(),
                    source_document_id: SourceDocumentId::new(),
                    scope: KnowledgeScope::Project,
                    title: "Deploy SOP".to_owned(),
                    body: "Run deploy through the approved workflow.".to_owned(),
                    citation: Citation {
                        source_uri: "git://fjulian-me/docs/deploy.md".to_owned(),
                        source_path: Some("docs/deploy.md".to_owned()),
                        line_start: Some(10),
                        line_end: Some(18),
                    },
                    score: 0.91,
                }])
            });

        let service = RetrievalService::new(repository);
        let response = service
            .retrieve_context(RetrieveContextRequest {
                project_id,
                query: "deploy flow".to_owned(),
                include_global: true,
                limit: 3,
            })
            .expect("retrieval should succeed");

        assert_eq!(response.project_id, project_id);
        assert_eq!(response.items.len(), 1);
        assert_eq!(
            response.items[0].citation.source_path.as_deref(),
            Some("docs/deploy.md")
        );
    }
}

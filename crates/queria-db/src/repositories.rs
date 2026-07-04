use mockall::automock;
use queria_core::QueriaResult;
use queria_core::contracts::RetrievedContextItem;
use queria_core::ids::{ProjectId, SourceDocumentId};

#[automock]
pub trait KnowledgeRepository: Send + Sync {
    fn search_approved_chunks(
        &self,
        project_id: ProjectId,
        query: &str,
        limit: u32,
    ) -> QueriaResult<Vec<RetrievedContextItem>>;
}

#[automock]
pub trait SourceRepository: Send + Sync {
    fn get_source_document(&self, source_document_id: SourceDocumentId) -> QueriaResult<String>;
}

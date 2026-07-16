//! Database repository implementations and shared record types.

mod auth;
mod projects;
mod types;

pub use auth::PgAuthRepository;
pub use projects::PgProjectRepository;
pub use types::{
    AgentTokenRecord, ApprovalRecord, ApprovedKnowledgeRecord, AuthUser, AuthenticatedAgentToken,
    AuthenticatedSession, CompleteSetupParams, CreateAgentTokenParams, CreateProjectParams,
    CreatedAdmin, KnowledgeItemRecord, ProjectRecord, ProposeMemoryParams, ProposedMemoryRecord,
    RegisterSourceDocumentParams, SourceDocumentRecord,
};

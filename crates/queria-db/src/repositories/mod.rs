//! Database repository implementations and shared record types.

mod auth;
mod orgs;
mod projects;
mod types;

pub use auth::{PgAuthRepository, resolve_active_organization_id};
pub use orgs::{
    AcceptOrgInviteParams, AcceptedOrgInvite, CreateOrgInviteParams, CreateOrganizationParams,
    OrgInviteForAccept, OrgInviteRecord, OrgMemberRecord, OrganizationRecord, PgOrgRepository,
};
pub use projects::PgProjectRepository;
pub use types::{
    AgentTokenRecord, ApprovalRecord, ApprovedKnowledgeRecord, AuthUser, AuthenticatedAgentToken,
    AuthenticatedSession, CompleteSetupParams, CreateAgentTokenParams, CreateProjectParams,
    CreatedAdmin, IndexMemoryParams, IndexMemoryResult, IndexedMemoryRecord, KnowledgeItemRecord,
    MarkScratchChunkReadyParams, ProjectRecord, ProposeMemoryParams, ProposedMemoryRecord,
    RegisterSourceDocumentParams, SourceDocumentRecord,
};

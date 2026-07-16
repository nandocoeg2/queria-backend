use crate::embedding::{EmbeddingProvider, VectorIndex, VectorSearchRequest};
use crate::hybrid::{RankedChunk, reciprocal_rank_fusion};
use crate::qdrant::{QdrantClient, QdrantConfig};
use crate::voyage::VoyageClient;
use async_trait::async_trait;
use chrono::Utc;
use queria_core::contracts::{
    RetrievalDiagnostics, RetrievalMode, RetrieveContextRequest, RetrieveContextResponse,
    RetrievedContextItem,
};
use queria_core::ids::{ChunkId, ProjectId};
use queria_core::{AppConfig, QueriaError, QueriaResult};
use queria_db::hybrid::{DbRankedChunk, PgHybridRetrievalRepository, RetrievalAccess};
use sqlx::PgPool;
use std::collections::HashMap;
use std::time::Duration;
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RetrievalPrincipal {
    User {
        user_id: Uuid,
    },
    Agent {
        organization_id: Uuid,
        project_slugs: Vec<String>,
        allow_global_knowledge: bool,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RetrievalConfig {
    pub embedding_profile_version: String,
    pub rrf_k: u32,
    pub candidate_multiplier: u32,
    pub candidate_cap: u32,
}

#[async_trait]
pub trait HybridRetrievalStore: Send + Sync {
    async fn authorize(
        &self,
        principal: &RetrievalPrincipal,
        project_id: ProjectId,
        include_global: bool,
    ) -> QueriaResult<RetrievalAccess>;
    async fn lexical_search(
        &self,
        access: &RetrievalAccess,
        query: &str,
        limit: i64,
    ) -> QueriaResult<Vec<DbRankedChunk>>;
    async fn hydrate(
        &self,
        access: &RetrievalAccess,
        chunk_ids: &[ChunkId],
    ) -> QueriaResult<Vec<RetrievedContextItem>>;
}

pub struct RetrievalService<S, E, V> {
    store: S,
    embedding_provider: E,
    vector_index: V,
    config: RetrievalConfig,
}

pub type PgRetrievalService =
    RetrievalService<PgHybridRetrievalRepository, VoyageClient, QdrantClient>;

pub fn build_pg_retrieval_service(
    config: &AppConfig,
    pool: PgPool,
) -> QueriaResult<PgRetrievalService> {
    let dimension = usize::try_from(config.embedding.dimension)
        .map_err(|_| QueriaError::Config("embedding dimension is invalid".to_owned()))?;
    Ok(RetrievalService::new(
        PgHybridRetrievalRepository::new(pool),
        VoyageClient::new(
            config.embedding.voyage_api_key.clone(),
            config.embedding.model.clone(),
            dimension,
            Duration::from_secs(config.embedding.timeout_seconds),
            config.embedding.max_retries,
        )?,
        QdrantClient::new(QdrantConfig {
            url: config.qdrant.url.clone(),
            api_key: config.qdrant.api_key.clone(),
            collection: config.qdrant.collection.clone(),
            vector_name: config.qdrant.vector_name.clone(),
            dimension,
        })?,
        RetrievalConfig {
            embedding_profile_version: config.embedding.profile_version.clone(),
            rrf_k: config.retrieval.rrf_k,
            candidate_multiplier: config.retrieval.candidate_multiplier,
            candidate_cap: config.retrieval.candidate_cap,
        },
    ))
}

impl<S, E, V> RetrievalService<S, E, V>
where
    S: HybridRetrievalStore,
    E: EmbeddingProvider,
    V: VectorIndex,
{
    #[must_use]
    pub fn new(store: S, embedding_provider: E, vector_index: V, config: RetrievalConfig) -> Self {
        Self {
            store,
            embedding_provider,
            vector_index,
            config,
        }
    }

    pub async fn retrieve_context(
        &self,
        principal: &RetrievalPrincipal,
        request: RetrieveContextRequest,
    ) -> QueriaResult<RetrieveContextResponse> {
        request.validate()?;
        let access = self
            .store
            .authorize(principal, request.project_id, request.include_global)
            .await?;
        let candidate_count = request
            .limit
            .saturating_mul(self.config.candidate_multiplier)
            .min(self.config.candidate_cap);
        let (lexical_result, embedding_result) = tokio::join!(
            self.store
                .lexical_search(&access, &request.query, i64::from(candidate_count)),
            self.embedding_provider.embed_query(&request.query)
        );
        let lexical = lexical_result?;
        let semantic_result = match embedding_result {
            Ok(vector) => {
                self.vector_index
                    .search(VectorSearchRequest {
                        organization_id: access.organization_id,
                        project_id: access.project_id.as_uuid(),
                        include_global: access.include_global,
                        embedding_profile_version: self.config.embedding_profile_version.clone(),
                        vector,
                        limit: candidate_count,
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        let (semantic, mode) = match semantic_result {
            Ok(candidates) => (candidates, RetrievalMode::Hybrid),
            Err(error) => {
                tracing::warn!(
                    error = %sanitized_provider_error(&error),
                    project_id = %request.project_id,
                    "semantic retrieval unavailable; using PostgreSQL FTS"
                );
                (Vec::new(), RetrievalMode::LexicalFallback)
            }
        };
        let lexical_ranked = lexical
            .iter()
            .map(|candidate| RankedChunk::new(candidate.chunk_id, candidate.score))
            .collect::<Vec<_>>();
        let semantic_ranked = semantic
            .iter()
            .map(|candidate| RankedChunk::new(candidate.chunk_id, candidate.score))
            .collect::<Vec<_>>();
        let fused = reciprocal_rank_fusion(
            &lexical_ranked,
            &semantic_ranked,
            self.config.rrf_k,
            usize::try_from(request.limit)
                .map_err(|_| QueriaError::Validation("retrieval limit is invalid".to_owned()))?,
        );
        let fused_ids = fused
            .iter()
            .map(|candidate| candidate.chunk_id)
            .collect::<Vec<_>>();
        let score_by_id = fused
            .iter()
            .map(|candidate| (candidate.chunk_id, candidate.score))
            .collect::<HashMap<_, _>>();
        let mut items = self.store.hydrate(&access, &fused_ids).await?;
        for item in &mut items {
            item.score = score_by_id.get(&item.chunk_id).copied().unwrap_or_default();
        }

        Ok(RetrieveContextResponse {
            project_id: request.project_id,
            query: request.query,
            items,
            retrieval: RetrievalDiagnostics {
                mode,
                lexical_candidates: bounded_count(lexical.len()),
                semantic_candidates: bounded_count(semantic.len()),
                embedding_profile_version: self.config.embedding_profile_version.clone(),
            },
            generated_at: Utc::now(),
        })
    }
}

#[async_trait]
impl HybridRetrievalStore for PgHybridRetrievalRepository {
    async fn authorize(
        &self,
        principal: &RetrievalPrincipal,
        project_id: ProjectId,
        include_global: bool,
    ) -> QueriaResult<RetrievalAccess> {
        match principal {
            RetrievalPrincipal::User { user_id } => {
                self.authorize_user(*user_id, project_id, include_global)
                    .await
            }
            RetrievalPrincipal::Agent {
                organization_id,
                project_slugs,
                allow_global_knowledge,
            } => {
                self.authorize_agent(
                    *organization_id,
                    project_slugs,
                    *allow_global_knowledge,
                    project_id,
                    include_global,
                )
                .await
            }
        }
    }

    async fn lexical_search(
        &self,
        access: &RetrievalAccess,
        query: &str,
        limit: i64,
    ) -> QueriaResult<Vec<DbRankedChunk>> {
        PgHybridRetrievalRepository::lexical_search(self, access, query, limit).await
    }

    async fn hydrate(
        &self,
        access: &RetrievalAccess,
        chunk_ids: &[ChunkId],
    ) -> QueriaResult<Vec<RetrievedContextItem>> {
        PgHybridRetrievalRepository::hydrate(self, access, chunk_ids).await
    }
}

#[async_trait]
impl<S, E, V> crate::evaluation::EvaluationRetriever for RetrievalService<S, E, V>
where
    S: HybridRetrievalStore + Send + Sync,
    E: EmbeddingProvider + Send + Sync,
    V: VectorIndex + Send + Sync,
{
    async fn retrieve(
        &self,
        user_id: Uuid,
        request: RetrieveContextRequest,
    ) -> QueriaResult<RetrieveContextResponse> {
        self.retrieve_context(&RetrievalPrincipal::User { user_id }, request)
            .await
    }
}

fn bounded_count(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

fn sanitized_provider_error(error: &QueriaError) -> String {
    match error {
        QueriaError::Infrastructure(_) => "provider_unavailable".to_owned(),
        _ => error.to_string().chars().take(120).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::{EmbeddingDocument, EmbeddingVector, VectorCandidate, VectorPoint};
    use queria_core::contracts::Citation;
    use queria_core::ids::SourceDocumentId;
    use queria_core::model::KnowledgeScope;
    use std::sync::Mutex;

    struct FakeHybridStore {
        access: RetrievalAccess,
        authorize_checks: Mutex<Vec<(RetrievalPrincipal, ProjectId, bool)>>,
        lexical: Mutex<Option<Vec<DbRankedChunk>>>,
        hydrate_items: Mutex<Option<Vec<RetrievedContextItem>>>,
    }

    impl FakeHybridStore {
        fn new(
            access: RetrievalAccess,
            lexical: Vec<DbRankedChunk>,
            hydrate_items: Vec<RetrievedContextItem>,
        ) -> Self {
            Self {
                access,
                authorize_checks: Mutex::new(Vec::new()),
                lexical: Mutex::new(Some(lexical)),
                hydrate_items: Mutex::new(Some(hydrate_items)),
            }
        }
    }

    #[async_trait]
    impl HybridRetrievalStore for FakeHybridStore {
        async fn authorize(
            &self,
            principal: &RetrievalPrincipal,
            project_id: ProjectId,
            include_global: bool,
        ) -> QueriaResult<RetrievalAccess> {
            self.authorize_checks.lock().expect("lock").push((
                principal.clone(),
                project_id,
                include_global,
            ));
            Ok(self.access.clone())
        }

        async fn lexical_search(
            &self,
            _access: &RetrievalAccess,
            _query: &str,
            _limit: i64,
        ) -> QueriaResult<Vec<DbRankedChunk>> {
            Ok(self
                .lexical
                .lock()
                .expect("lock")
                .take()
                .unwrap_or_default())
        }

        async fn hydrate(
            &self,
            _access: &RetrievalAccess,
            _chunk_ids: &[ChunkId],
        ) -> QueriaResult<Vec<RetrievedContextItem>> {
            Ok(self
                .hydrate_items
                .lock()
                .expect("lock")
                .take()
                .unwrap_or_default())
        }
    }

    struct FailProvider;
    #[async_trait]
    impl EmbeddingProvider for FailProvider {
        async fn embed_documents(
            &self,
            _inputs: &[EmbeddingDocument],
        ) -> QueriaResult<Vec<EmbeddingVector>> {
            Err(QueriaError::Infrastructure(
                "provider unavailable".to_owned(),
            ))
        }

        async fn embed_query(&self, _query: &str) -> QueriaResult<EmbeddingVector> {
            Err(QueriaError::Infrastructure(
                "provider unavailable".to_owned(),
            ))
        }
    }

    struct OkProvider;
    #[async_trait]
    impl EmbeddingProvider for OkProvider {
        async fn embed_documents(
            &self,
            inputs: &[EmbeddingDocument],
        ) -> QueriaResult<Vec<EmbeddingVector>> {
            inputs
                .iter()
                .map(|_| EmbeddingVector::new(vec![0.1, 0.2], 2))
                .collect()
        }

        async fn embed_query(&self, _query: &str) -> QueriaResult<EmbeddingVector> {
            EmbeddingVector::new(vec![0.1, 0.2], 2)
        }
    }

    struct FakeIndex {
        candidates: Mutex<Option<Vec<VectorCandidate>>>,
        last_search: Mutex<Option<VectorSearchRequest>>,
    }

    impl FakeIndex {
        fn new(candidates: Vec<VectorCandidate>) -> Self {
            Self {
                candidates: Mutex::new(Some(candidates)),
                last_search: Mutex::new(None),
            }
        }

        fn empty() -> Self {
            Self::new(Vec::new())
        }
    }

    #[async_trait]
    impl VectorIndex for FakeIndex {
        async fn ensure_collection(&self) -> QueriaResult<()> {
            Ok(())
        }

        async fn upsert(&self, _points: &[VectorPoint]) -> QueriaResult<()> {
            Ok(())
        }

        async fn search(
            &self,
            request: VectorSearchRequest,
        ) -> QueriaResult<Vec<VectorCandidate>> {
            *self.last_search.lock().expect("lock") = Some(request);
            Ok(self
                .candidates
                .lock()
                .expect("lock")
                .take()
                .unwrap_or_default())
        }

        async fn delete(&self, _point_ids: &[Uuid]) -> QueriaResult<()> {
            Ok(())
        }

        async fn health(&self) -> QueriaResult<crate::embedding::VectorIndexHealth> {
            Ok(crate::embedding::VectorIndexHealth {
                collection: "test".to_owned(),
                points_count: 0,
            })
        }
    }

    #[tokio::test]
    async fn provider_failure_falls_back_to_lexical_candidates() {
        let project_id = ProjectId::new();
        let organization_id = Uuid::now_v7();
        let chunk_id = ChunkId::new();
        let source_document_id = SourceDocumentId::new();
        let store = FakeHybridStore::new(
            RetrievalAccess {
                organization_id,
                project_id,
                include_global: true,
            },
            vec![DbRankedChunk {
                chunk_id,
                score: 0.8,
            }],
            vec![RetrievedContextItem {
                chunk_id,
                source_document_id,
                scope: KnowledgeScope::Project,
                title: "Deploy SOP".to_owned(),
                body: "Deploy through CI.".to_owned(),
                citation: Citation {
                    source_uri: "git://repo/docs/deploy.md".to_owned(),
                    source_path: Some("docs/deploy.md".to_owned()),
                    line_start: Some(1),
                    line_end: Some(5),
                },
                score: 0.0,
            }],
        );
        let service = RetrievalService::new(store, FailProvider, FakeIndex::empty(), config());

        let response = service
            .retrieve_context(
                &RetrievalPrincipal::User {
                    user_id: Uuid::now_v7(),
                },
                RetrieveContextRequest {
                    project_id,
                    query: "deploy flow".to_owned(),
                    include_global: true,
                    limit: 5,
                },
            )
            .await
            .expect("lexical fallback should succeed");

        assert_eq!(response.retrieval.mode, RetrievalMode::LexicalFallback);
        assert_eq!(response.items.len(), 1);
        assert_eq!(response.items[0].score, 1.0);
    }

    #[tokio::test]
    async fn hybrid_mode_fuses_semantic_candidates() {
        let project_id = ProjectId::new();
        let organization_id = Uuid::now_v7();
        let shared = ChunkId::new();
        let source_document_id = SourceDocumentId::new();
        let store = FakeHybridStore::new(
            RetrievalAccess {
                organization_id,
                project_id,
                include_global: false,
            },
            vec![DbRankedChunk {
                chunk_id: shared,
                score: 0.8,
            }],
            vec![RetrievedContextItem {
                chunk_id: shared,
                source_document_id,
                scope: KnowledgeScope::Project,
                title: "Architecture".to_owned(),
                body: "System flow.".to_owned(),
                citation: Citation {
                    source_uri: "git://repo/README.md".to_owned(),
                    source_path: Some("README.md".to_owned()),
                    line_start: Some(1),
                    line_end: Some(3),
                },
                score: 0.0,
            }],
        );
        let index = FakeIndex::new(vec![VectorCandidate {
            chunk_id: shared,
            score: 0.9,
        }]);
        let service = RetrievalService::new(store, OkProvider, index, config());

        let response = service
            .retrieve_context(
                &RetrievalPrincipal::User {
                    user_id: Uuid::now_v7(),
                },
                RetrieveContextRequest {
                    project_id,
                    query: "architecture".to_owned(),
                    include_global: false,
                    limit: 5,
                },
            )
            .await
            .expect("hybrid retrieval should succeed");

        assert_eq!(response.retrieval.mode, RetrievalMode::Hybrid);
        assert_eq!(response.retrieval.semantic_candidates, 1);
        assert_eq!(response.items[0].score, 1.0);
    }

    #[tokio::test]
    async fn agent_without_global_permission_searches_project_scope_only() {
        let project_id = ProjectId::new();
        let organization_id = Uuid::now_v7();
        let chunk_id = ChunkId::new();
        let source_document_id = SourceDocumentId::new();
        let store = FakeHybridStore::new(
            RetrievalAccess {
                organization_id,
                project_id,
                include_global: false,
            },
            Vec::new(),
            vec![RetrievedContextItem {
                chunk_id,
                source_document_id,
                scope: KnowledgeScope::Project,
                title: "Project integration".to_owned(),
                body: "Only project knowledge.".to_owned(),
                citation: Citation {
                    source_uri: "git://repo/docs/integration.md".to_owned(),
                    source_path: Some("docs/integration.md".to_owned()),
                    line_start: Some(4),
                    line_end: Some(8),
                },
                score: 0.0,
            }],
        );
        let index = FakeIndex::new(vec![VectorCandidate {
            chunk_id,
            score: 0.9,
        }]);
        let service = RetrievalService::new(store, OkProvider, index, config());

        let principal = RetrievalPrincipal::Agent {
            organization_id,
            project_slugs: vec!["fjulian-me".to_owned()],
            allow_global_knowledge: false,
        };
        let response = service
            .retrieve_context(
                &principal,
                RetrieveContextRequest {
                    project_id,
                    query: "integration".to_owned(),
                    include_global: true,
                    limit: 5,
                },
            )
            .await
            .expect("agent retrieval should succeed");

        let checks = service.store.authorize_checks.lock().expect("lock");
        assert_eq!(checks.len(), 1);
        assert!(matches!(
            &checks[0].0,
            RetrievalPrincipal::Agent {
                organization_id: seen_organization_id,
                project_slugs,
                allow_global_knowledge: false,
            } if *seen_organization_id == organization_id
                && project_slugs == &vec!["fjulian-me".to_owned()]
        ));
        assert_eq!(checks[0].1, project_id);
        assert!(checks[0].2);

        let last_search = service
            .vector_index
            .last_search
            .lock()
            .expect("lock")
            .clone()
            .expect("search should have been called");
        assert_eq!(last_search.organization_id, organization_id);
        assert_eq!(last_search.project_id, project_id.as_uuid());
        assert!(!last_search.include_global);

        assert_eq!(response.retrieval.mode, RetrievalMode::Hybrid);
        assert_eq!(response.items[0].scope, KnowledgeScope::Project);
    }

    fn config() -> RetrievalConfig {
        RetrievalConfig {
            embedding_profile_version: "test-v1".to_owned(),
            rrf_k: 60,
            candidate_multiplier: 4,
            candidate_cap: 100,
        }
    }
}

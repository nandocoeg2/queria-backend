use crate::compress::{compress_items, resolve_compress_enabled};
use crate::embedding::{EmbeddingProvider, VectorIndex, VectorSearchRequest};
use crate::hybrid::{RankedChunk, reciprocal_rank_fusion};
use crate::qdrant::{QdrantClient, QdrantConfig};
use crate::rerank::{VoyageReranker, rerank_items, resolve_rerank_enabled};
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
use std::time::{Duration, Instant};
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
    /// Default when request omits `rerank` (from `QUERIA_RERANK_ENABLED`).
    pub rerank_enabled: bool,
    /// Default when request omits `compress` (from `QUERIA_COMPRESS_ENABLED`).
    pub compress_enabled: bool,
}

#[async_trait]
pub trait HybridRetrievalStore: Send + Sync {
    async fn authorize(
        &self,
        principal: &RetrievalPrincipal,
        project_id: ProjectId,
        include_global: bool,
        include_scratch: bool,
        include_needs_review: bool,
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
    /// Optional Voyage reranker. `None` when API key missing → fail-open skip.
    reranker: Option<VoyageReranker>,
}

pub type PgRetrievalService =
    RetrievalService<PgHybridRetrievalRepository, VoyageClient, QdrantClient>;

pub fn build_pg_retrieval_service(
    config: &AppConfig,
    pool: PgPool,
) -> QueriaResult<PgRetrievalService> {
    let dimension = usize::try_from(config.embedding.dimension)
        .map_err(|_| QueriaError::Config("embedding dimension is invalid".to_owned()))?;
    let reranker = VoyageReranker::try_new(
        &config.embedding.voyage_api_key,
        &config.retrieval.rerank_model,
        Duration::from_secs(config.retrieval.rerank_timeout_seconds),
    );
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
            rerank_enabled: config.retrieval.rerank_enabled,
            compress_enabled: config.retrieval.compress_enabled,
        },
    )
    .with_reranker(reranker))
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
            reranker: None,
        }
    }

    /// Attach an optional Voyage reranker (tests / production builder).
    #[must_use]
    pub fn with_reranker(mut self, reranker: Option<VoyageReranker>) -> Self {
        self.reranker = reranker;
        self
    }

    pub async fn retrieve_context(
        &self,
        principal: &RetrievalPrincipal,
        request: RetrieveContextRequest,
    ) -> QueriaResult<RetrieveContextResponse> {
        request.validate()?;
        let started = Instant::now();
        let access = self
            .store
            .authorize(
                principal,
                request.project_id,
                request.include_global,
                request.include_scratch,
                request.include_needs_review,
            )
            .await?;
        // Oversampled candidate pool: larger than final limit so rerank has headroom.
        // Dense Qdrant filter does not store knowledge status; when include_needs_review is
        // false, oversampling may include NR/inactive status ids that PG hydrate drops.
        let candidate_count = request
            .limit
            .saturating_mul(self.config.candidate_multiplier)
            .min(self.config.candidate_cap);
        let pool_limit = usize::try_from(candidate_count).map_err(|_| {
            QueriaError::Validation("retrieval candidate pool size is invalid".to_owned())
        })?;
        let final_limit = usize::try_from(request.limit)
            .map_err(|_| QueriaError::Validation("retrieval limit is invalid".to_owned()))?;
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
        // RRF over the oversampled pool (not final client limit).
        let fused = reciprocal_rank_fusion(
            &lexical_ranked,
            &semantic_ranked,
            self.config.rrf_k,
            pool_limit,
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

        // Rerank after hydrate on the pool; top_k = final limit (fail open clamps too).
        let rerank_on = resolve_rerank_enabled(request.rerank, self.config.rerank_enabled);
        let rerank_outcome = rerank_items(
            rerank_on,
            self.reranker.as_ref(),
            &request.query,
            items,
            final_limit,
        )
        .await;
        let rerank_applied = rerank_outcome.applied;
        let items = rerank_outcome.items;

        // Compress after ranking/rerank (prefer trusted over scratch).
        let compress_on = resolve_compress_enabled(request.compress, self.config.compress_enabled);
        let compress_outcome = compress_items(items, compress_on);
        let items = compress_outcome.items;
        let compress_dropped = compress_outcome.dropped;

        let latency_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);

        Ok(RetrieveContextResponse {
            project_id: request.project_id,
            query: request.query,
            items,
            retrieval: RetrievalDiagnostics {
                mode,
                lexical_candidates: bounded_count(lexical.len()),
                semantic_candidates: bounded_count(semantic.len()),
                embedding_profile_version: self.config.embedding_profile_version.clone(),
                rerank_applied,
                compress_dropped,
                latency_ms,
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
        include_scratch: bool,
        include_needs_review: bool,
    ) -> QueriaResult<RetrievalAccess> {
        match principal {
            RetrievalPrincipal::User { user_id } => {
                self.authorize_user(
                    *user_id,
                    project_id,
                    include_global,
                    include_scratch,
                    include_needs_review,
                )
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
                    include_scratch,
                    include_needs_review,
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
    use queria_core::contracts::{Citation, KnowledgeLane};
    use queria_core::ids::SourceDocumentId;
    use queria_core::model::{KnowledgeScope, KnowledgeStatus};
    use std::sync::Mutex;

    struct FakeHybridStore {
        access: RetrievalAccess,
        authorize_checks: Mutex<Vec<(RetrievalPrincipal, ProjectId, bool, bool, bool)>>,
        lexical: Mutex<Option<Vec<DbRankedChunk>>>,
        /// Hydrateable corpus keyed by chunk id (returned in fused order).
        hydrate_by_id: Mutex<HashMap<ChunkId, RetrievedContextItem>>,
        last_lexical_limit: Mutex<Option<i64>>,
        last_hydrate_ids: Mutex<Option<Vec<ChunkId>>>,
    }

    impl FakeHybridStore {
        fn new(
            access: RetrievalAccess,
            lexical: Vec<DbRankedChunk>,
            hydrate_items: Vec<RetrievedContextItem>,
        ) -> Self {
            let hydrate_by_id = hydrate_items
                .into_iter()
                .map(|item| (item.chunk_id, item))
                .collect();
            Self {
                access,
                authorize_checks: Mutex::new(Vec::new()),
                lexical: Mutex::new(Some(lexical)),
                hydrate_by_id: Mutex::new(hydrate_by_id),
                last_lexical_limit: Mutex::new(None),
                last_hydrate_ids: Mutex::new(None),
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
            include_scratch: bool,
            include_needs_review: bool,
        ) -> QueriaResult<RetrievalAccess> {
            self.authorize_checks.lock().expect("lock").push((
                principal.clone(),
                project_id,
                include_global,
                include_scratch,
                include_needs_review,
            ));
            Ok(self.access.clone())
        }

        async fn lexical_search(
            &self,
            _access: &RetrievalAccess,
            _query: &str,
            limit: i64,
        ) -> QueriaResult<Vec<DbRankedChunk>> {
            *self.last_lexical_limit.lock().expect("lock") = Some(limit);
            let mut all = self
                .lexical
                .lock()
                .expect("lock")
                .take()
                .unwrap_or_default();
            if limit >= 0 {
                let cap = usize::try_from(limit).unwrap_or(usize::MAX);
                all.truncate(cap);
            }
            Ok(all)
        }

        async fn hydrate(
            &self,
            _access: &RetrievalAccess,
            chunk_ids: &[ChunkId],
        ) -> QueriaResult<Vec<RetrievedContextItem>> {
            *self.last_hydrate_ids.lock().expect("lock") = Some(chunk_ids.to_vec());
            let map = self.hydrate_by_id.lock().expect("lock");
            Ok(chunk_ids
                .iter()
                .filter_map(|id| map.get(id).cloned())
                .collect())
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

        async fn search(&self, request: VectorSearchRequest) -> QueriaResult<Vec<VectorCandidate>> {
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
                include_scratch: true,
                include_needs_review: false,
            },
            vec![DbRankedChunk {
                chunk_id,
                score: 0.8,
            }],
            vec![RetrievedContextItem {
                chunk_id,
                source_document_id,
                scope: KnowledgeScope::Project,
                status: KnowledgeStatus::Approved,
                lane: KnowledgeLane::Trusted,
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
                    include_scratch: false,
                    include_needs_review: false,
                    limit: 5,
                    rerank: None,
                    compress: None,
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
                include_scratch: true,
                include_needs_review: false,
            },
            vec![DbRankedChunk {
                chunk_id: shared,
                score: 0.8,
            }],
            vec![RetrievedContextItem {
                chunk_id: shared,
                source_document_id,
                scope: KnowledgeScope::Project,
                status: KnowledgeStatus::Approved,
                lane: KnowledgeLane::Trusted,
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
                    include_scratch: false,
                    include_needs_review: false,
                    limit: 5,
                    rerank: None,
                    compress: None,
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
                include_scratch: true,
                include_needs_review: false,
            },
            Vec::new(),
            vec![RetrievedContextItem {
                chunk_id,
                source_document_id,
                scope: KnowledgeScope::Project,
                status: KnowledgeStatus::Approved,
                lane: KnowledgeLane::Trusted,
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
                    // VAL-DL-026: agent path default is include_scratch true
                    include_scratch: true,
                    include_needs_review: false,
                    limit: 5,
                    rerank: None,
                    compress: None,
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
        assert!(checks[0].3, "include_scratch must pass through authorize");
        assert!(
            !checks[0].4,
            "include_needs_review default false must pass through authorize"
        );

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
            // Unit tests here do not attach a reranker; default-off keeps RRF only.
            rerank_enabled: false,
            compress_enabled: true,
        }
    }

    /// VAL-RET-008 / VAL-CROSS-008: service path with rerank desired but no key still succeeds.
    #[tokio::test]
    async fn missing_reranker_fail_open_on_service() {
        let project_id = ProjectId::new();
        let organization_id = Uuid::now_v7();
        let chunk_a = ChunkId::new();
        let chunk_b = ChunkId::new();
        let source_document_id = SourceDocumentId::new();
        let store = FakeHybridStore::new(
            RetrievalAccess {
                organization_id,
                project_id,
                include_global: true,
                include_scratch: true,
                include_needs_review: false,
            },
            vec![
                DbRankedChunk {
                    chunk_id: chunk_a,
                    score: 0.9,
                },
                DbRankedChunk {
                    chunk_id: chunk_b,
                    score: 0.5,
                },
            ],
            vec![
                RetrievedContextItem {
                    chunk_id: chunk_a,
                    source_document_id,
                    scope: KnowledgeScope::Project,
                    status: KnowledgeStatus::Approved,
                    lane: KnowledgeLane::Trusted,
                    title: "A".to_owned(),
                    body: "first rrf".to_owned(),
                    citation: Citation {
                        source_uri: "git://repo/a.md".to_owned(),
                        source_path: Some("a.md".to_owned()),
                        line_start: Some(1),
                        line_end: Some(2),
                    },
                    score: 0.0,
                },
                RetrievedContextItem {
                    chunk_id: chunk_b,
                    source_document_id,
                    scope: KnowledgeScope::Project,
                    status: KnowledgeStatus::Approved,
                    lane: KnowledgeLane::Trusted,
                    title: "B".to_owned(),
                    body: "second rrf".to_owned(),
                    citation: Citation {
                        source_uri: "git://repo/b.md".to_owned(),
                        source_path: Some("b.md".to_owned()),
                        line_start: Some(1),
                        line_end: Some(2),
                    },
                    score: 0.0,
                },
            ],
        );
        let mut cfg = config();
        cfg.rerank_enabled = true;
        // No .with_reranker — simulates missing VOYAGE_API_KEY.
        let service = RetrievalService::new(store, FailProvider, FakeIndex::empty(), cfg);

        let response = service
            .retrieve_context(
                &RetrievalPrincipal::User {
                    user_id: Uuid::now_v7(),
                },
                RetrieveContextRequest {
                    project_id,
                    query: "rrf order".to_owned(),
                    include_global: true,
                    include_scratch: true,
                    include_needs_review: false,
                    limit: 5,
                    rerank: None,
                    compress: Some(false),
                },
            )
            .await
            .expect("missing key must not hard-fail retrieve");

        assert!(!response.retrieval.rerank_applied);
        assert_eq!(response.items.len(), 2);
        assert_eq!(response.items[0].chunk_id, chunk_a);
        assert_eq!(response.items[1].chunk_id, chunk_b);
        assert!(!response.items[0].body.is_empty());
        assert_eq!(response.items[0].lane, KnowledgeLane::Trusted);
    }

    /// VAL-RET-006: explicit rerank=false forces skip when config default on.
    #[tokio::test]
    async fn explicit_rerank_false_skips() {
        let project_id = ProjectId::new();
        let organization_id = Uuid::now_v7();
        let chunk_id = ChunkId::new();
        let source_document_id = SourceDocumentId::new();
        let store = FakeHybridStore::new(
            RetrievalAccess {
                organization_id,
                project_id,
                include_global: true,
                include_scratch: true,
                include_needs_review: false,
            },
            vec![DbRankedChunk {
                chunk_id,
                score: 0.8,
            }],
            vec![RetrievedContextItem {
                chunk_id,
                source_document_id,
                scope: KnowledgeScope::Project,
                status: KnowledgeStatus::Approved,
                lane: KnowledgeLane::Trusted,
                title: "One".to_owned(),
                body: "body".to_owned(),
                citation: Citation {
                    source_uri: "git://repo/one.md".to_owned(),
                    source_path: Some("one.md".to_owned()),
                    line_start: Some(1),
                    line_end: Some(1),
                },
                score: 0.0,
            }],
        );
        let mut cfg = config();
        cfg.rerank_enabled = true;
        let service = RetrievalService::new(store, FailProvider, FakeIndex::empty(), cfg);

        let response = service
            .retrieve_context(
                &RetrievalPrincipal::User {
                    user_id: Uuid::now_v7(),
                },
                RetrieveContextRequest {
                    project_id,
                    query: "skip rerank".to_owned(),
                    include_global: true,
                    include_scratch: false,
                    include_needs_review: false,
                    limit: 5,
                    rerank: Some(false),
                    compress: Some(false),
                },
            )
            .await
            .expect("override false should succeed");

        assert!(!response.retrieval.rerank_applied);
        assert_eq!(response.items.len(), 1);
    }

    fn citation(path: &str) -> Citation {
        Citation {
            source_uri: format!("git://repo/{path}"),
            source_path: Some(path.to_owned()),
            line_start: Some(1),
            line_end: Some(2),
        }
    }

    fn item_with(
        chunk_id: ChunkId,
        title: &str,
        body: &str,
        lane: KnowledgeLane,
    ) -> RetrievedContextItem {
        let status = match lane {
            KnowledgeLane::Trusted => KnowledgeStatus::Approved,
            KnowledgeLane::Scratch => KnowledgeStatus::Scratch,
            KnowledgeLane::NeedsReview => KnowledgeStatus::NeedsReview,
        };
        RetrievedContextItem {
            chunk_id,
            source_document_id: SourceDocumentId::new(),
            scope: KnowledgeScope::Project,
            status,
            lane,
            title: title.to_owned(),
            body: body.to_owned(),
            citation: citation(&format!("{title}.md")),
            score: 0.0,
        }
    }

    /// VAL-RET-001: RRF/hydrate pool is larger than final limit when corpus allows.
    #[tokio::test]
    async fn oversampled_pool_before_final_limit_cut() {
        let project_id = ProjectId::new();
        let organization_id = Uuid::now_v7();
        // 8 distinct lexical hits; limit=2, multiplier=4 → pool_limit=8.
        let chunks: Vec<ChunkId> = (0..8).map(|_| ChunkId::new()).collect();
        let lexical: Vec<DbRankedChunk> = chunks
            .iter()
            .enumerate()
            .map(|(i, id)| DbRankedChunk {
                chunk_id: *id,
                score: 1.0 - (i as f32) * 0.05,
            })
            .collect();
        let hydrate: Vec<RetrievedContextItem> = chunks
            .iter()
            .enumerate()
            .map(|(i, id)| {
                item_with(
                    *id,
                    &format!("Doc{i}"),
                    &format!("unique body number {i}"),
                    KnowledgeLane::Trusted,
                )
            })
            .collect();
        let store = FakeHybridStore::new(
            RetrievalAccess {
                organization_id,
                project_id,
                include_global: true,
                include_scratch: true,
                include_needs_review: false,
            },
            lexical,
            hydrate,
        );
        let mut cfg = config();
        cfg.candidate_multiplier = 4;
        cfg.candidate_cap = 100;
        cfg.rerank_enabled = false;
        cfg.compress_enabled = false;
        let service = RetrievalService::new(store, FailProvider, FakeIndex::empty(), cfg);

        let limit = 2_u32;
        let response = service
            .retrieve_context(
                &RetrievalPrincipal::User {
                    user_id: Uuid::now_v7(),
                },
                RetrieveContextRequest {
                    project_id,
                    query: "pool oversample".to_owned(),
                    include_global: true,
                    include_scratch: true,
                    include_needs_review: false,
                    limit,
                    rerank: Some(false),
                    compress: Some(false),
                },
            )
            .await
            .expect("retrieve should succeed");

        let lexical_limit = service
            .store
            .last_lexical_limit
            .lock()
            .expect("lock")
            .expect("lexical_search should record limit");
        assert_eq!(
            lexical_limit,
            i64::from(limit.saturating_mul(4)),
            "lexical search must request oversampled pool"
        );

        let hydrate_ids = service
            .store
            .last_hydrate_ids
            .lock()
            .expect("lock")
            .clone()
            .expect("hydrate should be called");
        assert!(
            hydrate_ids.len() as u32 > limit,
            "RRF pool ({}) must be larger than final limit ({})",
            hydrate_ids.len(),
            limit
        );
        assert_eq!(
            hydrate_ids.len(),
            8,
            "full 8-candidate corpus should fit in pool before cut"
        );
        // VAL-RET-002: final items never exceed limit.
        assert!(response.items.len() <= limit as usize);
        assert_eq!(response.items.len(), limit as usize);
    }

    /// VAL-RET-002: final item count respects limit after full pipeline.
    #[tokio::test]
    async fn final_items_respect_limit_with_rich_corpus() {
        let project_id = ProjectId::new();
        let organization_id = Uuid::now_v7();
        let chunks: Vec<ChunkId> = (0..12).map(|_| ChunkId::new()).collect();
        let lexical: Vec<DbRankedChunk> = chunks
            .iter()
            .enumerate()
            .map(|(i, id)| DbRankedChunk {
                chunk_id: *id,
                score: 1.0 - (i as f32) * 0.01,
            })
            .collect();
        let hydrate: Vec<RetrievedContextItem> = chunks
            .iter()
            .enumerate()
            .map(|(i, id)| {
                item_with(
                    *id,
                    &format!("Hit{i}"),
                    &format!("distinct content {i}"),
                    KnowledgeLane::Trusted,
                )
            })
            .collect();
        let store = FakeHybridStore::new(
            RetrievalAccess {
                organization_id,
                project_id,
                include_global: true,
                include_scratch: true,
                include_needs_review: false,
            },
            lexical,
            hydrate,
        );
        let service = RetrievalService::new(store, FailProvider, FakeIndex::empty(), config());
        let limit = 5_u32;

        let response = service
            .retrieve_context(
                &RetrievalPrincipal::User {
                    user_id: Uuid::now_v7(),
                },
                RetrieveContextRequest {
                    project_id,
                    query: "limit clamp".to_owned(),
                    include_global: true,
                    include_scratch: true,
                    include_needs_review: false,
                    limit,
                    rerank: Some(false),
                    compress: Some(false),
                },
            )
            .await
            .expect("ok");

        assert!(response.items.len() <= limit as usize);
        assert_eq!(response.items.len(), 5);
    }

    /// VAL-RET-013: include_scratch dual-lane still passes authorize and returns lane truth.
    #[tokio::test]
    async fn include_scratch_dual_lane_with_pipeline_stages() {
        let project_id = ProjectId::new();
        let organization_id = Uuid::now_v7();
        let trusted_id = ChunkId::new();
        let scratch_id = ChunkId::new();
        let store = FakeHybridStore::new(
            RetrievalAccess {
                organization_id,
                project_id,
                include_global: true,
                include_scratch: true,
                include_needs_review: false,
            },
            vec![
                DbRankedChunk {
                    chunk_id: trusted_id,
                    score: 0.9,
                },
                DbRankedChunk {
                    chunk_id: scratch_id,
                    score: 0.8,
                },
            ],
            vec![
                item_with(
                    trusted_id,
                    "Trusted",
                    "shared topic text",
                    KnowledgeLane::Trusted,
                ),
                item_with(
                    scratch_id,
                    "Scratch",
                    "shared topic text",
                    KnowledgeLane::Scratch,
                ),
            ],
        );
        // compress on: near-dup prefers trusted, drops scratch.
        let mut cfg = config();
        cfg.compress_enabled = true;
        cfg.rerank_enabled = false;
        let service = RetrievalService::new(store, FailProvider, FakeIndex::empty(), cfg);

        let response = service
            .retrieve_context(
                &RetrievalPrincipal::User {
                    user_id: Uuid::now_v7(),
                },
                RetrieveContextRequest {
                    project_id,
                    query: "dual lane".to_owned(),
                    include_global: true,
                    include_scratch: true,
                    include_needs_review: false,
                    limit: 5,
                    rerank: Some(false),
                    compress: Some(true),
                },
            )
            .await
            .expect("dual-lane retrieve ok");

        let checks = service.store.authorize_checks.lock().expect("lock");
        assert_eq!(checks.len(), 1);
        assert!(checks[0].3, "include_scratch=true must reach authorize");
        // VAL-RET-010 interaction: trusted survives compress.
        assert_eq!(response.items.len(), 1);
        assert_eq!(response.items[0].chunk_id, trusted_id);
        assert_eq!(response.items[0].lane, KnowledgeLane::Trusted);
        assert!(response.retrieval.compress_dropped >= 1);
    }

    /// IMP-L3: include_needs_review=true reaches authorize; default false.
    #[tokio::test]
    async fn include_needs_review_true_passes_authorize() {
        let project_id = ProjectId::new();
        let organization_id = Uuid::now_v7();
        let chunk_id = ChunkId::new();
        let store = FakeHybridStore::new(
            RetrievalAccess {
                organization_id,
                project_id,
                include_global: true,
                include_scratch: true,
                include_needs_review: true,
            },
            vec![DbRankedChunk {
                chunk_id,
                score: 0.9,
            }],
            vec![item_with(
                chunk_id,
                "NeedsReviewDoc",
                "needs review body",
                KnowledgeLane::NeedsReview,
            )],
        );
        let service = RetrievalService::new(store, FailProvider, FakeIndex::empty(), config());
        let response = service
            .retrieve_context(
                &RetrievalPrincipal::User {
                    user_id: Uuid::now_v7(),
                },
                RetrieveContextRequest {
                    project_id,
                    query: "needs review".to_owned(),
                    include_global: true,
                    include_scratch: true,
                    include_needs_review: true,
                    limit: 5,
                    rerank: Some(false),
                    compress: Some(false),
                },
            )
            .await
            .expect("retrieve");
        let checks = service.store.authorize_checks.lock().expect("lock");
        assert!(
            checks[0].4,
            "include_needs_review=true must reach authorize"
        );
        assert_eq!(response.items.len(), 1);
        assert_eq!(response.items[0].lane, KnowledgeLane::NeedsReview);
        assert_eq!(response.items[0].status, KnowledgeStatus::NeedsReview);
    }

    /// VAL-RET-013 false path: include_scratch=false still forwarded to authorize.
    #[tokio::test]
    async fn include_scratch_false_still_filters_via_authorize() {
        let project_id = ProjectId::new();
        let organization_id = Uuid::now_v7();
        let trusted_id = ChunkId::new();
        let store = FakeHybridStore::new(
            RetrievalAccess {
                organization_id,
                project_id,
                include_global: true,
                include_scratch: false,
                include_needs_review: false,
            },
            vec![DbRankedChunk {
                chunk_id: trusted_id,
                score: 0.9,
            }],
            vec![item_with(
                trusted_id,
                "OnlyTrusted",
                "body",
                KnowledgeLane::Trusted,
            )],
        );
        let service = RetrievalService::new(store, FailProvider, FakeIndex::empty(), config());

        let _ = service
            .retrieve_context(
                &RetrievalPrincipal::User {
                    user_id: Uuid::now_v7(),
                },
                RetrieveContextRequest {
                    project_id,
                    query: "no scratch".to_owned(),
                    include_global: true,
                    include_scratch: false,
                    include_needs_review: false,
                    limit: 3,
                    rerank: Some(false),
                    compress: Some(false),
                },
            )
            .await
            .expect("ok");

        let checks = service.store.authorize_checks.lock().expect("lock");
        assert!(!checks[0].3, "include_scratch=false must reach authorize");
    }

    /// VAL-RET-014: lexical fallback still returns items; diagnostics mode + latency.
    #[tokio::test]
    async fn lexical_fallback_with_diagnostics_and_no_5xx() {
        let project_id = ProjectId::new();
        let organization_id = Uuid::now_v7();
        let chunk_id = ChunkId::new();
        let store = FakeHybridStore::new(
            RetrievalAccess {
                organization_id,
                project_id,
                include_global: true,
                include_scratch: true,
                include_needs_review: false,
            },
            vec![DbRankedChunk {
                chunk_id,
                score: 0.7,
            }],
            vec![item_with(
                chunk_id,
                "LexicalOnly",
                "fts body",
                KnowledgeLane::Trusted,
            )],
        );
        // Rerank desired but no key → fail-open must not 5xx.
        let mut cfg = config();
        cfg.rerank_enabled = true;
        let service = RetrievalService::new(store, FailProvider, FakeIndex::empty(), cfg);

        let response = service
            .retrieve_context(
                &RetrievalPrincipal::User {
                    user_id: Uuid::now_v7(),
                },
                RetrieveContextRequest {
                    project_id,
                    query: "fallback path".to_owned(),
                    include_global: true,
                    include_scratch: true,
                    include_needs_review: false,
                    limit: 5,
                    rerank: None,
                    compress: Some(false),
                },
            )
            .await
            .expect("lexical fallback must not hard-fail");

        assert_eq!(response.retrieval.mode, RetrievalMode::LexicalFallback);
        assert_eq!(response.items.len(), 1);
        assert!(!response.retrieval.rerank_applied);
        assert_eq!(response.retrieval.compress_dropped, 0);
        // VAL-RET diagnostics complete: latency_ms is finite (always for Instant).
        let _ = response.retrieval.latency_ms;
        assert!(!response.retrieval.embedding_profile_version.is_empty());
    }

    /// End-to-end stage order: pool → RRF hydrate → compress after rank; diagnostics complete.
    #[tokio::test]
    async fn end_to_end_stage_order_pool_then_compress_diagnostics() {
        let project_id = ProjectId::new();
        let organization_id = Uuid::now_v7();
        // 6 candidates with one near-dup pair at top ranks.
        let ids: Vec<ChunkId> = (0..6).map(|_| ChunkId::new()).collect();
        let lexical: Vec<DbRankedChunk> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| DbRankedChunk {
                chunk_id: *id,
                score: 1.0 - (i as f32) * 0.1,
            })
            .collect();
        let mut hydrate: Vec<RetrievedContextItem> = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let body = if i == 0 || i == 1 {
                    "near dup body   with   spaces".to_owned()
                } else {
                    format!("unique body {i}")
                };
                let lane = if i == 1 {
                    KnowledgeLane::Scratch
                } else {
                    KnowledgeLane::Trusted
                };
                item_with(*id, &format!("T{i}"), &body, lane)
            })
            .collect();
        // Force first = scratch near-dup, second = trusted near-dup so compress prefers trusted.
        hydrate[0].lane = KnowledgeLane::Scratch;
        hydrate[1].lane = KnowledgeLane::Trusted;

        let store = FakeHybridStore::new(
            RetrievalAccess {
                organization_id,
                project_id,
                include_global: true,
                include_scratch: true,
                include_needs_review: false,
            },
            lexical,
            hydrate,
        );
        let mut cfg = config();
        cfg.candidate_multiplier = 3;
        cfg.compress_enabled = true;
        cfg.rerank_enabled = false;
        let service = RetrievalService::new(store, FailProvider, FakeIndex::empty(), cfg);
        let limit = 3_u32;

        let response = service
            .retrieve_context(
                &RetrievalPrincipal::User {
                    user_id: Uuid::now_v7(),
                },
                RetrieveContextRequest {
                    project_id,
                    query: "stage order".to_owned(),
                    include_global: true,
                    include_scratch: true,
                    include_needs_review: false,
                    limit,
                    rerank: Some(false),
                    compress: Some(true),
                },
            )
            .await
            .expect("pipeline ok");

        // Pool stage saw more than final limit.
        let pool_len = service
            .store
            .last_hydrate_ids
            .lock()
            .expect("lock")
            .as_ref()
            .map(Vec::len)
            .unwrap_or(0);
        assert!(
            pool_len as u32 > limit,
            "hydrate pool {pool_len} should exceed limit {limit}"
        );

        // Final ≤ limit; compress dropped the near-dup; diagnostics complete.
        assert!(response.items.len() <= limit as usize);
        assert!(response.retrieval.compress_dropped >= 1);
        assert!(!response.retrieval.rerank_applied);
        assert_eq!(response.retrieval.mode, RetrievalMode::LexicalFallback);
        assert_eq!(response.retrieval.lexical_candidates, 6);
        assert_eq!(response.retrieval.semantic_candidates, 0);
        assert!(!response.retrieval.embedding_profile_version.is_empty());
        // latency_ms always present (Instant elapsed; may be 0 in fast unit tests).
        let _latency = response.retrieval.latency_ms;

        // Trusted preferred among near-dups (VAL-RET-010 in full path).
        let survivor_of_pair = response
            .items
            .iter()
            .find(|i| i.body.contains("near dup") || i.body.contains("near"));
        if let Some(item) = survivor_of_pair {
            assert_eq!(item.lane, KnowledgeLane::Trusted);
        }
    }

    /// Hybrid path records semantic search with candidate pool size (not limit).
    #[tokio::test]
    async fn hybrid_semantic_search_uses_candidate_pool_size() {
        let project_id = ProjectId::new();
        let organization_id = Uuid::now_v7();
        let chunks: Vec<ChunkId> = (0..6).map(|_| ChunkId::new()).collect();
        let lexical: Vec<DbRankedChunk> = chunks
            .iter()
            .take(3)
            .enumerate()
            .map(|(i, id)| DbRankedChunk {
                chunk_id: *id,
                score: 0.9 - i as f32 * 0.1,
            })
            .collect();
        let semantic: Vec<VectorCandidate> = chunks
            .iter()
            .skip(1)
            .take(5)
            .enumerate()
            .map(|(i, id)| VectorCandidate {
                chunk_id: *id,
                score: 0.95 - i as f32 * 0.05,
            })
            .collect();
        let hydrate: Vec<RetrievedContextItem> = chunks
            .iter()
            .enumerate()
            .map(|(i, id)| {
                item_with(
                    *id,
                    &format!("H{i}"),
                    &format!("hybrid body {i}"),
                    KnowledgeLane::Trusted,
                )
            })
            .collect();
        let store = FakeHybridStore::new(
            RetrievalAccess {
                organization_id,
                project_id,
                include_global: false,
                include_scratch: true,
                include_needs_review: false,
            },
            lexical,
            hydrate,
        );
        let index = FakeIndex::new(semantic);
        let mut cfg = config();
        cfg.candidate_multiplier = 4;
        cfg.rerank_enabled = false;
        cfg.compress_enabled = false;
        let service = RetrievalService::new(store, OkProvider, index, cfg);
        let limit = 2_u32;

        let response = service
            .retrieve_context(
                &RetrievalPrincipal::User {
                    user_id: Uuid::now_v7(),
                },
                RetrieveContextRequest {
                    project_id,
                    query: "hybrid pool".to_owned(),
                    include_global: false,
                    include_scratch: true,
                    include_needs_review: false,
                    limit,
                    rerank: Some(false),
                    compress: Some(false),
                },
            )
            .await
            .expect("hybrid ok");

        let last_search = service
            .vector_index
            .last_search
            .lock()
            .expect("lock")
            .clone()
            .expect("search called");
        assert_eq!(
            last_search.limit,
            limit.saturating_mul(4),
            "dense search must use candidate pool not final limit"
        );

        let pool_len = service
            .store
            .last_hydrate_ids
            .lock()
            .expect("lock")
            .as_ref()
            .map(Vec::len)
            .unwrap_or(0);
        assert!(
            pool_len as u32 > limit,
            "fused pool should exceed final limit when corpus allows"
        );
        assert!(response.items.len() <= limit as usize);
        assert_eq!(response.retrieval.mode, RetrievalMode::Hybrid);
        assert!(response.retrieval.semantic_candidates > 0);
    }
}

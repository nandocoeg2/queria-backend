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

impl RetrievalPrincipal {
    /// Build the agent principal used by both MCP and HTTP agent retrieve.
    #[must_use]
    pub fn agent(
        organization_id: Uuid,
        project_slugs: impl IntoIterator<Item = impl Into<String>>,
        allow_global_knowledge: bool,
    ) -> Self {
        Self::Agent {
            organization_id,
            project_slugs: project_slugs.into_iter().map(Into::into).collect(),
            allow_global_knowledge,
        }
    }
}

/// Limit policy for agent retrieve transports (same business path; different budgets).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentRetrieveLimitPolicy {
    /// MCP `retrieve_context` / `search_knowledge`: omit → default 5; no extra clamp
    /// (contract still validates 1..=20).
    Mcp,
    /// HTTP `POST /api/v1/agent/retrieve-context`: omit → 5; clamp 1..=10.
    HttpHook,
}

/// Input fields for [`build_agent_retrieve_request`] (MCP + HTTP thin wrappers).
#[derive(Clone, Debug)]
pub struct AgentRetrieveParams {
    pub project_id: ProjectId,
    pub query: String,
    pub include_global: Option<bool>,
    pub include_scratch: Option<bool>,
    pub include_needs_review: Option<bool>,
    pub limit: Option<u32>,
    pub limit_policy: AgentRetrieveLimitPolicy,
    pub rerank: Option<bool>,
    pub compress: Option<bool>,
}

/// Build a [`RetrieveContextRequest`] with agent dual-lane defaults for MCP or HTTP.
///
/// Defaults (both transports): `include_scratch=true`, `include_needs_review=false`,
/// `include_global=true`, omitted limit → 5. HTTP additionally clamps limit to 1..=10.
#[must_use]
pub fn build_agent_retrieve_request(params: AgentRetrieveParams) -> RetrieveContextRequest {
    use queria_core::{
        AGENT_HTTP_RETRIEVE_LIMIT_DEFAULT, AGENT_RETRIEVE_LIMIT_DEFAULT, agent_include_global,
        agent_include_needs_review, agent_include_scratch, clamp_agent_http_retrieve_limit,
    };

    let limit = match params.limit_policy {
        AgentRetrieveLimitPolicy::Mcp => params.limit.unwrap_or(AGENT_RETRIEVE_LIMIT_DEFAULT),
        AgentRetrieveLimitPolicy::HttpHook => clamp_agent_http_retrieve_limit(
            params.limit.unwrap_or(AGENT_HTTP_RETRIEVE_LIMIT_DEFAULT),
        ),
    };

    RetrieveContextRequest {
        project_id: params.project_id,
        query: params.query,
        include_global: agent_include_global(params.include_global),
        include_scratch: agent_include_scratch(params.include_scratch),
        include_needs_review: agent_include_needs_review(params.include_needs_review),
        limit,
        rerank: params.rerank,
        compress: params.compress,
    }
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

    /// Shared agent retrieve entry used by MCP `retrieve_context` and HTTP
    /// `POST /api/v1/agent/retrieve-context`. Always runs under
    /// [`RetrievalPrincipal::Agent`] — no separate hybrid scoring path per transport.
    pub async fn retrieve_for_agent(
        &self,
        organization_id: Uuid,
        project_slugs: impl IntoIterator<Item = impl Into<String>>,
        allow_global_knowledge: bool,
        request: RetrieveContextRequest,
    ) -> QueriaResult<RetrieveContextResponse> {
        let principal =
            RetrievalPrincipal::agent(organization_id, project_slugs, allow_global_knowledge);
        self.retrieve_context(&principal, request).await
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
#[path = "retrieval_tests.rs"]
mod tests;

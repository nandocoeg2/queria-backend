use crate::server::McpState;
use crate::tools;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use queria_core::auth::agent_token::AgentTokenIssuer;
use queria_core::contracts::{
    RetrieveContextRequest, RetrieveContextResponse, scratch_content_hash, validate_memory_body,
};
use queria_core::ids::{KnowledgeItemId, ProjectId, SourceDocumentId};
use queria_db::repositories::{
    AuthenticatedAgentToken, IndexMemoryParams, KnowledgeItemRecord, NeedsReviewActionRecord,
    NeedsReviewItemRecord, PgProjectRepository, ProjectRecord, ProposeMemoryParams,
    SourceDocumentRecord,
};
use queria_search::retrieval::RetrievalPrincipal;
use queria_search::scratch_embed::{build_embed_clients, index_memory_with_sync_embed};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    id: Option<Value>,
    method: String,
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ToolCallParams {
    name: String,
    #[serde(default)]
    arguments: Value,
}

#[derive(Debug, Deserialize)]
struct RetrievalArgs {
    project_id: ProjectId,
    query: String,
    include_global: Option<bool>,
    /// Agent default true (IMP-14); omit or true for dual-lane, false for trusted-only.
    include_scratch: Option<bool>,
    /// Default false (IMP-L3); when true include project-scoped needs_review.
    include_needs_review: Option<bool>,
    limit: Option<u32>,
    /// `None` uses server `QUERIA_RERANK_ENABLED` default.
    rerank: Option<bool>,
    /// `None` uses server `QUERIA_COMPRESS_ENABLED` default.
    compress: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct GetSourceArgs {
    source_document_id: SourceDocumentId,
}

#[derive(Debug, Deserialize)]
struct ProposeMemoryArgs {
    project_slug: String,
    title: String,
    body: String,
    category: String,
    #[serde(default)]
    tags: Vec<String>,
}

/// Args for `index_memory` (IMP-13 write path + IMP-23 body validation).
#[derive(Debug, Deserialize)]
struct IndexMemoryArgs {
    #[serde(default)]
    project_id: Option<ProjectId>,
    #[serde(default)]
    project_slug: Option<String>,
    body: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

/// Args for `list_needs_review` (IMP-L5 privileged).
#[derive(Debug, Deserialize)]
struct ListNeedsReviewArgs {
    #[serde(default)]
    project_slug: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}

/// Args for `promote_knowledge` / `reject_needs_review` (IMP-L5 privileged).
#[derive(Debug, Deserialize)]
struct NeedsReviewItemArgs {
    knowledge_item_id: KnowledgeItemId,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

pub fn router() -> Router<McpState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/mcp", post(mcp))
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "queria-mcp",
    })
}

async fn mcp(
    State(state): State<McpState>,
    headers: HeaderMap,
    Json(request): Json<JsonRpcRequest>,
) -> impl IntoResponse {
    if bearer_token(&headers).is_none() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "agent_token_required" })),
        );
    }

    let repository = match state.project_repository() {
        Some(repository) => repository,
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json_rpc_error(
                    request.id,
                    -32603,
                    "knowledge_store_not_configured",
                )),
            );
        }
    };

    let agent = match require_agent_token(&repository, &headers).await {
        Ok(agent) => agent,
        Err(status) => return (status, Json(json!({ "error": "agent_token_required" }))),
    };

    let response = match request.method.as_str() {
        "initialize" => json_rpc_result(request.id, initialize_result()),
        "notifications/initialized" => json_rpc_result(request.id, json!({})),
        "tools/list" => json_rpc_result(
            request.id,
            json!({ "tools": tools::tool_definitions(&agent.permissions) }),
        ),
        "tools/call" => match call_tool(&state, &repository, &agent, request.params).await {
            Ok(value) => json_rpc_result(request.id, value),
            Err(error) => json_rpc_error(request.id, -32602, &error),
        },
        _ => json_rpc_error(request.id, -32601, "method_not_found"),
    };

    (StatusCode::OK, Json(response))
}

async fn call_tool(
    state: &McpState,
    repository: &PgProjectRepository,
    agent: &AuthenticatedAgentToken,
    params: Option<Value>,
) -> Result<Value, String> {
    let params: ToolCallParams =
        serde_json::from_value(params.unwrap_or_else(|| json!({}))).map_err(invalid_params)?;
    let Some(permission) = tools::permission_for_tool(&params.name) else {
        return Err("unknown_tool".to_owned());
    };

    if !agent.permissions.can_call(&permission) {
        return Ok(tool_error("permission_denied"));
    }

    match params.name.as_str() {
        "list_projects" => {
            let projects = repository
                .list_projects_for_agent(agent)
                .await
                .map_err(infrastructure_error)?;
            Ok(tool_success(json!({
                "projects": projects.into_iter().map(project_json).collect::<Vec<_>>()
            })))
        }
        "retrieve_context" => {
            let args: RetrievalArgs =
                serde_json::from_value(params.arguments).map_err(invalid_params)?;
            let request = RetrieveContextRequest {
                project_id: args.project_id,
                query: args.query,
                include_global: args.include_global.unwrap_or(true),
                // VAL-DL-026 / VAL-CROSS-004: agent default include_scratch=true
                include_scratch: args.include_scratch.unwrap_or(true),
                // IMP-L3: default false even for agents
                include_needs_review: args.include_needs_review.unwrap_or(false),
                limit: args.limit.unwrap_or(5),
                rerank: args.rerank,
                compress: args.compress,
            };
            let response = hybrid_retrieve(state, agent, request).await?;
            Ok(tool_success(json!(response)))
        }
        "search_knowledge" => {
            let args: RetrievalArgs =
                serde_json::from_value(params.arguments).map_err(invalid_params)?;
            let request = RetrieveContextRequest {
                project_id: args.project_id,
                query: args.query,
                include_global: args.include_global.unwrap_or(true),
                include_scratch: args.include_scratch.unwrap_or(true),
                include_needs_review: args.include_needs_review.unwrap_or(false),
                limit: args.limit.unwrap_or(10),
                rerank: args.rerank,
                compress: args.compress,
            };
            let response = hybrid_retrieve(state, agent, request).await?;
            Ok(tool_success(json!(response)))
        }
        "get_source" => {
            let args: GetSourceArgs =
                serde_json::from_value(params.arguments).map_err(invalid_params)?;
            let source = repository
                .get_source_document_for_agent(agent, args.source_document_id)
                .await
                .map_err(infrastructure_error)?
                .ok_or_else(|| "source_document_not_found".to_owned())?;
            Ok(tool_success(source_json(source)))
        }
        "propose_memory" => {
            let args: ProposeMemoryArgs =
                serde_json::from_value(params.arguments).map_err(invalid_params)?;
            let params = args.into_params(state.config.max_body_bytes)?;
            let proposed = repository
                .propose_memory_for_agent(agent, params)
                .await
                .map_err(infrastructure_error)?;
            Ok(tool_success(json!({
                "knowledge_item_id": proposed.knowledge_item_id,
                "status": proposed.status,
                "title": proposed.title
            })))
        }
        "index_memory" => {
            let args: IndexMemoryArgs =
                serde_json::from_value(params.arguments).map_err(invalid_params)?;
            let write_params = args.into_params(state.config.max_body_bytes)?;
            let (voyage, qdrant, profile) =
                build_embed_clients(&state.config).map_err(infrastructure_error)?;
            let indexed = index_memory_with_sync_embed(
                repository,
                agent,
                write_params,
                &voyage,
                &qdrant,
                &profile,
            )
            .await
            .map_err(index_memory_error)?;
            Ok(tool_success(json!({
                "knowledge_item_id": indexed.knowledge_item_id,
                "chunk_id": indexed.chunk_id,
                "project_id": indexed.project_id,
                "status": indexed.status,
                "scope": indexed.scope,
                "title": indexed.title,
                "content_hash": indexed.content_hash,
                "created": indexed.created,
                "idempotent": indexed.idempotent
            })))
        }
        "list_needs_review" => {
            let args_value = if params.arguments.is_null() {
                json!({})
            } else {
                params.arguments
            };
            let args: ListNeedsReviewArgs =
                serde_json::from_value(args_value).map_err(invalid_params)?;
            let items = repository
                .list_needs_review_for_agent(
                    agent,
                    args.project_slug.as_deref(),
                    args.limit.unwrap_or(100),
                )
                .await
                .map_err(needs_review_error)?;
            Ok(tool_success(json!({
                "items": items.into_iter().map(needs_review_item_json).collect::<Vec<_>>()
            })))
        }
        "promote_knowledge" => {
            let args: NeedsReviewItemArgs =
                serde_json::from_value(params.arguments).map_err(invalid_params)?;
            let Some(record) = repository
                .promote_needs_review_for_agent(agent, args.knowledge_item_id)
                .await
                .map_err(needs_review_error)?
            else {
                return Ok(tool_error("knowledge_item_not_found"));
            };
            Ok(tool_success(needs_review_action_json(record)))
        }
        "reject_needs_review" => {
            let args: NeedsReviewItemArgs =
                serde_json::from_value(params.arguments).map_err(invalid_params)?;
            let Some(record) = repository
                .reject_needs_review_for_agent(agent, args.knowledge_item_id, args.reason)
                .await
                .map_err(needs_review_error)?
            else {
                return Ok(tool_error("knowledge_item_not_found"));
            };
            Ok(tool_success(needs_review_action_json(record)))
        }
        _ => Err("unknown_tool".to_owned()),
    }
}

async fn hybrid_retrieve(
    state: &McpState,
    agent: &AuthenticatedAgentToken,
    request: RetrieveContextRequest,
) -> Result<RetrieveContextResponse, String> {
    request.validate().map_err(|error| error.to_string())?;
    let service = state
        .retrieval
        .as_ref()
        .ok_or_else(|| "knowledge_store_not_configured".to_owned())?;
    service
        .retrieve_context(
            &RetrievalPrincipal::Agent {
                organization_id: agent.organization_id,
                project_slugs: agent.permissions.project_slugs.clone(),
                allow_global_knowledge: agent.permissions.allow_global_knowledge,
            },
            request,
        )
        .await
        .map_err(infrastructure_error)
}

impl ProposeMemoryArgs {
    fn into_params(self, max_body_bytes: usize) -> Result<ProposeMemoryParams, String> {
        let project_slug = self.project_slug.trim().to_owned();
        let title = self.title.trim().to_owned();
        let body = validate_memory_body(&self.body, max_body_bytes).map_err(validation_error)?;
        let category = self.category.trim().to_owned();
        let tags = self
            .tags
            .into_iter()
            .map(|tag| tag.trim().to_owned())
            .filter(|tag| !tag.is_empty())
            .collect::<Vec<_>>();

        if !valid_slug(&project_slug) {
            return Err("invalid_project_slug".to_owned());
        }
        if title.is_empty() || title.len() > 256 {
            return Err("invalid_title".to_owned());
        }
        if category.is_empty() || category.len() > 64 {
            return Err("invalid_category".to_owned());
        }
        if tags.len() > 25 || tags.iter().any(|tag| tag.len() > 64) {
            return Err("invalid_tags".to_owned());
        }

        Ok(ProposeMemoryParams {
            project_slug,
            title,
            body,
            category,
            tags,
        })
    }
}

impl IndexMemoryArgs {
    /// Validate + normalize index_memory arguments (IMP-13/22/23).
    ///
    /// Rejects global scope (project selector required; no scope override field).
    fn into_params(self, max_body_bytes: usize) -> Result<IndexMemoryParams, String> {
        let body = validate_memory_body(&self.body, max_body_bytes).map_err(validation_error)?;
        let project_slug = self
            .project_slug
            .map(|slug| slug.trim().to_owned())
            .filter(|slug| !slug.is_empty());
        if self.project_id.is_none() && project_slug.is_none() {
            return Err("invalid_project".to_owned());
        }
        if let Some(ref slug) = project_slug
            && !valid_slug(slug)
        {
            return Err("invalid_project_slug".to_owned());
        }

        let title = self
            .title
            .as_ref()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| {
                let truncated: String = body.chars().take(80).collect();
                if truncated.is_empty() {
                    "scratch".to_owned()
                } else {
                    truncated
                }
            });
        if title.len() > 256 {
            return Err("invalid_title".to_owned());
        }

        let category = self
            .category
            .as_ref()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "scratch".to_owned());
        if category.len() > 64 {
            return Err("invalid_category".to_owned());
        }

        let tags = self
            .tags
            .into_iter()
            .map(|tag| tag.trim().to_owned())
            .filter(|tag| !tag.is_empty())
            .collect::<Vec<_>>();
        if tags.len() > 25 || tags.iter().any(|tag| tag.len() > 64) {
            return Err("invalid_tags".to_owned());
        }

        let content_hash = scratch_content_hash(&body);
        Ok(IndexMemoryParams {
            project_id: self.project_id.map(ProjectId::as_uuid),
            project_slug,
            title,
            body,
            category,
            tags,
            content_hash,
        })
    }
}

async fn require_agent_token(
    repository: &PgProjectRepository,
    headers: &HeaderMap,
) -> Result<AuthenticatedAgentToken, StatusCode> {
    let raw_token = bearer_token(headers).ok_or(StatusCode::UNAUTHORIZED)?;
    let token_hash = AgentTokenIssuer::hash_token(raw_token);
    repository
        .authenticate_agent_token(&token_hash)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "agent token authentication failed");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::UNAUTHORIZED)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| token.starts_with("qria_"))
}

fn initialize_result() -> Value {
    json!({
        "protocolVersion": "2025-11-25",
        "capabilities": {
            "tools": { "listChanged": false }
        },
        "serverInfo": {
            "name": "queria",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

fn tool_success(value: Value) -> Value {
    let text = serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_owned());
    json!({
        "content": [{ "type": "text", "text": text }],
        "structuredContent": value,
        "isError": false
    })
}

fn tool_error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true
    })
}

fn json_rpc_result(id: Option<Value>, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result
    })
}

fn json_rpc_error(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message
        }
    })
}

fn project_json(project: ProjectRecord) -> Value {
    json!({
        "id": project.id,
        "slug": project.slug,
        "name": project.name,
        "description": project.description,
        "default_embedding_model": project.default_embedding_model,
        "include_global_default": project.include_global_default,
        "created_at": project.created_at,
        "updated_at": project.updated_at
    })
}

fn source_json(source: SourceDocumentRecord) -> Value {
    json!({
        "id": source.id,
        "project_id": source.project_id,
        "kind": source.kind,
        "uri": source.uri,
        "title": source.title,
        "source_path": source.source_path,
        "branch": source.branch,
        "commit_sha": source.commit_sha,
        "content_hash": source.content_hash,
        "metadata": source.metadata,
        "created_at": source.created_at,
        "updated_at": source.updated_at
    })
}

fn needs_review_item_json(item: NeedsReviewItemRecord) -> Value {
    json!({
        "knowledge_item_id": item.knowledge_item_id,
        "project_id": item.project_id,
        "project_slug": item.project_slug,
        "source_document_id": item.source_document_id,
        "title": item.title,
        "path": item.path,
        "origin_url": item.origin_url,
        "commit_sha": item.commit_sha,
        "branch": item.branch,
        "category": item.category,
        "created_at": item.created_at,
        "updated_at": item.updated_at
    })
}

fn knowledge_item_json(item: KnowledgeItemRecord) -> Value {
    json!({
        "id": item.id,
        "project_id": item.project_id,
        "source_document_id": item.source_document_id,
        "scope": item.scope,
        "status": item.status,
        "title": item.title,
        "body": item.body,
        "category": item.category,
        "tags": item.tags,
        "approved_at": item.approved_at,
        "created_at": item.created_at,
        "updated_at": item.updated_at
    })
}

fn needs_review_action_json(record: NeedsReviewActionRecord) -> Value {
    json!({
        "knowledge_item": knowledge_item_json(record.knowledge_item),
        "chunk_ids": record.chunk_ids
    })
}

fn invalid_params(error: serde_json::Error) -> String {
    format!("invalid_params: {error}")
}

fn validation_error(error: queria_core::QueriaError) -> String {
    match error {
        queria_core::QueriaError::Validation(message) => message,
        other => other.to_string(),
    }
}

fn infrastructure_error(error: queria_core::QueriaError) -> String {
    tracing::error!(error = %error, "mcp tool failed");
    "tool_failed".to_owned()
}

fn index_memory_error(error: queria_core::QueriaError) -> String {
    match error {
        queria_core::QueriaError::Validation(message) => message,
        queria_core::QueriaError::PermissionDenied => "permission_denied".to_owned(),
        queria_core::QueriaError::NotFound(message) => message,
        other => {
            tracing::error!(error = %other, "index_memory failed");
            // Surface embed/Qdrant failures without claiming success (VAL-DL-032).
            "index_memory_embed_failed".to_owned()
        }
    }
}

fn needs_review_error(error: queria_core::QueriaError) -> String {
    match error {
        queria_core::QueriaError::Validation(message) => message,
        queria_core::QueriaError::PermissionDenied => "permission_denied".to_owned(),
        queria_core::QueriaError::NotFound(message) => message,
        other => {
            tracing::error!(error = %other, "needs_review tool failed");
            "tool_failed".to_owned()
        }
    }
}

fn valid_slug(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    let Some(last) = bytes.last() else {
        return false;
    };

    (3..=64).contains(&bytes.len())
        && first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_propose(body: &str) -> ProposeMemoryArgs {
        ProposeMemoryArgs {
            project_slug: "fjulian-me".to_owned(),
            title: "note".to_owned(),
            body: body.to_owned(),
            category: "scratch".to_owned(),
            tags: vec![],
        }
    }

    /// VAL-DL-025 / VAL-DL-022: propose_memory shares configured max (not hard-only 20k).
    #[test]
    fn propose_memory_rejects_body_over_configured_max() {
        let max = 64usize;
        let args = valid_propose(&"x".repeat(max + 1));
        let err = args.into_params(max).expect_err("oversized body");
        assert!(
            err.starts_with("body_too_large"),
            "clear size error expected, got {err}"
        );
    }

    /// VAL-DL-023: body under/equal max accepted for propose_memory validation.
    #[test]
    fn propose_memory_accepts_body_at_configured_max() {
        let max = 128usize;
        let body = "a".repeat(max);
        let params = valid_propose(&body)
            .into_params(max)
            .expect("body at max should pass");
        assert_eq!(params.body.len(), max);
    }

    /// VAL-DL-024: empty / blank body rejected on propose path.
    #[test]
    fn propose_memory_rejects_empty_body() {
        let err = valid_propose("   ")
            .into_params(20_000)
            .expect_err("blank body");
        assert_eq!(err, "invalid_body");
    }

    /// VAL-DL-022/024: index_memory args use the same validate_memory_body helper.
    #[test]
    fn index_memory_body_validation_matches_shared_helper() {
        let max = 32usize;
        assert!(validate_memory_body("", max).is_err());
        assert!(validate_memory_body("  ", max).is_err());
        assert!(validate_memory_body(&"y".repeat(max + 1), max).is_err());
        assert!(validate_memory_body(&"y".repeat(max), max).is_ok());
        assert!(validate_memory_body("mission-dl-ok", max).is_ok());
    }

    /// IMP-23: default config max is 20_000 and applied to propose path.
    #[test]
    fn default_max_body_bytes_is_twenty_thousand() {
        let config = queria_core::AppConfig::default_local();
        assert_eq!(config.max_body_bytes, 20_000);
        let ok = valid_propose(&"n".repeat(20_000))
            .into_params(config.max_body_bytes)
            .expect("exactly 20k ok");
        assert_eq!(ok.body.len(), 20_000);
        let err = valid_propose(&"n".repeat(20_001))
            .into_params(config.max_body_bytes)
            .expect_err("20_001 over default");
        assert!(err.starts_with("body_too_large"));
    }

    fn valid_index(body: &str) -> IndexMemoryArgs {
        IndexMemoryArgs {
            project_id: None,
            project_slug: Some("fjulian-me".to_owned()),
            body: body.to_owned(),
            title: Some("scratch note".to_owned()),
            category: Some("note".to_owned()),
            tags: vec!["mission-dl".to_owned()],
        }
    }

    /// VAL-DL-007/013: project selector required; no global default.
    #[test]
    fn index_memory_requires_project_selector() {
        let args = IndexMemoryArgs {
            project_id: None,
            project_slug: None,
            body: "mission-dl-body".to_owned(),
            title: None,
            category: None,
            tags: vec![],
        };
        let err = args.into_params(20_000).expect_err("must require project");
        assert_eq!(err, "invalid_project");
    }

    /// VAL-DL-018/019: content_hash is stable after normalize.
    #[test]
    fn index_memory_params_hash_is_idempotent_for_whitespace() {
        let a = valid_index("hello   world")
            .into_params(20_000)
            .expect("ok");
        let b = IndexMemoryArgs {
            project_id: None,
            project_slug: Some("fjulian-me".to_owned()),
            body: "  hello world  ".to_owned(),
            title: None,
            category: None,
            tags: vec![],
        }
        .into_params(20_000)
        .expect("ok");
        assert_eq!(a.content_hash, b.content_hash);
        assert_eq!(a.content_hash, scratch_content_hash("hello world"));
    }

    /// VAL-DL-042: optional title/category defaults; oversized tags fail.
    #[test]
    fn index_memory_rejects_absurd_tag_counts() {
        let mut args = valid_index("body");
        args.tags = (0..30).map(|i| format!("tag{i}")).collect();
        let err = args.into_params(20_000).expect_err("too many tags");
        assert_eq!(err, "invalid_tags");
    }

    #[test]
    fn index_memory_accepts_slug_only_and_defaults_title() {
        let params = IndexMemoryArgs {
            project_id: None,
            project_slug: Some("fjulian-me".to_owned()),
            body: "mission-dl-unique-marker-xyz".to_owned(),
            title: None,
            category: None,
            tags: vec![],
        }
        .into_params(20_000)
        .expect("valid");
        assert_eq!(params.project_slug.as_deref(), Some("fjulian-me"));
        assert_eq!(params.category, "scratch");
        assert!(!params.title.is_empty());
        assert_eq!(params.content_hash.len(), 64);
        // No global scope field exists; writes are always project-scoped.
        assert!(params.project_id.is_none() || params.project_slug.is_some());
    }

    /// VAL-DL-022: oversize body rejected on index_memory params.
    #[test]
    fn index_memory_rejects_oversized_body() {
        let max = 40usize;
        let err = valid_index(&"x".repeat(max + 1))
            .into_params(max)
            .expect_err("oversize");
        assert!(err.starts_with("body_too_large"));
    }

    /// VAL-DL-034: index_memory schema/args never accept a knowledge id mutate target.
    #[test]
    fn index_memory_args_have_no_trusted_id_mutate_field() {
        // Round-trip through JSON without knowledge_item_id; extra field must not bind.
        let value = json!({
            "project_slug": "fjulian-me",
            "body": "mission-dl-no-mutate",
            "knowledge_item_id": "00000000-0000-0000-0000-000000000001",
            "id": "00000000-0000-0000-0000-000000000001"
        });
        let args: IndexMemoryArgs = serde_json::from_value(value).expect("deserialize");
        let params = args
            .into_params(20_000)
            .expect("valid without mutate fields");
        // Only body/title/hash project selectors exist; no approved id carry-through.
        assert!(params.project_id.is_none());
        assert_eq!(params.project_slug.as_deref(), Some("fjulian-me"));
        assert_eq!(params.body, "mission-dl-no-mutate");
    }

    /// VAL-DL-032: infrastructure embed failures map to non-success tool error string.
    #[test]
    fn index_memory_error_maps_infrastructure_to_embed_failed() {
        let msg = index_memory_error(queria_core::QueriaError::Infrastructure(
            "voyage down".to_owned(),
        ));
        assert_eq!(msg, "index_memory_embed_failed");
        assert_ne!(msg, "permission_denied");
        let validation = index_memory_error(queria_core::QueriaError::Validation(
            "invalid_body".to_owned(),
        ));
        assert_eq!(validation, "invalid_body");
    }

    /// VAL-DL-042: valid optional tags accepted within bounds.
    #[test]
    fn index_memory_accepts_valid_optional_tags() {
        let mut args = valid_index("mission-dl-tags-ok");
        args.tags = vec!["a".to_owned(), "b".to_owned()];
        args.title = Some("t".to_owned());
        args.category = Some("note".to_owned());
        let params = args.into_params(20_000).expect("valid tags");
        assert_eq!(params.tags, vec!["a".to_owned(), "b".to_owned()]);
        assert_eq!(params.title, "t");
        assert_eq!(params.category, "note");
    }

    /// VAL-CROSS-004: omit include_scratch → agent default true.
    #[test]
    fn retrieval_args_default_include_scratch_true() {
        let args: RetrievalArgs = serde_json::from_str(
            r#"{"project_id":"019083a0-0000-7000-8000-000000000001","query":"hello"}"#,
        )
        .expect("minimal retrieval args");
        assert!(args.include_scratch.unwrap_or(true));
        assert!(!args.include_needs_review.unwrap_or(false));
        assert!(args.rerank.is_none());
        assert!(args.compress.is_none());
    }

    /// IMP-L3: omit include_needs_review → default false on MCP args.
    #[test]
    fn retrieval_args_default_include_needs_review_false() {
        let args: RetrievalArgs = serde_json::from_str(
            r#"{"project_id":"019083a0-0000-7000-8000-000000000001","query":"hello"}"#,
        )
        .expect("minimal retrieval args");
        assert!(!args.include_needs_review.unwrap_or(false));
    }

    /// VAL-CROSS-001/002: MCP tools accept optional rerank/compress overrides.
    #[test]
    fn retrieval_args_accept_flag_overrides() {
        let args: RetrievalArgs = serde_json::from_str(
            r#"{
                "project_id":"019083a0-0000-7000-8000-000000000001",
                "query":"hello",
                "include_scratch": false,
                "include_needs_review": true,
                "rerank": false,
                "compress": true
            }"#,
        )
        .expect("retrieval args with flags");
        assert_eq!(args.include_scratch, Some(false));
        assert_eq!(args.include_needs_review, Some(true));
        assert_eq!(args.rerank, Some(false));
        assert_eq!(args.compress, Some(true));
    }
}

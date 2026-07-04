use crate::server::McpState;
use crate::tools;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use queria_auth::agent_token::AgentTokenIssuer;
use queria_core::contracts::{RetrieveContextRequest, RetrieveContextResponse};
use queria_core::ids::{ProjectId, SourceDocumentId};
use queria_db::repositories::{
    AuthenticatedAgentToken, PgProjectRepository, ProjectRecord, ProposeMemoryParams,
    SourceDocumentRecord,
};
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
    limit: Option<u32>,
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
        "tools/call" => match call_tool(&repository, &agent, request.params).await {
            Ok(value) => json_rpc_result(request.id, value),
            Err(error) => json_rpc_error(request.id, -32602, &error),
        },
        _ => json_rpc_error(request.id, -32601, "method_not_found"),
    };

    (StatusCode::OK, Json(response))
}

async fn call_tool(
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
                limit: args.limit.unwrap_or(5),
            };
            request.validate().map_err(|error| error.to_string())?;
            let items = repository
                .search_approved_chunks_for_agent(
                    agent,
                    request.project_id,
                    &request.query,
                    request.include_global,
                    request.limit,
                )
                .await
                .map_err(infrastructure_error)?;
            Ok(tool_success(json!(RetrieveContextResponse {
                project_id: request.project_id,
                query: request.query,
                items,
                generated_at: chrono::Utc::now(),
            })))
        }
        "search_knowledge" => {
            let args: RetrievalArgs =
                serde_json::from_value(params.arguments).map_err(invalid_params)?;
            let request = RetrieveContextRequest {
                project_id: args.project_id,
                query: args.query,
                include_global: args.include_global.unwrap_or(true),
                limit: args.limit.unwrap_or(10),
            };
            request.validate().map_err(|error| error.to_string())?;
            let items = repository
                .search_approved_chunks_for_agent(
                    agent,
                    request.project_id,
                    &request.query,
                    request.include_global,
                    request.limit,
                )
                .await
                .map_err(infrastructure_error)?;
            Ok(tool_success(json!({
                "project_id": request.project_id,
                "query": request.query,
                "items": items
            })))
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
            let params = args.into_params()?;
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
        _ => Err("unknown_tool".to_owned()),
    }
}

impl ProposeMemoryArgs {
    fn into_params(self) -> Result<ProposeMemoryParams, String> {
        let project_slug = self.project_slug.trim().to_owned();
        let title = self.title.trim().to_owned();
        let body = self.body.trim().to_owned();
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
        if body.is_empty() || body.len() > 20_000 {
            return Err("invalid_body".to_owned());
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

fn invalid_params(error: serde_json::Error) -> String {
    format!("invalid_params: {error}")
}

fn infrastructure_error(error: queria_core::QueriaError) -> String {
    tracing::error!(error = %error, "mcp tool failed");
    "tool_failed".to_owned()
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

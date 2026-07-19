//! Agent-bearer local multi-git index-here upload.
//!
//! `POST /api/v1/agent/index-local` accepts rooted file batches from CLI,
//! re-applies quality gates, auto-creates projects, persists `needs_review`
//! knowledge/chunks, and enqueues embedding backfill jobs.

use crate::app::ApiState;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    routing::post,
};
use queria_core::auth::agent_token::AgentTokenIssuer;
use queria_core::auth::permissions::AgentToolPermission;
use queria_core::QueriaError;
use queria_db::repositories::{
    AuthenticatedAgentToken, IndexLocalFileParams, PgProjectRepository, ProjectRecord,
};
use queria_ingestion::local_index_gates::{
    content_hash, content_is_indexable, normalize_project_slug_from_origin, should_index_local_file,
    MAX_LOCAL_FILE_BYTES,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Hard limits for v1 (CLI splits larger batches).
const MAX_ROOTS: usize = 20;
const MAX_FILES_PER_REQUEST: usize = 500;

#[derive(Debug, Deserialize)]
struct IndexLocalRequest {
    roots: Vec<IndexLocalRoot>,
}

#[derive(Debug, Deserialize)]
struct IndexLocalRoot {
    #[serde(default)]
    origin_url: Option<String>,
    #[serde(default)]
    local_path_hint: Option<String>,
    #[serde(default)]
    commit_sha: Option<String>,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    files: Vec<IndexLocalFile>,
}

#[derive(Debug, Deserialize)]
struct IndexLocalFile {
    path: String,
    content: String,
    #[serde(default)]
    content_hash: Option<String>,
}

#[derive(Debug, Serialize)]
struct IndexLocalResponse {
    job_ids: Vec<String>,
    roots: Vec<RootStats>,
}

#[derive(Debug, Serialize)]
struct RootStats {
    project_slug: String,
    project_id: String,
    files_accepted: u32,
    files_skipped: u32,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

type ApiResult<T> = Result<(StatusCode, Json<T>), (StatusCode, Json<ErrorResponse>)>;

pub fn router() -> Router<ApiState> {
    Router::new().route("/agent/index-local", post(agent_index_local))
}

async fn agent_index_local(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(payload): Json<IndexLocalRequest>,
) -> ApiResult<IndexLocalResponse> {
    let raw = require_raw_bearer(&headers)?;
    let repository = project_repository(&state)?;
    let mut agent = authenticate_raw(&repository, raw).await?;
    if !agent.permissions.can_call(&AgentToolPermission::IndexLocal) {
        return Err(error(StatusCode::FORBIDDEN, "permission_denied"));
    }

    if payload.roots.is_empty() {
        return Err(error(StatusCode::BAD_REQUEST, "roots_required"));
    }
    if payload.roots.len() > MAX_ROOTS {
        return Err(error(StatusCode::BAD_REQUEST, "too_many_roots"));
    }
    let total_files: usize = payload.roots.iter().map(|r| r.files.len()).sum();
    if total_files > MAX_FILES_PER_REQUEST {
        return Err(error(StatusCode::BAD_REQUEST, "too_many_files"));
    }

    let profile_version = state.config.embedding.profile_version.clone();
    let mut job_ids: Vec<String> = Vec::new();
    let mut root_stats: Vec<RootStats> = Vec::with_capacity(payload.roots.len());

    for root in &payload.roots {
        let basename = basename_from_hint(root.local_path_hint.as_deref());
        let origin = root
            .origin_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let mut slug =
            normalize_project_slug_from_origin(origin, &basename);
        slug = ensure_db_slug(&slug);

        let project = resolve_or_create_project(
            &repository,
            &agent,
            &slug,
            origin,
            root.local_path_hint.as_deref(),
        )
        .await?;

        // Attach to token allowlist so list_projects/retrieve work after auto-create.
        if !agent.permissions.project_slugs.iter().any(|s| s == &project.slug) {
            repository
                .attach_project_slug_to_agent_token(agent.id, &project.slug)
                .await
                .map_err(map_infra)?;
            agent.permissions.project_slugs.push(project.slug.clone());
        }

        let mut accepted = 0_u32;
        let mut skipped = 0_u32;

        for file in &root.files {
            match gate_and_hash_file(file) {
                Ok((path, body, hash)) => {
                    let params = IndexLocalFileParams {
                        project_id: project.id,
                        path,
                        body,
                        content_hash: hash,
                        origin_url: origin.map(str::to_owned),
                        commit_sha: root.commit_sha.clone(),
                        branch: root.branch.clone(),
                        local_path_hint: root.local_path_hint.clone(),
                    };
                    match repository.index_local_file_for_agent(&agent, params).await {
                        Ok(_) => accepted += 1,
                        Err(err) => {
                            tracing::warn!(error = %err, "index_local file persist skipped");
                            skipped += 1;
                        }
                    }
                }
                Err(_) => skipped += 1,
            }
        }

        if accepted > 0 {
            if let Some(job_id) = repository
                .enqueue_embedding_backfill_for_agent(&agent, project.id, &profile_version)
                .await
                .map_err(map_infra)?
            {
                let id = job_id.to_string();
                if !job_ids.contains(&id) {
                    job_ids.push(id);
                }
            }
        }

        root_stats.push(RootStats {
            project_slug: project.slug,
            project_id: project.id.to_string(),
            files_accepted: accepted,
            files_skipped: skipped,
        });
    }

    Ok((
        StatusCode::ACCEPTED,
        Json(IndexLocalResponse {
            job_ids,
            roots: root_stats,
        }),
    ))
}

fn gate_and_hash_file(file: &IndexLocalFile) -> Result<(String, String, String), &'static str> {
    let path = file.path.trim().to_owned();
    if path.is_empty() {
        return Err("empty_path");
    }
    let size = file.content.len() as u64;
    if !should_index_local_file(&path, size) {
        return Err("gate_denied");
    }
    if !content_is_indexable(&file.content) {
        return Err("empty_content");
    }
    if size > MAX_LOCAL_FILE_BYTES {
        return Err("too_large");
    }
    let computed = content_hash(&file.content);
    if let Some(client_hash) = file
        .content_hash
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if client_hash != computed {
            return Err("content_hash_mismatch");
        }
        return Ok((path, file.content.clone(), client_hash.to_owned()));
    }
    Ok((path, file.content.clone(), computed))
}

async fn resolve_or_create_project(
    repository: &PgProjectRepository,
    agent: &AuthenticatedAgentToken,
    slug: &str,
    origin: Option<&str>,
    local_path_hint: Option<&str>,
) -> Result<ProjectRecord, (StatusCode, Json<ErrorResponse>)> {
    // Origin identity wins (re-run same remote → same project).
    if let Some(origin_url) = origin {
        if let Some(project) = repository
            .find_project_by_origin_in_org(agent.organization_id, origin_url)
            .await
            .map_err(map_infra)?
        {
            return Ok(project);
        }
    }

    // No origin match: reuse free slug, or same-slug when no origin on request.
    // If slug taken and we have a new origin, allocate slug-2… per plan.
    if let Some(project) = repository
        .find_project_by_slug_in_org(agent.organization_id, slug)
        .await
        .map_err(map_infra)?
    {
        if origin.is_none() {
            return Ok(project);
        }
        // Different origin identity than existing slug holder → slug-N.
        let mut n = 2_u32;
        loop {
            let candidate = ensure_db_slug(&format!("{slug}-{n}"));
            if repository
                .find_project_by_slug_in_org(agent.organization_id, &candidate)
                .await
                .map_err(map_infra)?
                .is_none()
            {
                let name = display_name(local_path_hint, origin, &candidate);
                return repository
                    .create_project_for_agent(agent, &candidate, &name)
                    .await
                    .map_err(map_infra);
            }
            n += 1;
            if n > 100 {
                return Err(error(StatusCode::CONFLICT, "project_slug_exhausted"));
            }
        }
    }

    let name = display_name(local_path_hint, origin, slug);
    repository
        .create_project_for_agent(agent, slug, &name)
        .await
        .map_err(map_infra)
}

fn display_name(local_path_hint: Option<&str>, origin: Option<&str>, fallback: &str) -> String {
    local_path_hint
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(origin)
        .unwrap_or(fallback)
        .to_owned()
}

fn basename_from_hint(hint: Option<&str>) -> String {
    let raw = hint.unwrap_or("repo").trim();
    if raw.is_empty() {
        return "repo".to_owned();
    }
    Path::new(raw)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(raw)
        .to_owned()
}

/// Ensure slug satisfies DB check `^[a-z0-9][a-z0-9-]{1,62}[a-z0-9]$` (3–64).
fn ensure_db_slug(slug: &str) -> String {
    let mut s = slug.to_owned();
    if s.is_empty() {
        s = "repo".to_owned();
    }
    if s.len() > 64 {
        s.truncate(64);
        s = s.trim_end_matches('-').to_owned();
        if s.is_empty() {
            s = "repo".to_owned();
        }
    }
    while s.len() < 3 {
        s.push('x');
    }
    // Ensure first/last alnum (pad path already alnum after sanitize).
    let bytes = s.as_bytes();
    if !bytes[0].is_ascii_alphanumeric() || !bytes[bytes.len() - 1].is_ascii_alphanumeric() {
        s = format!("r-{s}-x");
        if s.len() > 64 {
            s.truncate(64);
        }
    }
    s
}

fn project_repository(
    state: &ApiState,
) -> Result<PgProjectRepository, (StatusCode, Json<ErrorResponse>)> {
    state.project_repository().ok_or_else(|| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "knowledge_store_not_configured",
        )
    })
}

fn require_raw_bearer(
    headers: &HeaderMap,
) -> Result<&str, (StatusCode, Json<ErrorResponse>)> {
    bearer_token(headers).ok_or_else(|| error(StatusCode::UNAUTHORIZED, "agent_token_required"))
}

async fn authenticate_raw(
    repository: &PgProjectRepository,
    raw: &str,
) -> Result<AuthenticatedAgentToken, (StatusCode, Json<ErrorResponse>)> {
    let token_hash = AgentTokenIssuer::hash_token(raw);
    repository
        .authenticate_agent_token(&token_hash)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "agent token authentication failed");
            error(StatusCode::INTERNAL_SERVER_ERROR, "repository_failed")
        })?
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "agent_token_required"))
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|token| token.starts_with("qria_"))
}

fn map_infra(err: QueriaError) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %err, "agent index-local repository failed");
    match err {
        QueriaError::Validation(message) => error(StatusCode::BAD_REQUEST, &message),
        QueriaError::PermissionDenied => error(StatusCode::FORBIDDEN, "permission_denied"),
        QueriaError::NotFound(message) => error(StatusCode::NOT_FOUND, &message),
        QueriaError::Authentication => error(StatusCode::UNAUTHORIZED, "agent_token_required"),
        QueriaError::Config(message) | QueriaError::Infrastructure(message) => {
            tracing::error!(error = %message, "index-local infra");
            error(StatusCode::INTERNAL_SERVER_ERROR, "repository_failed")
        }
    }
}

fn error(status: StatusCode, message: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        status,
        Json(ErrorResponse {
            error: message.to_owned(),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::build_app;
    use axum::body::Body;
    use http::Request;
    use queria_core::AppConfig;
    use tower::ServiceExt;

    async fn body_string(response: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        String::from_utf8(bytes.to_vec()).expect("utf8")
    }

    #[test]
    fn ensure_db_slug_pads_short() {
        assert_eq!(ensure_db_slug("z"), "zxx");
        assert_eq!(ensure_db_slug("ab"), "abx");
        assert_eq!(ensure_db_slug("app"), "app");
        assert_eq!(ensure_db_slug(""), "repo");
    }

    #[test]
    fn gate_accepts_md_and_hashes() {
        let file = IndexLocalFile {
            path: "docs/runbook.md".to_owned(),
            content: "hello world".to_owned(),
            content_hash: None,
        };
        let (path, body, hash) = gate_and_hash_file(&file).expect("gate");
        assert_eq!(path, "docs/runbook.md");
        assert_eq!(body, "hello world");
        assert_eq!(hash, content_hash("hello world"));
    }

    #[test]
    fn gate_rejects_mismatch_hash() {
        let file = IndexLocalFile {
            path: "docs/runbook.md".to_owned(),
            content: "hello".to_owned(),
            content_hash: Some("deadbeef".to_owned()),
        };
        assert_eq!(gate_and_hash_file(&file), Err("content_hash_mismatch"));
    }

    #[test]
    fn gate_rejects_denied_paths() {
        let file = IndexLocalFile {
            path: "node_modules/pkg/readme.md".to_owned(),
            content: "x".to_owned(),
            content_hash: None,
        };
        assert!(gate_and_hash_file(&file).is_err());
    }

    #[test]
    fn gate_rejects_empty_content() {
        let file = IndexLocalFile {
            path: "docs/a.md".to_owned(),
            content: "   \n".to_owned(),
            content_hash: None,
        };
        assert_eq!(gate_and_hash_file(&file), Err("empty_content"));
    }

    #[test]
    fn basename_from_nested_hint() {
        assert_eq!(basename_from_hint(Some("services/api")), "api");
        assert_eq!(basename_from_hint(None), "repo");
        assert_eq!(basename_from_hint(Some("")), "repo");
    }

    #[tokio::test]
    async fn index_local_requires_bearer() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/agent/index-local")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"roots":[]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let body = body_string(response).await;
        assert!(body.contains("agent_token_required"), "body={body}");
    }

    #[tokio::test]
    async fn index_local_rejects_non_qria_bearer() {
        let app = build_app(AppConfig::default_local());
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/agent/index-local")
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer not_a_queria_token")
                    .body(Body::from(
                        r#"{"roots":[{"origin_url":"git@h:a.git","files":[]}]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn request_deserializes() {
        let payload: IndexLocalRequest = serde_json::from_str(
            r#"{
              "roots": [{
                "origin_url": "git@selfhosted:team/api.git",
                "local_path_hint": "services/api",
                "commit_sha": "abc",
                "branch": "main",
                "files": [{"path": "src/main.ts", "content": "export {}", "content_hash": "h"}]
              }]
            }"#,
        )
        .expect("deserialize");
        assert_eq!(payload.roots.len(), 1);
        assert_eq!(payload.roots[0].files.len(), 1);
        assert_eq!(payload.roots[0].files[0].path, "src/main.ts");
    }

    #[test]
    fn response_serializes_202_shape() {
        let body = IndexLocalResponse {
            job_ids: vec!["11111111-1111-1111-1111-111111111111".to_owned()],
            roots: vec![RootStats {
                project_slug: "api".to_owned(),
                project_id: "22222222-2222-2222-2222-222222222222".to_owned(),
                files_accepted: 1,
                files_skipped: 0,
            }],
        };
        let v: Value = serde_json::to_value(&body).expect("ser");
        assert!(v.get("job_ids").is_some());
        assert!(v.get("roots").is_some());
        assert_eq!(v["roots"][0]["files_accepted"], 1);
    }
}

//! Voyage AI rerank stage (IMP-01 / architecture rerank).
//!
//! Concrete client only (no multi-provider trait). Call after hydrate on the
//! ranked pool, `top_k = request.limit`. Fail open on error/timeout/empty:
//! preserve RRF order and set `rerank_applied=false`.

use queria_core::contracts::RetrievedContextItem;
use queria_core::{QueriaError, QueriaResult};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://api.voyageai.com/v1";
const DEFAULT_MODEL: &str = "rerank-2.5";

/// Per-document body ceiling before sending to Voyage (chars, not tokens).
/// Voyage `rerank-2.5` allows large contexts; we still truncate extremely long
/// hydrated bodies to keep request size bounded. Truncation is applied to the
/// request text only; the item body returned to clients is unchanged.
const MAX_RERANK_DOC_CHARS: usize = 24_000;

/// One hit from Voyage `/v1/rerank` (or a test double).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RerankHit {
    pub index: usize,
    pub relevance_score: f32,
}

/// Result of the optional rerank stage.
#[derive(Clone, Debug)]
pub struct RerankOutcome {
    pub items: Vec<RetrievedContextItem>,
    pub applied: bool,
}

/// Resolve whether rerank should be attempted: request override or config default.
#[must_use]
pub fn resolve_rerank_enabled(request_flag: Option<bool>, config_default: bool) -> bool {
    request_flag.unwrap_or(config_default)
}

/// Reorder hydrated items by Voyage hit indices and truncate to `top_k`.
///
/// Hits are expected in descending relevance order. Out-of-range indices and
/// duplicates are skipped. Returns empty if no valid hit maps to an item.
#[must_use]
pub fn apply_rerank_hits(
    items: Vec<RetrievedContextItem>,
    hits: &[RerankHit],
    top_k: usize,
) -> Vec<RetrievedContextItem> {
    if items.is_empty() || hits.is_empty() || top_k == 0 {
        return items.into_iter().take(top_k).collect();
    }

    let mut used = vec![false; items.len()];
    let mut out = Vec::with_capacity(top_k.min(items.len()));

    for hit in hits {
        if out.len() >= top_k {
            break;
        }
        let Some(item) = items.get(hit.index) else {
            continue;
        };
        if used[hit.index] {
            continue;
        }
        used[hit.index] = true;
        let mut reordered = item.clone();
        reordered.score = hit.relevance_score;
        out.push(reordered);
    }

    out
}

/// Concrete Voyage rerank client (`POST /v1/rerank`, model default `rerank-2.5`).
#[derive(Clone)]
pub struct VoyageReranker {
    client: Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl VoyageReranker {
    /// Build a production client against `https://api.voyageai.com/v1`.
    pub fn new(api_key: String, model: String, timeout: Duration) -> QueriaResult<Self> {
        Self::with_base_url(DEFAULT_BASE_URL.to_owned(), api_key, model, timeout)
    }

    /// Build against an arbitrary base URL (used by unit tests with a fake HTTP server).
    pub fn with_base_url(
        base_url: String,
        api_key: String,
        model: String,
        timeout: Duration,
    ) -> QueriaResult<Self> {
        if api_key.trim().is_empty() {
            return Err(QueriaError::Config(
                "Voyage API key is required for rerank".to_owned(),
            ));
        }
        let model = if model.trim().is_empty() {
            DEFAULT_MODEL.to_owned()
        } else {
            model
        };
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|error| {
                QueriaError::Config(format!(
                    "failed to build Voyage rerank HTTP client: {error}"
                ))
            })?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
            model,
        })
    }

    /// Optional constructor: missing/blank key → `None` (fail-open path for retrieve).
    #[must_use]
    pub fn try_new(api_key: &str, model: &str, timeout: Duration) -> Option<Self> {
        if api_key.trim().is_empty() {
            return None;
        }
        Self::new(api_key.to_owned(), model.to_owned(), timeout).ok()
    }

    /// Call Voyage and return hits sorted by descending relevance (API guarantee).
    pub async fn rerank(
        &self,
        query: &str,
        documents: &[String],
        top_k: usize,
    ) -> QueriaResult<Vec<RerankHit>> {
        if documents.is_empty() || top_k == 0 {
            return Ok(Vec::new());
        }

        let top_k_u32 = u32::try_from(top_k).unwrap_or(u32::MAX);
        let request = VoyageRerankRequest {
            query: query.to_owned(),
            documents: documents.to_vec(),
            model: self.model.clone(),
            top_k: top_k_u32,
        };

        let response = self
            .client
            .post(format!("{}/rerank", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await
            .map_err(|error| {
                // Never include secrets; reqwest error strings do not embed the key.
                QueriaError::Infrastructure(format!("Voyage rerank request failed: {error}"))
            })?;

        let status = response.status();
        if !status.is_success() {
            let request_id = response
                .headers()
                .get("request-id")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("unavailable")
                .to_owned();
            // Drain body without surfacing it (may contain provider details).
            let _ = response.bytes().await;
            return Err(QueriaError::Infrastructure(format!(
                "Voyage rerank failed with status {status}; request_id={request_id}"
            )));
        }

        let payload: VoyageRerankResponse = response.json().await.map_err(|error| {
            QueriaError::Infrastructure(format!(
                "Voyage rerank returned an invalid response: {error}"
            ))
        })?;

        if payload.data.is_empty() {
            return Err(QueriaError::Infrastructure(
                "Voyage rerank returned empty data".to_owned(),
            ));
        }

        Ok(payload
            .data
            .into_iter()
            .map(|hit| RerankHit {
                index: hit.index,
                relevance_score: hit.relevance_score,
            })
            .collect())
    }
}

#[derive(Clone, Debug, Serialize)]
struct VoyageRerankRequest {
    query: String,
    documents: Vec<String>,
    model: String,
    top_k: u32,
}

#[derive(Debug, Deserialize)]
struct VoyageRerankResponse {
    data: Vec<VoyageRerankData>,
}

#[derive(Debug, Deserialize)]
struct VoyageRerankData {
    index: usize,
    relevance_score: f32,
}

/// Document texts sent to Voyage: hydrated body, truncated per ceiling.
#[must_use]
pub fn documents_for_rerank(items: &[RetrievedContextItem]) -> Vec<String> {
    items.iter().map(|item| truncate_doc(&item.body)).collect()
}

fn truncate_doc(body: &str) -> String {
    if body.chars().count() <= MAX_RERANK_DOC_CHARS {
        return body.to_owned();
    }
    body.chars().take(MAX_RERANK_DOC_CHARS).collect()
}

/// Run optional rerank with fail-open semantics.
///
/// - `enabled=false` or empty items → preserve order, truncate to `top_k`, `applied=false`
/// - `reranker=None` (missing key) → preserve order, truncate to `top_k`, `applied=false`
/// - client error / empty → preserve RRF order and scores, truncate to `top_k`, `applied=false`
/// - success → reorder by hits, `top_k`, `applied=true`
///
/// Always clamps final length to `top_k` (request limit) so an oversampled
/// hydrated pool cannot leak past the client limit when rerank is skipped.
pub async fn rerank_items(
    enabled: bool,
    reranker: Option<&VoyageReranker>,
    query: &str,
    items: Vec<RetrievedContextItem>,
    top_k: usize,
) -> RerankOutcome {
    if items.is_empty() {
        return RerankOutcome {
            items,
            applied: false,
        };
    }
    if !enabled || top_k == 0 {
        return RerankOutcome {
            items: take_top(items, top_k),
            applied: false,
        };
    }

    let Some(client) = reranker else {
        tracing::warn!("rerank desired but Voyage API key is not configured; keeping RRF order");
        return RerankOutcome {
            items: take_top(items, top_k),
            applied: false,
        };
    };

    let documents = documents_for_rerank(&items);
    match client.rerank(query, &documents, top_k).await {
        Ok(hits) if !hits.is_empty() => {
            let reordered = apply_rerank_hits(items.clone(), &hits, top_k);
            if reordered.is_empty() {
                // Defensive: no valid indices → keep RRF order, still clamp.
                tracing::warn!("voyage rerank hits mapped to no items; keeping RRF order");
                RerankOutcome {
                    items: take_top(items, top_k),
                    applied: false,
                }
            } else {
                RerankOutcome {
                    items: reordered,
                    applied: true,
                }
            }
        }
        Ok(_) => {
            tracing::warn!("voyage rerank returned no hits; keeping RRF order");
            RerankOutcome {
                items: take_top(items, top_k),
                applied: false,
            }
        }
        Err(error) => {
            tracing::warn!(
                error = %sanitized_rerank_error(&error),
                "voyage rerank failed; keeping RRF order"
            );
            RerankOutcome {
                items: take_top(items, top_k),
                applied: false,
            }
        }
    }
}

fn take_top(items: Vec<RetrievedContextItem>, top_k: usize) -> Vec<RetrievedContextItem> {
    items.into_iter().take(top_k).collect()
}

fn sanitized_rerank_error(error: &QueriaError) -> String {
    match error {
        QueriaError::Infrastructure(_) => "provider_unavailable".to_owned(),
        QueriaError::Config(_) => "config_error".to_owned(),
        _ => error.to_string().chars().take(120).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use queria_core::contracts::{Citation, KnowledgeLane};
    use queria_core::ids::{ChunkId, SourceDocumentId};
    use queria_core::model::{KnowledgeScope, KnowledgeStatus};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use std::time::Duration;

    fn item(body: &str, score: f32) -> RetrievedContextItem {
        RetrievedContextItem {
            chunk_id: ChunkId::new(),
            source_document_id: SourceDocumentId::new(),
            scope: KnowledgeScope::Project,
            status: KnowledgeStatus::Approved,
            lane: KnowledgeLane::Trusted,
            title: "t".to_owned(),
            body: body.to_owned(),
            citation: Citation {
                source_uri: "git://repo/doc.md".to_owned(),
                source_path: Some("doc.md".to_owned()),
                line_start: Some(1),
                line_end: Some(2),
            },
            score,
        }
    }

    /// Minimal one-shot HTTP/1.1 JSON server for Voyage rerank fakes.
    fn spawn_json_server(status_line: &str, body: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        let status = status_line.to_owned();
        let body = body.to_owned();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0_u8; 8192];
                let _ = stream.read(&mut buf);
                let response = format!(
                    "{status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        format!("http://{addr}")
    }

    /// VAL-RET-005 / resolve helper.
    #[test]
    fn resolve_omitted_uses_config_default() {
        assert!(resolve_rerank_enabled(None, true));
        assert!(!resolve_rerank_enabled(None, false));
    }

    /// VAL-RET-006: explicit overrides both directions.
    #[test]
    fn resolve_explicit_overrides() {
        assert!(resolve_rerank_enabled(Some(true), false));
        assert!(!resolve_rerank_enabled(Some(false), true));
    }

    /// VAL-RET-018: successful hits reorder vs input (RRF) order and set scores.
    #[test]
    fn apply_hits_reorders_and_sets_scores() {
        let a = item("alpha", 0.3);
        let b = item("beta", 0.2);
        let c = item("gamma", 0.1);
        let a_id = a.chunk_id;
        let b_id = b.chunk_id;
        let c_id = c.chunk_id;
        // RRF order a,b,c — Voyage prefers c, then a, then b.
        let hits = [
            RerankHit {
                index: 2,
                relevance_score: 0.99,
            },
            RerankHit {
                index: 0,
                relevance_score: 0.80,
            },
            RerankHit {
                index: 1,
                relevance_score: 0.10,
            },
        ];
        let out = apply_rerank_hits(vec![a, b, c], &hits, 3);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].chunk_id, c_id);
        assert_eq!(out[1].chunk_id, a_id);
        assert_eq!(out[2].chunk_id, b_id);
        assert!((out[0].score - 0.99).abs() < f32::EPSILON);
        assert!((out[1].score - 0.80).abs() < f32::EPSILON);
    }

    /// top_k truncates after rerank order.
    #[test]
    fn apply_hits_respects_top_k() {
        let a = item("a", 0.3);
        let b = item("b", 0.2);
        let c = item("c", 0.1);
        let c_id = c.chunk_id;
        let hits = [
            RerankHit {
                index: 2,
                relevance_score: 0.9,
            },
            RerankHit {
                index: 0,
                relevance_score: 0.5,
            },
            RerankHit {
                index: 1,
                relevance_score: 0.1,
            },
        ];
        let out = apply_rerank_hits(vec![a, b, c], &hits, 1);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].chunk_id, c_id);
    }

    /// VAL-RET-008: missing key does not hard-fail; RRF order kept.
    #[tokio::test]
    async fn missing_key_fail_open_preserves_order() {
        let a = item("first", 1.0);
        let b = item("second", 0.5);
        let a_id = a.chunk_id;
        let b_id = b.chunk_id;
        let outcome = rerank_items(true, None, "q", vec![a, b], 2).await;
        assert!(!outcome.applied);
        assert_eq!(outcome.items.len(), 2);
        assert_eq!(outcome.items[0].chunk_id, a_id);
        assert_eq!(outcome.items[1].chunk_id, b_id);
        assert!((outcome.items[0].score - 1.0).abs() < f32::EPSILON);
    }

    /// Oversampled pool + fail-open still clamps to top_k (request limit).
    #[tokio::test]
    async fn fail_open_clamps_oversampled_pool_to_top_k() {
        let items: Vec<_> = (0..8)
            .map(|i| item(&format!("doc{i}"), 1.0 - i as f32 * 0.1))
            .collect();
        let first_id = items[0].chunk_id;
        let second_id = items[1].chunk_id;
        let outcome = rerank_items(true, None, "q", items, 2).await;
        assert!(!outcome.applied);
        assert_eq!(outcome.items.len(), 2);
        assert_eq!(outcome.items[0].chunk_id, first_id);
        assert_eq!(outcome.items[1].chunk_id, second_id);
    }

    /// Disabled rerank also clamps to top_k.
    #[tokio::test]
    async fn disabled_clamps_oversampled_pool_to_top_k() {
        let items: Vec<_> = (0..6).map(|i| item(&format!("x{i}"), 0.5)).collect();
        let outcome = rerank_items(false, None, "q", items, 3).await;
        assert!(!outcome.applied);
        assert_eq!(outcome.items.len(), 3);
    }

    /// VAL-RET-006: rerank=false skips even if client present.
    #[tokio::test]
    async fn disabled_flag_skips_rerank() {
        // Client would work, but flag is off.
        let base = spawn_json_server(
            "HTTP/1.1 200 OK",
            r#"{"data":[{"index":1,"relevance_score":0.9},{"index":0,"relevance_score":0.1}]}"#,
        );
        let client = VoyageReranker::with_base_url(
            base,
            "test-key".to_owned(),
            "rerank-2.5".to_owned(),
            Duration::from_secs(2),
        )
        .expect("client");
        let a = item("a", 0.9);
        let b = item("b", 0.1);
        let a_id = a.chunk_id;
        let outcome = rerank_items(false, Some(&client), "q", vec![a, b], 2).await;
        assert!(!outcome.applied);
        assert_eq!(outcome.items[0].chunk_id, a_id);
    }

    /// VAL-RET-007 / VAL-CROSS-008: HTTP failure keeps RRF order and scores.
    #[tokio::test]
    async fn http_error_fail_open_preserves_rrf() {
        let base = spawn_json_server("HTTP/1.1 500 Internal Server Error", r#"{"error":"boom"}"#);
        let client = VoyageReranker::with_base_url(
            base,
            "test-key".to_owned(),
            "rerank-2.5".to_owned(),
            Duration::from_secs(2),
        )
        .expect("client");
        let a = item("first-rrf", 0.8);
        let b = item("second-rrf", 0.4);
        let a_id = a.chunk_id;
        let b_id = b.chunk_id;
        let outcome = rerank_items(true, Some(&client), "query", vec![a, b], 2).await;
        assert!(!outcome.applied);
        assert_eq!(outcome.items[0].chunk_id, a_id);
        assert_eq!(outcome.items[1].chunk_id, b_id);
        assert!((outcome.items[0].score - 0.8).abs() < f32::EPSILON);
        assert!((outcome.items[1].score - 0.4).abs() < f32::EPSILON);
    }

    /// VAL-RET-018 + client: successful fake HTTP reorders and sets applied=true.
    #[tokio::test]
    async fn successful_fake_rerank_reorders() {
        let base = spawn_json_server(
            "HTTP/1.1 200 OK",
            r#"{"object":"list","data":[{"index":2,"relevance_score":0.97},{"index":0,"relevance_score":0.55},{"index":1,"relevance_score":0.12}],"model":"rerank-2.5"}"#,
        );
        let client = VoyageReranker::with_base_url(
            base,
            "test-key".to_owned(),
            "rerank-2.5".to_owned(),
            Duration::from_secs(2),
        )
        .expect("client");
        let a = item("rrf-1", 0.3);
        let b = item("rrf-2", 0.2);
        let c = item("rrf-3", 0.1);
        let a_id = a.chunk_id;
        let c_id = c.chunk_id;
        let outcome = rerank_items(true, Some(&client), "relevance order", vec![a, b, c], 2).await;
        assert!(outcome.applied);
        assert_eq!(outcome.items.len(), 2);
        assert_eq!(outcome.items[0].chunk_id, c_id);
        assert_eq!(outcome.items[1].chunk_id, a_id);
        assert!((outcome.items[0].score - 0.97).abs() < f32::EPSILON);
    }

    /// VAL-RET-005: config default on with successful client applies.
    #[tokio::test]
    async fn default_on_applies_when_client_succeeds() {
        let base = spawn_json_server(
            "HTTP/1.1 200 OK",
            r#"{"data":[{"index":0,"relevance_score":0.5}]}"#,
        );
        let client = VoyageReranker::with_base_url(
            base,
            "k".to_owned(),
            "rerank-2.5".to_owned(),
            Duration::from_secs(2),
        )
        .expect("client");
        let enabled = resolve_rerank_enabled(None, true);
        let outcome = rerank_items(enabled, Some(&client), "q", vec![item("only", 0.1)], 1).await;
        assert!(enabled);
        assert!(outcome.applied);
    }

    /// VAL-RET-006: override true when config default off still attempts.
    #[tokio::test]
    async fn override_true_when_default_off_applies() {
        let base = spawn_json_server(
            "HTTP/1.1 200 OK",
            r#"{"data":[{"index":0,"relevance_score":0.7}]}"#,
        );
        let client = VoyageReranker::with_base_url(
            base,
            "k".to_owned(),
            "rerank-2.5".to_owned(),
            Duration::from_secs(2),
        )
        .expect("client");
        let enabled = resolve_rerank_enabled(Some(true), false);
        let outcome = rerank_items(enabled, Some(&client), "q", vec![item("x", 0.1)], 1).await;
        assert!(enabled);
        assert!(outcome.applied);
    }

    /// try_new: blank key → None (missing key path).
    #[test]
    fn try_new_blank_key_is_none() {
        assert!(VoyageReranker::try_new("", "rerank-2.5", Duration::from_secs(1)).is_none());
        assert!(VoyageReranker::try_new("   ", "rerank-2.5", Duration::from_secs(1)).is_none());
    }

    /// VAL-CROSS-009: infrastructure errors and sanitizer do not embed the API key.
    #[tokio::test]
    async fn errors_do_not_leak_api_key() {
        let secret = "super-secret-voyage-key-xyz";
        let base = spawn_json_server("HTTP/1.1 401 Unauthorized", r#"{"error":"nope"}"#);
        let client = VoyageReranker::with_base_url(
            base,
            secret.to_owned(),
            "rerank-2.5".to_owned(),
            Duration::from_secs(2),
        )
        .expect("client");
        let err = client
            .rerank("q", &["doc".to_owned()], 1)
            .await
            .expect_err("should fail");
        let display = err.to_string();
        assert!(
            !display.contains(secret),
            "error must not contain API key: {display}"
        );
        let sanitized = sanitized_rerank_error(&err);
        assert!(!sanitized.contains(secret));
        assert_eq!(sanitized, "provider_unavailable");
    }

    /// Empty docs / top_k short-circuit without network.
    #[tokio::test]
    async fn empty_documents_returns_empty_hits() {
        let client = VoyageReranker::with_base_url(
            "http://127.0.0.1:1".to_owned(),
            "k".to_owned(),
            "rerank-2.5".to_owned(),
            Duration::from_millis(50),
        )
        .expect("client");
        let hits = client.rerank("q", &[], 5).await.expect("empty ok");
        assert!(hits.is_empty());
    }

    /// Default model when constructor model string is blank.
    #[test]
    fn blank_model_defaults_to_rerank_2_5() {
        let client = VoyageReranker::with_base_url(
            "http://example.invalid".to_owned(),
            "k".to_owned(),
            String::new(),
            Duration::from_secs(1),
        )
        .expect("client");
        assert_eq!(client.model, "rerank-2.5");
    }
}

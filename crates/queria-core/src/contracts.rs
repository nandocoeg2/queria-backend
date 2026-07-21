use crate::ids::{ChunkId, ProjectId, SourceDocumentId};
use crate::model::{KnowledgeScope, KnowledgeStatus};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

fn default_include_scratch() -> bool {
    true
}

fn default_include_needs_review() -> bool {
    false
}

fn default_rerank_applied() -> bool {
    false
}

fn default_compress_dropped() -> u32 {
    0
}

fn default_latency_ms() -> u32 {
    0
}

/// Derived lane for retrieve (no separate DB column).
/// Ranking preference: trusted > scratch > needs_review.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeLane {
    Trusted,
    Scratch,
    NeedsReview,
}

impl KnowledgeLane {
    #[must_use]
    pub const fn from_status(status: KnowledgeStatus) -> Self {
        if status.is_scratch_lane() {
            Self::Scratch
        } else if matches!(status, KnowledgeStatus::NeedsReview) {
            Self::NeedsReview
        } else {
            Self::Trusted
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Scratch => "scratch",
            Self::NeedsReview => "needs_review",
        }
    }

    /// Lower is preferred (approved/trusted first).
    #[must_use]
    pub const fn preference_rank(self) -> u8 {
        match self {
            Self::Trusted => 0,
            Self::Scratch => 1,
            Self::NeedsReview => 2,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RetrieveContextRequest {
    pub project_id: ProjectId,
    pub query: String,
    pub include_global: bool,
    /// When true (agent default), include project-scoped scratch alongside approved.
    /// Golden/eval and trusted-only probes set false.
    #[serde(default = "default_include_scratch")]
    pub include_scratch: bool,
    /// When true, include project-scoped `needs_review` items for principals with
    /// project access (all org members; not owner-only). Default false everywhere
    /// (agents, operators, CLI, eval).
    #[serde(default = "default_include_needs_review")]
    pub include_needs_review: bool,
    pub limit: u32,
    /// `None` uses `QUERIA_RERANK_ENABLED` / RetrievalSettings default.
    #[serde(default)]
    pub rerank: Option<bool>,
    /// `None` uses `QUERIA_COMPRESS_ENABLED` / RetrievalSettings default.
    #[serde(default)]
    pub compress: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrieveContextResponse {
    pub project_id: ProjectId,
    pub query: String,
    pub items: Vec<RetrievedContextItem>,
    pub retrieval: RetrievalDiagnostics,
    pub generated_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalMode {
    Hybrid,
    LexicalFallback,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RetrievalDiagnostics {
    pub mode: RetrievalMode,
    pub lexical_candidates: u32,
    pub semantic_candidates: u32,
    pub embedding_profile_version: String,
    /// Whether Voyage rerank successfully reordered the candidate set.
    #[serde(default = "default_rerank_applied")]
    pub rerank_applied: bool,
    /// Near-duplicate items dropped by compress (0 when compress off/skipped).
    #[serde(default = "default_compress_dropped")]
    pub compress_dropped: u32,
    /// End-to-end retrieve latency in milliseconds.
    #[serde(default = "default_latency_ms")]
    pub latency_ms: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RetrievedContextItem {
    pub chunk_id: ChunkId,
    pub source_document_id: SourceDocumentId,
    pub scope: KnowledgeScope,
    /// Lean status signal: `approved`, optional `scratch`, optional `needs_review`.
    pub status: KnowledgeStatus,
    /// Lane derived from status (`trusted` / `scratch` / `needs_review`).
    pub lane: KnowledgeLane,
    pub title: String,
    pub body: String,
    pub citation: Citation,
    pub score: f32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Citation {
    pub source_uri: String,
    pub source_path: Option<String>,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
}

impl RetrieveContextRequest {
    pub fn validate(&self) -> crate::QueriaResult<()> {
        let query = self.query.trim();
        if query.is_empty() {
            return Err(crate::QueriaError::Validation(
                "query must not be blank".to_owned(),
            ));
        }

        if query.len() > 512 {
            return Err(crate::QueriaError::Validation(
                "query must be at most 512 bytes".to_owned(),
            ));
        }

        if !(1..=20).contains(&self.limit) {
            return Err(crate::QueriaError::Validation(
                "limit must be between 1 and 20".to_owned(),
            ));
        }

        Ok(())
    }
}

/// Shared body validation for MCP write tools (`propose_memory`, `index_memory`).
///
/// Trims leading/trailing whitespace, rejects blank bodies, and enforces
/// `max_body_bytes` (UTF-8 byte length of the trimmed body). Used by IMP-23.
pub fn validate_memory_body(body: &str, max_body_bytes: usize) -> crate::QueriaResult<String> {
    let body = body.trim().to_owned();
    if body.is_empty() {
        return Err(crate::QueriaError::Validation("invalid_body".to_owned()));
    }
    if body.len() > max_body_bytes {
        return Err(crate::QueriaError::Validation(format!(
            "body_too_large: max {max_body_bytes} bytes"
        )));
    }
    Ok(body)
}

/// Normalize memory body for IMP-22 content hashing.
///
/// Trims ends and collapses internal runs of whitespace to a single space so
/// trivial whitespace variants map to one scratch item.
#[must_use]
pub fn normalize_memory_body_for_hash(body: &str) -> String {
    body.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// SHA-256 hex of the normalized body used for scratch idempotency (IMP-22).
#[must_use]
pub fn scratch_content_hash(body: &str) -> String {
    use sha2::{Digest, Sha256};
    let normalized = normalize_memory_body_for_hash(body);
    let digest = Sha256::digest(normalized.as_bytes());
    format!("{digest:x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::QueriaError;

    #[test]
    fn retrieve_context_request_rejects_blank_query() {
        let request = RetrieveContextRequest {
            project_id: ProjectId::new(),
            query: "  ".to_owned(),
            include_global: true,
            include_scratch: true,
            include_needs_review: false,
            limit: 5,
            rerank: None,
            compress: None,
        };

        assert!(request.validate().is_err());
    }

    /// VAL-DL-040 / VAL-RET-012: blank/long query and limit bounds stay validated.
    #[test]
    fn retrieve_context_request_rejects_overlong_query_and_bad_limit() {
        let overlong = "q".repeat(513);
        let bad_query = RetrieveContextRequest {
            project_id: ProjectId::new(),
            query: overlong,
            include_global: true,
            include_scratch: true,
            include_needs_review: false,
            limit: 5,
            rerank: None,
            compress: None,
        };
        let err = bad_query.validate().expect_err("512 byte max");
        assert!(
            matches!(err, QueriaError::Validation(ref msg) if msg.contains("512")),
            "expected overlong query validation, got {err:?}"
        );

        for limit in [0_u32, 21] {
            let req = RetrieveContextRequest {
                project_id: ProjectId::new(),
                query: "ok".to_owned(),
                include_global: true,
                include_scratch: false,
                include_needs_review: false,
                limit,
                rerank: None,
                compress: None,
            };
            assert!(req.validate().is_err(), "limit {limit} must be rejected");
        }
    }

    #[test]
    fn retrieve_context_request_accepts_bounded_query() {
        let request = RetrieveContextRequest {
            project_id: ProjectId::new(),
            query: "how to deploy fjulian-me".to_owned(),
            include_global: true,
            include_scratch: true,
            include_needs_review: false,
            limit: 8,
            rerank: None,
            compress: None,
        };

        request.validate().expect("bounded query should be valid");
    }

    /// VAL-RET-012: limit 1 and 20 remain valid after flag additions.
    #[test]
    fn retrieve_context_request_accepts_limit_bounds() {
        for limit in [1_u32, 20] {
            let request = RetrieveContextRequest {
                project_id: ProjectId::new(),
                query: "ok".to_owned(),
                include_global: true,
                include_scratch: true,
                include_needs_review: false,
                limit,
                rerank: Some(true),
                compress: Some(false),
            };
            request
                .validate()
                .unwrap_or_else(|err| panic!("limit {limit} must be accepted: {err}"));
        }
    }

    /// VAL-CROSS-004 / VAL-RET: omitted rerank/compress deserialize as None (use config).
    #[test]
    fn retrieve_context_omitted_rerank_compress_are_none() {
        let value = serde_json::json!({
            "project_id": ProjectId::new(),
            "query": "marker",
            "include_global": true,
            "limit": 5
        });
        let request: RetrieveContextRequest =
            serde_json::from_value(value).expect("old client payload deserializes");
        assert_eq!(request.rerank, None);
        assert_eq!(request.compress, None);
        assert!(request.include_scratch);
        assert!(!request.include_needs_review);
    }

    /// Optional flags deserialize when present and override independently.
    #[test]
    fn retrieve_context_explicit_rerank_compress_flags() {
        let value = serde_json::json!({
            "project_id": ProjectId::new(),
            "query": "marker",
            "include_global": true,
            "include_scratch": false,
            "include_needs_review": true,
            "limit": 3,
            "rerank": false,
            "compress": true
        });
        let request: RetrieveContextRequest =
            serde_json::from_value(value).expect("explicit flags deserialize");
        assert_eq!(request.rerank, Some(false));
        assert_eq!(request.compress, Some(true));
        assert!(!request.include_scratch);
        assert!(request.include_needs_review);
    }

    /// VAL-RET-011: diagnostics serialize new fields; old payloads still deserialize.
    #[test]
    fn retrieval_diagnostics_new_fields_roundtrip_and_default() {
        let full = RetrievalDiagnostics {
            mode: RetrievalMode::Hybrid,
            lexical_candidates: 4,
            semantic_candidates: 6,
            embedding_profile_version: "voyage-4-1024-v1".to_owned(),
            rerank_applied: true,
            compress_dropped: 2,
            latency_ms: 42,
        };
        let value = serde_json::to_value(&full).expect("serialize diagnostics");
        assert_eq!(value["rerank_applied"], true);
        assert_eq!(value["compress_dropped"], 2);
        assert_eq!(value["latency_ms"], 42);

        let legacy = serde_json::json!({
            "mode": "hybrid",
            "lexical_candidates": 1,
            "semantic_candidates": 2,
            "embedding_profile_version": "voyage-4-1024-v1"
        });
        let parsed: RetrievalDiagnostics =
            serde_json::from_value(legacy).expect("legacy diagnostics deserialize");
        assert!(!parsed.rerank_applied);
        assert_eq!(parsed.compress_dropped, 0);
        assert_eq!(parsed.latency_ms, 0);
    }

    /// VAL-DL-036 / VAL-CROSS-008: approved maps to trusted lane leanly.
    #[test]
    fn approved_status_maps_to_trusted_lane_leanly() {
        assert!(KnowledgeStatus::Approved.is_trusted_lane());
        assert!(!KnowledgeStatus::Approved.is_scratch_lane());
        assert_eq!(
            KnowledgeLane::from_status(KnowledgeStatus::Approved).as_str(),
            "trusted"
        );
        assert_eq!(KnowledgeStatus::Approved.as_str(), "approved");
    }

    /// VAL-DL-026 / IMP-14: agent default for include_scratch is true.
    #[test]
    fn retrieve_context_include_scratch_defaults_true() {
        let value = serde_json::json!({
            "project_id": ProjectId::new(),
            "query": "marker",
            "include_global": true,
            "limit": 5
        });
        let request: RetrieveContextRequest =
            serde_json::from_value(value).expect("deserialize without include_scratch");
        assert!(request.include_scratch);
    }

    /// IMP-L3: include_needs_review defaults false (eval/agent/operator).
    #[test]
    fn retrieve_context_include_needs_review_defaults_false() {
        let value = serde_json::json!({
            "project_id": ProjectId::new(),
            "query": "marker",
            "include_global": true,
            "limit": 5
        });
        let request: RetrieveContextRequest =
            serde_json::from_value(value).expect("deserialize without include_needs_review");
        assert!(!request.include_needs_review);
    }

    /// VAL-DL-035 / VAL-DL-036: lean lane derivation from status.
    #[test]
    fn knowledge_lane_derives_from_status() {
        assert_eq!(
            KnowledgeLane::from_status(KnowledgeStatus::Scratch),
            KnowledgeLane::Scratch
        );
        assert_eq!(
            KnowledgeLane::from_status(KnowledgeStatus::Approved),
            KnowledgeLane::Trusted
        );
        assert_eq!(
            KnowledgeLane::from_status(KnowledgeStatus::NeedsReview),
            KnowledgeLane::NeedsReview
        );
        assert_eq!(KnowledgeLane::Scratch.as_str(), "scratch");
        assert_eq!(KnowledgeLane::Trusted.as_str(), "trusted");
        assert_eq!(KnowledgeLane::NeedsReview.as_str(), "needs_review");
        assert!(
            KnowledgeLane::Trusted.preference_rank() < KnowledgeLane::Scratch.preference_rank()
        );
        assert!(
            KnowledgeLane::Scratch.preference_rank() < KnowledgeLane::NeedsReview.preference_rank()
        );
    }

    #[test]
    fn retrieval_mode_serializes_as_stable_snake_case() {
        assert_eq!(
            serde_json::to_value(RetrievalMode::LexicalFallback).expect("mode should serialize"),
            serde_json::json!("lexical_fallback")
        );
    }

    /// VAL-DL-024: empty / whitespace-only body rejected.
    #[test]
    fn validate_memory_body_rejects_empty_and_blank() {
        for raw in ["", "   ", "\n\t"] {
            let err = validate_memory_body(raw, 20_000).expect_err("blank must fail");
            assert!(
                matches!(err, QueriaError::Validation(ref msg) if msg == "invalid_body"),
                "unexpected error for {raw:?}: {err:?}"
            );
        }
    }

    /// VAL-DL-022 / VAL-DL-025: oversized body rejected under configured max.
    #[test]
    fn validate_memory_body_rejects_oversized() {
        let max = 32usize;
        let oversize = "x".repeat(max + 1);
        let err = validate_memory_body(&oversize, max).expect_err("oversized must fail");
        match err {
            QueriaError::Validation(msg) => {
                assert!(
                    msg.starts_with("body_too_large"),
                    "clear client error expected, got {msg}"
                );
                assert!(msg.contains(&max.to_string()));
            }
            other => panic!("expected Validation, got {other:?}"),
        }
    }

    /// VAL-DL-023: body at limit accepted; under max accepted; shared for both write tools.
    #[test]
    fn validate_memory_body_accepts_under_and_equal_max() {
        let max = 16usize;
        let exact = "a".repeat(max);
        let under = "hello".to_owned();
        assert_eq!(
            validate_memory_body(&exact, max).expect("exact max ok"),
            exact
        );
        assert_eq!(
            validate_memory_body(&format!("  {under}  "), max).expect("trimmed under ok"),
            under
        );
    }

    /// IMP-23: same max_body_bytes applies to propose and index tool bodies.
    #[test]
    fn shared_max_applies_identically_to_both_write_tools() {
        let max = 20_000usize;
        let legal = "mission-dl-body-note".to_owned();
        let huge = "z".repeat(max + 1);
        assert!(validate_memory_body(&legal, max).is_ok());
        assert!(validate_memory_body(&huge, max).is_err());
        // Identical bounds: any tool that calls this helper shares the env limit.
        assert_eq!(
            validate_memory_body(&legal, max).unwrap(),
            validate_memory_body(&legal, max).unwrap()
        );
    }

    /// VAL-DL-019 / IMP-22: whitespace-normalized bodies share content hash.
    #[test]
    fn scratch_content_hash_normalizes_whitespace() {
        let a = scratch_content_hash("hello   world\n");
        let b = scratch_content_hash("  hello world  ");
        let c = scratch_content_hash("hello world");
        assert_eq!(a, b);
        assert_eq!(b, c);
        assert_eq!(a.len(), 64);
    }

    /// VAL-DL-020: different bodies yield different hashes.
    #[test]
    fn scratch_content_hash_differs_for_distinct_bodies() {
        let a = scratch_content_hash("mission-dl-marker-one");
        let b = scratch_content_hash("mission-dl-marker-two");
        assert_ne!(a, b);
    }

    #[test]
    fn normalize_memory_body_for_hash_collapses_runs() {
        assert_eq!(
            normalize_memory_body_for_hash("  foo \t bar\nbaz  "),
            "foo bar baz"
        );
    }
}

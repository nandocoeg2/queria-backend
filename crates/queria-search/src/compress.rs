//! Near-duplicate compression after ranking (IMP-02 / architecture compress stage).
//!
//! Pure, deterministic: no network. Near-dups match on normalized body whitespace
//! and/or equal non-empty content hash (same normalization as scratch idempotency).
//! Preference when near-dups collide: trusted > scratch > needs_review.

use queria_core::contracts::{
    RetrievedContextItem, normalize_memory_body_for_hash, scratch_content_hash,
};
use std::collections::HashMap;

/// Result of compressing a ranked item list.
#[derive(Clone, Debug)]
pub struct CompressOutcome {
    pub items: Vec<RetrievedContextItem>,
    pub dropped: u32,
}

/// Resolve whether compress should run: request override or config default.
#[must_use]
pub fn resolve_compress_enabled(request_flag: Option<bool>, config_default: bool) -> bool {
    request_flag.unwrap_or(config_default)
}

/// Drop near-duplicate hydrated items, preferring higher-trust lanes.
///
/// Preference: trusted (approved) > scratch > needs_review.
///
/// Input must already be in rank order (RRF or rerank). Survivors keep that order
/// among distinct keys; when a later higher-preference item near-dups an earlier
/// lower-preference survivor, the higher one replaces it in place (lower counts
/// as dropped).
#[must_use]
pub fn compress_items(items: Vec<RetrievedContextItem>, enabled: bool) -> CompressOutcome {
    if !enabled || items.len() < 2 {
        return CompressOutcome { dropped: 0, items };
    }

    let mut seen: HashMap<String, usize> = HashMap::with_capacity(items.len());
    let mut out: Vec<RetrievedContextItem> = Vec::with_capacity(items.len());
    let mut dropped: u32 = 0;

    for item in items {
        let key = near_dup_key(&item);
        if key.is_empty() {
            // Empty bodies do not participate in collapse (avoid matching everything).
            out.push(item);
            continue;
        }

        if let Some(&idx) = seen.get(&key) {
            // Lower preference_rank wins (trusted=0, scratch=1, needs_review=2).
            let prefer_incoming = item.lane.preference_rank() < out[idx].lane.preference_rank();
            if prefer_incoming {
                out[idx] = item;
            }
            dropped = dropped.saturating_add(1);
        } else {
            seen.insert(key, out.len());
            out.push(item);
        }
    }

    CompressOutcome {
        items: out,
        dropped,
    }
}

/// Near-dup key: non-empty content hash of whitespace-normalized body
/// (equivalent to normalized body equality for non-empty text).
fn near_dup_key(item: &RetrievedContextItem) -> String {
    let normalized = normalize_memory_body_for_hash(&item.body);
    if normalized.is_empty() {
        return String::new();
    }
    // Hash is stable for normalized form; also documents "content hash" criterion.
    scratch_content_hash(&item.body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use queria_core::contracts::{Citation, KnowledgeLane};
    use queria_core::ids::{ChunkId, SourceDocumentId};
    use queria_core::model::{KnowledgeScope, KnowledgeStatus};

    fn item(body: &str, lane: KnowledgeLane, score: f32) -> RetrievedContextItem {
        let status = match lane {
            KnowledgeLane::Trusted => KnowledgeStatus::Approved,
            KnowledgeLane::Scratch => KnowledgeStatus::Scratch,
            KnowledgeLane::NeedsReview => KnowledgeStatus::NeedsReview,
        };
        RetrievedContextItem {
            chunk_id: ChunkId::new(),
            source_document_id: SourceDocumentId::new(),
            scope: KnowledgeScope::Project,
            status,
            lane,
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

    /// VAL-RET-009: near-dup bodies (normalized whitespace) collapse to one survivor.
    #[test]
    fn drops_near_dup_bodies() {
        let a = item("hello   world\n", KnowledgeLane::Trusted, 0.9);
        let b = item("  hello world  ", KnowledgeLane::Trusted, 0.8);
        let c = item("distinct fact", KnowledgeLane::Trusted, 0.7);
        let outcome = compress_items(vec![a.clone(), b, c.clone()], true);
        assert_eq!(outcome.items.len(), 2);
        assert_eq!(outcome.items[0].chunk_id, a.chunk_id);
        assert_eq!(outcome.items[1].chunk_id, c.chunk_id);
        assert!(outcome.dropped >= 1);
    }

    /// VAL-RET-009 / survivors preserve distinct content.
    #[test]
    fn preserves_distinct_bodies() {
        let a = item("alpha recipe", KnowledgeLane::Trusted, 0.9);
        let b = item("beta recipe", KnowledgeLane::Trusted, 0.8);
        let outcome = compress_items(vec![a.clone(), b.clone()], true);
        assert_eq!(outcome.items.len(), 2);
        assert_eq!(outcome.dropped, 0);
        assert_eq!(outcome.items[0].chunk_id, a.chunk_id);
        assert_eq!(outcome.items[1].chunk_id, b.chunk_id);
    }

    /// VAL-RET-010: trusted beats scratch near-dup.
    #[test]
    fn prefers_trusted_over_scratch() {
        let scratch = item("same memory body", KnowledgeLane::Scratch, 0.95);
        let trusted = item("same memory body", KnowledgeLane::Trusted, 0.90);
        let outcome = compress_items(vec![scratch.clone(), trusted.clone()], true);
        assert_eq!(outcome.items.len(), 1);
        assert_eq!(outcome.items[0].lane, KnowledgeLane::Trusted);
        assert_eq!(outcome.items[0].chunk_id, trusted.chunk_id);
        assert_eq!(outcome.items[0].status, KnowledgeStatus::Approved);
        assert_eq!(outcome.dropped, 1);
    }

    /// IMP-L3: trusted beats needs_review near-dup; scratch beats needs_review.
    #[test]
    fn prefers_trusted_and_scratch_over_needs_review() {
        let needs = item("shared index-here fact", KnowledgeLane::NeedsReview, 0.99);
        let trusted = item("shared index-here fact", KnowledgeLane::Trusted, 0.5);
        let outcome = compress_items(vec![needs.clone(), trusted.clone()], true);
        assert_eq!(outcome.items.len(), 1);
        assert_eq!(outcome.items[0].lane, KnowledgeLane::Trusted);
        assert_eq!(outcome.items[0].chunk_id, trusted.chunk_id);

        let needs = item("shared note", KnowledgeLane::NeedsReview, 0.95);
        let scratch = item("shared note", KnowledgeLane::Scratch, 0.4);
        let outcome = compress_items(vec![needs, scratch.clone()], true);
        assert_eq!(outcome.items.len(), 1);
        assert_eq!(outcome.items[0].lane, KnowledgeLane::Scratch);
        assert_eq!(outcome.items[0].chunk_id, scratch.chunk_id);
    }

    /// Trusted already first: scratch near-dup is dropped (VAL-RET-010 path).
    #[test]
    fn keeps_trusted_when_scratch_follows() {
        let trusted = item("deploy via CI", KnowledgeLane::Trusted, 0.9);
        let scratch = item("deploy via CI", KnowledgeLane::Scratch, 0.85);
        let outcome = compress_items(vec![trusted.clone(), scratch], true);
        assert_eq!(outcome.items.len(), 1);
        assert_eq!(outcome.items[0].chunk_id, trusted.chunk_id);
        assert_eq!(outcome.items[0].lane, KnowledgeLane::Trusted);
        assert_eq!(outcome.dropped, 1);
    }

    /// VAL-RET-004: compress=false keeps near-dups.
    #[test]
    fn compress_off_keeps_near_dups() {
        let a = item("hello world", KnowledgeLane::Trusted, 0.9);
        let b = item("hello world", KnowledgeLane::Scratch, 0.8);
        let outcome = compress_items(vec![a.clone(), b.clone()], false);
        assert_eq!(outcome.items.len(), 2);
        assert_eq!(outcome.dropped, 0);
        assert_eq!(outcome.items[0].chunk_id, a.chunk_id);
        assert_eq!(outcome.items[1].chunk_id, b.chunk_id);
    }

    /// VAL-RET-009: compress_dropped counts removed near-dups.
    #[test]
    fn counts_dropped() {
        let a = item("dup", KnowledgeLane::Trusted, 0.9);
        let b = item("dup", KnowledgeLane::Trusted, 0.8);
        let c = item("dup", KnowledgeLane::Scratch, 0.7);
        let outcome = compress_items(vec![a, b, c], true);
        assert_eq!(outcome.items.len(), 1);
        assert_eq!(outcome.dropped, 2);
    }

    /// VAL-RET-020: with enough near-dups, survivor count can be < input (proxy for < limit).
    #[test]
    fn may_return_fewer_than_limit() {
        let limit = 5_usize;
        let items = vec![
            item("shared", KnowledgeLane::Trusted, 0.99),
            item("shared", KnowledgeLane::Scratch, 0.98),
            item("shared", KnowledgeLane::Trusted, 0.97),
            item("other", KnowledgeLane::Trusted, 0.5),
            item("other", KnowledgeLane::Scratch, 0.4),
        ];
        assert_eq!(items.len(), limit);
        let outcome = compress_items(items, true);
        assert!(outcome.items.len() < limit);
        assert_eq!(outcome.items.len(), 2);
        assert_eq!(outcome.dropped, 3);
    }

    /// VAL-RET-003: survivors preserve relative pre-compress rank among distinct items.
    #[test]
    fn preserves_rank_order_among_survivors() {
        let first = item("fact one", KnowledgeLane::Trusted, 0.9);
        let dup = item("fact one", KnowledgeLane::Scratch, 0.85);
        let second = item("fact two", KnowledgeLane::Trusted, 0.8);
        let third = item("fact three", KnowledgeLane::Scratch, 0.7);
        let outcome = compress_items(
            vec![first.clone(), dup, second.clone(), third.clone()],
            true,
        );
        assert_eq!(outcome.items.len(), 3);
        assert_eq!(outcome.items[0].chunk_id, first.chunk_id);
        assert_eq!(outcome.items[1].chunk_id, second.chunk_id);
        assert_eq!(outcome.items[2].chunk_id, third.chunk_id);
        assert_eq!(outcome.dropped, 1);
    }

    /// VAL-RET-019: compress does not rewrite lane/status of survivors.
    #[test]
    fn lane_identity_preserved() {
        let trusted = item("unique trusted", KnowledgeLane::Trusted, 0.9);
        let scratch = item("unique scratch", KnowledgeLane::Scratch, 0.8);
        let outcome = compress_items(vec![trusted.clone(), scratch.clone()], true);
        assert_eq!(outcome.items[0].lane, KnowledgeLane::Trusted);
        assert_eq!(outcome.items[0].status, KnowledgeStatus::Approved);
        assert_eq!(outcome.items[1].lane, KnowledgeLane::Scratch);
        assert_eq!(outcome.items[1].status, KnowledgeStatus::Scratch);
    }

    /// VAL-RET-016: omitted request flag follows config default.
    #[test]
    fn omitted_flag_uses_config_default() {
        assert!(resolve_compress_enabled(None, true));
        assert!(!resolve_compress_enabled(None, false));
    }

    /// VAL-RET-017: explicit compress override both ways.
    #[test]
    fn explicit_compress_override_both_ways() {
        assert!(!resolve_compress_enabled(Some(false), true));
        assert!(resolve_compress_enabled(Some(true), false));
    }

    /// Equal non-empty content hash (via normalized body) collapses.
    #[test]
    fn equal_content_hash_collapses() {
        // Same normalized form => same scratch_content_hash.
        let a = item("token-a  token-b", KnowledgeLane::Trusted, 0.9);
        let b = item("token-a token-b", KnowledgeLane::Trusted, 0.8);
        assert_eq!(scratch_content_hash(&a.body), scratch_content_hash(&b.body));
        let outcome = compress_items(vec![a, b], true);
        assert_eq!(outcome.items.len(), 1);
        assert_eq!(outcome.dropped, 1);
    }
}

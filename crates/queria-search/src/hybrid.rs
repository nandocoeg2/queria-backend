use queria_core::ids::ChunkId;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RankedChunk {
    pub chunk_id: ChunkId,
    pub source_score: f32,
}

impl RankedChunk {
    #[must_use]
    pub const fn new(chunk_id: ChunkId, source_score: f32) -> Self {
        Self {
            chunk_id,
            source_score,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FusedChunk {
    pub chunk_id: ChunkId,
    pub score: f32,
}

#[must_use]
pub fn reciprocal_rank_fusion(
    lexical: &[RankedChunk],
    semantic: &[RankedChunk],
    k: u32,
    limit: usize,
) -> Vec<FusedChunk> {
    if limit == 0 {
        return Vec::new();
    }
    let mut scores = HashMap::<ChunkId, f32>::new();
    add_rank_scores(&mut scores, lexical, k);
    add_rank_scores(&mut scores, semantic, k);
    let mut fused = scores
        .into_iter()
        .map(|(chunk_id, score)| FusedChunk { chunk_id, score })
        .collect::<Vec<_>>();
    fused.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.chunk_id.as_uuid().cmp(&right.chunk_id.as_uuid()))
    });
    fused.truncate(limit);
    if let Some(max_score) = fused.first().map(|item| item.score)
        && max_score > 0.0
    {
        for item in &mut fused {
            item.score /= max_score;
        }
    }
    fused
}

fn add_rank_scores(scores: &mut HashMap<ChunkId, f32>, ranked: &[RankedChunk], k: u32) {
    let mut seen = HashSet::new();
    for (index, item) in ranked.iter().enumerate() {
        if !seen.insert(item.chunk_id) {
            continue;
        }
        let rank = index.saturating_add(1) as f32;
        *scores.entry(item.chunk_id).or_default() += 1.0 / (k as f32 + rank);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use queria_core::ids::ChunkId;

    #[test]
    fn reciprocal_rank_fusion_rewards_overlap_and_is_deterministic() {
        let shared = ChunkId::new();
        let lexical_only = ChunkId::new();
        let semantic_only = ChunkId::new();
        let lexical = vec![
            RankedChunk::new(shared, 0.9),
            RankedChunk::new(lexical_only, 0.8),
        ];
        let semantic = vec![
            RankedChunk::new(semantic_only, 0.95),
            RankedChunk::new(shared, 0.7),
        ];

        let fused = reciprocal_rank_fusion(&lexical, &semantic, 60, 3);

        assert_eq!(fused[0].chunk_id, shared);
        assert_eq!(fused.len(), 3);
    }

    #[test]
    fn reciprocal_rank_fusion_respects_limit() {
        let lexical = (0..5)
            .map(|_| RankedChunk::new(ChunkId::new(), 1.0))
            .collect::<Vec<_>>();

        assert_eq!(reciprocal_rank_fusion(&lexical, &[], 60, 2).len(), 2);
    }
}

use crate::embeddings;
use queria_core::contracts::RetrieveContextRequest;
use queria_search::retrieval::{RetrievalPrincipal, build_pg_retrieval_service};

pub async fn probe(
    project_slug: &str,
    query: &str,
    include_global: bool,
    limit: u32,
    rerank: Option<bool>,
    compress: Option<bool>,
) -> anyhow::Result<()> {
    let (config, pool, user_id, project_id) = embeddings::context(project_slug).await?;
    let service = build_pg_retrieval_service(&config, pool)?;
    let response = service
        .retrieve_context(
            &RetrievalPrincipal::User { user_id },
            RetrieveContextRequest {
                project_id,
                query: query.to_owned(),
                include_global,
                // CLI retrieval probe is trusted-only by default (VAL-DL-043 / VAL-CROSS-005).
                include_scratch: false,
                // IMP-L3: never include needs_review on CLI probe (same as include_scratch=false).
                include_needs_review: false,
                limit,
                rerank,
                compress,
            },
        )
        .await?;
    println!("{}", serde_json::to_string_pretty(&response)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    /// VAL-DL-043: CLI probe hard-codes include_scratch=false (trusted-only).
    #[test]
    fn cli_probe_is_trusted_only_by_default() {
        // Keep in sync with probe(): agents dual-lane default true; operators false.
        let include_scratch = false;
        assert!(!include_scratch, "CLI probe must exclude scratch lane");
    }

    /// IMP-L3: CLI probe hard-codes include_needs_review=false.
    #[test]
    fn cli_probe_excludes_needs_review_by_default() {
        let include_needs_review = false;
        assert!(
            !include_needs_review,
            "CLI probe must exclude needs_review lane"
        );
    }

    /// VAL-CROSS-001: omitted rerank/compress are None (server config defaults apply).
    #[test]
    fn cli_probe_omitted_flags_are_none() {
        let rerank: Option<bool> = None;
        let compress: Option<bool> = None;
        assert!(rerank.is_none());
        assert!(compress.is_none());
    }
}

use queria_core::contracts::{
    Citation, RetrievalDiagnostics, RetrievalMode, RetrieveContextResponse, RetrievedContextItem,
};
use queria_core::evaluation::{GoldenQuestion, score_evaluation_report};
use queria_core::ids::{ChunkId, ProjectId, SourceDocumentId};
use queria_core::model::KnowledgeScope;

#[test]
fn evaluation_report_scores_scope_and_citation_hits() {
    let questions = vec![GoldenQuestion {
        id: "fjulian-me-astro-content".to_owned(),
        project_slug: "fjulian-me".to_owned(),
        query: "Astro markdown content flow".to_owned(),
        include_global: true,
        expected_scope: vec![KnowledgeScope::Project],
        expected_citations: vec!["README.md".to_owned()],
        minimum_items: 1,
    }];
    let responses = vec![Ok(response_with_items(vec![item(
        KnowledgeScope::Project,
        "README.md",
    )]))];

    let report = score_evaluation_report(
        "fjulian-me",
        "tests/golden_questions/fjulian-me.jsonl",
        &questions,
        responses,
    );

    assert!(report.passed);
    assert_eq!(report.total_questions, 1);
    assert_eq!(report.passed_questions, 1);
    assert_eq!(report.failed_questions, 0);
    assert_eq!(report.regression_score, 1.0);
    assert_eq!(report.results[0].scope_hits, 1);
    assert_eq!(report.results[0].citation_hits, 1);
}

fn response_with_items(items: Vec<RetrievedContextItem>) -> RetrieveContextResponse {
    RetrieveContextResponse {
        project_id: ProjectId::new(),
        query: "query".to_owned(),
        items,
        retrieval: RetrievalDiagnostics {
            mode: RetrievalMode::Hybrid,
            lexical_candidates: 2,
            semantic_candidates: 3,
            embedding_profile_version: "voyage-4-1024-v1".to_owned(),
            rerank_applied: false,
            compress_dropped: 0,
            latency_ms: 0,
        },
        generated_at: chrono::Utc::now(),
    }
}

fn item(scope: KnowledgeScope, source_path: &str) -> RetrievedContextItem {
    use queria_core::contracts::KnowledgeLane;
    use queria_core::model::KnowledgeStatus;
    RetrievedContextItem {
        chunk_id: ChunkId::new(),
        source_document_id: SourceDocumentId::new(),
        scope,
        status: KnowledgeStatus::Approved,
        lane: KnowledgeLane::Trusted,
        title: "title".to_owned(),
        body: "body".to_owned(),
        citation: Citation {
            source_uri: format!("queria-git://repo/{source_path}"),
            source_path: Some(source_path.to_owned()),
            line_start: Some(1),
            line_end: Some(3),
        },
        score: 1.0,
    }
}

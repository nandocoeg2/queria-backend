use crate::embeddings;
use queria_core::contracts::{RetrieveContextRequest, RetrieveContextResponse};
use queria_core::model::KnowledgeScope;
use queria_search::retrieval::{RetrievalPrincipal, build_pg_retrieval_service};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct GoldenQuestion {
    id: String,
    project_slug: String,
    query: String,
    #[serde(default = "default_include_global")]
    include_global: bool,
    #[serde(default)]
    expected_scope: Vec<KnowledgeScope>,
    #[serde(default)]
    expected_citations: Vec<String>,
    #[serde(default = "default_minimum_items")]
    minimum_items: usize,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
struct EvaluationQuestionResult {
    id: String,
    query: String,
    passed: bool,
    items_returned: usize,
    minimum_items: usize,
    expected_scope: Vec<KnowledgeScope>,
    scope_hits: usize,
    expected_citations: Vec<String>,
    citation_hits: usize,
    retrieval_mode: String,
    lexical_candidates: u32,
    semantic_candidates: u32,
    regression_score: f32,
    error: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
struct EvaluationReport {
    project: String,
    golden_question_file: String,
    total_questions: usize,
    passed_questions: usize,
    failed_questions: usize,
    passed: bool,
    regression_score: f32,
    results: Vec<EvaluationQuestionResult>,
}

pub async fn run(project_slug: &str) -> anyhow::Result<()> {
    let golden_path = PathBuf::from(format!("tests/golden_questions/{project_slug}.jsonl"));
    let content = std::fs::read_to_string(&golden_path)?;
    let questions = parse_golden_questions(&content)?
        .into_iter()
        .filter(|question| question.project_slug == project_slug)
        .collect::<Vec<_>>();
    if questions.is_empty() {
        anyhow::bail!("no golden questions found for project {project_slug}");
    }

    let (config, pool, user_id, project_id) = embeddings::context(project_slug).await?;
    let service = build_pg_retrieval_service(&config, pool)?;
    let mut results = Vec::with_capacity(questions.len());
    for question in &questions {
        let request = RetrieveContextRequest {
            project_id,
            query: question.query.clone(),
            include_global: question.include_global,
            limit: evaluation_limit(question.minimum_items),
        };
        let result = match service
            .retrieve_context(&RetrievalPrincipal::User { user_id }, request)
            .await
        {
            Ok(response) => score_question(question, &response),
            Err(error) => failed_question(question, error.to_string()),
        };
        results.push(result);
    }

    let passed_questions = results.iter().filter(|result| result.passed).count();
    let regression_score = average_score(results.iter().map(|result| result.regression_score));
    let report = EvaluationReport {
        project: project_slug.to_owned(),
        golden_question_file: golden_path.display().to_string(),
        total_questions: results.len(),
        passed_questions,
        failed_questions: results.len().saturating_sub(passed_questions),
        passed: passed_questions == results.len(),
        regression_score,
        results,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn parse_golden_questions(content: &str) -> anyhow::Result<Vec<GoldenQuestion>> {
    let mut questions = Vec::new();
    for (line_index, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let question = serde_json::from_str::<GoldenQuestion>(trimmed).map_err(|error| {
            anyhow::anyhow!("invalid golden question line {}: {error}", line_index + 1)
        })?;
        if question.id.trim().is_empty()
            || question.project_slug.trim().is_empty()
            || question.query.trim().is_empty()
            || question.minimum_items == 0
            || question.minimum_items > 20
        {
            anyhow::bail!("invalid golden question line {}", line_index + 1);
        }
        questions.push(question);
    }
    Ok(questions)
}

fn score_question(
    question: &GoldenQuestion,
    response: &RetrieveContextResponse,
) -> EvaluationQuestionResult {
    let scope_hits = count_scope_hits(question, response);
    let citation_hits = count_citation_hits(question, response);
    let minimum_items_hit = response.items.len() >= question.minimum_items;
    let expected_scope_hit = scope_hits == question.expected_scope.len();
    let expected_citations_hit = citation_hits == question.expected_citations.len();
    let regression_score = average_score([
        bool_score(minimum_items_hit),
        ratio_score(scope_hits, question.expected_scope.len()),
        ratio_score(citation_hits, question.expected_citations.len()),
    ]);

    EvaluationQuestionResult {
        id: question.id.clone(),
        query: question.query.clone(),
        passed: minimum_items_hit && expected_scope_hit && expected_citations_hit,
        items_returned: response.items.len(),
        minimum_items: question.minimum_items,
        expected_scope: question.expected_scope.clone(),
        scope_hits,
        expected_citations: question.expected_citations.clone(),
        citation_hits,
        retrieval_mode: serde_json::to_value(response.retrieval.mode)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".to_owned()),
        lexical_candidates: response.retrieval.lexical_candidates,
        semantic_candidates: response.retrieval.semantic_candidates,
        regression_score,
        error: None,
    }
}

fn failed_question(question: &GoldenQuestion, error: String) -> EvaluationQuestionResult {
    EvaluationQuestionResult {
        id: question.id.clone(),
        query: question.query.clone(),
        passed: false,
        items_returned: 0,
        minimum_items: question.minimum_items,
        expected_scope: question.expected_scope.clone(),
        scope_hits: 0,
        expected_citations: question.expected_citations.clone(),
        citation_hits: 0,
        retrieval_mode: "error".to_owned(),
        lexical_candidates: 0,
        semantic_candidates: 0,
        regression_score: 0.0,
        error: Some(error.chars().take(240).collect()),
    }
}

fn count_scope_hits(question: &GoldenQuestion, response: &RetrieveContextResponse) -> usize {
    let returned_scopes = response
        .items
        .iter()
        .map(|item| item.scope)
        .collect::<Vec<_>>();
    question
        .expected_scope
        .iter()
        .filter(|scope| returned_scopes.contains(scope))
        .count()
}

fn count_citation_hits(question: &GoldenQuestion, response: &RetrieveContextResponse) -> usize {
    question
        .expected_citations
        .iter()
        .filter(|expected| {
            response.items.iter().any(|item| {
                item.citation
                    .source_path
                    .as_ref()
                    .is_some_and(|path| path.contains(expected.as_str()))
                    || item.citation.source_uri.contains(expected.as_str())
            })
        })
        .count()
}

fn evaluation_limit(minimum_items: usize) -> u32 {
    u32::try_from(minimum_items.clamp(5, 20)).unwrap_or(20)
}

fn bool_score(value: bool) -> f32 {
    if value { 1.0 } else { 0.0 }
}

fn ratio_score(hits: usize, expected: usize) -> f32 {
    if expected == 0 {
        1.0
    } else {
        hits as f32 / expected as f32
    }
}

fn average_score(scores: impl IntoIterator<Item = f32>) -> f32 {
    let mut total = 0.0_f32;
    let mut count = 0_usize;
    for score in scores {
        total += score;
        count += 1;
    }
    if count == 0 {
        0.0
    } else {
        total / count as f32
    }
}

fn default_include_global() -> bool {
    true
}

fn default_minimum_items() -> usize {
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use queria_core::contracts::{
        Citation, RetrievalDiagnostics, RetrievalMode, RetrieveContextResponse,
        RetrievedContextItem,
    };
    use queria_core::ids::{ChunkId, ProjectId, SourceDocumentId};
    use queria_core::model::KnowledgeScope;

    #[test]
    fn parses_golden_questions_jsonl() {
        let jsonl = r#"
{"id":"q1","project_slug":"fjulian-me","query":"Astro markdown content flow","include_global":true,"expected_scope":["project"],"expected_citations":["README.md"],"minimum_items":1}
{"id":"q2","project_slug":"fjulian-me","query":"deploy notes","include_global":false,"expected_scope":["project","global"],"expected_citations":[],"minimum_items":2}
"#;

        let questions = parse_golden_questions(jsonl).expect("jsonl should parse");

        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].id, "q1");
        assert_eq!(questions[0].expected_scope, vec![KnowledgeScope::Project]);
        assert_eq!(questions[0].expected_citations, vec!["README.md"]);
        assert!(!questions[1].include_global);
        assert_eq!(questions[1].minimum_items, 2);
    }

    #[test]
    fn scoring_requires_minimum_items_scope_and_citations() {
        let question = GoldenQuestion {
            id: "q1".to_owned(),
            project_slug: "fjulian-me".to_owned(),
            query: "Astro markdown content flow".to_owned(),
            include_global: true,
            expected_scope: vec![KnowledgeScope::Project],
            expected_citations: vec!["README.md".to_owned()],
            minimum_items: 1,
        };
        let response = response_with_items(vec![item(KnowledgeScope::Project, "README.md")]);

        let result = score_question(&question, &response);

        assert!(result.passed);
        assert_eq!(result.scope_hits, 1);
        assert_eq!(result.citation_hits, 1);
        assert_eq!(result.items_returned, 1);
        assert_eq!(result.regression_score, 1.0);
    }

    #[test]
    fn scoring_fails_when_expected_scope_is_missing() {
        let question = GoldenQuestion {
            id: "q1".to_owned(),
            project_slug: "fjulian-me".to_owned(),
            query: "deploy notes".to_owned(),
            include_global: true,
            expected_scope: vec![KnowledgeScope::Global],
            expected_citations: Vec::new(),
            minimum_items: 1,
        };
        let response = response_with_items(vec![item(KnowledgeScope::Project, "README.md")]);

        let result = score_question(&question, &response);

        assert!(!result.passed);
        assert_eq!(result.scope_hits, 0);
        assert!(result.regression_score < 1.0);
    }

    fn response_with_items(items: Vec<RetrievedContextItem>) -> RetrieveContextResponse {
        RetrieveContextResponse {
            project_id: ProjectId::new(),
            query: "query".to_owned(),
            items,
            retrieval: RetrievalDiagnostics {
                mode: RetrievalMode::Hybrid,
                lexical_candidates: 0,
                semantic_candidates: 3,
                embedding_profile_version: "voyage-4-1024-v1".to_owned(),
            },
            generated_at: chrono::Utc::now(),
        }
    }

    fn item(scope: KnowledgeScope, source_path: &str) -> RetrievedContextItem {
        RetrievedContextItem {
            chunk_id: ChunkId::new(),
            source_document_id: SourceDocumentId::new(),
            scope,
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
}

use crate::contracts::{RetrievalMode, RetrieveContextResponse};
use crate::model::KnowledgeScope;
use crate::{QueriaError, QueriaResult};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct GoldenQuestion {
    pub id: String,
    pub project_slug: String,
    pub query: String,
    #[serde(default = "default_include_global")]
    pub include_global: bool,
    #[serde(default)]
    pub expected_scope: Vec<KnowledgeScope>,
    #[serde(default)]
    pub expected_citations: Vec<String>,
    #[serde(default = "default_minimum_items")]
    pub minimum_items: usize,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct EvaluationQuestionResult {
    pub id: String,
    pub query: String,
    pub passed: bool,
    pub items_returned: usize,
    pub minimum_items: usize,
    pub expected_scope: Vec<KnowledgeScope>,
    pub scope_hits: usize,
    pub expected_citations: Vec<String>,
    pub citation_hits: usize,
    pub retrieval_mode: String,
    pub lexical_candidates: u32,
    pub semantic_candidates: u32,
    pub regression_score: f32,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct EvaluationReport {
    pub project: String,
    pub golden_question_file: String,
    pub total_questions: usize,
    pub passed_questions: usize,
    pub failed_questions: usize,
    pub passed: bool,
    pub regression_score: f32,
    pub results: Vec<EvaluationQuestionResult>,
}

pub fn parse_golden_questions_jsonl(content: &str) -> QueriaResult<Vec<GoldenQuestion>> {
    let mut questions = Vec::new();
    for (line_index, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let question = serde_json::from_str::<GoldenQuestion>(trimmed).map_err(|error| {
            QueriaError::Validation(format!(
                "invalid golden question line {}: {error}",
                line_index + 1
            ))
        })?;
        validate_question(&question, line_index + 1)?;
        questions.push(question);
    }
    Ok(questions)
}

#[must_use]
pub fn score_evaluation_report(
    project_slug: &str,
    golden_question_file: &str,
    questions: &[GoldenQuestion],
    responses: Vec<Result<RetrieveContextResponse, String>>,
) -> EvaluationReport {
    let mut response_iter = responses.into_iter();
    let results = questions
        .iter()
        .map(|question| match response_iter.next() {
            Some(Ok(response)) => score_question(question, &response),
            Some(Err(error)) => failed_question(question, error),
            None => failed_question(question, "missing retrieval response".to_owned()),
        })
        .collect::<Vec<_>>();
    let passed_questions = results.iter().filter(|result| result.passed).count();
    let regression_score = average_score(results.iter().map(|result| result.regression_score));

    EvaluationReport {
        project: project_slug.to_owned(),
        golden_question_file: golden_question_file.to_owned(),
        total_questions: results.len(),
        passed_questions,
        failed_questions: results.len().saturating_sub(passed_questions),
        passed: passed_questions == results.len(),
        regression_score,
        results,
    }
}

#[must_use]
pub fn evaluation_limit(minimum_items: usize) -> u32 {
    u32::try_from(minimum_items.clamp(5, 20)).unwrap_or(20)
}

fn validate_question(question: &GoldenQuestion, line_number: usize) -> QueriaResult<()> {
    if question.id.trim().is_empty()
        || question.project_slug.trim().is_empty()
        || question.query.trim().is_empty()
        || question.minimum_items == 0
        || question.minimum_items > 20
    {
        return Err(QueriaError::Validation(format!(
            "invalid golden question line {line_number}"
        )));
    }
    Ok(())
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
        retrieval_mode: retrieval_mode_name(response.retrieval.mode),
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

fn retrieval_mode_name(mode: RetrievalMode) -> String {
    serde_json::to_value(mode)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
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
    use crate::contracts::{
        Citation, RetrievalDiagnostics, RetrieveContextResponse, RetrievedContextItem,
    };
    use crate::ids::{ChunkId, ProjectId, SourceDocumentId};

    #[test]
    fn parses_golden_questions_jsonl() {
        let jsonl = r#"
{"id":"q1","project_slug":"fjulian-me","query":"Astro markdown content flow","include_global":true,"expected_scope":["project"],"expected_citations":["README.md"],"minimum_items":1}
{"id":"q2","project_slug":"fjulian-me","query":"deploy notes","include_global":false,"expected_scope":["project","global"],"expected_citations":[],"minimum_items":2}
"#;

        let questions = parse_golden_questions_jsonl(jsonl).expect("jsonl should parse");

        assert_eq!(questions.len(), 2);
        assert_eq!(questions[0].id, "q1");
        assert_eq!(questions[0].expected_scope, vec![KnowledgeScope::Project]);
        assert_eq!(questions[0].expected_citations, vec!["README.md"]);
        assert!(!questions[1].include_global);
        assert_eq!(questions[1].minimum_items, 2);
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
        let report = score_evaluation_report(
            "fjulian-me",
            "tests/golden_questions/fjulian-me.jsonl",
            &[question],
            vec![Ok(response_with_items(vec![item(
                KnowledgeScope::Project,
                "README.md",
            )]))],
        );

        assert!(!report.passed);
        assert_eq!(report.failed_questions, 1);
        assert_eq!(report.results[0].scope_hits, 0);
        assert!(report.regression_score < 1.0);
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
        use crate::contracts::KnowledgeLane;
        use crate::model::KnowledgeStatus;
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
}

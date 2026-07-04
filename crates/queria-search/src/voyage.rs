use crate::embedding::{EmbeddingDocument, EmbeddingProvider, EmbeddingVector};
use async_trait::async_trait;
use queria_core::{QueriaError, QueriaResult};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://api.voyageai.com/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VoyageInputType {
    Document,
    Query,
}

impl VoyageInputType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Query => "query",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
struct VoyageRequest {
    input: Vec<String>,
    model: String,
    input_type: &'static str,
    output_dimension: usize,
}

impl VoyageRequest {
    fn new(
        input: Vec<String>,
        model: &str,
        input_type: VoyageInputType,
        output_dimension: usize,
    ) -> Self {
        Self {
            input,
            model: model.to_owned(),
            input_type: input_type.as_str(),
            output_dimension,
        }
    }
}

#[derive(Debug, Deserialize)]
struct VoyageResponse {
    data: Vec<VoyageEmbedding>,
}

#[derive(Debug, Deserialize)]
struct VoyageEmbedding {
    embedding: Vec<f32>,
    index: usize,
}

#[derive(Clone)]
pub struct VoyageClient {
    client: Client,
    base_url: String,
    api_key: String,
    model: String,
    dimension: usize,
    max_retries: u32,
}

impl VoyageClient {
    pub fn new(
        api_key: String,
        model: String,
        dimension: usize,
        timeout: Duration,
        max_retries: u32,
    ) -> QueriaResult<Self> {
        Self::with_base_url(
            DEFAULT_BASE_URL.to_owned(),
            api_key,
            model,
            dimension,
            timeout,
            max_retries,
        )
    }

    pub fn with_base_url(
        base_url: String,
        api_key: String,
        model: String,
        dimension: usize,
        timeout: Duration,
        max_retries: u32,
    ) -> QueriaResult<Self> {
        if api_key.trim().is_empty() || model.trim().is_empty() || dimension == 0 {
            return Err(QueriaError::Config(
                "Voyage API key, model, and dimension are required".to_owned(),
            ));
        }
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|error| {
                QueriaError::Config(format!("failed to build Voyage HTTP client: {error}"))
            })?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_owned(),
            api_key,
            model,
            dimension,
            max_retries,
        })
    }

    async fn embed(
        &self,
        input: Vec<String>,
        input_type: VoyageInputType,
    ) -> QueriaResult<Vec<EmbeddingVector>> {
        if input.is_empty() {
            return Ok(Vec::new());
        }
        let expected_count = input.len();
        let request = VoyageRequest::new(input, &self.model, input_type, self.dimension);
        let mut attempt = 0_u32;

        loop {
            let response = self
                .client
                .post(format!("{}/embeddings", self.base_url))
                .bearer_auth(&self.api_key)
                .json(&request)
                .send()
                .await
                .map_err(|error| {
                    QueriaError::Infrastructure(format!("Voyage request failed: {error}"))
                })?;
            let status = response.status();
            let request_id = response
                .headers()
                .get("request-id")
                .and_then(|value| value.to_str().ok())
                .unwrap_or("unavailable")
                .to_owned();

            if status.is_success() {
                let mut payload: VoyageResponse = response.json().await.map_err(|error| {
                    QueriaError::Infrastructure(format!(
                        "Voyage returned an invalid response: {error}"
                    ))
                })?;
                if payload.data.len() != expected_count {
                    return Err(QueriaError::Infrastructure(
                        "Voyage response count did not match request count".to_owned(),
                    ));
                }
                payload.data.sort_by_key(|item| item.index);
                return payload
                    .data
                    .into_iter()
                    .map(|item| EmbeddingVector::new(item.embedding, self.dimension))
                    .collect();
            }

            let retryable = status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error();
            if retryable && attempt < self.max_retries {
                let delay_ms = 50_u64.saturating_mul(1_u64 << attempt.min(6));
                attempt += 1;
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                continue;
            }
            return Err(QueriaError::Infrastructure(format!(
                "Voyage request failed with status {status}; request_id={request_id}"
            )));
        }
    }
}

#[async_trait]
impl EmbeddingProvider for VoyageClient {
    async fn embed_documents(
        &self,
        inputs: &[EmbeddingDocument],
    ) -> QueriaResult<Vec<EmbeddingVector>> {
        self.embed(
            inputs.iter().map(|input| input.text.clone()).collect(),
            VoyageInputType::Document,
        )
        .await
    }

    async fn embed_query(&self, query: &str) -> QueriaResult<EmbeddingVector> {
        if query.trim().is_empty() {
            return Err(QueriaError::Validation(
                "embedding query must not be blank".to_owned(),
            ));
        }
        self.embed(vec![query.to_owned()], VoyageInputType::Query)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| {
                QueriaError::Infrastructure("Voyage returned no query embedding".to_owned())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_and_query_requests_use_distinct_input_types() {
        let documents = VoyageRequest::new(
            vec!["one".to_owned()],
            "voyage-4",
            VoyageInputType::Document,
            1024,
        );
        let query = VoyageRequest::new(
            vec!["one".to_owned()],
            "voyage-4",
            VoyageInputType::Query,
            1024,
        );

        assert_eq!(documents.input_type, "document");
        assert_eq!(query.input_type, "query");
    }
}

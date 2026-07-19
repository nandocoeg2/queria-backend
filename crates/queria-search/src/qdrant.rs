use crate::embedding::{
    VectorCandidate, VectorIndex, VectorIndexHealth, VectorPoint, VectorSearchRequest,
};
use async_trait::async_trait;
use queria_core::ids::ChunkId;
use queria_core::{QueriaError, QueriaResult};
use reqwest::{Client, RequestBuilder, StatusCode};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QdrantConfig {
    pub url: String,
    pub api_key: String,
    pub collection: String,
    pub vector_name: String,
    pub dimension: usize,
}

#[derive(Clone)]
pub struct QdrantClient {
    client: Client,
    config: QdrantConfig,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    result: Vec<ScoredPoint>,
}

#[derive(Debug, Deserialize)]
struct ScoredPoint {
    id: Value,
    score: f32,
}

#[derive(Debug, Deserialize)]
struct CollectionResponse {
    result: CollectionInfo,
}

#[derive(Debug, Deserialize)]
struct CollectionInfo {
    #[serde(default)]
    points_count: u64,
}

impl QdrantClient {
    pub fn new(config: QdrantConfig) -> QueriaResult<Self> {
        if config.url.trim().is_empty()
            || config.collection.trim().is_empty()
            || config.vector_name.trim().is_empty()
            || config.dimension == 0
            || !config
                .collection
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        {
            return Err(QueriaError::Config(
                "invalid Qdrant URL, collection, vector name, or dimension".to_owned(),
            ));
        }
        Ok(Self {
            client: Client::new(),
            config: QdrantConfig {
                url: config.url.trim_end_matches('/').to_owned(),
                ..config
            },
        })
    }

    fn request(&self, builder: RequestBuilder) -> RequestBuilder {
        if self.config.api_key.is_empty() {
            builder
        } else {
            builder.header("api-key", &self.config.api_key)
        }
    }

    fn collection_url(&self) -> String {
        format!("{}/collections/{}", self.config.url, self.config.collection)
    }

    async fn delete_payload_index(&self, field_name: &str) -> QueriaResult<()> {
        let response = self
            .request(self.client.delete(format!(
                "{}/index/{}?wait=true",
                self.collection_url(),
                field_name
            )))
            .send()
            .await
            .map_err(qdrant_transport_error)?;
        if response.status().is_success() || response.status() == StatusCode::NOT_FOUND {
            Ok(())
        } else {
            Err(qdrant_status_error("delete payload index", &response))
        }
    }

    async fn ensure_payload_index(&self, field_name: &str) -> QueriaResult<()> {
        let response = self
            .request(
                self.client
                    .put(format!("{}/index?wait=true", self.collection_url()))
                    .json(&json!({
                        "field_name": field_name,
                        "field_schema": payload_index_schema(field_name)
                    })),
            )
            .send()
            .await
            .map_err(qdrant_transport_error)?;
        if response.status().is_success() || response.status() == StatusCode::CONFLICT {
            Ok(())
        } else {
            Err(qdrant_status_error("create payload index", &response))
        }
    }
}

#[async_trait]
impl VectorIndex for QdrantClient {
    async fn ensure_collection(&self) -> QueriaResult<()> {
        let response = self
            .request(self.client.get(self.collection_url()))
            .send()
            .await
            .map_err(qdrant_transport_error)?;
        if response.status() == StatusCode::NOT_FOUND {
            let create_response = self
                .request(self.client.put(self.collection_url()).json(&json!({
                    "vectors": {
                        &self.config.vector_name: {
                            "size": self.config.dimension,
                            "distance": "Cosine"
                        }
                    }
                })))
                .send()
                .await
                .map_err(qdrant_transport_error)?;
            if !create_response.status().is_success()
                && create_response.status() != StatusCode::CONFLICT
            {
                return Err(qdrant_status_error("create collection", &create_response));
            }
        } else if !response.status().is_success() {
            return Err(qdrant_status_error("inspect collection", &response));
        }

        for field in [
            "organization_id",
            "project_id",
            "scope",
            "embedding_profile_version",
        ] {
            self.ensure_payload_index(field).await?;
        }
        self.delete_payload_index("is_active").await?;
        self.ensure_payload_index("is_active").await?;
        Ok(())
    }

    async fn upsert(&self, points: &[VectorPoint]) -> QueriaResult<()> {
        if points.is_empty() {
            return Ok(());
        }
        let points = points
            .iter()
            .map(|point| {
                json!({
                    "id": point.id,
                    "vector": {
                        &self.config.vector_name: point.vector.values()
                    },
                    "payload": point.payload
                })
            })
            .collect::<Vec<_>>();
        let response = self
            .request(
                self.client
                    .put(format!("{}/points?wait=true", self.collection_url()))
                    .json(&json!({ "points": points })),
            )
            .send()
            .await
            .map_err(qdrant_transport_error)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(qdrant_status_error("upsert points", &response))
        }
    }

    async fn search(&self, request: VectorSearchRequest) -> QueriaResult<Vec<VectorCandidate>> {
        let response = self
            .request(
                self.client
                    .post(format!("{}/points/search", self.collection_url()))
                    .json(&json!({
                        "vector": {
                            "name": self.config.vector_name,
                            "vector": request.vector.values()
                        },
                        "filter": search_filter(
                            request.organization_id,
                            request.project_id,
                            request.include_global,
                            &request.embedding_profile_version
                        ),
                        "limit": request.limit,
                        "with_payload": false,
                        "with_vector": false
                    })),
            )
            .send()
            .await
            .map_err(qdrant_transport_error)?;
        if !response.status().is_success() {
            return Err(qdrant_status_error("search points", &response));
        }
        let payload: SearchResponse = response.json().await.map_err(|error| {
            QueriaError::Infrastructure(format!("Qdrant returned an invalid response: {error}"))
        })?;
        payload
            .result
            .into_iter()
            .map(|point| {
                let id = point
                    .id
                    .as_str()
                    .ok_or_else(|| {
                        QueriaError::Infrastructure(
                            "Qdrant returned a non-UUID point ID".to_owned(),
                        )
                    })?
                    .parse::<Uuid>()
                    .map_err(|_| {
                        QueriaError::Infrastructure(
                            "Qdrant returned an invalid UUID point ID".to_owned(),
                        )
                    })?;
                Ok(VectorCandidate {
                    chunk_id: ChunkId::from_uuid(id),
                    score: point.score,
                })
            })
            .collect()
    }

    async fn delete(&self, point_ids: &[Uuid]) -> QueriaResult<()> {
        if point_ids.is_empty() {
            return Ok(());
        }
        let response = self
            .request(
                self.client
                    .post(format!("{}/points/delete?wait=true", self.collection_url()))
                    .json(&json!({ "points": point_ids })),
            )
            .send()
            .await
            .map_err(qdrant_transport_error)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(qdrant_status_error("delete points", &response))
        }
    }

    async fn health(&self) -> QueriaResult<VectorIndexHealth> {
        let response = self
            .request(self.client.get(self.collection_url()))
            .send()
            .await
            .map_err(qdrant_transport_error)?;
        if !response.status().is_success() {
            return Err(qdrant_status_error("inspect collection", &response));
        }
        let payload: CollectionResponse = response.json().await.map_err(|error| {
            QueriaError::Infrastructure(format!("Qdrant returned an invalid response: {error}"))
        })?;
        Ok(VectorIndexHealth {
            collection: self.config.collection.clone(),
            points_count: payload.result.points_count,
        })
    }
}

fn search_filter(
    organization_id: Uuid,
    project_id: Uuid,
    include_global: bool,
    embedding_profile_version: &str,
) -> Value {
    // Status is NOT on Qdrant payload (no migration). inactive/needs_review/scratch may
    // appear among dense candidates; PG HYDRATE_SQL enforces status/lane gates afterward.
    // When include_needs_review is false, oversampling may still return NR ids that hydrate drops.
    let mut should = vec![json!({
        "key": "project_id",
        "match": { "value": project_id }
    })];
    if include_global {
        should.push(json!({
            "key": "scope",
            "match": { "value": "global" }
        }));
    }
    json!({
        "must": [
            {
                "key": "organization_id",
                "match": { "value": organization_id }
            },
            {
                "key": "embedding_profile_version",
                "match": { "value": embedding_profile_version }
            },
            {
                "key": "is_active",
                "match": { "value": true }
            }
        ],
        "should": should
    })
}

fn payload_index_schema(field_name: &str) -> Value {
    match field_name {
        "is_active" => json!("bool"),
        _ => json!("keyword"),
    }
}

fn qdrant_transport_error(error: reqwest::Error) -> QueriaError {
    QueriaError::Infrastructure(format!("Qdrant request failed: {error}"))
}

fn qdrant_status_error(operation: &str, response: &reqwest::Response) -> QueriaError {
    QueriaError::Infrastructure(format!(
        "Qdrant {operation} failed with status {}",
        response.status()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_filter_includes_global_only_when_requested() {
        let organization_id = Uuid::now_v7();
        let project_id = Uuid::now_v7();

        let project_only = search_filter(organization_id, project_id, false, "voyage-4-1024-v1");
        let with_global = search_filter(organization_id, project_id, true, "voyage-4-1024-v1");

        assert_eq!(project_only["should"].as_array().map(Vec::len), Some(1));
        assert_eq!(with_global["should"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn is_active_payload_index_uses_bool_schema() {
        assert_eq!(payload_index_schema("is_active"), json!("bool"));
        assert_eq!(payload_index_schema("project_id"), json!("keyword"));
    }
}

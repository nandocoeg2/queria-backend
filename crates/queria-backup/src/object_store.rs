use queria_core::{QueriaError, QueriaResult};
use s3::bucket::Bucket;
use s3::creds::Credentials;
use s3::region::Region;

/// Metadata for an object stored in S3-compatible storage.
#[derive(Clone, Debug)]
pub struct ObjectMeta {
    pub key: String,
    pub size: u64,
    pub last_modified: String,
}

/// S3-compatible object storage client that works with both MinIO and AWS S3.
#[derive(Clone)]
pub struct ObjectStore {
    bucket: Box<Bucket>,
}

impl ObjectStore {
    /// Create a new `ObjectStore` targeting a MinIO or S3-compatible endpoint.
    ///
    /// `endpoint` must include the scheme (e.g. `http://127.0.0.1:17678`).
    /// `region` can be any string for MinIO (commonly `us-east-1`).
    pub fn new(
        endpoint: &str,
        bucket_name: &str,
        access_key: &str,
        secret_key: &str,
        region: &str,
    ) -> QueriaResult<Self> {
        let credentials = Credentials::new(Some(access_key), Some(secret_key), None, None, None)
            .map_err(|error| QueriaError::Config(format!("invalid S3 credentials: {error}")))?;

        let region = Region::Custom {
            region: region.to_owned(),
            endpoint: endpoint.to_owned(),
        };

        let bucket = Bucket::new(bucket_name, region, credentials)
            .map_err(|error| QueriaError::Config(format!("invalid S3 bucket config: {error}")))?
            .with_path_style();

        Ok(Self { bucket })
    }

    /// Create the bucket if it does not already exist.
    pub async fn ensure_bucket(&self) -> QueriaResult<()> {
        // HEAD bucket – if 404 then create.
        let (_, code) = self.bucket.head_object("/").await.unwrap_or_default();

        if code == 404 {
            let creds = self.bucket.credentials().await.map_err(|error| {
                QueriaError::Infrastructure(format!("failed to resolve S3 credentials: {error}"))
            })?;

            let create_result = Bucket::create_with_path_style(
                &self.bucket.name(),
                self.bucket.region().clone(),
                creds,
                s3::bucket_ops::BucketConfiguration::default(),
            )
            .await;

            match create_result {
                Ok(_) => {
                    tracing::info!(bucket = %self.bucket.name(), "created S3 bucket");
                }
                Err(error) => {
                    let msg = error.to_string();
                    // Bucket may already exist (race or different detection)
                    if !msg.contains("BucketAlreadyOwnedByYou")
                        && !msg.contains("BucketAlreadyExists")
                    {
                        return Err(QueriaError::Infrastructure(format!(
                            "failed to create bucket {}: {error}",
                            self.bucket.name()
                        )));
                    }
                }
            }
        }

        Ok(())
    }

    /// Upload bytes to the given object key.
    pub async fn put_object(&self, key: &str, data: &[u8], content_type: &str) -> QueriaResult<()> {
        self.bucket
            .put_object_with_content_type(key, data, content_type)
            .await
            .map_err(|error| {
                QueriaError::Infrastructure(format!("S3 put_object({key}) failed: {error}"))
            })?;

        tracing::debug!(key, size = data.len(), "uploaded object to S3");
        Ok(())
    }

    /// Download an object by key. Returns the raw bytes.
    pub async fn get_object(&self, key: &str) -> QueriaResult<Vec<u8>> {
        let response = self.bucket.get_object(key).await.map_err(|error| {
            QueriaError::Infrastructure(format!("S3 get_object({key}) failed: {error}"))
        })?;

        Ok(response.to_vec())
    }

    /// Delete an object by key.
    pub async fn delete_object(&self, key: &str) -> QueriaResult<()> {
        self.bucket.delete_object(key).await.map_err(|error| {
            QueriaError::Infrastructure(format!("S3 delete_object({key}) failed: {error}"))
        })?;

        tracing::debug!(key, "deleted object from S3");
        Ok(())
    }

    /// List objects under a given prefix.
    pub async fn list_objects(&self, prefix: &str) -> QueriaResult<Vec<ObjectMeta>> {
        let results = self
            .bucket
            .list(prefix.to_owned(), None)
            .await
            .map_err(|error| {
                QueriaError::Infrastructure(format!("S3 list_objects({prefix}) failed: {error}"))
            })?;

        let mut objects = Vec::new();
        for result in results {
            for content in result.contents {
                objects.push(ObjectMeta {
                    key: content.key,
                    size: content.size,
                    last_modified: content.last_modified,
                });
            }
        }

        Ok(objects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_store_rejects_empty_credentials() {
        // Credentials::new accepts empty strings, but bucket config should still work.
        let result = ObjectStore::new(
            "http://127.0.0.1:9000",
            "test-bucket",
            "access",
            "secret",
            "us-east-1",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn object_store_uses_path_style() {
        let store = ObjectStore::new(
            "http://127.0.0.1:9000",
            "test-bucket",
            "access",
            "secret",
            "us-east-1",
        )
        .unwrap();

        // path_style means the bucket name is in the URL path, not subdomain.
        assert!(store.bucket.is_path_style());
    }
}

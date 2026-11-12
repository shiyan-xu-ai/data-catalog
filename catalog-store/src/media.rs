//! Short-lived presigned https URLs for media objects (videos referenced by table rows), so the
//! browser reads them from object storage directly instead of through the catalog.

use std::str::FromStr;
use std::time::Duration;

use lance_namespace::Result;
use object_store::aws::{AmazonS3Builder, AmazonS3ConfigKey};
use object_store::path::Path;
use object_store::signer::{Method, Signer};

use crate::namespace::{internal, invalid_input, PostgresNamespace};

impl PostgresNamespace {
    /// A GET URL for `uri` (`s3://bucket/key`) signed with the catalog's storage credentials (the
    /// `AWS_*` environment, overridden by its storage options) and valid for `expires_in`. Errors
    /// with `InvalidInput` for anything but an `s3://bucket/key` URI. Signing sends no request, so
    /// a missing object only fails when the URL is fetched.
    pub async fn presign_media(&self, uri: &str, expires_in: Duration) -> Result<String> {
        let invalid = || invalid_input(format!("{uri} is not an s3://bucket/key URI"));
        let (bucket, key) = uri
            .strip_prefix("s3://")
            .and_then(|rest| rest.split_once('/'))
            .filter(|(bucket, _)| !bucket.is_empty())
            .ok_or_else(invalid)?;
        let path = Path::parse(key).map_err(|_| invalid())?;
        let mut builder = AmazonS3Builder::from_env().with_bucket_name(bucket);
        for (name, value) in &self.storage_options {
            if let Ok(key) = AmazonS3ConfigKey::from_str(name) {
                builder = builder.with_config(key, value);
            }
        }
        let store = builder
            .build()
            .map_err(|e| internal(format!("configure S3 access for {uri}: {e}")))?;
        let url = store
            .signed_url(Method::GET, &path, expires_in)
            .await
            .map_err(|e| internal(format!("sign {uri}: {e}")))?;
        Ok(url.to_string())
    }
}

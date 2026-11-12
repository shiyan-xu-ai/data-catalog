//! Short-lived presigned https URLs for media objects (videos referenced by table rows), so the
//! browser reads them from object storage directly instead of through the catalog.

use std::str::FromStr;
use std::time::Duration;

use lance_namespace::Result;
use object_store::aws::{AmazonS3Builder, AmazonS3ConfigKey};
use object_store::path::Path;
use object_store::signer::{Method, SignedUrlOptions, Signer};

use crate::namespace::{internal, invalid_input, PostgresNamespace};

/// Presigned GET URLs for one media object.
#[derive(Debug, Clone)]
pub struct MediaUrls {
    /// Reads the object as stored (supports byte ranges).
    pub url: String,
    /// Same object, served with `Content-Disposition: attachment` so browsers save it.
    pub download_url: String,
}

impl PostgresNamespace {
    /// GET URLs for `uri` (`s3://bucket/key`) signed with the catalog's storage credentials (the
    /// `AWS_*` environment, overridden by its storage options) and valid for `expires_in`. Errors
    /// with `InvalidInput` for anything but an `s3://bucket/key` URI. Signing sends no request, so
    /// a missing object only fails when a URL is fetched.
    pub async fn presign_media(&self, uri: &str, expires_in: Duration) -> Result<MediaUrls> {
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
        let sign = |options: SignedUrlOptions| {
            let store = &store;
            let path = &path;
            async move {
                store
                    .signed_url_opts(Method::GET, path, expires_in, &options)
                    .await
                    .map(|url| url.to_string())
                    .map_err(|e| internal(format!("sign {uri}: {e}")))
            }
        };
        let name: String = path
            .filename()
            .unwrap_or("download")
            .chars()
            .map(|c| {
                if c == '"' || c == '\\' || c.is_control() {
                    '_'
                } else {
                    c
                }
            })
            .collect();
        let disposition = format!("attachment; filename=\"{name}\"");
        Ok(MediaUrls {
            url: sign(SignedUrlOptions::default()).await?,
            download_url: sign(
                SignedUrlOptions::default()
                    .with_query([("response-content-disposition", disposition)]),
            )
            .await?,
        })
    }
}

//! AWS credential hydration from GCP Secret Manager for Cloud Run.
//!
//! Cloud Run runs under a *GCP* service account, but the catalog's data lives in *AWS* S3, so
//! there is no ambient AWS credential the way IRSA provided one on EKS. The operator stores the
//! AWS keys as app secrets (`apps-platform app secret set AWS_ACCESS_KEY_ID ...`), which land in
//! Secret Manager as `<service>-aws-access-key-id` etc. The platform does NOT inject them as env
//! vars — the app reads them at runtime. This module performs that read and returns them as
//! `object_store` S3 options, merged into the store builders.
//!
//! Locally (and in CI/tests) there is no metadata server: if `AWS_ACCESS_KEY_ID` is already in
//! the environment (real S3, MinIO, or a dev keypair), or the service-name prefix can't be
//! determined, this is a no-op and the existing env-based credential path is used unchanged.

use anyhow::{Context, Result};
use base64::Engine;
use serde::Deserialize;

const METADATA_BASE: &str = "http://metadata.google.internal/computeMetadata/v1";
const SECRET_MANAGER_BASE: &str = "https://secretmanager.googleapis.com/v1";

/// Required AWS option keys (object_store env-var form). Boot fails if these are absent from
/// Secret Manager while running on Cloud Run.
const REQUIRED_KEYS: &[&str] = &["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"];
/// Optional AWS keys — fetched if present, skipped on 404. `AWS_DEFAULT_REGION`/endpoint are
/// usually plain (non-secret) `[cloudrun].env_vars` instead, and are picked up from the env.
const OPTIONAL_KEYS: &[&str] = &["AWS_SESSION_TOKEN", "AWS_DEFAULT_REGION"];

/// Fetch AWS credentials from Secret Manager and return them as `object_store` S3 options.
///
/// Returns an empty vec (no-op) when credentials are already in the environment, or when not
/// running on Cloud Run (no `K_SERVICE`/`CATALOG_SECRET_PREFIX`) — the env-based path then
/// applies. On Cloud Run with a prefix set, a missing *required* secret is a hard error so the
/// service fails fast rather than starting unable to reach S3.
pub async fn fetch_aws_secret_opts() -> Result<Vec<(String, String)>> {
    if std::env::var_os("AWS_ACCESS_KEY_ID").is_some() {
        tracing::info!("AWS credentials present in environment; skipping Secret Manager");
        return Ok(Vec::new());
    }
    let Some(prefix) = secret_prefix() else {
        tracing::warn!(
            "no AWS_* env and no K_SERVICE/CATALOG_SECRET_PREFIX; S3 requests will rely on the \
             default credential chain"
        );
        return Ok(Vec::new());
    };

    let client = reqwest::Client::builder()
        .build()
        .context("build HTTP client for Secret Manager")?;
    let token = metadata_token(&client)
        .await
        .context("obtain metadata access token")?;
    let project = metadata_project(&client)
        .await
        .context("obtain GCP project id")?;

    let mut opts = Vec::new();
    for key in REQUIRED_KEYS {
        let value = access_secret(&client, &token, &project, &secret_name(&prefix, key))
            .await?
            .with_context(|| {
                format!("required AWS secret for {key} not found in Secret Manager")
            })?;
        opts.push((key.to_string(), value));
    }
    for key in OPTIONAL_KEYS {
        if let Some(value) =
            access_secret(&client, &token, &project, &secret_name(&prefix, key)).await?
        {
            opts.push((key.to_string(), value));
        }
    }
    tracing::info!(
        count = opts.len(),
        "hydrated AWS credentials from Secret Manager"
    );
    Ok(opts)
}

fn secret_prefix() -> Option<String> {
    std::env::var("CATALOG_SECRET_PREFIX")
        .ok()
        .or_else(|| std::env::var("K_SERVICE").ok())
        .filter(|s| !s.is_empty())
}

/// `<prefix>-<kebab(key)>` — the Secret Manager name `apps-platform app secret set` produces
/// (e.g. `AWS_ACCESS_KEY_ID` for service `lance-catalog` → `lance-catalog-aws-access-key-id`).
fn secret_name(prefix: &str, key: &str) -> String {
    format!("{prefix}-{}", key.to_ascii_lowercase().replace('_', "-"))
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

async fn metadata_token(client: &reqwest::Client) -> Result<String> {
    let resp = client
        .get(format!(
            "{METADATA_BASE}/instance/service-accounts/default/token"
        ))
        .header("Metadata-Flavor", "Google")
        .send()
        .await?
        .error_for_status()?
        .json::<TokenResponse>()
        .await?;
    Ok(resp.access_token)
}

async fn metadata_project(client: &reqwest::Client) -> Result<String> {
    let project = client
        .get(format!("{METADATA_BASE}/project/project-id"))
        .header("Metadata-Flavor", "Google")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    Ok(project)
}

#[derive(Deserialize)]
struct AccessSecretResponse {
    payload: SecretPayload,
}

#[derive(Deserialize)]
struct SecretPayload {
    data: String,
}

/// Read the `latest` version of one secret. `Ok(None)` if it doesn't exist (404); other HTTP
/// failures propagate.
async fn access_secret(
    client: &reqwest::Client,
    token: &str,
    project: &str,
    name: &str,
) -> Result<Option<String>> {
    let url =
        format!("{SECRET_MANAGER_BASE}/projects/{project}/secrets/{name}/versions/latest:access");
    let resp = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .with_context(|| format!("request secret {name}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let body = resp
        .error_for_status()
        .with_context(|| format!("access secret {name}"))?
        .json::<AccessSecretResponse>()
        .await
        .with_context(|| format!("decode secret {name}"))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body.payload.data.trim())
        .with_context(|| format!("base64-decode secret {name}"))?;
    let value = String::from_utf8(bytes).with_context(|| format!("secret {name} is not UTF-8"))?;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_name_kebabs_and_prefixes_like_the_platform() {
        // Mirrors `apps-platform app secret set AWS_ACCESS_KEY_ID` under service `lance-catalog`.
        assert_eq!(
            secret_name("lance-catalog", "AWS_ACCESS_KEY_ID"),
            "lance-catalog-aws-access-key-id"
        );
        assert_eq!(
            secret_name("lance-catalog", "AWS_SECRET_ACCESS_KEY"),
            "lance-catalog-aws-secret-access-key"
        );
    }
}

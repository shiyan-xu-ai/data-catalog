//! Read a small sample of rows from an auxiliary table (lance dataset or parquet directory),
//! so the UI can peek at aux content without any external tooling.
//!
//! Strictly read-only. Lance aux is scanned through the lance scanner (against the entry's
//! `dataset_path`, which for nested bundles points at the openable root); parquet aux dirs go
//! through DataFusion with the sync's object store registered, so the same credentials/endpoint
//! plumbing applies. Row counts are clamped (`MAX_SAMPLE_ROWS`) and the whole operation is
//! timeout-bounded by the caller.

use anyhow::{anyhow, Context, Result};
use arrow_array::RecordBatch;
use catalog_store::SyncConfig;
use datafusion::execution::context::SessionContext;
use futures::TryStreamExt;

pub const DEFAULT_SAMPLE_ROWS: usize = 20;
pub const MAX_SAMPLE_ROWS: usize = 100;

/// Rows (as JSON objects) plus the arrow schema they came with.
pub struct Sample {
    pub rows: Vec<serde_json::Value>,
    pub schema: Vec<serde_json::Value>,
}

/// Sample up to `limit` rows from the lance dataset at `uri`.
pub async fn sample_lance(uri: &str, limit: usize) -> Result<Sample> {
    let dataset = lance::Dataset::open(uri)
        .await
        .with_context(|| format!("open lance dataset {uri}"))?;
    let mut scan = dataset.scan();
    scan.limit(Some(limit as i64), None)
        .context("apply sample limit")?;
    let batches: Vec<RecordBatch> = scan
        .try_into_stream()
        .await
        .context("start lance scan")?
        .try_collect()
        .await
        .context("read lance sample batches")?;
    batches_to_sample(&batches)
}

/// Sample up to `limit` rows from the parquet files under `uri` (a directory), via DataFusion.
/// `sync_cfg` supplies the object store for `s3://` URIs; plain paths read locally.
pub async fn sample_parquet(sync_cfg: &SyncConfig, uri: &str, limit: usize) -> Result<Sample> {
    let ctx = SessionContext::new();
    if let Ok(url) = url::Url::parse(uri) {
        if url.scheme() != "file" {
            // Register the sync's already-configured store for this bucket so DataFusion
            // reads through the same endpoint/credentials.
            let base = url::Url::parse(&format!(
                "{}://{}",
                url.scheme(),
                url.host_str().unwrap_or_default()
            ))
            .context("derive object-store base url")?;
            ctx.register_object_store(&base, sync_cfg.store.clone());
        }
    }
    // Trailing slash => treat as a directory listing of parquet files.
    let dir = format!("{}/", uri.trim_end_matches('/'));
    let df = ctx
        .read_parquet(
            dir,
            datafusion::execution::options::ParquetReadOptions {
                file_extension: ".parquet",
                ..Default::default()
            },
        )
        .await
        .with_context(|| format!("open parquet dir {uri}"))?
        .limit(0, Some(limit))
        .context("apply sample limit")?;
    let batches = df.collect().await.context("read parquet sample batches")?;
    batches_to_sample(&batches)
}

/// Serialize record batches to JSON rows + a name/type schema listing.
fn batches_to_sample(batches: &[RecordBatch]) -> Result<Sample> {
    let schema = batches
        .first()
        .map(|b| {
            b.schema()
                .fields()
                .iter()
                .map(|f| {
                    serde_json::json!({
                        "name": f.name(),
                        "data_type": format!("{:?}", f.data_type()),
                        "nullable": f.is_nullable(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut writer = arrow_json::ArrayWriter::new(Vec::new());
    for batch in batches {
        writer.write(batch).context("serialize sample batch")?;
    }
    writer.finish().context("finish sample serialization")?;
    let bytes = writer.into_inner();
    let rows: Vec<serde_json::Value> = if bytes.is_empty() {
        Vec::new()
    } else {
        serde_json::from_slice(&bytes).map_err(|e| anyhow!("decode sample rows: {e}"))?
    };
    Ok(Sample { rows, schema })
}

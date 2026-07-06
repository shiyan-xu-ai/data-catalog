//! `_catalog/users` Lance table: the set of catalog users, upserted on every sighting.
//!
//! Same JSON-into-Utf8-column pattern as `registry.rs`/`ttl_audit.rs`, but written with Lance's
//! `merge_insert` (upsert) keyed on `email` rather than a wholesale `Overwrite` or an `Append`:
//! a re-sight of a known email UPDATES its row (`last_seen_at`) in place instead of duplicating
//! it, and a first sight INSERTs a new row — all in one commit, so two instances recording the
//! same login concurrently can't both insert a duplicate. Lance's builder retries manifest and
//! semantic commit conflicts internally (`commit_retries`/`conflict_retries`), so a lost update
//! under contention is retried rather than silently dropped.

use std::sync::Arc;

use anyhow::{Context, Result};
use arrow_array::{Array, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use chrono::{DateTime, Utc};
use lance::dataset::{MergeInsertBuilder, WhenMatched, WhenNotMatched, WriteMode, WriteParams};
use lance::Dataset;

use crate::types::UserRecord;

fn users_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("email", DataType::Utf8, false),
        Field::new("role", DataType::Utf8, false),
        Field::new("created_at", DataType::Utf8, false),
        Field::new("last_seen_at", DataType::Utf8, false),
    ]))
}

fn records_to_batch(records: &[UserRecord]) -> Result<RecordBatch> {
    let schema = users_schema();

    let id: StringArray = records.iter().map(|r| Some(r.id.as_str())).collect();
    let email: StringArray = records.iter().map(|r| Some(r.email.as_str())).collect();
    let role: StringArray = records.iter().map(|r| Some(r.role.as_str())).collect();
    let created_at: StringArray = records
        .iter()
        .map(|r| Some(r.created_at.to_rfc3339()))
        .collect();
    let last_seen_at: StringArray = records
        .iter()
        .map(|r| Some(r.last_seen_at.to_rfc3339()))
        .collect();

    Ok(RecordBatch::try_new(
        schema,
        vec![
            Arc::new(id),
            Arc::new(email),
            Arc::new(role),
            Arc::new(created_at),
            Arc::new(last_seen_at),
        ],
    )?)
}

fn batch_to_records(batch: &RecordBatch) -> Result<Vec<UserRecord>> {
    let str_col = |name: &str| -> Result<&StringArray> {
        batch
            .column_by_name(name)
            .with_context(|| format!("missing column {name}"))?
            .as_any()
            .downcast_ref::<StringArray>()
            .with_context(|| format!("column {name} is not Utf8"))
    };
    let id = str_col("id")?;
    let email = str_col("email")?;
    let role = str_col("role")?;
    let created_at = str_col("created_at")?;
    let last_seen_at = str_col("last_seen_at")?;

    let parse_ts = |s: &str, field: &str| -> Result<DateTime<Utc>> {
        Ok(chrono::DateTime::parse_from_rfc3339(s)
            .with_context(|| format!("parse {field}"))?
            .with_timezone(&Utc))
    };

    let mut records = Vec::with_capacity(batch.num_rows());
    for i in 0..batch.num_rows() {
        records.push(UserRecord {
            id: id.value(i).to_string(),
            email: email.value(i).to_string(),
            role: role.value(i).to_string(),
            created_at: parse_ts(created_at.value(i), "created_at")?,
            last_seen_at: parse_ts(last_seen_at.value(i), "last_seen_at")?,
        });
    }
    Ok(records)
}

/// Read all user records at `path`.
///
/// Returns `Ok(None)` only when the dataset does not exist yet (no user has ever been recorded).
/// Every other failure is propagated as `Err`.
pub async fn read_users(path: &str) -> Result<Option<Vec<UserRecord>>> {
    let dataset = match Dataset::open(path).await {
        Ok(dataset) => dataset,
        Err(lance::Error::DatasetNotFound { .. }) => return Ok(None),
        Err(e) => return Err(e).context("open users dataset"),
    };
    let batch = dataset
        .scan()
        .try_into_batch()
        .await
        .context("scan users dataset")?;
    Ok(Some(batch_to_records(&batch)?))
}

/// Record that `email` was seen at `now`, upserting the users table at `path`.
///
/// The email is lowercased (it is the merge key). On first sight a new row is inserted with a
/// fresh UUIDv7 `id`, `created_at = now`, and `role = "viewer"`; on a re-sight the existing row's
/// `id`/`created_at`/`role` are preserved and only `last_seen_at` advances — so re-sights never
/// duplicate a user. When the dataset does not exist yet the first sighting creates it with a
/// single-row `WriteMode::Create` write; thereafter the write is a `merge_insert` keyed on
/// `email`, whose builder retries manifest/semantic commit conflicts internally so a concurrent
/// sighting is retried rather than lost.
pub async fn record_user_seen(path: &str, email: &str, now: DateTime<Utc>) -> Result<()> {
    let email = email.to_ascii_lowercase();
    let schema = users_schema();

    let dataset = match Dataset::open(path).await {
        Ok(dataset) => dataset,
        Err(lance::Error::DatasetNotFound { .. }) => {
            // First sighting ever: create the dataset with this one user.
            let record = UserRecord {
                id: uuid::Uuid::now_v7().to_string(),
                email,
                role: "viewer".to_string(),
                created_at: now,
                last_seen_at: now,
            };
            let batch = records_to_batch(&[record])?;
            let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
            let params = WriteParams {
                mode: WriteMode::Create,
                ..Default::default()
            };
            Dataset::write(reader, path, Some(params))
                .await
                .context("create users dataset")?;
            return Ok(());
        }
        Err(e) => return Err(e).context("open users dataset for upsert"),
    };

    // The table is tiny; read it all and find any existing row for this email so the upsert can
    // preserve its id/created_at/role. Absent -> a brand-new viewer row.
    let existing = dataset
        .scan()
        .try_into_batch()
        .await
        .context("scan users dataset for upsert")?;
    let existing = batch_to_records(&existing)?;
    let prior = existing.iter().find(|u| u.email == email);
    let source = UserRecord {
        id: prior
            .map(|u| u.id.clone())
            .unwrap_or_else(|| uuid::Uuid::now_v7().to_string()),
        role: prior
            .map(|u| u.role.clone())
            .unwrap_or_else(|| "viewer".to_string()),
        created_at: prior.map(|u| u.created_at).unwrap_or(now),
        email,
        last_seen_at: now,
    };

    let batch = records_to_batch(&[source])?;
    let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
    let mut builder = MergeInsertBuilder::try_new(Arc::new(dataset), vec!["email".to_string()])
        .context("build users merge_insert")?;
    let job = builder
        .when_matched(WhenMatched::UpdateAll)
        .when_not_matched(WhenNotMatched::InsertAll)
        .try_build()
        .context("configure users merge_insert")?;
    job.execute_reader(reader)
        .await
        .context("execute users merge_insert")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn record_user_seen_inserts_then_updates_without_duplicating() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("users").to_str().unwrap().to_string();
        let t1 = chrono::Utc::now();
        record_user_seen(&path, "A@x.co", t1).await.unwrap(); // create path (dataset absent)
        let t2 = t1 + chrono::Duration::seconds(5);
        record_user_seen(&path, "a@x.co", t2).await.unwrap(); // update path, case-insensitive
        let users = read_users(&path).await.unwrap().unwrap();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].email, "a@x.co");
        assert_eq!(users[0].role, "viewer");
        // id/created_at are stable across the re-sight; only last_seen_at advanced.
        let first = &users[0];
        assert_eq!(first.created_at, t1);
        assert_eq!(first.last_seen_at, t2);
        let first_id = first.id.clone();

        record_user_seen(&path, "b@y.co", t2).await.unwrap(); // second user inserts
        let users = read_users(&path).await.unwrap().unwrap();
        assert_eq!(users.len(), 2);
        assert_eq!(
            users.iter().find(|u| u.email == "a@x.co").unwrap().id,
            first_id
        );
    }
}

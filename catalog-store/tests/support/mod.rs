//! Shared fixtures for database-backed tests; also included by `catalog-api/tests` via `#[path]`.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::{Int32Array, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::{DataType, Field, Schema};
use catalog_store::PostgresNamespace;
use lance::Dataset;
use lance_namespace::{Error, NamespaceError};

/// A namespace backed by a schema named after the test (dropped and recreated each run).
/// Returns `None`, after printing a skip notice, when `CATALOG_TEST_DATABASE_URL` is unset,
/// unless `CATALOG_REQUIRE_DB_TESTS=1` makes that a failure.
pub async fn namespace(test: &str) -> Option<Arc<PostgresNamespace>> {
    let Ok(url) = std::env::var("CATALOG_TEST_DATABASE_URL") else {
        assert!(
            std::env::var("CATALOG_REQUIRE_DB_TESTS").as_deref() != Ok("1"),
            "CATALOG_TEST_DATABASE_URL must be set when CATALOG_REQUIRE_DB_TESTS=1"
        );
        eprintln!("skipping {test}: CATALOG_TEST_DATABASE_URL is not set");
        return None;
    };
    let schema = format!("test_{test}");
    let admin = sqlx::PgPool::connect(&url).await.expect("connect admin");
    sqlx::query(&format!("DROP SCHEMA IF EXISTS {schema} CASCADE"))
        .execute(&admin)
        .await
        .expect("drop test schema");
    admin.close().await;
    let pool = catalog_store::connect(&url, Some(&schema))
        .await
        .expect("connect test schema");
    Some(Arc::new(PostgresNamespace::new(pool, HashMap::new())))
}

/// Writes a three-row Lance dataset (`id: int32`, `name: utf8`) and returns its URI.
pub async fn write_dataset(dir: &std::path::Path, name: &str) -> String {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int32Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec!["a", "b", "c"])),
        ],
    )
    .unwrap();
    let uri = dir.join(name).to_str().unwrap().to_string();
    let reader = RecordBatchIterator::new([Ok(batch)], schema);
    Dataset::write(reader, &uri, None)
        .await
        .expect("write dataset");
    uri
}

/// Appends one row (`4`, `"d"`) to a dataset written by [`write_dataset`].
pub async fn append_row(uri: &str) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int32Array::from(vec![4])),
            Arc::new(StringArray::from(vec!["d"])),
        ],
    )
    .unwrap();
    let params = lance::dataset::WriteParams {
        mode: lance::dataset::WriteMode::Append,
        ..Default::default()
    };
    Dataset::write(
        RecordBatchIterator::new([Ok(batch)], schema),
        uri,
        Some(params),
    )
    .await
    .expect("append row");
}

/// Adds a BTree index named `id_idx` on `id`, committing the dataset's next version.
pub async fn add_scalar_index(uri: &str) {
    use lance::index::DatasetIndexExt;
    let mut dataset = Dataset::open(uri).await.expect("open dataset");
    dataset
        .create_index(
            &["id"],
            lance_index::IndexType::BTree,
            Some("id_idx".into()),
            &lance_index::scalar::ScalarIndexParams::default(),
            false,
        )
        .await
        .expect("create index");
}

/// The spec error code carried by a namespace error.
pub fn code(err: Error) -> u32 {
    match &err {
        Error::Namespace { source, .. } => source
            .downcast_ref::<NamespaceError>()
            .map(|e| e.code().as_u32())
            .unwrap_or_else(|| panic!("not a NamespaceError: {err}")),
        _ => panic!("not a namespace error: {err}"),
    }
}

pub fn id(parts: &[&str]) -> Option<Vec<String>> {
    Some(parts.iter().map(|s| s.to_string()).collect())
}

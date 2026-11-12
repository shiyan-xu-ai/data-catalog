//! Current-version semantics and the per-version cache, against real Postgres and local datasets.

mod support;

use std::collections::HashMap;
use std::time::Duration;

use arrow_array::{Int32Array, RecordBatch, RecordBatchIterator, StringArray};
use arrow_schema::{DataType, Field, Schema};
use catalog_store::{PostgresNamespace, SqlLimits};
use lance::dataset::{WriteMode, WriteParams};
use lance::Dataset;
use lance_namespace::models::{
    CreateNamespaceRequest, DescribeTableRequest, DescribeTableVersionRequest,
    GetTableStatsRequest, ListTableIndicesRequest, ListTableVersionsRequest, RegisterTableRequest,
};
use lance_namespace::{ErrorCode, LanceNamespace};
use std::sync::Arc;
use support::{code, id};

const TABLE_NOT_FOUND: u32 = ErrorCode::TableNotFound as u32;
const TABLE_VERSION_NOT_FOUND: u32 = ErrorCode::TableVersionNotFound as u32;

fn register(parts: &[&str], location: &str, overwrite: bool) -> RegisterTableRequest {
    RegisterTableRequest {
        id: id(parts),
        location: location.to_string(),
        mode: overwrite.then(|| "overwrite".to_string()),
        ..Default::default()
    }
}

fn detailed(parts: &[&str]) -> DescribeTableRequest {
    DescribeTableRequest {
        id: id(parts),
        load_detailed_metadata: Some(true),
        ..Default::default()
    }
}

async fn current(ns: &PostgresNamespace, parts: &[&str]) -> i64 {
    ns.describe_table(detailed(parts))
        .await
        .unwrap()
        .version
        .unwrap()
}

fn limits() -> SqlLimits {
    SqlLimits {
        max_rows: 100,
        timeout: Duration::from_secs(30),
        memory_limit_bytes: 64 << 20,
    }
}

async fn sql(ns: &PostgresNamespace, parts: &[&str], query: &str) -> (u64, serde_json::Value) {
    let result = ns
        .query_sql(&id(parts).unwrap(), query, None, &limits())
        .await
        .unwrap();
    (result.version, result.rows[0][0].clone())
}

/// Replaces the dataset's contents with the single row `(10, "z")`, committing a new version.
async fn overwrite(uri: &str) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int32Array::from(vec![10])),
            Arc::new(StringArray::from(vec!["z"])),
        ],
    )
    .unwrap();
    let params = WriteParams {
        mode: WriteMode::Overwrite,
        ..Default::default()
    };
    Dataset::write(
        RecordBatchIterator::new([Ok(batch)], schema),
        uri,
        Some(params),
    )
    .await
    .unwrap();
}

/// Deletes `version`'s manifest and its data files, as cleanup does once a newer version no
/// longer references them.
async fn delete_version(uri: &str, version: u64) {
    let dataset = Dataset::open(uri)
        .await
        .unwrap()
        .checkout_version(version)
        .await
        .unwrap();
    for fragment in dataset.get_fragments() {
        for file in &fragment.metadata().files {
            std::fs::remove_file(format!("{uri}/data/{}", file.path)).unwrap();
        }
    }
    std::fs::remove_file(format!("/{}", dataset.manifest_location().path)).unwrap();
}

#[tokio::test]
async fn current_version_lifecycle() {
    let Some(pool) = support::pool("store_current").await else {
        return;
    };
    let ns = PostgresNamespace::new(pool.clone(), HashMap::new());
    let dir = tempfile::tempdir().unwrap();
    let uri = support::write_dataset(dir.path(), "t.lance").await;
    let table = ["prod", "t"];
    ns.create_namespace(CreateNamespaceRequest {
        id: id(&["prod"]),
        ..Default::default()
    })
    .await
    .unwrap();
    ns.register_table(register(&table, &uri, false))
        .await
        .unwrap();
    assert_eq!(current(&ns, &table).await, 1);

    // A commit after registration is listed and readable by number, but unversioned reads
    // keep answering for the current version.
    support::append_row(&uri).await;
    assert_eq!(current(&ns, &table).await, 1);
    let versions = ns
        .list_table_versions(ListTableVersionsRequest {
            id: id(&table),
            ..Default::default()
        })
        .await
        .unwrap();
    let listed: Vec<i64> = versions.versions.iter().map(|v| v.version).collect();
    assert_eq!(listed, [1, 2]);
    let v2 = ns
        .describe_table_version(DescribeTableVersionRequest {
            id: id(&table),
            version: Some(2),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(v2.version.version, 2);
    assert_eq!(
        sql(&ns, &table, "SELECT count(*) FROM t").await,
        (1, 3.into())
    );
    let pinned = ns
        .query_sql(
            &id(&table).unwrap(),
            "SELECT count(*) FROM t",
            Some(2),
            &limits(),
        )
        .await
        .unwrap();
    assert_eq!(pinned.rows[0][0], 4);

    // Stats and column storage share one data-statistics computation.
    let stats = ns
        .get_table_stats(GetTableStatsRequest {
            id: id(&table),
            ..Default::default()
        })
        .await
        .unwrap();
    let columns = ns.column_storage(&id(&table).unwrap()).await.unwrap();
    assert_eq!(columns.version, 1);
    assert_eq!(
        stats.total_bytes as u64,
        columns.columns.iter().map(|c| c.bytes_on_disk).sum::<u64>()
    );
    assert_eq!(stats.num_rows, 3);

    // Refresh advances every instance; a second refresh with no commits changes nothing.
    let other = PostgresNamespace::new(pool.clone(), HashMap::new());
    assert_eq!(current(&other, &table).await, 1);
    let refresh = ns.refresh_table(&id(&table).unwrap()).await.unwrap();
    assert_eq!(
        (refresh.previous_version, refresh.current_version),
        (Some(1), 2)
    );
    assert_eq!(current(&ns, &table).await, 2);
    assert_eq!(current(&other, &table).await, 2);
    assert_eq!(
        sql(&other, &table, "SELECT count(*) FROM t").await,
        (2, 4.into())
    );
    let again = ns.refresh_table(&id(&table).unwrap()).await.unwrap();
    assert_eq!(
        (again.previous_version, again.current_version),
        (Some(2), 2)
    );
    let err = ns
        .refresh_table(&id(&["prod", "missing"]).unwrap())
        .await
        .unwrap_err();
    assert_eq!(code(err), TABLE_NOT_FOUND);

    // A version that never existed is not found and leaves the current version alone.
    let err = ns
        .describe_table(DescribeTableRequest {
            version: Some(9),
            ..detailed(&table)
        })
        .await
        .unwrap_err();
    assert_eq!(code(err), TABLE_VERSION_NOT_FOUND);
    assert_eq!(current(&ns, &table).await, 2);

    // A row from before current versions existed resolves to the newest version on first read.
    sqlx::query("UPDATE tables SET current_version = NULL, current_e_tag = NULL")
        .execute(&pool)
        .await
        .unwrap();
    support::append_row(&uri).await;
    let (a, b) = (
        PostgresNamespace::new(pool.clone(), HashMap::new()),
        PostgresNamespace::new(pool.clone(), HashMap::new()),
    );
    let (first, second) = tokio::join!(current(&a, &table), current(&b, &table));
    assert_eq!((first, second), (3, 3));
    let recorded: Option<i64> = sqlx::query_scalar("SELECT current_version FROM tables")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(recorded, Some(3));
}

#[tokio::test]
async fn warm_reads_skip_object_storage() {
    let Some(pool) = support::pool("store_warm").await else {
        return;
    };
    let ns = PostgresNamespace::new(pool.clone(), HashMap::new());
    let dir = tempfile::tempdir().unwrap();
    let uri = support::write_dataset(dir.path(), "t.lance").await;
    support::add_scalar_index(&uri).await;
    let table = ["prod", "t"];
    ns.create_namespace(CreateNamespaceRequest {
        id: id(&["prod"]),
        ..Default::default()
    })
    .await
    .unwrap();
    ns.register_table(register(&table, &uri, false))
        .await
        .unwrap();
    let stats_request = || GetTableStatsRequest {
        id: id(&table),
        ..Default::default()
    };
    let indices_request = || ListTableIndicesRequest {
        id: id(&table),
        ..Default::default()
    };
    let table_id = id(&table).unwrap();
    let describe = ns.describe_table(detailed(&table)).await.unwrap();
    let stats = ns.get_table_stats(stats_request()).await.unwrap();
    let columns = ns.column_storage(&table_id).await.unwrap();
    let indices = ns.list_table_indices(indices_request()).await.unwrap();
    let storage = ns.storage_footprint(&table_id).await.unwrap();

    // With the whole dataset gone, a warm instance still answers every version-tied read from
    // its cache: none of them reaches object storage.
    std::fs::remove_dir_all(&uri).unwrap();
    assert_eq!(ns.describe_table(detailed(&table)).await.unwrap(), describe);
    assert_eq!(ns.get_table_stats(stats_request()).await.unwrap(), stats);
    assert_eq!(ns.column_storage(&table_id).await.unwrap(), columns);
    assert_eq!(
        ns.list_table_indices(indices_request()).await.unwrap(),
        indices
    );
    assert_eq!(ns.storage_footprint(&table_id).await.unwrap(), storage);

    // A budget too small to hold anything still answers, recomputing every time.
    let dir = tempfile::tempdir().unwrap();
    let uri = support::write_dataset(dir.path(), "u.lance").await;
    let tiny = PostgresNamespace::new(pool, HashMap::new()).with_cache_budget(1);
    tiny.register_table(register(&table, &uri, true))
        .await
        .unwrap();
    for _ in 0..2 {
        assert_eq!(current(&tiny, &table).await, 1);
        assert!(tiny.column_storage(&table_id).await.unwrap().columns[0].bytes_on_disk > 0);
    }
}

#[tokio::test]
async fn current_version_advances_when_cleaned_up() {
    let Some(pool) = support::pool("store_cleanup").await else {
        return;
    };
    let ns = PostgresNamespace::new(pool.clone(), HashMap::new());
    let dir = tempfile::tempdir().unwrap();
    let uri = support::write_dataset(dir.path(), "t.lance").await;
    let table = ["prod", "t"];
    ns.create_namespace(CreateNamespaceRequest {
        id: id(&["prod"]),
        ..Default::default()
    })
    .await
    .unwrap();
    ns.register_table(register(&table, &uri, false))
        .await
        .unwrap();
    assert_eq!(
        sql(&ns, &table, "SELECT sum(id) FROM t").await,
        (1, 6.into())
    );

    // Cleanup removes v1 after an overwrite. The warm instance learns of it from the failed
    // data read and answers from v2 within the same request.
    overwrite(&uri).await;
    delete_version(&uri, 1).await;
    assert_eq!(
        sql(&ns, &table, "SELECT sum(id) FROM t").await,
        (2, 10.into())
    );

    // A cold instance meets the missing manifest on open. A read pinned to the current version
    // is not found and moves nothing; an unversioned read advances.
    overwrite(&uri).await;
    delete_version(&uri, 2).await;
    let cold = PostgresNamespace::new(pool.clone(), HashMap::new());
    let err = cold
        .describe_table(DescribeTableRequest {
            version: Some(2),
            ..detailed(&table)
        })
        .await
        .unwrap_err();
    assert_eq!(code(err), TABLE_VERSION_NOT_FOUND);
    assert_eq!(current(&cold, &table).await, 3);
    assert_eq!(current(&ns, &table).await, 3);

    // A dataset recreated at the same location restarts at version 1 with a new e-tag; an
    // instance without it cached advances to the new incarnation instead of marking it current as v3.
    std::fs::remove_dir_all(&uri).unwrap();
    let recreated = support::write_dataset(dir.path(), "t.lance").await;
    support::append_row(&recreated).await;
    support::append_row(&recreated).await;
    let stats = PostgresNamespace::new(pool, HashMap::new())
        .get_table_stats(GetTableStatsRequest {
            id: id(&table),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(stats.num_rows, 5);
}

#[tokio::test]
async fn reregistering_never_reuses_cached_values() {
    let Some(pool) = support::pool("store_reregister").await else {
        return;
    };
    let ns = PostgresNamespace::new(pool, HashMap::new());
    let dir = tempfile::tempdir().unwrap();
    let small = support::write_dataset(dir.path(), "small.lance").await;
    let large = support::write_dataset(dir.path(), "large.lance").await;
    for _ in 0..3 {
        support::append_row(&large).await;
    }
    let table = ["prod", "t"];
    ns.create_namespace(CreateNamespaceRequest {
        id: id(&["prod"]),
        ..Default::default()
    })
    .await
    .unwrap();
    ns.register_table(register(&table, &small, false))
        .await
        .unwrap();
    let table_id = id(&table).unwrap();
    let before = ns.column_storage(&table_id).await.unwrap();
    assert_eq!(before.version, 1);

    ns.register_table(register(&table, &large, true))
        .await
        .unwrap();
    let after = ns.column_storage(&table_id).await.unwrap();
    assert_eq!(after.version, 4);
    let rows = ns
        .get_table_stats(GetTableStatsRequest {
            id: id(&table),
            ..Default::default()
        })
        .await
        .unwrap()
        .num_rows;
    assert_eq!(rows, 6);
}

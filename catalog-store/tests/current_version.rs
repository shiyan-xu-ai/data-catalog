mod support;

use std::collections::HashMap;
use std::time::Duration;

use catalog_store::{PostgresNamespace, SqlLimits};
use lance::dataset::{WriteMode, WriteParams};
use lance::Dataset;
use lance_namespace::models::{
    DescribeTableRequest, DescribeTableVersionRequest, GetTableStatsRequest,
    ListTableIndicesRequest, ListTableVersionsRequest, RegisterTableRequest,
};
use lance_namespace::LanceNamespace;
use sqlx::PgPool;
use support::{code, create, id, register, TABLE_VERSION_NOT_FOUND};

const TABLE: [&str; 2] = ["prod", "t"];

async fn register_in_prod(ns: &PostgresNamespace, uri: &str) {
    ns.create_namespace(create(&["prod"])).await.unwrap();
    ns.register_table(register(&TABLE, uri)).await.unwrap();
}

fn overwrite_registration(location: &str) -> RegisterTableRequest {
    RegisterTableRequest {
        mode: Some("overwrite".into()),
        ..register(&TABLE, location)
    }
}

fn detailed(parts: &[&str]) -> DescribeTableRequest {
    DescribeTableRequest {
        id: id(parts),
        load_detailed_metadata: Some(true),
        ..Default::default()
    }
}

fn stats(parts: &[&str]) -> GetTableStatsRequest {
    GetTableStatsRequest {
        id: id(parts),
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

async fn overwrite(uri: &str) {
    let params = WriteParams {
        mode: WriteMode::Overwrite,
        ..Default::default()
    };
    support::write_rows(uri, &[10], &["z"], Some(params)).await;
}

async fn simulate_cleanup(uri: &str, version: u64) {
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

async fn forget_current_versions(pool: &PgPool) {
    sqlx::query("UPDATE tables SET current_version = NULL, current_e_tag = NULL")
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn current_version_lifecycle() {
    let Some(pool) = support::pool("store_current").await else {
        return;
    };
    let ns = PostgresNamespace::new(pool.clone(), HashMap::new());
    let dir = tempfile::tempdir().unwrap();
    let uri = support::write_dataset(dir.path(), "t.lance").await;
    register_in_prod(&ns, &uri).await;
    assert_eq!(current(&ns, &TABLE).await, 1);

    support::append_row(&uri).await;
    assert_eq!(current(&ns, &TABLE).await, 1);
    let versions = ns
        .list_table_versions(ListTableVersionsRequest {
            id: id(&TABLE),
            ..Default::default()
        })
        .await
        .unwrap();
    let listed: Vec<i64> = versions.versions.iter().map(|v| v.version).collect();
    assert_eq!(listed, [1, 2]);
    let v2 = ns
        .describe_table_version(DescribeTableVersionRequest {
            id: id(&TABLE),
            version: Some(2),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(v2.version.version, 2);
    let table_stats = ns.get_table_stats(stats(&TABLE)).await.unwrap();
    let columns = ns.column_storage(&id(&TABLE).unwrap()).await.unwrap();
    assert_eq!(columns.version, 1);
    assert_eq!(table_stats.num_rows, 3);

    let other = PostgresNamespace::new(pool.clone(), HashMap::new());
    assert_eq!(current(&other, &TABLE).await, 1);
    let refresh = ns.refresh_table(&id(&TABLE).unwrap()).await.unwrap();
    assert_eq!(
        (refresh.previous_version, refresh.current_version),
        (Some(1), 2)
    );
    assert_eq!(current(&ns, &TABLE).await, 2);
    assert_eq!(current(&other, &TABLE).await, 2);
    assert_eq!(
        sql(&other, &TABLE, "SELECT count(*) FROM t").await,
        (2, 4.into())
    );
    let again = ns.refresh_table(&id(&TABLE).unwrap()).await.unwrap();
    assert_eq!(
        (again.previous_version, again.current_version),
        (Some(2), 2)
    );

    let err = ns
        .describe_table(DescribeTableRequest {
            version: Some(9),
            ..detailed(&TABLE)
        })
        .await
        .unwrap_err();
    assert_eq!(code(err), TABLE_VERSION_NOT_FOUND);
    assert_eq!(current(&ns, &TABLE).await, 2);

    forget_current_versions(&pool).await;
    support::append_row(&uri).await;
    let (a, b) = (
        PostgresNamespace::new(pool.clone(), HashMap::new()),
        PostgresNamespace::new(pool.clone(), HashMap::new()),
    );
    let (first, second) = tokio::join!(current(&a, &TABLE), current(&b, &TABLE));
    assert_eq!((first, second), (3, 3));
    let recorded: Option<i64> = sqlx::query_scalar("SELECT current_version FROM tables")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(recorded, Some(3));
}

#[tokio::test]
async fn concurrent_refreshes_write_once() {
    let Some(pool) = support::pool("store_refresh_race").await else {
        return;
    };
    let ns = PostgresNamespace::new(pool.clone(), HashMap::new());
    let dir = tempfile::tempdir().unwrap();
    let uri = support::write_dataset(dir.path(), "t.lance").await;
    register_in_prod(&ns, &uri).await;
    support::append_row(&uri).await;
    support::append_row(&uri).await;
    // Every write to a table row lands in `row_writes`.
    for statement in [
        "CREATE TABLE row_writes (version bigint)",
        "CREATE FUNCTION log_row_write() RETURNS trigger LANGUAGE plpgsql AS
         $$ BEGIN INSERT INTO row_writes VALUES (NEW.current_version); RETURN NEW; END $$",
        "CREATE TRIGGER log_row_write AFTER UPDATE ON tables
         FOR EACH ROW EXECUTE FUNCTION log_row_write()",
    ] {
        sqlx::query(statement).execute(&pool).await.unwrap();
    }

    // One namespace per request, as on separate instances.
    let instances: Vec<_> = (0..8)
        .map(|_| PostgresNamespace::new(pool.clone(), HashMap::new()))
        .collect();
    let table = id(&TABLE).unwrap();
    let refreshes =
        futures::future::join_all(instances.iter().map(|ns| ns.refresh_table(&table))).await;
    for refresh in refreshes {
        assert_eq!(refresh.unwrap().current_version, 3);
    }
    let writes: Vec<Option<i64>> = sqlx::query_scalar("SELECT version FROM row_writes")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(writes, [Some(3)]);
    assert_eq!(current(&ns, &TABLE).await, 3);
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
    register_in_prod(&ns, &uri).await;
    let indices_request = || ListTableIndicesRequest {
        id: id(&TABLE),
        ..Default::default()
    };
    let table_id = id(&TABLE).unwrap();
    let describe = ns.describe_table(detailed(&TABLE)).await.unwrap();
    let table_stats = ns.get_table_stats(stats(&TABLE)).await.unwrap();
    let columns = ns.column_storage(&table_id).await.unwrap();
    let indices = ns.list_table_indices(indices_request()).await.unwrap();
    let storage = ns.storage_footprint(&table_id).await.unwrap();

    std::fs::remove_dir_all(&uri).unwrap();
    assert_eq!(ns.describe_table(detailed(&TABLE)).await.unwrap(), describe);
    assert_eq!(
        ns.get_table_stats(stats(&TABLE)).await.unwrap(),
        table_stats
    );
    assert_eq!(ns.column_storage(&table_id).await.unwrap(), columns);
    assert_eq!(
        ns.list_table_indices(indices_request()).await.unwrap(),
        indices
    );
    assert_eq!(ns.storage_footprint(&table_id).await.unwrap(), storage);

    let dir = tempfile::tempdir().unwrap();
    let uri = support::write_dataset(dir.path(), "u.lance").await;
    let tiny = PostgresNamespace::new(pool, HashMap::new()).with_cache_budget(1);
    tiny.register_table(overwrite_registration(&uri))
        .await
        .unwrap();
    for _ in 0..2 {
        assert_eq!(current(&tiny, &TABLE).await, 1);
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
    register_in_prod(&ns, &uri).await;
    assert_eq!(
        sql(&ns, &TABLE, "SELECT sum(id) FROM t").await,
        (1, 6.into())
    );

    overwrite(&uri).await;
    simulate_cleanup(&uri, 1).await;
    assert_eq!(
        sql(&ns, &TABLE, "SELECT sum(id) FROM t").await,
        (2, 10.into())
    );

    overwrite(&uri).await;
    simulate_cleanup(&uri, 2).await;
    let cold = PostgresNamespace::new(pool.clone(), HashMap::new());
    let err = cold
        .describe_table(DescribeTableRequest {
            version: Some(2),
            ..detailed(&TABLE)
        })
        .await
        .unwrap_err();
    assert_eq!(code(err), TABLE_VERSION_NOT_FOUND);
    assert_eq!(current(&cold, &TABLE).await, 3);
    assert_eq!(current(&ns, &TABLE).await, 3);

    // The new incarnation's v3 holds 5 rows; only the e-tag check moves past it to v4.
    std::fs::remove_dir_all(&uri).unwrap();
    let recreated = support::write_dataset(dir.path(), "t.lance").await;
    for _ in 0..3 {
        support::append_row(&recreated).await;
    }
    let table_stats = PostgresNamespace::new(pool, HashMap::new())
        .get_table_stats(stats(&TABLE))
        .await
        .unwrap();
    assert_eq!(table_stats.num_rows, 6);
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
    register_in_prod(&ns, &small).await;
    let table_id = id(&TABLE).unwrap();
    let before = ns.column_storage(&table_id).await.unwrap();
    assert_eq!(before.version, 1);

    ns.register_table(overwrite_registration(&large))
        .await
        .unwrap();
    let after = ns.column_storage(&table_id).await.unwrap();
    assert_eq!(after.version, 4);
    let rows = ns.get_table_stats(stats(&TABLE)).await.unwrap().num_rows;
    assert_eq!(rows, 6);
}

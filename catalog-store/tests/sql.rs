//! The multi-table SQL engine against real Postgres and local Lance datasets.

mod support;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use catalog_store::{PostgresNamespace, SqlAnswer, SqlLimits, SqlResult, SqlTable, Truncation};
use lance::dataset::{WriteMode, WriteParams};
use lance_namespace::{Error, ErrorCode, LanceNamespace};
use serde_json::{json, Value};
use support::{code, create, register, TABLE_NOT_FOUND, TABLE_VERSION_NOT_FOUND};

const INVALID_INPUT: u32 = ErrorCode::InvalidInput as u32;

struct Fixture {
    ns: PostgresNamespace,
    dir: tempfile::TempDir,
}

fn limits() -> SqlLimits {
    SqlLimits {
        max_rows: 100,
        max_result_bytes: 1 << 20,
        timeout: Duration::from_secs(30),
        memory_limit_bytes: 64 << 20,
    }
}

impl Fixture {
    async fn new(test: &str) -> Option<Self> {
        let pool = support::pool(test).await?;
        Some(Self {
            ns: PostgresNamespace::new(pool, HashMap::new()),
            dir: tempfile::tempdir().unwrap(),
        })
    }

    /// Registers a table at `path` (creating its namespaces) holding the given rows.
    async fn table(&self, path: &[&str], ids: &[i32], names: &[&str]) -> String {
        for end in 1..path.len() {
            // Parents shared with an earlier table already exist.
            let _ = self.ns.create_namespace(create(&path[..end])).await;
        }
        let uri = self
            .dir
            .path()
            .join(format!("{}.lance", path.join("_")))
            .to_str()
            .unwrap()
            .to_string();
        support::write_rows(&uri, ids, names, None).await;
        self.ns.register_table(register(path, &uri)).await.unwrap();
        uri
    }

    /// Replaces the table's rows with a new version and makes it the current one.
    async fn overwrite(&self, path: &[&str], uri: &str, ids: &[i32], names: &[&str]) {
        let params = WriteParams {
            mode: WriteMode::Overwrite,
            ..Default::default()
        };
        support::write_rows(uri, ids, names, Some(params)).await;
        let id = path.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        self.ns.refresh_table(&id).await.unwrap();
    }

    async fn query(&self, sql: &str) -> Result<SqlAnswer, Error> {
        self.ns.query_sql(sql, &limits()).await
    }

    /// Whether the result cache answered the query.
    async fn cached(&self, sql: &str) -> bool {
        self.query(sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .cached
    }

    async fn ok(&self, sql: &str) -> SqlResult {
        let answer = self
            .query(sql)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
        Arc::unwrap_or_clone(answer.result)
    }

    async fn rows(&self, sql: &str) -> Vec<Vec<Value>> {
        self.ok(sql).await.rows
    }

    /// The error's code and message.
    async fn failure(&self, sql: &str) -> (u32, String) {
        let err = self
            .query(sql)
            .await
            .expect_err(&format!("{sql} should fail"));
        let message = err.to_string();
        (code(err), message)
    }
}

fn read(path: &[&str], version: u64) -> SqlTable {
    SqlTable {
        id: path.iter().map(|s| s.to_string()).collect(),
        version,
    }
}

const EVENTS: [&str; 3] = ["a", "b", "events"];
const USERS: [&str; 2] = ["x", "users"];

#[tokio::test]
async fn joins_tables_across_namespaces_at_pinned_versions() {
    let Some(f) = Fixture::new("sql_join").await else {
        return;
    };
    f.table(&EVENTS, &[1, 2, 3], &["a", "b", "c"]).await;
    let users = f.table(&USERS, &[1, 2, 3], &["old1", "old2", "old3"]).await;
    f.overwrite(&USERS, &users, &[1, 2, 3], &["new1", "new2", "new3"])
        .await;

    let pinned = f
        .ok(r#"SELECT e.id, u.name FROM "a"."b"."events" e JOIN "x"."users@v1" u ON e.id = u.id ORDER BY e.id"#)
        .await;
    assert_eq!(
        json!(pinned.rows),
        json!([[1, "old1"], [2, "old2"], [3, "old3"]])
    );
    assert_eq!(pinned.tables, [read(&EVENTS, 1), read(&USERS, 1)]);

    let current = f
        .ok(r#"SELECT e.id, u.name FROM "a"."b"."events" e JOIN "x"."users" u ON e.id = u.id ORDER BY e.id"#)
        .await;
    assert_eq!(current.rows[0], [json!(1), json!("new1")]);
    assert_eq!(current.tables, [read(&EVENTS, 1), read(&USERS, 2)]);
}

#[tokio::test]
async fn reads_a_four_segment_path() {
    let Some(f) = Fixture::new("sql_deep").await else {
        return;
    };
    f.table(&["a", "b", "c", "events"], &[1, 2, 3], &["a", "b", "c"])
        .await;
    let result = f.ok(r#"SELECT count(*) FROM "a"."b"."c"."events""#).await;
    assert_eq!(result.rows, [[3]]);
    assert_eq!(result.tables, [read(&["a", "b", "c", "events"], 1)]);
}

#[tokio::test]
async fn unquoted_segments_keep_their_case() {
    let Some(f) = Fixture::new("sql_case").await else {
        return;
    };
    f.table(&["Prod", "Events"], &[1, 2, 3], &["a", "b", "c"])
        .await;
    assert_eq!(f.rows("SELECT count(*) FROM Prod.Events").await, [[3]]);
    assert_eq!(
        f.rows("SELECT Events.id FROM Prod.Events ORDER BY 1").await,
        [[1], [2], [3]]
    );
    let (error_code, message) = f.failure("SELECT count(*) FROM prod.events").await;
    assert_eq!(error_code, TABLE_NOT_FOUND);
    assert!(message.contains(r#""prod"."events""#), "{message}");
}

#[tokio::test]
async fn resolves_leaf_aliases_and_path_qualified_references() {
    let Some(f) = Fixture::new("sql_alias").await else {
        return;
    };
    f.table(&EVENTS, &[1, 2, 3], &["a", "b", "c"]).await;

    for sql in [
        r#"SELECT events.id FROM "a"."b"."events" ORDER BY 1"#,
        r#"SELECT e.id FROM "a"."b"."events" AS e ORDER BY 1"#,
        r#"SELECT "a"."b"."events".id FROM "a"."b"."events" ORDER BY 1"#,
    ] {
        assert_eq!(f.rows(sql).await, [[1], [2], [3]], "{sql}");
    }
    // A user alias replaces the default one.
    let (error_code, _) = f
        .failure(r#"SELECT events.id FROM "a"."b"."events" e"#)
        .await;
    assert_eq!(error_code, INVALID_INPUT);

    let wildcard = f
        .ok(r#"SELECT "a"."b"."events".* FROM "a"."b"."events" ORDER BY 1"#)
        .await;
    let names: Vec<_> = wildcard.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "name"]);
    assert_eq!(wildcard.rows[2], [json!(3), json!("c")]);
}

#[tokio::test]
async fn leaf_alias_survives_reading_one_table_twice() {
    let Some(f) = Fixture::new("sql_repeat").await else {
        return;
    };
    f.table(&EVENTS, &[1, 2, 3], &["a", "b", "c"]).await;

    for sql in [
        r#"SELECT events.id FROM "a"."b"."events" WHERE id IN (SELECT id FROM "a"."b"."events" WHERE id > 1) ORDER BY 1"#,
        r#"SELECT events.id FROM "a"."b"."events" WHERE id > 2
           UNION ALL SELECT events.id FROM "a"."b"."events" WHERE id < 2 ORDER BY 1"#,
    ] {
        assert!(!f.ok(sql).await.rows.is_empty(), "{sql}");
    }
    // Two unaliased relations of one table still clash on their own alias.
    let (error_code, _) = f
        .failure(r#"SELECT count(*) FROM "a"."b"."events", "a"."b"."events""#)
        .await;
    assert_eq!(error_code, INVALID_INPUT);
}

#[tokio::test]
async fn same_leaf_in_two_namespaces_joins() {
    let Some(f) = Fixture::new("sql_same_leaf").await else {
        return;
    };
    f.table(&EVENTS, &[1, 2, 3], &["a", "b", "c"]).await;
    f.table(&["x", "events"], &[2, 3, 4], &["p", "q", "r"])
        .await;
    let result = f
        .ok(r#"SELECT "a"."b"."events".id AS l, "x"."events".id AS r,
                      "a"."b"."events".name AS ln, "x"."events".name AS rn
               FROM "a"."b"."events" JOIN "x"."events"
                 ON "a"."b"."events".id = "x"."events".id
               ORDER BY 1"#)
        .await;
    assert_eq!(
        result.rows,
        [
            vec![json!(2), json!(2), json!("b"), json!("p")],
            vec![json!(3), json!(3), json!("c"), json!("q")],
        ]
    );
    assert_eq!(result.tables, [read(&EVENTS, 1), read(&["x", "events"], 1)]);
}

#[tokio::test]
async fn compares_current_and_pinned_versions_of_one_table() {
    let Some(f) = Fixture::new("sql_compare").await else {
        return;
    };
    let users = f.table(&USERS, &[1, 2, 3], &["old1", "old2", "old3"]).await;
    f.overwrite(&USERS, &users, &[1, 2, 3], &["new1", "new2", "new3"])
        .await;
    let result = f
        .ok(r#"SELECT n.name AS new, o.name AS old
               FROM "x"."users" n JOIN "x"."users@v1" o ON n.id = o.id ORDER BY n.id"#)
        .await;
    assert_eq!(
        result.rows,
        [["new1", "old1"], ["new2", "old2"], ["new3", "old3"]]
    );
    assert_eq!(result.tables, [read(&USERS, 2), read(&USERS, 1)]);
}

#[tokio::test]
async fn exact_leaf_wins_over_a_version_pin() {
    let Some(f) = Fixture::new("sql_exact_leaf").await else {
        return;
    };
    f.table(&["a", "t"], &[1, 2, 3], &["a", "b", "c"]).await;
    f.table(&["a", "t@v1"], &[9], &["z"]).await;
    let result = f.ok(r#"SELECT id FROM "a"."t@v1""#).await;
    assert_eq!(result.rows, [[9]]);
    assert_eq!(result.tables, [read(&["a", "t@v1"], 1)]);
}

#[tokio::test]
async fn one_part_names_must_be_cte_aliases() {
    let Some(f) = Fixture::new("sql_one_part").await else {
        return;
    };
    f.table(&EVENTS, &[1, 2, 3], &["a", "b", "c"]).await;

    let result = f
        .ok("WITH events AS (SELECT 1 AS id) SELECT id FROM events")
        .await;
    assert_eq!(result.rows, [[1]]);
    assert!(result.tables.is_empty());
    let result = f
        .ok(r#"WITH e AS (SELECT id FROM "a"."b"."events") SELECT count(*) FROM e"#)
        .await;
    assert_eq!(result.rows, [[3]]);
    assert_eq!(result.tables, [read(&EVENTS, 1)]);

    let (error_code, message) = f.failure("SELECT * FROM events").await;
    assert_eq!(error_code, INVALID_INPUT);
    assert!(message.contains("full path"), "{message}");
}

#[tokio::test]
async fn table_functions_are_not_catalog_paths() {
    let Some(f) = Fixture::new("sql_function").await else {
        return;
    };
    let result = f.ok("SELECT value FROM range(3) ORDER BY 1").await;
    assert_eq!(result.rows, [[0], [1], [2]]);
    assert!(result.tables.is_empty());
}

#[tokio::test]
async fn explain_plans_over_a_full_path() {
    let Some(f) = Fixture::new("sql_explain").await else {
        return;
    };
    f.table(&EVENTS, &[1, 2, 3], &["a", "b", "c"]).await;
    let result = f
        .ok(r#"EXPLAIN SELECT id FROM "a"."b"."events" WHERE id > 1"#)
        .await;
    let names: Vec<_> = result.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["plan_type", "plan"]);
    assert!(!result.rows.is_empty());
    assert_eq!(result.tables, [read(&EVENTS, 1)]);
}

#[tokio::test]
async fn failures_name_the_path_as_written() {
    let Some(f) = Fixture::new("sql_errors").await else {
        return;
    };
    f.table(&EVENTS, &[1, 2, 3], &["a", "b", "c"]).await;
    let gone = f.table(&["a", "b", "gone"], &[1], &["a"]).await;
    std::fs::remove_dir_all(&gone).unwrap();

    for (sql, expected_code, path) in [
        (
            r#"SELECT * FROM "a"."b"."nope""#,
            TABLE_NOT_FOUND,
            r#""a"."b"."nope""#,
        ),
        (
            r#"SELECT * FROM "a"."b"."nope@v2""#,
            TABLE_NOT_FOUND,
            r#""a"."b"."nope@v2""#,
        ),
        (
            r#"SELECT * FROM "a"."b"."gone""#,
            TABLE_NOT_FOUND,
            r#""a"."b"."gone""#,
        ),
        (
            r#"SELECT * FROM "a"."b"."events@v9""#,
            TABLE_VERSION_NOT_FOUND,
            r#""a"."b"."events@v9""#,
        ),
        (
            r#"SELECT * FROM "a"."b"."events" e JOIN "a"."b"."events@v9" o ON e.id = o.id"#,
            TABLE_VERSION_NOT_FOUND,
            r#""a"."b"."events@v9""#,
        ),
    ] {
        let (actual, message) = f.failure(sql).await;
        assert_eq!(actual, expected_code, "{sql}: {message}");
        assert!(message.contains(path), "{sql}: {message}");
    }
}

#[tokio::test]
async fn statements_that_write_are_rejected() {
    let Some(f) = Fixture::new("sql_readonly").await else {
        return;
    };
    f.table(&EVENTS, &[1, 2, 3], &["a", "b", "c"]).await;
    let copied = f.dir.path().join("copied.csv");
    let copied_query = f.dir.path().join("copied-query.csv");

    for sql in [
        "CREATE TABLE t AS SELECT 1".to_string(),
        r#"INSERT INTO "a"."b"."events" VALUES (4, 'd')"#.to_string(),
        r#"DROP TABLE "a"."b"."events""#.to_string(),
        format!(r#"COPY "a"."b"."events" TO '{}'"#, copied.display()),
        format!(
            r#"COPY (SELECT * FROM "a"."b"."events") TO '{}'"#,
            copied_query.display()
        ),
    ] {
        let (actual, message) = f.failure(&sql).await;
        assert_eq!(actual, INVALID_INPUT, "{sql}: {message}");
    }
    assert!(!copied.exists() && !copied_query.exists());
    assert_eq!(
        f.rows(r#"SELECT count(*) FROM "a"."b"."events""#).await,
        [[3]]
    );
}

#[tokio::test]
async fn a_query_without_tables_reads_none() {
    let Some(f) = Fixture::new("sql_no_tables").await else {
        return;
    };
    let result = f.ok("SELECT 1").await;
    assert_eq!(result.rows, [[1]]);
    assert!(result.tables.is_empty());
}

#[tokio::test]
async fn results_stop_at_the_row_or_byte_cap() {
    let Some(f) = Fixture::new("sql_caps").await else {
        return;
    };
    // Each row encodes as `["x…x"]`: 100 bytes of text and 4 of punctuation.
    let wide = "SELECT repeat('x', 100) AS s FROM range(50)";
    let capped = |max_rows, max_result_bytes| SqlLimits {
        max_rows,
        max_result_bytes,
        ..limits()
    };
    let run = |limits: SqlLimits| {
        let ns = &f.ns;
        async move { ns.query_sql(wide, &limits).await.unwrap().result }
    };

    let by_bytes = run(capped(100, 1000)).await;
    assert_eq!(by_bytes.rows.len(), 9);
    assert!(by_bytes
        .rows
        .iter()
        .all(|row| *row == [json!("x".repeat(100))]));
    assert_eq!(by_bytes.truncated_by, Some(Truncation::Bytes));

    let by_rows = run(capped(10, 1 << 20)).await;
    assert_eq!(by_rows.rows.len(), 10);
    assert_eq!(by_rows.truncated_by, Some(Truncation::Rows));

    let whole = run(capped(50, 50 * 104)).await;
    assert_eq!(whole.rows.len(), 50);
    assert_eq!(whole.truncated_by, None);
}

#[tokio::test]
async fn repeated_queries_over_unchanged_versions_are_cached() {
    let Some(f) = Fixture::new("sql_cache").await else {
        return;
    };
    let uri = f.table(&EVENTS, &[1, 2, 3], &["a", "b", "c"]).await;
    let sql = r#"SELECT count(*) FROM "a"."b"."events""#;
    assert!(!f.cached(sql).await);
    let hit = f.query(&format!("  {sql}\n")).await.unwrap();
    assert!(hit.cached);
    assert_eq!(hit.result.rows, [[3]]);
    assert_eq!(hit.result.tables, [read(&EVENTS, 1)]);
    // Another text over the same version runs.
    assert!(
        !f.cached(r#"SELECT count(*) AS n FROM "a"."b"."events""#)
            .await
    );

    f.overwrite(&EVENTS, &uri, &[1, 2], &["a", "b"]).await;
    let moved = f.query(sql).await.unwrap();
    assert!(!moved.cached);
    assert_eq!(moved.result.rows, [[2]]);
    assert_eq!(moved.result.tables, [read(&EVENTS, 2)]);
    assert!(f.cached(sql).await);
    // A pinned version stays cached beside the current one.
    let pinned = r#"SELECT count(*) FROM "a"."b"."events@v1""#;
    assert!(!f.cached(pinned).await);
    assert!(f.cached(pinned).await);
}

#[tokio::test]
async fn results_depending_on_more_than_table_versions_are_never_cached() {
    let Some(f) = Fixture::new("sql_uncached").await else {
        return;
    };
    f.table(&EVENTS, &[1, 2, 3], &["a", "b", "c"]).await;
    for sql in [
        "SELECT now()",
        "SELECT 1",
        r#"SELECT id FROM "a"."b"."events" WHERE now() > TIMESTAMP '2000-01-01'"#,
        r#"SELECT id, random() FROM "a"."b"."events""#,
        r#"SELECT id FROM "a"."b"."events" WHERE id IN (SELECT id FROM "a"."b"."events" WHERE now() IS NOT NULL)"#,
        r#"EXPLAIN SELECT id FROM "a"."b"."events""#,
    ] {
        for _ in 0..2 {
            assert!(!f.cached(sql).await, "{sql}");
        }
    }
}

#[tokio::test]
async fn failed_queries_are_not_cached() {
    let Some(f) = Fixture::new("sql_failed_uncached").await else {
        return;
    };
    f.table(&EVENTS, &[1, 2, 3], &["a", "b", "c"]).await;
    let sql = r#"SELECT count(*) FROM "a"."b"."events""#;
    let expired = SqlLimits {
        timeout: Duration::ZERO,
        ..limits()
    };
    let err = f.ns.query_sql(sql, &expired).await.unwrap_err();
    assert!(err.to_string().contains("time limit"), "{err}");
    assert!(!f.cached(sql).await);
    assert!(f.cached(sql).await);
}

#[tokio::test]
async fn the_result_cache_evicts_beyond_its_budget() {
    let Some(mut f) = Fixture::new("sql_cache_evict").await else {
        return;
    };
    // Each result weighs about 660 bytes, so one fits and two do not.
    f.ns = f.ns.with_sql_cache_budget(1000);
    f.table(&EVENTS, &[1, 2, 3], &["a", "b", "c"]).await;
    let first = r#"SELECT repeat('a', 600) FROM "a"."b"."events" LIMIT 1"#;
    let second = r#"SELECT repeat('b', 600) FROM "a"."b"."events" LIMIT 1"#;
    assert!(!f.cached(first).await);
    assert!(f.cached(first).await);
    assert!(!f.cached(second).await);
    assert!(f.cached(second).await);
    assert!(!f.cached(first).await);
}

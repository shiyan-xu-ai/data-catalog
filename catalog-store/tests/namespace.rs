//! `PostgresNamespace` against real Postgres and a local Lance dataset.

mod support;

use std::collections::HashMap;

use lance::Dataset;
use lance_namespace::models::{
    CreateNamespaceRequest, DeregisterTableRequest, DescribeNamespaceRequest,
    DescribeTableIndexStatsRequest, DescribeTableRequest, DescribeTableVersionRequest,
    DropNamespaceRequest, GetTableStatsRequest, ListNamespacesRequest, ListTableBranchesRequest,
    ListTableIndicesRequest, ListTableTagsRequest, ListTableVersionsRequest, ListTablesRequest,
    NamespaceExistsRequest, RegisterTableRequest, TableExistsRequest,
};
use lance_namespace::{ErrorCode, LanceNamespace};
use support::{code, id};

const NAMESPACE_NOT_FOUND: u32 = ErrorCode::NamespaceNotFound as u32;
const NAMESPACE_EXISTS: u32 = ErrorCode::NamespaceAlreadyExists as u32;
const NAMESPACE_NOT_EMPTY: u32 = ErrorCode::NamespaceNotEmpty as u32;
const TABLE_NOT_FOUND: u32 = ErrorCode::TableNotFound as u32;
const TABLE_EXISTS: u32 = ErrorCode::TableAlreadyExists as u32;
const TABLE_VERSION_NOT_FOUND: u32 = ErrorCode::TableVersionNotFound as u32;
const TABLE_INDEX_NOT_FOUND: u32 = ErrorCode::TableIndexNotFound as u32;
const INVALID_INPUT: u32 = ErrorCode::InvalidInput as u32;
const UNSUPPORTED: u32 = ErrorCode::Unsupported as u32;

fn create(parts: &[&str]) -> CreateNamespaceRequest {
    CreateNamespaceRequest {
        id: id(parts),
        ..Default::default()
    }
}

fn props(pairs: &[(&str, &str)]) -> Option<HashMap<String, String>> {
    Some(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
}

fn list_ns(
    parts: &[&str],
    page_token: Option<String>,
    limit: Option<i32>,
) -> ListNamespacesRequest {
    ListNamespacesRequest {
        id: id(parts),
        page_token,
        limit,
        ..Default::default()
    }
}

fn register(parts: &[&str], location: &str) -> RegisterTableRequest {
    RegisterTableRequest {
        id: id(parts),
        location: location.to_string(),
        ..Default::default()
    }
}

#[tokio::test]
async fn namespaces_lifecycle() {
    let Some(ns) = support::namespace("store_namespaces").await else {
        return;
    };

    // Root always exists and cannot be created or dropped.
    ns.namespace_exists(NamespaceExistsRequest::default())
        .await
        .unwrap();
    assert_eq!(
        code(ns.create_namespace(create(&[])).await.unwrap_err()),
        NAMESPACE_EXISTS
    );
    let drop_root = DropNamespaceRequest::default();
    assert_eq!(
        code(ns.drop_namespace(drop_root).await.unwrap_err()),
        INVALID_INPUT
    );

    // Parent must exist; names are validated.
    let err = ns
        .create_namespace(create(&["prod", "analytics"]))
        .await
        .unwrap_err();
    assert_eq!(code(err), NAMESPACE_NOT_FOUND);
    let err = ns
        .create_namespace(create(&["bad$name"]))
        .await
        .unwrap_err();
    assert_eq!(code(err), INVALID_INPUT);

    let created = ns
        .create_namespace(CreateNamespaceRequest {
            properties: props(&[("owner", "data")]),
            ..create(&["prod"])
        })
        .await
        .unwrap();
    assert_eq!(created.properties, props(&[("owner", "data")]));
    for name in ["c", "a", "b"] {
        ns.create_namespace(create(&["prod", name])).await.unwrap();
    }
    ns.create_namespace(create(&["prod", "a", "deep"]))
        .await
        .unwrap();
    ns.create_namespace(create(&["dev"])).await.unwrap();

    // Uniqueness and declared modes.
    let err = ns
        .create_namespace(create(&["prod", "a"]))
        .await
        .unwrap_err();
    assert_eq!(code(err), NAMESPACE_EXISTS);
    let kept = ns
        .create_namespace(CreateNamespaceRequest {
            mode: Some("ExistOk".into()),
            properties: props(&[("ignored", "x")]),
            ..create(&["prod"])
        })
        .await
        .unwrap();
    assert_eq!(kept.properties, props(&[("owner", "data")]));
    let err = ns
        .create_namespace(CreateNamespaceRequest {
            mode: Some("overwrite".into()),
            ..create(&["prod", "a"])
        })
        .await
        .unwrap_err();
    assert_eq!(
        code(err),
        NAMESPACE_NOT_EMPTY,
        "overwrite must not orphan children"
    );
    let replaced = ns
        .create_namespace(CreateNamespaceRequest {
            mode: Some("overwrite".into()),
            properties: props(&[("v", "2")]),
            ..create(&["prod", "b"])
        })
        .await
        .unwrap();
    assert_eq!(replaced.properties, props(&[("v", "2")]));
    let err = ns
        .create_namespace(CreateNamespaceRequest {
            mode: Some("bogus".into()),
            ..create(&["x"])
        })
        .await
        .unwrap_err();
    assert_eq!(code(err), INVALID_INPUT);

    // Listing returns direct children only, sorted, with start-after pagination.
    let root = ns.list_namespaces(list_ns(&[], None, None)).await.unwrap();
    assert_eq!(root.namespaces, ["dev", "prod"]);
    assert_eq!(root.page_token, None);
    let page1 = ns
        .list_namespaces(list_ns(&["prod"], None, Some(2)))
        .await
        .unwrap();
    assert_eq!(page1.namespaces, ["a", "b"]);
    assert_eq!(page1.page_token.as_deref(), Some("b"));
    let page2 = ns
        .list_namespaces(list_ns(&["prod"], page1.page_token, Some(2)))
        .await
        .unwrap();
    assert_eq!(page2.namespaces, ["c"]);
    assert_eq!(page2.page_token, None);
    let err = ns
        .list_namespaces(list_ns(&["nope"], None, None))
        .await
        .unwrap_err();
    assert_eq!(code(err), NAMESPACE_NOT_FOUND);

    let described = ns
        .describe_namespace(DescribeNamespaceRequest {
            id: id(&["prod"]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(described.properties, props(&[("owner", "data")]));
    let err = ns
        .namespace_exists(NamespaceExistsRequest {
            id: id(&["prod", "zzz"]),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(code(err), NAMESPACE_NOT_FOUND);

    // Drops are restricted to empty namespaces; Skip tolerates a missing one; no cascade.
    let drop = |parts: &[&str]| DropNamespaceRequest {
        id: id(parts),
        ..Default::default()
    };
    assert_eq!(
        code(ns.drop_namespace(drop(&["prod", "a"])).await.unwrap_err()),
        NAMESPACE_NOT_EMPTY
    );
    let cascade = DropNamespaceRequest {
        behavior: Some("Cascade".into()),
        ..drop(&["prod", "a"])
    };
    assert_eq!(
        code(ns.drop_namespace(cascade).await.unwrap_err()),
        UNSUPPORTED
    );
    ns.drop_namespace(drop(&["prod", "a", "deep"]))
        .await
        .unwrap();
    let dropped = ns.drop_namespace(drop(&["prod", "a"])).await.unwrap();
    assert_eq!(dropped.properties, props(&[]));
    assert_eq!(
        code(ns.drop_namespace(drop(&["prod", "a"])).await.unwrap_err()),
        NAMESPACE_NOT_FOUND
    );
    let skip = DropNamespaceRequest {
        mode: Some("skip".into()),
        ..drop(&["prod", "a"])
    };
    ns.drop_namespace(skip).await.unwrap();
}

#[tokio::test]
async fn tables_lifecycle() {
    let Some(ns) = support::namespace("store_tables").await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let users = support::write_dataset(dir.path(), "users.lance").await;
    let events = support::write_dataset(dir.path(), "events.lance").await;
    ns.create_namespace(create(&["prod"])).await.unwrap();

    // AE1: a missing parent namespace fails and creates nothing.
    let err = ns
        .register_table(register(&["prod", "analytics", "users"], &users))
        .await
        .unwrap_err();
    assert_eq!(code(err), NAMESPACE_NOT_FOUND);
    assert!(ns
        .list_namespaces(list_ns(&["prod"], None, None))
        .await
        .unwrap()
        .namespaces
        .is_empty());

    // AE3: an unreadable URI fails and creates nothing.
    let missing = dir.path().join("missing.lance");
    let err = ns
        .register_table(register(&["prod", "ghost"], missing.to_str().unwrap()))
        .await
        .unwrap_err();
    assert_eq!(code(err), INVALID_INPUT);
    let err = ns
        .table_exists(TableExistsRequest {
            id: id(&["prod", "ghost"]),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(code(err), TABLE_NOT_FOUND);

    // Tables need a non-root namespace.
    let err = ns
        .register_table(register(&["users"], &users))
        .await
        .unwrap_err();
    assert_eq!(code(err), INVALID_INPUT);

    // AE2 + R11: register with an owner property, then describe cheaply and in detail.
    let registered = ns
        .register_table(RegisterTableRequest {
            properties: props(&[("owner", "alice")]),
            ..register(&["prod", "users"], &users)
        })
        .await
        .unwrap();
    assert_eq!(registered.location.as_deref(), Some(users.as_str()));
    assert_eq!(registered.properties, props(&[("owner", "alice")]));
    let err = ns
        .register_table(register(&["prod", "users"], &events))
        .await
        .unwrap_err();
    assert_eq!(code(err), TABLE_EXISTS);

    let describe = |detailed: bool| DescribeTableRequest {
        id: id(&["prod", "users"]),
        load_detailed_metadata: Some(detailed),
        ..Default::default()
    };
    let cheap = ns.describe_table(describe(false)).await.unwrap();
    assert_eq!(cheap.table.as_deref(), Some("users"));
    assert_eq!(cheap.namespace, id(&["prod"]));
    assert_eq!(cheap.location.as_deref(), Some(users.as_str()));
    assert_eq!(cheap.properties.unwrap()["owner"], "alice");
    assert!(cheap.version.is_none() && cheap.schema.is_none() && cheap.stats.is_none());
    let detailed = ns.describe_table(describe(true)).await.unwrap();
    assert_eq!(detailed.version, Some(1));
    let fields: Vec<_> = detailed
        .schema
        .unwrap()
        .fields
        .into_iter()
        .map(|f| f.name)
        .collect();
    assert_eq!(fields, ["id", "name"]);
    let stats = detailed.stats.unwrap();
    assert_eq!((stats.num_fragments, stats.num_deleted_rows), (1, 0));
    assert!(detailed.storage_options.is_none());

    // Overwrite replaces the registration.
    for name in ["b_events", "a_events"] {
        ns.register_table(register(&["prod", name], &events))
            .await
            .unwrap();
    }
    ns.register_table(RegisterTableRequest {
        mode: Some("Overwrite".into()),
        ..register(&["prod", "a_events"], &users)
    })
    .await
    .unwrap();
    let a = ns
        .describe_table(DescribeTableRequest {
            id: id(&["prod", "a_events"]),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(a.location.as_deref(), Some(users.as_str()));

    // Listings come from Postgres, sorted, paginated.
    let list = |page_token, limit| ListTablesRequest {
        id: id(&["prod"]),
        page_token,
        limit,
        ..Default::default()
    };
    let page1 = ns.list_tables(list(None, Some(2))).await.unwrap();
    assert_eq!(page1.tables, ["a_events", "b_events"]);
    let page2 = ns
        .list_tables(list(page1.page_token, Some(2)))
        .await
        .unwrap();
    assert_eq!(page2.tables, ["users"]);
    assert_eq!(page2.page_token, None);
    let all = ns
        .list_all_tables(ListTablesRequest::default())
        .await
        .unwrap();
    assert_eq!(all.tables, ["prod$a_events", "prod$b_events", "prod$users"]);
    let all_page = ns
        .list_all_tables(ListTablesRequest {
            page_token: Some("prod$a_events".into()),
            limit: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(all_page.tables, ["prod$b_events"]);
    assert_eq!(all_page.page_token.as_deref(), Some("prod$b_events"));

    // AE4: a namespace holding tables cannot be dropped; everything remains.
    let err = ns
        .drop_namespace(DropNamespaceRequest {
            id: id(&["prod"]),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(code(err), NAMESPACE_NOT_EMPTY);
    ns.table_exists(TableExistsRequest {
        id: id(&["prod", "users"]),
        ..Default::default()
    })
    .await
    .unwrap();

    // AE5: deregistering removes only catalog metadata.
    let deregister = || DeregisterTableRequest {
        id: id(&["prod", "users"]),
        ..Default::default()
    };
    let removed = ns.deregister_table(deregister()).await.unwrap();
    assert_eq!(removed.location.as_deref(), Some(users.as_str()));
    assert_eq!(removed.properties, props(&[("owner", "alice")]));
    assert_eq!(
        code(ns.deregister_table(deregister()).await.unwrap_err()),
        TABLE_NOT_FOUND
    );
    assert_eq!(
        ns.list_tables(list(None, None)).await.unwrap().tables,
        ["a_events", "b_events"]
    );
    assert_eq!(
        Dataset::open(&users)
            .await
            .unwrap()
            .count_rows(None)
            .await
            .unwrap(),
        3
    );
}

#[tokio::test]
async fn table_details() {
    let Some(ns) = support::namespace("store_details").await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    // Version 1 writes three rows; version 2 adds the `id_idx` scalar index.
    let metrics = support::write_dataset(dir.path(), "metrics.lance").await;
    support::add_scalar_index(&metrics).await;
    ns.create_namespace(create(&["prod"])).await.unwrap();
    let table = || id(&["prod", "metrics"]);

    // AE3: catalog-managed keys are rejected; times are recorded and survive overwrite.
    for key in ["catalog.registered_at", "catalog.anything"] {
        let err = ns
            .register_table(RegisterTableRequest {
                properties: props(&[(key, "x")]),
                ..register(&["prod", "metrics"], &metrics)
            })
            .await
            .unwrap_err();
        assert_eq!(code(err), INVALID_INPUT);
    }
    let registered = ns
        .register_table(RegisterTableRequest {
            properties: props(&[("owner", "alice"), ("description", "d1"), ("tier", "gold")]),
            ..register(&["prod", "metrics"], &metrics)
        })
        .await
        .unwrap();
    assert_eq!(
        registered.properties,
        props(&[("owner", "alice"), ("description", "d1"), ("tier", "gold")])
    );
    let describe = |detailed: bool| DescribeTableRequest {
        id: table(),
        load_detailed_metadata: Some(detailed),
        ..Default::default()
    };
    let first = ns.describe_table(describe(false)).await.unwrap();
    let first = first.properties.unwrap();
    assert_eq!(first["owner"], "alice");
    assert_eq!(first["description"], "d1");
    assert_eq!(first["catalog.registered_at"], first["catalog.updated_at"]);
    assert!(first["catalog.registered_at"].ends_with('Z'));
    ns.register_table(RegisterTableRequest {
        mode: Some("overwrite".into()),
        properties: props(&[("owner", "bob")]),
        ..register(&["prod", "metrics"], &metrics)
    })
    .await
    .unwrap();
    let second = ns.describe_table(describe(false)).await.unwrap();
    let second = second.properties.unwrap();
    assert_eq!(second.len(), 3, "{second:?}");
    assert_eq!(second["owner"], "bob");
    assert_eq!(
        second["catalog.registered_at"],
        first["catalog.registered_at"]
    );
    assert!(second["catalog.updated_at"] > first["catalog.updated_at"]);

    // Detailed describe carries the manifest summary and storage format.
    let detailed = ns.describe_table(describe(true)).await.unwrap();
    assert_eq!(detailed.version, Some(2));
    let metadata = detailed.metadata.unwrap();
    assert_eq!(metadata["total_rows"], "3");
    assert_eq!(metadata["total_fragments"], "1");
    assert!(metadata["total_files_size"].parse::<u64>().unwrap() > 0);
    assert!(metadata.contains_key("lance.data_storage_version"));
    assert!(!metadata.keys().any(|k| k.starts_with("config.")));

    // Versions paginate by version number in either order.
    let list_versions = |page_token, limit, descending| ListTableVersionsRequest {
        id: table(),
        page_token,
        limit,
        descending,
        ..Default::default()
    };
    let page1 = ns
        .list_table_versions(list_versions(None, Some(1), None))
        .await
        .unwrap();
    assert_eq!(page1.versions.len(), 1);
    assert_eq!(page1.versions[0].version, 1);
    assert_eq!(page1.page_token.as_deref(), Some("1"));
    let v1 = &page1.versions[0];
    assert_eq!(v1.metadata.as_ref().unwrap()["total_rows"], "3");
    assert!(v1.timestamp_millis.is_some() && v1.manifest_size.is_some());
    let page2 = ns
        .list_table_versions(list_versions(page1.page_token, Some(1), None))
        .await
        .unwrap();
    assert_eq!(page2.versions[0].version, 2);
    assert_eq!(page2.page_token, None);
    let desc = ns
        .list_table_versions(list_versions(None, None, Some(true)))
        .await
        .unwrap();
    let numbers: Vec<_> = desc.versions.iter().map(|v| v.version).collect();
    assert_eq!(numbers, [2, 1]);
    let describe_version = |version| DescribeTableVersionRequest {
        id: table(),
        version,
        ..Default::default()
    };
    let v1 = ns
        .describe_table_version(describe_version(Some(1)))
        .await
        .unwrap();
    assert_eq!(*v1.version, page1.versions[0]);
    let latest = ns
        .describe_table_version(describe_version(None))
        .await
        .unwrap();
    assert_eq!(latest.version.version, 2);
    let err = ns
        .describe_table_version(describe_version(Some(9)))
        .await
        .unwrap_err();
    assert_eq!(code(err), TABLE_VERSION_NOT_FOUND);

    // Indices, index stats, and table stats.
    let indices = ns
        .list_table_indices(ListTableIndicesRequest {
            id: table(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(indices.indexes.len(), 1);
    let index = &indices.indexes[0];
    assert_eq!(index.index_name, "id_idx");
    assert_eq!(index.columns, ["id"]);
    assert_eq!(
        (index.num_indexed_rows, index.num_unindexed_rows),
        (Some(3), Some(0))
    );
    assert!(index.index_type.is_some() && index.size_bytes.is_some_and(|s| s > 0));
    let index_stats = |name: &str| DescribeTableIndexStatsRequest {
        id: table(),
        index_name: Some(name.into()),
        ..Default::default()
    };
    let stats = ns
        .describe_table_index_stats(index_stats("id_idx"))
        .await
        .unwrap();
    assert_eq!(stats.num_indexed_rows, Some(3));
    let err = ns
        .describe_table_index_stats(index_stats("nope"))
        .await
        .unwrap_err();
    assert_eq!(code(err), TABLE_INDEX_NOT_FOUND);
    let table_stats = ns
        .get_table_stats(GetTableStatsRequest {
            id: table(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!((table_stats.num_rows, table_stats.num_indices), (3, 1));
    assert!(table_stats.total_bytes > 0);
    let fragments = &table_stats.fragment_stats;
    assert_eq!(
        (fragments.num_fragments, fragments.num_small_fragments),
        (1, 1)
    );
    assert_eq!((fragments.lengths.min, fragments.lengths.max), (3, 3));

    // Column storage rolls fields up to top-level columns.
    let storage = ns.column_storage(&table().unwrap()).await.unwrap();
    assert_eq!(storage.version, 2);
    let names: Vec<_> = storage.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "name"]);
    assert!(storage.columns.iter().all(|c| c.bytes_on_disk > 0));
    assert_eq!(
        storage.columns.iter().map(|c| c.bytes_on_disk).sum::<u64>() as i64,
        table_stats.total_bytes
    );

    // No tags or branches; reading another branch is Unsupported.
    let tags = ns
        .list_table_tags(ListTableTagsRequest {
            id: table(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(tags.tags.is_empty());
    let branches = ns
        .list_table_branches(ListTableBranchesRequest {
            id: table(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(branches.branches.is_empty());
    let err = ns
        .list_table_versions(ListTableVersionsRequest {
            branch: Some("dev".into()),
            ..list_versions(None, None, None)
        })
        .await
        .unwrap_err();
    assert_eq!(code(err), UNSUPPORTED);

    // Detailed describe surfaces both manifest maps: `config` and `table_metadata`.
    let tagged = support::write_dataset(dir.path(), "tagged.lance").await;
    let mut dataset = lance::Dataset::open(&tagged).await.unwrap();
    dataset
        .update_config([("lance.auto_cleanup.interval", "20")])
        .await
        .unwrap();
    dataset
        .update_metadata([("producer", "export")])
        .await
        .unwrap();
    ns.register_table(register(&["prod", "tagged"], &tagged))
        .await
        .unwrap();
    let tagged_meta = ns
        .describe_table(DescribeTableRequest {
            id: id(&["prod", "tagged"]),
            load_detailed_metadata: Some(true),
            ..Default::default()
        })
        .await
        .unwrap()
        .metadata
        .unwrap();
    assert_eq!(tagged_meta["config.lance.auto_cleanup.interval"], "20");
    assert_eq!(tagged_meta["table_metadata.producer"], "export");

    // Timeline: one event per version, newest first, from manifests and transactions.
    let history = support::write_dataset(dir.path(), "history.lance").await;
    support::append_row(&history).await;
    support::add_scalar_index(&history).await;
    let mut dataset = lance::Dataset::open(&history).await.unwrap();
    dataset.delete("id = 2").await.unwrap();
    dataset
        .update_metadata([("producer", "export")])
        .await
        .unwrap();
    dataset.tags().create("release", 3).await.unwrap();
    ns.register_table(register(&["prod", "history"], &history))
        .await
        .unwrap();
    let history_id = id(&["prod", "history"]).unwrap();
    let page1 = ns.timeline(&history_id, None, Some(3)).await.unwrap();
    let page2 = ns
        .timeline(&history_id, page1.page_token.as_deref(), Some(3))
        .await
        .unwrap();
    assert_eq!(page1.page_token.as_deref(), Some("3"));
    assert_eq!(page2.page_token, None);
    let events: Vec<_> = page1.events.iter().chain(&page2.events).collect();
    let field = |key: &str| -> Vec<_> { events.iter().map(|e| e[key].clone()).collect() };
    assert_eq!(
        field("version"),
        [5, 4, 3, 2, 1].map(serde_json::Value::from)
    );
    assert_eq!(
        field("operation"),
        [
            "UpdateConfig",
            "Delete",
            "CreateIndex",
            "Append",
            "Overwrite"
        ]
        .map(serde_json::Value::from)
    );
    let [update_config, delete, create_index, append, overwrite] = events[..] else {
        unreachable!()
    };
    assert_eq!(
        update_config["details"]["table_metadata"]["set"]["producer"],
        "export"
    );
    assert_eq!(delete["details"]["predicate"], "id = 2");
    assert_eq!(delete["summary"]["total_deletion_file_rows"], "1");
    let added = &create_index["details"]["added"][0];
    assert_eq!(
        (&added["name"], &added["columns"][0]),
        (&"id_idx".into(), &"id".into())
    );
    assert_eq!(create_index["tags"], serde_json::json!(["release"]));
    assert_eq!(append["details"]["rows_added"], 1);
    assert_eq!(append["summary"]["total_rows"], "4");
    assert_eq!(overwrite["details"]["fields"], 2);
    assert_eq!(overwrite["writer"]["library"], "lance");
    assert!(overwrite["timestamp_millis"].is_i64() && overwrite["transaction"]["uuid"].is_string());
    let err = ns.timeline(&history_id, Some("x"), None).await.unwrap_err();
    assert_eq!(code(err), INVALID_INPUT);

    // AE4: with the dataset gone, catalog facts still describe; Lance reads are not found.
    std::fs::remove_dir_all(&metrics).unwrap();
    let cheap = ns.describe_table(describe(false)).await.unwrap();
    assert_eq!(cheap.properties.unwrap()["owner"], "bob");
    assert_eq!(
        code(ns.describe_table(describe(true)).await.unwrap_err()),
        TABLE_NOT_FOUND
    );
    let err = ns
        .list_table_versions(list_versions(None, None, None))
        .await
        .unwrap_err();
    assert_eq!(code(err), TABLE_NOT_FOUND);
    let err = ns.column_storage(&table().unwrap()).await.unwrap_err();
    assert_eq!(code(err), TABLE_NOT_FOUND);
    let err = ns
        .timeline(&table().unwrap(), None, None)
        .await
        .unwrap_err();
    assert_eq!(code(err), TABLE_NOT_FOUND);
}

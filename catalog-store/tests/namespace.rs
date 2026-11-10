//! `PostgresNamespace` against real Postgres and a local Lance dataset.

mod support;

use std::collections::HashMap;

use lance::Dataset;
use lance_namespace::models::{
    CreateNamespaceRequest, DeregisterTableRequest, DescribeNamespaceRequest, DescribeTableRequest,
    DropNamespaceRequest, ListNamespacesRequest, ListTablesRequest, NamespaceExistsRequest,
    RegisterTableRequest, TableExistsRequest,
};
use lance_namespace::{ErrorCode, LanceNamespace};
use support::{code, id};

const NAMESPACE_NOT_FOUND: u32 = ErrorCode::NamespaceNotFound as u32;
const NAMESPACE_EXISTS: u32 = ErrorCode::NamespaceAlreadyExists as u32;
const NAMESPACE_NOT_EMPTY: u32 = ErrorCode::NamespaceNotEmpty as u32;
const TABLE_NOT_FOUND: u32 = ErrorCode::TableNotFound as u32;
const TABLE_EXISTS: u32 = ErrorCode::TableAlreadyExists as u32;
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
    assert_eq!(cheap.properties, props(&[("owner", "alice")]));
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

//! The table timeline: one event per retained version, built from its manifest and transaction.

use std::collections::HashMap;

use futures::{StreamExt, TryStreamExt};
use lance::dataset::transaction::{Operation, Transaction, UpdateMap};
use lance::datatypes::Schema;
use lance::Dataset;
use lance_index::registry::display_type_from_url;
use lance_namespace::Result;
use serde_json::{json, Map, Value};

use crate::namespace::{internal, invalid_input, OpenTable, PostgresNamespace, READ_CONCURRENCY};

const DEFAULT_LIMIT: usize = 50;
const MAX_LIMIT: usize = 200;

/// A page of timeline events, newest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    /// Events as the `/timeline` route serves them.
    pub events: Vec<Value>,
    /// The last version returned, when older versions remain.
    pub page_token: Option<String>,
}

impl PostgresNamespace {
    /// Reads a page of retained versions, newest first, starting below `page_token`. Only
    /// manifests, transaction files, and tags are read; the dataset is never written.
    pub async fn timeline(
        &self,
        id: &[String],
        page_token: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Timeline> {
        let limit = limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let before = match page_token.filter(|t| !t.is_empty()) {
            Some(token) => Some(
                token
                    .parse::<u64>()
                    .map_err(|_| invalid_input(format!("invalid page token {token:?}")))?,
            ),
            None => None,
        };
        let OpenTable {
            location, dataset, ..
        } = self.open_table(id, None).await?;
        let read_error = |what: &str| {
            let context = format!("{what} at {location}");
            move |e: lance::Error| internal(format!("{context}: {e}"))
        };
        let mut versions: Vec<u64> = dataset
            .version_refs()
            .await
            .map_err(read_error("list versions"))?
            .into_iter()
            .map(|v| v.version)
            .filter(|&v| before.is_none_or(|b| v < b))
            .collect();
        versions.sort_unstable_by(|a, b| b.cmp(a));
        let page_token = (versions.len() > limit).then(|| versions[limit - 1].to_string());
        versions.truncate(limit);

        let mut tags: HashMap<u64, Vec<String>> = HashMap::new();
        for (name, contents) in dataset
            .tags()
            .list()
            .await
            .map_err(read_error("list tags"))?
        {
            if contents.branch.is_none() {
                tags.entry(contents.version).or_default().push(name);
            }
        }
        for names in tags.values_mut() {
            names.sort();
        }

        let dataset = &dataset;
        let tags = &tags;
        let events = futures::stream::iter(versions)
            .map(|version| async move {
                let checked_out;
                let at_version = if version == dataset.version_id() {
                    dataset
                } else {
                    checked_out = dataset.checkout_version(version).await?;
                    &checked_out
                };
                let transaction = at_version.read_transaction().await?;
                Ok::<_, lance::Error>(event(
                    at_version,
                    transaction,
                    tags.get(&version).cloned().unwrap_or_default(),
                ))
            })
            .buffered(READ_CONCURRENCY)
            .try_collect()
            .await
            .map_err(read_error("read versions"))?;
        Ok(Timeline { events, page_token })
    }
}

fn event(dataset: &Dataset, transaction: Option<Transaction>, tags: Vec<String>) -> Value {
    let version = dataset.version();
    let manifest = dataset.manifest();
    let writer = manifest
        .writer_version
        .as_ref()
        .map(|w| json!({"library": w.library, "version": w.version}));
    let (operation, details, transaction) = match transaction {
        Some(t) => (
            t.operation.name().to_string(),
            details(&t.operation, dataset.schema()),
            json!({
                "uuid": t.uuid,
                "read_version": t.read_version,
                "properties": t.transaction_properties.as_deref().cloned().unwrap_or_default(),
            }),
        ),
        None => ("Unknown".to_string(), json!({}), Value::Null),
    };
    json!({
        "version": version.version,
        "timestamp_millis": version.timestamp.timestamp_millis(),
        "operation": operation,
        "summary": version.metadata,
        "writer": writer,
        "transaction": transaction,
        "tags": tags,
        "details": details,
    })
}

/// Operation-specific details, with field ids resolved through the version's schema.
fn details(operation: &Operation, schema: &Schema) -> Value {
    let field_name = |id: i32| {
        schema
            .field_path(id)
            .unwrap_or_else(|_| format!("field {id}"))
    };
    match operation {
        Operation::Append { fragments } => json!({
            "fragments_added": fragments.len(),
            "rows_added": fragments.iter().filter_map(|f| f.physical_rows).sum::<usize>(),
            "files_added": fragments.iter().map(|f| f.files.len()).sum::<usize>(),
        }),
        Operation::Delete {
            updated_fragments,
            deleted_fragment_ids,
            predicate,
        } => json!({
            "predicate": predicate,
            "fragments_updated": updated_fragments.len(),
            "fragments_removed": deleted_fragment_ids.len(),
        }),
        Operation::Update {
            removed_fragment_ids,
            updated_fragments,
            new_fragments,
            fields_modified,
            ..
        } => json!({
            "fragments_updated": updated_fragments.len(),
            "fragments_removed": removed_fragment_ids.len(),
            "fragments_added": new_fragments.len(),
            "fields_modified": fields_modified
                .iter()
                .map(|&id| field_name(id as i32))
                .collect::<Vec<_>>(),
        }),
        Operation::Overwrite {
            fragments, schema, ..
        } => json!({
            "fragments": fragments.len(),
            "rows": fragments.iter().filter_map(|f| f.physical_rows).sum::<usize>(),
            "fields": schema.fields.len(),
        }),
        Operation::CreateIndex {
            new_indices,
            removed_indices,
        } => json!({
            "added": new_indices
                .iter()
                .map(|index| json!({
                    "name": index.name,
                    "columns": index.fields.iter().map(|&id| field_name(id)).collect::<Vec<_>>(),
                    "type": index
                        .index_details
                        .as_ref()
                        .map_or("Unknown", |d| display_type_from_url(&d.type_url)),
                }))
                .collect::<Vec<_>>(),
            "removed": removed_indices.iter().map(|i| &i.name).collect::<Vec<_>>(),
        }),
        Operation::Rewrite {
            groups,
            rewritten_indices,
            ..
        } => json!({
            "groups": groups.len(),
            "fragments_before": groups.iter().map(|g| g.old_fragments.len()).sum::<usize>(),
            "fragments_after": groups.iter().map(|g| g.new_fragments.len()).sum::<usize>(),
            "rewritten_indices": rewritten_indices.len(),
        }),
        Operation::Merge {
            fragments, schema, ..
        } => json!({
            "fragments": fragments.len(),
            "fields": schema.fields.len(),
        }),
        Operation::Project { schema, .. } => json!({
            "fields": schema.fields.iter().map(|f| &f.name).collect::<Vec<_>>(),
        }),
        Operation::UpdateConfig {
            config_updates,
            table_metadata_updates,
            schema_metadata_updates,
            field_metadata_updates,
        } => {
            let mut details = Map::new();
            for (key, updates) in [
                ("config", config_updates),
                ("table_metadata", table_metadata_updates),
            ] {
                if let Some(updates) = updates.as_ref().filter(|u| !u.update_entries.is_empty()) {
                    details.insert(key.into(), map_changes(updates, true));
                }
            }
            if let Some(updates) = schema_metadata_updates
                .as_ref()
                .filter(|u| !u.update_entries.is_empty())
            {
                details.insert("schema_metadata".into(), map_changes(updates, false));
            }
            if !field_metadata_updates.is_empty() {
                let mut ids: Vec<i32> = field_metadata_updates.keys().copied().collect();
                ids.sort_unstable();
                let fields: Vec<String> = ids.into_iter().map(field_name).collect();
                details.insert("field_metadata".into(), json!({ "fields": fields }));
            }
            Value::Object(details)
        }
        Operation::Restore { version } => json!({ "version": version }),
        Operation::DataReplacement { replacements } => {
            json!({ "replacements": replacements.len() })
        }
        _ => json!({}),
    }
}

/// `{set, removed}` of an update map; `set` maps keys to new values when `with_values`, and
/// lists keys otherwise.
fn map_changes(updates: &UpdateMap, with_values: bool) -> Value {
    let mut set = Map::new();
    let mut set_keys = Vec::new();
    let mut removed = Vec::new();
    for entry in &updates.update_entries {
        match &entry.value {
            Some(value) if with_values => {
                set.insert(entry.key.clone(), value.clone().into());
            }
            Some(_) => set_keys.push(entry.key.clone()),
            None => removed.push(entry.key.clone()),
        }
    }
    let set = if with_values {
        Value::Object(set)
    } else {
        set_keys.into()
    };
    json!({ "set": set, "removed": removed })
}

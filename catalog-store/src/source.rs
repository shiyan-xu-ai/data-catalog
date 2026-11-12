use std::fmt::Write;
use std::sync::LazyLock;

use lance::Dataset;
use lance_namespace::Result;
use prost_reflect::{DescriptorPool, DynamicMessage, Kind, MapKey, ReflectMessage, Value as Field};
use serde_json::{json, Map, Value};

use crate::namespace::{internal, lance_error, OpenTable, PostgresNamespace};
use crate::storage::relative_to;

static POOL: LazyLock<DescriptorPool> = LazyLock::new(|| {
    DescriptorPool::decode(include_bytes!(concat!(env!("OUT_DIR"), "/lance_table.bin")).as_ref())
        .expect("valid Lance descriptor set")
});

const MAGIC: &[u8] = b"LANC";
const FOOTER_LEN: usize = size_of::<u64>() + 2 * size_of::<u16>() + MAGIC.len();
const MAX_HEX_BYTES: usize = 64;
const ANY: &str = "google.protobuf.Any";

const MANIFEST_SECTIONS: [(&str, &str, &str); 3] = [
    ("index_section", "Index section", "lance.table.IndexSection"),
    (
        "transaction_section",
        "Transaction",
        "lance.table.Transaction",
    ),
    (
        "transaction_section_deprecated",
        "Transaction",
        "lance.table.Transaction",
    ),
];

impl PostgresNamespace {
    /// Decodes the files that record `version`. Each file lists its sections (one protobuf
    /// message each) with the byte offset of the section; a section that does not decode
    /// carries an `error` instead of `content`. Reads only those files; never writes.
    pub async fn version_files(&self, id: &[String], version: u64) -> Result<Vec<Value>> {
        let OpenTable {
            location, dataset, ..
        } = self.open_table(id, Some(version)).await?;
        let store = dataset
            .object_store(None)
            .await
            .map_err(lance_error(format!("open object store at {location}")))?;
        let manifest_path = &dataset.manifest_location().path;
        let transaction_path = dataset
            .manifest()
            .transaction_file
            .as_deref()
            .filter(|name| !name.is_empty())
            .map(|name| dataset.transactions_dir().join(name));
        let read_manifest = async {
            store
                .read_one_all(manifest_path)
                .await
                .map_err(lance_error(format!("read {manifest_path}")))
        };
        let read_transaction = async {
            match &transaction_path {
                Some(path) => store
                    .read_one_all(path)
                    .await
                    .map(Some)
                    .map_err(lance_error(format!("read {path}"))),
                None => Ok(None),
            }
        };
        let (bytes, transaction) = futures::try_join!(read_manifest, read_transaction)?;

        let (offset, manifest) = manifest_message(&bytes)
            .map_err(|e| internal(format!("decode manifest {manifest_path}: {e}")))?;
        let mut sections = vec![section("Manifest", offset, Ok(&manifest))];
        for (field, label, message) in MANIFEST_SECTIONS {
            // Unset fields read as their default, 0.
            if !manifest.has_field_by_name(field) {
                continue;
            }
            let Some(offset) = manifest.get_field_by_name(field).and_then(|v| v.as_u64()) else {
                continue;
            };
            let decoded = framed_message(&bytes, offset, message);
            sections.push(section(label, offset, decoded.as_ref()));
        }
        let mut files = vec![file(
            &dataset,
            manifest_path.as_ref(),
            bytes.len(),
            sections,
        )];

        if let (Some(path), Some(bytes)) = (&transaction_path, transaction) {
            let decoded = decode("lance.table.Transaction", &bytes);
            let sections = vec![section("Transaction", 0, decoded.as_ref())];
            files.push(file(&dataset, path.as_ref(), bytes.len(), sections));
        }
        Ok(files)
    }
}

fn file(dataset: &Dataset, path: &str, size: usize, sections: Vec<Value>) -> Value {
    let relative = relative_to(dataset.branch_location().path.as_ref(), path);
    json!({"path": relative, "size_bytes": size, "sections": sections})
}

fn section(
    name: &str,
    offset: u64,
    message: std::result::Result<&DynamicMessage, &String>,
) -> Value {
    let mut out = json!({"name": name, "offset": offset});
    match message {
        Ok(m) => {
            out["message"] = json!(m.descriptor().full_name());
            out["content"] = message_json(m);
        }
        Err(e) => out["error"] = json!(e),
    }
    out
}

fn manifest_message(bytes: &[u8]) -> std::result::Result<(u64, DynamicMessage), String> {
    let footer = bytes
        .len()
        .checked_sub(FOOTER_LEN)
        .filter(|_| bytes.ends_with(MAGIC))
        .ok_or("no Lance footer")?;
    let offset = u64::from_le_bytes(bytes[footer..footer + 8].try_into().expect("8 bytes"));
    Ok((
        offset,
        framed_message(bytes, offset, "lance.table.Manifest")?,
    ))
}

fn framed_message(
    bytes: &[u8],
    offset: u64,
    message: &str,
) -> std::result::Result<DynamicMessage, String> {
    let body = usize::try_from(offset)
        .ok()
        .and_then(|start| {
            let len = bytes.get(start..start.checked_add(4)?)?;
            let len = u32::from_le_bytes(len.try_into().ok()?) as usize;
            bytes.get(start + 4..(start + 4).checked_add(len)?)
        })
        .ok_or_else(|| format!("{message} at offset {offset} runs past the end of the file"))?;
    decode(message, body)
}

fn decode(message: &str, body: &[u8]) -> std::result::Result<DynamicMessage, String> {
    let descriptor = POOL
        .get_message_by_name(message)
        .ok_or_else(|| format!("unknown message {message}"))?;
    DynamicMessage::decode(descriptor, body).map_err(|e| format!("decode {message}: {e}"))
}

fn message_json(message: &DynamicMessage) -> Value {
    if message.descriptor().full_name() == ANY {
        if let Some(any) = any_json(message) {
            return any;
        }
    }
    let mut out = Map::new();
    for (field, value) in message.fields() {
        out.insert(field.name().to_string(), field_json(value, &field.kind()));
    }
    let unknown = message.unknown_fields().count();
    if unknown > 0 {
        out.insert("$unknown_fields".into(), json!(unknown));
    }
    Value::Object(out)
}

fn any_json(any: &DynamicMessage) -> Option<Value> {
    let type_url = any.get_field_by_name("type_url")?;
    let type_url = type_url.as_str()?;
    let descriptor = POOL.get_message_by_name(type_url.rsplit('/').next()?)?;
    let value = any.get_field_by_name("value")?;
    let decoded = DynamicMessage::decode(descriptor, value.as_bytes()?.clone()).ok()?;
    let mut out = Map::new();
    out.insert("@type".into(), json!(type_url));
    if let Value::Object(fields) = message_json(&decoded) {
        out.extend(fields);
    }
    Some(Value::Object(out))
}

fn field_json(value: &Field, kind: &Kind) -> Value {
    match value {
        Field::Bool(v) => json!(v),
        Field::I32(v) => json!(v),
        Field::U32(v) => json!(v),
        Field::I64(v) if v.unsigned_abs() <= JS_MAX_SAFE_INTEGER => json!(v),
        Field::I64(v) => json!(v.to_string()),
        Field::U64(v) if *v <= JS_MAX_SAFE_INTEGER => json!(v),
        Field::U64(v) => json!(v.to_string()),
        Field::F32(v) => json!(v),
        Field::F64(v) => json!(v),
        Field::String(v) => json!(v),
        Field::Bytes(v) if v.len() <= MAX_HEX_BYTES => {
            let mut hex = String::with_capacity(2 * v.len());
            for b in v.iter() {
                write!(hex, "{b:02x}").expect("writing to a String cannot fail");
            }
            Value::String(hex)
        }
        Field::Bytes(v) => json!(format!("<{} bytes>", v.len())),
        Field::EnumNumber(n) => kind
            .as_enum()
            .and_then(|e| e.get_value(*n))
            .map_or(json!(n), |v| json!(v.name())),
        Field::Message(m) => message_json(m),
        Field::List(items) => items.iter().map(|v| field_json(v, kind)).collect(),
        Field::Map(entries) => {
            let value_kind = kind
                .as_message()
                .map(|entry| entry.map_entry_value_field().kind());
            let mut out = Map::new();
            for (key, value) in entries {
                let key = match key {
                    MapKey::Bool(k) => k.to_string(),
                    MapKey::I32(k) => k.to_string(),
                    MapKey::I64(k) => k.to_string(),
                    MapKey::U32(k) => k.to_string(),
                    MapKey::U64(k) => k.to_string(),
                    MapKey::String(k) => k.clone(),
                };
                let value = match &value_kind {
                    Some(kind) => field_json(value, kind),
                    None => Value::Null,
                };
                out.insert(key, value);
            }
            Value::Object(out)
        }
    }
}

const JS_MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

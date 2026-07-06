//! Composite table identity: `<region>:<bucket>:<ns-seg>[:<ns-seg>...]:<name>`.
//!
//! `:` is legal raw in URL path segments (RFC 3986 pchar), safe in object keys,
//! and excluded from the per-segment charset, so splitting is unambiguous.

pub const ID_SEP: char = ':';

/// One segment of a composite id (region, bucket, namespace part, or table name).
/// Matches real table directory names; anything the object store would percent-encode
/// is refused so ids round-trip as filenames and URL path segments.
pub fn valid_id_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTableId {
    pub region: String,
    pub bucket: String,
    pub namespace: Vec<String>,
    pub name: String,
}

pub fn compose_table_id(region: &str, bucket: &str, namespace: &[String], name: &str) -> String {
    let mut parts: Vec<&str> = Vec::with_capacity(3 + namespace.len());
    parts.push(region);
    parts.push(bucket);
    parts.extend(namespace.iter().map(String::as_str));
    parts.push(name);
    parts.join(":")
}

pub fn parse_table_id(id: &str) -> Option<ParsedTableId> {
    let parts: Vec<&str> = id.split(ID_SEP).collect();
    if parts.len() < 4 || parts.iter().any(|p| !valid_id_segment(p)) {
        return None;
    }
    Some(ParsedTableId {
        region: parts[0].to_string(),
        bucket: parts[1].to_string(),
        namespace: parts[2..parts.len() - 1]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        name: parts[parts.len() - 1].to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_and_parse_round_trip_including_multi_segment_namespace() {
        let id = compose_table_id(
            "us-phoenix-1",
            "onroad-perception-datasets",
            &["a".to_string(), "b".to_string()],
            "smoke_test",
        );
        assert_eq!(id, "us-phoenix-1:onroad-perception-datasets:a:b:smoke_test");
        let p = parse_table_id(&id).unwrap();
        assert_eq!(p.region, "us-phoenix-1");
        assert_eq!(p.bucket, "onroad-perception-datasets");
        assert_eq!(p.namespace, vec!["a", "b"]);
        assert_eq!(p.name, "smoke_test");
    }

    #[test]
    fn parse_rejects_too_few_parts_bad_segments_and_empties() {
        assert!(parse_table_id("r:b:n").is_none()); // needs ≥4 parts
        assert!(parse_table_id("r:b::name").is_none()); // empty segment
        assert!(parse_table_id("r:b:ns:na/me").is_none()); // '/' not in charset
        assert!(parse_table_id("r:b:ns:..").is_none()); // dot-dot name
        assert!(parse_table_id("").is_none());
    }

    #[test]
    fn segment_charset_matches_the_declare_rules() {
        assert!(valid_id_segment("Table-1.x_y"));
        assert!(!valid_id_segment(""));
        assert!(!valid_id_segment("."));
        assert!(!valid_id_segment(".."));
        assert!(!valid_id_segment("a:b"));
        assert!(!valid_id_segment("a b"));
        assert!(!valid_id_segment(&"x".repeat(256)));
    }
}

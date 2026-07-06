//! Deployment-scoped catalog configuration, loaded once at boot from a committed
//! YAML file (`CATALOG_CONFIG_PATH`, default `catalog-config.yaml`). Declares the
//! region, the target buckets with their registered namespace prefixes, and the
//! admin email list. The service is regional: it runs colocated with these buckets.

use anyhow::{bail, Context, Result};
use catalog_core::valid_id_segment;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogConfig {
    pub region: String,
    pub buckets: Vec<BucketConfig>,
    #[serde(default)]
    pub admins: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BucketConfig {
    pub name: String,
    pub namespaces: Vec<String>, // '/'-joined prefix paths, e.g. "scenario_dataset_export" or "a/b"
}

impl CatalogConfig {
    pub fn load(path: &str) -> Result<Self> {
        let raw =
            std::fs::read_to_string(path).with_context(|| format!("read catalog config {path}"))?;
        let cfg: CatalogConfig =
            serde_yaml::from_str(&raw).with_context(|| format!("parse catalog config {path}"))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<()> {
        if !valid_id_segment(&self.region) {
            bail!("catalog config: invalid region {:?}", self.region);
        }
        if self.buckets.is_empty() {
            bail!("catalog config: no buckets declared");
        }
        let mut seen = std::collections::HashSet::new();
        for b in &self.buckets {
            if !valid_id_segment(&b.name) {
                bail!("catalog config: invalid bucket name {:?}", b.name);
            }
            if b.namespaces.is_empty() {
                bail!("catalog config: bucket {:?} declares no namespaces", b.name);
            }
            for ns in &b.namespaces {
                if ns.is_empty() || ns.split('/').any(|s| !valid_id_segment(s)) {
                    bail!(
                        "catalog config: invalid namespace {ns:?} in bucket {:?}",
                        b.name
                    );
                }
                if !seen.insert((b.name.as_str(), ns.as_str())) {
                    bail!(
                        "catalog config: duplicate namespace {ns:?} in bucket {:?}",
                        b.name
                    );
                }
            }
        }
        Ok(())
    }

    pub fn namespace_registered(&self, bucket: &str, ns: &[String]) -> bool {
        let joined = ns.join("/");
        self.buckets
            .iter()
            .any(|b| b.name == bucket && b.namespaces.contains(&joined))
    }

    pub fn is_admin(&self, email: &str) -> bool {
        self.admins.iter().any(|a| a.eq_ignore_ascii_case(email))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> anyhow::Result<CatalogConfig> {
        let cfg: CatalogConfig = serde_yaml::from_str(s)?;
        cfg.validate()?;
        Ok(cfg)
    }

    #[test]
    fn valid_config_parses_and_answers_lookups() {
        let cfg = parse(
            "region: us-phoenix-1\nbuckets:\n  - name: b1\n    namespaces: [ns1, a/b]\nadmins: [Admin@X.co]\n",
        )
        .unwrap();
        assert!(cfg.namespace_registered("b1", &["ns1".into()]));
        assert!(cfg.namespace_registered("b1", &["a".into(), "b".into()]));
        assert!(!cfg.namespace_registered("b1", &["a".into()])); // ancestor ≠ namespace
        assert!(!cfg.namespace_registered("b2", &["ns1".into()]));
        assert!(cfg.is_admin("admin@x.co")); // case-insensitive
        assert!(!cfg.is_admin("other@x.co"));
    }

    #[test]
    fn validation_rejects_bad_segments_duplicates_and_empty_sets() {
        assert!(parse("region: 'bad region'\nbuckets: [{name: b, namespaces: [n]}]\n").is_err());
        assert!(parse("region: r\nbuckets: []\n").is_err());
        assert!(parse("region: r\nbuckets: [{name: b, namespaces: []}]\n").is_err());
        assert!(parse("region: r\nbuckets: [{name: b, namespaces: [n, n]}]\n").is_err());
        assert!(parse("region: r\nbuckets: [{name: b, namespaces: ['x//y']}]\n").is_err());
    }
}

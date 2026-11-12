//! Postgres-backed Lance namespace: the catalog metastore and its migrations.

mod cache;
mod namespace;
mod source;
mod sql;
mod timeline;

use std::str::FromStr;
use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions};

pub use namespace::{
    ColumnBytes, ColumnStorage, PostgresNamespace, Refresh, StorageBreakdown, StorageCategory,
    StorageFootprint, DEFAULT_CACHE_BUDGET_BYTES, STORAGE_CATEGORIES,
};
pub use sql::{SqlColumn, SqlLimits, SqlResult};
pub use timeline::Timeline;

static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Connect to Postgres and apply the embedded migrations.
///
/// When `schema` is set, it is created if missing and becomes the `search_path` of every pooled
/// connection, so migrations and queries stay inside it. Only lowercase unquoted identifiers are
/// accepted to keep the startup parameter free of quoting rules.
pub async fn connect(database_url: &str, schema: Option<&str>) -> Result<PgPool, sqlx::Error> {
    let mut options = PgConnectOptions::from_str(database_url)?;
    if let Some(schema) = schema {
        if !is_plain_identifier(schema) {
            return Err(sqlx::Error::Configuration(
                format!("invalid schema name {schema:?}: use [a-z_][a-z0-9_]*").into(),
            ));
        }
        options = options.options([("search_path", schema)]);
    }
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_secs(5))
        .connect_with(options)
        .await?;
    if let Some(schema) = schema {
        sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
            .execute(&pool)
            .await?;
    }
    MIGRATOR.run(&pool).await?;
    Ok(pool)
}

fn is_plain_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && s.len() <= 63
}

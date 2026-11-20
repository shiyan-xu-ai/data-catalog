use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;
use std::sync::Arc;
use std::time::Duration;

use arrow_json::writer::JsonArray;
use arrow_json::WriterBuilder;
use arrow_schema::{Field, Schema};
use futures::{StreamExt, TryStreamExt};
use lance::datafusion::LanceTableProvider;
use lance::datatypes::BlobHandling;
use lance::deps::arrow_array::RecordBatch;
use lance::deps::datafusion::catalog::TableProvider;
use lance::deps::datafusion::common::TableReference;
use lance::deps::datafusion::error::DataFusionError;
use lance::deps::datafusion::execution::context::SQLOptions;
use lance::deps::datafusion::execution::disk_manager::{DiskManagerBuilder, DiskManagerMode};
use lance::deps::datafusion::execution::runtime_env::RuntimeEnvBuilder;
use lance::deps::datafusion::logical_expr::LogicalPlan;
use lance::deps::datafusion::prelude::{SessionConfig, SessionContext};
use lance::deps::datafusion::sql::parser::{CopyToSource, Statement};
use lance::deps::datafusion::sql::sqlparser::ast::{
    Expr, FunctionArg, FunctionArgExpr, FunctionArguments, Ident, ObjectName, ObjectNamePart,
    Query, Select, SelectItem, SelectItemQualifiedWildcardKind, TableAlias, TableFactor, VisitMut,
    VisitorMut,
};
use lance::Dataset;
use lance_namespace::{Error, NamespaceError, Result};
use serde_json::Value;

use crate::namespace::{
    internal, invalid_input, is_missing, is_version_missing, PathTable, READ_CONCURRENCY,
};
use crate::PostgresNamespace;

/// Bounds applied to every query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlLimits {
    /// Rows returned at most; a larger result is truncated and flagged.
    pub max_rows: usize,
    pub timeout: Duration,
    /// Memory pool of the query's DataFusion runtime; exceeding it fails the query.
    pub memory_limit_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlColumn {
    pub name: String,
    /// Arrow data type, as Arrow displays it (`Int32`, `Utf8`, ...).
    pub data_type: String,
    /// Arrow field metadata, such as `ARROW:extension:name` (`lance.blob.v2` on blob columns).
    pub metadata: HashMap<String, String>,
}

/// A table version a query read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlTable {
    /// Catalog path of the table.
    pub id: Vec<String>,
    pub version: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SqlResult {
    /// Every table version the query read, in order of first appearance in the statement.
    pub tables: Vec<SqlTable>,
    pub columns: Vec<SqlColumn>,
    /// One JSON array per row, values encoded with Arrow's JSON writer.
    pub rows: Vec<Vec<Value>>,
    pub truncated: bool,
}

impl PostgresNamespace {
    /// Runs one read-only query over any registered tables. A table is named by its full catalog
    /// path, `"a"."b"."events"`, which reads its current version; `"a"."b"."events@v12"` reads
    /// version 12.
    ///
    /// DDL, DML (including `COPY`), and other statements are rejected at planning; spilling is
    /// disabled so nothing touches the local disk; blob columns read as their descriptors, so
    /// payload bytes are never fetched. The time limit covers opening every table.
    pub async fn query_sql(&self, sql: &str, limits: &SqlLimits) -> Result<SqlResult> {
        tokio::time::timeout(limits.timeout, self.run_sql(sql, limits))
            .await
            .map_err(|_| {
                invalid_input(format!(
                    "query exceeded the {}s time limit",
                    limits.timeout.as_secs()
                ))
            })?
    }

    async fn run_sql(&self, sql: &str, limits: &SqlLimits) -> Result<SqlResult> {
        let ctx = session(limits)?;
        let mut statement = parse(&ctx, sql)?;
        let references = rewrite(&mut statement)?;
        let mut tables = self.open_references(&references).await?;
        let first = execute(&ctx, &statement, &references, &tables, limits).await;
        match &first {
            Err(e) if is_version_missing(e) => {}
            _ => return first,
        }
        // A current version whose files cleanup removed advances to the newest version, and the
        // query runs once more; pinned versions never advance.
        let mut advanced = false;
        for (reference, table) in references.iter().zip(&mut tables) {
            if table.pinned.is_some() {
                continue;
            }
            let opened = table.table.dataset.version_id();
            if let Some(newest) = self
                .advance_if_gone(&table.id, opened)
                .await
                .map_err(|e| path_error(e, &reference.quoted()))?
            {
                table.table = newest;
                advanced = true;
            }
        }
        if !advanced {
            return first;
        }
        execute(&ctx, &statement, &references, &tables, limits).await
    }

    async fn open_references(&self, references: &[Reference]) -> Result<Vec<PathTable>> {
        let opens: Vec<_> = references.iter().map(|r| self.open_reference(r)).collect();
        futures::stream::iter(opens)
            .buffered(READ_CONCURRENCY)
            .try_collect()
            .await
    }

    async fn open_reference(&self, reference: &Reference) -> Result<PathTable> {
        self.open_path(&reference.path)
            .await
            .map_err(|e| path_error(e, &reference.quoted()))
    }
}

/// Gives a per-reference failure the path as written in the statement, replacing the storage
/// location or joined id the open helpers report.
fn path_error(e: Error, path: &str) -> Error {
    let Error::Namespace { source, .. } = &e else {
        return e;
    };
    let Some(error) = source.downcast_ref::<NamespaceError>() else {
        return e;
    };
    let message = match error {
        NamespaceError::TableNotFound { .. } => {
            format!("table {path} not found: it is not registered, or its dataset no longer exists")
        }
        NamespaceError::TableVersionNotFound { .. } => {
            format!("table {path}: the version does not exist")
        }
        other => format!("table {path}: {}", other.message()),
    };
    NamespaceError::from_code(error.code().as_u32(), message).into()
}

fn session(limits: &SqlLimits) -> Result<SessionContext> {
    let runtime = RuntimeEnvBuilder::new()
        .with_memory_limit(limits.memory_limit_bytes, 1.0)
        .with_disk_manager_builder(
            DiskManagerBuilder::default().with_mode(DiskManagerMode::Disabled),
        )
        .build_arc()
        .map_err(sql_error)?;
    Ok(SessionContext::new_with_config_rt(
        SessionConfig::new(),
        runtime,
    ))
}

fn parse(ctx: &SessionContext, sql: &str) -> Result<Statement> {
    let state = ctx.state();
    let dialect = state.config().options().sql_parser.dialect;
    state.sql_to_statement(sql, &dialect).map_err(sql_error)
}

async fn execute(
    ctx: &SessionContext,
    statement: &Statement,
    references: &[Reference],
    opened: &[PathTable],
    limits: &SqlLimits,
) -> Result<SqlResult> {
    for (reference, table) in references.iter().zip(opened) {
        let name = TableReference::bare(reference.synthetic.as_str());
        ctx.deregister_table(name.clone()).map_err(sql_error)?;
        ctx.register_table(name, table_provider(table.table.dataset.clone()))
            .map_err(sql_error)?;
    }
    let mut tables: Vec<SqlTable> = Vec::new();
    for table in opened {
        let read = SqlTable {
            id: table.id.clone(),
            version: table.table.dataset.version_id(),
        };
        if !tables.contains(&read) {
            tables.push(read);
        }
    }
    let plan = ctx
        .state()
        .statement_to_plan(statement.clone())
        .await
        .map_err(sql_error)?;
    SQLOptions::new()
        .with_allow_ddl(false)
        .with_allow_dml(false)
        .with_allow_statements(false)
        .verify_plan(&plan)
        .map_err(sql_error)?;
    // DataFusion requires EXPLAIN and EXPLAIN ANALYZE at the plan's root, so a limit cannot wrap
    // them; their output is a handful of rows.
    let at_root = matches!(plan, LogicalPlan::Explain(_) | LogicalPlan::Analyze(_));
    let frame = ctx.execute_logical_plan(plan).await.map_err(sql_error)?;
    let schema = frame.schema().inner().clone();
    // One row past the limit tells whether the result was truncated.
    let frame = if at_root {
        frame
    } else {
        frame
            .limit(0, Some(limits.max_rows.saturating_add(1)))
            .map_err(sql_error)?
    };
    let batches = frame.collect().await.map_err(sql_error)?;
    let mut rows = encode_rows(&schema, &batches)?;
    let truncated = rows.len() > limits.max_rows;
    rows.truncate(limits.max_rows);
    Ok(SqlResult {
        tables,
        columns: schema
            .fields()
            .iter()
            .map(|f| SqlColumn {
                name: f.name().clone(),
                data_type: f.data_type().to_string(),
                metadata: f.metadata().clone(),
            })
            .collect(),
        rows,
        truncated,
    })
}

/// A catalog path a statement names, registered in the query's session as `synthetic`.
struct Reference {
    path: Vec<String>,
    leaf: Ident,
    synthetic: String,
    /// Whether some relation reading this path has no user alias; only then can the path
    /// qualify column references.
    unaliased: bool,
}

impl Reference {
    fn quoted(&self) -> String {
        self.path
            .iter()
            .map(|segment| format!("\"{}\"", segment.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(".")
    }
}

/// Rewrites the statement so every table it reads is a one-part name registered in the query's
/// session, and returns those tables. DataFusion names stop at `catalog.schema.table` while
/// namespaces nest to any depth, so catalog paths cannot go through its catalog hierarchy.
fn rewrite(statement: &mut Statement) -> Result<Vec<Reference>> {
    let mut rewriter = Rewriter::default();
    if let ControlFlow::Break(e) = walk(statement, &mut rewriter, &mut |rewriter, name| {
        rewriter.rewrite_name(name).map_continue(|_| ())
    }) {
        return Err(e);
    }
    let mut qualifier = Qualifier {
        references: &rewriter.references,
        default_aliases: &rewriter.default_aliases,
    };
    let _ = walk(statement, &mut qualifier, &mut |_, _| {
        ControlFlow::Continue(())
    });
    Ok(rewriter.references)
}

/// Visits what the planner would resolve table names in: the statement inside DataFusion's
/// wrapper, `EXPLAIN` bodies, and `COPY` sources. `copy_relation` handles a `COPY <name> TO`
/// source, which is a bare name rather than a query.
fn walk<V: VisitorMut>(
    statement: &mut Statement,
    visitor: &mut V,
    copy_relation: &mut impl FnMut(&mut V, &mut ObjectName) -> ControlFlow<V::Break>,
) -> ControlFlow<V::Break> {
    match statement {
        Statement::Statement(statement) => statement.visit(visitor),
        Statement::Explain(explain) => walk(&mut explain.statement, visitor, copy_relation),
        Statement::CopyTo(copy) => match &mut copy.source {
            CopyToSource::Query(query) => query.visit(visitor),
            CopyToSource::Relation(name) => copy_relation(visitor, name),
        },
        Statement::CreateExternalTable(_) | Statement::Reset(_) => ControlFlow::Continue(()),
    }
}

/// DataFusion's identifier normalization: unquoted identifiers are case-insensitive.
fn normalized(ident: &Ident) -> String {
    match ident.quote_style {
        Some(_) => ident.value.clone(),
        None => ident.value.to_lowercase(),
    }
}

#[derive(Default)]
struct Rewriter {
    ctes: HashSet<String>,
    references: Vec<Reference>,
    default_aliases: HashMap<String, HashSet<usize>>,
}

impl Rewriter {
    /// Replaces a catalog path with its synthetic name and returns its reference; `None` for a
    /// name this does not rewrite.
    fn rewrite_name(&mut self, name: &mut ObjectName) -> ControlFlow<Error, Option<usize>> {
        let Some(parts) = name
            .0
            .iter()
            .map(ObjectNamePart::as_ident)
            .collect::<Option<Vec<_>>>()
        else {
            return ControlFlow::Continue(None);
        };
        if let [only] = parts.as_slice() {
            return if self.ctes.contains(&normalized(only)) {
                ControlFlow::Continue(None)
            } else {
                ControlFlow::Break(invalid_input(format!(
                    "table {:?} is not a catalog path: name tables by their full path, \
                     such as \"namespace\".\"table\"",
                    only.value
                )))
            };
        }
        let path: Vec<String> = parts.iter().map(|part| part.value.clone()).collect();
        let index = match self.references.iter().position(|r| r.path == path) {
            Some(index) => index,
            None => {
                self.references.push(Reference {
                    leaf: parts[parts.len() - 1].clone(),
                    synthetic: format!("{}#{}", path.join("."), self.references.len()),
                    path,
                    unaliased: false,
                });
                self.references.len() - 1
            }
        };
        *name = ObjectName::from(vec![Ident::with_quote(
            '"',
            self.references[index].synthetic.clone(),
        )]);
        ControlFlow::Continue(Some(index))
    }
}

impl VisitorMut for Rewriter {
    type Break = Error;

    fn pre_visit_query(&mut self, query: &mut Query) -> ControlFlow<Error> {
        if let Some(with) = &query.with {
            self.ctes.extend(
                with.cte_tables
                    .iter()
                    .map(|cte| normalized(&cte.alias.name)),
            );
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, factor: &mut TableFactor) -> ControlFlow<Error> {
        if let TableFactor::Table {
            name,
            alias,
            args: None,
            ..
        } = factor
        {
            if let Some(index) = self.rewrite_name(name)? {
                if alias.is_none() {
                    let reference = &mut self.references[index];
                    reference.unaliased = true;
                    self.default_aliases
                        .entry(normalized(&reference.leaf))
                        .or_default()
                        .insert(index);
                }
            }
        }
        ControlFlow::Continue(())
    }
}

/// Gives rewritten relations their default alias and rewrites path-qualified references.
struct Qualifier<'a> {
    references: &'a [Reference],
    default_aliases: &'a HashMap<String, HashSet<usize>>,
}

impl Qualifier<'_> {
    /// Whether the reference's relation keeps its leaf as its alias: no other relation would
    /// get the same one.
    fn has_default_alias(&self, reference: &Reference) -> bool {
        self.default_aliases
            .get(&normalized(&reference.leaf))
            .is_some_and(|references| references.len() == 1)
    }

    fn qualifier(&self, reference: &Reference) -> Ident {
        if self.has_default_alias(reference) {
            reference.leaf.clone()
        } else {
            Ident::with_quote('"', reference.synthetic.clone())
        }
    }

    /// Replaces the longest leading run of `idents` that is a path read by an unaliased relation
    /// with that relation's qualifier, keeping at least `keep` trailing idents.
    fn qualify(&self, idents: &mut Vec<Ident>, keep: usize) {
        for len in (2..=idents.len().saturating_sub(keep)).rev() {
            let prefix = &idents[..len];
            let found = self.references.iter().find(|r| {
                r.unaliased
                    && r.path.len() == len
                    && r.path.iter().zip(prefix).all(|(s, i)| *s == i.value)
            });
            if let Some(reference) = found {
                let qualifier = self.qualifier(reference);
                idents.splice(..len, [qualifier]);
                return;
            }
        }
    }

    fn qualify_wildcard(&self, name: &mut ObjectName) {
        let Some(mut idents) = name
            .0
            .iter()
            .map(|part| part.as_ident().cloned())
            .collect::<Option<Vec<_>>>()
        else {
            return;
        };
        self.qualify(&mut idents, 0);
        *name = ObjectName::from(idents);
    }
}

impl VisitorMut for Qualifier<'_> {
    type Break = Error;

    fn pre_visit_table_factor(&mut self, factor: &mut TableFactor) -> ControlFlow<Error> {
        if let TableFactor::Table {
            name,
            alias: alias @ None,
            args: None,
            ..
        } = factor
        {
            let reference = match name.0.as_slice() {
                [ObjectNamePart::Identifier(only)] => {
                    self.references.iter().find(|r| r.synthetic == only.value)
                }
                _ => None,
            };
            if let Some(reference) = reference.filter(|r| self.has_default_alias(r)) {
                *alias = Some(TableAlias {
                    explicit: false,
                    name: reference.leaf.clone(),
                    columns: Vec::new(),
                    at: None,
                });
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_select(&mut self, select: &mut Select) -> ControlFlow<Error> {
        for item in &mut select.projection {
            if let SelectItem::QualifiedWildcard(
                SelectItemQualifiedWildcardKind::ObjectName(name),
                _,
            ) = item
            {
                self.qualify_wildcard(name);
            }
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_expr(&mut self, expr: &mut Expr) -> ControlFlow<Error> {
        match expr {
            Expr::CompoundIdentifier(idents) => self.qualify(idents, 1),
            Expr::QualifiedWildcard(name, _) => self.qualify_wildcard(name),
            Expr::Function(function) => {
                if let FunctionArguments::List(list) = &mut function.args {
                    for arg in &mut list.args {
                        let (FunctionArg::Named { arg, .. }
                        | FunctionArg::ExprNamed { arg, .. }
                        | FunctionArg::Unnamed(arg)) = arg;
                        if let FunctionArgExpr::QualifiedWildcard(name) = arg {
                            self.qualify_wildcard(name);
                        }
                    }
                }
            }
            _ => {}
        }
        ControlFlow::Continue(())
    }
}

/// Blob columns read as descriptors, so queries never fetch payload bytes.
fn table_provider(dataset: Arc<Dataset>) -> Arc<dyn TableProvider> {
    Arc::new(
        LanceTableProvider::new(dataset, false, false)
            .with_blob_handling(BlobHandling::BlobsDescriptions),
    )
}

/// Encodes rows with Arrow's JSON writer. Columns are renamed positionally first so duplicate
/// output names (`SELECT 1, 1`) cannot collide in the writer's JSON objects.
fn encode_rows(schema: &Schema, batches: &[RecordBatch]) -> Result<Vec<Vec<Value>>> {
    let keys: Vec<String> = (0..schema.fields().len())
        .map(|i| format!("c{i}"))
        .collect();
    let positional = Arc::new(Schema::new(
        schema
            .fields()
            .iter()
            .zip(&keys)
            .map(|(f, key)| Field::new(key, f.data_type().clone(), true))
            .collect::<Vec<_>>(),
    ));
    let mut writer = WriterBuilder::new()
        .with_explicit_nulls(true)
        .build::<_, JsonArray>(Vec::new());
    for batch in batches {
        let batch = RecordBatch::try_new(positional.clone(), batch.columns().to_vec())
            .map_err(|e| internal(format!("encode query result: {e}")))?;
        writer
            .write(&batch)
            .map_err(|e| invalid_input(format!("cannot encode query result as JSON: {e}")))?;
    }
    writer
        .finish()
        .map_err(|e| internal(format!("encode query result: {e}")))?;
    let buffer = writer.into_inner();
    let objects: Vec<serde_json::Map<String, Value>> = if buffer.is_empty() {
        Vec::new()
    } else {
        serde_json::from_slice(&buffer)
            .map_err(|e| internal(format!("decode query result: {e}")))?
    };
    Ok(objects
        .into_iter()
        .map(|mut object| {
            keys.iter()
                .map(|key| object.remove(key).unwrap_or(Value::Null))
                .collect()
        })
        .collect())
}

fn sql_error(e: DataFusionError) -> Error {
    let message = format!("query failed: {e}");
    match e.find_root() {
        DataFusionError::ResourcesExhausted(message) => {
            invalid_input(format!("query exceeded the memory limit: {message}"))
        }
        DataFusionError::ObjectStore(err)
            if matches!(
                **err,
                lance::deps::datafusion::object_store::Error::NotFound { .. }
            ) =>
        {
            NamespaceError::TableVersionNotFound { message }.into()
        }
        DataFusionError::External(err)
            if err.downcast_ref::<lance::Error>().is_some_and(is_missing) =>
        {
            NamespaceError::TableVersionNotFound { message }.into()
        }
        DataFusionError::ObjectStore(_)
        | DataFusionError::IoError(_)
        | DataFusionError::External(_)
        | DataFusionError::Internal(_) => internal(message),
        _ => invalid_input(e.to_string()),
    }
}

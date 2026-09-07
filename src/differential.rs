//! Differential Cypher (A8): the pinned query set, result cells normalized
//! across engines so a Bolt integer, a Redis bulk string and a Grust `Value`
//! compare as one thing, ordered and multiset comparison, and the route a
//! query takes on each store, named the way the LSQB harness names it.

use std::cmp::Ordering;

use grust::Value;
use grust_cypher::CypherResultTable;
use grust_cypher::pushdown::{NoTypeHints, SqlDialect, plan_read, plan_scalar_count_read};
use grust_cypher::read::{IndexedReadPlan, classify_indexed_read_query};

/// One result cell, as every engine's driver reports it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(untagged)]
pub enum Cell {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    List(Vec<Cell>),
}

/// The text a non-scalar Grust value carries across the wire: dates,
/// decimals and durations as their canonical strings.
pub fn value_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::String(s) => s.clone(),
        Value::StringArray(items) => items.join(","),
        Value::DateTime(d) => d.to_string(),
        Value::Decimal(d) => d.to_string(),
        Value::Duration(d) => d.to_string(),
        #[allow(unreachable_patterns)]
        other => format!("{other:?}"),
    }
}

impl Cell {
    pub fn from_grust(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(b) => Self::Bool(*b),
            Value::Int(i) => Self::Int(*i),
            Value::Float(f) => Self::Float(*f),
            Value::String(s) => Self::Str(s.clone()),
            Value::StringArray(items) => {
                Self::List(items.iter().map(|s| Self::Str(s.clone())).collect())
            }
            other => Self::Str(value_text(other)),
        }
    }

    pub fn from_json(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(b) => Self::Bool(*b),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => Self::Int(i),
                None => Self::Float(n.as_f64().unwrap_or(f64::NAN)),
            },
            serde_json::Value::String(s) => Self::Str(s.clone()),
            serde_json::Value::Array(items) => {
                Self::List(items.iter().map(Self::from_json).collect())
            }
            serde_json::Value::Object(_) => Self::Str(value.to_string()),
        }
    }

    /// A total order for multiset comparison: type, then value.
    fn rank(&self) -> u8 {
        match self {
            Self::Null => 0,
            Self::Bool(_) => 1,
            Self::Int(_) => 2,
            Self::Float(_) => 3,
            Self::Str(_) => 4,
            Self::List(_) => 5,
        }
    }

    fn compare(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            (Self::Int(a), Self::Int(b)) => a.cmp(b),
            (Self::Float(a), Self::Float(b)) => a.total_cmp(b),
            (Self::Str(a), Self::Str(b)) => a.cmp(b),
            (Self::List(a), Self::List(b)) => compare_rows(a, b),
            _ => self.rank().cmp(&other.rank()),
        }
    }
}

fn compare_rows(a: &[Cell], b: &[Cell]) -> Ordering {
    a.iter()
        .zip(b)
        .map(|(x, y)| x.compare(y))
        .find(|o| *o != Ordering::Equal)
        .unwrap_or_else(|| a.len().cmp(&b.len()))
}

/// Columns and rows as one engine returned them.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize)]
pub struct ResultSet {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
}

impl ResultSet {
    pub fn from_table(table: CypherResultTable) -> Self {
        Self {
            columns: table.columns,
            rows: table
                .rows
                .iter()
                .map(|row| row.iter().map(Cell::from_grust).collect())
                .collect(),
        }
    }

    /// Cells reordered to `columns` when every name is present, so engines
    /// that report columns in another order still compare cell by cell.
    pub fn aligned_to(mut self, columns: &[String]) -> Self {
        if self.rows.is_empty() && self.columns.is_empty() {
            // A driver that reports no rows may report no header either; an
            // empty result compares by its emptiness, not by a missing header.
            self.columns = columns.to_vec();
            return self;
        }
        if self.columns == columns || self.columns.len() != columns.len() {
            return self;
        }
        let positions: Option<Vec<usize>> = columns
            .iter()
            .map(|name| self.columns.iter().position(|c| c == name))
            .collect();
        let Some(positions) = positions else {
            return self;
        };
        self.rows = self
            .rows
            .into_iter()
            .map(|row| {
                positions
                    .iter()
                    .map(|&i| row.get(i).cloned().unwrap_or(Cell::Null))
                    .collect()
            })
            .collect();
        self.columns = columns.to_vec();
        self
    }

    /// Rows in a canonical order unless the query ordered them itself.
    pub fn normalized(mut self, ordered: bool) -> Self {
        if !ordered {
            self.rows.sort_by(|a, b| compare_rows(a, b));
        }
        self
    }

    /// `None` when equal; otherwise the first difference, in words.
    pub fn diff(&self, other: &Self) -> Option<String> {
        if self.columns.len() != other.columns.len() {
            return Some(format!(
                "{} columns vs {}",
                self.columns.len(),
                other.columns.len()
            ));
        }
        if self.rows.len() != other.rows.len() {
            return Some(format!("{} rows vs {}", self.rows.len(), other.rows.len()));
        }
        self.rows
            .iter()
            .zip(&other.rows)
            .enumerate()
            .find_map(|(i, (a, b))| {
                (compare_rows(a, b) != Ordering::Equal).then(|| {
                    format!(
                        "row {i}: {} vs {}",
                        serde_json::to_string(a).unwrap_or_default(),
                        serde_json::to_string(b).unwrap_or_default()
                    )
                })
            })
    }
}

/// One pinned query of the A8 set (`scenarios/v1/a8/queries.json`).
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct QuerySpec {
    pub id: String,
    pub schema: String,
    pub kind: String,
    pub ordered: bool,
    pub source: String,
    pub source_sha256: String,
    pub cypher: String,
}

#[derive(serde::Deserialize)]
struct QueryFile {
    queries: Vec<QuerySpec>,
}

const QUERIES: &str = include_str!("../scenarios/v1/a8/queries.json");

/// The queries defined over one schema, in file order.
pub fn queries_for(schema: &str) -> Vec<QuerySpec> {
    let file: QueryFile =
        serde_json::from_str(QUERIES).expect("scenarios/v1/a8/queries.json parses");
    file.queries
        .into_iter()
        .filter(|q| q.schema == schema)
        .collect()
}

/// The query schema a loader's format carries, if any.
pub fn schema_of(format: &str) -> Option<&'static str> {
    match format {
        crate::dataset::snb::FORMAT => Some("ldbc-snb"),
        crate::dataset::icij::FORMAT => Some("icij"),
        _ => None,
    }
}

/// How a store answers a query, in the LSQB harness's vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Route {
    /// grust-cypher's clause-by-clause reference executor over the in-process graph.
    InProcessReference,
    /// A proven non-materializing count plan over the resident typed index.
    ResidentIndexRustCount,
    /// The store's own `SELECT COUNT(*)` rendered by Grust pushdown.
    NativeAggregate,
    /// SQL row source rendered by Grust pushdown, projected in Rust.
    RowSourceRustProjection,
    /// The store's rows read back and the reference executor run over them.
    MaterializeRustReference,
    /// The engine's own Cypher, submitted as written.
    NativeCypher,
}

/// The oracle's answer: the proven count plan over a typed index of the
/// in-process graph where the indexed executor proves one (the LSQB
/// shapes materialize nothing that way; the clause-by-clause reference
/// executor exhausted 12 GB on a nine-hop chain at a 200,000-edge slice),
/// and the reference executor over the graph for every other shape.
pub fn oracle(
    graph: &grust::Graph,
    index: &grust::TypedGraphIndex,
    cypher: &str,
) -> grust::Result<(ResultSet, Route)> {
    let params = grust_cypher::CypherParameters::new();
    if resident_proven(cypher) {
        let table = grust_cypher::read::run_read_query_indexed(index, cypher, &params)?;
        return Ok((ResultSet::from_table(table), Route::ResidentIndexRustCount));
    }
    let table = grust_cypher::read::run_read_query(graph, cypher, &params)?;
    Ok((ResultSet::from_table(table), Route::InProcessReference))
}

/// Whether the indexed executor proves a non-materializing count plan.
pub fn resident_proven(cypher: &str) -> bool {
    let Ok(query) = grust_cypher::parser::parse_query(cypher) else {
        return false;
    };
    if grust_cypher::semantics::analyze(&query).is_err() {
        return false;
    }
    matches!(
        classify_indexed_read_query(&query),
        Ok(IndexedReadPlan::CountFactorized)
    )
}

/// The route a SQL-backed Grust store takes, mirroring its `run_read_query`.
pub fn sql_route(cypher: &str, dialect: &dyn SqlDialect) -> Route {
    let params = grust_cypher::CypherParameters::new();
    if plan_scalar_count_read(cypher, &params, &NoTypeHints)
        .ok()
        .flatten()
        .is_some_and(|plan| plan.supported_by(dialect))
    {
        return Route::NativeAggregate;
    }
    if plan_read(cypher, &params, &NoTypeHints)
        .ok()
        .flatten()
        .is_some_and(|plan| plan.supported_by(dialect))
    {
        return Route::RowSourceRustProjection;
    }
    Route::MaterializeRustReference
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_query_file_covers_both_schemas() {
        let snb = queries_for("ldbc-snb");
        let icij = queries_for("icij");
        assert_eq!(
            snb.len(),
            29,
            "9 LSQB shapes, 13 count attacks, 7 row queries"
        );
        assert_eq!(icij.len(), 9);
        assert!(
            snb.iter()
                .all(|q| !q.cypher.contains(":Post") && !q.cypher.contains(":Comment"))
        );
        assert!(
            snb.iter()
                .filter(|q| q.kind == "rows")
                .all(|q| q.ordered || q.id.contains("unordered"))
        );
    }

    #[test]
    fn cells_compare_across_encodings_and_multisets_ignore_order() {
        let a = ResultSet {
            columns: vec!["x".into()],
            rows: vec![
                vec![Cell::Int(2)],
                vec![Cell::Str("b".into())],
                vec![Cell::Int(1)],
            ],
        };
        let b = ResultSet {
            columns: vec!["x".into()],
            rows: vec![
                vec![Cell::Int(1)],
                vec![Cell::Int(2)],
                vec![Cell::Str("b".into())],
            ],
        };
        assert!(
            a.clone()
                .normalized(true)
                .diff(&b.clone().normalized(true))
                .is_some()
        );
        assert_eq!(a.normalized(false).diff(&b.normalized(false)), None);
        assert_eq!(Cell::from_json(&serde_json::json!(8)), Cell::Int(8));
        assert_eq!(Cell::from_grust(&Value::Int(8)), Cell::Int(8));
        let headerless = ResultSet::default().aligned_to(&["x".to_string()]);
        assert_eq!(
            headerless.diff(&ResultSet {
                columns: vec!["x".into()],
                rows: vec![]
            }),
            None
        );
        let short = ResultSet {
            columns: vec!["x".into()],
            rows: vec![],
        };
        assert_eq!(
            short.diff(&ResultSet {
                columns: vec!["x".into()],
                rows: vec![vec![Cell::Null]]
            }),
            Some("0 rows vs 1".into())
        );
    }

    #[test]
    fn routes_follow_the_planners() {
        assert!(resident_proven(
            "MATCH (p:Person)-[:KNOWS]->(q:Person) RETURN count(*) AS count"
        ));
        assert!(!resident_proven(
            "MATCH (p:Person) RETURN p.id AS id ORDER BY id LIMIT 5"
        ));
        let dialect = grust::TursoReadDialect::new("ag");
        assert_eq!(
            sql_route(
                "MATCH (n) WHERE n.kind = 'Comment' RETURN count(*)",
                &dialect
            ),
            Route::NativeAggregate
        );
        assert_eq!(
            sql_route(
                "MATCH (p:Person) RETURN p.id AS id ORDER BY id LIMIT 5",
                &dialect
            ),
            Route::RowSourceRustProjection
        );
    }
}

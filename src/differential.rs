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
    /// The harness's own Rust over the loaded graph, for the two pinned
    /// reference shapes the in-process executor cannot finish (the answer
    /// key only; never a store's route).
    NativeOracle,
}

/// The budget every in-process execution runs under: a cooperative wall
/// clock deadline and a cap on intermediate bytes, so a shape the executor
/// cannot answer stops itself instead of running on after the harness has
/// moved on, and no query can exhaust the host (the reference executor
/// exhausted 12 GB on a nine-hop chain before the typed index took the
/// proven counts). Every structural limit is lifted; only time and memory
/// bound the run.
pub const IN_PROCESS_BUDGET: std::time::Duration = std::time::Duration::from_secs(110);

/// The budget every store gets per A8 query. The harness stops waiting at
/// this deadline; a store whose protocol takes a deadline (FalkorDB's
/// `TIMEOUT`) is handed the same one, so it stops executing too.
pub const STORE_BUDGET: std::time::Duration = std::time::Duration::from_secs(120);

/// Whether a store's error is its own declared resource cap refusing the
/// query, stated in the store's typed message -- Memgraph's `--memory-limit`
/// ("Memory limit exceeded! … the maximum allowed size for allocation is
/// set to 5.00GiB"), Neo4j's transaction memory pool
/// (`MemoryPoolOutOfMemoryError`), FalkorDB's `QUERY_MEM_CAPACITY` ("mem
/// consumption exceeded capacity"). The store stopped the query, said
/// so, and is still up: A8 records it as refused, exactly as it records
/// Grust's bounded-read cap on the in-process and SQL routes, and the cell
/// is not comparable past it. Any other failure stays an error. A load
/// that ends at a store's limit is a different thing: no data to measure,
/// still a failing row (§48).
pub fn is_declared_limit(err: &grust::GrustError) -> bool {
    let text = err.to_string();
    matches!(err, grust::GrustError::Backend(_))
        && (text.contains("Memory limit exceeded")
            || text.contains("MemoryPoolOutOfMemoryError")
            || text.contains("mem consumption exceeded capacity"))
}

/// Whether a store's error says it stopped a query at the deadline the
/// harness handed it (FalkorDB: "Query timed out"): recorded as a timeout,
/// the way a query the harness stopped waiting for is, never as a crash.
pub fn is_store_deadline(err: &grust::GrustError) -> bool {
    let text = err.to_string().to_ascii_lowercase();
    matches!(err, grust::GrustError::Backend(_))
        && (text.contains("timed out") || text.contains("timeout"))
}

/// Whether a store's error is Grust's bounded-read policy refusing the
/// query (its cooperative deadline or a resource cap), as opposed to a
/// failure: `bounded read execution timed out`, `bounded read exceeded …`.
/// A8 records such an answer as refused, the way it records any store's
/// declared refusal, never as a crash or a hang.
pub fn is_policy_refusal(err: &grust::GrustError) -> bool {
    err.to_string().contains("bounded read")
}

/// The reference executor's own budget, separate from the store's: the
/// reference is not a measurement, only the answer key, and on LDBC sf0.1
/// two of its row shapes (posts per creator, reply fan-in) need more than
/// the store's 120 s in Grust's in-process executor. Every store was
/// coming out `unsupported` on A8 for that reason alone. Override with
/// AG_REFERENCE_BUDGET_S; the memory backend's own in-process route keeps
/// the store budget, since there it is the system under test.
pub fn reference_budget() -> std::time::Duration {
    std::time::Duration::from_secs(
        std::env::var("AG_REFERENCE_BUDGET_S")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(900),
    )
}

/// The reference executor's intermediate-bytes cap, separate from the
/// store route's 2 GiB: on ICIJ five of nine reference shapes exceed 2 GiB
/// while binding their start nodes, and the answer key was missing for
/// them. AG_REFERENCE_INTERMEDIATE_GB (default 8) sizes it to the host;
/// the host guard still bounds the process.
pub fn reference_intermediate_bytes() -> usize {
    std::env::var("AG_REFERENCE_INTERMEDIATE_GB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(8)
        << 30
}

pub fn in_process_policy() -> grust_cypher::ReadQueryPolicy {
    in_process_policy_with(IN_PROCESS_BUDGET, 2 << 30)
}

/// The reference executor's policy: its own time budget and intermediate cap.
pub fn reference_policy() -> grust_cypher::ReadQueryPolicy {
    in_process_policy_with(reference_budget(), reference_intermediate_bytes())
}

pub fn in_process_policy_with(
    max_execution_time: std::time::Duration,
    max_intermediate_bytes: usize,
) -> grust_cypher::ReadQueryPolicy {
    grust_cypher::ReadQueryPolicy {
        max_query_bytes: 1 << 20,
        max_parameter_bytes: 1 << 20,
        max_graph_nodes: usize::MAX,
        max_graph_edges: usize::MAX,
        max_graph_bytes: usize::MAX,
        max_candidate_work: usize::MAX,
        max_intermediate_bytes,
        max_result_rows: BOUNDED_LIMIT,
        max_output_bytes: usize::MAX,
        max_range_items: grust_cypher::MAX_RANGE_ITEMS,
        max_union_arms: 16,
        max_path_length: 256,
        max_execution_time,
        allow_graph_selection: false,
        allow_catalog_procedures: false,
        require_match: false,
    }
}

const BOUNDED_LIMIT: usize = 1_000_000_000;

/// The bounded read API requires a literal `LIMIT` on every arm. A pinned
/// query without one gets the policy's ceiling appended, which changes no
/// result: the store still receives the text as pinned.
pub fn bounded_text(cypher: &str) -> String {
    let mut out = String::with_capacity(cypher.len() + 32);
    let mut arm = String::new();
    let flush = |arm: &mut String, out: &mut String| {
        let has_limit = arm
            .split_whitespace()
            .any(|word| word.eq_ignore_ascii_case("limit"));
        out.push_str(arm.trim_end());
        if !has_limit {
            out.push_str(&format!("\nLIMIT {BOUNDED_LIMIT}"));
        }
        arm.clear();
    };
    for line in cypher.lines() {
        let word = line.trim();
        if word.eq_ignore_ascii_case("union") || word.eq_ignore_ascii_case("union all") {
            flush(&mut arm, &mut out);
            out.push('\n');
            out.push_str(word);
            out.push('\n');
            continue;
        }
        arm.push_str(line);
        arm.push('\n');
    }
    flush(&mut arm, &mut out);
    out
}

/// The oracle's answer: the proven count plan over a typed index of the
/// in-process graph where the indexed executor proves one (the LSQB
/// shapes materialize nothing that way), and the reference executor over
/// the graph for every other shape, under `in_process_policy`.
pub fn oracle(
    graph: &grust::Graph,
    index: &grust::TypedGraphIndex,
    cypher: &str,
) -> grust::Result<(ResultSet, Route)> {
    let params = grust_cypher::CypherParameters::new();
    if let Some(table) = native_oracle(graph, cypher) {
        return Ok((table, Route::NativeOracle));
    }
    if resident_proven(cypher) {
        // A proven count plan materializes nothing and answers in
        // milliseconds; it runs as pinned, since an appended LIMIT would
        // take it out of the proof.
        let table = grust_cypher::read::run_read_query_indexed(index, cypher, &params)?;
        return Ok((ResultSet::from_table(table), Route::ResidentIndexRustCount));
    }
    // The bounded indexed entrypoint falls back to the reference executor
    // for an unproven shape and measures the graph once through the index
    // instead of serializing it per query (10 s per query on a 200k slice).
    let _ = graph;
    let policy = reference_policy();
    let table = grust_cypher::run_bounded_read_query_indexed(
        index,
        &bounded_text(cypher),
        &params,
        &policy,
    )?;
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

    /// Diagnostic, not a test of behaviour: times the unbounded reference
    /// executor against the bounded indexed path on the SNB slice.
    /// `cargo test --release -- --ignored --nocapture oracle_probe`
    #[test]
    #[ignore = "needs datasets/ and a minute of CPU"]
    fn oracle_probe() {
        use std::sync::Arc;
        use std::time::Instant;
        let path = std::path::Path::new(
            "datasets/social_network-sf0.1-CsvBasic-LongDateFormatter.tar.zst",
        );
        let (loaded, stats, _) = crate::dataset::load_dataset(path, Some(200_000), false).unwrap();
        let crate::dataset::LoadedGraph::Full(graph) = loaded else {
            unreachable!("not compact")
        };
        eprintln!("loaded {} nodes / {} edges", stats.nodes, stats.edges);
        let index = grust::TypedGraphIndex::new(Arc::clone(&graph)).unwrap();
        let params = grust_cypher::CypherParameters::new();
        let policy = in_process_policy();
        for spec in queries_for("ldbc-snb").iter().filter(|q| q.kind == "rows") {
            let t = Instant::now();
            let a = grust_cypher::read::run_read_query(&graph, &spec.cypher, &params)
                .map(|t| t.rows.len());
            eprintln!(
                "{} unbounded reference: {:?} rows={a:?}",
                spec.id,
                t.elapsed()
            );
            let text = bounded_text(&spec.cypher);
            let t = Instant::now();
            let b = grust_cypher::run_bounded_read_query_indexed(&index, &text, &params, &policy)
                .map(|t| t.rows.len());
            eprintln!(
                "{} bounded indexed:     {:?} rows={b:?}",
                spec.id,
                t.elapsed()
            );
        }
    }

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
    fn a_store_deadline_is_a_timeout_and_other_errors_are_not() {
        let deadline = grust::GrustError::Backend("falkor GRAPH.RO_QUERY: Query timed out".into());
        assert!(is_store_deadline(&deadline));
        let memory = grust::GrustError::Backend(
            "neo4j: Memory limit exceeded! Attempting to allocate a chunk".into(),
        );
        assert!(!is_store_deadline(&memory));
        assert!(is_declared_limit(&memory));
        assert!(is_declared_limit(&grust::GrustError::Backend(
            "Neo.TransientError.General.MemoryPoolOutOfMemoryError The allocation of an extra 2.0 MiB would use more than the limit".into()
        )));
        assert!(!is_declared_limit(&grust::GrustError::Backend(
            "neo4j: connection reset by peer".into()
        )));
        assert!(!is_declared_limit(&deadline));
        let policy = grust::GrustError::CypherExecution("bounded read execution timed out".into());
        assert!(
            !is_store_deadline(&policy),
            "a policy refusal is classified by is_policy_refusal"
        );
        assert!(is_policy_refusal(&policy));
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
    fn bounded_text_appends_a_limit_to_every_arm_that_lacks_one() {
        let two_arms = "MATCH (p:Person)\nRETURN count(*) AS count\nUNION\nMATCH (p:Person)\nRETURN count(*) AS count";
        let bounded = bounded_text(two_arms);
        assert_eq!(bounded.matches("LIMIT 1000000000").count(), 2, "{bounded}");
        assert!(bounded.contains("\nUNION\n"));
        let limited = "MATCH (p:Person) RETURN p.id AS id ORDER BY id LIMIT 5";
        assert_eq!(bounded_text(limited), limited);
        let comment = "/* UNION in a comment */\nMATCH (n)\nRETURN count(n) AS count";
        assert_eq!(bounded_text(comment).matches("LIMIT").count(), 1);
    }

    /// The route each pinned ICIJ text takes on a SQL-backed store: the
    /// counts are proven on the resident index, the row shapes are SQL
    /// row sources, and c4's `WHERE o <> p` is the one the planners refuse
    /// -- the shape that reaches the executor over the store's snapshot.
    #[test]
    fn the_icij_set_routes_on_the_sql_dialect() {
        let dialect = grust::TursoReadDialect::new("ag");
        for spec in queries_for("icij") {
            let route = if resident_proven(&spec.cypher) {
                Route::ResidentIndexRustCount
            } else {
                sql_route(&spec.cypher, &dialect)
            };
            let expected = match spec.id.as_str() {
                "c4-co-officers" => Route::MaterializeRustReference,
                id if id.starts_with('c') => Route::ResidentIndexRustCount,
                _ => Route::RowSourceRustProjection,
            };
            assert_eq!(route, expected, "{}", spec.id);
        }
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

/// Whether a query names a label or relationship type other than the
/// untyped `V`/`E`: `(:Person)`, `[:KNOWS]`, `n:Message`.
pub fn mentions_labels(cypher: &str) -> bool {
    let bytes = cypher.as_bytes();
    let mut i = 0;
    while let Some(off) = cypher[i..].find(':') {
        let at = i + off;
        let rest = &cypher[at + 1..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '`')
            .collect();
        let name = name.trim_matches('`');
        let before = bytes[..at]
            .iter()
            .rev()
            .find(|b| !b.is_ascii_whitespace())
            .copied();
        // A label follows `(`, `[`, or an identifier; a map key or a
        // parameter does not.
        let label_position = matches!(before, Some(b'(') | Some(b'['))
            || before.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b')');
        if label_position
            && !name.is_empty()
            && name != crate::dataset::NODE_LABEL
            && name != crate::dataset::EDGE_LABEL
            && !name.chars().all(|c| c.is_ascii_digit())
        {
            return true;
        }
        i = at + 1;
    }
    false
}

/// The pinned row shapes, and the two count shapes the executor cannot
/// finish. Grust's in-process executor binds every start node of a MATCH
/// before it filters or aggregates, so a row query over the largest label
/// costs a graph's worth of intermediates: LDBC r2 posts per creator and
/// r5 reply fan-in ran past 30 minutes at sf0.1; ICIJ c4 co-officers and
/// r1 officer fan-out exceed 8 GiB; ICIJ r2, r3 and r4 each exceed 2 GiB
/// and took 13-23 s under an 8 GiB cap; and at LDBC sf1 the executor on
/// r3 tag popularity took the client past a 22 GB host guard on top of a
/// 14 GB graph. For exactly these pinned texts the answer key is computed
/// here in Rust over the loaded graph -- a group count with the query's
/// own ORDER BY and LIMIT, a co-occurrence pair count, a filtered
/// projection, a distinct property, an edge projection -- and recorded as
/// `native-oracle`. The count shapes stay with the executor and the
/// resident index's proven plans. Unit tests hold every shape against the
/// executor on a small graph.
pub fn native_oracle(graph: &grust::Graph, cypher: &str) -> Option<ResultSet> {
    const R2: &str = "MATCH (m:Message {kind: 'Post'})-[:HAS_CREATOR]->(p:Person)\nRETURN p.id AS person, count(m) AS posts\nORDER BY posts DESC, person\nLIMIT 50";
    const R5: &str = "MATCH (c:Message {kind: 'Comment'})-[:REPLY_OF]->(m:Message)\nRETURN m.id AS root, count(c) AS replies\nORDER BY replies DESC, root\nLIMIT 20";
    const ICIJ_R1: &str = "MATCH (o:Officer)-[:OFFICER_OF]->(e:Entity)\nRETURN o.id AS officer, count(e) AS entities\nORDER BY entities DESC, officer\nLIMIT 50";
    const ICIJ_C4: &str = "MATCH (o:Officer)-[:OFFICER_OF]->(:Entity)<-[:OFFICER_OF]-(p:Officer)\nWHERE o <> p\nRETURN count(*) AS count";
    const ICIJ_R2: &str = "MATCH (e:Entity)\nWHERE e.jurisdiction = 'SAM'\nRETURN e.id AS id, e.name AS name\nORDER BY id\nLIMIT 100";
    const ICIJ_R3: &str = "MATCH (e:Entity)-[:REGISTERED_ADDRESS]->(a:Address)\nRETURN a.id AS address, count(e) AS entities\nORDER BY entities DESC, address\nLIMIT 25";
    const ICIJ_R4: &str =
        "MATCH (e:Entity)\nRETURN DISTINCT e.jurisdiction AS jurisdiction\nORDER BY jurisdiction";
    const R1: &str = "MATCH (p:Person)-[:KNOWS]->(q:Person)\nRETURN p.id AS a, q.id AS b\nORDER BY a, b\nLIMIT 200";
    const R3: &str = "MATCH (t:Tag)<-[:HAS_TAG]-(m:Message)\nRETURN t.name AS tag, count(*) AS n\nORDER BY n DESC, tag\nLIMIT 25";
    const R4: &str = "MATCH (p:Person)\nWHERE p.gender = 'female'\nRETURN p.id AS id, p.firstName AS firstName\nORDER BY id\nLIMIT 100";
    const R6: &str = "MATCH (p:Person)-[:IS_LOCATED_IN]->(:City)-[:IS_PART_OF]->(c:Country)\nRETURN DISTINCT c.name AS country\nORDER BY country";
    const R7: &str = "MATCH (p:Person)-[:KNOWS]->(q:Person)\nWHERE p.gender = 'male' AND q.gender = 'female'\nRETURN p.id AS a, q.id AS b";
    let text = cypher.trim();
    use std::collections::HashMap;
    // Ascending with an absent value last, as the executor and the engines
    // order a null under ORDER BY.
    fn null_last(a: &Cell, b: &Cell) -> Ordering {
        match (a, b) {
            (Cell::Null, Cell::Null) => Ordering::Equal,
            (Cell::Null, _) => Ordering::Greater,
            (_, Cell::Null) => Ordering::Less,
            _ => a.compare(b),
        }
    }
    let prop =
        |n: &grust::Node, key: &str| n.props.get(key).map(Cell::from_grust).unwrap_or(Cell::Null);
    let string_prop_is = |n: &grust::Node, key: &str, want: &str| matches!(n.props.get(key), Some(grust::Value::String(v)) if v == want);
    // Label by node id, for the edge shapes.
    let label_of = || -> HashMap<&str, &str> {
        graph
            .nodes
            .iter()
            .map(|n| (n.id.as_str(), n.label.as_str()))
            .collect()
    };
    let pairs = |columns: [&str; 2], rows: Vec<(&str, &str)>| ResultSet {
        columns: columns.iter().map(|c| c.to_string()).collect(),
        rows: rows
            .into_iter()
            .map(|(a, b)| vec![Cell::Str(a.to_string()), Cell::Str(b.to_string())])
            .collect(),
    };
    if text == R1 || text == R7 {
        // KNOWS edges between Person nodes; r7 keeps the male -> female ones.
        let gender: HashMap<&str, &str> = graph
            .nodes
            .iter()
            .filter(|n| n.label.as_str() == "Person")
            .filter_map(|n| match n.props.get("gender") {
                Some(grust::Value::String(g)) => Some((n.id.as_str(), g.as_str())),
                _ => None,
            })
            .collect();
        let labels = label_of();
        let mut rows: Vec<(&str, &str)> = graph
            .edges
            .iter()
            .filter(|e| e.label.as_str() == "KNOWS")
            .filter(|e| {
                labels.get(e.from.as_str()) == Some(&"Person")
                    && labels.get(e.to.as_str()) == Some(&"Person")
            })
            .filter(|e| {
                text == R1
                    || (gender.get(e.from.as_str()) == Some(&"male")
                        && gender.get(e.to.as_str()) == Some(&"female"))
            })
            .map(|e| (e.from.as_str(), e.to.as_str()))
            .collect();
        if text == R1 {
            rows.sort();
            rows.truncate(200);
        }
        return Some(pairs(["a", "b"], rows));
    }
    if text == R3 {
        // HAS_TAG edges from a Message to a Tag, counted per tag *name* (the
        // grouping key is the value, so two tags with one name are one row).
        let names: HashMap<&str, Cell> = graph
            .nodes
            .iter()
            .filter(|n| n.label.as_str() == "Tag")
            .map(|n| (n.id.as_str(), prop(n, "name")))
            .collect();
        let labels = label_of();
        let mut counts: Vec<(Cell, i64)> = Vec::new();
        // Grouped on the cell's JSON text: `Cell` carries floats and is not
        // hashable itself.
        let mut at: HashMap<String, usize> = HashMap::new();
        for e in graph.edges.iter().filter(|e| e.label.as_str() == "HAS_TAG") {
            if labels.get(e.from.as_str()) != Some(&"Message") {
                continue;
            }
            let Some(name) = names.get(e.to.as_str()) else {
                continue;
            };
            let key = serde_json::to_string(name).unwrap_or_default();
            match at.get(&key) {
                Some(&i) => counts[i].1 += 1,
                None => {
                    at.insert(key, counts.len());
                    counts.push((name.clone(), 1));
                }
            }
        }
        counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| null_last(&a.0, &b.0)));
        counts.truncate(25);
        return Some(ResultSet {
            columns: vec!["tag".to_string(), "n".to_string()],
            rows: counts
                .into_iter()
                .map(|(name, n)| vec![name, Cell::Int(n)])
                .collect(),
        });
    }
    if text == R4 {
        let mut rows: Vec<(&str, Cell)> = graph
            .nodes
            .iter()
            .filter(|n| n.label.as_str() == "Person" && string_prop_is(n, "gender", "female"))
            .map(|n| (n.id.as_str(), prop(n, "firstName")))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        rows.truncate(100);
        return Some(ResultSet {
            columns: vec!["id".to_string(), "firstName".to_string()],
            rows: rows
                .into_iter()
                .map(|(id, first)| vec![Cell::Str(id.to_string()), first])
                .collect(),
        });
    }
    if text == R6 {
        // Countries of the cities some Person is located in, by name, distinct.
        let labels = label_of();
        let cities: std::collections::HashSet<&str> = graph
            .edges
            .iter()
            .filter(|e| e.label.as_str() == "IS_LOCATED_IN")
            .filter(|e| {
                labels.get(e.from.as_str()) == Some(&"Person")
                    && labels.get(e.to.as_str()) == Some(&"City")
            })
            .map(|e| e.to.as_str())
            .collect();
        let name_of: HashMap<&str, Cell> = graph
            .nodes
            .iter()
            .filter(|n| n.label.as_str() == "Country")
            .map(|n| (n.id.as_str(), prop(n, "name")))
            .collect();
        let mut names: Vec<Cell> = graph
            .edges
            .iter()
            .filter(|e| e.label.as_str() == "IS_PART_OF" && cities.contains(e.from.as_str()))
            .filter_map(|e| name_of.get(e.to.as_str()).cloned())
            .collect();
        names.sort_by(null_last);
        names.dedup();
        return Some(ResultSet {
            columns: vec!["country".to_string()],
            rows: names.into_iter().map(|v| vec![v]).collect(),
        });
    }
    if text == ICIJ_R2 {
        // Entities whose `jurisdiction` is the string 'SAM', by id.
        let mut rows: Vec<(&str, Cell)> = graph
            .nodes
            .iter()
            .filter(|n| n.label.as_str() == "Entity")
            .filter(|n| {
                matches!(n.props.get("jurisdiction"), Some(grust::Value::String(j)) if j == "SAM")
            })
            .map(|n| {
                (
                    n.id.as_str(),
                    n.props.get("name").map(Cell::from_grust).unwrap_or(Cell::Null),
                )
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        rows.truncate(100);
        return Some(ResultSet {
            columns: vec!["id".to_string(), "name".to_string()],
            rows: rows
                .into_iter()
                .map(|(id, name)| vec![Cell::Str(id.to_string()), name])
                .collect(),
        });
    }
    if text == ICIJ_R4 {
        // Every distinct `jurisdiction` an Entity carries, ascending, an
        // absent property as one null row that sorts last.
        let mut values: Vec<Cell> = graph
            .nodes
            .iter()
            .filter(|n| n.label.as_str() == "Entity")
            .map(|n| {
                n.props
                    .get("jurisdiction")
                    .map(Cell::from_grust)
                    .unwrap_or(Cell::Null)
            })
            .collect();
        values.sort_by(null_last);
        values.dedup();
        return Some(ResultSet {
            columns: vec!["jurisdiction".to_string()],
            rows: values.into_iter().map(|v| vec![v]).collect(),
        });
    }
    // A node's label and, where the shape filters on it, its `kind`.
    let kind_of = || -> HashMap<&str, (&str, Option<&str>)> {
        graph
            .nodes
            .iter()
            .map(|n| {
                let kind = match n.props.get("kind") {
                    Some(grust::Value::String(k)) => Some(k.as_str()),
                    _ => None,
                };
                (n.id.as_str(), (n.label.as_str(), kind))
            })
            .collect()
    };
    // Group count of edges `rel` whose source has (label, kind) and whose
    // target has label `to_label`, grouped by the `by` end; ORDER BY the
    // count DESC then the id; LIMIT.
    let group_count = |rel: &str,
                       from: (&str, Option<&str>),
                       to_label: &str,
                       by_target: bool,
                       columns: [&str; 2],
                       limit: usize| {
        let kinds = kind_of();
        let mut counts: HashMap<&str, i64> = HashMap::new();
        for e in &graph.edges {
            if e.label.as_str() != rel {
                continue;
            }
            let (Some(f), Some(t)) = (kinds.get(e.from.as_str()), kinds.get(e.to.as_str())) else {
                continue;
            };
            if f.0 == from.0 && (from.1.is_none() || f.1 == from.1) && t.0 == to_label {
                let key = if by_target {
                    e.to.as_str()
                } else {
                    e.from.as_str()
                };
                *counts.entry(key).or_default() += 1;
            }
        }
        let mut rows: Vec<(&str, i64)> = counts.into_iter().collect();
        rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        rows.truncate(limit);
        ResultSet {
            columns: columns.iter().map(|c| c.to_string()).collect(),
            rows: rows
                .into_iter()
                .map(|(id, n)| vec![Cell::Str(id.to_string()), Cell::Int(n)])
                .collect(),
        }
    };
    if text == R2 {
        return Some(group_count(
            "HAS_CREATOR",
            ("Message", Some("Post")),
            "Person",
            true,
            ["person", "posts"],
            50,
        ));
    }
    if text == R5 {
        return Some(group_count(
            "REPLY_OF",
            ("Message", Some("Comment")),
            "Message",
            true,
            ["root", "replies"],
            20,
        ));
    }
    if text == ICIJ_R1 {
        return Some(group_count(
            "OFFICER_OF",
            ("Officer", None),
            "Entity",
            false,
            ["officer", "entities"],
            50,
        ));
    }
    if text == ICIJ_R3 {
        return Some(group_count(
            "REGISTERED_ADDRESS",
            ("Entity", None),
            "Address",
            true,
            ["address", "entities"],
            25,
        ));
    }
    if text == ICIJ_C4 {
        // Ordered pairs of distinct officers sharing an entity: per entity,
        // k officers with an OFFICER_OF edge into it give k(k-1) matches
        // (the loaders keep one edge per (from, label, to), so an officer
        // has at most one edge into an entity).
        let kinds = kind_of();
        let mut officers_of: HashMap<&str, std::collections::HashSet<&str>> = HashMap::new();
        for e in &graph.edges {
            if e.label.as_str() != "OFFICER_OF" {
                continue;
            }
            let (Some(f), Some(t)) = (kinds.get(e.from.as_str()), kinds.get(e.to.as_str())) else {
                continue;
            };
            if f.0 == "Officer" && t.0 == "Entity" {
                officers_of
                    .entry(e.to.as_str())
                    .or_default()
                    .insert(e.from.as_str());
            }
        }
        let count: i64 = officers_of
            .values()
            .map(|s| (s.len() as i64) * (s.len() as i64 - 1))
            .sum();
        return Some(ResultSet {
            columns: vec!["count".to_string()],
            rows: vec![vec![Cell::Int(count)]],
        });
    }
    None
}

#[cfg(test)]
mod native_oracle_tests {
    use super::*;

    #[test]
    fn the_native_oracle_matches_the_executor_on_a_small_snb_shape() {
        use grust::{Edge, Node, Props, Value};
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let msg = |id: &str, kind: &str| {
            let mut p = Props::new();
            p.insert("kind".into(), Value::String(kind.into()));
            Node::new("Message", id, p)
        };
        for i in 0..4 {
            nodes.push(Node::new("Person", format!("p{i}"), Props::new()));
        }
        // posts: p0 has 3, p1 has 2, p2 has 2 (tie broken by id), p3 none
        let mut posts = Vec::new();
        for (i, owner) in [0, 0, 0, 1, 1, 2, 2].iter().enumerate() {
            let id = format!("post{i}");
            nodes.push(msg(&id, "Post"));
            edges.push(Edge::new(
                "HAS_CREATOR",
                id.clone(),
                format!("p{owner}"),
                Props::new(),
            ));
            posts.push(id);
        }
        // a comment by p3 must not count as a post; replies: post0 gets 2, post1 gets 1
        for (i, root) in [0, 0, 1].iter().enumerate() {
            let id = format!("c{i}");
            nodes.push(msg(&id, "Comment"));
            edges.push(Edge::new("HAS_CREATOR", id.clone(), "p3", Props::new()));
            edges.push(Edge::new(
                "REPLY_OF",
                id,
                posts[*root].clone(),
                Props::new(),
            ));
        }
        // genders and names: p0 male Ann, p1 female Bea, p2 female (no
        // firstName), p3 no gender
        for (i, gender, first) in [
            (0, Some("male"), Some("Ann")),
            (1, Some("female"), Some("Bea")),
            (2, Some("female"), None),
            (3, None, Some("Dee")),
        ] {
            let n = nodes
                .iter_mut()
                .find(|n| n.id.as_str() == format!("p{i}"))
                .unwrap();
            if let Some(g) = gender {
                n.props.insert("gender".into(), Value::String(g.into()));
            }
            if let Some(f) = first {
                n.props.insert("firstName".into(), Value::String(f.into()));
            }
        }
        // KNOWS: p0->p1 (male->female), p0->p2 (male->female), p1->p0, p3->p1,
        // p2->p3; a KNOWS edge from a post must not count
        for (a, b) in [(0, 1), (0, 2), (1, 0), (3, 1), (2, 3)] {
            edges.push(Edge::new(
                "KNOWS",
                format!("p{a}"),
                format!("p{b}"),
                Props::new(),
            ));
        }
        edges.push(Edge::new("KNOWS", "post0", "p1", Props::new()));
        // tags: t0 and t3 both named "rust" (one group), t1 "graph", t2 unnamed;
        // HAS_TAG from posts and comments; one from a Person must not count
        for (i, name) in [
            (0, Some("rust")),
            (1, Some("graph")),
            (2, None),
            (3, Some("rust")),
        ] {
            let mut p = Props::new();
            if let Some(name) = name {
                p.insert("name".into(), Value::String(name.into()));
            }
            nodes.push(Node::new("Tag", format!("t{i}"), p));
        }
        for (m, t) in [
            ("post0", 0),
            ("post1", 0),
            ("post2", 3),
            ("c0", 1),
            ("c1", 1),
            ("c2", 2),
            ("post3", 2),
            ("post4", 2),
        ] {
            edges.push(Edge::new("HAS_TAG", m, format!("t{t}"), Props::new()));
        }
        edges.push(Edge::new("HAS_TAG", "p0", "t1", Props::new()));
        // places: p0,p1 in city0 (country "Fr"), p2 in city1 (country
        // "De"), city2 has no person (country "Xx" must not appear), city3
        // has p3 and a country without a name
        for i in 0..4 {
            nodes.push(Node::new("City", format!("city{i}"), Props::new()));
        }
        for (i, name) in [(0, Some("Fr")), (1, Some("De")), (2, Some("Xx")), (3, None)] {
            let mut p = Props::new();
            if let Some(name) = name {
                p.insert("name".into(), Value::String(name.into()));
            }
            nodes.push(Node::new("Country", format!("k{i}"), p));
        }
        for (person, city) in [(0, 0), (1, 0), (2, 1), (3, 3)] {
            edges.push(Edge::new(
                "IS_LOCATED_IN",
                format!("p{person}"),
                format!("city{city}"),
                Props::new(),
            ));
        }
        for i in 0..4 {
            edges.push(Edge::new(
                "IS_PART_OF",
                format!("city{i}"),
                format!("k{i}"),
                Props::new(),
            ));
        }
        let graph = grust::Graph::new(nodes, edges);
        let index = grust::TypedGraphIndex::new(std::sync::Arc::new(graph.clone())).unwrap();
        for spec in queries_for("ldbc-snb").iter().filter(|q| q.kind == "rows") {
            let native = native_oracle(&graph, &spec.cypher).expect("pinned text recognised");
            let params = grust_cypher::CypherParameters::new();
            let executor = grust_cypher::run_bounded_read_query_indexed(
                &index,
                &bounded_text(&spec.cypher),
                &params,
                &reference_policy(),
            )
            .unwrap();
            let executor = ResultSet::from_table(executor).normalized(spec.ordered);
            assert_eq!(native.normalized(spec.ordered), executor, "{}", spec.id);
        }
        let by_id = |id: &str| {
            native_oracle(
                &graph,
                &queries_for("ldbc-snb")
                    .iter()
                    .find(|q| q.id == id)
                    .unwrap()
                    .cypher,
            )
            .unwrap()
        };
        let s = |v: &str| Cell::Str(v.into());
        assert_eq!(
            by_id("r1-knows-pairs").rows,
            vec![
                vec![s("p0"), s("p1")],
                vec![s("p0"), s("p2")],
                vec![s("p1"), s("p0")],
                vec![s("p2"), s("p3")],
                vec![s("p3"), s("p1")],
            ]
        );
        assert_eq!(
            by_id("r7-knows-unordered").normalized(false).rows,
            vec![vec![s("p0"), s("p1")], vec![s("p0"), s("p2")]]
        );
        assert_eq!(
            by_id("r3-tag-popularity").rows,
            vec![
                vec![s("rust"), Cell::Int(3)],
                vec![Cell::Null, Cell::Int(3)],
                vec![s("graph"), Cell::Int(2)],
            ]
        );
        assert_eq!(
            by_id("r4-female-persons").rows,
            vec![vec![s("p1"), s("Bea")], vec![s("p2"), Cell::Null]]
        );
        assert_eq!(
            by_id("r6-countries-distinct").rows,
            vec![vec![s("De")], vec![s("Fr")], vec![Cell::Null]]
        );
        for spec in queries_for("ldbc-snb").iter().filter(|q| q.kind != "rows") {
            assert!(native_oracle(&graph, &spec.cypher).is_none(), "{}", spec.id);
        }
    }

    #[test]
    fn the_native_oracle_matches_the_executor_on_a_small_icij_shape() {
        use grust::{Edge, Node, Props};
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        for i in 0..5 {
            nodes.push(Node::new("Officer", format!("o{i}"), Props::new()));
        }
        // jurisdictions: e0 SAM, e1 SAM (no name), e2 BVI, e3 none; the
        // Officer o0 carries a SAM jurisdiction that must not count
        for (i, jurisdiction, name) in [
            (0, Some("SAM"), Some("Zeta Ltd")),
            (1, Some("SAM"), None),
            (2, Some("BVI"), Some("Alpha Inc")),
            (3, None, Some("Beta")),
        ] {
            let mut p = Props::new();
            if let Some(j) = jurisdiction {
                p.insert("jurisdiction".into(), grust::Value::String(j.into()));
            }
            if let Some(n) = name {
                p.insert("name".into(), grust::Value::String(n.into()));
            }
            nodes.push(Node::new("Entity", format!("e{i}"), p));
        }
        nodes[0]
            .props
            .insert("jurisdiction".into(), grust::Value::String("SAM".into()));
        // addresses: a0 registered by e0, e1, e2; a1 by e3; an Officer's
        // REGISTERED_ADDRESS edge must not count
        for i in 0..2 {
            nodes.push(Node::new("Address", format!("a{i}"), Props::new()));
        }
        for (e, a) in [(0, 0), (1, 0), (2, 0), (3, 1)] {
            edges.push(Edge::new(
                "REGISTERED_ADDRESS",
                format!("e{e}"),
                format!("a{a}"),
                Props::new(),
            ));
        }
        edges.push(Edge::new("REGISTERED_ADDRESS", "o1", "a1", Props::new()));
        // e0: officers 0,1,2 (6 ordered pairs); e1: 0,1 (2); e2: 3 (0); e3: none
        for (o, e) in [(0, 0), (1, 0), (2, 0), (0, 1), (1, 1), (3, 2)] {
            edges.push(Edge::new(
                "OFFICER_OF",
                format!("o{o}"),
                format!("e{e}"),
                Props::new(),
            ));
        }
        // an Entity->Entity OFFICER_OF edge and an Officer->Officer one must not count
        edges.push(Edge::new("OFFICER_OF", "e3", "e0", Props::new()));
        edges.push(Edge::new("OFFICER_OF", "o4", "o0", Props::new()));
        let graph = grust::Graph::new(nodes, edges);
        let index = grust::TypedGraphIndex::new(std::sync::Arc::new(graph.clone())).unwrap();
        let native_shapes = [
            "c4-co-officers",
            "r1-officer-fanout",
            "r2-samoa-entities",
            "r3-address-fanin",
            "r4-jurisdictions-distinct",
        ];
        for spec in queries_for("icij")
            .iter()
            .filter(|q| native_shapes.contains(&q.id.as_str()))
        {
            let native = native_oracle(&graph, &spec.cypher).expect("pinned text recognised");
            let params = grust_cypher::CypherParameters::new();
            let executor = grust_cypher::run_bounded_read_query_indexed(
                &index,
                &bounded_text(&spec.cypher),
                &params,
                &reference_policy(),
            )
            .unwrap();
            let executor = ResultSet::from_table(executor).normalized(spec.ordered);
            assert_eq!(native.normalized(spec.ordered), executor, "{}", spec.id);
        }
        assert_eq!(
            native_oracle(
                &graph,
                &queries_for("icij")
                    .iter()
                    .find(|q| q.id == "c4-co-officers")
                    .unwrap()
                    .cypher
            )
            .unwrap()
            .rows,
            vec![vec![Cell::Int(8)]]
        );
        let by_id = |id: &str| {
            native_oracle(
                &graph,
                &queries_for("icij")
                    .iter()
                    .find(|q| q.id == id)
                    .unwrap()
                    .cypher,
            )
            .unwrap()
            .rows
        };
        assert_eq!(
            by_id("r2-samoa-entities"),
            vec![
                vec![Cell::Str("e0".into()), Cell::Str("Zeta Ltd".into())],
                vec![Cell::Str("e1".into()), Cell::Null],
            ]
        );
        assert_eq!(
            by_id("r3-address-fanin"),
            vec![
                vec![Cell::Str("a0".into()), Cell::Int(3)],
                vec![Cell::Str("a1".into()), Cell::Int(1)],
            ]
        );
        assert_eq!(
            by_id("r4-jurisdictions-distinct"),
            vec![
                vec![Cell::Str("BVI".into())],
                vec![Cell::Str("SAM".into())],
                vec![Cell::Null],
            ]
        );
        // Every other pinned ICIJ text still goes to the executor.
        for spec in queries_for("icij")
            .iter()
            .filter(|q| !native_shapes.contains(&q.id.as_str()))
        {
            assert!(native_oracle(&graph, &spec.cypher).is_none(), "{}", spec.id);
        }
    }
}
